//! 設定ファイル（`%APPDATA%\shunsoku\settings.toml`）の読み込み。
//!
//! - `load` は項目ごとに値を取り出す。不正な項目はその項目だけ既定値にし、項目名を `SettingsIssue` に載せる。
//!   知らないキーは無視する。ファイル全体が TOML として読めなければ、すべて既定値にする。
//! - アプリから書き戻すときは `write`（`settings/write.rs`）で変わった項目だけを書き換え、手で書いたコメントと
//!   並び順を残す。自分で書いた直後の中身を監視が拾ったら、`OwnWrites` で見分けて読み込み直さない。
//! - 読み込み直しでは `diff` で変わった部分を調べ、`app` が変わったものだけを反映し直す。
//! - キーの重なりは `keymap` で見つける。同じ範囲（ホットキー同士、ショートカット同士）の重なりは `assign` の
//!   規則で両方を無効にし、ホットキーとショートカットの重なりは警告だけにする。
//!
//! スキーマ（書かなかった項目は既定値）:
//!
//! ```toml
//! version = 1                                  # 必須。1 だけ
//! log_level = "info"                           # 隠し項目: off / error / warn / info / debug / trace
//!
//! [general]
//! save_dir = 'D:\shots'                        # 既定: ピクチャ フォルダの下の shunsoku
//! file_name_template = "shunsoku_{date}-{time}"
//! notifications = true                         # false で止まるのは成功の通知だけ
//! trash_limit = 10                             # ごみ箱の上限（0〜100 件）。0 なら閉じたらすぐに破棄
//! dpi_scaling = "keep_pixels"                  # DPI の違うモニタへ動かしたとき: keep_pixels（画素を等倍に保つ）
//!                                              #   / keep_apparent_size（見かけの大きさを保つ）
//! capture_cursor = false                       # キャプチャにマウスカーソルを写す（全方式で共通）
//!
//! [hotkeys]
//! select_capture = "Ctrl+Shift+A"              # 既定は空（未設定）。"" も未設定
//! restore_last = "Ctrl+Shift+Z"                # 最後に閉じたものを復元。既定は空で、必須ではない
//! active_window_capture = "Ctrl+Shift+W"       # アクティブウィンドウのキャプチャ。既定は空で、必須ではない
//!
//! [shortcuts]                                  # スクラップ操作の id ごとの上書き。"" は割り当てなし
//! copy = "Ctrl+Shift+C"
//!
//! [mouse]                                      # スクラップ操作の id ごとのマウスの上書き。"" は割り当てなし
//! close = "DoubleClick"                        # WheelUp / WheelDown / Ctrl+WheelUp / Ctrl+WheelDown /
//! toggle_compact = "MiddleClick"               #   MiddleClick / DoubleClick
//!
//! [scrap]                                      # 見た目の既定値
//! opacity = 100                                # 10〜100（%）
//! border = true
//! border_width = 1                             # 1〜16（px）
//! border_color = "#808080"
//! shadow = true
//!
//! [selection]                                  # 範囲選択の画面
//! mode = "normal"                              # 開いたときのモード: normal / precise
//! crosshair = false                            # 通常モードでも十字線を出す（精密モードでは常に出す）
//! [[regions]]                                  # 定型領域。並び順がトレイのサブメニューの順
//! name = "領域 1"                               # 必須。空と重複は不可（重複した後ろのものを捨てる）
//! hotkey = "Ctrl+Alt+1"                        # 既定は空
//! monitor = '\\?\DISPLAY#…'                    # 必須。monitor::MonitorInfo::id
//! x = 0                                        # 必須。モニタ相対の物理ピクセル。x / y >= 0、
//! y = 0                                        #   width / height >= 1
//! width = 800
//! height = 600
//! save_dir = 'D:\shots'                        # 既定は全体の保存先
//! file_name_template = "{name}_{date}-{time}"
//! save = true                                  # save と scrap の両方 false は save を true に戻す
//! scrap = false
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use toml_edit::{DocumentMut, Item, TableLike};

use crate::geometry::Rect;
use crate::keys::Chord;
use crate::scrap::ops::{MouseBindings, MouseGesture, OperationDescriptor, Shortcuts};
use crate::selection::Mode;

mod write;
pub use write::{OwnWrites, rewrite, write};

/// 対応している設定ファイルの形式の版。
pub const VERSION: i64 = 1;

/// 全体のファイル名テンプレートの既定値。
pub const DEFAULT_FILE_NAME_TEMPLATE: &str = "shunsoku_{date}-{time}";

/// 透明度（%）として書ける範囲。
pub const OPACITY_RANGE: std::ops::RangeInclusive<u8> = 10..=100;

/// ごみ箱の上限（件）の既定値。
pub const DEFAULT_TRASH_LIMIT: u8 = 10;

/// ごみ箱の上限（件）として書ける範囲。
pub const TRASH_LIMIT_RANGE: std::ops::RangeInclusive<u8> = 0..=100;

/// 枠の太さ（px）として書ける範囲。
pub const BORDER_WIDTH_RANGE: std::ops::RangeInclusive<u32> = 1..=16;

/// 設定。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub general: General,
    pub hotkeys: Hotkeys,
    pub shortcuts: ShortcutOverrides,
    pub mouse: MouseOverrides,
    pub scrap: ScrapAppearance,
    pub selection: SelectionScreen,
    /// ログのレベル（隠し項目）。
    pub log_level: LogLevel,
    /// 定型領域（`[[regions]]`）。ファイルの並び順。
    pub regions: Vec<Region>,
}

/// 定型領域のファイル名テンプレートの既定値。
pub const DEFAULT_REGION_FILE_NAME_TEMPLATE: &str = "{name}_{date}-{time}";

/// 定型領域（`[[regions]]` の 1 つ）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// 名前。空でなく、ほかの定型領域と重ならない。
    pub name: String,
    pub hotkey: Option<Chord>,
    /// モニタの識別子（`monitor::MonitorInfo::id`）。
    pub monitor: String,
    /// 範囲。モニタの左上を原点にした物理ピクセル。
    pub rect: Rect,
    /// 保存先。None なら全体の保存先。
    pub save_dir: Option<PathBuf>,
    pub file_name_template: String,
    /// 撮ったら保存する。
    pub save: bool,
    /// 撮ったらスクラップとして表示する。`save` と両方 false にはならない（読み込みで `save` を true に戻す）。
    pub scrap: bool,
}

/// 範囲選択の画面で登録する定型領域。名前は `"{base} N"`（N は、ほかの定型領域と重ならない最も小さい
/// 1 以上の数。消した番号は埋める）。`monitor` はモニタの識別子、`rect` はモニタ相対の範囲。
/// 保存先とファイル名テンプレートは全体設定のものを初期値にし、撮影後は保存だけする。
pub fn new_region(settings: &Settings, base: &str, monitor: &str, rect: Rect) -> Region {
    let used = |name: &str| settings.regions.iter().any(|r| r.name == name);
    let name = (1..)
        .map(|n: u64| format!("{base} {n}"))
        .find(|name| !used(name))
        .expect("some number is unused");
    Region {
        name,
        hotkey: None,
        monitor: monitor.to_owned(),
        rect,
        save_dir: settings.general.save_dir.clone(),
        file_name_template: settings.general.file_name_template.clone(),
        save: true,
        scrap: false,
    }
}

/// 全体設定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct General {
    /// 全体の保存先。None ならピクチャ フォルダの下の `shunsoku`（Win32 層で解決する）。
    pub save_dir: Option<PathBuf>,
    /// 全体のファイル名テンプレート。
    pub file_name_template: String,
    /// 通知。オフで止めるのは成功の通知だけで、エラーと警告は常に出す（spec #32）。
    pub notifications: bool,
    /// ごみ箱の上限（件）。0 なら閉じたらすぐに破棄する。
    pub trash_limit: u8,
    /// DPI の違うモニタへスクラップを動かしたときの見え方。
    pub dpi_scaling: DpiScaling,
    /// キャプチャにマウスカーソルを写す。全方式（範囲選択・ウィンドウ・定型領域・CLI）で共通。
    pub capture_cursor: bool,
}

impl Default for General {
    fn default() -> General {
        General {
            save_dir: None,
            file_name_template: DEFAULT_FILE_NAME_TEMPLATE.to_owned(),
            notifications: true,
            trash_limit: DEFAULT_TRASH_LIMIT,
            dpi_scaling: DpiScaling::default(),
            capture_cursor: false,
        }
    }
}

/// ホットキーで呼び出すもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HotkeyAction {
    /// 範囲選択キャプチャ。必須（空なら未設定の通知を出す）。
    SelectCapture,
    /// 最後に閉じたスクラップをごみ箱から復元する。任意（空でも通知しない）。
    RestoreLast,
    /// アクティブウィンドウのキャプチャ。任意（空でも通知しない）。
    ActiveWindowCapture,
    /// 定型領域キャプチャ。`Settings::regions` の中の位置。`[[regions]]` の `hotkey` に書く。
    Region(usize),
}

impl HotkeyAction {
    /// `[hotkeys]` に書くもののすべて（定型領域は除く）。
    pub const ALL: [HotkeyAction; 3] = [
        HotkeyAction::SelectCapture,
        HotkeyAction::RestoreLast,
        HotkeyAction::ActiveWindowCapture,
    ];

    /// `[hotkeys]` の中のキー（定型領域では `[[regions]]` の中のキー）。
    pub fn key(self) -> &'static str {
        match self {
            HotkeyAction::SelectCapture => "select_capture",
            HotkeyAction::RestoreLast => "restore_last",
            HotkeyAction::ActiveWindowCapture => "active_window_capture",
            HotkeyAction::Region(_) => "hotkey",
        }
    }

    /// 通知に出す項目名（`hotkeys.select_capture`、`regions[N].hotkey`）。N は `Settings::regions` の位置 + 1
    /// （読み込みで捨てた定型領域があれば、ファイルの中の位置とずれる）。
    pub fn item(self) -> String {
        match self {
            HotkeyAction::Region(i) => format!("regions[{}].{}", i + 1, self.key()),
            _ => format!("hotkeys.{}", self.key()),
        }
    }
}

/// ホットキー。既定はすべて空。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hotkeys {
    pub select_capture: Option<Chord>,
    pub restore_last: Option<Chord>,
    pub active_window_capture: Option<Chord>,
}

impl Hotkeys {
    /// `action` のホットキー。空なら None。
    pub fn get(&self, action: HotkeyAction) -> Option<Chord> {
        match action {
            HotkeyAction::SelectCapture => self.select_capture,
            HotkeyAction::RestoreLast => self.restore_last,
            HotkeyAction::ActiveWindowCapture => self.active_window_capture,
            HotkeyAction::Region(_) => None,
        }
    }

    /// 空でないホットキーを割り当てる（ホットキー同士の重なりは、どれにも付けない）。
    pub fn assign(&self) -> Assigned<HotkeyAction> {
        assign(
            HotkeyAction::ALL
                .into_iter()
                .filter_map(|action| Some((self.get(action)?, action))),
        )
    }

    fn get_mut(&mut self, action: HotkeyAction) -> Option<&mut Option<Chord>> {
        match action {
            HotkeyAction::SelectCapture => Some(&mut self.select_capture),
            HotkeyAction::RestoreLast => Some(&mut self.restore_last),
            HotkeyAction::ActiveWindowCapture => Some(&mut self.active_window_capture),
            HotkeyAction::Region(_) => None,
        }
    }
}

/// スクラップ操作のショートカットの上書き。キーはスクラップ操作の id（`scrap::ops` の表）。
/// 値が None なら、その操作にショートカットを割り当てない。書かなかった操作は表の既定値のまま。
/// id が表にあるかどうかはここでは確かめない。表と重ねる `scrap::ops::Shortcuts::new` が、表にない id と
/// 引数のある操作の id を無視して `Shortcuts::ignored_overrides` に載せる。
pub type ShortcutOverrides = BTreeMap<String, Option<Chord>>;

/// スクラップ操作のマウスの割り当ての上書き。キーはスクラップ操作の id（`scrap::ops` の表）。
/// 値が None なら、その操作にマウスの操作を割り当てない。書かなかった操作は表の既定値のまま。
/// id と値が表に合うかどうかはここでは確かめない。表と重ねる `scrap::ops::MouseBindings::new` が確かめる。
pub type MouseOverrides = BTreeMap<String, Option<MouseGesture>>;

/// スクラップの見た目の既定値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrapAppearance {
    /// 透明度（%）。100 で不透明。
    pub opacity: u8,
    pub border: Border,
    /// 影を付ける。
    pub shadow: bool,
}

impl Default for ScrapAppearance {
    fn default() -> ScrapAppearance {
        ScrapAppearance {
            opacity: 100,
            border: Border::default(),
            shadow: true,
        }
    }
}

/// スクラップの枠。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border {
    pub visible: bool,
    /// 太さ（px）。
    pub width: u32,
    pub color: Rgb,
}

impl Default for Border {
    fn default() -> Border {
        Border {
            visible: true,
            width: 1,
            color: Rgb(0x80, 0x80, 0x80),
        }
    }
}

/// 色。設定ファイルでは `#RRGGBB` で書く。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// 範囲選択の画面（`[selection]`）。開くたびに読む。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectionScreen {
    /// 開いたときのモード。`Tab` での切り替えはその回だけで、次に開いたらこれに戻る。
    pub mode: Mode,
    /// 通常モードでも十字線を出す。精密モードでは設定によらず出す。
    pub crosshair: bool,
}

/// DPI の違うモニタへスクラップを動かしたときの見え方（`[general] dpi_scaling`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DpiScaling {
    /// 画素を等倍に保つ。表示の大きさ（物理ピクセル）は変えず、見かけの大きさが変わる。
    #[default]
    KeepPixels,
    /// 見かけの大きさを保つ。移動先の DPI に合わせて表示の大きさを変え、倍率を画素基準で計算し直す。
    KeepApparentSize,
}

/// ログのレベル。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

/// 同じ範囲（ホットキー同士、ショートカット同士）でのキーの割り当て（`assign`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assigned<T> {
    /// 1 つにだけ付いたキーの組み合わせと、その持ち主。
    pub bound: BTreeMap<Chord, T>,
    /// 2 つ以上に付いたため、どれにも付けなかったキーの組み合わせ。
    pub conflicts: Vec<Conflict<T>>,
}

/// 同じ範囲で重なったキーの組み合わせ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict<T> {
    pub chord: Chord,
    /// 重なった持ち主（候補の順）。
    pub owners: Vec<T>,
}

/// キーの組み合わせと持ち主の候補から、同じ範囲での割り当てを決める。同じ組み合わせが 2 つ以上の持ち主に
/// 付いたら、どれにも付けない（どちらが効くか分からない状態を避ける。spec #32）。ホットキー同士と
/// ショートカット同士（`scrap::ops::Shortcuts`）の重なりは、どちらもこの規則で決める。
pub fn assign<T>(candidates: impl IntoIterator<Item = (Chord, T)>) -> Assigned<T> {
    let mut grouped: BTreeMap<Chord, Vec<T>> = BTreeMap::new();
    for (chord, owner) in candidates {
        grouped.entry(chord).or_default().push(owner);
    }
    let mut bound = BTreeMap::new();
    let mut conflicts = Vec::new();
    for (chord, mut owners) in grouped {
        if owners.len() == 1 {
            bound.insert(chord, owners.remove(0));
        } else {
            conflicts.push(Conflict { chord, owners });
        }
    }
    Assigned { bound, conflicts }
}

/// 設定ファイルの問題。通知で知らせる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsIssue {
    /// ファイル全体が TOML として読めない。すべて既定値になる。
    NotToml { message: String },
    /// 不正な項目。その項目だけ既定値になる。`item` は `general.save_dir` のような `.` でつないだ名前。
    /// 表にない操作と、ショートカットを持てない操作への `[shortcuts]` の上書きもこれにする（無視する）。
    Invalid { item: String },
    /// 同じ範囲（ホットキー同士、ショートカット同士）で重なったキーの組み合わせ。重なった項目のどれにも
    /// 付けない。`items` は重なった項目名（`shortcuts.close` など。表の既定値から来たものも含む）。
    Conflict { chord: Chord, items: Vec<String> },
    /// ホットキーとショートカットの重なり。警告だけで、どちらも付けたままにする（OS に登録したホットキーが
    /// 先にキーを取るので、そのショートカットは効かない）。
    Overlap {
        chord: Chord,
        hotkey: String,
        shortcut: String,
    },
    /// 2 つ以上の操作に付いたマウスの操作。どの操作にも付けない。`items` は重なった項目名
    /// （`mouse.close` など。表の既定値から来たものも含む）。
    MouseConflict {
        gesture: MouseGesture,
        items: Vec<String>,
    },
}

/// 設定から作ったキーの割り当て（`keymap`）。
#[derive(Debug, Clone)]
pub struct Keymap {
    /// 登録するホットキー（ホットキー同士の重なりを除いたもの）。
    pub hotkeys: Assigned<HotkeyAction>,
    /// スクラップ操作のショートカットの対応表。
    pub shortcuts: Shortcuts,
    /// スクラップ操作のマウスの対応表。
    pub mouse: MouseBindings,
    /// 重なりと、無視した上書き。`load` の問題と合わせて通知する。
    pub issues: Vec<SettingsIssue>,
}

/// 設定のホットキーと、`ops`（ふつうは `scrap::ops::OPERATIONS`）の既定値に `[shortcuts]` を重ねた
/// ショートカットを割り当て、重なりを見つける。
///
/// - ショートカット同士・ホットキー同士の、同じ範囲での重なりは、両方を無効にして `Conflict` にする。
/// - ホットキーとショートカットの重なりは、警告（`Overlap`）だけにする。
/// - 表にない操作と、ショートカットを持てない操作への上書きは、無視して `Invalid` にする。
/// - マウスの割り当て（`[mouse]`）も同じ規則で表に重ねる。表にない操作・引数のある操作への上書きと、
///   候補にない値（引数のない操作への左ドラッグ）は `Invalid`、重なりは `MouseConflict` にする。
pub fn keymap(settings: &Settings, ops: &'static [OperationDescriptor]) -> Keymap {
    let region_hotkeys = settings
        .regions
        .iter()
        .enumerate()
        .filter_map(|(i, region)| Some((region.hotkey?, HotkeyAction::Region(i))));
    let hotkeys = assign(
        HotkeyAction::ALL
            .into_iter()
            .filter_map(|action| Some((settings.hotkeys.get(action)?, action)))
            .chain(region_hotkeys),
    );
    let shortcuts = Shortcuts::new(ops, &settings.shortcuts);
    let shortcut_item = |id: &str| format!("shortcuts.{id}");
    let hotkey_item = |action: HotkeyAction| action.item();

    let mut issues: Vec<SettingsIssue> = shortcuts
        .ignored_overrides()
        .iter()
        .map(|id| SettingsIssue::Invalid {
            item: shortcut_item(id),
        })
        .collect();
    issues.extend(
        shortcuts
            .conflicts()
            .iter()
            .map(|c| SettingsIssue::Conflict {
                chord: c.chord,
                items: c.owners.iter().map(|id| shortcut_item(id)).collect(),
            }),
    );
    issues.extend(hotkeys.conflicts.iter().map(|c| SettingsIssue::Conflict {
        chord: c.chord,
        items: c.owners.iter().map(|&a| hotkey_item(a)).collect(),
    }));
    for (&chord, &action) in &hotkeys.bound {
        if let Some(op) = shortcuts.operation(chord) {
            issues.push(SettingsIssue::Overlap {
                chord,
                hotkey: hotkey_item(action),
                shortcut: shortcut_item(op.id),
            });
        }
    }
    let mouse = MouseBindings::new(ops, &settings.mouse);
    let mouse_item = |id: &str| format!("mouse.{id}");
    issues.extend(
        mouse
            .ignored_overrides()
            .iter()
            .map(|id| SettingsIssue::Invalid {
                item: mouse_item(id),
            }),
    );
    issues.extend(
        mouse
            .conflicts()
            .iter()
            .map(|c| SettingsIssue::MouseConflict {
                gesture: c.gesture,
                items: c.owners.iter().map(|id| mouse_item(id)).collect(),
            }),
    );
    Keymap {
        hotkeys,
        shortcuts,
        mouse,
        issues,
    }
}

/// 読み込み直しで変わった部分（`diff`）。`app` はこれを見て、変わったものだけを反映し直す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsChanges {
    /// `[hotkeys]` か、定型領域のホットキー（位置ごと）。ホットキーを登録し直す。
    pub hotkeys: bool,
    /// `[shortcuts]`。ショートカットの対応表とメニューを作り直す。
    pub shortcuts: bool,
    /// `[mouse]`。マウスの対応表を作り直す。
    pub mouse: bool,
    /// 全体の保存先かファイル名テンプレート。次の保存から使う。
    pub save: bool,
    /// 通知のオン/オフ。
    pub notifications: bool,
    /// ごみ箱の上限。下げたら超えた分を古いものから捨てる。
    pub trash: bool,
    /// 見た目の既定値（`[scrap]`）。浮かべているスクラップを描き直す。
    pub appearance: bool,
    /// ログのレベル。
    pub log_level: bool,
}

impl SettingsChanges {
    /// 何も変わっていない。
    pub fn is_empty(&self) -> bool {
        *self == SettingsChanges::default()
    }
}

/// `old` から `new` への変化。
pub fn diff(old: &Settings, new: &Settings) -> SettingsChanges {
    SettingsChanges {
        hotkeys: old.hotkeys != new.hotkeys || region_hotkeys(old) != region_hotkeys(new),
        shortcuts: old.shortcuts != new.shortcuts,
        mouse: old.mouse != new.mouse,
        save: old.general.save_dir != new.general.save_dir
            || old.general.file_name_template != new.general.file_name_template,
        notifications: old.general.notifications != new.general.notifications,
        trash: old.general.trash_limit != new.general.trash_limit,
        appearance: old.scrap != new.scrap,
        log_level: old.log_level != new.log_level,
    }
}

/// 定型領域のホットキーを位置の順に並べたもの。`HotkeyAction::Region` は位置で持つので、位置とホットキーの
/// 対応が変わったら登録し直す。
fn region_hotkeys(settings: &Settings) -> Vec<Option<Chord>> {
    settings
        .regions
        .iter()
        .map(|region| region.hotkey)
        .collect()
}

/// 設定ファイルの中身を読む。問題があっても、読めた項目と既定値で `Settings` を返す。
pub fn load(src: &str) -> (Settings, Vec<SettingsIssue>) {
    let doc: DocumentMut = match src.parse() {
        Ok(doc) => doc,
        Err(e) => {
            let issue = SettingsIssue::NotToml {
                message: e.to_string(),
            };
            return (Settings::default(), vec![issue]);
        }
    };
    let mut r = Reader { issues: Vec::new() };
    let root: &dyn TableLike = doc.as_table();
    let mut s = Settings::default();

    // 形式が変わったら移行するための版。違う版でも、読める項目は読む
    if root.get("version").and_then(Item::as_integer) != Some(VERSION) {
        r.invalid("version");
    }
    if let Some(level) = r.value(Some(root), "", "log_level", log_level) {
        s.log_level = level;
    }

    let general = r.table(root, "general");
    if let Some(dir) = r.value(general, "general", "save_dir", save_dir) {
        s.general.save_dir = Some(dir);
    }
    if let Some(t) = r.value(general, "general", "file_name_template", non_empty_str) {
        s.general.file_name_template = t;
    }
    if let Some(on) = r.value(general, "general", "notifications", Item::as_bool) {
        s.general.notifications = on;
    }
    if let Some(limit) = r.value(general, "general", "trash_limit", |i| {
        in_range(i, TRASH_LIMIT_RANGE)
    }) {
        s.general.trash_limit = limit;
    }
    if let Some(scaling) = r.value(general, "general", "dpi_scaling", dpi_scaling) {
        s.general.dpi_scaling = scaling;
    }
    if let Some(on) = r.value(general, "general", "capture_cursor", Item::as_bool) {
        s.general.capture_cursor = on;
    }

    let hotkeys = r.table(root, "hotkeys");
    for action in HotkeyAction::ALL {
        if let Some(chord) = r.value(hotkeys, "hotkeys", action.key(), optional_chord)
            && let Some(slot) = s.hotkeys.get_mut(action)
        {
            *slot = chord;
        }
    }

    if let Some(shortcuts) = r.table(root, "shortcuts") {
        for (op, _) in shortcuts.iter() {
            if let Some(chord) = r.value(Some(shortcuts), "shortcuts", op, optional_chord) {
                s.shortcuts.insert(op.to_owned(), chord);
            }
        }
    }

    if let Some(mouse) = r.table(root, "mouse") {
        for (op, _) in mouse.iter() {
            if let Some(gesture) = r.value(Some(mouse), "mouse", op, optional_gesture) {
                s.mouse.insert(op.to_owned(), gesture);
            }
        }
    }

    let scrap = r.table(root, "scrap");
    if let Some(opacity) = r.value(scrap, "scrap", "opacity", |i| in_range(i, OPACITY_RANGE)) {
        s.scrap.opacity = opacity;
    }
    if let Some(on) = r.value(scrap, "scrap", "border", Item::as_bool) {
        s.scrap.border.visible = on;
    }
    if let Some(w) = r.value(scrap, "scrap", "border_width", |i| {
        in_range(i, BORDER_WIDTH_RANGE)
    }) {
        s.scrap.border.width = w;
    }
    if let Some(color) = r.value(scrap, "scrap", "border_color", rgb) {
        s.scrap.border.color = color;
    }
    if let Some(on) = r.value(scrap, "scrap", "shadow", Item::as_bool) {
        s.scrap.shadow = on;
    }

    let selection = r.table(root, "selection");
    if let Some(mode) = r.value(selection, "selection", "mode", selection_mode) {
        s.selection.mode = mode;
    }
    if let Some(on) = r.value(selection, "selection", "crosshair", Item::as_bool) {
        s.selection.crosshair = on;
    }

    s.regions = regions(root, &mut r);

    (s, r.issues)
}

/// `[[regions]]` を読む。名前・モニタ・範囲のどれかが読めない定型領域と、前の定型領域と名前が重なる定型領域は
/// 捨てる（撮れないため）。ほかの項目は、その項目だけ既定値にする。項目名は `regions[N].key`（N は 1 から）。
fn regions(root: &dyn TableLike, r: &mut Reader) -> Vec<Region> {
    let Some(item) = root.get("regions") else {
        return Vec::new();
    };
    let Some(tables) = item.as_array_of_tables() else {
        r.invalid("regions");
        return Vec::new();
    };
    read_regions(tables, r)
        .into_iter()
        .map(|(_, region)| region)
        .collect()
}

/// `[[regions]]` の要素のうち読み込みで残る定型領域と、その要素の位置（0 から）。
fn read_regions(tables: &toml_edit::ArrayOfTables, r: &mut Reader) -> Vec<(usize, Region)> {
    let mut regions: Vec<(usize, Region)> = Vec::new();
    for (i, table) in tables.iter().enumerate() {
        let path = format!("regions[{}]", i + 1);
        let t: Option<&dyn TableLike> = Some(table);
        let name = r.required(table, &path, "name", non_empty_str);
        let monitor = r.required(table, &path, "monitor", non_empty_str);
        let rect = region_rect(table, &path, r);
        let hotkey = r.value(t, &path, "hotkey", optional_chord).flatten();
        let save_dir = r.value(t, &path, "save_dir", save_dir);
        let file_name_template = r
            .value(t, &path, "file_name_template", non_empty_str)
            .unwrap_or_else(|| DEFAULT_REGION_FILE_NAME_TEMPLATE.to_owned());
        let mut save = r.value(t, &path, "save", Item::as_bool).unwrap_or(true);
        let scrap = r.value(t, &path, "scrap", Item::as_bool).unwrap_or(false);
        if !save && !scrap {
            r.invalid(format!("{path}.save"));
            save = true;
        }
        let (Some(name), Some(monitor), Some(rect)) = (name, monitor, rect) else {
            continue;
        };
        if regions.iter().any(|(_, region)| region.name == name) {
            r.invalid(format!("{path}.name"));
            continue;
        }
        regions.push((
            i,
            Region {
                name,
                hotkey,
                monitor,
                rect,
                save_dir,
                file_name_template,
                save,
                scrap,
            },
        ));
    }
    regions
}

/// 定型領域の範囲（`x` / `y` / `width` / `height`）。どれかが読めなければ None。
fn region_rect(table: &toml_edit::Table, path: &str, r: &mut Reader) -> Option<Rect> {
    let mut read = |key: &str, min: i32| r.required(table, path, key, |i| region_number(i, min));
    let x = read("x", 0);
    let y = read("y", 0);
    let width = read("width", 1);
    let height = read("height", 1);
    let (x, y, width, height) = (x?, y?, width?, height?);
    Some(Rect::new(
        x,
        y,
        x.checked_add(width)?,
        y.checked_add(height)?,
    ))
}

/// 定型領域の範囲の数 1 つ。`min` 以上の i32。
fn region_number(item: &Item, min: i32) -> Option<i32> {
    i32::try_from(item.as_integer()?).ok().filter(|&n| n >= min)
}

/// 実行中に設定ファイルを読み込み直す。`src` は新しい中身（ファイルがなければ None で、既定値にする）。
/// ファイル全体が TOML として読めなければ、`current`（直前の有効な設定）を使い続け、問題だけを返す
/// （書きかけの保存で壊れないように。spec #32）。起動時は `load` を使い、既定値で起動する。
pub fn reload(current: &Settings, src: Option<&str>) -> (Settings, Vec<SettingsIssue>) {
    let Some(src) = src else {
        return (Settings::default(), Vec::new());
    };
    let (settings, issues) = load(src);
    if issues
        .iter()
        .any(|i| matches!(i, SettingsIssue::NotToml { .. }))
    {
        return (current.clone(), issues);
    }
    (settings, issues)
}

/// 項目を読みながら問題を集める。
struct Reader {
    issues: Vec<SettingsIssue>,
}

impl Reader {
    fn invalid(&mut self, item: impl Into<String>) {
        self.issues
            .push(SettingsIssue::Invalid { item: item.into() });
    }

    /// `root` の中の表 `name`。ないときは None。表でなければ問題にして None。
    fn table<'a>(&mut self, root: &'a dyn TableLike, name: &str) -> Option<&'a dyn TableLike> {
        let item = root.get(name)?;
        let table = item.as_table_like();
        if table.is_none() {
            self.invalid(name.to_owned());
        }
        table
    }

    /// 必須の項目を読む。ないときも、読めないときと同じく問題にして None。
    fn required<T>(
        &mut self,
        table: &dyn TableLike,
        path: &str,
        key: &str,
        parse: impl FnOnce(&Item) -> Option<T>,
    ) -> Option<T> {
        if table.get(key).is_none() {
            self.invalid(format!("{path}.{key}"));
            return None;
        }
        self.value(Some(table), path, key, parse)
    }

    /// 表 `table` の `key` を `parse` で読む。ないときは None（既定値のまま）。読めなければ問題にして None。
    fn value<T>(
        &mut self,
        table: Option<&dyn TableLike>,
        path: &str,
        key: &str,
        parse: impl FnOnce(&Item) -> Option<T>,
    ) -> Option<T> {
        let item = table?.get(key)?;
        let value = parse(item);
        if value.is_none() {
            let name = if path.is_empty() {
                key.to_owned()
            } else {
                format!("{path}.{key}")
            };
            self.invalid(name);
        }
        value
    }
}

fn non_empty_str(item: &Item) -> Option<String> {
    item.as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

/// キーの組み合わせ。空の文字列は「割り当てなし」（Some(None)）。
fn optional_chord(item: &Item) -> Option<Option<Chord>> {
    let s = item.as_str()?;
    if s.trim().is_empty() {
        return Some(None);
    }
    s.parse().ok().map(Some)
}

/// マウスの操作の名前（`MouseGesture` の `FromStr`）。空の文字列は「割り当てなし」（Some(None)）。
fn optional_gesture(item: &Item) -> Option<Option<MouseGesture>> {
    let s = item.as_str()?;
    if s.trim().is_empty() {
        return Some(None);
    }
    s.parse().ok().map(Some)
}

/// 保存先。Windows の絶対パス（`C:\…` か `\\server\share\…`）だけを受け付ける。
fn save_dir(item: &Item) -> Option<PathBuf> {
    let s = item.as_str()?;
    let b = s.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && is_sep(b[2]);
    let unc = b.len() >= 3 && is_sep(b[0]) && is_sep(b[1]) && !is_sep(b[2]);
    (drive || unc).then(|| PathBuf::from(s))
}

fn is_sep(b: u8) -> bool {
    b == b'\\' || b == b'/'
}

fn in_range<T: TryFrom<i64> + PartialOrd>(
    item: &Item,
    range: std::ops::RangeInclusive<T>,
) -> Option<T> {
    let n = T::try_from(item.as_integer()?).ok()?;
    range.contains(&n).then_some(n)
}

/// `#RRGGBB`。
fn rgb(item: &Item) -> Option<Rgb> {
    let hex = item.as_str()?.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
}

fn selection_mode(item: &Item) -> Option<Mode> {
    match item.as_str()? {
        "normal" => Some(Mode::Normal),
        "precise" => Some(Mode::Precise),
        _ => None,
    }
}

fn dpi_scaling(item: &Item) -> Option<DpiScaling> {
    match item.as_str()? {
        "keep_pixels" => Some(DpiScaling::KeepPixels),
        "keep_apparent_size" => Some(DpiScaling::KeepApparentSize),
        _ => None,
    }
}

fn log_level(item: &Item) -> Option<LogLevel> {
    let level = match item.as_str()?.to_ascii_lowercase().as_str() {
        "off" => LogLevel::Off,
        "error" => LogLevel::Error,
        "warn" => LogLevel::Warn,
        "info" => LogLevel::Info,
        "debug" => LogLevel::Debug,
        "trace" => LogLevel::Trace,
        _ => return None,
    };
    Some(level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrap::ops::OPERATIONS;

    #[test]
    fn version_only_file_gives_defaults_without_issues() {
        let (settings, issues) = load("version = 1\n");
        assert_eq!(issues, vec![]);
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.general.save_dir, None);
        assert_eq!(
            settings.general.file_name_template,
            "shunsoku_{date}-{time}"
        );
        assert!(settings.general.notifications);
        assert_eq!(settings.hotkeys.select_capture, None);
        assert!(settings.shortcuts.is_empty());
        assert_eq!(settings.scrap.opacity, 100);
        assert!(settings.scrap.border.visible);
        assert_eq!(settings.scrap.border.width, 1);
        assert_eq!(settings.scrap.border.color, Rgb(0x80, 0x80, 0x80));
        assert!(settings.scrap.shadow);
        assert_eq!(settings.log_level, LogLevel::Info);
    }

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    fn invalid(item: &str) -> SettingsIssue {
        SettingsIssue::Invalid {
            item: item.to_owned(),
        }
    }

    #[test]
    fn reads_every_item_of_the_m1_schema() {
        let src = r##"
version = 1
log_level = "debug"

[general]
save_dir = 'D:\shots'
file_name_template = "cap_{datetime:yyyy}"
notifications = false
trash_limit = 3
dpi_scaling = "keep_apparent_size"
capture_cursor = true

[hotkeys]
select_capture = "Win+Shift+PrintScreen"
restore_last = "Ctrl+Alt+Z"

[shortcuts]
copy = "Ctrl+Shift+C"
close = ""

[mouse]
close = "DoubleClick"

[scrap]
opacity = 80
border = false
border_width = 3
border_color = "#1a2B3c"
shadow = false
"##;
        let (settings, issues) = load(src);
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings,
            Settings {
                general: General {
                    save_dir: Some(PathBuf::from(r"D:\shots")),
                    file_name_template: "cap_{datetime:yyyy}".to_owned(),
                    notifications: false,
                    trash_limit: 3,
                    dpi_scaling: DpiScaling::KeepApparentSize,
                    capture_cursor: true,
                },
                hotkeys: Hotkeys {
                    select_capture: Some(chord("Win+Shift+PrintScreen")),
                    restore_last: Some(chord("Ctrl+Alt+Z")),
                    active_window_capture: None,
                },
                shortcuts: BTreeMap::from([
                    ("close".to_owned(), None),
                    ("copy".to_owned(), Some(chord("Ctrl+Shift+C"))),
                ]),
                mouse: BTreeMap::from([("close".to_owned(), Some(MouseGesture::DoubleClick))]),
                scrap: ScrapAppearance {
                    opacity: 80,
                    border: Border {
                        visible: false,
                        width: 3,
                        color: Rgb(0x1A, 0x2B, 0x3C),
                    },
                    shadow: false,
                },
                selection: SelectionScreen::default(),
                log_level: LogLevel::Debug,
                regions: Vec::new(),
            }
        );
    }

    #[test]
    fn hotkey_lookup_by_action() {
        let (settings, _) = load("version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Alt+S\"\n");
        assert_eq!(
            settings.hotkeys.get(HotkeyAction::SelectCapture),
            Some(chord("Ctrl+Alt+S"))
        );
    }

    #[test]
    fn empty_hotkey_means_unset_and_is_not_an_issue() {
        let (settings, issues) = load("version = 1\n[hotkeys]\nselect_capture = \"\"\n");
        assert_eq!(issues, vec![]);
        assert_eq!(settings.hotkeys.select_capture, None);
    }

    #[test]
    fn only_invalid_items_fall_back_to_defaults_and_are_named() {
        let src = r##"
version = 1
log_level = "loud"

[general]
save_dir = "relative\\dir"
file_name_template = ""
notifications = "yes"

[hotkeys]
select_capture = "Ctrl+Nope"

[shortcuts]
copy = "Ctrl+Shift+C"
save = 5
close = "Ctrl+"

[scrap]
opacity = 5
border = true
border_width = 0
border_color = "gray"
shadow = 1
"##;
        let (settings, issues) = load(src);
        assert_eq!(
            issues,
            vec![
                invalid("log_level"),
                invalid("general.save_dir"),
                invalid("general.file_name_template"),
                invalid("general.notifications"),
                invalid("hotkeys.select_capture"),
                invalid("shortcuts.save"),
                invalid("shortcuts.close"),
                invalid("scrap.opacity"),
                invalid("scrap.border_width"),
                invalid("scrap.border_color"),
                invalid("scrap.shadow"),
            ]
        );
        let mut expected = Settings::default();
        expected
            .shortcuts
            .insert("copy".to_owned(), Some(chord("Ctrl+Shift+C")));
        assert_eq!(settings, expected);
    }

    #[test]
    fn a_section_that_is_not_a_table_is_named_and_defaulted() {
        let (settings, issues) = load("version = 1\ngeneral = 3\nscrap = \"x\"\n");
        assert_eq!(issues, vec![invalid("general"), invalid("scrap")]);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn values_out_of_range_are_invalid() {
        let (_, issues) = load("version = 1\n[scrap]\nopacity = 101\nborder_width = 17\n");
        assert_eq!(
            issues,
            vec![invalid("scrap.opacity"), invalid("scrap.border_width")]
        );
        let (settings, issues) = load("version = 1\n[scrap]\nopacity = 10\nborder_width = 16\n");
        assert_eq!(issues, vec![]);
        assert_eq!(
            (settings.scrap.opacity, settings.scrap.border.width),
            (10, 16)
        );
    }

    #[test]
    fn an_opacity_outside_10_to_100_falls_back_to_the_default_and_is_named() {
        for value in ["9", "101", "0", "-10"] {
            let (settings, issues) = load(&format!("version = 1\n[scrap]\nopacity = {value}\n"));
            assert_eq!(issues, vec![invalid("scrap.opacity")], "{value}");
            assert_eq!(settings.scrap.opacity, 100, "{value}");
        }
        for (value, opacity) in [("10", 10), ("55", 55), ("100", 100)] {
            let (settings, issues) = load(&format!("version = 1\n[scrap]\nopacity = {value}\n"));
            assert_eq!(issues, vec![], "{value}");
            assert_eq!(settings.scrap.opacity, opacity, "{value}");
        }
    }

    #[test]
    fn the_trash_limit_defaults_to_10_and_out_of_range_values_fall_back_to_it() {
        assert_eq!(load("version = 1\n").0.general.trash_limit, 10);
        for (value, limit) in [("0", 0), ("2", 2), ("100", 100)] {
            let (settings, issues) =
                load(&format!("version = 1\n[general]\ntrash_limit = {value}\n"));
            assert_eq!(issues, vec![], "{value}");
            assert_eq!(settings.general.trash_limit, limit, "{value}");
        }
        for value in ["101", "-1", "\"5\"", "2.5"] {
            let (settings, issues) =
                load(&format!("version = 1\n[general]\ntrash_limit = {value}\n"));
            assert_eq!(issues, vec![invalid("general.trash_limit")], "{value}");
            assert_eq!(settings.general.trash_limit, 10, "{value}");
        }
    }

    #[test]
    fn capturing_the_cursor_defaults_to_off_and_a_non_bool_falls_back_to_it() {
        assert!(!load("version = 1\n").0.general.capture_cursor);
        let (settings, issues) = load("version = 1\n[general]\ncapture_cursor = true\n");
        assert_eq!(issues, vec![]);
        assert!(settings.general.capture_cursor);
        let (settings, issues) = load("version = 1\n[general]\ncapture_cursor = \"yes\"\n");
        assert_eq!(issues, vec![invalid("general.capture_cursor")]);
        assert!(!settings.general.capture_cursor);
    }

    #[test]
    fn the_dpi_scaling_defaults_to_keeping_pixels_and_unknown_values_fall_back_to_it() {
        assert_eq!(
            load("version = 1\n").0.general.dpi_scaling,
            DpiScaling::KeepPixels
        );
        for (value, scaling) in [
            ("keep_pixels", DpiScaling::KeepPixels),
            ("keep_apparent_size", DpiScaling::KeepApparentSize),
        ] {
            let (settings, issues) = load(&format!(
                "version = 1\n[general]\ndpi_scaling = \"{value}\"\n"
            ));
            assert_eq!(issues, vec![], "{value}");
            assert_eq!(settings.general.dpi_scaling, scaling, "{value}");
        }
        for value in ["\"keep_size\"", "\"\"", "true", "1"] {
            let (settings, issues) =
                load(&format!("version = 1\n[general]\ndpi_scaling = {value}\n"));
            assert_eq!(issues, vec![invalid("general.dpi_scaling")], "{value}");
            assert_eq!(
                settings.general.dpi_scaling,
                DpiScaling::KeepPixels,
                "{value}"
            );
        }
    }

    #[test]
    fn the_selection_screen_defaults_to_normal_mode_without_crosshair() {
        let (settings, _) = load("version = 1\n");
        assert_eq!(settings.selection.mode, Mode::Normal);
        assert!(!settings.selection.crosshair);
        let (settings, issues) =
            load("version = 1\n[selection]\nmode = \"precise\"\ncrosshair = true\n");
        assert_eq!(issues, vec![]);
        assert_eq!(settings.selection.mode, Mode::Precise);
        assert!(settings.selection.crosshair);
        let (settings, issues) = load("version = 1\n[selection]\nmode = \"normal\"\n");
        assert_eq!(issues, vec![]);
        assert_eq!(settings.selection.mode, Mode::Normal);
    }

    #[test]
    fn invalid_selection_screen_items_fall_back_to_the_defaults_and_are_named() {
        for value in ["\"Precise\"", "\"fine\"", "\"\"", "true", "1"] {
            let (settings, issues) = load(&format!("version = 1\n[selection]\nmode = {value}\n"));
            assert_eq!(issues, vec![invalid("selection.mode")], "{value}");
            assert_eq!(settings.selection.mode, Mode::Normal, "{value}");
        }
        for value in ["\"true\"", "1"] {
            let (settings, issues) =
                load(&format!("version = 1\n[selection]\ncrosshair = {value}\n"));
            assert_eq!(issues, vec![invalid("selection.crosshair")], "{value}");
            assert!(!settings.selection.crosshair, "{value}");
        }
        let (_, issues) = load("version = 1\nselection = 1\n");
        assert_eq!(issues, vec![invalid("selection")]);
    }

    #[test]
    fn unc_save_dir_is_accepted() {
        let (settings, issues) = load("version = 1\n[general]\nsave_dir = '\\\\nas\\shots'\n");
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings.general.save_dir,
            Some(PathBuf::from(r"\\nas\shots"))
        );
    }

    #[test]
    fn unknown_keys_and_tables_are_ignored() {
        let src = r##"
version = 1
future_option = true

[general]
notifications = false
language = "en"

[future_table]
foo = 1

[[unknown_list]]
a = 1
"##;
        let (settings, issues) = load(src);
        assert_eq!(issues, vec![]);
        assert!(!settings.general.notifications);
    }

    #[test]
    fn missing_or_other_version_is_an_issue_but_items_are_still_read() {
        for src in [
            "[general]\nnotifications = false\n",
            "version = 2\n[general]\nnotifications = false\n",
            "version = \"1\"\n[general]\nnotifications = false\n",
        ] {
            let (settings, issues) = load(src);
            assert_eq!(issues, vec![invalid("version")], "{src:?}");
            assert!(!settings.general.notifications, "{src:?}");
        }
    }

    #[test]
    fn diff_of_equal_settings_has_no_changes() {
        let (settings, _) = load(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Alt+S\"\n[scrap]\nopacity = 50\n",
        );
        assert_eq!(
            diff(&settings, &settings.clone()),
            SettingsChanges::default()
        );
        assert!(diff(&settings, &settings.clone()).is_empty());
    }

    #[test]
    fn diff_tells_which_parts_changed() {
        let old = Settings::default();
        let changed = |f: fn(&mut Settings)| {
            let mut new = old.clone();
            f(&mut new);
            diff(&old, &new)
        };
        assert_eq!(
            changed(|s| s.hotkeys.select_capture = Some(chord("Ctrl+Alt+S"))),
            SettingsChanges {
                hotkeys: true,
                ..SettingsChanges::default()
            }
        );
        assert_eq!(
            changed(|s| {
                s.shortcuts.insert("close".to_owned(), None);
            }),
            SettingsChanges {
                shortcuts: true,
                ..SettingsChanges::default()
            }
        );
        assert_eq!(
            changed(|s| {
                s.mouse.insert("close".to_owned(), None);
            }),
            SettingsChanges {
                mouse: true,
                ..SettingsChanges::default()
            }
        );
        assert_eq!(
            changed(|s| s.general.save_dir = Some(PathBuf::from(r"D:\shots"))),
            SettingsChanges {
                save: true,
                ..SettingsChanges::default()
            }
        );
        assert_eq!(
            changed(|s| s.general.file_name_template = "x_{date}".to_owned()),
            SettingsChanges {
                save: true,
                ..SettingsChanges::default()
            }
        );
        assert_eq!(
            changed(|s| s.general.notifications = false),
            SettingsChanges {
                notifications: true,
                ..SettingsChanges::default()
            }
        );
        for f in [
            (|s: &mut Settings| s.scrap.opacity = 50) as fn(&mut Settings),
            |s| s.scrap.border.visible = false,
            |s| s.scrap.border.width = 4,
            |s| s.scrap.border.color = Rgb(1, 2, 3),
            |s| s.scrap.shadow = false,
        ] {
            assert_eq!(
                changed(f),
                SettingsChanges {
                    appearance: true,
                    ..SettingsChanges::default()
                }
            );
        }
        assert_eq!(
            changed(|s| s.general.trash_limit = 2),
            SettingsChanges {
                trash: true,
                ..SettingsChanges::default()
            }
        );
        assert_eq!(
            changed(|s| s.log_level = LogLevel::Trace),
            SettingsChanges {
                log_level: true,
                ..SettingsChanges::default()
            }
        );
        let mut new = old.clone();
        new.hotkeys.select_capture = Some(chord("Ctrl+Alt+S"));
        new.scrap.shadow = false;
        let both = diff(&old, &new);
        assert!(both.hotkeys && both.appearance && !both.shortcuts && !both.is_empty());
    }

    #[test]
    fn keys_overlapping_in_the_same_scope_are_disabled_for_all_their_owners() {
        let assigned = assign([
            (chord("Ctrl+A"), "a"),
            (chord("Ctrl+B"), "b"),
            (chord("Ctrl+A"), "c"),
            (chord("Esc"), "d"),
            (chord("Ctrl+A"), "e"),
        ]);
        assert_eq!(
            assigned.bound,
            BTreeMap::from([(chord("Ctrl+B"), "b"), (chord("Esc"), "d")])
        );
        assert_eq!(
            assigned.conflicts,
            vec![Conflict {
                chord: chord("Ctrl+A"),
                owners: vec!["a", "c", "e"],
            }]
        );
    }

    #[test]
    fn hotkeys_are_assigned_without_the_unset_ones() {
        assert!(Hotkeys::default().assign().bound.is_empty());
        let hotkeys = Hotkeys {
            select_capture: Some(chord("Ctrl+Alt+S")),
            restore_last: None,
            active_window_capture: None,
        };
        let assigned = hotkeys.assign();
        assert_eq!(
            assigned.bound,
            BTreeMap::from([(chord("Ctrl+Alt+S"), HotkeyAction::SelectCapture)])
        );
        assert!(assigned.conflicts.is_empty());
    }

    #[test]
    fn restore_last_hotkey_is_unset_by_default() {
        let (settings, issues) = load("version = 1\n");
        assert_eq!(issues, vec![]);
        assert_eq!(settings.hotkeys.get(HotkeyAction::RestoreLast), None);
    }

    #[test]
    fn restore_last_hotkey_is_read_and_assigned() {
        let (settings, issues) = load("version = 1\n[hotkeys]\nrestore_last = \"Ctrl+Alt+Z\"\n");
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings.hotkeys.get(HotkeyAction::RestoreLast),
            Some(chord("Ctrl+Alt+Z"))
        );
        assert_eq!(
            settings.hotkeys.assign().bound,
            BTreeMap::from([(chord("Ctrl+Alt+Z"), HotkeyAction::RestoreLast)])
        );
    }

    #[test]
    fn active_window_capture_hotkey_is_unset_by_default() {
        let (settings, issues) = load("version = 1\n");
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings.hotkeys.get(HotkeyAction::ActiveWindowCapture),
            None
        );
    }

    #[test]
    fn active_window_capture_hotkey_is_read_and_assigned() {
        let (settings, issues) =
            load("version = 1\n[hotkeys]\nactive_window_capture = \"Ctrl+Alt+W\"\n");
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings.hotkeys.get(HotkeyAction::ActiveWindowCapture),
            Some(chord("Ctrl+Alt+W"))
        );
        assert_eq!(
            settings.hotkeys.assign().bound,
            BTreeMap::from([(chord("Ctrl+Alt+W"), HotkeyAction::ActiveWindowCapture)])
        );
    }

    #[test]
    fn restore_last_hotkey_overlapping_another_hotkey_disables_both() {
        let keymap = keymap_of(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Alt+S\"\nrestore_last = \"Ctrl+Alt+S\"\n",
        );
        assert!(keymap.hotkeys.bound.is_empty());
        assert_eq!(
            keymap.issues,
            vec![SettingsIssue::Conflict {
                chord: chord("Ctrl+Alt+S"),
                items: vec![
                    "hotkeys.select_capture".to_owned(),
                    "hotkeys.restore_last".to_owned()
                ],
            }]
        );
    }

    #[test]
    fn regions_are_read_in_file_order_with_their_defaults() {
        let src = r#"
version = 1

[[regions]]
name = "dashboard"
hotkey = "Ctrl+Alt+1"
monitor = '\\?\DISPLAY#GSM5B08#1'
x = 10
y = 20
width = 640
height = 480
save_dir = 'D:\dash'
file_name_template = "dash_{seq:3}"
save = false
scrap = true

[[regions]]
name = "game"
monitor = '\\?\DISPLAY#DEL4321#2'
x = 0
y = 0
width = 1
height = 1
"#;
        let (settings, issues) = load(src);
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings.regions,
            vec![
                Region {
                    name: "dashboard".to_owned(),
                    hotkey: Some(chord("Ctrl+Alt+1")),
                    monitor: r"\\?\DISPLAY#GSM5B08#1".to_owned(),
                    rect: Rect::new(10, 20, 650, 500),
                    save_dir: Some(PathBuf::from(r"D:\dash")),
                    file_name_template: "dash_{seq:3}".to_owned(),
                    save: false,
                    scrap: true,
                },
                Region {
                    name: "game".to_owned(),
                    hotkey: None,
                    monitor: r"\\?\DISPLAY#DEL4321#2".to_owned(),
                    rect: Rect::new(0, 0, 1, 1),
                    save_dir: None,
                    file_name_template: "{name}_{date}-{time}".to_owned(),
                    save: true,
                    scrap: false,
                },
            ]
        );
    }

    /// `[[regions]]` の 1 つ。`extra` を足す。
    fn region_src(name: &str, extra: &str) -> String {
        format!(
            "[[regions]]\nname = \"{name}\"\nmonitor = 'M1'\nx = 0\ny = 0\nwidth = 10\nheight = 10\n{extra}\n"
        )
    }

    fn names(settings: &Settings) -> Vec<&str> {
        settings.regions.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn a_region_whose_name_is_already_used_is_dropped_and_named() {
        let src = format!(
            "version = 1\n{}{}{}",
            region_src("a", ""),
            region_src("b", ""),
            region_src("a", "save = false\nscrap = true")
        );
        let (settings, issues) = load(&src);
        assert_eq!(issues, vec![invalid("regions[3].name")]);
        assert_eq!(names(&settings), vec!["a", "b"]);
        assert!(settings.regions[0].save);
    }

    #[test]
    fn a_region_that_neither_saves_nor_shows_a_scrap_falls_back_to_saving() {
        let src = format!("version = 1\n{}", region_src("a", "save = false"));
        let (settings, issues) = load(&src);
        assert_eq!(issues, vec![invalid("regions[1].save")]);
        assert!(settings.regions[0].save);
        assert!(!settings.regions[0].scrap);
    }

    #[test]
    fn a_region_without_a_name_monitor_or_range_is_dropped_and_named() {
        let src = r#"
version = 1

[[regions]]
monitor = 'M1'
x = 0
y = 0
width = 10
height = 10

[[regions]]
name = ""
monitor = 'M1'
x = 0
y = 0
width = 10
height = 10

[[regions]]
name = "no monitor"
x = 0
y = 0
width = 10
height = 10

[[regions]]
name = "empty range"
monitor = 'M1'
x = -1
y = 0
width = 0
"#;
        let (settings, issues) = load(src);
        assert_eq!(
            issues,
            vec![
                invalid("regions[1].name"),
                invalid("regions[2].name"),
                invalid("regions[3].monitor"),
                invalid("regions[4].x"),
                invalid("regions[4].width"),
                invalid("regions[4].height"),
            ]
        );
        assert!(settings.regions.is_empty());
    }

    #[test]
    fn invalid_optional_region_items_fall_back_to_their_defaults() {
        let src = format!(
            "version = 1\n{}",
            region_src(
                "a",
                "hotkey = \"Ctrl+Nope\"\nsave_dir = 'relative'\nfile_name_template = \"\"\nscrap = 1"
            )
        );
        let (settings, issues) = load(&src);
        assert_eq!(
            issues,
            vec![
                invalid("regions[1].hotkey"),
                invalid("regions[1].save_dir"),
                invalid("regions[1].file_name_template"),
                invalid("regions[1].scrap"),
            ]
        );
        let region = &settings.regions[0];
        assert_eq!(region.hotkey, None);
        assert_eq!(region.save_dir, None);
        assert_eq!(region.file_name_template, "{name}_{date}-{time}");
        assert!(region.save && !region.scrap);
    }

    #[test]
    fn regions_that_are_not_an_array_of_tables_are_named() {
        let (settings, issues) = load("version = 1\nregions = 3\n");
        assert_eq!(issues, vec![invalid("regions")]);
        assert!(settings.regions.is_empty());
    }

    #[test]
    fn region_hotkeys_are_assigned_with_the_other_hotkeys_and_overlaps_disable_both() {
        let src = format!(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Alt+S\"\n{}{}{}",
            region_src("a", "hotkey = \"Ctrl+Alt+1\""),
            region_src("b", ""),
            region_src("c", "hotkey = \"Ctrl+Alt+S\"")
        );
        let keymap = keymap_of(&src);
        assert_eq!(
            keymap.hotkeys.bound,
            BTreeMap::from([(chord("Ctrl+Alt+1"), HotkeyAction::Region(0))])
        );
        assert_eq!(
            keymap.issues,
            vec![SettingsIssue::Conflict {
                chord: chord("Ctrl+Alt+S"),
                items: vec![
                    "hotkeys.select_capture".to_owned(),
                    "regions[3].hotkey".to_owned()
                ],
            }]
        );
    }

    #[test]
    fn diff_tells_that_hotkeys_changed_when_a_region_hotkey_changed() {
        let (old, _) = load(&format!("version = 1\n{}", region_src("a", "")));
        let mut new = old.clone();
        new.regions[0].hotkey = Some(chord("Ctrl+Alt+1"));
        assert!(diff(&old, &new).hotkeys);
        let mut renamed = old.clone();
        renamed.regions[0].name = "b".to_owned();
        assert!(!diff(&old, &renamed).hotkeys);
    }

    fn keymap_of(src: &str) -> Keymap {
        let (settings, issues) = load(src);
        assert_eq!(issues, vec![], "{src}");
        keymap(&settings, OPERATIONS)
    }

    fn bound_op(keymap: &Keymap, s: &str) -> Option<&'static str> {
        keymap.shortcuts.operation(chord(s)).map(|op| op.id)
    }

    #[test]
    fn keymap_of_the_defaults_has_no_issues() {
        let keymap = keymap_of("version = 1\n");
        assert!(keymap.hotkeys.bound.is_empty());
        assert_eq!(bound_op(&keymap, "Esc"), Some("close"));
        assert_eq!(keymap.issues, vec![]);
    }

    #[test]
    fn overlapping_shortcuts_are_disabled_and_named_as_an_issue() {
        let keymap = keymap_of("version = 1\n[shortcuts]\ncopy = \"Ctrl+W\"\nclose = \"Ctrl+W\"\n");
        assert_eq!(bound_op(&keymap, "Ctrl+W"), None);
        assert_eq!(bound_op(&keymap, "Ctrl+C"), None);
        assert_eq!(bound_op(&keymap, "Esc"), None);
        assert_eq!(bound_op(&keymap, "Ctrl+S"), Some("save_default"));
        assert_eq!(
            keymap.issues,
            vec![SettingsIssue::Conflict {
                chord: chord("Ctrl+W"),
                items: vec!["shortcuts.copy".to_owned(), "shortcuts.close".to_owned()],
            }]
        );
    }

    #[test]
    fn a_shortcut_overriding_onto_a_default_one_disables_both() {
        let keymap = keymap_of("version = 1\n[shortcuts]\nclose = \"Ctrl+S\"\n");
        assert_eq!(bound_op(&keymap, "Ctrl+S"), None);
        assert_eq!(
            keymap.issues,
            vec![SettingsIssue::Conflict {
                chord: chord("Ctrl+S"),
                items: vec![
                    "shortcuts.save_default".to_owned(),
                    "shortcuts.close".to_owned()
                ],
            }]
        );
    }

    #[test]
    fn a_hotkey_on_a_shortcut_is_only_a_warning_and_keeps_both() {
        let keymap = keymap_of("version = 1\n[hotkeys]\nselect_capture = \"Ctrl+C\"\n");
        assert_eq!(
            keymap.hotkeys.bound,
            BTreeMap::from([(chord("Ctrl+C"), HotkeyAction::SelectCapture)])
        );
        assert_eq!(bound_op(&keymap, "Ctrl+C"), Some("copy"));
        assert_eq!(
            keymap.issues,
            vec![SettingsIssue::Overlap {
                chord: chord("Ctrl+C"),
                hotkey: "hotkeys.select_capture".to_owned(),
                shortcut: "shortcuts.copy".to_owned(),
            }]
        );
        // 重なりで無効になったショートカットとは、重ならない
        let keymap = keymap_of(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+W\"\n[shortcuts]\ncopy = \"Ctrl+W\"\nclose = \"Ctrl+W\"\n",
        );
        assert!(
            keymap
                .issues
                .iter()
                .all(|i| !matches!(i, SettingsIssue::Overlap { .. })),
            "{:?}",
            keymap.issues
        );
    }

    #[test]
    fn mouse_overrides_are_read_by_gesture_name_and_empty_removes() {
        let src = "version = 1\n[mouse]\nclose = \"DoubleClick\"\ntoggle_compact = \"MiddleClick\"\nzoom_in = \"Ctrl+WheelUp\"\nopacity_up = \"wheelup\"\nzoom_out = \"\"\n";
        let (settings, issues) = load(src);
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings.mouse,
            MouseOverrides::from([
                ("close".to_owned(), Some(MouseGesture::DoubleClick)),
                ("opacity_up".to_owned(), Some(MouseGesture::WheelUp)),
                ("toggle_compact".to_owned(), Some(MouseGesture::MiddleClick)),
                ("zoom_in".to_owned(), Some(MouseGesture::CtrlWheelUp)),
                ("zoom_out".to_owned(), None),
            ])
        );
    }

    #[test]
    fn mouse_overrides_that_are_not_gesture_names_are_invalid_items() {
        let src =
            "version = 1\n[mouse]\nclose = \"RightClick\"\nzoom_in = 1\ncopy = \"DoubleClick\"\n";
        let (settings, issues) = load(src);
        assert_eq!(
            issues,
            vec![invalid("mouse.close"), invalid("mouse.zoom_in")]
        );
        assert_eq!(
            settings.mouse,
            MouseOverrides::from([("copy".to_owned(), Some(MouseGesture::DoubleClick))])
        );
        let (_, issues) = load("version = 1\nmouse = 3\n");
        assert_eq!(issues, vec![invalid("mouse")]);
    }

    #[test]
    fn keymap_names_ignored_and_overlapping_mouse_overrides() {
        let keymap = keymap_of(
            "version = 1\n[mouse]\nfly = \"WheelUp\"\nclose = \"LeftDrag\"\ncopy = \"DoubleClick\"\n",
        );
        assert_eq!(
            keymap.issues,
            vec![
                invalid("mouse.close"),
                invalid("mouse.fly"),
                SettingsIssue::MouseConflict {
                    gesture: MouseGesture::DoubleClick,
                    items: vec!["mouse.copy".to_owned(), "mouse.toggle_compact".to_owned()],
                },
            ]
        );
        assert_eq!(
            keymap
                .mouse
                .operation(MouseGesture::MiddleClick)
                .map(|op| op.id),
            Some("close")
        );
        assert!(keymap.mouse.operation(MouseGesture::DoubleClick).is_none());
    }

    #[test]
    fn overrides_that_cannot_take_a_shortcut_are_invalid_items() {
        let keymap =
            keymap_of("version = 1\n[shortcuts]\nfly = \"F\"\nmove = \"M\"\nclose = \"Q\"\n");
        assert_eq!(bound_op(&keymap, "Q"), Some("close"));
        assert_eq!(
            keymap.issues,
            vec![invalid("shortcuts.fly"), invalid("shortcuts.move")]
        );
    }

    #[test]
    fn reloading_text_that_is_not_toml_keeps_the_current_settings() {
        let (current, _) = load("version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Alt+S\"\n");
        let (settings, issues) = reload(&current, Some("version = 1\n[hotkeys\n"));
        assert_eq!(settings, current);
        assert!(
            matches!(issues.as_slice(), [SettingsIssue::NotToml { .. }]),
            "{issues:?}"
        );
    }

    #[test]
    fn reloading_reads_the_new_file_and_its_invalid_items_fall_back_to_defaults() {
        let (current, _) = load("version = 1\n[scrap]\nopacity = 50\nshadow = false\n");
        let (settings, issues) = reload(
            &current,
            Some("version = 1\n[scrap]\nopacity = 5\n[general]\nnotifications = false\n"),
        );
        assert_eq!(issues, vec![invalid("scrap.opacity")]);
        let mut expected = Settings::default();
        expected.general.notifications = false;
        assert_eq!(settings, expected);
    }

    #[test]
    fn reloading_without_a_file_gives_the_defaults() {
        let (current, _) = load("version = 1\n[scrap]\nopacity = 50\n");
        assert_eq!(reload(&current, None), (Settings::default(), vec![]));
    }

    #[test]
    fn text_that_is_not_toml_gives_defaults_and_one_issue() {
        let (settings, issues) = load("version = 1\n[hotkeys\nselect_capture = \"Ctrl+A\"\n");
        assert_eq!(settings, Settings::default());
        assert!(
            matches!(issues.as_slice(), [SettingsIssue::NotToml { message }] if !message.is_empty()),
            "{issues:?}"
        );
    }

    /// docs/settings.sample.toml（README から案内するサンプル）。
    const SAMPLE: &str = include_str!("../../../docs/settings.sample.toml");

    #[test]
    fn the_sample_file_reads_without_issues_and_holds_the_defaults() {
        let (settings, issues) = load(SAMPLE);
        assert_eq!(issues, vec![]);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn the_examples_commented_out_in_the_sample_file_are_valid() {
        // `# key = value` の行の # を外す（説明のコメントは `key = value` の形でないので残る）
        let uncommented: String = SAMPLE
            .lines()
            .map(|line| match line.strip_prefix("# ") {
                Some(rest @ "[[regions]]") => rest,
                Some(rest)
                    if rest.split_once(" = ").is_some_and(|(key, _)| {
                        key.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    }) =>
                {
                    rest
                }
                _ => line,
            })
            .map(|line| format!("{line}\n"))
            .collect();
        let (settings, issues) = load(&uncommented);
        assert_eq!(issues, vec![]);
        assert_eq!(keymap(&settings, OPERATIONS).issues, vec![]);
        assert_eq!(settings.general.save_dir, Some(PathBuf::from(r"D:\shots")));
        assert_eq!(
            settings.hotkeys.select_capture,
            Some("Ctrl+Shift+A".parse().unwrap())
        );
        assert_eq!(
            settings.shortcuts,
            ShortcutOverrides::from([
                ("copy".to_owned(), Some("Ctrl+Shift+C".parse().unwrap())),
                ("close".to_owned(), None),
            ])
        );
        assert_eq!(names(&settings), vec!["ダッシュボード"]);
        assert_eq!(settings.regions[0].rect, Rect::new(0, 0, 800, 600));
    }

    fn named(names: &[&str]) -> Settings {
        let region = |name: &str| Region {
            name: name.to_owned(),
            hotkey: None,
            monitor: "m".to_owned(),
            rect: Rect::new(0, 0, 10, 10),
            save_dir: None,
            file_name_template: DEFAULT_REGION_FILE_NAME_TEMPLATE.to_owned(),
            save: true,
            scrap: false,
        };
        Settings {
            regions: names.iter().map(|n| region(n)).collect(),
            ..Settings::default()
        }
    }

    #[test]
    fn registered_regions_are_named_with_the_smallest_unused_number() {
        let name =
            |names: &[&str]| new_region(&named(names), "領域", "m", Rect::new(0, 0, 1, 1)).name;
        assert_eq!(name(&[]), "領域 1");
        assert_eq!(name(&["領域 1", "領域 2"]), "領域 3");
        // 消した番号（欠番）は埋める
        assert_eq!(name(&["領域 1", "領域 3"]), "領域 2");
        assert_eq!(name(&["領域 2", "領域 3"]), "領域 1");
        // 手で付けた名前は数えない。同じ名前だけを避ける
        assert_eq!(name(&["ダッシュボード", "領域 01", "領域1"]), "領域 1");
    }

    #[test]
    fn a_registered_region_starts_from_the_general_save_dir_and_template_and_only_saves() {
        let mut settings = named(&[]);
        settings.general.save_dir = Some(PathBuf::from(r"D:\shots"));
        settings.general.file_name_template = "cap_{seq:3}".to_owned();
        let region = new_region(
            &settings,
            "領域",
            r"\\?\DISPLAY#AUS",
            Rect::new(10, 20, 110, 70),
        );
        assert_eq!(
            region,
            Region {
                name: "領域 1".to_owned(),
                hotkey: None,
                monitor: r"\\?\DISPLAY#AUS".to_owned(),
                rect: Rect::new(10, 20, 110, 70),
                save_dir: Some(PathBuf::from(r"D:\shots")),
                file_name_template: "cap_{seq:3}".to_owned(),
                save: true,
                scrap: false,
            }
        );
    }
}
