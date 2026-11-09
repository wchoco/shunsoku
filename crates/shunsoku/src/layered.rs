//! レイヤード ウィンドウに中身を出す（`UpdateLayeredWindowIndirect`）。スクラップ（`scrap_window`）と書き込みのツールバー
//! （`toolbar_window`）の両方が使う、ウィンドウを持たない道具（`wide` と同じ）。ウィンドウのモジュール同士が
//! 呼び合わないように（ADR-0005）、ここに置く。

use shunsoku_core::geometry::Rect;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, HDC};
use windows::Win32::UI::WindowsAndMessaging::{
    ULW_ALPHA, UPDATELAYEREDWINDOWINFO, UpdateLayeredWindowIndirect,
};
use windows::core::Result;

/// レイヤード ウィンドウ `hwnd` に、`source`（premultiplied BGRA の DIB を選んだ DC、大きさは `rect` と同じ）の
/// 中身を `rect` で出す。`constant_alpha` は全体のアルファ（`Appearance::constant_alpha`）。
/// `dirty`（`source` の中の座標）があれば、その範囲だけを写す（範囲の外は前に出した中身のまま）。前に出したときと
/// 大きさが同じときだけ渡すこと。
pub fn present(
    hwnd: HWND,
    source: HDC,
    rect: Rect,
    dirty: Option<Rect>,
    constant_alpha: u8,
) -> Result<()> {
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
        SourceConstantAlpha: constant_alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let dirty = dirty.map(|r| RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    });
    let info = UPDATELAYEREDWINDOWINFO {
        cbSize: size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
        hdcDst: HDC::default(),
        pptDst: &position,
        psize: &size,
        hdcSrc: source,
        pptSrc: &origin,
        crKey: COLORREF(0),
        pblend: &blend,
        dwFlags: ULW_ALPHA,
        prcDirty: dirty
            .as_ref()
            .map_or(std::ptr::null(), |r| r as *const RECT),
    };
    // SAFETY: 呼び元のスレッドのウィンドウ。info の指す値はこの関数の中で生きている
    unsafe { UpdateLayeredWindowIndirect(hwnd, &info) }.ok()
}
