//! タスクトレイのアイコンとメニュー。`NOTIFYICON_VERSION_4` で登録する。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NIN_SELECT,
    NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HICON, MF_GRAYED, MF_STRING, PostMessageW,
    SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    WM_CONTEXTMENU, WM_NULL,
};
use windows::core::PCWSTR;

use crate::wide::{copy_truncated, to_wide};

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
    Other,
}

/// メニューの項目。
pub struct MenuItem<'a> {
    pub id: u32,
    pub label: &'a str,
    pub enabled: bool,
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
    } else {
        Event::Other
    }
}

/// メニューを出して、選ばれた項目の `id` を返す。モーダルなので、呼ぶ間はアプリの状態を借りないこと。
pub fn show_menu(owner: HWND, items: &[MenuItem], x: i32, y: i32) -> Option<u32> {
    // SAFETY: メニューはこの関数の中で作って壊す。文字列は AppendMenuW の間生きている
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for item in items {
            let label = to_wide(item.label);
            let flags = if item.enabled {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            let _ = AppendMenuW(menu, flags, item.id as usize, PCWSTR(label.as_ptr()));
        }
        // 前面にしないと、メニューの外をクリックしても閉じない
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            x,
            y,
            None,
            owner,
            None,
        );
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        (chosen.0 != 0).then_some(chosen.0 as u32)
    }
}
