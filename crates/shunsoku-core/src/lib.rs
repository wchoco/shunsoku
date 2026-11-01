//! shunsoku のロジック層。`windows` クレートに依存せず、Windows API なしでテストできる（ADR-0005）。

pub mod capture;
pub mod geometry;
pub mod i18n;
pub mod image;
pub mod keys;
pub mod log_file;
pub mod perf;
pub mod scrap;
pub mod selection;
pub mod settings;

/// アプリの名前。
pub const APP_NAME: &str = "shunsoku";
