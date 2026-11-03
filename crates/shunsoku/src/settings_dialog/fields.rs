//! キーの組み合わせ以外の欄（チェックボックス・テキスト・数値・ドロップダウン）と、その横のボタン。
//!
//! - 欄は `Field`（写しのどの項目か）で指す。欄を置くのは `PageBuilder` の `check` / `text` / `number` /
//!   `choice` と、組になった `folder_field`（保存先と「参照…」）・`color_field`（色と「選ぶ…」）・
//!   `template_field`（ファイル名テンプレートと「変数を挿入」とプレビュー）。
//! - 写しの値を欄に出すのは `fill`、欄の入力を写しに入れるのは `set`。どちらも `Field` の `match` で項目と結ぶ。
//!   項目を足すときは `Field` に足し、`kind`・`shown`・`set` の腕を足す。
//! - ファイル名テンプレートの欄は、どのテンプレートかを `TemplateOf` で指す。定型領域タブ（#98）は
//!   `TemplateOf::Region(i)` を足し、`template_of` / `template_slot` / `preview_target` に腕を足して使い回す。
//! - フォーカスのあるテキストの欄は `fill` で書き換えない（打っている途中のキャレットを動かさない）。読めない入力
//!   （範囲の外の数、`#RRGGBB` でない色、空のテンプレート）は写しを変えず、フォーカスが外れたら写しの値に戻る。

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use shunsoku_core::annotate::toolbar::{FONT_SIZES, WIDTHS};
use shunsoku_core::selection::Mode;
use shunsoku_core::settings::editor::{Editor, choices};
use shunsoku_core::settings::{
    BORDER_WIDTH_RANGE, DpiScaling, OPACITY_RANGE, Rgb, Settings, TRASH_LIMIT_RANGE,
};
use shunsoku_core::template;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DEFAULT_CHARSET, EnumFontFamiliesExW, GetDC, LOGFONTW, ReleaseDC, TEXTMETRICW,
};
use windows::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Controls::{
    EM_SETCUEBANNER, UDM_SETRANGE32, UDS_ALIGNRIGHT, UDS_ARROWKEYS, UDS_AUTOBUDDY, UDS_NOTHOUSANDS,
    UDS_SETBUDDYINT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCOUNT,
    CB_GETCURSEL, CB_GETLBTEXT, CB_GETLBTEXTLEN, CB_RESETCONTENT, CB_SETCURSEL, CBS_DROPDOWNLIST,
    ES_AUTOHSCROLL, ES_NUMBER, GetWindowTextLengthW, GetWindowTextW, SendMessageW, SetDlgItemTextW,
    SetWindowTextW, WINDOW_STYLE, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VSCROLL,
};
use windows::core::PCWSTR;

use super::{Class, Item, PAGE_X, PAGE_Y, PageBuilder, SS_LEFT, Text, item};
use crate::save;
use crate::wide::to_wide;

/// 欄が結ぶ写しの項目（キーの組み合わせの項目は `SettingsItem` と `ChordRow`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `[general] autostart`。
    Autostart,
    /// `[general] notifications`。
    Notifications,
    /// `[general] save_dir`。空なら既定（ピクチャ フォルダの下）。
    SaveDir,
    /// ファイル名テンプレート。
    Template(TemplateOf),
    /// `[general] capture_cursor`。
    CaptureCursor,
    /// `[selection] mode`。
    SelectionMode,
    /// `[selection] crosshair`。
    Crosshair,
    /// `[scrap] opacity`。
    Opacity,
    /// `[scrap] border`。
    Border,
    /// `[scrap] border_width`。
    BorderWidth,
    /// `[scrap] border_color`。
    BorderColor,
    /// `[scrap] shadow`。
    Shadow,
    /// `[general] trash_limit`。
    TrashLimit,
    /// `[general] dpi_scaling`。
    DpiScaling,
    /// `[annotation] color`。
    AnnotationColor,
    /// `[annotation] width`。
    AnnotationWidth,
    /// `[annotation] font_size`。
    AnnotationFontSize,
    /// `[annotation] font`。
    AnnotationFont,
}

/// どのファイル名テンプレートか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateOf {
    /// `[general] file_name_template`。
    General,
}

/// 欄の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Check,
    /// テキスト。`preview` はファイル名テンプレートのプレビューの Static の ID。
    Text {
        preview: Option<i32>,
    },
    /// 数（横に上下のボタン `updown` が付く）。
    Number {
        updown: i32,
    },
    /// ドロップダウン（選ぶだけ）。
    Choice,
}

/// 欄 1 つ。
#[derive(Debug, Clone, Copy)]
pub struct FieldRow {
    pub field: Field,
    /// コントロールの ID。
    pub control: i32,
    pub kind: Kind,
}

/// 欄の横のボタンが押されたときにすること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// フォルダを選ばせて `Field` に入れる。
    BrowseFolder(Field),
    /// 色を選ばせて `Field` に入れる。
    PickColor(Field),
    /// 変数のメニューを出し、テンプレートの欄に入れる。
    InsertVariable(TemplateOf),
}

/// ボタン 1 つ。
#[derive(Debug, Clone, Copy)]
pub struct ButtonRow {
    pub id: i32,
    pub action: Action,
}

/// ダイアログの欄とボタン。`fill` に渡すため写せる（書体の一覧は共有する）。
#[derive(Debug, Clone, Default)]
pub struct Fields {
    pub rows: Vec<FieldRow>,
    pub buttons: Vec<ButtonRow>,
    /// インストール済みの書体（昇順）。書体のドロップダウンの候補。
    fonts: Rc<Vec<String>>,
}

impl Fields {
    /// ID `id` の欄。
    pub fn row(&self, id: i32) -> Option<FieldRow> {
        self.rows.iter().find(|r| r.control == id).copied()
    }

    /// 項目 `field` の欄。
    pub fn row_of(&self, field: Field) -> Option<FieldRow> {
        self.rows.iter().find(|r| r.field == field).copied()
    }

    /// ID `id` のボタン。
    pub fn button(&self, id: i32) -> Option<ButtonRow> {
        self.buttons.iter().find(|b| b.id == id).copied()
    }
}

fn kind(field: Field) -> Kind {
    match field {
        Field::Autostart
        | Field::Notifications
        | Field::CaptureCursor
        | Field::Crosshair
        | Field::Border
        | Field::Shadow => Kind::Check,
        Field::SaveDir | Field::Template(_) | Field::BorderColor | Field::AnnotationColor => {
            Kind::Text { preview: None }
        }
        Field::Opacity | Field::BorderWidth | Field::TrashLimit => Kind::Number { updown: 0 },
        Field::SelectionMode
        | Field::DpiScaling
        | Field::AnnotationWidth
        | Field::AnnotationFontSize
        | Field::AnnotationFont => Kind::Choice,
    }
}

/// 数の欄の範囲。
fn number_range(field: Field) -> (u32, u32) {
    let r = |r: std::ops::RangeInclusive<u8>| (u32::from(*r.start()), u32::from(*r.end()));
    match field {
        Field::Opacity => r(OPACITY_RANGE),
        Field::TrashLimit => r(TRASH_LIMIT_RANGE),
        _ => (*BORDER_WIDTH_RANGE.start(), *BORDER_WIDTH_RANGE.end()),
    }
}

/// 列挙のドロップダウンの項目（値と文言のキー）。
const SELECTION_MODES: [(Mode, &str); 2] = [
    (Mode::Normal, "settings_gui.capture.mode_normal"),
    (Mode::Precise, "settings_gui.capture.mode_precise"),
];
const DPI_SCALINGS: [(DpiScaling, &str); 2] = [
    (DpiScaling::KeepPixels, "settings_gui.scrap.dpi_keep_pixels"),
    (
        DpiScaling::KeepApparentSize,
        "settings_gui.scrap.dpi_keep_apparent_size",
    ),
];

/// 欄に出す値。
enum Shown {
    Check(bool),
    Text(String),
    Choice { items: Vec<String>, selected: usize },
}

/// 写しの項目 `field` を欄に出す形にする。数のドロップダウンは、開いた時点（base）の値が候補の外なら候補に
/// 足しておく（選び直しても消えない）。
fn shown(field: Field, editor: &Editor, fonts: &[String], text: Text) -> Shown {
    let d = editor.draft();
    let b = editor.base();
    let numbers = |candidates: &[u32], base: u32, current: u32| {
        let mut candidates = candidates.to_vec();
        if !candidates.contains(&base) {
            candidates.push(base);
            candidates.sort_unstable();
        }
        let c = choices(&candidates, &current);
        Shown::Choice {
            items: c.items.iter().map(u32::to_string).collect(),
            selected: c.selected,
        }
    };
    let labels = |keys: &[&'static str], selected: usize| Shown::Choice {
        items: keys.iter().map(|&k| text(k).to_owned()).collect(),
        selected,
    };
    match field {
        Field::Autostart => Shown::Check(d.general.autostart),
        Field::Notifications => Shown::Check(d.general.notifications),
        Field::CaptureCursor => Shown::Check(d.general.capture_cursor),
        Field::Crosshair => Shown::Check(d.selection.crosshair),
        Field::Border => Shown::Check(d.scrap.border.visible),
        Field::Shadow => Shown::Check(d.scrap.shadow),
        Field::SaveDir => Shown::Text(
            d.general
                .save_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        Field::Template(of) => Shown::Text(template_of(d, of).to_owned()),
        Field::BorderColor => Shown::Text(d.scrap.border.color.to_string()),
        Field::AnnotationColor => Shown::Text(d.annotation.color.to_string()),
        Field::Opacity => Shown::Text(d.scrap.opacity.to_string()),
        Field::BorderWidth => Shown::Text(d.scrap.border.width.to_string()),
        Field::TrashLimit => Shown::Text(d.general.trash_limit.to_string()),
        Field::SelectionMode => labels(
            &SELECTION_MODES.map(|(_, k)| k),
            SELECTION_MODES
                .iter()
                .position(|(m, _)| *m == d.selection.mode)
                .unwrap_or(0),
        ),
        Field::DpiScaling => labels(
            &DPI_SCALINGS.map(|(_, k)| k),
            DPI_SCALINGS
                .iter()
                .position(|(v, _)| *v == d.general.dpi_scaling)
                .unwrap_or(0),
        ),
        Field::AnnotationWidth => numbers(&WIDTHS, b.annotation.width, d.annotation.width),
        Field::AnnotationFontSize => {
            numbers(&FONT_SIZES, b.annotation.font_size, d.annotation.font_size)
        }
        Field::AnnotationFont => {
            let mut candidates = fonts.to_vec();
            if let Err(i) = candidates.binary_search(&b.annotation.font) {
                candidates.insert(i, b.annotation.font.clone());
            }
            let c = choices(&candidates, &d.annotation.font);
            Shown::Choice {
                items: c.items,
                selected: c.selected,
            }
        }
    }
}

/// 欄の入力。
pub enum Input {
    Check(bool),
    Text(String),
    /// ドロップダウンで選んだ位置と、その文言。
    Choice(usize, String),
}

/// 欄 `field` の入力 `input` を写しに入れる。読めない入力なら何もしない。
pub fn set(field: Field, draft: &mut Settings, input: Input) {
    match (field, input) {
        (Field::Autostart, Input::Check(v)) => draft.general.autostart = v,
        (Field::Notifications, Input::Check(v)) => draft.general.notifications = v,
        (Field::CaptureCursor, Input::Check(v)) => draft.general.capture_cursor = v,
        (Field::Crosshair, Input::Check(v)) => draft.selection.crosshair = v,
        (Field::Border, Input::Check(v)) => draft.scrap.border.visible = v,
        (Field::Shadow, Input::Check(v)) => draft.scrap.shadow = v,
        (Field::SaveDir, Input::Text(s)) => {
            let s = s.trim();
            if s.is_empty() {
                draft.general.save_dir = None;
            } else if is_absolute_dir(s) {
                draft.general.save_dir = Some(PathBuf::from(s));
            }
        }
        (Field::Template(of), Input::Text(s)) if !s.trim().is_empty() => {
            *template_slot(draft, of) = s;
        }
        (Field::BorderColor, Input::Text(s)) => {
            if let Ok(c) = s.trim().parse::<Rgb>() {
                draft.scrap.border.color = c;
            }
        }
        (Field::AnnotationColor, Input::Text(s)) => {
            if let Ok(c) = s.trim().parse::<Rgb>() {
                draft.annotation.color = c;
            }
        }
        (Field::Opacity | Field::BorderWidth | Field::TrashLimit, Input::Text(s)) => {
            let (min, max) = number_range(field);
            let Some(n) = s
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|n| (min..=max).contains(n))
            else {
                return;
            };
            match field {
                Field::Opacity => draft.scrap.opacity = n as u8,
                Field::TrashLimit => draft.general.trash_limit = n as u8,
                _ => draft.scrap.border.width = n,
            }
        }
        (Field::SelectionMode, Input::Choice(i, _)) => {
            if let Some((m, _)) = SELECTION_MODES.get(i) {
                draft.selection.mode = *m;
            }
        }
        (Field::DpiScaling, Input::Choice(i, _)) => {
            if let Some((v, _)) = DPI_SCALINGS.get(i) {
                draft.general.dpi_scaling = *v;
            }
        }
        (Field::AnnotationWidth, Input::Choice(_, s)) => {
            if let Ok(n) = s.parse() {
                draft.annotation.width = n;
            }
        }
        (Field::AnnotationFontSize, Input::Choice(_, s)) => {
            if let Ok(n) = s.parse() {
                draft.annotation.font_size = n;
            }
        }
        (Field::AnnotationFont, Input::Choice(_, s)) if !s.is_empty() => {
            draft.annotation.font = s;
        }
        _ => {}
    }
}

/// 色の欄 `field` の写しの値。
pub fn color_of(settings: &Settings, field: Field) -> Option<Rgb> {
    match field {
        Field::BorderColor => Some(settings.scrap.border.color),
        Field::AnnotationColor => Some(settings.annotation.color),
        _ => None,
    }
}

/// フォルダの欄 `field` で、選ぶダイアログを開く場所（空なら既定の保存先）。
pub fn folder_of(settings: &Settings, field: Field) -> PathBuf {
    let set = match field {
        Field::SaveDir => settings.general.save_dir.clone(),
        _ => None,
    };
    set.or_else(|| save::default_dir().ok()).unwrap_or_default()
}

/// 保存先として受け付けるパス（設定の読み込みと同じく、`C:\…` か `\\server\share\…`）。
fn is_absolute_dir(s: &str) -> bool {
    let b = s.as_bytes();
    let sep = |c: u8| c == b'\\' || c == b'/';
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && sep(b[2]);
    let unc = b.len() >= 3 && sep(b[0]) && sep(b[1]) && !sep(b[2]);
    drive || unc
}

/// テンプレート `of` の写しの値。
pub fn template_of(settings: &Settings, of: TemplateOf) -> &str {
    match of {
        TemplateOf::General => &settings.general.file_name_template,
    }
}

fn template_slot(settings: &mut Settings, of: TemplateOf) -> &mut String {
    match of {
        TemplateOf::General => &mut settings.general.file_name_template,
    }
}

/// テンプレート `of` で保存するときの保存先（None なら全体の既定）と `{name}` の値。
fn preview_target(settings: &Settings, of: TemplateOf) -> (Option<PathBuf>, String) {
    match of {
        TemplateOf::General => (settings.general.save_dir.clone(), String::new()),
    }
}

/// テンプレート `of` の「変数を挿入」に出す変数。`{name}` は定型領域のテンプレートでだけ出す。
pub fn variables(of: TemplateOf) -> impl Iterator<Item = &'static template::Variable> {
    let region = match of {
        TemplateOf::General => false,
    };
    template::VARIABLES
        .iter()
        .filter(move |v| region || !v.region_only)
}

/// いま撮ったら付く名前（保存先の既存のファイルと重ならないもの）。
fn preview(settings: &Settings, of: TemplateOf) -> String {
    let (dir, name) = preview_target(settings, of);
    let dir = match dir {
        Some(dir) => dir,
        None => save::default_dir().unwrap_or_default(),
    };
    match save::destination(&dir, template_of(settings, of), &name, save::now(), &[]) {
        Ok(path) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Err(e) => {
            log::debug!("could not list {}: {e}", dir.display());
            String::new()
        }
    }
}

thread_local! {
    /// `fill` の間。テキストの欄を書き換えると `EN_CHANGE` が来るので、それを入力として扱わない。
    static FILLING: Cell<bool> = const { Cell::new(false) };
}

/// `fill` で欄を書き換えている途中か（届いた通知を入力として扱わない）。
pub fn is_filling() -> bool {
    FILLING.get()
}

/// 写し `editor` を欄に出す。
pub fn fill(hwnd: HWND, fields: &Fields, editor: &Editor, text: Text) {
    FILLING.set(true);
    // SAFETY: 引数なし
    let focus = unsafe { GetFocus() };
    for row in &fields.rows {
        let Some(control) = item(hwnd, row.control) else {
            continue;
        };
        match shown(row.field, editor, &fields.fonts, text) {
            Shown::Check(checked) => {
                // SAFETY: 自分のチェックボックスへのメッセージ
                unsafe {
                    SendMessageW(
                        control,
                        BM_SETCHECK,
                        Some(WPARAM(usize::from(checked))),
                        None,
                    );
                }
            }
            Shown::Text(value) => {
                if control != focus && window_text(control) != value {
                    set_window_text(control, &value);
                }
            }
            Shown::Choice { items, selected } => {
                if combo_items(control) != items {
                    // SAFETY: 自分のドロップダウンへのメッセージ。文字列は呼び出しの間生きている
                    unsafe {
                        SendMessageW(control, CB_RESETCONTENT, None, None);
                        for label in &items {
                            let label = to_wide(label);
                            SendMessageW(
                                control,
                                CB_ADDSTRING,
                                None,
                                Some(LPARAM(label.as_ptr() as isize)),
                            );
                        }
                    }
                }
                // SAFETY: 自分のドロップダウンへのメッセージ
                unsafe {
                    SendMessageW(control, CB_SETCURSEL, Some(WPARAM(selected)), None);
                }
            }
        }
        if let (Kind::Text { preview: Some(id) }, Field::Template(of)) = (row.kind, row.field) {
            let line = format!(
                "{}{}",
                text("settings_gui.template.preview"),
                preview(editor.draft(), of)
            );
            let line = to_wide(&line);
            // SAFETY: 文字列は呼び出しの間生きている
            unsafe {
                let _ = SetDlgItemTextW(hwnd, id, PCWSTR(line.as_ptr()));
            }
        }
    }
    FILLING.set(false);
}

/// 欄 `row`（コントロール `control`）の今の入力。
pub fn input(row: FieldRow, control: HWND) -> Input {
    match row.kind {
        // SAFETY: 自分のチェックボックスへのメッセージ
        Kind::Check => {
            Input::Check(unsafe { SendMessageW(control, BM_GETCHECK, None, None) }.0 == 1)
        }
        Kind::Text { .. } | Kind::Number { .. } => Input::Text(window_text(control)),
        Kind::Choice => {
            // SAFETY: 自分のドロップダウンへのメッセージ
            let i = unsafe { SendMessageW(control, CB_GETCURSEL, None, None) }.0;
            let i = usize::try_from(i).unwrap_or(0);
            Input::Choice(i, combo_text(control, i))
        }
    }
}

/// 組み直したダイアログで 1 回だけすること（数の欄の範囲、保存先の欄の既定の表示）。
pub fn init(hwnd: HWND, fields: &Fields, text: Text) {
    for row in &fields.rows {
        match row.kind {
            Kind::Number { updown } => {
                let (min, max) = number_range(row.field);
                if let Some(updown) = item(hwnd, updown) {
                    // SAFETY: 自分のコントロールへのメッセージ
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
            Kind::Text { .. } if row.field == Field::SaveDir => {
                let default = save::default_dir()
                    .map(|d| {
                        format!(
                            "{}{}",
                            text("settings_gui.general.default_dir"),
                            d.display()
                        )
                    })
                    .unwrap_or_default();
                let default = to_wide(&default);
                if let Some(control) = item(hwnd, row.control) {
                    // SAFETY: 文字列は呼び出しの間生きている（コントロールが写す）
                    unsafe {
                        SendMessageW(
                            control,
                            EM_SETCUEBANNER,
                            Some(WPARAM(1)),
                            Some(LPARAM(default.as_ptr() as isize)),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// ウィンドウの文字列。
pub fn window_text(hwnd: HWND) -> String {
    // SAFETY: バッファは長さ + 1 で確保する
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

/// ウィンドウの文字列を変える。
pub fn set_window_text(hwnd: HWND, value: &str) {
    let value = to_wide(value);
    // SAFETY: 文字列は呼び出しの間生きている
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(value.as_ptr()));
    }
}

fn combo_items(combo: HWND) -> Vec<String> {
    // SAFETY: 自分のドロップダウンへのメッセージ
    let count = unsafe { SendMessageW(combo, CB_GETCOUNT, None, None) }
        .0
        .max(0) as usize;
    (0..count).map(|i| combo_text(combo, i)).collect()
}

fn combo_text(combo: HWND, index: usize) -> String {
    // SAFETY: バッファは長さ + 1 で確保する
    unsafe {
        let len = SendMessageW(combo, CB_GETLBTEXTLEN, Some(WPARAM(index)), None).0;
        let Ok(len) = usize::try_from(len) else {
            return String::new();
        };
        let mut buf = vec![0u16; len + 1];
        let n = SendMessageW(
            combo,
            CB_GETLBTEXT,
            Some(WPARAM(index)),
            Some(LPARAM(buf.as_mut_ptr() as isize)),
        )
        .0;
        String::from_utf16_lossy(&buf[..usize::try_from(n).unwrap_or(0).min(len)])
    }
}

/// インストール済みの書体の名前（昇順、重なりなし）。縦書きの `@` の付いたものは除く。
pub fn installed_fonts() -> Vec<String> {
    unsafe extern "system" fn each(
        font: *const LOGFONTW,
        _: *const TEXTMETRICW,
        _: u32,
        lparam: LPARAM,
    ) -> i32 {
        // SAFETY: font は列挙の間生きている。lparam は installed_fonts の Vec
        unsafe {
            let name = &(*font).lfFaceName;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            let name = String::from_utf16_lossy(&name[..len]);
            if !name.is_empty() && !name.starts_with('@') {
                (*(lparam.0 as *mut Vec<String>)).push(name);
            }
        }
        1
    }
    let mut names: Vec<String> = Vec::new();
    let query = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    // SAFETY: 画面の DC は使ったら返す。コールバックは呼び出しの間だけ names を指す
    unsafe {
        let dc = GetDC(None);
        EnumFontFamiliesExW(
            dc,
            &query,
            Some(each),
            LPARAM(&mut names as *mut Vec<String> as isize),
            0,
        );
        ReleaseDC(None, dc);
    }
    names.sort();
    names.dedup();
    names
}

impl Fields {
    /// 書体の一覧を入れた空の表。
    pub fn new() -> Fields {
        Fields {
            rows: Vec::new(),
            buttons: Vec::new(),
            fonts: Rc::new(installed_fonts()),
        }
    }
}

/// 色を選ばせる。キャンセルなら None。モーダルなので `App` を借りずに呼ぶ。
pub fn pick_color(owner: HWND, current: Rgb) -> Option<Rgb> {
    thread_local! {
        /// 「作成した色」。開くたびに引き継ぐ。
        static CUSTOM: Cell<[COLORREF; 16]> = const { Cell::new([COLORREF(0x00FF_FFFF); 16]) };
    }
    let Rgb(r, g, b) = current;
    let mut custom = CUSTOM.get();
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner,
        rgbResult: COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    // SAFETY: 構造体と「作成した色」は呼び出しの間生きている
    let ok = unsafe { ChooseColorW(&mut cc) }.as_bool();
    CUSTOM.set(custom);
    ok.then(|| {
        let c = cc.rgbResult.0;
        Rgb(c as u8, (c >> 8) as u8, (c >> 16) as u8)
    })
}

impl PageBuilder<'_> {
    /// テキストの欄（枠付き）。ほかのコントロールは `PageBuilder::control` で置く。
    fn edit(&mut self, style: WINDOW_STYLE, x: i16, y: i16, cx: i16) -> i32 {
        let id = self.next_id();
        self.template.item(Item {
            class: Class::Edit,
            text: "",
            id,
            style: WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32) | style,
            ex_style: WS_EX_CLIENTEDGE,
            rect: (PAGE_X + x, PAGE_Y + y, cx, 14),
        });
        id
    }

    /// 文言のキー `key` のチェックボックス。
    pub fn check(&mut self, field: Field, key: &'static str, x: i16, y: i16, cx: i16) {
        let text = (self.text)(key);
        let style = WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32);
        let id = self.control(Class::Button, text, style, x, y, cx, 12);
        self.push(field, id, Kind::Check);
    }

    /// テキストの欄。
    pub fn text(&mut self, field: Field, x: i16, y: i16, cx: i16) -> i32 {
        let id = self.edit(WINDOW_STYLE(0), x, y, cx);
        self.push(field, id, kind(field));
        id
    }

    /// 数の欄（上下のボタン付き）。
    pub fn number(&mut self, field: Field, x: i16, y: i16, cx: i16) {
        let id = self.edit(WINDOW_STYLE(ES_NUMBER as u32), x, y, cx);
        // 上下のボタンは直前の欄（Z 順の前）に付く
        let style = WINDOW_STYLE(
            UDS_AUTOBUDDY | UDS_ALIGNRIGHT | UDS_ARROWKEYS | UDS_SETBUDDYINT | UDS_NOTHOUSANDS,
        );
        let updown = self.control(Class::UpDown, "", style, x + cx, y, 10, 14);
        self.push(field, id, Kind::Number { updown });
    }

    /// ドロップダウン（`list` は開いたときの一覧の高さ）。
    pub fn choice(&mut self, field: Field, x: i16, y: i16, cx: i16, list: i16) {
        let style = WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32);
        let id = self.control(Class::ComboBox, "", style, x, y, cx, list);
        self.push(field, id, Kind::Choice);
    }

    /// 欄の横のボタン。
    pub fn button(&mut self, action: Action, key: &'static str, x: i16, y: i16, cx: i16) {
        let text = (self.text)(key);
        let style = WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32);
        let id = self.control(Class::Button, text, style, x, y, cx, 14);
        self.fields.buttons.push(ButtonRow { id, action });
    }

    /// 保存先の欄と「参照…」（幅は合わせて `cx`）。
    pub fn folder_field(&mut self, field: Field, x: i16, y: i16, cx: i16) {
        self.text(field, x, y, cx - BUTTON_GAP - SIDE_BUTTON);
        self.button(
            Action::BrowseFolder(field),
            "settings_gui.browse",
            x + cx - SIDE_BUTTON,
            y,
            SIDE_BUTTON,
        );
    }

    /// 色の欄（`#RRGGBB`）と「選ぶ…」。
    pub fn color_field(&mut self, field: Field, x: i16, y: i16) {
        self.text(field, x, y, 50);
        self.button(
            Action::PickColor(field),
            "settings_gui.pick_color",
            x + 50 + BUTTON_GAP,
            y,
            SIDE_BUTTON,
        );
    }

    /// ファイル名テンプレートの欄と「変数を挿入」（幅は合わせて `cx`）、その下のプレビュー。プレビューを
    /// 含めた高さは 28。
    pub fn template_field(&mut self, of: TemplateOf, x: i16, y: i16, cx: i16) {
        let field = Field::Template(of);
        let id = self.text(field, x, y, cx - BUTTON_GAP - SIDE_BUTTON);
        self.button(
            Action::InsertVariable(of),
            "settings_gui.insert_variable",
            x + cx - SIDE_BUTTON,
            y,
            SIDE_BUTTON,
        );
        let preview = self.control(Class::Static, "", SS_LEFT, x, y + 17, cx, 10);
        if let Some(row) = self.fields.rows.iter_mut().find(|r| r.control == id) {
            row.kind = Kind::Text {
                preview: Some(preview),
            };
        }
    }

    fn push(&mut self, field: Field, control: i32, kind: Kind) {
        self.fields.rows.push(FieldRow {
            field,
            control,
            kind,
        });
    }
}

/// 欄の横のボタンの幅と、欄との間。
const SIDE_BUTTON: i16 = 50;
const BUTTON_GAP: i16 = 4;
