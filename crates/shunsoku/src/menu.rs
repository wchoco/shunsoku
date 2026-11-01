//! ポップアップ メニュー（トレイのメニューとスクラップの右クリックメニュー）。

use std::borrow::Cow;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MENU_ITEM_FLAGS, MF_CHECKED, MF_GRAYED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW, SetForegroundWindow, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{PCWSTR, Result};

use crate::wide::to_wide;

/// メニューの 1 行。
pub enum Entry<'a> {
    /// `id` は 0 以外。`label` に `\t` を挟むと、その後ろが右寄せ（ショートカットの欄）になる。
    Item {
        id: u32,
        label: Cow<'a, str>,
        enabled: bool,
        /// 左にチェックを付ける。
        checked: bool,
    },
    /// 子メニュー。選ばれた子の `id` が `show` から返る。
    Submenu {
        label: Cow<'a, str>,
        enabled: bool,
        items: Vec<Entry<'a>>,
    },
    Separator,
}

/// メニューを出して、選ばれた項目の `id` を返す。モーダルなので、呼ぶ間はアプリの状態を借りないこと。
pub fn show(owner: HWND, entries: &[Entry], x: i32, y: i32) -> Option<u32> {
    // SAFETY: メニューはこの関数の中で作って壊す（子メニューは親を壊すと一緒に壊れる）
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        if let Err(e) = append(menu, entries) {
            log::warn!("could not build a menu: {e}");
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

/// `menu` に `entries` を足す。子メニューは作って `menu` に付ける（付けたら `menu` が持つ）。
///
/// # Safety
/// `menu` は生きているメニュー。
unsafe fn append(menu: HMENU, entries: &[Entry]) -> Result<()> {
    let grayed = |enabled: bool| {
        if enabled {
            MENU_ITEM_FLAGS(0)
        } else {
            MF_GRAYED
        }
    };
    for entry in entries {
        // SAFETY: 文字列は AppendMenuW の間生きている。子メニューは付けるまでここで持ち、付けられなければ壊す
        unsafe {
            match entry {
                Entry::Item {
                    id,
                    label,
                    enabled,
                    checked,
                } => {
                    let label = to_wide(label);
                    let mut flags = MF_STRING | grayed(*enabled);
                    if *checked {
                        flags |= MF_CHECKED;
                    }
                    AppendMenuW(menu, flags, *id as usize, PCWSTR(label.as_ptr()))?;
                }
                Entry::Submenu {
                    label,
                    enabled,
                    items,
                } => {
                    let sub = CreatePopupMenu()?;
                    let label = to_wide(label);
                    let built = append(sub, items).and_then(|()| {
                        AppendMenuW(
                            menu,
                            MF_POPUP | MF_STRING | grayed(*enabled),
                            sub.0 as usize,
                            PCWSTR(label.as_ptr()),
                        )
                    });
                    if let Err(e) = built {
                        let _ = DestroyMenu(sub);
                        return Err(e);
                    }
                }
                Entry::Separator => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
                }
            }
        }
    }
    Ok(())
}
