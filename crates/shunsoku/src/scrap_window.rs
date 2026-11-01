//! スクラップのウィンドウ: 最前面のレイヤード ウィンドウ。中身は `UpdateLayeredWindow` で
//! premultiplied BGRA の DIB（`render::Canvas` の DC）から出す。
//!
//! - 位置と大きさはスクリーン座標の物理ピクセル。画素を等倍で出すので、DPI が変わっても大きさを変えない
//!   （ウィンドウ プロシージャは `app` が持ち、`WM_DPICHANGED` で 0 を返す）。
//! - 透明度は `SourceConstantAlpha` で掛ける。
//! - drop でウィンドウを壊す。

use shunsoku_core::geometry::Rect;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, HDC};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, IDC_ARROW, LoadCursorW, RegisterClassExW, SW_SHOW,
    SW_SHOWNOACTIVATE, SetForegroundWindow, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
    WNDCLASSEXW, WNDPROC, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

/// スクラップのウィンドウのクラス名。
const CLASS_NAME: PCWSTR = w!("shunsoku.scrap");

/// ウィンドウのクラスを登録する。`wndproc` は `app` のもの。
pub fn register_class(module: HINSTANCE, wndproc: WNDPROC) -> Result<()> {
    // SAFETY: 共有のカーソルなので壊さなくてよい
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: wndproc,
        hInstance: module,
        hCursor: cursor,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: class の文字列は 'static
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(())
}

/// スクラップのウィンドウ 1 枚。
pub struct ScrapWindow {
    hwnd: HWND,
}

impl ScrapWindow {
    /// 見えないままのウィンドウを `rect`（スクリーン座標）に作る。
    ///
    /// 作ったウィンドウにはメッセージが届くので、呼ぶ間はアプリの状態を借りないこと。
    pub fn create(module: HINSTANCE, rect: Rect) -> Result<ScrapWindow> {
        // SAFETY: 登録したクラスでウィンドウを作る
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("shunsoku"),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.width() as i32,
                rect.height() as i32,
                None,
                None,
                Some(module),
                None,
            )?
        };
        Ok(ScrapWindow { hwnd })
    }

    /// `source`（premultiplied BGRA の DIB を選んだ DC、大きさは `rect` と同じ）の中身を `rect` に出す。
    /// `opacity` は透明度（%）。
    pub fn present(&self, source: HDC, rect: Rect, opacity: u8) -> Result<()> {
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
        // SAFETY: 自分のウィンドウ。引数はこの関数の中で生きている
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
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

    /// 表示する。`focus` なら前面にしてキー入力を受けられるようにする。
    pub fn show(&self, focus: bool) {
        // SAFETY: 自分のウィンドウ
        unsafe {
            if focus {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
                let _ = SetForegroundWindow(self.hwnd);
            } else {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }
}

impl Drop for ScrapWindow {
    fn drop(&mut self) {
        // SAFETY: 自分で作ったウィンドウを、作ったスレッドで壊す
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
