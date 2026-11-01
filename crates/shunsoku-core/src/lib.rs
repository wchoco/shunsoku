//! shunsoku のロジック層。`windows` クレートに依存せず、Windows API なしでテストできる（ADR-0005）。

pub mod capture;
pub mod i18n;
pub mod log_file;

/// アプリの名前。
pub const APP_NAME: &str = "shunsoku";
