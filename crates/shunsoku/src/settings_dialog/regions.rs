//! 「定型領域」タブ。左に定型領域の一覧、右に選んだ領域の詳細（spec #36 の画面の構成の 2）。
//!
//! - 一覧（ListView）の列は 名前 / ホットキー / 範囲 / 印。並びは写しの `Settings::regions` の順で、トレイの
//!   「定型領域」サブメニューの順になる。下の「追加」「複製」「削除」「上へ」「下へ」は呼び出し側が
//!   `Editor` の `add_region` などで写しを変え、`select` で移った行を選ぶ。
//! - 詳細の欄は `fields` の `Field::Region(i, …)` と `TemplateOf::Region(i)`、ホットキーは `ChordRow` の
//!   `SettingsItem::RegionHotkey(i)`。行を選び直したら呼び出し側が `Fields::bind_region` と `ChordRow::item` を
//!   その行に差し替える（`hotkey_id`）。
//! - 範囲の欄の下に、登録したモニタがない・範囲がはみ出す警告を出す（core の `monitors::locate_by_id`）。モニタの
//!   一覧は `refresh_monitors` で集め直す（開いたとき、行を選び直したとき、画面の構成が変わったとき）。
//! - 一覧の行の数は `sync_list` で写しに揃える。揃えて選んでいる行がなくなったら、詳細が指していた位置の行を選ぶ。
//!   呼び出し側は、欄を出す（`fill`）前に `sync_list` を呼ぶ。`sync_list` の中で行を選んで届く通知では欄を結び
//!   直すだけにする（`is_syncing`）。
//! - 「範囲を選び直す」（`reselect_id`）と「試しに撮る」（`test_capture_id`）は、呼び出し側が範囲選択の画面を
//!   開く・写しの値で撮る。
//! - コントロールの ID は、ページの帯の最初からの決まった順（`build` が置く順）。

use std::cell::{Cell, RefCell};

use shunsoku_core::geometry::{Point, Rect, Size};
use shunsoku_core::monitors::{self, LocateError, Monitor};
use shunsoku_core::settings::SettingsItem;
use shunsoku_core::settings::editor::Editor;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    EM_SETCUEBANNER, LIST_VIEW_ITEM_STATE_FLAGS, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_STATE,
    LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETEALLITEMS, LVM_ENSUREVISIBLE,
    LVM_GETITEMCOUNT, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETCOLUMNWIDTH,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVNI_SELECTED,
    LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_NOSORTHEADER, LVS_REPORT, LVS_SHOWSELALWAYS,
    LVS_SINGLESEL,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, GetClientRect, SM_CXVSCROLL, SendMessageW, SetDlgItemTextW, WINDOW_STYLE,
    WS_BORDER, WS_TABSTOP,
};
use windows::core::{PCWSTR, PWSTR};

use super::fields::{Action, Field, RegionKey, TemplateOf};
use super::{Class, PAGE_WIDTH, Page, PageBuilder, SS_LEFT, Text, item, page_first_id};
use crate::wide::to_wide;
use crate::{monitor_list, save};

/// ページの帯の最初からの位置（`build` が置く順）。
const LIST: i32 = 0;
const ADD: i32 = 1;
const DUPLICATE: i32 = 2;
const REMOVE: i32 = 3;
const UP: i32 = 4;
const DOWN: i32 = 5;
/// 詳細の最初（名前の見出し）。ここから `DETAIL_END` の手前までを、選んだ行がなければ使えなくする。
const DETAIL_FIRST: i32 = 6;
const NAME: i32 = 7;
const NAME_MARK: i32 = 8;
const HOTKEY: i32 = 10;
const MONITOR: i32 = 13;
const RESELECT: i32 = 20;
const TEST_CAPTURE: i32 = 27;
const WARNING: i32 = 28;
const SAVE_DIR: i32 = 30;
const DETAIL_END: i32 = 39;

/// 一覧の幅と高さ、その下のボタン（ページの中のダイアログ単位）。
const LIST_WIDTH: i16 = 112;
const LIST_HEIGHT: i16 = 152;
const LIST_BUTTON_GAP: i16 = 2;
/// 詳細の左端と幅、見出しの幅と欄の左端。
const DETAIL_X: i16 = LIST_WIDTH + 6;
const DETAIL_WIDTH: i16 = PAGE_WIDTH - DETAIL_X;
const LABEL_WIDTH: i16 = 44;
const FIELD_X: i16 = DETAIL_X + LABEL_WIDTH + 2;
const FIELD_WIDTH: i16 = PAGE_WIDTH - FIELD_X;
/// 範囲の数の欄（上下のボタンを除く幅）と、横のボタン。
const NUMBER_WIDTH: i16 = 32;
const RANGE_BUTTON_X: i16 = DETAIL_X + 126;
/// 一覧の列と、幅の割合。列の幅は一覧の中の幅（縦のスクロールバーの分を除く）をこの割合で分ける
/// （`fit_columns`）。入りきらない文言は一覧が「…」で略す。
const COLUMNS: [(&str, i32); 4] = [
    ("settings_gui.regions.name", 3),
    ("settings_gui.regions.hotkey", 3),
    ("settings_gui.regions.range", 4),
    ("settings_gui.scrap_ops.mark", 2),
];

/// 「追加」で足す定型領域の大きさ（物理ピクセル。モニタより大きければモニタに収める）。
const NEW_REGION_SIZE: Size = Size::new(640, 480);

/// タブのコントロールを置く。
pub(super) fn build(b: &mut PageBuilder) {
    let first = b.next_id;
    let style = |s: u32| WINDOW_STYLE(s);
    let push = WS_TABSTOP | style(BS_PUSHBUTTON as u32);
    let list = b.control(
        Class::ListView,
        "",
        WS_TABSTOP
            | WS_BORDER
            | style(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER),
        0,
        0,
        LIST_WIDTH,
        LIST_HEIGHT,
    );
    let mut list_buttons = Vec::new();
    let rows: [&[&'static str]; 2] = [
        &[
            "settings_gui.regions.add",
            "settings_gui.regions.duplicate",
            "settings_gui.regions.remove",
        ],
        &["settings_gui.regions.up", "settings_gui.regions.down"],
    ];
    for (row, keys) in rows.iter().enumerate() {
        let n = keys.len() as i16;
        let width = (LIST_WIDTH - (n - 1) * LIST_BUTTON_GAP) / n;
        let y = LIST_HEIGHT + 4 + row as i16 * 16;
        for (i, key) in keys.iter().enumerate() {
            let x = i as i16 * (width + LIST_BUTTON_GAP);
            let text = (b.text)(key);
            list_buttons.push(b.control(Class::Button, text, push, x, y, width, 14));
        }
    }

    // 名前とその印
    b.label(
        "settings_gui.regions.name_label",
        DETAIL_X,
        2,
        LABEL_WIDTH,
        10,
    );
    let name = b.text(Field::Region(0, RegionKey::Name), FIELD_X, 0, FIELD_WIDTH);
    let name_mark = b.control(Class::Static, "", SS_LEFT, FIELD_X, 16, FIELD_WIDTH, 10);
    // ホットキー（印は欄の下）
    b.label(
        "settings_gui.regions.hotkey_label",
        DETAIL_X,
        30,
        LABEL_WIDTH,
        10,
    );
    let hotkey = b.next_id;
    b.chord_field(SettingsItem::RegionHotkey(0), FIELD_X, 28, 110);
    // 範囲: モニタ、X / Y / 幅 / 高さ、警告
    b.label(
        "settings_gui.regions.monitor_label",
        DETAIL_X,
        62,
        LABEL_WIDTH,
        10,
    );
    let monitor = b.control(Class::Static, "", SS_LEFT, FIELD_X, 62, FIELD_WIDTH, 10);
    let mut range_buttons = Vec::new();
    let range_rows = [
        (
            74,
            [
                ("settings_gui.regions.x", RegionKey::X),
                ("settings_gui.regions.y", RegionKey::Y),
            ],
            "settings_gui.regions.reselect",
        ),
        (
            90,
            [
                ("settings_gui.regions.width", RegionKey::Width),
                ("settings_gui.regions.height", RegionKey::Height),
            ],
            "settings_gui.regions.test_capture",
        ),
    ];
    for (y, numbers, button) in range_rows {
        for (i, (key, region_key)) in numbers.into_iter().enumerate() {
            let x = DETAIL_X + i as i16 * 62;
            b.label(key, x, y + 2, 16, 10);
            b.number(Field::Region(0, region_key), x + 18, y, NUMBER_WIDTH);
        }
        let text = (b.text)(button);
        range_buttons.push(b.control(
            Class::Button,
            text,
            push,
            RANGE_BUTTON_X,
            y,
            PAGE_WIDTH - RANGE_BUTTON_X,
            14,
        ));
    }
    let warning = b.control(Class::Static, "", SS_LEFT, DETAIL_X, 106, DETAIL_WIDTH, 10);
    // 保存先と、その下の「参照…」「フォルダを開く」
    b.label(
        "settings_gui.regions.save_dir_label",
        DETAIL_X,
        120,
        LABEL_WIDTH,
        10,
    );
    let save_dir = Field::Region(0, RegionKey::SaveDir);
    let save_dir_id = b.text(save_dir, FIELD_X, 118, FIELD_WIDTH);
    b.button(
        Action::BrowseFolder(save_dir),
        "settings_gui.browse",
        FIELD_X,
        134,
        50,
    );
    b.button(
        Action::OpenFolder(save_dir),
        "settings_gui.regions.open_folder",
        FIELD_X + 54,
        134,
        64,
    );
    // ファイル名テンプレート（プレビューを含めて高さ 28）
    b.label(
        "settings_gui.regions.template_label",
        DETAIL_X,
        152,
        LABEL_WIDTH,
        10,
    );
    b.template_field(TemplateOf::Region(0), FIELD_X, 150, FIELD_WIDTH);
    // 撮影後の動作
    b.check(
        Field::Region(0, RegionKey::Save),
        "settings_gui.regions.save",
        DETAIL_X,
        180,
        60,
    );
    b.check(
        Field::Region(0, RegionKey::Scrap),
        "settings_gui.regions.scrap",
        DETAIL_X + 64,
        180,
        DETAIL_WIDTH - 64,
    );
    let offsets = |ids: &[i32]| ids.iter().map(|id| id - first).collect::<Vec<_>>();
    debug_assert_eq!(
        offsets(&[list]),
        [LIST],
        "the list must come first in the page band"
    );
    debug_assert_eq!(offsets(&list_buttons), [ADD, DUPLICATE, REMOVE, UP, DOWN]);
    debug_assert_eq!(
        offsets(&[name, name_mark, hotkey, monitor, warning, save_dir_id]),
        [NAME, NAME_MARK, HOTKEY, MONITOR, WARNING, SAVE_DIR]
    );
    debug_assert_eq!(offsets(&range_buttons), [RESELECT, TEST_CAPTURE]);
    debug_assert_eq!(b.next_id - first, DETAIL_END);
}

fn id(offset: i32) -> i32 {
    page_first_id(Page::Regions) + offset
}

/// 一覧の ID。
pub fn list_id() -> i32 {
    id(LIST)
}

/// 一覧の下のボタン。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListButton {
    Add,
    Duplicate,
    Remove,
    Up,
    Down,
}

/// ID `id` が一覧の下のボタンならそれ。
pub fn list_button(id: i32) -> Option<ListButton> {
    let first = page_first_id(Page::Regions);
    match id - first {
        ADD => Some(ListButton::Add),
        DUPLICATE => Some(ListButton::Duplicate),
        REMOVE => Some(ListButton::Remove),
        UP => Some(ListButton::Up),
        DOWN => Some(ListButton::Down),
        _ => None,
    }
}

/// 名前の欄の ID（新しい領域を選んで名前を付けさせるときに、フォーカスを置く）。
pub fn name_id() -> i32 {
    id(NAME)
}

/// ホットキーの欄の ID。行を選び直したら、この `ChordRow` の項目を差し替える。
pub fn hotkey_id() -> i32 {
    id(HOTKEY)
}

/// 「範囲を選び直す」の ID。
pub fn reselect_id() -> i32 {
    id(RESELECT)
}

/// 「試しに撮る」の ID。
pub fn test_capture_id() -> i32 {
    id(TEST_CAPTURE)
}

/// 一覧で選んでいる行。
pub fn selected(hwnd: HWND) -> Option<usize> {
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
    usize::try_from(index).ok()
}

/// 一覧の行 `index` を選んで見えるようにする。選び直しの通知（`LVN_ITEMCHANGED`）が届く。
pub fn select(hwnd: HWND, index: usize) {
    let Some(list) = item(hwnd, list_id()) else {
        return;
    };
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

thread_local! {
    /// 集めたモニタの一覧（`refresh_monitors`）。
    static MONITORS: RefCell<Vec<Monitor>> = const { RefCell::new(Vec::new()) };
    /// `sync_list` の間。
    static SYNCING: Cell<bool> = const { Cell::new(false) };
}

/// モニタの一覧を集め直す。
pub fn refresh_monitors() {
    let list = monitor_list::list();
    MONITORS.with_borrow_mut(|m| *m = list);
}

/// `sync_list` で行を揃えている途中か（届いた選び直しの通知では、欄を結び直すだけにする）。
pub fn is_syncing() -> bool {
    SYNCING.get()
}

/// 「追加」で足す定型領域のモニタと範囲: メインモニタの真ん中の `NEW_REGION_SIZE`。モニタがなければ None。
pub fn new_region_place() -> Option<(String, Rect)> {
    MONITORS.with_borrow(|monitors| {
        let monitor = monitors
            .iter()
            .find(|m| m.primary)
            .or_else(|| monitors.first())?;
        let width = NEW_REGION_SIZE.width.min(monitor.rect.width());
        let height = NEW_REGION_SIZE.height.min(monitor.rect.height());
        let origin = Point::new(
            ((monitor.rect.width() - width) / 2) as i32,
            ((monitor.rect.height() - height) / 2) as i32,
        );
        Some((
            monitor.id.clone(),
            Rect::from_origin_size(origin, Size::new(width, height)),
        ))
    })
}

/// 一覧の行を写し `editor` の定型領域に揃え、各行の文言を出す。選んでいる行がなくなったら、詳細が指していた
/// 位置 `bound`（末尾を越えれば末尾）の行を選ぶ。
pub fn sync_list(hwnd: HWND, editor: &Editor, bound: usize, text: Text) {
    let Some(list) = item(hwnd, list_id()) else {
        return;
    };
    SYNCING.set(true);
    if MONITORS.with_borrow(Vec::is_empty) {
        refresh_monitors();
    }
    let regions = &editor.draft().regions;
    // SAFETY: 自分のダイアログの一覧へのメッセージ
    let count = unsafe { SendMessageW(list, LVM_GETITEMCOUNT, None, None) }.0;
    if count == 0 && column_count(list) == 0 {
        init_columns(list, text);
        fit_columns(hwnd);
    }
    if usize::try_from(count).ok() != Some(regions.len()) {
        // SAFETY: 自分のダイアログの一覧へのメッセージ。構造体は呼び出しの間生きている（一覧が写す）
        unsafe {
            SendMessageW(list, LVM_DELETEALLITEMS, None, None);
            for i in 0..regions.len() {
                let mut empty = [0u16];
                let entry = LVITEMW {
                    mask: LVIF_TEXT,
                    iItem: i as i32,
                    pszText: PWSTR(empty.as_mut_ptr()),
                    ..Default::default()
                };
                SendMessageW(
                    list,
                    LVM_INSERTITEMW,
                    None,
                    Some(LPARAM(&entry as *const LVITEMW as isize)),
                );
            }
        }
    }
    for (i, region) in regions.iter().enumerate() {
        set_cell(list, i, 0, &region.name);
        let hotkey = region.hotkey.map_or_else(
            || text("settings_gui.scrap_ops.none").to_owned(),
            |c| c.to_string(),
        );
        set_cell(list, i, 1, &hotkey);
        set_cell(list, i, 2, &range_text(&region.rect));
        let mark = editor
            .mark(&SettingsItem::RegionName(i))
            .or_else(|| editor.mark(&SettingsItem::RegionHotkey(i)))
            .map_or("", |m| text(m.label_key()));
        set_cell(list, i, 3, mark);
    }
    if selected(hwnd).is_none() && !regions.is_empty() {
        select(hwnd, bound.min(regions.len() - 1));
    }
    SYNCING.set(false);
}

/// 選んだ行の詳細のうち、欄（`fields`）以外を出す: 名前の印、モニタの名前、警告、保存先の既定の表示、ボタンを
/// 押せるか。欄の値は `fields::fill`、ホットキーの欄の値と印は `settings_dialog::fill` が出す。
pub(super) fn fill(hwnd: HWND, editor: &Editor, text: Text) {
    let draft = editor.draft();
    let len = draft.regions.len();
    let selected = selected(hwnd).filter(|&i| i < len);
    for offset in DETAIL_FIRST..DETAIL_END {
        enable(hwnd, id(offset), selected.is_some());
    }
    enable(hwnd, id(DUPLICATE), selected.is_some());
    enable(hwnd, id(REMOVE), selected.is_some());
    enable(hwnd, id(UP), selected.is_some_and(|i| i > 0));
    enable(hwnd, id(DOWN), selected.is_some_and(|i| i + 1 < len));

    let region = selected.map(|i| (i, &draft.regions[i]));
    let name_mark = region
        .and_then(|(i, _)| editor.mark(&SettingsItem::RegionName(i)))
        .map_or("", |m| text(m.label_key()));
    set_text(hwnd, id(NAME_MARK), name_mark);

    let (monitor, warning) = match region {
        Some((_, r)) => MONITORS.with_borrow(|monitors| {
            let name = monitors
                .iter()
                .find(|m| m.id.eq_ignore_ascii_case(&r.monitor))
                .map_or_else(|| r.monitor.clone(), |m| m.name.clone());
            let warning = match monitors::locate_by_id(monitors, &r.monitor, r.rect) {
                Ok(_) => "",
                Err(LocateError::OutOfBounds) => text("settings_gui.regions.out_of_bounds"),
                Err(_) => text("settings_gui.regions.monitor_not_found"),
            };
            (name, warning)
        }),
        None => (String::new(), ""),
    };
    set_text(hwnd, id(MONITOR), &monitor);
    set_text(hwnd, id(WARNING), warning);

    // 保存先が空なら全体の保存先に保存する
    let general = draft
        .general
        .save_dir
        .clone()
        .or_else(|| save::default_dir().ok());
    let cue = general
        .map(|d| {
            format!(
                "{}{}",
                text("settings_gui.general.default_dir"),
                d.display()
            )
        })
        .unwrap_or_default();
    if let Some(control) = item(hwnd, id(SAVE_DIR)) {
        let cue = to_wide(&cue);
        // SAFETY: 文字列は呼び出しの間生きている（コントロールが写す）
        unsafe {
            SendMessageW(
                control,
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(cue.as_ptr() as isize)),
            );
        }
    }
}

/// 一覧の範囲の列の文言（大きさと、モニタの中の左上）。
fn range_text(rect: &Rect) -> String {
    format!(
        "{}×{} ({}, {})",
        rect.width(),
        rect.height(),
        rect.left,
        rect.top
    )
}

/// 一覧の列を入れる。
fn init_columns(list: HWND, text: Text) {
    // SAFETY: 自分のダイアログの一覧へのメッセージ。文字列と構造体は呼び出しの間生きている（一覧が写す）
    unsafe {
        let ex = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            Some(WPARAM(ex as usize)),
            Some(LPARAM(ex as isize)),
        );
        for (i, (key, _)) in COLUMNS.iter().enumerate() {
            let mut label = to_wide(text(key));
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: 0,
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
    }
}

/// 一覧の列の幅を、一覧の中の幅に収まるように決め直す（列を入れたとき、DPI が変わったとき）。縦のスクロールバーは
/// 行が増えると出るので、その分を常に空けておく（出たときに横のスクロールバーが出ないように）。
pub fn fit_columns(hwnd: HWND) {
    let Some(list) = item(hwnd, list_id()) else {
        return;
    };
    let mut client = RECT::default();
    // SAFETY: 自分のダイアログの一覧の大きさを読む
    let scroll_bar = unsafe {
        let _ = GetClientRect(list, &mut client);
        GetSystemMetricsForDpi(SM_CXVSCROLL, GetDpiForWindow(list))
    };
    let widths = column_widths(client.right - client.left - scroll_bar);
    for (i, width) in widths.into_iter().enumerate() {
        // SAFETY: 自分のダイアログの一覧へのメッセージ
        unsafe {
            SendMessageW(
                list,
                LVM_SETCOLUMNWIDTH,
                Some(WPARAM(i)),
                Some(LPARAM(width as isize)),
            );
        }
    }
}

/// 幅 `available`（ピクセル）を列の割合で分ける。端数は最後の列に足す。
fn column_widths(available: i32) -> [i32; 4] {
    let available = available.max(0);
    let total: i32 = COLUMNS.iter().map(|(_, w)| w).sum();
    let mut widths = COLUMNS.map(|(_, w)| available * w / total);
    widths[3] += available - widths.iter().sum::<i32>();
    widths
}

/// 一覧の列の数。
fn column_count(list: HWND) -> isize {
    use windows::Win32::UI::Controls::{HDM_GETITEMCOUNT, LVM_GETHEADER};
    // SAFETY: 自分のダイアログの一覧と、その見出しへのメッセージ
    unsafe {
        let header = SendMessageW(list, LVM_GETHEADER, None, None).0;
        if header == 0 {
            return 0;
        }
        SendMessageW(HWND(header as *mut _), HDM_GETITEMCOUNT, None, None).0
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

fn set_text(hwnd: HWND, id: i32, value: &str) {
    let value = to_wide(value);
    // SAFETY: 文字列は呼び出しの間生きている
    unsafe {
        let _ = SetDlgItemTextW(hwnd, id, PCWSTR(value.as_ptr()));
    }
}

fn enable(hwnd: HWND, id: i32, enabled: bool) {
    if let Some(control) = item(hwnd, id) {
        // SAFETY: 自分のダイアログのコントロール
        unsafe {
            let _ = EnableWindow(control, enabled);
        }
    }
}
