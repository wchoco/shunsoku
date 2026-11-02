//! レイヤード ウィンドウに中身を出す（`UpdateLayeredWindow`）。スクラップ（`scrap_window`）と書き込みのツールバー
//! （`toolbar_window`）の両方が使う、ウィンドウを持たない道具（`wide` と同じ）。ウィンドウのモジュール同士が
//! 呼び合わないように（ADR-0005）、ここに置く。

use shunsoku_core::geometry::Rect;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, HDC};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::Result;

/// レイヤード ウィンドウ `hwnd` に、`source`（premultiplied BGRA の DIB を選んだ DC、大きさは `rect` と同じ）の
/// 中身を `rect` で出す。`opacity` は透明度（%）。
pub fn present(hwnd: HWND, source: HDC, rect: Rect, opacity: u8) -> Result<()> {
    let position = POINT {
        x: rect.left,
        y: rect.top,
    };
    let size = SIZE {
        cx: rect.width() as i32,
        cy: rect.height() as i32,
    };
    let origin = POINT::default();
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: (u32::from(opacity.min(100)) * 255 / 100) as u8,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    // SAFETY: 呼び元のスレッドのウィンドウ。引数はこの関数の中で生きている
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            None,
            Some(&position),
            Some(&size),
            Some(source),
            Some(&origin),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    }
}
