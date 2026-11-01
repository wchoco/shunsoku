//! ポップアップ メニュー（トレイのメニューとスクラップの右クリックメニュー）。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW,
    SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::PCWSTR;

use crate::wide::to_wide;

/// メニューの 1 行。
pub enum Entry<'a> {
    /// `id` は 0 以外。`label` に `\t` を挟むと、その後ろが右寄せ（ショートカットの欄）になる。
    Item {
        id: u32,
        label: &'a str,
        enabled: bool,
    },
    Separator,
}

/// メニューを出して、選ばれた項目の `id` を返す。モーダルなので、呼ぶ間はアプリの状態を借りないこと。
pub fn show(owner: HWND, entries: &[Entry], x: i32, y: i32) -> Option<u32> {
    // SAFETY: メニューはこの関数の中で作って壊す。文字列は AppendMenuW の間生きている
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for entry in entries {
            match entry {
                Entry::Item { id, label, enabled } => {
                    let label = to_wide(label);
                    let flags = if *enabled {
                        MF_STRING
                    } else {
                        MF_STRING | MF_GRAYED
                    };
                    let _ = AppendMenuW(menu, flags, *id as usize, PCWSTR(label.as_ptr()));
                }
                Entry::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
            }
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
