//! 「スクラップ操作」タブ。操作ごとのショートカットとマウスの割り当ての一覧（spec #36 の画面の構成の 4）。
//!
//! - 上に一覧（ListView）、下に選んだ行の欄を置く。行は core の `scrap::ops::binding_rows` から作る。一覧の
//!   列は 操作 / ショートカット / マウス / 印。印は同じ範囲の重なり（適用できない）と、ホットキーとの重なり（警告）。
//! - ショートカットの欄は全体のホットキーと同じ入力欄（`chord_field`）で、`ChordRow` の 1 つ。行を選び直したら、
//!   呼び出し側が `ChordRow::item` をその行の `SettingsItem::Shortcut` に差し替える（`field_id` / `selected`）。
//!   上書きのない間は既定の一覧を灰色で表示だけする（`chord_field::set_placeholder`）。引数のある操作（左ドラッグの
//!   移動）は欄を使えなくする。
//! - マウスは候補（`BindingRow::mouse_choices`）のドロップダウン。先頭は「なし」。移動は左ドラッグだけで変えられない。
//! - 「既定に戻す」で行のショートカットとマウスの上書きを消す（`Editor::reset_binding`）。
//! - 一覧の列と行は、最初の `fill` で入れる（ダイアログのテンプレートでは入れられないため）。
//! - コントロールの ID は、ページの帯の最初からの決まった順（`build` が置く順）。

use shunsoku_core::scrap::ops::{BindingRow, MouseGesture, OPERATIONS, binding_rows};
use shunsoku_core::settings::SettingsItem;
use shunsoku_core::settings::editor::{Editor, ShortcutValue};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    LIST_VIEW_ITEM_STATE_FLAGS, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_STATE, LVIF_TEXT,
    LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_GETITEMCOUNT, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW,
    LVM_INSERTITEMW, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMSTATE, LVM_SETITEMTEXTW,
    LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_NOSORTHEADER, LVS_REPORT,
    LVS_SHOWSELALWAYS, LVS_SINGLESEL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CBS_DROPDOWNLIST,
    MapDialogRect, SendMessageW, SetDlgItemTextW, WINDOW_STYLE, WS_BORDER, WS_TABSTOP, WS_VSCROLL,
};
use windows::core::{PCWSTR, PWSTR};

use super::{Class, PAGE_WIDTH, Page, PageBuilder, Text, chord_field, item, page_first_id};
use crate::wide::to_wide;

/// ページの帯の最初からの位置（`build` が置く順）。
const LIST: i32 = 0;
const FIELD: i32 = 2;
const RESET: i32 = 4;
const MOUSE: i32 = 6;
const MOUSE_MARK: i32 = 7;

/// 一覧の高さと、下の欄の位置（ページの中のダイアログ単位）。
const LIST_HEIGHT: i16 = 80;
const SHORTCUT_Y: i16 = LIST_HEIGHT + 6;
const MOUSE_Y: i16 = SHORTCUT_Y + 36;
const LABEL_WIDTH: i16 = 55;
const FIELD_X: i16 = LABEL_WIDTH + 3;
const FIELD_WIDTH: i16 = 110;
const RESET_WIDTH: i16 = 60;
/// 一覧の列の幅（ダイアログ単位）。最後の列（印）は残り。
const COLUMNS: [(&str, i16); 4] = [
    ("settings_gui.scrap_ops.operation", 80),
    ("settings_gui.scrap_ops.shortcut", 95),
    ("settings_gui.scrap_ops.mouse", 55),
    ("settings_gui.scrap_ops.mark", 50),
];

/// タブのコントロールを置く。
pub(super) fn build(b: &mut PageBuilder) {
    let first = b.next_id;
    let style = |s: u32| WINDOW_STYLE(s);
    b.control(
        Class::ListView,
        "",
        WS_TABSTOP
            | WS_BORDER
            | style(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER),
        0,
        0,
        PAGE_WIDTH,
        LIST_HEIGHT,
    );
    b.label(
        "settings_gui.scrap_ops.shortcut_label",
        0,
        SHORTCUT_Y + 2,
        LABEL_WIDTH,
        10,
    );
    let first_row = binding_rows(OPERATIONS)[0];
    b.chord_field(
        SettingsItem::Shortcut(first_row.op.id.to_owned()),
        FIELD_X,
        SHORTCUT_Y,
        FIELD_WIDTH,
    );
    let reset_text = (b.text)("settings_gui.scrap_ops.reset");
    let reset = b.control(
        Class::Button,
        reset_text,
        WS_TABSTOP | style(BS_PUSHBUTTON as u32),
        PAGE_WIDTH - RESET_WIDTH,
        SHORTCUT_Y,
        RESET_WIDTH,
        14,
    );
    b.label(
        "settings_gui.scrap_ops.mouse_label",
        0,
        MOUSE_Y + 2,
        LABEL_WIDTH,
        10,
    );
    // ドロップダウンの高さは開いたときの一覧の高さ
    let mouse = b.control(
        Class::ComboBox,
        "",
        WS_TABSTOP | WS_VSCROLL | style(CBS_DROPDOWNLIST as u32),
        FIELD_X,
        MOUSE_Y,
        FIELD_WIDTH,
        100,
    );
    let mark_x = FIELD_X + FIELD_WIDTH + 5;
    let mouse_mark = b.control(
        Class::Static,
        "",
        WINDOW_STYLE(0),
        mark_x,
        MOUSE_Y + 2,
        PAGE_WIDTH - mark_x,
        10,
    );
    debug_assert_eq!(
        (reset - first, mouse - first, mouse_mark - first),
        (RESET, MOUSE, MOUSE_MARK)
    );
}

fn id(offset: i32) -> i32 {
    page_first_id(Page::ScrapOps) + offset
}

/// 一覧の ID。
pub fn list_id() -> i32 {
    id(LIST)
}

/// ショートカットの欄の ID。
pub fn field_id() -> i32 {
    id(FIELD)
}

/// 「既定に戻す」の ID。
pub fn reset_id() -> i32 {
    id(RESET)
}

/// マウスのドロップダウンの ID。
pub fn mouse_id() -> i32 {
    id(MOUSE)
}

/// 一覧で選んでいる行。
pub fn selected(hwnd: HWND) -> Option<BindingRow> {
    let list = item(hwnd, list_id())?;
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
    binding_rows(OPERATIONS)
        .get(usize::try_from(index).ok()?)
        .copied()
}

/// ドロップダウンで選んだマウスの割り当て（None は「なし」）。選んだ行のマウスを変えられなければ None。
pub fn chosen_mouse(hwnd: HWND) -> Option<(BindingRow, Option<MouseGesture>)> {
    let row = selected(hwnd)?;
    if row.op.drag_only {
        return None;
    }
    let combo = item(hwnd, mouse_id())?;
    // SAFETY: 自分のダイアログのドロップダウンへのメッセージ
    let index = unsafe { SendMessageW(combo, CB_GETCURSEL, None, None) }.0;
    let gesture = match usize::try_from(index).ok()? {
        0 => None,
        i => Some(*row.mouse_choices().get(i - 1)?),
    };
    Some((row, gesture))
}

/// 写し `editor` を一覧と選んだ行の欄に出す。ショートカットの欄の値と印は `settings_dialog::fill` が出す。
pub(super) fn fill(hwnd: HWND, editor: &Editor, text: Text) {
    let Some(list) = item(hwnd, list_id()) else {
        return;
    };
    let rows = binding_rows(OPERATIONS);
    // SAFETY: 自分のダイアログの一覧へのメッセージ
    if unsafe { SendMessageW(list, LVM_GETITEMCOUNT, None, None) }.0 == 0 {
        init_list(hwnd, list, &rows, text);
    }
    for (i, row) in rows.iter().enumerate() {
        set_cell(list, i, 1, &shortcut_text(editor, row, text));
        set_cell(list, i, 2, &mouse_text(editor.mouse(row), text));
        let mark = editor
            .mark(&SettingsItem::Shortcut(row.op.id.to_owned()))
            .or_else(|| editor.mark(&SettingsItem::Mouse(row.op.id.to_owned())))
            .map_or("", |m| text(m.label_key()));
        set_cell(list, i, 3, mark);
    }

    let Some(row) = selected(hwnd) else {
        return;
    };
    if let Some(field) = item(hwnd, field_id()) {
        let placeholder = match editor.shortcut(&row) {
            ShortcutValue::Defaults(defaults) => join(defaults),
            ShortcutValue::Set(_) => String::new(),
        };
        chord_field::set_placeholder(field, &placeholder);
        enable(field, row.default_shortcut.is_some());
    }
    if let Some(combo) = item(hwnd, mouse_id()) {
        let choices = row.mouse_choices();
        let with_none = !row.op.drag_only;
        let current = editor.mouse(&row);
        // SAFETY: 自分のダイアログのドロップダウンへのメッセージ。文字列は呼び出しの間生きている
        unsafe {
            SendMessageW(combo, CB_RESETCONTENT, None, None);
            let labels = with_none
                .then(|| text("settings_gui.scrap_ops.none"))
                .into_iter()
                .chain(choices.iter().map(|g| text(g.label_key())));
            for label in labels {
                let label = to_wide(label);
                SendMessageW(
                    combo,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(label.as_ptr() as isize)),
                );
            }
            let position = choices.iter().position(|&g| Some(g) == current);
            let index = match (with_none, position) {
                (true, Some(p)) => p + 1,
                (true, None) => 0,
                (false, p) => p.unwrap_or(0),
            };
            SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(index)), None);
        }
        enable(combo, with_none);
    }
    if let Some(reset) = item(hwnd, reset_id()) {
        enable(reset, editor.is_overridden(&row));
    }
    let mouse_mark = editor
        .mark(&SettingsItem::Mouse(row.op.id.to_owned()))
        .map_or("", |m| text(m.label_key()));
    let mouse_mark = to_wide(mouse_mark);
    // SAFETY: 文字列は呼び出しの間生きている
    unsafe {
        let _ = SetDlgItemTextW(hwnd, id(MOUSE_MARK), PCWSTR(mouse_mark.as_ptr()));
    }
}

/// 一覧の列と行を入れ、最初の行を選ぶ。
fn init_list(hwnd: HWND, list: HWND, rows: &[BindingRow], text: Text) {
    // SAFETY: 自分のダイアログの一覧へのメッセージ。文字列と構造体は呼び出しの間生きている（一覧が写す）
    unsafe {
        let ex = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            Some(WPARAM(ex as usize)),
            Some(LPARAM(ex as isize)),
        );
        for (i, (key, width)) in COLUMNS.iter().enumerate() {
            let mut label = to_wide(text(key));
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: dialog_units(hwnd, *width),
                pszText: PWSTR(label.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(i)),
                Some(LPARAM(&column as *const LVCOLUMNW as isize)),
            );
        }
        for (i, row) in rows.iter().enumerate() {
            let mut label = to_wide(text(row.op.label_key));
            let entry = LVITEMW {
                mask: LVIF_TEXT,
                iItem: i as i32,
                pszText: PWSTR(label.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                list,
                LVM_INSERTITEMW,
                None,
                Some(LPARAM(&entry as *const LVITEMW as isize)),
            );
        }
        let state = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
        let entry = LVITEMW {
            mask: LVIF_STATE,
            state,
            stateMask: state,
            ..Default::default()
        };
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            Some(WPARAM(0)),
            Some(LPARAM(&entry as *const LVITEMW as isize)),
        );
    }
}

/// 一覧の `row` 行 `column` 列に `value` を入れる。
fn set_cell(list: HWND, row: usize, column: i32, value: &str) {
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

/// 一覧のショートカットの列の文言。
fn shortcut_text(editor: &Editor, row: &BindingRow, text: Text) -> String {
    if row.default_shortcut.is_none() {
        return String::from("—");
    }
    match editor.shortcut(row) {
        ShortcutValue::Set(Some(chord)) => chord.to_string(),
        ShortcutValue::Defaults(defaults) if !defaults.is_empty() => join(defaults),
        ShortcutValue::Set(None) | ShortcutValue::Defaults(_) => {
            text("settings_gui.scrap_ops.none").to_owned()
        }
    }
}

fn mouse_text(gesture: Option<MouseGesture>, text: Text) -> String {
    text(gesture.map_or("settings_gui.scrap_ops.none", MouseGesture::label_key)).to_owned()
}

/// 既定の一覧の表示。
fn join(chords: &[shunsoku_core::keys::Chord]) -> String {
    chords
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" / ")
}

fn enable(hwnd: HWND, enabled: bool) {
    // SAFETY: 自分のダイアログのコントロール
    unsafe {
        let _ = EnableWindow(hwnd, enabled);
    }
}

/// ダイアログ単位の幅 `width` をピクセルにする。
fn dialog_units(hwnd: HWND, width: i16) -> i32 {
    let mut rect = RECT {
        right: i32::from(width),
        ..Default::default()
    };
    // SAFETY: rect は呼び出しの間生きている
    unsafe {
        let _ = MapDialogRect(hwnd, &mut rect);
    }
    rect.right
}
