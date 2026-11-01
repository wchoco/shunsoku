//! タスクトレイのアイコン。`NOTIFYICON_VERSION_4` で登録する。メニューは `menu` で出す。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
    NIN_BALLOONUSERCLICK, NIN_SELECT, NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{HICON, WM_CONTEXTMENU};

use crate::wide::copy_truncated;

/// トレイのアイコンの番号。通知（`notify`）も同じアイコンに出す。
pub const ICON_ID: u32 = 1;

/// トレイのアイコン。drop で消える。
pub struct Tray {
    hwnd: HWND,
    callback_message: u32,
    icon: HICON,
    tooltip: String,
}

/// トレイのアイコンへの操作（`callback_message` の wParam/lParam を読んだもの）。
pub enum Event {
    /// 左クリック、または選んで Enter / Space。
    Select,
    /// 右クリック、またはキーボードでのメニュー。座標はスクリーン座標（物理ピクセル）。
    ContextMenu {
        x: i32,
        y: i32,
    },
    /// 通知（`notify`）がクリックされた。何の通知だったかは持たない。
    NotificationClick,
    Other,
}

impl Tray {
    /// まだ登録はしない。`register` で登録する。
    pub fn new(hwnd: HWND, callback_message: u32, icon: HICON, tooltip: String) -> Tray {
        Tray {
            hwnd,
            callback_message,
            icon,
            tooltip,
        }
    }

    /// アイコンを登録する。前の登録が残っていれば消してから登録し直す。
    /// Explorer が動いていないと失敗する（そのときは TaskbarCreated で呼び直す）。
    pub fn register(&self) -> bool {
        let mut data = NOTIFYICONDATAW {
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: self.callback_message,
            hIcon: self.icon,
            ..base(self.hwnd)
        };
        copy_truncated(&mut data.szTip, &self.tooltip);
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: data は呼び出しの間生きている
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &base(self.hwnd));
            Shell_NotifyIconW(NIM_ADD, &data).as_bool()
                && Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool()
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: data は呼び出しの間生きている
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &base(self.hwnd));
        }
    }
}

fn base(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

/// `callback_message` の引数を読む。
pub fn event(wparam: WPARAM, lparam: LPARAM) -> Event {
    // v4: lParam の下位がイベント、上位がアイコンの番号。wParam が座標
    let code = (lparam.0 & 0xFFFF) as u32;
    if code == NIN_SELECT || code == NIN_SELECT | NINF_KEY {
        Event::Select
    } else if code == WM_CONTEXTMENU {
        Event::ContextMenu {
            x: (wparam.0 & 0xFFFF) as u16 as i16 as i32,
            y: ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
        }
    } else if code == NIN_BALLOONUSERCLICK {
        Event::NotificationClick
    } else {
        Event::Other
    }
}
