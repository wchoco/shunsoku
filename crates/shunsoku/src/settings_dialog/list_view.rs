//! タブの一覧（ListView、1 行だけ選べる詳細表示）を扱う、タブに共通の操作。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    LIST_VIEW_ITEM_STATE_FLAGS, LVIF_STATE, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW,
    LVM_ENSUREVISIBLE, LVM_GETNEXTITEM, LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVNI_SELECTED,
};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
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
