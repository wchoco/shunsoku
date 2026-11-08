//! ごみ箱の置き場。方針（上限、古いものから捨てる、復元の順番）は core の `trash::Trash` が持ち、ここは
//! 預かったスクラップの画素を一時ファイルに退避する。
//!
//! - 状態は、閉じる直前の `ScrapModel` を丸ごとメモリに持つ。`ScrapModel` に足したフィールド（倍率、見た目の
//!   個別値、コンパクト表示など）は、ここに手を入れずにそのまま戻る。
//! - 画素（非圧縮の BGRA）は `%TEMP%\shunsoku\<プロセス ID>\` に裏のスレッドで書き、書き終えたらメモリから
//!   捨てる（常駐時のメモリを増やさないため）。書き終えるまでと、書けなかったときはメモリに持つ。
//!   書き終えたら `PostMessageW(window, message)` で UI スレッドを起こし、`app` が `on_written` を呼ぶ。
//! - 退避する画素は `Stash` 1 つずつ。いまは元の画像（`Entry::image`）だけ。白紙スクラップは画素を退避せず、
//!   戻すときに背景色から作り直す（`ScrapModel::blank_image`）。
//! - 一覧に付けるサムネイル（`Entry::thumbnail`、閉じたときの見た目を縮めたもの）は小さいのでメモリに持ち、
//!   一覧を出すたびに一時ファイルを読まない。項目と一緒に捨てる。
//! - ごみ箱から消えたもの（上限で捨てた・戻した）の一時ファイルは消す。drop（終了時）でフォルダごと消す。
//!   起動時（`new`）には前回の残り（`%TEMP%\shunsoku\` の下のほかのフォルダ）も消す。
//! - `%TEMP%\shunsoku` は同じユーザーのどのプロセスも書き換えられる。管理者として動く常駐本体が、下の整合性
//!   レベルのプロセスが置いた junction をたどって別の場所を消したり書いたりしないよう、`%TEMP%\shunsoku` と
//!   自分のフォルダは、別の場所を指すもの（reparse point）でないと確かめてから、消せず名前も変えられない形で
//!   開いたまま（`Pinned`）にする。確かめられなければ一時ファイルは使わず、画素はメモリに持つ。

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
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle, OPEN_EXISTING,
};
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
    /// 閉じたときの見た目（トリムと書き込みを反映したもの）を縮めた画像。
    pub thumbnail: &'a Image,
    /// 閉じた時刻。
    pub closed_at: LocalTime,
}

/// 預かったスクラップ 1 枚。
struct Entry {
    model: ScrapModel,
    closed_at: LocalTime,
    /// 元の画像の画素。白紙スクラップなら None（`ScrapModel::blank_image` で作り直す）。
    image: Option<Stash>,
    thumbnail: Image,
}

impl Entry {
    /// 画素を読み戻して、閉じる直前のスクラップにする。一時ファイルは消す。
    fn take(self) -> std::io::Result<Trashed> {
        let image = match self.image {
            Some(stash) => stash.take()?,
            None => self.model.blank_image().ok_or_else(|| {
                std::io::Error::other("the trashed scrap has neither pixels nor a background")
            })?,
        };
        Ok(Trashed {
            model: self.model,
            image,
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
    /// `%TEMP%\shunsoku\<プロセス ID>`。確かめられなかったら None（画素はメモリに持つ）。
    folder: Option<Folder>,
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
        let folder = Folder::prepare(&root, &own)
            .inspect_err(|e| {
                log::warn!(
                    "could not prepare {}; keeping the trashed pixels in memory: {e}",
                    root.display()
                )
            })
            .ok();
        TrashStore {
            trash: Trash::new(usize::from(limit)),
            folder,
            next_file: 0,
            in_flight: 0,
            written: mpsc::channel(),
            window: window.0 as isize,
            message,
        }
    }

    /// 閉じたスクラップを入れ、画素を裏のスレッドで一時ファイルに書き始める。上限が 0 なら、そのまま捨てる。
    /// `thumbnail` は一覧に付ける縮めた見た目。
    pub fn push(
        &mut self,
        model: ScrapModel,
        image: Image,
        thumbnail: Image,
        closed_at: LocalTime,
    ) {
        // 白紙スクラップの画素は背景色から作り直せるので、預からない
        let image = (!model.is_blank()).then(|| Stash::new(image));
        let entry = Entry {
            model,
            closed_at,
            image,
            thumbnail,
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
        let Some(folder) = &self.folder else {
            return;
        };
        let path = folder.path.join(format!("{}.bgra", self.next_file));
        let Some(stash) = self.trash.get_mut(id).and_then(|e| e.image.as_mut()) else {
            return;
        };
        self.next_file += 1;
        let Some(image) = stash.memory.clone() else {
            return;
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
            Err(e) => {
                log::warn!("could not start the trash thread; keeping the pixels in memory: {e}")
            }
        }
    }

    /// 書き終えた一時ファイルを受け取る（`message` を受けたら呼ぶ）。書けたらメモリの画素を捨てる。
    /// 持ち主がもうごみ箱にいなければ（捨てた・戻した）、一時ファイルを消す。
    pub fn on_written(&mut self) {
        let results: Vec<Written> = self.written.1.try_iter().collect();
        for written in results {
            self.in_flight = self.in_flight.saturating_sub(1);
            let stash = self
                .trash
                .get_mut(written.id)
                .and_then(|entry| entry.image.as_mut())
                .filter(|s| s.file.as_deref() == Some(written.path.as_path()));
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
                thumbnail: &entry.thumbnail,
                closed_at: entry.closed_at,
            })
            .collect()
    }

    /// `id` のスクラップを取り出す。もうなければ None。画素を読めなければ Some(Err)。
    pub fn restore(&mut self, id: TrashId) -> Option<std::io::Result<Trashed>> {
        self.trash.restore(id).map(Entry::take)
    }

    /// 最後に閉じたスクラップを取り出す。空なら None。画素を読めなければ Some(Err)。
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
        if let Some(folder) = self.folder.take() {
            folder.remove();
        }
    }
}

/// 開いている間、消すことも名前を変えることも、別の場所を指すものに差し替えることもできないフォルダ。
/// 読み取りのアクセスで、`FILE_SHARE_DELETE` を付けずに開く（共有の確かめは中身へのアクセスを持つハンドルにだけ
/// かかるので、属性だけのアクセスでは足りない）。
struct Pinned(HANDLE);

impl Pinned {
    /// `path` を開き、reparse point（junction・シンボリックリンク）でないふつうのフォルダであることを確かめる。
    /// 開くときにリンクをたどらないので、確かめたものと開いたままにするものは同じ。
    fn open(path: &Path) -> std::io::Result<Pinned> {
        let name = windows::core::HSTRING::from(path.as_os_str());
        // SAFETY: name は呼び出しの間生きている。開いたハンドルは Pinned が閉じる
        let handle = unsafe {
            CreateFileW(
                &name,
                (FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }?;
        let pinned = Pinned(handle);
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: info はこの関数の中で生きている
        unsafe { GetFileInformationByHandle(pinned.0, &mut info) }?;
        let attributes = info.dwFileAttributes;
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(std::io::Error::other(format!(
                "{} points elsewhere (a reparse point)",
                path.display()
            )));
        }
        if attributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
            return Err(std::io::Error::other(format!(
                "{} is not a folder",
                path.display()
            )));
        }
        Ok(pinned)
    }
}

impl Drop for Pinned {
    fn drop(&mut self) {
        // SAFETY: 開いたハンドルを 1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 一時ファイルを置く自分のフォルダ（`<root>\<own>`）。`root` と自分のフォルダを開いたままにする。
struct Folder {
    path: PathBuf,
    // 落とす順に並べる（自分のフォルダを先に閉じる）
    _own: Pinned,
    _root: Pinned,
}

impl Folder {
    /// `root` を確かめて開いたままにし、その下の前回までの残りを消してから、自分のフォルダを新しく作る。
    fn prepare(root: &Path, own: &str) -> std::io::Result<Folder> {
        match fs::create_dir(root) {
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
            _ => {}
        }
        let pinned_root = Pinned::open(root)?;
        remove_leftovers(root);
        let path = root.join(own);
        // 残りを消したあとなので、ここにあるのは消したあとに置かれたもの。使わない
        fs::create_dir(&path)?;
        let pinned_own = Pinned::open(&path)?;
        Ok(Folder {
            path,
            _own: pinned_own,
            _root: pinned_root,
        })
    }

    /// 自分のフォルダを閉じてから消す。
    fn remove(self) {
        let Folder { path, _own, _root } = self;
        drop(_own);
        remove_entry(&path);
    }
}

impl Drop for TrashStore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 捨てたスクラップの一時ファイルを消す。
fn discard(entry: &mut Entry) {
    if let Some(stash) = &mut entry.image {
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

/// `root` の下のもの（前回までのプロセスの残り）をすべて消す。`root` は `Pinned` で確かめて開いたものを渡す。
fn remove_leftovers(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        log::info!("removing leftover trash {}", entry.path().display());
        remove_entry(&entry.path());
    }
}

fn remove_file(path: &Path) {
    if let Err(e) = fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not remove {}: {e}", path.display());
    }
}

/// `path` を消す。フォルダなら中身ごと消す。別の場所を指すもの（junction・シンボリックリンク）は、指す先を
/// たどらずにリンクだけを消す。
fn remove_entry(path: &Path) {
    let removed = match fs::symlink_metadata(path) {
        Err(e) => Err(e),
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::remove_dir(path).or_else(|_| fs::remove_file(path))
        }
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    };
    if let Err(e) = removed
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not remove {}: {e}", path.display());
    }
}

/// 前回の残りの片付けが junction をたどらないこと。試験用のフォルダはビルドの出力（テストの exe の隣）に作り、
/// ユーザーの `%TEMP%` には触れない。
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// ビルドの出力の下の試験用のフォルダ。落ちて残ったものは次の回の始めに消える。
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Sandbox {
            let exe = std::env::current_exe().unwrap();
            let root = exe
                .parent()
                .unwrap()
                .join("test-sandbox")
                .join(format!("shunsoku-trash-{name}"));
            remove_entry(&root);
            fs::create_dir_all(&root).unwrap();
            Sandbox(root)
        }

        /// 消されては困るファイルを置いたフォルダ（junction の指す先）。
        fn protected(&self) -> PathBuf {
            let dir = self.0.join("protected");
            fs::create_dir_all(dir.join("inner")).unwrap();
            fs::write(dir.join("keep.txt"), b"keep").unwrap();
            fs::write(dir.join("inner").join("keep.txt"), b"keep").unwrap();
            dir
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            remove_entry(&self.0);
        }
    }

    fn junction(link: &Path, target: &Path) {
        let made = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink /J failed: {made:?}");
    }

    fn protected_is_intact(dir: &Path) -> bool {
        dir.join("keep.txt").is_file() && dir.join("inner").join("keep.txt").is_file()
    }

    #[test]
    fn a_root_that_is_a_junction_is_refused_and_its_target_is_left_alone() {
        let sandbox = Sandbox::new("root-junction");
        let protected = sandbox.protected();
        let root = sandbox.0.join("shunsoku");
        junction(&root, &protected);

        assert!(Folder::prepare(&root, "1").is_err());
        assert!(protected_is_intact(&protected));
        assert!(!protected.join("1").exists());
    }

    #[test]
    fn a_leftover_junction_is_removed_without_following_it() {
        let sandbox = Sandbox::new("leftover-junction");
        let protected = sandbox.protected();
        let root = sandbox.0.join("shunsoku");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("2")).unwrap();
        fs::write(root.join("2").join("0.bgra"), b"old").unwrap();
        junction(&root.join("3"), &protected);

        let folder = Folder::prepare(&root, "1").unwrap();

        assert!(protected_is_intact(&protected));
        assert!(!root.join("2").exists());
        assert!(!root.join("3").exists());
        assert!(folder.path.is_dir());
        folder.remove();
        assert!(!root.join("1").exists());
    }

    #[test]
    fn the_root_cannot_be_swapped_while_the_folder_is_open() {
        let sandbox = Sandbox::new("pinned");
        let root = sandbox.0.join("shunsoku");
        let folder = Folder::prepare(&root, "1").unwrap();

        assert!(fs::rename(&root, sandbox.0.join("moved")).is_err());
        assert!(fs::remove_dir_all(&root).is_err());
        assert!(root.join("1").is_dir());

        folder.remove();
        assert!(!root.join("1").exists());
        assert!(fs::rename(&root, sandbox.0.join("moved")).is_ok());
    }
}
