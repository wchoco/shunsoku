//! メニューの項目の文言。

/// ユーザーが付けた名前などを、メニューの項目にそのまま出す文言にする。メニューは `&` を次の文字の
/// アクセスキーの印として読み、`&` 自体は出さないので、`&&` にして `&` を 1 文字として出す。
pub fn literal_label(text: &str) -> String {
    text.replace('&', "&&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ampersand_in_a_name_is_shown_as_it_is() {
        assert_eq!(literal_label("R&D"), "R&&D");
        assert_eq!(literal_label("&&"), "&&&&");
        assert_eq!(literal_label("全画面 & 右"), "全画面 && 右");
    }

    #[test]
    fn a_name_without_an_ampersand_is_unchanged() {
        assert_eq!(literal_label("左半分"), "左半分");
        assert_eq!(literal_label(""), "");
    }
}
