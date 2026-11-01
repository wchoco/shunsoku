//! ごみ箱の置き場。方針（上限、古いものから捨てる、復元の順番）は core の `trash::Trash` が持ち、ここは
//! 預かったスクラップの画素を一時ファイルに退避する。
//!
//! - 状態は、閉じる直前の `ScrapModel` を丸ごとメモリに持つ。`ScrapModel` に足したフィールド（倍率、見た目の
//!   個別値、コンパクト表示など）は、ここに手を入れずにそのまま戻る。
//! - 画素（非圧縮の BGRA）は `%TEMP%\shunsoku\<プロセス ID>\` に裏のスレッドで書き、書き終えたらメモリから
//!   捨てる（常駐時のメモリを増やさないため）。書き終えるまでと、書けなかったときはメモリに持つ。
//!   書き終えたら `PostMessageW(window, message)` で UI スレッドを起こし、`app` が `on_written` を呼ぶ。
//! - 退避する画素は `Stash` 1 つずつ。画素を足すとき（トリム前の画像など）は、`Entry` に `Stash` を足して
//!   `Entry::stashes_mut` と `Entry::take` に加える。
//! - ごみ箱から消えたもの（上限で捨てた・戻した）の一時ファイルは消す。drop（終了時）でフォルダごと消す。
//!   起動時（`new`）には前回の残り（`%TEMP%\shunsoku\` の下のほかのフォルダ）も消す。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use shunsoku_core::APP_NAME;
use shunsoku_core::geometry::Size;
use shunsoku_core::image::Image;
use shunsoku_core::scrap::ScrapModel;
use shunsoku_core::template::LocalTime;
use shunsoku_core::trash::{Trash, TrashId};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// 終了時に、書き込み中の一時ファイルを書き終えるまで待つ時間の上限。
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

/// ごみ箱から取り出したスクラップ（閉じる直前の状態）。
pub struct Trashed {
    pub model: ScrapModel,
    pub image: Image,
}

/// ごみ箱の一覧の 1 行。
pub struct Listed<'a> {
    pub id: TrashId,
    pub model: &'a ScrapModel,
    /// 閉じた時刻。
    pub closed_at: LocalTime,
}

/// 預かったスクラップ 1 枚。
struct Entry {
    model: ScrapModel,
    closed_at: LocalTime,
    image: Stash,
}

impl Entry {
    /// 退避する画素のすべて。
    fn stashes_mut(&mut self) -> impl Iterator<Item = &mut Stash> {
        [&mut self.image].into_iter()
    }

    /// 画素を読み戻して、閉じる直前のスクラップにする。一時ファイルは消す。
    fn take(self) -> std::io::Result<Trashed> {
        Ok(Trashed {
            model: self.model,
            image: self.image.take()?,
        })
    }
}

/// 退避する画素 1 枚。
struct Stash {
    size: Size,
    /// 書き終えるまでと、書けなかったときの画素。書き終えたら捨てる。
    memory: Option<Arc<Image>>,
    /// 一時ファイル。書いている途中のこともある。
    file: Option<PathBuf>,
}

impl Stash {
    fn new(image: Image) -> Stash {
        Stash {
            size: image.size(),
            memory: Some(Arc::new(image)),
            file: None,
        }
    }

    /// 画素を取り出す。メモリになければ一時ファイルから読む。一時ファイルは消す。
    fn take(mut self) -> std::io::Result<Image> {
        let image = match self.memory.take() {
            Some(image) => Arc::try_unwrap(image).unwrap_or_else(|shared| (*shared).clone()),
            None => {
                let path = self.file.as_deref().ok_or_else(|| {
                    std::io::Error::other("the trashed pixels are neither in memory nor in a file")
                })?;
                let pixels = fs::read(path)?;
                Image::from_bgra(self.size, pixels).ok_or_else(|| {
                    std::io::Error::other(format!("{} has an unexpected size", path.display()))
                })?
            }
        };
        self.discard();
        Ok(image)
    }

    /// 一時ファイルを消す。書いている途中なら消せないことがあるが、書き終えたとき（`on_written`）に
    /// 持ち主がもういないので、そこで消す。
    fn discard(&mut self) {
        if let Some(path) = self.file.take() {
            remove_file(&path);
        }
    }
}

/// 一時ファイルを書き終えた（裏のスレッドから）。
struct Written {
    id: TrashId,
    path: PathBuf,
    result: Result<(), String>,
}

/// ごみ箱と、その画素の一時ファイル。
pub struct TrashStore {
    trash: Trash<Entry>,
    /// `%TEMP%\shunsoku\<プロセス ID>`。
    dir: PathBuf,
    /// 一時ファイルの名前の番号。
    next_file: u64,
    /// 書き込み中の一時ファイルの数。終了時に待つ。
    in_flight: usize,
    written: (mpsc::Sender<Written>, mpsc::Receiver<Written>),
    /// 書き終えたら起こすウィンドウ（`HWND` は Send でないので番号で持つ）とメッセージ。
    window: isize,
    message: u32,
}

impl TrashStore {
    /// 前回の残りを消して、空のごみ箱を作る。書き終えたら `window` に `message` を送る。
    pub fn new(limit: u8, window: HWND, message: u32) -> TrashStore {
        let root = std::env::temp_dir().join(APP_NAME);
        let own = std::process::id().to_string();
        remove_leftovers(&root, &own);
        TrashStore {
            trash: Trash::new(usize::from(limit)),
            dir: root.join(own),
            next_file: 0,
            in_flight: 0,
            written: mpsc::channel(),
            window: window.0 as isize,
            message,
        }
    }

    /// 閉じたスクラップを入れ、画素を裏のスレッドで一時ファイルに書き始める。上限が 0 なら、そのまま捨てる。
    pub fn push(&mut self, model: ScrapModel, image: Image, closed_at: LocalTime) {
        let entry = Entry {
            model,
            closed_at,
            image: Stash::new(image),
        };
        let pushed = self.trash.push(entry);
        for mut discarded in pushed.discarded {
            discard(&mut discarded);
        }
        let Some(id) = pushed.id else {
            log::info!("scrap discarded (the trash limit is 0)");
            return;
        };
        log::info!("scrap moved to the trash");
        if let Err(e) = fs::create_dir_all(&self.dir) {
            log::warn!(
                "could not create {}; keeping the trashed pixels in memory: {e}",
                self.dir.display()
            );
            return;
        }
        let Some(entry) = self.trash.get_mut(id) else {
            return;
        };
        for stash in entry.stashes_mut() {
            let path = self.dir.join(format!("{}.bgra", self.next_file));
            self.next_file += 1;
            let Some(image) = stash.memory.clone() else {
                continue;
            };
            let sender = self.written.0.clone();
            let window = self.window;
            let message = self.message;
            let job_path = path.clone();
            let spawned = std::thread::Builder::new()
                .name("trash".into())
                .spawn(move || {
                    let result = catch_unwind(AssertUnwindSafe(|| write_pixels(&job_path, &image)))
                        .map_err(|_| "panicked".to_owned())
                        .and_then(|written| written.map_err(|e| e.to_string()));
                    drop(image);
                    let _ = sender.send(Written {
                        id,
                        path: job_path,
                        result,
                    });
                    // SAFETY: UI スレッドのメッセージ専用ウィンドウに送るだけ。もう壊れていれば失敗するだけ
                    let _ = unsafe {
                        PostMessageW(Some(HWND(window as *mut _)), message, WPARAM(0), LPARAM(0))
                    };
                });
            match spawned {
                Ok(_) => {
                    stash.file = Some(path);
                    self.in_flight += 1;
                }
                Err(e) => log::warn!(
                    "could not start the trash thread; keeping the pixels in memory: {e}"
                ),
            }
        }
    }

    /// 書き終えた一時ファイルを受け取る（`message` を受けたら呼ぶ）。書けたらメモリの画素を捨てる。
    /// 持ち主がもうごみ箱にいなければ（捨てた・戻した）、一時ファイルを消す。
    pub fn on_written(&mut self) {
        let results: Vec<Written> = self.written.1.try_iter().collect();
        for written in results {
            self.in_flight = self.in_flight.saturating_sub(1);
            let stash = self.trash.get_mut(written.id).and_then(|entry| {
                entry
                    .stashes_mut()
                    .find(|s| s.file.as_deref() == Some(written.path.as_path()))
            });
            match (stash, written.result) {
                (Some(stash), Ok(())) => {
                    stash.memory = None;
                }
                (Some(stash), Err(e)) => {
                    log::warn!(
                        "could not write {}; keeping the pixels in memory: {e}",
                        written.path.display()
                    );
                    stash.discard();
                }
                (None, _) => remove_file(&written.path),
            }
        }
    }

    /// 新しいものから並べた一覧。
    pub fn list(&self) -> Vec<Listed<'_>> {
        self.trash
            .list()
            .map(|(id, entry)| Listed {
                id,
                model: &entry.model,
                closed_at: entry.closed_at,
            })
            .collect()
    }

    /// `id` のスクラップを取り出す。もうなければ None。画素を読めなければ Some(Err)。
    pub fn restore(&mut self, id: TrashId) -> Option<std::io::Result<Trashed>> {
        self.trash.restore(id).map(Entry::take)
    }

    /// 最後に閉じたスクラップを取り出す。空なら None。画素を読めなければ Some(Err)。
    #[allow(dead_code)] // 復元のホットキー（#57）で使う
    pub fn restore_last(&mut self) -> Option<std::io::Result<Trashed>> {
        self.trash.restore_last().map(Entry::take)
    }

    /// 上限を変える。下げて超えた分は古いものから捨てる。
    pub fn set_limit(&mut self, limit: u8) {
        for mut discarded in self.trash.set_limit(usize::from(limit)) {
            discard(&mut discarded);
        }
    }

    /// 空にして、一時ファイルのフォルダを消す。書き込み中のものは、書き終えるまで少し待つ。
    pub fn shutdown(&mut self) {
        self.trash.clear();
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        while self.in_flight > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.written.1.recv_timeout(left) {
                Ok(_) => self.in_flight -= 1,
                Err(_) => {
                    log::warn!("{} trash file(s) still being written", self.in_flight);
                    break;
                }
            }
        }
        remove_dir(&self.dir);
    }
}

impl Drop for TrashStore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 捨てたスクラップの一時ファイルを消す。
fn discard(entry: &mut Entry) {
    for stash in entry.stashes_mut() {
        stash.discard();
    }
}

/// 画素を `path` に新しいファイルとして書く（裏のスレッドで呼ぶ）。失敗したら作りかけのファイルを消す。
fn write_pixels(path: &Path, image: &Image) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file.write_all(image.pixels()).and_then(|()| file.flush());
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    written
}

/// `root` の下の、`own` 以外のフォルダ（前回までのプロセスの残り）を消す。
fn remove_leftovers(root: &Path, own: &str) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        if entry.file_name() != own {
            log::info!("removing leftover trash {}", entry.path().display());
            remove_dir(&entry.path());
        }
    }
}

fn remove_file(path: &Path) {
    if let Err(e) = fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not remove {}: {e}", path.display());
    }
}

fn remove_dir(path: &Path) {
    let removed = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    if let Err(e) = removed
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not remove {}: {e}", path.display());
    }
}
