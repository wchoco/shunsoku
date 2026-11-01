//! 配布物の性能を測るハーネス（開発専用。zip には入れない）。測り方と指標の定義は `docs/perf.md`。
//!
//! ```text
//! perf-harness <shunsoku.exe> [--runs N] [--hotkey <組み合わせ>] [memory] [hotkey] [cold-start]
//! ```
//!
//! 指標を書かなければ 3 つとも測る。本体は `SHUNSOKU_PERF_LOG` で区切りの時刻を書き、ハーネスはそれを読む。

#[cfg(windows)]
mod measure;

use std::path::PathBuf;
use std::str::FromStr;

use shunsoku_core::keys::Chord;

/// 各指標を測る回数の既定値（spec #32）。
const DEFAULT_RUNS: usize = 20;

/// ハーネスが本体に設定するホットキーの既定値。ほかのアプリと重なりにくい組み合わせ。
const DEFAULT_HOTKEY: &str = "Ctrl+Alt+Shift+F9";

const USAGE: &str = "usage: perf-harness <shunsoku.exe> [--runs N] [--hotkey <組み合わせ>] [memory] [hotkey] [cold-start]";

/// 測る指標。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// 常駐時のメモリ（Private Working Set）。
    Memory,
    /// ホットキーから範囲選択の画面まで。
    Hotkey,
    /// コールドスタート。
    ColdStart,
}

/// コマンドラインの指定。
#[derive(Debug)]
pub struct Options {
    pub exe: PathBuf,
    pub runs: usize,
    pub hotkey: Chord,
    pub metrics: Vec<Metric>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut exe = None;
    let mut runs = DEFAULT_RUNS;
    let mut hotkey = Chord::from_str(DEFAULT_HOTKEY).map_err(|e| e.to_string())?;
    let mut metrics = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runs" => {
                let value = args.next().ok_or("--runs の値がない")?;
                runs = value
                    .parse()
                    .ok()
                    .filter(|&n| n > 0)
                    .ok_or_else(|| format!("--runs の値が正の整数でない: {value}"))?;
            }
            "--hotkey" => {
                let value = args.next().ok_or("--hotkey の値がない")?;
                hotkey = Chord::from_str(&value).map_err(|e| e.to_string())?;
            }
            "memory" => metrics.push(Metric::Memory),
            "hotkey" => metrics.push(Metric::Hotkey),
            "cold-start" => metrics.push(Metric::ColdStart),
            "-h" | "--help" => return Err(USAGE.to_owned()),
            _ if arg.starts_with('-') => return Err(format!("知らないオプション: {arg}\n{USAGE}")),
            _ if exe.is_none() => exe = Some(PathBuf::from(arg)),
            _ => return Err(format!("知らない指標: {arg}\n{USAGE}")),
        }
    }
    if metrics.is_empty() {
        metrics = vec![Metric::Memory, Metric::Hotkey, Metric::ColdStart];
    }
    Ok(Options {
        exe: exe.ok_or(USAGE)?,
        runs,
        hotkey,
        metrics,
    })
}

fn main() {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    #[cfg(windows)]
    if let Err(e) = measure::run(&options) {
        eprintln!("計測に失敗しました: {e}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    {
        let _ = options;
        eprintln!("perf-harness は Windows でだけ動きます");
        std::process::exit(1);
    }
}
