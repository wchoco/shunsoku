//! `log` のファサードの書き出し先。`%APPDATA%\shunsoku\shunsoku.log` に書き、1MB を超えたら `.old` に回す。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};
use shunsoku_core::log_file::RotatingFile;
use windows::Win32::System::SystemInformation::GetLocalTime;

/// ログのファイルの大きさの上限。
const LIMIT: u64 = 1024 * 1024;

struct FileLogger {
    file: Mutex<RotatingFile>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // SAFETY: 引数のない取得だけ
        let t = unsafe { GetLocalTime() };
        let thread = std::thread::current();
        let line = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {:<5} [{}] {}: {}",
            t.wYear,
            t.wMonth,
            t.wDay,
            t.wHour,
            t.wMinute,
            t.wSecond,
            t.wMilliseconds,
            record.level(),
            thread.name().unwrap_or("?"),
            record.target(),
            record.args()
        );
        // 書けなくてもアプリは止めない
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = file.write_line(&line);
    }

    fn flush(&self) {}
}

/// `dir` の `shunsoku.log` をログの書き出し先にする。フォルダがなければ作る。既定のレベルは `info`。
/// 失敗したらログは捨てられる（アプリは動かし続ける）。
pub fn init(dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("shunsoku.log");
    let file = RotatingFile::open(&path, LIMIT)?;
    let logger = Box::leak(Box::new(FileLogger {
        file: Mutex::new(file),
    }));
    log::set_logger(logger).map_err(|_| io::Error::other("logger is already set"))?;
    log::set_max_level(LevelFilter::Info);
    Ok(path)
}
