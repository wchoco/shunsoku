//! 性能の計測。本体と `tools/perf-harness` が共有する、計測のログの形式と集計（spec #32、`docs/perf.md`）。
//!
//! - 本体は環境変数 `SHUNSOKU_PERF_LOG` があるときだけ、区切り（`Mark`）ごとの QPC の時刻を `Recorder` で書く。
//! - ハーネスは書かれている途中のログを `PerfLog::parse` で読み、自分の QPC の時刻と比べる
//!   （QPC はプロセスをまたいで同じ時計）。各指標の 20 回分を `Summary` で中央値と p90 にする。

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

/// 計測のログの書き先を指す環境変数。cargo feature では切り替えない（spec #32）。
pub const LOG_VARIABLE: &str = "SHUNSOKU_PERF_LOG";

/// 環境変数 `SHUNSOKU_PERF_LOG` の値から、計測のログの書き先を決める。
/// 変数がないか空なら None で、そのときは何も書かない。
pub fn log_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// 区切り。ログには `name` の名前で書く。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// プロセスの開始（`main` に入った直後）。
    ProcessStart,
    /// トレイの最初の登録の完了（起動時に登録できなければ、あとで初めて登録できた時点）。
    TrayRegistered,
    /// ホットキーの受信（`WM_HOTKEY`）。
    HotkeyReceived,
    /// 範囲選択の画面を出し、全モニタで `DwmFlush` が返った時点。
    OverlayFlushed,
    /// スクラップを 1 枚出し終えた時点。常駐時のメモリの参考値（スクラップ 1 枚表示中）を読む合図。
    ScrapShown,
}

const MARKS: [Mark; 5] = [
    Mark::ProcessStart,
    Mark::TrayRegistered,
    Mark::HotkeyReceived,
    Mark::OverlayFlushed,
    Mark::ScrapShown,
];

impl Mark {
    /// ログに書く名前。
    pub fn name(self) -> &'static str {
        match self {
            Mark::ProcessStart => "process_start",
            Mark::TrayRegistered => "tray_registered",
            Mark::HotkeyReceived => "hotkey_received",
            Mark::OverlayFlushed => "overlay_flushed",
            Mark::ScrapShown => "scrap_shown",
        }
    }

    fn from_name(name: &str) -> Option<Mark> {
        MARKS.into_iter().find(|m| m.name() == name)
    }
}

/// ログの 1 行目に書く、QPC の周波数（1 秒あたりのカウント）の名前。
const FREQUENCY: &str = "frequency";

/// 区切りの時刻（QPC のカウント）を書く。1 行目に周波数、そのあと 1 区切り 1 行で `<名前> <カウント>`。
/// 読む側が書いている途中のファイルを読めるように、1 行ずつまとめて書いて flush する。
pub struct Recorder<W: Write> {
    out: W,
}

impl<W: Write> Recorder<W> {
    /// 周波数を書いて始める。
    pub fn new(out: W, frequency: i64) -> io::Result<Recorder<W>> {
        let mut rec = Recorder { out };
        rec.line(FREQUENCY, frequency)?;
        Ok(rec)
    }

    /// 区切りの時刻を書く。
    pub fn mark(&mut self, mark: Mark, ticks: i64) -> io::Result<()> {
        self.line(mark.name(), ticks)
    }

    fn line(&mut self, name: &str, value: i64) -> io::Result<()> {
        self.out.write_all(format!("{name} {value}\n").as_bytes())?;
        self.out.flush()
    }
}

/// 読んだ計測のログ。
#[derive(Debug, Default, PartialEq)]
pub struct PerfLog {
    /// QPC の周波数。
    pub frequency: Option<i64>,
    /// 書かれた順の区切りと時刻（QPC のカウント）。
    pub marks: Vec<(Mark, i64)>,
}

impl PerfLog {
    /// ログを読む。書いている途中の最後の行（改行で終わっていない行）と、読めない行は飛ばす。
    pub fn parse(text: &str) -> PerfLog {
        let complete = text.rfind('\n').map_or("", |end| &text[..end]);
        let mut log = PerfLog::default();
        for line in complete.lines() {
            let Some((name, value)) = line.trim().split_once(' ') else {
                continue;
            };
            let Ok(value) = value.trim().parse::<i64>() else {
                continue;
            };
            if name == FREQUENCY {
                log.frequency = Some(value);
            } else if let Some(mark) = Mark::from_name(name) {
                log.marks.push((mark, value));
            }
        }
        log
    }

    /// `mark` の `n` 回目（0 始まり）の時刻。
    pub fn nth(&self, mark: Mark, n: usize) -> Option<i64> {
        self.marks
            .iter()
            .filter(|(m, _)| *m == mark)
            .nth(n)
            .map(|&(_, ticks)| ticks)
    }

    /// `from` から `to` までのカウントを ms にする。周波数が書かれていなければ None。
    pub fn millis(&self, from: i64, to: i64) -> Option<f64> {
        let frequency = self.frequency.filter(|&f| f > 0)?;
        Some((to - from) as f64 * 1000.0 / frequency as f64)
    }
}

/// 測った値の中央値と p90。
///
/// - 中央値: 個数が偶数なら真ん中の 2 つの平均。
/// - p90: 最近順位法（小さい方から ceil(0.9 × 個数) 番目の値）。補間しないので、必ず測った値のどれか。
///   20 回なら 18 番目。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub median: f64,
    pub p90: f64,
}

impl Summary {
    /// 値の集まりをまとめる。空なら None。
    pub fn of(values: &[f64]) -> Option<Summary> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let median = if n.is_multiple_of(2) {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
        } else {
            sorted[n / 2]
        };
        // 最近順位法: 小さい方から ceil(0.9 n) 番目
        let rank = (n * 9).div_ceil(10);
        Some(Summary {
            count: n,
            median,
            p90: sorted[rank - 1],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn recorded_marks_are_read_back_in_order() {
        let mut out = Vec::new();
        let mut rec = Recorder::new(&mut out, 10_000_000).unwrap();
        rec.mark(Mark::ProcessStart, 100).unwrap();
        rec.mark(Mark::TrayRegistered, 2_500_100).unwrap();
        rec.mark(Mark::HotkeyReceived, 9_000_000).unwrap();
        rec.mark(Mark::OverlayFlushed, 9_400_000).unwrap();
        rec.mark(Mark::ScrapShown, 9_900_000).unwrap();

        let log = PerfLog::parse(std::str::from_utf8(&out).unwrap());
        assert_eq!(log.frequency, Some(10_000_000));
        assert_eq!(
            log.marks,
            vec![
                (Mark::ProcessStart, 100),
                (Mark::TrayRegistered, 2_500_100),
                (Mark::HotkeyReceived, 9_000_000),
                (Mark::OverlayFlushed, 9_400_000),
                (Mark::ScrapShown, 9_900_000),
            ]
        );
    }

    #[test]
    fn the_log_is_written_only_when_the_variable_names_a_file() {
        assert_eq!(log_path(None), None);
        assert_eq!(log_path(Some(OsString::new())), None);
        assert_eq!(
            log_path(Some(OsString::from(r"C:\tmp\perf.log"))),
            Some(PathBuf::from(r"C:\tmp\perf.log"))
        );
    }

    #[test]
    fn the_nth_occurrence_of_a_mark_and_its_distance_in_ms() {
        let log = PerfLog::parse(
            "frequency 10000000\n\
             process_start 100\n\
             tray_registered 2500100\n\
             hotkey_received 9000000\n\
             overlay_flushed 9400000\n\
             hotkey_received 20000000\n\
             overlay_flushed 20750000\n",
        );
        assert_eq!(log.nth(Mark::OverlayFlushed, 1), Some(20_750_000));
        assert_eq!(log.nth(Mark::OverlayFlushed, 2), None);
        assert_eq!(log.millis(2_000_000, 20_750_000), Some(1875.0));
        // 周波数がなければ ms にできない
        assert_eq!(PerfLog::parse("process_start 1\n").millis(0, 1), None);
    }

    #[test]
    fn a_line_still_being_written_is_not_read() {
        // "tray_registered 2500100\n" の途中まで書かれたところを読んだ
        let log = PerfLog::parse("frequency 10000000\nprocess_start 100\ntray_registered 25");
        assert_eq!(log.marks, vec![(Mark::ProcessStart, 100)]);
    }

    #[test]
    fn summary_of_twenty_values_takes_the_middle_pair_and_the_18th() {
        // 1..=20 を混ぜた順で渡す
        let values: Vec<f64> = [
            7, 3, 20, 11, 1, 15, 9, 18, 2, 14, 5, 19, 10, 6, 16, 4, 12, 17, 8, 13,
        ]
        .iter()
        .map(|&v| f64::from(v))
        .collect();
        let s = Summary::of(&values).unwrap();
        assert_eq!(s.count, 20);
        assert_eq!(s.median, 10.5);
        assert_eq!(s.p90, 18.0);
    }

    #[test]
    fn summary_of_an_odd_count_takes_the_middle_value() {
        let s = Summary::of(&[30.0, 10.0, 20.0]).unwrap();
        assert_eq!((s.count, s.median, s.p90), (3, 20.0, 30.0));
    }

    #[test]
    fn summary_of_one_value_is_that_value() {
        let s = Summary::of(&[4.5]).unwrap();
        assert_eq!((s.median, s.p90), (4.5, 4.5));
    }

    #[test]
    fn summary_of_nothing_is_none() {
        assert_eq!(Summary::of(&[]), None);
    }
}
