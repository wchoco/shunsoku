//! 設定ファイルの監視。裏のスレッドで、設定ファイルのフォルダを `ReadDirectoryChangesW` で見張る。
//!
//! - フォルダを見張り、名前が設定ファイルのものの変化だけを拾う。エディタは一時ファイルに書いて名前を変えて
//!   保存することが多いので、書き込みと名前の変化の両方を見る。
//! - 1 回の保存で通知が何度も届くので、最後の通知から `DEBOUNCE_MS` だけ静かになってから読み込み直す。
//! - 読んだ中身（`Event`）は `mpsc` に入れ、`PostMessageW` で UI スレッドを起こす。中身の解釈（`settings::reload`）
//!   と反映は UI スレッドの `app` が行う。
//! - フォルダがなければ作る（ログと同じフォルダ）。設定ファイルは作らない（spec #32）。
//! - drop で止める（止める合図のイベントを立て、スレッドの終わりを待つ）。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_NOTIFY_ENUM_DIR, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::PCWSTR;

use crate::wide::to_wide;

/// 最後の変化の通知から、読み込み直すまで待つ時間。
const DEBOUNCE_MS: u32 = 200;
/// 読み込みが共有違反などで失敗したときに、読み直す回数と間隔。
const READ_TRIES: u32 = 5;
const READ_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// 監視のスレッドから UI スレッドへの知らせ。
pub enum Event {
    /// 設定ファイルが変わったので読み込み直した。
    Changed(Content),
    /// 監視が止まった（フォルダを開けない、パニックなど）。理由を持つ。
    Stopped(String),
}

/// 読み込み直した設定ファイルの中身。
pub enum Content {
    /// ファイルがない。
    Missing,
    Text(String),
    /// 読めなかった（共有違反が続いた、UTF-8 でないなど）。理由を持つ。
    Unreadable(String),
}

/// 動いている監視。drop で止める。
pub struct Watcher {
    /// 止める合図（手動リセットのイベント）。
    stop: HANDLE,
    thread: Option<JoinHandle<()>>,
}

/// `path`（設定ファイル）の監視を始める。変化を読み込み直すたびに `sender` に入れ、`window` に `message` を送る。
pub fn start(
    path: &Path,
    sender: mpsc::Sender<Event>,
    window: HWND,
    message: u32,
) -> windows::core::Result<Watcher> {
    // SAFETY: 名前なしの手動リセットのイベントを作るだけ
    let stop = unsafe { CreateEventW(None, true, false, None)? };
    // HANDLE と HWND は Send でないので、番号にして渡す
    let stop_raw = stop.0 as isize;
    let window = window.0 as isize;
    let path = path.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("settings-watch".into())
        .spawn(move || {
            let post = |event: Event| {
                let _ = sender.send(event);
                // SAFETY: UI スレッドのメッセージ専用ウィンドウに送るだけ。もう壊れていれば失敗するだけ
                let _ = unsafe {
                    PostMessageW(Some(HWND(window as *mut _)), message, WPARAM(0), LPARAM(0))
                };
            };
            let stop = HANDLE(stop_raw as *mut _);
            let result = catch_unwind(AssertUnwindSafe(|| watch(&path, stop, &post)));
            let reason = match result {
                Ok(Ok(())) => return,
                Ok(Err(e)) => e.to_string(),
                Err(_) => "panic".to_owned(),
            };
            log::error!("settings watcher stopped: {reason}");
            post(Event::Stopped(reason));
        });
    match spawned {
        Ok(thread) => Ok(Watcher {
            stop,
            thread: Some(thread),
        }),
        Err(e) => {
            // SAFETY: 上で作ったイベントを閉じる
            unsafe {
                let _ = CloseHandle(stop);
            }
            Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                e.to_string(),
            ))
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: 自分で作ったイベントを立て、スレッドが終わってから閉じる
        unsafe {
            let _ = SetEvent(self.stop);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // SAFETY: スレッドはもう終わったので、イベントを使う者はいない
        unsafe {
            let _ = CloseHandle(self.stop);
        }
    }
}

/// 閉じ忘れないハンドル。
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: 自分で開いたハンドルを 1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// `stop` が立つまで見張る。変化を読み込み直すたびに `post` を呼ぶ。
fn watch(path: &Path, stop: HANDLE, post: &dyn Fn(Event)) -> windows::core::Result<()> {
    let dir = path.parent().ok_or_else(|| invalid_path(path))?;
    let name: Vec<u16> = path
        .file_name()
        .ok_or_else(|| invalid_path(path))?
        .to_string_lossy()
        .encode_utf16()
        .collect();
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("could not create {}: {e}", dir.display());
    }
    let wide = to_wide(&dir.to_string_lossy());
    // SAFETY: wide は呼び出しの間生きている
    let handle = Owned(unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )?
    });
    // SAFETY: 名前なしの手動リセットのイベントを作るだけ
    let event = Owned(unsafe { CreateEventW(None, true, false, None)? });
    log::info!("watching {} for settings changes", dir.display());

    // FILE_NOTIFY_INFORMATION は DWORD 境界に置く
    let mut buffer = vec![0u32; 2048];
    let mut overlapped = OVERLAPPED {
        hEvent: event.0,
        ..Default::default()
    };
    let mut outstanding = false;
    let mut changed = false;
    let result = loop {
        if !outstanding {
            // SAFETY: buffer と overlapped は、読み取りが終わるか取り消されるまでこの関数の中で生きている
            let issued = unsafe {
                let _ = ResetEvent(event.0);
                ReadDirectoryChangesW(
                    handle.0,
                    buffer.as_mut_ptr().cast(),
                    size_of_val(buffer.as_slice()) as u32,
                    false,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_SIZE,
                    None,
                    Some(&mut overlapped),
                    None,
                )
            };
            if let Err(e) = issued {
                break Err(e);
            }
            outstanding = true;
        }
        let timeout = if changed { DEBOUNCE_MS } else { INFINITE };
        // SAFETY: どちらのハンドルも生きている
        let waited = unsafe { WaitForMultipleObjects(&[event.0, stop], false, timeout) };
        if waited == WAIT_OBJECT_0 {
            outstanding = false;
            let mut bytes = 0u32;
            // SAFETY: 終わった読み取りの結果を受け取る
            match unsafe { GetOverlappedResult(handle.0, &overlapped, &mut bytes, false) } {
                // 0 バイトはバッファがあふれた。何が変わったか分からないので、変わったことにする
                Ok(()) if bytes == 0 => changed = true,
                // SAFETY: bytes までは読み取りが書いた
                Ok(()) => changed |= unsafe { mentions(&buffer, bytes as usize, &name) },
                Err(e) if e.code() == ERROR_NOTIFY_ENUM_DIR.to_hresult() => changed = true,
                Err(e) => break Err(e),
            }
        } else if waited.0 == WAIT_OBJECT_0.0 + 1 {
            break Ok(());
        } else if waited == WAIT_TIMEOUT {
            changed = false;
            post(Event::Changed(read(path)));
        } else {
            break Err(windows::core::Error::from_thread());
        }
    };
    if outstanding {
        // SAFETY: 出したままの読み取りを取り消し、終わるまで待ってから buffer を捨てる
        unsafe {
            let _ = CancelIoEx(handle.0, Some(&overlapped));
            let mut bytes = 0u32;
            let _ = GetOverlappedResult(handle.0, &overlapped, &mut bytes, true);
        }
    }
    result
}

fn invalid_path(path: &Path) -> windows::core::Error {
    windows::core::Error::new(
        windows::Win32::Foundation::E_INVALIDARG,
        format!("not a file path: {}", path.display()),
    )
}

/// 変化の一覧（`FILE_NOTIFY_INFORMATION` の並び）に、名前が `name` のファイルがあるか。大文字と小文字は区別しない。
///
/// # Safety
/// `buffer` の先頭 `len` バイトは `ReadDirectoryChangesW` が書いたものであること。
unsafe fn mentions(buffer: &[u32], len: usize, name: &[u16]) -> bool {
    let base = buffer.as_ptr().cast::<u8>();
    let len = len.min(size_of_val(buffer));
    let mut offset = 0usize;
    // NextEntryOffset、Action、FileNameLength（バイト）の 3 つの u32 のあとに FileName
    const HEADER: usize = 12;
    while offset + HEADER <= len {
        // SAFETY: offset + HEADER <= len で、各要素は DWORD 境界に置かれる
        let (next, name_bytes) = unsafe {
            let entry = base.add(offset).cast::<u32>();
            (*entry as usize, *entry.add(2) as usize)
        };
        let units = name_bytes / 2;
        if offset + HEADER + name_bytes <= len {
            // SAFETY: 名前は範囲の中にある。u16 の境界（4 の倍数 + 12）に置かれる
            let entry_name = unsafe {
                std::slice::from_raw_parts(base.add(offset + HEADER).cast::<u16>(), units)
            };
            if eq_ignore_case(entry_name, name) {
                return true;
            }
        }
        if next == 0 {
            break;
        }
        offset += next;
    }
    false
}

fn eq_ignore_case(a: &[u16], b: &[u16]) -> bool {
    let lower = |s: &[u16]| String::from_utf16_lossy(s).to_lowercase();
    a.len() == b.len() && lower(a) == lower(b)
}

/// 設定ファイルを読む。保存の直後は他のプロセスが開いていて読めないことがあるので、何度か読み直す。
fn read(path: &Path) -> Content {
    let mut tries = 0;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) => return Content::Text(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Content::Missing,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return Content::Unreadable(e.to_string());
            }
            Err(e) => {
                tries += 1;
                if tries >= READ_TRIES {
                    return Content::Unreadable(e.to_string());
                }
                log::debug!("could not read {} yet: {e}", path.display());
                std::thread::sleep(READ_RETRY_INTERVAL);
            }
        }
    }
}
