//! トレイのアイコンのバルーン（`NIF_INFO`）で出す通知。Windows ではトーストとして出る。
//!
//! - 新しい通知は前の通知を置き換える。
//! - 集中モードなどで出ないことがある。通知が出たかどうかで処理を分けないこと。
//! - 通知のクリック（`NIN_BALLOONUSERCLICK`）は中身を持たないので、何の通知だったかは `app` が覚えておく。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_INFO, NIIF_ERROR, NIIF_INFO, NIIF_RESPECT_QUIET_TIME, NIIF_WARNING, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};

use crate::wide::copy_truncated;

/// 通知の種類。設定で通知をオフにしても、止めるのは `Success` だけ（spec #32）。
pub enum Level {
    #[expect(dead_code, reason = "保存の通知（#44）で使う")]
    Success,
    Warning,
    Error,
}

/// `hwnd` と `icon_id` のトレイのアイコンに通知を出す。出せたら true。
/// タイトルは 63 文字、本文は 255 文字（UTF-16）で切る。
pub fn show(hwnd: HWND, icon_id: u32, level: Level, title: &str, text: &str) -> bool {
    let flags = match level {
        Level::Success => NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
        Level::Warning => NIIF_WARNING,
        Level::Error => NIIF_ERROR,
    };
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: icon_id,
        uFlags: NIF_INFO,
        dwInfoFlags: flags,
        ..Default::default()
    };
    copy_truncated(&mut data.szInfoTitle, title);
    copy_truncated(&mut data.szInfo, text);
    // SAFETY: data は呼び出しの間生きている
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
}
