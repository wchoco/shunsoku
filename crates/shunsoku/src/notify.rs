//! トレイのアイコンのバルーン（`NIF_INFO`）で出す通知。Windows ではトーストとして出る。
//!
//! - 新しい通知は前の通知を置き換える（`show` が出す前に今の通知を消す）。
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
    Success,
    Warning,
    Error,
}

/// `hwnd` と `icon_id` のトレイのアイコンに通知を出す。出せたら true。
/// タイトルは 63 文字、本文は 255 文字（UTF-16）で切る。
///
/// 出す前に、今出ている通知を消す（`dismiss`）。
///
/// - 一次資料（NOTIFYICONDATAW の Remarks）: 「No more than one balloon notification at a time can be displayed
///   for the taskbar. If an application attempts to display a notification when one is already being
///   displayed, the new notification is queued and displayed when the older notification goes away.」
///   新しい `NIF_INFO` を送るだけでは置き換わらず、待ち行列に入る。v0.1.0-rc.1 の実機（Windows 11）では、
///   2 つ目は 1 つ目が消えるまで出ず、続けて出した通知が出ないこともあった（M1-15、M3）。
/// - 選んだ方法: `szInfo` を空にした `NIF_INFO` を `NIM_MODIFY` で送って今の通知を消し、それから新しい通知を
///   出す。資料（szInfo）: 「To remove the balloon notification from the UI, either delete the icon (with
///   NIM_DELETE) or set the NIF_INFO flag in uFlags and set szInfo to an empty string.」。古い通知が
///   「goes away」するので、新しい通知が待たずに出る。
/// - 選ばなかった方法: `NIF_REALTIME`。資料（uFlags）: 「If the balloon notification cannot be displayed
///   immediately, discard it.」。前の通知が出ている間は、新しい方が捨てられる。置き換えたいのは古い方なので、
///   逆になる。
/// - `NIM_DELETE` でアイコンごと消す方法は、トレイのアイコンが一瞬消えて並びが変わりうるので使わない。
pub fn show(hwnd: HWND, icon_id: u32, level: Level, title: &str, text: &str) -> bool {
    dismiss(hwnd, icon_id);
    let flags = match level {
        Level::Success => NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
        Level::Warning => NIIF_WARNING,
        Level::Error => NIIF_ERROR,
    };
    let mut data = blank_balloon(hwnd, icon_id);
    data.dwInfoFlags = flags;
    copy_truncated(&mut data.szInfoTitle, title);
    copy_truncated(&mut data.szInfo, text);
    // SAFETY: data は呼び出しの間生きている
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
}

/// 今出ている（か、待っている）通知を消す。`szInfo` を空にした `NIF_INFO`（`show` の説明）。
/// 出ていなくても害はないので、失敗は無視する。
fn dismiss(hwnd: HWND, icon_id: u32) {
    let data = blank_balloon(hwnd, icon_id);
    // SAFETY: data は呼び出しの間生きている
    let removed = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() };
    if !removed {
        log::debug!("could not dismiss the current notification");
    }
}

/// `NIF_INFO` の通知の型紙。`szInfo` は空（このまま送ると今の通知を消す）。
fn blank_balloon(hwnd: HWND, icon_id: u32) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: icon_id,
        uFlags: NIF_INFO,
        ..Default::default()
    }
}
