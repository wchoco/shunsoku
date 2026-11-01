//! 二重起動の防止。ユーザーとセッションごとの名前付きミューテックスで 1 プロセスに限る。

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, HWND_MESSAGE, PostMessageW};
use windows::core::{PCWSTR, Result};

use crate::wide::to_wide;

/// 1 つ目のプロセスであることの印。生かしている間だけ、他のプロセスは 2 つ目になる。
pub struct Guard(HANDLE);

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: CreateMutexW で得たハンドルを 1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 1 つ目なら `Some(Guard)`、既に動いているプロセスがあれば `None`。
pub fn acquire() -> Result<Option<Guard>> {
    // `Local\` でセッションごと、ユーザー名でユーザーごとにする（名前に `\` は使えないので消す）
    let user = std::env::var("USERNAME")
        .unwrap_or_default()
        .replace('\\', "_");
    let name = to_wide(&format!("Local\\shunsoku-{user}"));
    // SAFETY: 名前は呼び出しの間生きている。GetLastError は CreateMutexW の直後に読む
    let (handle, already) = unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))?;
        (handle, GetLastError() == ERROR_ALREADY_EXISTS)
    };
    let guard = Guard(handle);
    Ok((!already).then_some(guard))
}

/// 既にあるプロセスのメッセージ専用ウィンドウ（クラス名 `class`）に `message` を送る。送れたら true。
pub fn notify_first(class: PCWSTR, message: u32) -> bool {
    // SAFETY: メッセージ専用ウィンドウは FindWindowW では見つからないので HWND_MESSAGE の下を探す
    unsafe {
        match FindWindowExW(Some(HWND_MESSAGE), None, class, PCWSTR::null()) {
            Ok(peer) => PostMessageW(Some(peer), message, WPARAM(0), LPARAM(0)).is_ok(),
            Err(_) => false,
        }
    }
}
