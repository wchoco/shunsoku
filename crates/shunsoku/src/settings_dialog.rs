//! 設定 GUI のダイアログ（ADR-0004）。テンプレートをメモリ上で組み、`CreateDialogIndirectParamW` でモードレスに開く。
//! アプリの状態は持たない。写し（core の `settings::editor::Editor`）を欄に出すのと、欄の場所を返すのがここの仕事で、
//! 入力を受けて写しを変えるのは呼び出し側（`app::settings_gui`）。
//!
//! - 組むのは `create` の 1 か所。文言はそのときにカタログ（`text`）から入れるので、言語を変えたら同じページで
//!   組み直せる（ダイアログを壊して `create` し直し、`fill` で写しを出す）。
//! - タブのページは `PAGES` の表の行。ページを足すときは `Page` に足し、`PAGES` にタブの文言のキーと、コントロールを
//!   置く関数（`PageBuilder` を受ける）の行を足す。ページの中の座標はタブの中の左上（`PAGE_X`, `PAGE_Y`）からの
//!   ダイアログ単位。コントロールの ID はページごとの帯（`PAGE_ID_FIRST` + ページの添字 × `PAGE_ID_SPAN`）から振り、
//!   タブを切り替えたら帯で出し分ける（`show_page`）。
//! - キーの組み合わせの欄（ホットキー・ショートカット）は `PageBuilder::chord_field` で置く。欄（`chord_field`）と、
//!   その横の印の欄を組にして `ChordRow` として返す。呼び出し側は `ChordRow::item` で写しの項目と結ぶ。
//! - それ以外の欄（チェックボックス・テキスト・数・ドロップダウンと、横の「参照…」などのボタン）は `fields`。
//!   一般・スクラップ・書き込みのタブは `pages`、定型領域のタブは `regions`、スクラップ操作のタブは `scrap_ops`。
//! - タイトルバーだけダークモードに合わせる（`apply_dark`）。DPI の変化は PMv2 のダイアログマネージャに任せる。

pub mod chord_field;
pub mod fields;
mod list_view;
mod pages;
pub mod regions;
pub mod scrap_ops;

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::thread::LocalKey;

use shunsoku_core::monitors::Monitor;
use shunsoku_core::settings::SettingsItem;
use shunsoku_core::settings::editor::Editor;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Controls::{
    EnableThemeDialogTexture, ICC_HOTKEY_CLASS, ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES,
    ICC_TAB_CLASSES, ICC_UPDOWN_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, TCIF_TEXT,
    TCITEMW, TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateDialogIndirectParamW, DLGPROC, DLGTEMPLATE, DS_CENTER,
    DS_MODALFRAME, DS_SETFONT, EnumChildWindows, GWLP_ID, GetDlgItem, GetWindowLongPtrW, IDCANCEL,
    IDOK, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SendMessageW, SetDlgItemTextW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
    WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT,
    WS_GROUP, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{BOOL, PCWSTR, Result, w};

use crate::wide::to_wide;
pub use fields::Fields;

/// 適用のボタン。OK とキャンセルは `IDOK` / `IDCANCEL`。
pub const ID_APPLY: i32 = 10;
/// タブ。
pub const ID_TAB: i32 = 100;
/// Static の左寄せ（`SS_LEFT`）。
const SS_LEFT: WINDOW_STYLE = WINDOW_STYLE(0);
/// ダイアログの背景をタブの中の色で塗る（`ETDT_ENABLE | ETDT_USETABTEXTURE`）。ページの Static の背景がタブと揃う。
const ETDT_ENABLETAB: u32 = 6;
/// ページのコントロールの ID の帯。
const PAGE_ID_FIRST: i32 = 1000;
const PAGE_ID_SPAN: i32 = 100;

/// ダイアログの大きさ（ダイアログ単位）。
const WIDTH: i16 = 340;
const HEIGHT: i16 = 250;
const MARGIN: i16 = 7;
const BUTTON_WIDTH: i16 = 50;
const BUTTON_HEIGHT: i16 = 14;
/// ページの中の左上（タブの見出しの下）。
pub const PAGE_X: i16 = MARGIN + 8;
pub const PAGE_Y: i16 = MARGIN + 20;
/// ページに使える幅。
pub const PAGE_WIDTH: i16 = WIDTH - 2 * PAGE_X;

/// タブのページ。既定（開くページを指定しないとき）は最初の「全般」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    General,
    Capture,
    Regions,
    Scrap,
    ScrapOps,
    Annotation,
}

/// タブのページの表の行。
struct PageDef {
    page: Page,
    label_key: &'static str,
    build: fn(&mut PageBuilder),
}

/// タブの順。
const PAGES: &[PageDef] = &[
    PageDef {
        page: Page::General,
        label_key: "settings_gui.tab.general",
        build: pages::general,
    },
    PageDef {
        page: Page::Capture,
        label_key: "settings_gui.tab.capture",
        build: capture_page,
    },
    PageDef {
        page: Page::Regions,
        label_key: "settings_gui.tab.regions",
        build: regions::build,
    },
    PageDef {
        page: Page::Scrap,
        label_key: "settings_gui.tab.scrap",
        build: pages::scrap,
    },
    PageDef {
        page: Page::ScrapOps,
        label_key: "settings_gui.tab.scrap_ops",
        build: scrap_ops::build,
    },
    PageDef {
        page: Page::Annotation,
        label_key: "settings_gui.tab.annotation",
        build: pages::annotation,
    },
];

/// ページの左の見出しの幅と、欄の左端。見出しは欄より 2 下げる。
pub const LABEL_WIDTH: i16 = 100;
pub const FIELD_X: i16 = LABEL_WIDTH + 5;

/// 「キャプチャ」タブ。
fn capture_page(b: &mut PageBuilder) {
    use fields::Field;
    let hotkeys = [
        (
            "settings_gui.capture.select_capture",
            SettingsItem::SelectCaptureHotkey,
        ),
        (
            "settings_gui.capture.active_window_capture",
            SettingsItem::ActiveWindowCaptureHotkey,
        ),
        (
            "settings_gui.capture.restore_last",
            SettingsItem::RestoreLastHotkey,
        ),
    ];
    let mut y = 0;
    for (key, item) in hotkeys {
        b.label(key, 0, y + 2, LABEL_WIDTH, 10);
        b.chord_field(item, FIELD_X, y, 110);
        y += 34;
    }
    b.check(
        Field::CaptureCursor,
        "settings_gui.capture.capture_cursor",
        0,
        y,
        PAGE_WIDTH,
    );
    y += 16;
    b.label("settings_gui.capture.mode", 0, y + 2, LABEL_WIDTH, 10);
    b.choice(Field::SelectionMode, FIELD_X, y, 110, 60);
    y += 20;
    b.check(
        Field::Crosshair,
        "settings_gui.capture.crosshair",
        0,
        y,
        PAGE_WIDTH,
    );
    y += 18;
    b.label("settings_gui.capture.hint", 0, y, PAGE_WIDTH, 20);
}

/// 文言を引く関数（`app::text`）。
pub type Text = fn(&'static str) -> &'static str;

/// フォルダ `dir` に、ファイル名テンプレート `template`（`{name}` は `name`）でいま撮ったら付く名前
/// （`app` が `save` で決める）。決められなければ None。
pub type FileName = fn(dir: &Path, template: &str, name: &str) -> Option<String>;

/// ダイアログの外から要るもの。`app` が集めて渡す（ADR-0005: Win32 のモジュールは `app` を通して互いを使う）。
#[derive(Debug, Clone)]
pub struct Env {
    /// いまのモニタ（定型領域のモニタの名前・はみ出しの警告・追加の場所・範囲の絵）。
    pub monitors: Vec<Monitor>,
    /// 全体の保存先が空のときに保存する先（ピクチャ）。
    pub default_dir: Option<PathBuf>,
    pub file_name: FileName,
}

/// 処理の間だけ旗を立てておく（その処理がコントロールに送ったメッセージで届く通知を、ダイアログが見分ける）。
/// 落としたら旗を下ろす。
pub struct During(&'static LocalKey<Cell<bool>>);

impl During {
    pub fn enter(flag: &'static LocalKey<Cell<bool>>) -> During {
        flag.set(true);
        During(flag)
    }
}

impl Drop for During {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// キーの組み合わせの欄と、その横の印の欄。
#[derive(Debug, Clone)]
pub struct ChordRow {
    pub item: SettingsItem,
    /// 欄のコントロールの ID。
    pub field: i32,
    /// 印の欄（Static）の ID。
    pub mark: i32,
}

/// 組んだダイアログ。
pub struct Dialog {
    pub hwnd: HWND,
    pub rows: Vec<ChordRow>,
    pub fields: Fields,
}

/// 設定 GUI を組んで（見えないまま）作る。`page` のタブを選んでおく。`proc` がダイアログ プロシージャ。
/// `WM_INITDIALOG` は、タブの見出しを入れる前に `proc` に届く。
pub fn create(
    module: HINSTANCE,
    page: Page,
    text: Text,
    env: &Env,
    proc: DLGPROC,
) -> Result<Dialog> {
    let icc = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_HOTKEY_CLASS
            | ICC_STANDARD_CLASSES
            | ICC_TAB_CLASSES
            | ICC_LISTVIEW_CLASSES
            | ICC_UPDOWN_CLASS,
    };
    // SAFETY: 構造体はこの呼び出しの間生きている
    unsafe {
        let _ = InitCommonControlsEx(&icc);
    }

    let mut t = Template::new(text("settings_gui.title"), text("settings_gui.font"));
    let mut rows = Vec::new();
    let mut fields = Fields::new();
    for (index, def) in PAGES.iter().enumerate() {
        let mut b = PageBuilder {
            template: &mut t,
            text,
            next_id: PAGE_ID_FIRST + index as i32 * PAGE_ID_SPAN,
            rows: &mut rows,
            fields: &mut fields,
        };
        (def.build)(&mut b);
    }
    let button_y = HEIGHT - MARGIN - BUTTON_HEIGHT;
    let button_x = |n: i16| WIDTH - MARGIN - n * BUTTON_WIDTH - (n - 1) * 4;
    t.item(Item {
        class: Class::Button,
        text: text("settings_gui.ok"),
        id: IDOK.0,
        style: WS_TABSTOP | WS_GROUP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
        ex_style: WINDOW_EX_STYLE(0),
        rect: (button_x(3), button_y, BUTTON_WIDTH, BUTTON_HEIGHT),
    });
    t.item(Item {
        class: Class::Button,
        text: text("settings_gui.cancel"),
        id: IDCANCEL.0,
        style: WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
        ex_style: WINDOW_EX_STYLE(0),
        rect: (button_x(2), button_y, BUTTON_WIDTH, BUTTON_HEIGHT),
    });
    t.item(Item {
        class: Class::Button,
        text: text("settings_gui.apply"),
        id: ID_APPLY,
        style: WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
        ex_style: WINDOW_EX_STYLE(0),
        rect: (button_x(1), button_y, BUTTON_WIDTH, BUTTON_HEIGHT),
    });
    // タブはページのコントロールより下に重ねる（テンプレートの後ろほど Z 順が下）。兄弟を描きつぶさないよう切り抜く
    t.item(Item {
        class: Class::Tab,
        text: "",
        id: ID_TAB,
        style: WS_TABSTOP | WS_GROUP | WS_CLIPSIBLINGS,
        ex_style: WINDOW_EX_STYLE(0),
        rect: (MARGIN, MARGIN, WIDTH - 2 * MARGIN, button_y - 6 - MARGIN),
    });

    let buffer = t.finish();
    // SAFETY: テンプレートは DWORD 境界にそろえた Vec<u32> で、呼び出しの間生きている
    let hwnd = unsafe {
        CreateDialogIndirectParamW(
            Some(module),
            buffer.as_ptr().cast::<DLGTEMPLATE>(),
            None,
            proc,
            LPARAM(0),
        )?
    };

    for row in &rows {
        if let Some(field) = item(hwnd, row.field) {
            chord_field::attach(field, text("settings_gui.chord_none"));
        }
    }
    fields::init(hwnd, &fields, text, env);
    if let Some(tab) = item(hwnd, ID_TAB) {
        for (index, def) in PAGES.iter().enumerate() {
            let mut label = to_wide(text(def.label_key));
            let entry = TCITEMW {
                mask: TCIF_TEXT,
                pszText: windows::core::PWSTR(label.as_mut_ptr()),
                ..Default::default()
            };
            // SAFETY: 見出しの文字列は呼び出しの間生きている（タブが写す）
            unsafe {
                SendMessageW(
                    tab,
                    TCM_INSERTITEMW,
                    Some(WPARAM(index)),
                    Some(LPARAM(&entry as *const TCITEMW as isize)),
                );
            }
        }
    }
    // SAFETY: 作ったばかりの自分のダイアログ
    if let Err(e) = unsafe { EnableThemeDialogTexture(hwnd, ETDT_ENABLETAB) } {
        log::warn!("could not enable the tab texture: {e}");
    }
    show_page(hwnd, page);
    apply_dark(hwnd);
    Ok(Dialog { hwnd, rows, fields })
}

/// ページ `page` のコントロールの ID の帯の最初。
fn page_first_id(page: Page) -> i32 {
    let index = PAGES.iter().position(|def| def.page == page).unwrap_or(0);
    PAGE_ID_FIRST + index as i32 * PAGE_ID_SPAN
}

/// 選んでいるタブのページ。
pub fn current_page(hwnd: HWND) -> Page {
    let index = item(hwnd, ID_TAB)
        // SAFETY: 自分のダイアログのタブへのメッセージ
        .map(|tab| unsafe { SendMessageW(tab, TCM_GETCURSEL, None, None) }.0)
        .unwrap_or(0);
    PAGES
        .get(usize::try_from(index).unwrap_or(0))
        .map_or_else(Page::default, |def| def.page)
}

/// `page` のタブを選び、そのページのコントロールだけを出す。
pub fn show_page(hwnd: HWND, page: Page) {
    let index = PAGES.iter().position(|def| def.page == page).unwrap_or(0);
    if let Some(tab) = item(hwnd, ID_TAB) {
        // SAFETY: 自分のダイアログのタブへのメッセージ
        unsafe {
            SendMessageW(tab, TCM_SETCURSEL, Some(WPARAM(index)), None);
        }
    }
    let first = PAGE_ID_FIRST + index as i32 * PAGE_ID_SPAN;
    let shown = first..first + PAGE_ID_SPAN;
    let mut children = Vec::new();
    // SAFETY: コールバックは呼び出しの間だけ children を指す
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(collect_child),
            LPARAM(&mut children as *mut Vec<HWND> as isize),
        );
    }
    let all = PAGE_ID_FIRST..PAGE_ID_FIRST + PAGES.len() as i32 * PAGE_ID_SPAN;
    for child in children {
        // SAFETY: 子ウィンドウの ID を読む
        let id = unsafe { GetWindowLongPtrW(child, GWLP_ID) } as i32;
        if all.contains(&id) {
            // SAFETY: 自分の子ウィンドウ
            unsafe {
                let _ = ShowWindow(
                    child,
                    if shown.contains(&id) {
                        SW_SHOW
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
    }
}

unsafe extern "system" fn collect_child(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam は show_page の Vec<HWND>
    unsafe { (*(lparam.0 as *mut Vec<HWND>)).push(hwnd) };
    BOOL(1)
}

/// 写し `editor` を欄に出す: キーの組み合わせの欄の値、横の印、OK と適用を押せるか。
pub fn fill(hwnd: HWND, rows: &[ChordRow], editor: &Editor, text: Text, env: &Env) {
    for row in rows {
        if let Some(field) = item(hwnd, row.field) {
            chord_field::set(field, editor.chord(&row.item));
        }
        let mark = editor.mark(&row.item).map_or("", |m| text(m.label_key()));
        let mark = to_wide(mark);
        // SAFETY: 文字列は呼び出しの間生きている
        unsafe {
            let _ = SetDlgItemTextW(hwnd, row.mark, PCWSTR(mark.as_ptr()));
        }
    }
    scrap_ops::fill(hwnd, editor, text);
    regions::fill(hwnd, editor, text, env);
    let buttons = editor.buttons();
    for (id, enabled) in [(IDOK.0, buttons.ok), (ID_APPLY, buttons.apply)] {
        if let Some(button) = item(hwnd, id) {
            // SAFETY: 自分のダイアログのボタン
            unsafe {
                let _ = EnableWindow(button, enabled);
            }
        }
    }
}

/// ダイアログの中の ID `id` のコントロール。
pub fn item(hwnd: HWND, id: i32) -> Option<HWND> {
    // SAFETY: 自分のダイアログの子を探すだけ
    unsafe { GetDlgItem(Some(hwnd), id) }.ok()
}

/// タイトルバーを、いまのアプリのモード（ダーク / ライト）に合わせる。モードが変わったとき（`WM_SETTINGCHANGE` の
/// `ImmersiveColorSet`）にも呼ぶ。
pub fn apply_dark(hwnd: HWND) {
    let dark = BOOL::from(apps_use_dark());
    // SAFETY: 値は呼び出しの間生きている。枠の描き直しを頼むだけ
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// アプリのモードがダークか（`AppsUseLightTheme` が 0）。読めなければライト。
fn apps_use_dark() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: 値とサイズは呼び出しの間生きている
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 0
}

/// ページのコントロールを置く。座標はページの左上からのダイアログ単位。
pub struct PageBuilder<'a> {
    template: &'a mut Template,
    text: Text,
    next_id: i32,
    rows: &'a mut Vec<ChordRow>,
    fields: &'a mut Fields,
}

impl PageBuilder<'_> {
    fn next_id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// 文言のキー `key` のラベル。
    pub fn label(&mut self, key: &'static str, x: i16, y: i16, cx: i16, cy: i16) {
        let id = self.next_id();
        let text = (self.text)(key);
        self.template.item(Item {
            class: Class::Static,
            text,
            id,
            style: SS_LEFT,
            ex_style: WINDOW_EX_STYLE(0),
            rect: (PAGE_X + x, PAGE_Y + y, cx, cy),
        });
    }

    /// クラス `class` のコントロールを置き、その ID を返す。
    #[allow(clippy::too_many_arguments)]
    fn control(
        &mut self,
        class: Class,
        text: &str,
        style: WINDOW_STYLE,
        x: i16,
        y: i16,
        cx: i16,
        cy: i16,
    ) -> i32 {
        let id = self.next_id();
        self.template.item(Item {
            class,
            text,
            id,
            style,
            ex_style: WINDOW_EX_STYLE(0),
            rect: (PAGE_X + x, PAGE_Y + y, cx, cy),
        });
        id
    }

    /// 項目 `item` のキーの組み合わせの欄（幅 `cx`）と、その下の印の欄。
    pub fn chord_field(&mut self, item: SettingsItem, x: i16, y: i16, cx: i16) {
        let field = self.next_id();
        self.template.item(Item {
            class: Class::HotKey,
            text: "",
            id: field,
            style: WS_TABSTOP,
            ex_style: WS_EX_CLIENTEDGE,
            rect: (PAGE_X + x, PAGE_Y + y, cx, 14),
        });
        let mark = self.next_id();
        self.template.item(Item {
            class: Class::Static,
            text: "",
            id: mark,
            style: SS_LEFT,
            ex_style: WINDOW_EX_STYLE(0),
            rect: (PAGE_X + x, PAGE_Y + y + 16, PAGE_WIDTH - x, 18),
        });
        self.rows.push(ChordRow { item, field, mark });
    }
}

/// コントロールのクラス。
#[derive(Clone, Copy)]
enum Class {
    Button,
    Static,
    Edit,
    UpDown,
    HotKey,
    Tab,
    ListView,
    ComboBox,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Button => "Button",
            Class::Static => "Static",
            Class::Edit => "Edit",
            Class::UpDown => "msctls_updown32",
            Class::HotKey => "msctls_hotkey32",
            Class::Tab => "SysTabControl32",
            Class::ListView => "SysListView32",
            Class::ComboBox => "ComboBox",
        }
    }
}

/// テンプレートの 1 コントロール。
struct Item<'a> {
    class: Class,
    text: &'a str,
    id: i32,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    /// x, y, 幅, 高さ（ダイアログ単位）。
    rect: (i16, i16, i16, i16),
}

/// メモリ上のダイアログのテンプレート（`DLGTEMPLATEEX` に `DLGITEMTEMPLATEEX` を並べたもの）。
struct Template {
    bytes: Vec<u8>,
    count: u16,
}

/// `DLGTEMPLATEEX` の `cDlgItems` の位置（dlgVer, signature, helpID, exStyle, style の後）。
const COUNT_OFFSET: usize = 2 + 2 + 4 + 4 + 4;

impl Template {
    fn new(title: &str, font: &str) -> Template {
        let mut t = Template {
            bytes: Vec::new(),
            count: 0,
        };
        let style = WS_POPUP
            | WS_CAPTION
            | WS_SYSMENU
            | WS_MINIMIZEBOX
            | WINDOW_STYLE((DS_MODALFRAME | DS_SETFONT | DS_CENTER) as u32);
        t.u16(1); // dlgVer
        t.u16(0xFFFF); // signature
        t.u32(0); // helpID
        t.u32((WS_EX_APPWINDOW | WS_EX_CONTROLPARENT).0);
        t.u32(style.0);
        t.u16(0); // cDlgItems（finish で入れる）
        for v in [0, 0, WIDTH, HEIGHT] {
            t.u16(v as u16);
        }
        t.u16(0); // メニューなし
        t.u16(0); // 既定のダイアログのクラス
        t.string(title);
        t.u16(9); // ポイント
        t.u16(400); // 太さ
        t.bytes.push(0); // 斜体
        t.bytes.push(1); // DEFAULT_CHARSET
        t.string(font);
        t
    }

    fn item(&mut self, item: Item) {
        self.align4();
        self.u32(0); // helpID
        self.u32(item.ex_style.0);
        self.u32((item.style | WS_CHILD | WS_VISIBLE).0);
        let (x, y, cx, cy) = item.rect;
        for v in [x, y, cx, cy] {
            self.u16(v as u16);
        }
        self.u32(item.id as u32);
        self.string(item.class.name());
        self.string(item.text);
        self.u16(0); // extraCount
        self.count += 1;
    }

    /// DWORD 境界にそろえた中身。
    fn finish(mut self) -> Vec<u32> {
        let count = self.count.to_le_bytes();
        self.bytes[COUNT_OFFSET..COUNT_OFFSET + 2].copy_from_slice(&count);
        self.align4();
        self.bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn string(&mut self, s: &str) {
        for unit in s.encode_utf16().chain([0]) {
            self.u16(unit);
        }
    }

    fn align4(&mut self) {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
    }
}
