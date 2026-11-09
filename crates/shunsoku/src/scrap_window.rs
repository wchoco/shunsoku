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
//! - 描き先の DIB（`canvas`）はウィンドウが持ち、描き直しのたびに使い回す（大きさが変わったときだけ作り直す）。
//! - drop でウィンドウを壊す。描き先も一緒に返す。
//! - 書き込みの編集モードで消しゴムを選んでいる間のカーソル（`set_eraser_cursor`）もここで作る。

use std::cell::{Cell, RefCell};

use shunsoku_core::geometry::Rect;
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND};
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DestroyWindow, IDC_ARROW, IsWindowVisible, LoadCursorW,
    RegisterClassExW, RemovePropW, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SetForegroundWindow, SetPropW, SetWindowPos, ShowWindow, WNDCLASSEXW, WNDPROC,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateCursor, GetSystemMetrics, HCURSOR, SM_CXCURSOR, SM_CYCURSOR, SetCursor,
};
use windows::core::{PCWSTR, Result, w};

use crate::layered;
use crate::render::Canvas;

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

thread_local! {
    /// 消しゴムのカーソル。UI スレッドで最初に使うときに作り、終わるまで持つ（壊さない）。作れなければ None。
    static ERASER_CURSOR: Option<HCURSOR> = eraser_cursor();
}

/// カーソルを消しゴムの形（中の空いた正方形）にする。作れなかったら矢印のまま。
pub fn set_eraser_cursor() {
    if let Some(cursor) = ERASER_CURSOR.with(|c| *c) {
        // SAFETY: UI スレッドで呼ぶ
        unsafe { SetCursor(Some(cursor)) };
    }
}

/// 標準の大きさで、黒い縁の内側に白い縁を重ねた正方形のカーソルを作る（どの色の上でも見えるように）。
/// 真ん中の 1 画素は下の色を反転して、ホットスポットを示す。
fn eraser_cursor() -> Option<HCURSOR> {
    // SAFETY: 引数のない問い合わせ
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXCURSOR), GetSystemMetrics(SM_CYCURSOR)) };
    let (width, height) = (width.max(16) as usize, height.max(16) as usize);
    // 1 行は WORD の境まで詰める
    let stride = width.div_ceil(16) * 2;
    // AND が 1・XOR が 0 で透明、0・0 で黒、0・1 で白、1・1 で反転
    let mut and = vec![0xFF_u8; stride * height];
    let mut xor = vec![0_u8; stride * height];
    let mut put = |x: usize, y: usize, and_bit: bool, xor_bit: bool| {
        let (byte, bit) = (y * stride + x / 8, 0x80 >> (x % 8));
        and[byte] = if and_bit {
            and[byte] | bit
        } else {
            and[byte] & !bit
        };
        xor[byte] = if xor_bit {
            xor[byte] | bit
        } else {
            xor[byte] & !bit
        };
    };
    let (cx, cy) = (width / 2, height / 2);
    let half = width.min(height) / 4;
    for y in cy - half..=cy + half {
        for x in cx - half..=cx + half {
            let ring = (x.abs_diff(cx)).max(y.abs_diff(cy));
            if ring == half {
                put(x, y, false, false);
            } else if ring == half - 1 {
                put(x, y, false, true);
            }
        }
    }
    put(cx, cy, true, true);
    // SAFETY: 2 つの面は width x height の 1 ビットの画素で、呼ぶ間は生きている
    let cursor = unsafe {
        CreateCursor(
            None,
            cx as i32,
            cy as i32,
            width as i32,
            height as i32,
            and.as_ptr().cast(),
            xor.as_ptr().cast(),
        )
    };
    match cursor {
        Ok(cursor) => Some(cursor),
        Err(e) => {
            log::warn!("could not create the eraser cursor: {e}");
            None
        }
    }
}

/// スクラップのウィンドウ 1 枚。
pub struct ScrapWindow {
    hwnd: HWND,
    /// 描き先。まだ描いていなければ None。
    canvas: RefCell<Option<Canvas>>,
    /// 出した回数（`present`）。
    presents: Cell<u64>,
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
        Ok(ScrapWindow {
            hwnd,
            canvas: RefCell::new(None),
            presents: Cell::new(0),
        })
    }

    /// `source`（premultiplied BGRA の DIB を選んだ DC、大きさは `rect` と同じ）の中身を `rect` に出す。
    /// `dirty`（`source` の中の座標）があればその範囲だけを写す（前に出したときと大きさが同じときだけ渡す）。
    /// `constant_alpha` は全体のアルファ（`Appearance::constant_alpha`）。
    pub fn present(
        &self,
        source: HDC,
        rect: Rect,
        dirty: Option<Rect>,
        constant_alpha: u8,
    ) -> Result<()> {
        self.presents.set(self.presents.get().wrapping_add(1));
        layered::present(self.hwnd, source, rect, dirty, constant_alpha)
    }

    /// 出した回数（`present` を呼んだ回数）。出している間にほかの描き直しが出したかを見分ける。
    pub fn presents(&self) -> u64 {
        self.presents.get()
    }

    /// 描き直しで使い回す描き先（`Canvas::reuse` に渡す）。`UpdateLayeredWindow` は中身を写し取るので、出し終えた
    /// 描き先は次の描き直しで使ってよい。
    pub fn canvas(&self) -> &RefCell<Option<Canvas>> {
        &self.canvas
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
