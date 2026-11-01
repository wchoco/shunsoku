//! 計測のログ。環境変数 `SHUNSOKU_PERF_LOG=<パス>` があるときだけ、区切りごとの QPC の時刻をそのファイルに
//! 書く（形式は core の `perf`）。ないときは何も開かず、`mark` は何もしない。
//!
//! 呼ぶのは `app` だけ。区切りの意味は `shunsoku_core::perf::Mark` と `docs/perf.md` にある。

use std::fs::File;
use std::io;
use std::sync::{Mutex, OnceLock};

use shunsoku_core::perf::{self, Mark, Recorder};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

static RECORDER: OnceLock<Mutex<Recorder<File>>> = OnceLock::new();

/// プロセスの開始を記録する。`run` の最初に 1 回だけ呼ぶ。変数がなければ何もしない。
/// ログはまだ使えないので、開けなかったときのエラーは返して、呼んだ側があとでログに書く。
pub fn start() -> io::Result<()> {
    // 時刻はファイルを開く前に取る
    let ticks = now();
    let Some(path) = perf::log_path(std::env::var_os(perf::LOG_VARIABLE)) else {
        return Ok(());
    };
    let mut frequency = 0;
    // SAFETY: frequency はこの関数の中で生きている
    unsafe { QueryPerformanceFrequency(&mut frequency) }?;
    let mut recorder = Recorder::new(File::create(path)?, frequency)?;
    recorder.mark(Mark::ProcessStart, ticks)?;
    let _ = RECORDER.set(Mutex::new(recorder));
    Ok(())
}

/// 区切りの時刻を書く。計測していなければ何もしない。書けなくても動作は続ける。
pub fn mark(mark: Mark) {
    let Some(recorder) = RECORDER.get() else {
        return;
    };
    let ticks = now();
    if let Ok(mut recorder) = recorder.lock()
        && let Err(e) = recorder.mark(mark, ticks)
    {
        log::warn!("could not write the perf log: {e}");
    }
}

fn now() -> i64 {
    let mut ticks = 0;
    // SAFETY: ticks はこの関数の中で生きている。XP 以降は失敗しない
    let _ = unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}
