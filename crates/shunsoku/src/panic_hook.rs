//! パニックのフック。どのスレッドのパニックもログに書く。UI スレッドのパニックではメッセージ ボックスを出す。
//!
//! UI スレッドのパニックのあとの終わり方は `app` が受け持つ（ウィンドウ プロシージャの `catch_unwind`）。
//! 裏のスレッドのパニックはログに書くだけで、その仕事の失敗として扱うのは呼び出し側。

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;

use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
};
use windows::core::PCWSTR;

use crate::wide::to_wide;

static UI_PANICKED: AtomicBool = AtomicBool::new(false);

/// UI スレッドがパニックしたか。立ったあとは、ウィンドウ プロシージャはアプリの状態に触らない。
pub fn ui_panicked() -> bool {
    UI_PANICKED.load(Ordering::SeqCst)
}

/// フックを入れる。`texts` はメッセージ ボックスの題と本文（`i18n` で引いたもの）を返す。パニックした時点の
/// UI の言語で引くため、出すときに呼ぶ。
pub fn install(ui_thread: ThreadId, texts: impl Fn() -> (String, String) + Send + Sync + 'static) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let message = info.payload_as_str().unwrap_or("(no message)");
        let location = info.location().map(|l| l.to_string()).unwrap_or_default();
        log::error!(
            "panic on thread '{}' at {location}: {message}",
            thread.name().unwrap_or("?")
        );
        // デバッグ ビルドのコンソール向け
        default_hook(info);

        // メッセージ ボックスの間もメッセージは配られるので、先に旗を立てる
        if thread.id() == ui_thread && !UI_PANICKED.swap(true, Ordering::SeqCst) {
            let (title, body) = texts();
            let title = to_wide(&title);
            let text = to_wide(&format!("{body}\n\n{message}"));
            // SAFETY: 文字列はこの呼び出しの間生きている
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND,
                );
            }
        }
    }));
}
