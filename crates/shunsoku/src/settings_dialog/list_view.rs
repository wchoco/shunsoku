//! タブの一覧（ListView、1 行だけ選べる詳細表示）を扱う、タブに共通の操作。

use shunsoku_core::settings::columns::column_widths;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::{
    LIST_VIEW_ITEM_STATE_FLAGS, LVCF_TEXT, LVCOLUMNW, LVIF_STATE, LVIS_FOCUSED, LVIS_SELECTED,
    LVITEMW, LVM_ENSUREVISIBLE, LVM_GETCOLUMNW, LVM_GETCOLUMNWIDTH, LVM_GETNEXTITEM,
    LVM_GETSTRINGWIDTHW, LVM_SETCOLUMNWIDTH, LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVNI_SELECTED,
    LVSCW_AUTOSIZE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, SendMessageW, WM_SETREDRAW};
use windows::core::PWSTR;

use crate::wide::to_wide;

/// 一覧 `list` で選んでいる行。
pub fn selected(list: HWND) -> Option<usize> {
    // SAFETY: 自分のダイアログの一覧へのメッセージ
    let index = unsafe {
        SendMessageW(
            list,
            LVM_GETNEXTITEM,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(LVNI_SELECTED as isize)),
        )
    }
    .0;
    usize::try_from(index).ok()
}

/// 一覧 `list` の行 `index` を選んで見えるようにする。選び直しの通知（`LVN_ITEMCHANGED`）が届く。
pub fn select(list: HWND, index: usize) {
    let state = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
    let entry = LVITEMW {
        mask: LVIF_STATE,
        state,
        stateMask: state,
        ..Default::default()
    };
    // SAFETY: 自分のダイアログの一覧へのメッセージ。構造体は呼び出しの間生きている
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            Some(WPARAM(index)),
            Some(LPARAM(&entry as *const LVITEMW as isize)),
        );
        SendMessageW(
            list,
            LVM_ENSUREVISIBLE,
            Some(WPARAM(index)),
            Some(LPARAM(0)),
        );
    }
}

/// 一覧 `list` の `row` 行 `column` 列に `value` を入れる。
pub fn set_cell(list: HWND, row: usize, column: i32, value: &str) {
    let mut value = to_wide(value);
    let entry = LVITEMW {
        iSubItem: column,
        pszText: PWSTR(value.as_mut_ptr()),
        ..Default::default()
    };
    // SAFETY: 文字列と構造体は呼び出しの間生きている（一覧が写す）
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMTEXTW,
            Some(WPARAM(row)),
            Some(LPARAM(&entry as *const LVITEMW as isize)),
        );
    }
}

/// 見出しの文言の左右の余白の合計（96 DPI のピクセル）。
const HEADER_PADDING: i32 = shunsoku_core::app_window::LIST_HEADER_PADDING;

/// 一覧 `list` の列の幅を中身（見出しと全部の行の文言）に合わせ、余りは最後の列に足す
/// （`columns::column_widths`）。一覧に入りきらなければ略さずに横にスクロールさせる。
///
/// - `minimum` は列ごとの最小の幅（ピクセル。文言でなく絵を描く列のため）。足りない列は 0 とみなす。
/// - `reserve` は一覧の中の幅から先に空けておく幅（ピクセル。後から出る縦のスクロールバーの分など）。
pub fn fit_columns(list: HWND, columns: usize, minimum: &[i32], reserve: i32) {
    let mut client = RECT::default();
    // SAFETY: 自分のダイアログの一覧へのメッセージ。バッファと構造体は呼び出しの間生きている
    let needed: Vec<i32> = unsafe {
        let _ = GetClientRect(list, &mut client);
        let padding = HEADER_PADDING * GetDpiForWindow(list) as i32 / 96;
        SendMessageW(list, WM_SETREDRAW, Some(WPARAM(0)), None);
        (0..columns)
            .map(|i| {
                // 行の文言の幅は一覧に測らせる（列の余白込み）
                SendMessageW(
                    list,
                    LVM_SETCOLUMNWIDTH,
                    Some(WPARAM(i)),
                    Some(LPARAM(LVSCW_AUTOSIZE as isize)),
                );
                let rows = SendMessageW(list, LVM_GETCOLUMNWIDTH, Some(WPARAM(i)), None).0 as i32;
                let mut label = [0u16; 64];
                let mut column = LVCOLUMNW {
                    mask: LVCF_TEXT,
                    pszText: PWSTR(label.as_mut_ptr()),
                    cchTextMax: label.len() as i32,
                    ..Default::default()
                };
                SendMessageW(
                    list,
                    LVM_GETCOLUMNW,
                    Some(WPARAM(i)),
                    Some(LPARAM(&mut column as *mut LVCOLUMNW as isize)),
                );
                let header = SendMessageW(
                    list,
                    LVM_GETSTRINGWIDTHW,
                    None,
                    Some(LPARAM(label.as_ptr() as isize)),
                )
                .0 as i32
                    + padding;
                rows.max(header).max(minimum.get(i).copied().unwrap_or(0))
            })
            .collect()
    };
    let widths = column_widths(&needed, client.right - client.left - reserve);
    // SAFETY: 自分のダイアログの一覧へのメッセージ
    unsafe {
        for (i, width) in widths.into_iter().enumerate() {
            SendMessageW(
                list,
                LVM_SETCOLUMNWIDTH,
                Some(WPARAM(i)),
                Some(LPARAM(width as isize)),
            );
        }
        SendMessageW(list, WM_SETREDRAW, Some(WPARAM(1)), None);
        let _ = InvalidateRect(Some(list), None, true);
    }
}
