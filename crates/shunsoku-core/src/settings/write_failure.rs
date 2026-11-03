//! 設定ファイルに書き戻せなかった理由（`WriteFailure`）と、通知に添える文言。

use crate::i18n::Catalog;

/// 設定ファイルに書き戻せなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteFailure {
    /// 設定ファイルの場所が決まっていない。
    NoLocation,
    /// 今のファイルが TOML として読めないので、手で書いた中身を消さないよう上書きしなかった。
    NotToml { path: String },
    /// ファイルの操作を OS に断られた。
    Io { path: String, error: String },
}

impl WriteFailure {
    /// 通知の本文。文言のキー `headline` の「書けませんでした」の行の下に、理由の 1 行（`detail`）を添える。
    pub fn notice(&self, catalog: &Catalog, headline: &str) -> String {
        format!("{}\n{}", catalog.text(headline), self.detail(catalog))
    }

    /// 通知で「書けませんでした」の後ろに添える、理由の 1 行。
    ///
    /// - OS に断られたときは、断られたファイルと OS のエラーの文言だけを出す。書く前の読み直しで断られても
    ///   「読めない」とは言わない（書けなかったことは前の行が言う。M5-16）。OS のエラーの文言は OS の言語で出る。
    /// - それ以外は UI の言語の文言にする。
    pub fn detail(&self, catalog: &Catalog) -> String {
        match self {
            WriteFailure::NoLocation => catalog.text("settings.write_no_location").to_owned(),
            WriteFailure::NotToml { path } => {
                format!("{}{path}", catalog.text("settings.write_not_toml"))
            }
            WriteFailure::Io { path, error } => format!("{path}: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, catalog};

    #[test]
    fn an_io_failure_gives_the_path_and_the_os_error_without_saying_read() {
        // M5-16: 書く前の読み直しを OS に断られても、通知は「読めない」と言わない
        let failure = WriteFailure::Io {
            path: r"C:\portC\settings.toml".to_owned(),
            error: "アクセスが拒否されました。 (os error 5)".to_owned(),
        };
        for language in Language::ALL {
            assert_eq!(
                failure.detail(catalog(language)),
                r"C:\portC\settings.toml: アクセスが拒否されました。 (os error 5)"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_toml_says_it_was_not_overwritten_in_the_ui_language() {
        let failure = WriteFailure::NotToml {
            path: r"C:\portC\settings.toml".to_owned(),
        };
        assert_eq!(
            failure.detail(catalog(Language::Japanese)),
            r"今の設定ファイルを TOML として読めないため、上書きしませんでした: C:\portC\settings.toml"
        );
        assert_eq!(
            failure.detail(catalog(Language::English)),
            r"The current settings file is not valid TOML, so it was not overwritten: C:\portC\settings.toml"
        );
    }

    #[test]
    fn the_notice_puts_the_headline_above_the_reason() {
        let failure = WriteFailure::Io {
            path: r"C:\portC\settings.toml".to_owned(),
            error: "Access is denied. (os error 5)".to_owned(),
        };
        assert_eq!(
            failure.notice(catalog(Language::English), "region.register_failed"),
            "Could not save the region.\nC:\\portC\\settings.toml: Access is denied. (os error 5)"
        );
        assert_eq!(
            WriteFailure::NoLocation
                .notice(catalog(Language::Japanese), "settings_gui.write_failed"),
            "設定ファイルに書けませんでした。設定 GUI の内容はまだ適用されていません。\n設定ファイルの場所が決まっていません。"
        );
    }

    #[test]
    fn an_unknown_location_says_so_in_the_ui_language() {
        assert_eq!(
            WriteFailure::NoLocation.detail(catalog(Language::Japanese)),
            "設定ファイルの場所が決まっていません。"
        );
        assert_eq!(
            WriteFailure::NoLocation.detail(catalog(Language::English)),
            "The location of the settings file is unknown."
        );
    }
}
