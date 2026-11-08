//! 「レシピ」タブ。左にレシピの一覧、右に選んだレシピの名前・操作の並び・ショートカット（#127、ADR-0007）。
//!
//! - 一覧（ListView）の列は 名前 / ショートカット / 印。並びは写しの `Settings::recipes` の順。下の「追加」
//!   「削除」は呼び出し側が `Editor::add_recipe` / `remove_recipe` で写しを変え、`select` で移った行を選ぶ。
//! - 名前の欄は `fields` を通さず、ここで出す（`fill`。フォーカスのある間は書き換えない）。入力は呼び出し側が
//!   `Editor::set_recipe_name` で写しに入れる。
//! - 操作の並びは、見出しのない 1 列の一覧と、横の「上へ」「下へ」「削除」。下のドロップダウンには、その時点で
//!   レシピに足せる操作（`Editor::step_choices`）だけを出し、「足す」で足す（`chosen_step`）。
//! - ショートカットの欄の下に、選んでいる操作のパラメータの欄（#157）。種類（`ParamFields`）ごとの欄のまとまりを
//!   同じ位置に重ねて置き（`PARAM_FIELDS`）、`Editor::param_fields` が返す種類のまとまりだけを出す。パラメータの
//!   ない操作を選んでいるか、何も選んでいなければ、どれも出さない。タブを出し直したとき（`settings_dialog::show_page`）
//!   も、出していないまとまりは隠したまま（`is_hidden_param`）。種類を足すときは、`build` の最後にまとまりを置き、
//!   `PARAM_FIELDS` に足す。
//! - 見た目のパラメータの欄は 5 項目（枠を出すか・枠の色・枠の太さ・透明度・影、#141）。どの欄も先頭が
//!   「変えない」で、枠と影はドロップダウン、太さと透明度は数の欄、枠の色は文字の欄と「選ぶ…」（部品は
//!   「スクラップ」タブと同じ `PageBuilder` の `choice_control` / `number_controls` / `color_controls`）。値の出し入れは
//!   `Editor::appearance` / `set_appearance` で、呼び出し側が `appearance_item` / `appearance_input` で受ける。
//! - ショートカットは全体のホットキーと同じ入力欄（`chord_field`）で、`ChordRow` の 1 つ。行を選び直したら、
//!   呼び出し側が `ChordRow::item` をその行の `SettingsItem::RecipeShortcut` に差し替える（`shortcut_id`）。
//! - 一覧の行の数は `sync_list` で写しに揃える。揃えて選んでいる行がなくなったら、ショートカットの欄が指していた
//!   位置の行を選ぶ。`sync_list` と `fill` の中で行を選んで届く通知と、名前の欄に文字を入れて届く通知では、
//!   写しを変えない（`is_syncing`）。
//! - コントロールの ID は、ページの帯の最初からの決まった順（`build` が置く順）。

use std::cell::{Cell, RefCell};

use shunsoku_core::settings::SettingsItem;
use shunsoku_core::settings::editor::{
    AppearanceInput, AppearanceItem, Editor, ParamFields, SWITCH_CHOICES, StepChoice,
    UNCHANGED_KEY, step_label,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    EM_SETCUEBANNER, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS,
    LVM_GETITEMCOUNT, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_NOCOLUMNHEADER, LVS_NOSORTHEADER, LVS_REPORT,
    LVS_SHOWSELALWAYS, LVS_SINGLESEL, UDM_SETRANGE32,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCOUNT, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL,
    IsWindowVisible, SM_CXVSCROLL, SW_HIDE, SW_SHOW, SendMessageW, SetDlgItemTextW, ShowWindow,
    WINDOW_STYLE, WS_BORDER, WS_TABSTOP,
};
use windows::core::{PCWSTR, PWSTR};

use super::fields::{set_window_text, window_text};
use super::list_view::{self, set_cell};
use super::{
    Class, During, PAGE_WIDTH, Page, PageBuilder, STATIC_TEXT, Text, current_page, item,
    page_first_id,
};
use crate::wide::to_wide;

/// ページの帯の最初からの位置（`build` が置く順）。
const LIST: i32 = 0;
const ADD: i32 = 1;
const REMOVE: i32 = 2;
/// 詳細の最初（名前の見出し）。ここから `DETAIL_END` の手前までを、選んだ行がなければ使えなくする。
const DETAIL_FIRST: i32 = 3;
const NAME: i32 = 4;
const NAME_MARK: i32 = 5;
const STEPS: i32 = 7;
const STEP_UP: i32 = 8;
const STEP_DOWN: i32 = 9;
const STEP_REMOVE: i32 = 10;
const STEPS_MARK: i32 = 11;
const STEP_CHOICE: i32 = 12;
const STEP_ADD: i32 = 13;
const SHORTCUT: i32 = 15;
const DETAIL_END: i32 = 17;
/// 見た目のパラメータの欄（見出しと上下のボタンを含む）。
const APPEARANCE_FIRST: i32 = DETAIL_END;
const BORDER: i32 = 18;
const BORDER_COLOR: i32 = 20;
const PICK_COLOR: i32 = 21;
const BORDER_WIDTH: i32 = 23;
const BORDER_WIDTH_UPDOWN: i32 = 24;
const OPACITY: i32 = 26;
const OPACITY_UPDOWN: i32 = 27;
const SHADOW: i32 = 29;
const APPEARANCE_END: i32 = 30;
/// パラメータの欄の種類と、そのまとまり（ページの帯の位置の範囲）。選んでいる操作の種類のまとまりだけを出す。
const PARAM_FIELDS: [(ParamFields, i32, i32); 1] =
    [(ParamFields::Appearance, APPEARANCE_FIRST, APPEARANCE_END)];
/// ページの帯の、最後のコントロールの次。
const PAGE_END: i32 = APPEARANCE_END;
/// 見た目を指定する操作の項目と、その欄。
const APPEARANCE_FIELDS: [(AppearanceItem, i32); 5] = [
    (AppearanceItem::Border, BORDER),
    (AppearanceItem::BorderColor, BORDER_COLOR),
    (AppearanceItem::BorderWidth, BORDER_WIDTH),
    (AppearanceItem::Opacity, OPACITY),
    (AppearanceItem::Shadow, SHADOW),
];
/// 数の欄の上下のボタン。
const UPDOWNS: [(AppearanceItem, i32); 2] = [
    (AppearanceItem::BorderWidth, BORDER_WIDTH_UPDOWN),
    (AppearanceItem::Opacity, OPACITY_UPDOWN),
];

/// 一覧の幅と高さ、その下のボタン（ページの中のダイアログ単位）。
const LIST_WIDTH: i16 = 112;
const LIST_HEIGHT: i16 = 168;
const LIST_BUTTON_GAP: i16 = 2;
/// 詳細の左端、見出しの幅と欄の左端。
const DETAIL_X: i16 = LIST_WIDTH + 6;
const LABEL_WIDTH: i16 = 44;
const FIELD_X: i16 = DETAIL_X + LABEL_WIDTH + 2;
const FIELD_WIDTH: i16 = PAGE_WIDTH - FIELD_X;
/// 操作の並びの一覧と、その横のボタン。
const STEPS_Y: i16 = 30;
const STEPS_HEIGHT: i16 = 46;
/// 並びの下の印。範囲外の文言は 2 行になる。
const STEPS_MARK_HEIGHT: i16 = 20;
const STEP_BUTTON_WIDTH: i16 = 40;
const STEPS_WIDTH: i16 = FIELD_WIDTH - STEP_BUTTON_WIDTH - 4;
const STEP_BUTTON_X: i16 = FIELD_X + STEPS_WIDTH + 4;
/// 足す操作のドロップダウンと、ショートカット（印は欄の下に 2 行まで）。
const STEP_CHOICE_Y: i16 = STEPS_Y + STEPS_HEIGHT + 2 + STEPS_MARK_HEIGHT + 2;
const SHORTCUT_Y: i16 = STEP_CHOICE_Y + 22;
/// パラメータの欄の上端（ショートカットの印の下）。種類ごとのまとまりをここに重ねて置く。
const PARAMS_Y: i16 = SHORTCUT_Y + 16 + 18 + 2;
/// 見た目のパラメータの欄（4 行）。上から枠を出すか、枠の色、枠の太さ、透明度と影。2 列目の見出しと欄の左端。
const APPEARANCE_ROW: i16 = 16;
const SWITCH_WIDTH: i16 = 56;
const NUMBER_WIDTH: i16 = 48;
const SECOND_LABEL_X: i16 = FIELD_X + SWITCH_WIDTH + 6;
const SECOND_LABEL_WIDTH: i16 = 32;
const SECOND_FIELD_X: i16 = FIELD_X + 96;
/// 一覧の列の見出し。幅は中身に合わせる（`fit_columns`）。
const COLUMNS: [&str; 3] = [
    "settings_gui.recipes.name",
    "settings_gui.recipes.shortcut",
    "settings_gui.scrap_ops.mark",
];

/// タブのコントロールを置く。
pub(super) fn build(b: &mut PageBuilder) {
    let first = b.next_id;
    let style = |s: u32| WINDOW_STYLE(s);
    let push = WS_TABSTOP | style(BS_PUSHBUTTON as u32);
    let list_style = WS_TABSTOP
        | WS_BORDER
        | style(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER);
    let list = b.control(
        Class::ListView,
        "",
        list_style,
        0,
        0,
        LIST_WIDTH,
        LIST_HEIGHT,
    );
    let mut list_buttons = Vec::new();
    let keys = ["settings_gui.recipes.add", "settings_gui.recipes.remove"];
    let width = (LIST_WIDTH - LIST_BUTTON_GAP) / 2;
    for (i, key) in keys.iter().enumerate() {
        let x = i as i16 * (width + LIST_BUTTON_GAP);
        let text = (b.text)(key);
        list_buttons.push(b.control(Class::Button, text, push, x, LIST_HEIGHT + 4, width, 14));
    }

    // 名前とその印
    b.label(
        "settings_gui.recipes.name_label",
        DETAIL_X,
        2,
        LABEL_WIDTH,
        10,
    );
    let name = b.edit(WINDOW_STYLE(0), FIELD_X, 0, FIELD_WIDTH);
    let name_mark = b.control(Class::Static, "", STATIC_TEXT, FIELD_X, 16, FIELD_WIDTH, 10);
    // 操作の並びと、横の「上へ」「下へ」「削除」、下の印
    b.label(
        "settings_gui.recipes.steps_label",
        DETAIL_X,
        STEPS_Y + 2,
        LABEL_WIDTH,
        10,
    );
    let steps = b.control(
        Class::ListView,
        "",
        list_style | style(LVS_NOCOLUMNHEADER),
        FIELD_X,
        STEPS_Y,
        STEPS_WIDTH,
        STEPS_HEIGHT,
    );
    let mut step_buttons = Vec::new();
    let keys = [
        "settings_gui.regions.up",
        "settings_gui.regions.down",
        "settings_gui.regions.remove",
    ];
    for (i, key) in keys.iter().enumerate() {
        let text = (b.text)(key);
        step_buttons.push(b.control(
            Class::Button,
            text,
            push,
            STEP_BUTTON_X,
            STEPS_Y + i as i16 * 16,
            STEP_BUTTON_WIDTH,
            14,
        ));
    }
    let steps_mark = b.control(
        Class::Static,
        "",
        STATIC_TEXT,
        FIELD_X,
        STEPS_Y + STEPS_HEIGHT + 2,
        FIELD_WIDTH,
        STEPS_MARK_HEIGHT,
    );
    // 足す操作を選ぶドロップダウン（高さは開いたときの一覧の高さ）と「足す」
    let choice = b.choice_control(FIELD_X, STEP_CHOICE_Y, STEPS_WIDTH, 120);
    let add_text = (b.text)("settings_gui.recipes.add_step");
    let step_add = b.control(
        Class::Button,
        add_text,
        push,
        STEP_BUTTON_X,
        STEP_CHOICE_Y,
        STEP_BUTTON_WIDTH,
        14,
    );
    // ショートカット（印は欄の下）
    b.label(
        "settings_gui.recipes.shortcut_label",
        DETAIL_X,
        SHORTCUT_Y + 2,
        LABEL_WIDTH,
        10,
    );
    let shortcut = b.next_id;
    b.chord_field(SettingsItem::RecipeShortcut(0), FIELD_X, SHORTCUT_Y, 110);

    // パラメータの欄。種類ごとのまとまりを同じ位置（`PARAMS_Y`）に重ねて置く（`PARAM_FIELDS`）。
    // 見た目: 部品は「スクラップ」タブと同じ（数は上下のボタン付き、色は「選ぶ…」付き）。オン・オフは
    // 「変えない」を先頭に置くドロップダウン
    let appearance_first = b.next_id;
    let label = |b: &mut PageBuilder, item: AppearanceItem, x: i16, y: i16, cx: i16| {
        b.label(item.label_key(), x, y + 2, cx, 10);
    };
    let mut y = PARAMS_Y;
    label(b, AppearanceItem::Border, DETAIL_X, y, LABEL_WIDTH);
    let border = b.choice_control(FIELD_X, y, SWITCH_WIDTH, 60);
    y += APPEARANCE_ROW;
    label(b, AppearanceItem::BorderColor, DETAIL_X, y, LABEL_WIDTH);
    let (border_color, pick_color) = b.color_controls(FIELD_X, y);
    y += APPEARANCE_ROW;
    label(b, AppearanceItem::BorderWidth, DETAIL_X, y, LABEL_WIDTH);
    let (border_width, border_width_updown) = b.number_controls(FIELD_X, y, NUMBER_WIDTH);
    y += APPEARANCE_ROW;
    label(b, AppearanceItem::Opacity, DETAIL_X, y, LABEL_WIDTH);
    let (opacity, opacity_updown) = b.number_controls(FIELD_X, y, NUMBER_WIDTH);
    label(
        b,
        AppearanceItem::Shadow,
        SECOND_LABEL_X,
        y,
        SECOND_LABEL_WIDTH,
    );
    let shadow_width = PAGE_WIDTH - SECOND_FIELD_X;
    let shadow = b.choice_control(SECOND_FIELD_X, y, shadow_width, 60);
    let appearance_end = b.next_id;

    let offsets = |ids: &[i32]| ids.iter().map(|id| id - first).collect::<Vec<_>>();
    debug_assert_eq!(offsets(&[list]), [LIST]);
    debug_assert_eq!(offsets(&list_buttons), [ADD, REMOVE]);
    debug_assert_eq!(
        offsets(&[
            name, name_mark, steps, steps_mark, choice, step_add, shortcut
        ]),
        [
            NAME,
            NAME_MARK,
            STEPS,
            STEPS_MARK,
            STEP_CHOICE,
            STEP_ADD,
            SHORTCUT
        ]
    );
    debug_assert_eq!(offsets(&step_buttons), [STEP_UP, STEP_DOWN, STEP_REMOVE]);
    debug_assert_eq!(
        offsets(&[
            appearance_first,
            border,
            border_color,
            pick_color,
            border_width,
            border_width_updown,
            opacity,
            opacity_updown,
            shadow,
            appearance_end
        ]),
        [
            APPEARANCE_FIRST,
            BORDER,
            BORDER_COLOR,
            PICK_COLOR,
            BORDER_WIDTH,
            BORDER_WIDTH_UPDOWN,
            OPACITY,
            OPACITY_UPDOWN,
            SHADOW,
            APPEARANCE_END
        ]
    );
    debug_assert_eq!(b.next_id - first, PAGE_END);
}

fn id(offset: i32) -> i32 {
    page_first_id(Page::Recipes) + offset
}

/// レシピの一覧の ID。
pub fn list_id() -> i32 {
    id(LIST)
}

/// 操作の並びの一覧の ID。
pub fn steps_id() -> i32 {
    id(STEPS)
}

/// 名前の欄の ID。
pub fn name_id() -> i32 {
    id(NAME)
}

/// ショートカットの欄の ID。行を選び直したら、この `ChordRow` の項目を差し替える。
pub fn shortcut_id() -> i32 {
    id(SHORTCUT)
}

/// タブのボタン。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// レシピを足す。
    Add,
    /// レシピを消す。
    Remove,
    /// 選んだ操作を上へ。
    StepUp,
    /// 選んだ操作を下へ。
    StepDown,
    /// 選んだ操作を消す。
    StepRemove,
    /// ドロップダウンで選んだ操作を足す。
    StepAdd,
    /// 見た目を指定する操作の枠の色を、色を選ぶダイアログで選ぶ。
    PickColor,
}

/// ID `id` がタブのボタンならそれ。
pub fn button(id: i32) -> Option<Button> {
    match id - page_first_id(Page::Recipes) {
        ADD => Some(Button::Add),
        REMOVE => Some(Button::Remove),
        STEP_UP => Some(Button::StepUp),
        STEP_DOWN => Some(Button::StepDown),
        STEP_REMOVE => Some(Button::StepRemove),
        STEP_ADD => Some(Button::StepAdd),
        PICK_COLOR => Some(Button::PickColor),
        _ => None,
    }
}

/// レシピの一覧で選んでいる行。
pub fn selected(hwnd: HWND) -> Option<usize> {
    list_view::selected(item(hwnd, list_id())?)
}

/// レシピの一覧の行 `index` を選んで見えるようにする。選び直しの通知（`LVN_ITEMCHANGED`）が届く。
pub fn select(hwnd: HWND, index: usize) {
    if let Some(list) = item(hwnd, list_id()) {
        list_view::select(list, index);
    }
}

/// 操作の並びの一覧で選んでいる操作。
pub fn selected_step(hwnd: HWND) -> Option<usize> {
    list_view::selected(item(hwnd, steps_id())?)
}

/// 操作の並びの一覧の操作 `index` を選ぶ。選び直しの通知が届く。
pub fn select_step(hwnd: HWND, index: usize) {
    if let Some(list) = item(hwnd, steps_id()) {
        list_view::select(list, index);
    }
}

/// ID `id` が見た目を指定する操作の欄なら、その項目。
pub fn appearance_item(id: i32) -> Option<AppearanceItem> {
    let offset = id - page_first_id(Page::Recipes);
    APPEARANCE_FIELDS
        .iter()
        .find(|&&(_, o)| o == offset)
        .map(|&(item, _)| item)
}

/// 見た目を指定する操作の欄 `item` の今の入力。
pub fn appearance_input(hwnd: HWND, item: AppearanceItem) -> Option<AppearanceInput> {
    let control = self::item(hwnd, appearance_id(item))?;
    if !item.is_switch() {
        return Some(AppearanceInput::Text(window_text(control)));
    }
    // SAFETY: 自分のダイアログのドロップダウンへのメッセージ
    let index = unsafe { SendMessageW(control, CB_GETCURSEL, None, None) }.0;
    let (on, _) = SWITCH_CHOICES.get(usize::try_from(index).ok()?)?;
    Some(AppearanceInput::Switch(*on))
}

fn appearance_id(item: AppearanceItem) -> i32 {
    let offset = APPEARANCE_FIELDS
        .iter()
        .find(|&&(i, _)| i == item)
        .map_or(BORDER, |&(_, o)| o);
    id(offset)
}

/// 名前の欄の今の文字。
pub fn name_text(hwnd: HWND) -> Option<String> {
    item(hwnd, name_id()).map(window_text)
}

thread_local! {
    /// `sync_list` か `fill` が一覧の行や名前の欄を揃えている間。
    static SYNCING: Cell<bool> = const { Cell::new(false) };
    /// ドロップダウンに出している、足せるもの（`chosen_step`）。
    static CHOICES: RefCell<Vec<StepChoice>> = const { RefCell::new(Vec::new()) };
    /// 操作の並びの一覧に出しているレシピの位置と、操作の文言。
    static STEPS_SHOWN: RefCell<(Option<usize>, Vec<String>)> = const { RefCell::new((None, Vec::new())) };
    /// 出しているパラメータの欄の種類。
    static PARAMS_SHOWN: Cell<Option<ParamFields>> = const { Cell::new(None) };
}

/// 一覧の行や名前の欄を揃えている途中か（届いた通知では写しを変えない）。
pub fn is_syncing() -> bool {
    SYNCING.get()
}

/// ドロップダウンで選んでいる、足すもの。
pub fn chosen_step(hwnd: HWND) -> Option<StepChoice> {
    let combo = item(hwnd, id(STEP_CHOICE))?;
    // SAFETY: 自分のダイアログのドロップダウンへのメッセージ
    let index = unsafe { SendMessageW(combo, CB_GETCURSEL, None, None) }.0;
    let index = usize::try_from(index).ok()?;
    CHOICES.with_borrow(|c| c.get(index).copied())
}

/// レシピの一覧の行を写し `editor` のレシピに揃え、各行の文言を出す。選んでいる行がなくなったら、ショートカットの
/// 欄が指していた位置 `bound`（末尾を越えれば末尾）の行を選ぶ。
pub fn sync_list(hwnd: HWND, editor: &Editor, bound: usize, text: Text) {
    let Some(list) = item(hwnd, list_id()) else {
        return;
    };
    let _syncing = During::enter(&SYNCING);
    let recipes = &editor.draft().recipes;
    if column_count(list) == 0 {
        init_columns(list, &COLUMNS.map(text));
    }
    set_row_count(list, recipes.len());
    for (i, r) in recipes.iter().enumerate() {
        set_cell(list, i, 0, &r.name);
        let shortcut = r.shortcut.map_or_else(
            || text("settings_gui.scrap_ops.none").to_owned(),
            |c| c.to_string(),
        );
        set_cell(list, i, 1, &shortcut);
        let mark = [
            SettingsItem::RecipeName(i),
            SettingsItem::RecipeSteps(i),
            SettingsItem::RecipeShortcut(i),
        ]
        .iter()
        .find_map(|item| editor.mark(item))
        .map_or("", |m| text(m.label_key()));
        set_cell(list, i, 2, mark);
    }
    fit_columns(hwnd);
    if selected(hwnd).is_none() && !recipes.is_empty() {
        select(hwnd, bound.min(recipes.len() - 1));
    }
}

/// 選んだレシピの詳細のうち、ショートカットの欄以外を出す: 名前、名前と操作の並びの印、操作の並び、足せる操作、
/// ボタンを押せるか。ショートカットの欄の値と印は `settings_dialog::fill` が出す。
pub(super) fn fill(hwnd: HWND, editor: &Editor, text: Text) {
    let _syncing = During::enter(&SYNCING);
    let recipes = &editor.draft().recipes;
    let selected = selected(hwnd).filter(|&i| i < recipes.len());
    let recipe = selected.map(|i| (i, &recipes[i]));

    // 名前（打っている途中の欄は書き換えない）
    if let Some(name) = item(hwnd, name_id()) {
        // SAFETY: フォーカスのあるウィンドウを読むだけ
        let focused = unsafe { GetFocus() } == name;
        let value = recipe.map_or("", |(_, r)| r.name.as_str());
        if !focused && window_text(name) != value {
            set_window_text(name, value);
        }
    }
    let mark = |item: Option<SettingsItem>| {
        item.and_then(|item| editor.mark(&item))
            .map_or("", |m| text(m.label_key()))
    };
    set_text(
        hwnd,
        id(NAME_MARK),
        mark(selected.map(SettingsItem::RecipeName)),
    );
    set_text(
        hwnd,
        id(STEPS_MARK),
        mark(selected.map(SettingsItem::RecipeSteps)),
    );

    // 操作の並び。変わったときだけ入れ直す（一覧で操作を選んだ通知の中で、行を消さないため）。レシピを選び直したら、
    // 操作は選ばない
    let labels: Vec<String> = recipe.map_or_else(Vec::new, |(_, r)| {
        r.steps.iter().map(|s| step_label(s, text)).collect()
    });
    let steps = labels.len();
    if let Some(list) = item(hwnd, steps_id()) {
        if column_count(list) == 0 {
            init_columns(list, &[""]);
        }
        // SAFETY: 自分のダイアログの一覧へのメッセージ
        let count = unsafe { SendMessageW(list, LVM_GETITEMCOUNT, None, None) }.0;
        let (same_recipe, same_rows) = STEPS_SHOWN.with_borrow(|(of, shown)| {
            (
                *of == selected,
                *shown == labels && usize::try_from(count) == Ok(steps),
            )
        });
        if !(same_recipe && same_rows) {
            let step = same_recipe.then(|| list_view::selected(list)).flatten();
            // SAFETY: 自分のダイアログの一覧へのメッセージ
            unsafe { SendMessageW(list, LVM_DELETEALLITEMS, None, None) };
            set_row_count(list, steps);
            for (i, label) in labels.iter().enumerate() {
                set_cell(list, i, 0, label);
            }
            fit_steps_column(list);
            if let Some(step) = step.filter(|_| steps > 0) {
                list_view::select(list, step.min(steps - 1));
            }
            STEPS_SHOWN.set((selected, labels));
        }
    }
    let step = selected_step(hwnd).filter(|&s| s < steps);

    // 足せる操作。変わったときだけ入れ直す（選んでいる操作を残すため）
    let choices: Vec<StepChoice> = selected.map(|i| editor.step_choices(i)).unwrap_or_default();
    if let Some(combo) = item(hwnd, id(STEP_CHOICE))
        // SAFETY: 自分のダイアログのドロップダウンへのメッセージ
        && (CHOICES.with_borrow(|c| *c != choices)
            || usize::try_from(unsafe { SendMessageW(combo, CB_GETCOUNT, None, None) }.0)
                != Ok(choices.len()))
    {
        let kept = chosen_step(hwnd);
        // SAFETY: 自分のダイアログのドロップダウンへのメッセージ。文字列は呼び出しの間生きている
        unsafe {
            SendMessageW(combo, CB_RESETCONTENT, None, None);
            for choice in &choices {
                let label = to_wide(&choice_label(*choice, text));
                SendMessageW(
                    combo,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(label.as_ptr() as isize)),
                );
            }
            let index = kept
                .and_then(|k| choices.iter().position(|&c| c == k))
                .unwrap_or(0);
            if !choices.is_empty() {
                SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(index)), None);
            }
        }
        CHOICES.set(choices.clone());
    }

    let params = selected
        .zip(step)
        .and_then(|(i, s)| editor.param_fields(i, s));
    fill_appearance(
        hwnd,
        editor,
        selected
            .zip(step)
            .filter(|_| params == Some(ParamFields::Appearance)),
        text,
    );
    show_params(hwnd, params);

    for offset in DETAIL_FIRST..DETAIL_END {
        enable(hwnd, id(offset), selected.is_some());
    }
    enable(hwnd, id(REMOVE), selected.is_some());
    enable(hwnd, id(STEP_UP), step.is_some_and(|s| s > 0));
    enable(hwnd, id(STEP_DOWN), step.is_some_and(|s| s + 1 < steps));
    enable(hwnd, id(STEP_REMOVE), step.is_some());
    enable(
        hwnd,
        id(STEP_ADD),
        selected.is_some() && !choices.is_empty(),
    );
}

/// パラメータの欄のうち、種類 `params` のまとまりだけを出し、ほかは隠す（None なら全部隠す）。レシピのタブを
/// 出していなければ、覚えるだけで出さない（タブを出すときに `show_page` が `is_hidden_param` で従う）。
fn show_params(hwnd: HWND, params: Option<ParamFields>) {
    PARAMS_SHOWN.set(params);
    let page_shown = current_page(hwnd) == Page::Recipes;
    for (kind, first, end) in PARAM_FIELDS {
        let shown = page_shown && params == Some(kind);
        for offset in first..end {
            // 出し方が変わるときだけ（打つたびに出し直さない）
            if let Some(control) = item(hwnd, id(offset))
                // SAFETY: 自分のダイアログのコントロール
                && unsafe { IsWindowVisible(control) }.as_bool() != shown
            {
                // SAFETY: 自分のダイアログのコントロール
                unsafe {
                    let _ = ShowWindow(control, if shown { SW_SHOW } else { SW_HIDE });
                }
            }
        }
    }
}

/// ID `id` が、いま出していないパラメータの欄か（タブを出し直しても隠したままにする）。
pub(super) fn is_hidden_param(id: i32) -> bool {
    let offset = id - page_first_id(Page::Recipes);
    let shown = PARAMS_SHOWN.get();
    PARAM_FIELDS
        .iter()
        .any(|&(kind, first, end)| (first..end).contains(&offset) && shown != Some(kind))
}

/// 見た目のパラメータの欄に、レシピと操作 `at` の値を出す。None なら欄を空にする。初めて出すときに、
/// ドロップダウンの選択肢・数の範囲・空のときの「変えない」を入れる。
fn fill_appearance(hwnd: HWND, editor: &Editor, at: Option<(usize, usize)>, text: Text) {
    // SAFETY: フォーカスのあるウィンドウを読むだけ
    let focus = unsafe { GetFocus() };
    let shown = at
        .and_then(|(i, s)| editor.appearance(i, s, AppearanceItem::Border))
        .is_some();
    for (item, offset) in APPEARANCE_FIELDS {
        let Some(control) = self::item(hwnd, id(offset)) else {
            continue;
        };
        let value = at.and_then(|(i, s)| editor.appearance(i, s, item));
        if item.is_switch() {
            // SAFETY: 自分のダイアログのドロップダウンへのメッセージ。文字列は呼び出しの間生きている
            unsafe {
                if SendMessageW(control, CB_GETCOUNT, None, None).0 == 0 {
                    for (_, key) in SWITCH_CHOICES {
                        let label = to_wide(text(key));
                        SendMessageW(
                            control,
                            CB_ADDSTRING,
                            None,
                            Some(LPARAM(label.as_ptr() as isize)),
                        );
                    }
                }
                // 見た目を指定する操作でなければ、何も選ばない（-1）
                let index = match value {
                    Some(AppearanceInput::Switch(on)) => SWITCH_CHOICES
                        .iter()
                        .position(|(v, _)| *v == on)
                        .map_or(-1, |i| i as isize),
                    _ => -1,
                };
                SendMessageW(control, CB_SETCURSEL, Some(WPARAM(index as usize)), None);
            }
        } else {
            let value = match value {
                Some(AppearanceInput::Text(s)) => s,
                _ => String::new(),
            };
            // 打っている途中の欄は書き換えない
            if control != focus && window_text(control) != value {
                set_window_text(control, &value);
            }
            // 空のときは「変えない」を薄く出す（見た目を指定する操作を選んでいる間だけ）
            let cue = to_wide(if shown && value.is_empty() {
                text(UNCHANGED_KEY)
            } else {
                ""
            });
            // SAFETY: 自分のエディットへのメッセージ。文字列は呼び出しの間生きている（エディットが写す）
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
    for (item, offset) in UPDOWNS {
        if let (Some(updown), Some((min, max))) =
            (self::item(hwnd, id(offset)), item.number_range())
        {
            // SAFETY: 自分のダイアログの上下のボタンへのメッセージ
            unsafe {
                SendMessageW(
                    updown,
                    UDM_SETRANGE32,
                    Some(WPARAM(min as usize)),
                    Some(LPARAM(max as isize)),
                );
            }
        }
    }
}

/// 足せるもの `choice` の文言（ドロップダウンに出す）。表にない id はそのまま。
fn choice_label(choice: StepChoice, text: Text) -> String {
    match (choice.label_key(), choice) {
        (Some(key), _) => text(key).to_owned(),
        (None, StepChoice::Op(id)) => id.to_owned(),
        (None, StepChoice::Appearance) => String::new(),
    }
}

/// 一覧 `list` の行を `count` 行にする（数が違えば空の行で作り直す）。
fn set_row_count(list: HWND, count: usize) {
    // SAFETY: 自分のダイアログの一覧へのメッセージ。構造体は呼び出しの間生きている（一覧が写す）
    unsafe {
        let now = SendMessageW(list, LVM_GETITEMCOUNT, None, None).0;
        if usize::try_from(now).ok() == Some(count) {
            return;
        }
        SendMessageW(list, LVM_DELETEALLITEMS, None, None);
        for i in 0..count {
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

/// 一覧 `list` に見出し `labels` の列を入れる。
fn init_columns(list: HWND, labels: &[&str]) {
    // SAFETY: 自分のダイアログの一覧へのメッセージ。文字列と構造体は呼び出しの間生きている（一覧が写す）
    unsafe {
        let ex = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            Some(WPARAM(ex as usize)),
            Some(LPARAM(ex as isize)),
        );
        for (i, label) in labels.iter().enumerate() {
            let mut label = to_wide(label);
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

/// 縦のスクロールバーの幅（一覧 `list` の DPI で）。行が増えると出るので、その分を常に空けておく。
fn scroll_bar(list: HWND) -> i32 {
    // SAFETY: 一覧の DPI を読む
    unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, GetDpiForWindow(list)) }
}

/// レシピの一覧の列の幅を中身に合わせ、余りは最後の列（印）に足す（`list_view::fit_columns`）。行の文言を出し
/// 直したとき（`sync_list`）と、DPI が変わってダイアログが大きさと書体を直し終えたときに呼ぶ。
pub fn fit_columns(hwnd: HWND) {
    if let Some(list) = item(hwnd, list_id()) {
        list_view::fit_columns(list, COLUMNS.len(), &[], scroll_bar(list));
    }
    if let Some(list) = item(hwnd, steps_id()) {
        fit_steps_column(list);
    }
}

/// 操作の並びの一覧の 1 列を、一覧の幅いっぱいにする。
fn fit_steps_column(list: HWND) {
    list_view::fit_columns(list, 1, &[], scroll_bar(list));
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
