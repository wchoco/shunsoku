//! 書き込みの編集モードのツールバーのウィンドウ: スクラップの直下に出す、小さな最前面のレイヤード ウィンドウ。
//!
//! - スクラップに持たれる（owned）ウィンドウにする。いつもスクラップより手前に出て、スクラップより先に壊れる。
//! - `WS_EX_NOACTIVATE` で、押してもスクラップからフォーカスを奪わない（キー入力はスクラップに残り、スクラップの
//!   `WM_ACTIVATE` の `WA_INACTIVE` も来ない）。プロシージャ（`app`）は `WM_MOUSEACTIVATE` にも `MA_NOACTIVATE` を返す。
//! - 並びと当たり判定は core の `annotate::toolbar::Layout`、中身は `render::Renderer::paint_toolbar` で描いて
//!   `UpdateLayeredWindow` で出す（スクラップと同じ）。背景は不透明に塗る（alpha 0 の画素はクリックが下へ抜ける）。
//! - 大きさはスクラップのモニタの DPI で決める。ツールバー自身に来る `WM_DPICHANGED` の提案の矩形は当てない。
//! - drop でウィンドウを壊す。

use shunsoku_core::geometry::Rect;
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, IDC_ARROW, IsWindow, LoadCursorW, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow,
    WNDCLASSEXW, WNDPROC, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use crate::layered;

/// ツールバーのウィンドウのクラス名。
const CLASS_NAME: PCWSTR = w!("shunsoku.toolbar");

/// ウィンドウのクラスを登録する。`wndproc` は `app` のもの。ボタンを速く続けて押せるよう、ダブルクリックは
/// 受けない（`CS_DBLCLKS` を付けない）。
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

/// ツールバーのウィンドウ 1 枚。
pub struct ToolbarWindow {
    hwnd: HWND,
}

impl ToolbarWindow {
    /// スクラップ `owner` に持たれる、見えないままのウィンドウを作る。
    ///
    /// 作ったウィンドウにはメッセージが届くが、`app` のプロシージャは作る間のメッセージでは App を借りない。
    pub fn create(module: HINSTANCE, owner: HWND) -> Result<ToolbarWindow> {
        // SAFETY: 登録したクラスでウィンドウを作る
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("shunsoku"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                Some(owner),
                None,
                Some(module),
                None,
            )?
        };
        Ok(ToolbarWindow { hwnd })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// `source` の中身を `rect`（スクリーン座標）に出す。いつも不透明。
    pub fn present(&self, source: HDC, rect: Rect) -> Result<()> {
        layered::present(self.hwnd, source, rect, None, 0xFF)
    }

    /// 大きさを変えずに、左上を `rect` の左上に動かす（中身は描き直さない）。
    pub fn move_to(&self, rect: Rect) -> Result<()> {
        // SAFETY: 自分のウィンドウ
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                rect.left,
                rect.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
    }

    /// フォーカスを移さずに出す。
    pub fn show(&self) {
        // SAFETY: 自分のウィンドウ
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
    }

    /// 隠す（壊さない）。
    pub fn hide(&self) {
        // SAFETY: 自分のウィンドウ
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }
}

impl Drop for ToolbarWindow {
    fn drop(&mut self) {
        // 持ち主（スクラップ）が先に壊れていれば、Windows がもう壊している
        // SAFETY: 自分で作ったウィンドウを、作ったスレッドで壊す
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
