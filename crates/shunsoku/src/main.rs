//! shunsoku の常駐本体。Win32 層（`windows` クレートを使うモジュール）は Windows でだけ組む。

// リリースではコンソールを出さない。デバッグでは標準エラーを見られるようにコンソールを残す。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
mod grab;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod logger;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod notify;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod panic_hook;
#[cfg(windows)]
mod perf;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod save;
#[cfg(windows)]
mod scrap_window;
#[cfg(windows)]
mod single_instance;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod wide;

#[cfg(windows)]
fn main() {
    std::process::exit(app::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("{} は Windows でだけ動きます", shunsoku_core::APP_NAME);
    std::process::exit(1);
}
