//! 設定ファイル（`%APPDATA%\shunsoku\settings.toml`）の読み込み。
//!
//! - `load` は項目ごとに値を取り出す。不正な項目はその項目だけ既定値にし、項目名を `SettingsIssue` に載せる。
//!   知らないキーは無視する。ファイル全体が TOML として読めなければ、すべて既定値にする。
//! - ファイルは書き換えない（`write` は M3）。
//! - 読み込み直しでは `diff` で変わった部分を調べ、`app` が変わったものだけを反映し直す。
//! - キーの重なりは `keymap` で見つける。同じ範囲（ホットキー同士、ショートカット同士）の重なりは `assign` の
//!   規則で両方を無効にし、ホットキーとショートカットの重なりは警告だけにする。
//!
//! M1 のスキーマ（書かなかった項目は既定値）:
//!
//! ```toml
//! version = 1                                  # 必須。1 だけ
//! log_level = "info"                           # 隠し項目: off / error / warn / info / debug / trace
//!
//! [general]
//! save_dir = 'D:\shots'                        # 既定: ピクチャ フォルダの下の shunsoku
//! file_name_template = "shunsoku_{date}-{time}"
//! notifications = true                         # false で止まるのは成功の通知だけ
//!
//! [hotkeys]
//! select_capture = "Ctrl+Shift+A"              # 既定は空（未設定）。"" も未設定
//!
//! [shortcuts]                                  # スクラップ操作の id ごとの上書き。"" は割り当てなし
//! copy = "Ctrl+Shift+C"
//!
//! [scrap]                                      # 見た目の既定値
//! opacity = 100                                # 10〜100（%）
//! border = true
//! border_width = 1                             # 1〜16（px）
//! border_color = "#808080"
//! shadow = true
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use toml_edit::{DocumentMut, Item, TableLike};

use crate::keys::Chord;
use crate::scrap::ops::{OperationDescriptor, Shortcuts};

/// 対応している設定ファイルの形式の版。
pub const VERSION: i64 = 1;

/// 全体のファイル名テンプレートの既定値。
pub const DEFAULT_FILE_NAME_TEMPLATE: &str = "shunsoku_{date}-{time}";

/// 透明度（%）として書ける範囲。
pub const OPACITY_RANGE: std::ops::RangeInclusive<u8> = 10..=100;

/// 枠の太さ（px）として書ける範囲。
pub const BORDER_WIDTH_RANGE: std::ops::RangeInclusive<u32> = 1..=16;

/// 設定。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub general: General,
    pub hotkeys: Hotkeys,
    pub shortcuts: ShortcutOverrides,
    pub scrap: ScrapAppearance,
    /// ログのレベル（隠し項目）。
    pub log_level: LogLevel,
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
}

impl Default for General {
    fn default() -> General {
        General {
            save_dir: None,
            file_name_template: DEFAULT_FILE_NAME_TEMPLATE.to_owned(),
            notifications: true,
        }
    }
}

/// ホットキーで呼び出すもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HotkeyAction {
    /// 範囲選択キャプチャ。必須（空なら未設定の通知を出す）。
    SelectCapture,
}

impl HotkeyAction {
    /// すべて。
    pub const ALL: [HotkeyAction; 1] = [HotkeyAction::SelectCapture];

    /// `[hotkeys]` の中のキー。
    pub fn key(self) -> &'static str {
        match self {
            HotkeyAction::SelectCapture => "select_capture",
        }
    }
}

/// ホットキー。既定はすべて空。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hotkeys {
    pub select_capture: Option<Chord>,
}

impl Hotkeys {
    /// `action` のホットキー。空なら None。
    pub fn get(&self, action: HotkeyAction) -> Option<Chord> {
        match action {
            HotkeyAction::SelectCapture => self.select_capture,
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

    fn get_mut(&mut self, action: HotkeyAction) -> &mut Option<Chord> {
        match action {
            HotkeyAction::SelectCapture => &mut self.select_capture,
        }
    }
}

/// スクラップ操作のショートカットの上書き。キーはスクラップ操作の id（`scrap::ops` の表）。
/// 値が None なら、その操作にショートカットを割り当てない。書かなかった操作は表の既定値のまま。
/// id が表にあるかどうかはここでは確かめない。表と重ねる `scrap::ops::Shortcuts::new` が、表にない id と
/// 引数のある操作の id を無視して `Shortcuts::ignored_overrides` に載せる。
pub type ShortcutOverrides = BTreeMap<String, Option<Chord>>;

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
}

/// 設定から作ったキーの割り当て（`keymap`）。
#[derive(Debug, Clone)]
pub struct Keymap {
    /// 登録するホットキー（ホットキー同士の重なりを除いたもの）。
    pub hotkeys: Assigned<HotkeyAction>,
    /// スクラップ操作のショートカットの対応表。
    pub shortcuts: Shortcuts,
    /// 重なりと、無視した上書き。`load` の問題と合わせて通知する。
    pub issues: Vec<SettingsIssue>,
}

/// 設定のホットキーと、`ops`（ふつうは `scrap::ops::OPERATIONS`）の既定値に `[shortcuts]` を重ねた
/// ショートカットを割り当て、重なりを見つける。
///
/// - ショートカット同士・ホットキー同士の、同じ範囲での重なりは、両方を無効にして `Conflict` にする。
/// - ホットキーとショートカットの重なりは、警告（`Overlap`）だけにする。
/// - 表にない操作と、ショートカットを持てない操作への上書きは、無視して `Invalid` にする。
pub fn keymap(settings: &Settings, ops: &'static [OperationDescriptor]) -> Keymap {
    let hotkeys = settings.hotkeys.assign();
    let shortcuts = Shortcuts::new(ops, &settings.shortcuts);
    let shortcut_item = |id: &str| format!("shortcuts.{id}");
    let hotkey_item = |action: HotkeyAction| format!("hotkeys.{}", action.key());

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
    Keymap {
        hotkeys,
        shortcuts,
        issues,
    }
}

/// 読み込み直しで変わった部分（`diff`）。`app` はこれを見て、変わったものだけを反映し直す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsChanges {
    /// `[hotkeys]`。ホットキーを登録し直す。
    pub hotkeys: bool,
    /// `[shortcuts]`。ショートカットの対応表とメニューを作り直す。
    pub shortcuts: bool,
    /// 全体の保存先かファイル名テンプレート。次の保存から使う。
    pub save: bool,
    /// 通知のオン/オフ。
    pub notifications: bool,
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
        hotkeys: old.hotkeys != new.hotkeys,
        shortcuts: old.shortcuts != new.shortcuts,
        save: old.general.save_dir != new.general.save_dir
            || old.general.file_name_template != new.general.file_name_template,
        notifications: old.general.notifications != new.general.notifications,
        appearance: old.scrap != new.scrap,
        log_level: old.log_level != new.log_level,
    }
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

    let hotkeys = r.table(root, "hotkeys");
    for action in HotkeyAction::ALL {
        if let Some(chord) = r.value(hotkeys, "hotkeys", action.key(), optional_chord) {
            *s.hotkeys.get_mut(action) = chord;
        }
    }

    if let Some(shortcuts) = r.table(root, "shortcuts") {
        for (op, _) in shortcuts.iter() {
            if let Some(chord) = r.value(Some(shortcuts), "shortcuts", op, optional_chord) {
                s.shortcuts.insert(op.to_owned(), chord);
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

    (s, r.issues)
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

[hotkeys]
select_capture = "Win+Shift+PrintScreen"

[shortcuts]
copy = "Ctrl+Shift+C"
close = ""

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
                },
                hotkeys: Hotkeys {
                    select_capture: Some(chord("Win+Shift+PrintScreen")),
                },
                shortcuts: BTreeMap::from([
                    ("close".to_owned(), None),
                    ("copy".to_owned(), Some(chord("Ctrl+Shift+C"))),
                ]),
                scrap: ScrapAppearance {
                    opacity: 80,
                    border: Border {
                        visible: false,
                        width: 3,
                        color: Rgb(0x1A, 0x2B, 0x3C),
                    },
                    shadow: false,
                },
                log_level: LogLevel::Debug,
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

[regions]
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
        };
        let assigned = hotkeys.assign();
        assert_eq!(
            assigned.bound,
            BTreeMap::from([(chord("Ctrl+Alt+S"), HotkeyAction::SelectCapture)])
        );
        assert!(assigned.conflicts.is_empty());
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
    }
}
