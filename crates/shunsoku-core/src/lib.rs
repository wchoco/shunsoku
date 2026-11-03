//! shunsoku のロジック層。`windows` クレートに依存せず、Windows API なしでテストできる（ADR-0005）。

pub mod annotate;
pub mod app_window;
pub mod autostart;
pub mod capture;
pub mod cli_args;
pub mod cursor;
pub mod geometry;
pub mod i18n;
pub mod image;
pub mod keys;
pub mod log_file;
pub mod magnifier;
pub mod menu;
pub mod monitors;
pub mod notice;
pub mod perf;
pub mod protocol;
pub mod recipe;
pub mod scrap;
pub mod selection;
pub mod settings;
pub mod template;
pub mod trash;
pub mod tray;

/// アプリの名前。
pub const APP_NAME: &str = "shunsoku";
