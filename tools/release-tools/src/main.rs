//! リリースの workflow（`.github/workflows/release.yml`）の下ごしらえ。開発専用で zip には入れない。
//!
//! ```text
//! release-tools check-version <タグ>   タグが workspace の version と一致しなければ終了コード 1
//! release-tools perf-table <perf.md>  累積表の見出しと最新の行を標準出力に出す
//! ```

use std::process::ExitCode;

use release_tools::{latest_perf_table, tag_matches_version};

const USAGE: &str = "usage: release-tools check-version <tag> | perf-table <perf.md>";

/// workspace の version（`Cargo.toml` の `[workspace.package]`）。
const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["check-version", tag] => {
            if tag_matches_version(tag, WORKSPACE_VERSION) {
                println!("タグ {tag} は workspace の version {WORKSPACE_VERSION} と一致する");
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "タグ {tag} が workspace の version {WORKSPACE_VERSION} と一致しない（v<version> か v<version>-<プレリリース> にする）"
                );
                ExitCode::FAILURE
            }
        }
        ["perf-table", path] => {
            let text = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("{path} を読めない: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match latest_perf_table(&text) {
                Some(table) => {
                    println!("{table}");
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("{path} に累積表の行が見つからない");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
