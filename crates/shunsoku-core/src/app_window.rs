//! 本体のウィンドウと、外から操作する道具（`tools/perf-harness`、`tools/win-driver`）との約束。
//! 道具は本体のクラス名でウィンドウを探し、トレイの通知のメッセージを本体に送り、一覧の文字が切れていないかを
//! 本体と同じ余白で測る。ずれないよう、ここに 1 つだけ置く。

/// 本体のメッセージ専用ウィンドウのクラス名。
pub const APP_CLASS: &str = "shunsoku.app";

/// [`APP_CLASS`] の UTF-16（NUL 終わり）。`RegisterClassExW` と `FindWindowExW` に渡す。
pub const APP_CLASS_WIDE: &[u16] = &ascii_wide::<13>(APP_CLASS);

/// トレイのアイコンの通知のメッセージの、`WM_APP` からの差（メッセージは `WM_APP + 1`）。
pub const TRAY_MESSAGE_FROM_WM_APP: u32 = 1;

/// トレイのアイコンの番号（`NOTIFYICONDATAW::uID`）。通知も同じアイコンに出す。
pub const TRAY_ICON_ID: u32 = 1;

/// 設定のダイアログの一覧の、見出しの文言の左右の余白の合計（96 DPI のピクセル）。
pub const LIST_HEADER_PADDING: i32 = 14;

/// ASCII の文字列を NUL 終わりの UTF-16 にする。`N` は文字数 + 1。
const fn ascii_wide<const N: usize>(s: &str) -> [u16; N] {
    let bytes = s.as_bytes();
    assert!(bytes.len() + 1 == N);
    let mut out = [0u16; N];
    let mut i = 0;
    while i < bytes.len() {
        assert!(bytes[i].is_ascii());
        out[i] = bytes[i] as u16;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wide_class_name_is_the_class_name_with_a_nul() {
        let expected: Vec<u16> = "shunsoku.app".encode_utf16().chain([0]).collect();
        assert_eq!(APP_CLASS_WIDE, expected.as_slice());
    }
}
