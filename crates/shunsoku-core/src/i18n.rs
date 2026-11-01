//! UI の文言。`locales/*.toml` をキーから引く。
//!
//! テーブルは `.` でつないだキーになる（`[tray.menu]` の `exit` は `tray.menu.exit`）。
//! 文字列でない値は無視する。

use std::collections::HashMap;
use std::fmt;

use toml_edit::{DocumentMut, TableLike};

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
    fn bundled_japanese_catalog_has_tray_menu_texts() {
        let catalog = Catalog::parse(include_str!("../../../locales/ja.toml")).unwrap();
        assert_eq!(catalog.text("tray.menu.exit"), "終了");
        assert_eq!(catalog.text("tray.tooltip"), "shunsoku");
    }
}
