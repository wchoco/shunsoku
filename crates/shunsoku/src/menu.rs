//! ポップアップ メニュー（トレイのメニューとスクラップの右クリックメニュー）。
//!
//! 画像を付けた項目（`Entry::Picture`、ごみ箱のサムネイル）は、画像を 32bit の premultiplied な DIB にして
//! `MIIM_BITMAP` で付ける。メニューはビットマップを持たないので、`show` がメニューを壊したあとで消す。

use std::borrow::Cow;
use std::ffi::c_void;

use shunsoku_core::image::Image;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MENU_ITEM_FLAGS, MENUITEMINFOW, MF_CHECKED,
    MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MIIM_BITMAP, PostMessageW, SetForegroundWindow,
    SetMenuItemInfoW, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
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
    /// 左に `image` を付けた項目（ごみ箱の一覧のサムネイル）。`id` は 0 以外。画像を付けられなければ文言だけ出す。
    Picture {
        id: u32,
        label: Cow<'a, str>,
        image: &'a Image,
    },
    Separator,
}

impl<'a> Entry<'a> {
    /// チェックを付けない項目。
    pub fn item(id: u32, label: impl Into<Cow<'a, str>>, enabled: bool) -> Entry<'a> {
        Entry::Item {
            id,
            label: label.into(),
            enabled,
            checked: false,
        }
    }
}

/// メニューを出して、選ばれた項目の `id` を返す。モーダルなので、呼ぶ間はアプリの状態を借りないこと。
pub fn show(owner: HWND, entries: &[Entry], x: i32, y: i32) -> Option<u32> {
    // SAFETY: メニューはこの関数の中で作って壊す（子メニューは親を壊すと一緒に壊れる）
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let mut bitmaps = Vec::new();
        if let Err(e) = append(menu, entries, &mut bitmaps) {
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
        for bitmap in bitmaps {
            let _ = DeleteObject(bitmap.into());
        }
        (chosen.0 != 0).then_some(chosen.0 as u32)
    }
}

/// `menu` に `entries` を足す。子メニューは作って `menu` に付ける（付けたら `menu` が持つ）。項目に付けた
/// ビットマップは `bitmaps` に足す（メニューを壊したあとで呼び出し側が消す）。
///
/// # Safety
/// `menu` は生きているメニュー。
unsafe fn append(menu: HMENU, entries: &[Entry], bitmaps: &mut Vec<HBITMAP>) -> Result<()> {
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
                    let built = append(sub, items, bitmaps).and_then(|()| {
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
                Entry::Picture { id, label, image } => {
                    let label = to_wide(label);
                    AppendMenuW(menu, MF_STRING, *id as usize, PCWSTR(label.as_ptr()))?;
                    match bitmap_of(image) {
                        Ok(bitmap) => {
                            bitmaps.push(bitmap);
                            let info = MENUITEMINFOW {
                                cbSize: size_of::<MENUITEMINFOW>() as u32,
                                fMask: MIIM_BITMAP,
                                hbmpItem: bitmap,
                                ..Default::default()
                            };
                            if let Err(e) = SetMenuItemInfoW(menu, *id, false, &info) {
                                log::warn!("could not put a picture on a menu item: {e}");
                            }
                        }
                        Err(e) => log::warn!("could not make a menu picture: {e}"),
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

/// `image` を、メニューに付けられる 32bit の premultiplied な DIB にする。消すのは呼び出し側。
fn bitmap_of(image: &Image) -> Result<HBITMAP> {
    let size = image.size();
    if size.width == 0 || size.height == 0 {
        return Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_INVALIDARG,
        ));
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size.width as i32,
            // 正の高さ（下の行から並べる）にする。メニューは上下逆の DIB を正しく描かないことがある
            biHeight: size.height as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let pixels = image.to_premultiplied();
    let stride = size.width as usize * 4;
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: bits は CreateDIBSection が返した、stride * height バイトの DIB。下の行から写す
    unsafe {
        let bitmap = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        if bits.is_null() {
            let _ = DeleteObject(bitmap.into());
            return Err(windows::core::Error::from_thread());
        }
        let dest = std::slice::from_raw_parts_mut(bits.cast::<u8>(), pixels.len());
        for (row, line) in pixels.chunks_exact(stride).rev().enumerate() {
            dest[row * stride..(row + 1) * stride].copy_from_slice(line);
        }
        Ok(bitmap)
    }
}
