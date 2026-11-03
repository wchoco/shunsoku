//! リリースの workflow（`.github/workflows/release.yml`）が使う判定。

/// タグ（`v0.1.0`、`v0.1.0-rc.1` など）が workspace の version と一致するか。
/// プレリリースの部分（`-` から後ろ）は除いて比べる。
pub fn tag_matches_version(tag: &str, version: &str) -> bool {
    let Some(rest) = tag.strip_prefix('v') else {
        return false;
    };
    match rest.split_once('-') {
        Some((core, pre)) => !pre.is_empty() && core == version,
        None => rest == version,
    }
}

/// `docs/perf.md` の「累積表」の見出し行・区切り行・最新（最後）の行を、Markdown の表として返す。
/// 累積表が見つからないか、行が 1 つもなければ `None`。
pub fn latest_perf_table(perf_md: &str) -> Option<String> {
    let mut lines = perf_md
        .lines()
        .skip_while(|line| line.trim_end() != "## 累積表")
        .skip(1)
        .skip_while(|line| !line.starts_with('|'));
    let header = lines.next()?;
    let separator = lines.next()?;
    let latest = lines.take_while(|line| line.starts_with('|')).last()?;
    Some(format!("{header}\n{separator}\n{latest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERF_MD: &str = "\
# 性能の記録

| 指標 | 目標（参考） |
|---|---|
| 常駐時のメモリ | 30MB |

## 累積表

値は中央値 / p90。

| マイルストーン | 測った日 | メモリ |
|---|---|---|
| M1 | 2026-11-01 | 1.3 / 1.3 |
| M2 | 2026-11-02 | 1.4 / 1.5 |

測った環境は備考に書く。

### 参考値の累積表

| マイルストーン | 測った日 | 撮影 10 回後 |
|---|---|---|
| M1 | 2026-11-01 | 51.2 / 52.1 |
| M2 | 2026-11-02 | 51.4 / 52.1 |
";

    #[test]
    fn 累積表の見出しと最新の行を返す() {
        assert_eq!(
            latest_perf_table(PERF_MD).as_deref(),
            Some(
                "| マイルストーン | 測った日 | メモリ |\n\
                 |---|---|---|\n\
                 | M2 | 2026-11-02 | 1.4 / 1.5 |"
            )
        );
    }

    #[test]
    fn リポジトリの_perf_md_から最新の行が取れる() {
        let table = latest_perf_table(include_str!("../../../docs/perf.md")).unwrap();
        assert!(table.starts_with("| マイルストーン |"), "{table}");
        assert_eq!(table.lines().count(), 3, "{table}");
    }

    #[test]
    fn 累積表がないか行がなければ_none() {
        assert_eq!(
            latest_perf_table("# 性能の記録\n\n| a |\n|---|\n| 1 |\n"),
            None
        );
        assert_eq!(
            latest_perf_table("## 累積表\n\n| マイルストーン |\n|---|\n\n本文\n"),
            None
        );
    }

    #[test]
    fn 同じ版のタグは一致する() {
        assert!(tag_matches_version("v0.1.0", "0.1.0"));
    }

    #[test]
    fn プレリリースの部分は除いて比べる() {
        assert!(tag_matches_version("v0.1.0-rc.1", "0.1.0"));
    }

    #[test]
    fn 版の違うタグは一致しない() {
        assert!(!tag_matches_version("v0.2.0", "0.1.0"));
        assert!(!tag_matches_version("v0.2.0-rc.1", "0.1.0"));
        // 前方一致では通さない
        assert!(!tag_matches_version("v0.1.00", "0.1.0"));
        assert!(!tag_matches_version("v0.1", "0.1.0"));
    }

    #[test]
    fn v_で始まらないタグとプレリリースの部分が空のタグは一致しない() {
        assert!(!tag_matches_version("0.1.0", "0.1.0"));
        assert!(!tag_matches_version("v0.1.0-", "0.1.0"));
    }
}
