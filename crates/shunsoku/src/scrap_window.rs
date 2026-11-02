//! スクラップのウィンドウ: 最前面のレイヤード ウィンドウ。中身は `UpdateLayeredWindow` で
//! premultiplied BGRA の DIB（`render::Canvas` の DC）から出す。
//!
//! - 位置と大きさはスクリーン座標の物理ピクセル。大きさは core の `ScrapModel::window_rect`（倍率を掛けた
//!   表示の大きさ + 余白）で決まる。`WM_DPICHANGED` で提案された矩形は当てない（ウィンドウ プロシージャは
//!   `app` が持ち、新しい DPI を core の `ScrapModel::set_dpi` に渡すだけにする）。
//! - 透明度は `SourceConstantAlpha` で掛ける。
//! - シェルの全画面の判定から外す（`NonRudeHWND`）。モニタ全体を覆うスクラップを最前面に出すと、
//!   シェルが全画面のウィンドウとみなしてタスクバーを後ろに回し、動かすたびにモニタを覆う・覆わないが
//!   切り替わってタスクバーがちらつくため。タスクバーは常にスクラップの上に出る。
//! - drop でウィンドウを壊す。

use shunsoku_core::geometry::Rect;
use windows::Win32::Foundation::{COLORREF, HANDLE, HINSTANCE, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, HDC};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DestroyWindow, IDC_ARROW, IsWindowVisible, LoadCursorW,
    RegisterClassExW, RemovePropW, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SetForegroundWindow, SetPropW, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WNDCLASSEXW, WNDPROC, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

/// スクラップのウィンドウのクラス名。
const CLASS_NAME: PCWSTR = w!("shunsoku.scrap");

/// シェルの全画面の判定から外すウィンドウのプロパティ。値は何でもよく、あることだけが見られる。
/// <https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist2-markfullscreenwindow>
const NON_RUDE: PCWSTR = w!("NonRudeHWND");

/// ウィンドウのクラスを登録する。`wndproc` は `app` のもの。ダブルクリック（コンパクト表示の切り替え）を
/// 受けるため `CS_DBLCLKS` を付ける。
pub fn register_class(module: HINSTANCE, wndproc: WNDPROC) -> Result<()> {
    // SAFETY: 共有のカーソルなので壊さなくてよい
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW)? };
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
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
        // 表示する前に付ける（シェルは表示したときに判定する）。付けられなくてもスクラップは使えるので、ログだけ
        // SAFETY: 自分のウィンドウ。プロパティ名は 'static
        if let Err(e) = unsafe { SetPropW(hwnd, NON_RUDE, Some(HANDLE(1 as _))) } {
            log::warn!("could not opt the scrap out of full-screen detection: {e}");
        }
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

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// ウィンドウのあるモニタの DPI。取れなければ 96。
    pub fn dpi(&self) -> u32 {
        // SAFETY: 自分のウィンドウ
        match unsafe { GetDpiForWindow(self.hwnd) } {
            0 => 96,
            dpi => dpi,
        }
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

    /// 隠す（壊さない。`show` で出し直せる）。
    pub fn hide(&self) {
        // SAFETY: 自分のウィンドウ
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// 出ているか（`hide` で隠していないか）。
    pub fn is_visible(&self) -> bool {
        // SAFETY: 自分のウィンドウ
        unsafe { IsWindowVisible(self.hwnd) }.as_bool()
    }
}

impl Drop for ScrapWindow {
    fn drop(&mut self) {
        // SAFETY: 自分で作ったウィンドウを、作ったスレッドで壊す
        unsafe {
            // ウィンドウのプロパティは壊す前に外す決まり
            let _ = RemovePropW(self.hwnd, NON_RUDE);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
