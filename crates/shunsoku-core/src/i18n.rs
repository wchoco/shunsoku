//! UI の文言。`locales/*.toml` をキーから引く。
//!
//! テーブルは `.` でつないだキーになる（`[tray.menu]` の `exit` は `tray.menu.exit`）。
//! 文字列でない値は無視する。
//!
//! 文言のファイルは言語（`Language`）ごとに 1 冊で、exe に埋め込む（`catalog`）。キーは ja と en でそろえる
//! （テストで確かめる。ja に足したら en にも足す）。

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::LazyLock;

use toml_edit::{DocumentMut, TableLike};

/// UI の言語。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Japanese,
    English,
}

impl Language {
    /// 設定 GUI の並びの順。
    pub const ALL: [Language; 2] = [Language::Japanese, Language::English];

    /// OS の表示言語（`GetUserDefaultUILanguage` の LANGID）から決める既定の言語。日本語でなければ英語。
    pub fn from_ui_language(langid: u16) -> Language {
        const LANG_JAPANESE: u16 = 0x11;
        if langid & 0x3ff == LANG_JAPANESE {
            Language::Japanese
        } else {
            Language::English
        }
    }

    /// 設定ファイルに書く名前（`[general] language`）。
    pub fn code(self) -> &'static str {
        match self {
            Language::Japanese => "ja",
            Language::English => "en",
        }
    }

    /// `code` の逆。
    pub fn from_code(code: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.code() == code)
    }
}

static JAPANESE: LazyLock<Catalog> =
    LazyLock::new(|| bundled(include_str!("../../../locales/ja.toml")));
static ENGLISH: LazyLock<Catalog> =
    LazyLock::new(|| bundled(include_str!("../../../locales/en.toml")));

/// 埋め込んだ言語の文言の表。
pub fn catalog(language: Language) -> &'static Catalog {
    match language {
        Language::Japanese => &JAPANESE,
        Language::English => &ENGLISH,
    }
}

/// 埋め込んだ文言のファイルが TOML として読めることは、テストで確かめている。読めなければ空の表（キーが
/// そのまま出る）。
fn bundled(src: &str) -> Catalog {
    Catalog::parse(src).unwrap_or_default()
}

/// 文言の表。
#[derive(Debug, Default)]
pub struct Catalog {
    texts: HashMap<String, String>,
}

impl Catalog {
    /// TOML の文字列から表を作る。
    pub fn parse(src: &str) -> Result<Catalog, ParseError> {
        let doc: DocumentMut = src.parse().map_err(|e| ParseError(format!("{e}")))?;
        let mut texts = HashMap::new();
        collect("", doc.as_table(), &mut texts);
        Ok(Catalog { texts })
    }

    /// キーの文言を返す。キーがなければキーそのものを返す。
    pub fn text<'a>(&'a self, key: &'a str) -> &'a str {
        self.texts.get(key).map_or(key, String::as_str)
    }

    /// 表にあるキーの一覧。
    pub fn keys(&self) -> BTreeSet<&str> {
        self.texts.keys().map(String::as_str).collect()
    }
}

fn collect(prefix: &str, table: &dyn TableLike, texts: &mut HashMap<String, String>) {
    for (key, item) in table.iter() {
        let full = if prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{prefix}.{key}")
        };
        if let Some(s) = item.as_str() {
            texts.insert(full, s.to_owned());
        } else if let Some(sub) = item.as_table_like() {
            collect(&full, sub, texts);
        }
    }
}

/// 文言のファイルが TOML として読めない。
#[derive(Debug)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_text_by_dotted_key() {
        let catalog = Catalog::parse("[tray.menu]\nexit = \"終了\"\n").unwrap();
        assert_eq!(catalog.text("tray.menu.exit"), "終了");
    }

    #[test]
    fn returns_key_itself_when_missing() {
        let catalog = Catalog::parse("[tray.menu]\nexit = \"終了\"\n").unwrap();
        assert_eq!(catalog.text("tray.menu.nope"), "tray.menu.nope");
        assert_eq!(catalog.text("tray.menu"), "tray.menu");
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(Catalog::parse("exit = \"終了").is_err());
    }

    #[test]
    fn japanese_and_english_catalogs_have_the_same_keys() {
        let ja = catalog(Language::Japanese).keys();
        let en = catalog(Language::English).keys();
        let only_ja: Vec<&str> = ja.difference(&en).copied().collect();
        let only_en: Vec<&str> = en.difference(&ja).copied().collect();
        // 片方にしかないキーは、もう片方の locales/*.toml に足す
        assert!(only_ja.is_empty(), "missing in en.toml: {only_ja:?}");
        assert!(only_en.is_empty(), "missing in ja.toml: {only_en:?}");
    }

    #[test]
    fn bundled_catalogs_are_looked_up_by_language() {
        assert_eq!(catalog(Language::Japanese).text("tray.menu.exit"), "終了");
        assert_eq!(catalog(Language::English).text("tray.menu.exit"), "Exit");
    }

    #[test]
    fn the_os_display_language_decides_the_default_language() {
        // LANGID: 下位 10 ビットが主言語（LANG_JAPANESE = 0x11）
        assert_eq!(Language::from_ui_language(0x0411), Language::Japanese); // ja-JP
        assert_eq!(Language::from_ui_language(0x0409), Language::English); // en-US
        assert_eq!(Language::from_ui_language(0x0809), Language::English); // en-GB
        assert_eq!(Language::from_ui_language(0x0804), Language::English); // zh-CN
        assert_eq!(Language::from_ui_language(0), Language::English);
    }

    #[test]
    fn bundled_japanese_catalog_has_tray_menu_texts() {
        let catalog = catalog(Language::Japanese);
        assert_eq!(catalog.text("tray.menu.exit"), "終了");
        assert_eq!(
            catalog.text("tray.menu.select_capture"),
            "範囲選択キャプチャ"
        );
        assert_eq!(catalog.text("tray.tooltip"), "shunsoku");
    }
}
