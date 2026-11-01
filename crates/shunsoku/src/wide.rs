//! Win32 に渡す UTF-16 の文字列。

/// NUL で終わる UTF-16 にする。`PCWSTR(v.as_ptr())` で渡す（`v` は呼び出しの間生かしておく）。
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// 固定長のバッファ（`szTip` など）に NUL で終わるように写す。入りきらない分は切り捨てる。
pub fn copy_truncated(dst: &mut [u16], s: &str) {
    let Some(max) = dst.len().checked_sub(1) else {
        return;
    };
    let mut n = 0;
    for unit in s.encode_utf16() {
        if n == max {
            break;
        }
        dst[n] = unit;
        n += 1;
    }
    // サロゲート ペアの途中で切れたら、前半も捨てる
    if n > 0 && (0xD800..0xDC00).contains(&dst[n - 1]) {
        n -= 1;
    }
    dst[n] = 0;
}
