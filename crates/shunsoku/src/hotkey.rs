//! ホットキー。メッセージ専用ウィンドウに `RegisterHotKey` で登録し、`WM_HOTKEY` で受ける。
//!
//! 登録に失敗したホットキー（他のアプリが先に取っている、OS が予約しているなど）はそれだけを無効にし、
//! ほかは生かす。失敗したものは呼び出し側がまとめて 1 回だけ通知する。

use shunsoku_core::keys::Chord;
use shunsoku_core::settings::{HotkeyAction, Hotkeys};
use windows::Win32::Foundation::{HWND, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};

/// 登録したホットキー。drop で登録を外す。
pub struct Registered {
    hwnd: HWND,
    ids: Vec<(i32, HotkeyAction)>,
}

/// 登録できなかったホットキー。
pub struct Failed {
    pub action: HotkeyAction,
    pub chord: Chord,
    pub error: windows::core::Error,
}

/// `hotkeys` のうち空でないものを `hwnd` に登録する。`hwnd` を作ったスレッドで呼ぶこと。
pub fn register(hwnd: HWND, hotkeys: &Hotkeys) -> (Registered, Vec<Failed>) {
    let mut registered = Registered {
        hwnd,
        ids: Vec::new(),
    };
    let mut failed = Vec::new();
    for (i, action) in HotkeyAction::ALL.into_iter().enumerate() {
        let Some(chord) = hotkeys.get(action) else {
            continue;
        };
        // 0 は使わない（id の範囲は 0x0000〜0xBFFF）
        let id = i as i32 + 1;
        // SAFETY: 自分のスレッドのウィンドウに登録する
        let result =
            unsafe { RegisterHotKey(Some(hwnd), id, modifiers(chord), u32::from(chord.key.vk())) };
        match result {
            Ok(()) => registered.ids.push((id, action)),
            Err(error) => failed.push(Failed {
                action,
                chord,
                error,
            }),
        }
    }
    (registered, failed)
}

impl Registered {
    /// `WM_HOTKEY` の wParam（登録した id）から、呼び出すものを引く。
    pub fn action(&self, wparam: WPARAM) -> Option<HotkeyAction> {
        self.ids
            .iter()
            .find(|&&(id, _)| usize::try_from(id) == Ok(wparam.0))
            .map(|&(_, action)| action)
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        for &(id, _) in &self.ids {
            // SAFETY: 自分で登録したものを外す
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
            }
        }
    }
}

/// 押しっぱなしで繰り返し届かないように `MOD_NOREPEAT` を付ける。
fn modifiers(chord: Chord) -> HOT_KEY_MODIFIERS {
    let m = chord.modifiers;
    let mut mods = MOD_NOREPEAT;
    for (on, flag) in [
        (m.win, MOD_WIN),
        (m.ctrl, MOD_CONTROL),
        (m.alt, MOD_ALT),
        (m.shift, MOD_SHIFT),
    ] {
        if on {
            mods |= flag;
        }
    }
    mods
}
