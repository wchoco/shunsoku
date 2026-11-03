//! テスト用のインスタンスに付ける環境変数（`docs/agents/manual-check.md` の「起動の安全」）。
//!
//! 起動するコマンドは必ずここを通す。付けずに起動する道はない。

/// `root`（テスト用のフォルダ）で起動するときに上書きする環境変数。
///
/// - `APPDATA`（設定とログ）、`TEMP` / `TMP` は `root` の下にする。ポータブルモードの項目（`portable`）だけ
///   `APPDATA` を上書きせず、`%APPDATA%` が使われないことを確かめられるようにする。
/// - `USERNAME`（多重起動の判定の名前）と `SHUNSOKU_AUTOSTART_VALUE`（Run キーの値の名前）は、
///   `root` のフォルダ名から `shunsoku-test-<名前>` にする。ユーザーの常駐と Run キーの "shunsoku" に触れない。
pub fn test_env(root: &str, portable: bool) -> Vec<(&'static str, String)> {
    let root = root.trim_end_matches(['\\', '/']);
    let name = root.rsplit(['\\', '/']).next().unwrap_or(root);
    let test_name = format!("shunsoku-test-{name}");
    let temp = format!(r"{root}\temp");
    let mut env = vec![
        ("TEMP", temp.clone()),
        ("TMP", temp),
        ("USERNAME", test_name.clone()),
        ("SHUNSOKU_AUTOSTART_VALUE", test_name),
    ];
    if !portable {
        env.push(("APPDATA", format!(r"{root}\appdata")));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(env: &'a [(&str, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn everything_points_into_the_test_folder() {
        let env = test_env(r"C:\Users\me\shunsoku-build\check-m5", false);
        assert_eq!(
            get(&env, "APPDATA"),
            Some(r"C:\Users\me\shunsoku-build\check-m5\appdata")
        );
        assert_eq!(
            get(&env, "TEMP"),
            Some(r"C:\Users\me\shunsoku-build\check-m5\temp")
        );
        assert_eq!(
            get(&env, "TMP"),
            Some(r"C:\Users\me\shunsoku-build\check-m5\temp")
        );
        assert_eq!(get(&env, "USERNAME"), Some("shunsoku-test-check-m5"));
        assert_eq!(
            get(&env, "SHUNSOKU_AUTOSTART_VALUE"),
            Some("shunsoku-test-check-m5")
        );
    }

    #[test]
    fn portable_drops_only_appdata() {
        let env = test_env(r"C:\t\m5\", true);
        assert_eq!(get(&env, "APPDATA"), None);
        assert_eq!(get(&env, "TEMP"), Some(r"C:\t\m5\temp"));
        assert_eq!(get(&env, "USERNAME"), Some("shunsoku-test-m5"));
        assert_eq!(
            get(&env, "SHUNSOKU_AUTOSTART_VALUE"),
            Some("shunsoku-test-m5")
        );
    }
}
