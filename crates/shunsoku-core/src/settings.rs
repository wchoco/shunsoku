//! 設定ファイル（`%APPDATA%\shunsoku\settings.toml`）の読み込み。
//!
//! - `load` は項目ごとに値を取り出す。不正な項目はその項目だけ既定値にし、項目名を `SettingsIssue` に載せる。
//!   知らないキーは無視する。ファイル全体が TOML として読めなければ、すべて既定値にする。
//! - ファイルは書き換えない（`write` は M3）。読み込み直しの差分と重なりの検出は設定の監視のチケット（#46）で足す。
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

/// 設定ファイルの問題。通知で知らせる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsIssue {
    /// ファイル全体が TOML として読めない。すべて既定値になる。
    NotToml { message: String },
    /// 不正な項目。その項目だけ既定値になる。`item` は `general.save_dir` のような `.` でつないだ名前。
    Invalid { item: String },
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
    fn text_that_is_not_toml_gives_defaults_and_one_issue() {
        let (settings, issues) = load("version = 1\n[hotkeys\nselect_capture = \"Ctrl+A\"\n");
        assert_eq!(settings, Settings::default());
        assert!(
            matches!(issues.as_slice(), [SettingsIssue::NotToml { message }] if !message.is_empty()),
            "{issues:?}"
        );
    }
}
