//! 範囲選択の画面: 固定された画面を、モニタごとの最前面ウィンドウに flip-model のスワップチェーンで出す
//! （ADR-0002）。描画は D3D11 で行い、GDI は使わない。
//!
//! - ウィンドウはモニタ（`grab::Shot`）ごとに 1 枚。位置と大きさはモニタの `rect`（物理ピクセル）と同じ。
//!   範囲選択の状態はスクリーン座標で 1 つだけ持ち、各ウィンドウはそのうち自分のモニタの部分を描く。
//! - キー入力は前面にした 1 枚にしか来ない。どのウィンドウのイベントも、ウィンドウごとではなく
//!   画面全体の状態に渡すこと。
//! - ウィンドウ プロシージャは `app` が持つ（Win32 層のモジュールは `app` を通してつながる）。
//! - drop でスワップチェーンを解放してからウィンドウを壊す。そのあと `grab::Gpu::trim` を呼ぶ。

use windows::Win32::Foundation::{HINSTANCE, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct3D11::{D3D11_BOX, D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MWA_NO_ALT_ENTER, DXGI_MWA_NO_WINDOW_CHANGES, DXGI_PRESENT, DXGI_SCALING_NONE,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetCursorPos, IDC_CROSS, LoadCursorW, RegisterClassExW,
    SW_SHOW, SetCursor, SetForegroundWindow, ShowWindow, WNDCLASSEXW, WNDPROC,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Interface, PCWSTR, Result, w};

use crate::grab::Shot;

/// 範囲選択の画面のウィンドウのクラス名。
const CLASS_NAME: PCWSTR = w!("shunsoku.overlay");

/// ウィンドウのクラスを登録する。`wndproc` は `app` のもの。
pub fn register_class(module: HINSTANCE, wndproc: WNDPROC) -> Result<()> {
    // SAFETY: 共有のカーソルなので壊さなくてよい
    let cursor = unsafe { LoadCursorW(None, IDC_CROSS)? };
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: wndproc,
        hInstance: module,
        hCursor: cursor,
        lpszClassName: CLASS_NAME,
        // 背景は塗らない（GDI では描かない）
        ..Default::default()
    };
    // SAFETY: class の文字列は 'static
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(())
}

/// 範囲選択の画面。モニタごとのウィンドウの集まり。
pub struct Overlay {
    monitors: Vec<MonitorView>,
}

/// 1 モニタ分のウィンドウと、そこに出す固定された画面。
struct MonitorView {
    hwnd: HWND,
    shot: Shot,
    /// drop でウィンドウより先に解放する。
    surface: Option<Surface>,
}

struct Surface {
    swap_chain: IDXGISwapChain1,
    back_buffer: ID3D11Texture2D,
}

impl Overlay {
    /// モニタごとにウィンドウとスワップチェーンを作り、固定された画面を出す。全モニタで DWM に
    /// 反映される（`DwmFlush` が返る）まで待つ。
    ///
    /// 作ったウィンドウにはメッセージが届くので、呼ぶ間はアプリの状態を借りないこと。
    pub fn open(shots: Vec<Shot>, module: HINSTANCE) -> Result<Overlay> {
        let mut overlay = Overlay {
            monitors: Vec::with_capacity(shots.len()),
        };
        // 途中で失敗したら、overlay の drop で作った分を片付ける
        for shot in shots {
            let hwnd = create_window(shot.rect, module)?;
            let mut view = MonitorView {
                hwnd,
                shot,
                surface: None,
            };
            view.surface = Some(create_surface(&view)?);
            overlay.monitors.push(view);
        }

        // SAFETY: 作ったウィンドウを表示する
        unsafe {
            for view in &overlay.monitors {
                let _ = ShowWindow(view.hwnd, SW_SHOW);
            }
            // キー入力（Esc）を受けるため前面にする。トレイの操作やホットキーの直後は前面にできる
            let _ = SetForegroundWindow(overlay.focus_target());
        }
        overlay.redraw();
        // SAFETY: カーソルが動く前から十字にする。DWM への反映を待つ
        unsafe {
            if let Ok(cursor) = LoadCursorW(None, IDC_CROSS) {
                SetCursor(Some(cursor));
            }
            let _ = DwmFlush();
        }
        Ok(overlay)
    }

    /// 全モニタのウィンドウを描き直す。固定された画面を暗くせずにそのまま出す。
    pub fn redraw(&self) {
        for view in &self.monitors {
            if let Err(e) = view.draw() {
                log::warn!("could not draw the overlay: {e}");
            }
        }
    }

    /// 前面にするウィンドウ。カーソルのあるモニタのもの。
    fn focus_target(&self) -> HWND {
        let mut cursor = POINT::default();
        // SAFETY: cursor はこの関数の中で生きている
        let found = unsafe { GetCursorPos(&mut cursor) }.ok().and_then(|()| {
            self.monitors.iter().find(|m| {
                let r = m.shot.rect;
                (r.left..r.right).contains(&cursor.x) && (r.top..r.bottom).contains(&cursor.y)
            })
        });
        found.unwrap_or(&self.monitors[0]).hwnd
    }
}

impl MonitorView {
    fn draw(&self) -> Result<()> {
        let Some(surface) = &self.surface else {
            return Ok(());
        };
        let context = &self.shot.context;
        // SAFETY: どちらもこのモニタのデバイスのテクスチャ。UI スレッドで呼ぶ
        unsafe {
            match &self.shot.texture {
                Some(texture) => {
                    // 画像とバックバッファはどちらもモニタの大きさ。念のため小さいほうに合わせる
                    // （はみ出した箱を渡すと何も写らない）
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    texture.GetDesc(&mut desc);
                    let width = desc
                        .Width
                        .min((self.shot.rect.right - self.shot.rect.left) as u32);
                    let height = desc
                        .Height
                        .min((self.shot.rect.bottom - self.shot.rect.top) as u32);
                    let area = D3D11_BOX {
                        left: 0,
                        top: 0,
                        front: 0,
                        right: width,
                        bottom: height,
                        back: 1,
                    };
                    context.CopySubresourceRegion(
                        &surface.back_buffer,
                        0,
                        0,
                        0,
                        0,
                        texture,
                        0,
                        Some(&area),
                    );
                }
                None => {
                    // 出せないモニタは黒のまま（FLIP_DISCARD のバッファの中身は決まらないので塗る）
                    let mut view = None;
                    self.shot.device.CreateRenderTargetView(
                        &surface.back_buffer,
                        None,
                        Some(&mut view),
                    )?;
                    if let Some(view) = view {
                        context.ClearRenderTargetView(&view, &[0.0, 0.0, 0.0, 1.0]);
                    }
                }
            }
            surface.swap_chain.Present(0, DXGI_PRESENT(0)).ok()
        }
    }
}

impl Drop for MonitorView {
    fn drop(&mut self) {
        // バックバッファとスワップチェーンを、ウィンドウより先に解放する
        drop(self.surface.take());
        // SAFETY: 自分で作ったウィンドウを、作ったスレッドで壊す
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn create_window(rect: RECT, module: HINSTANCE) -> Result<HWND> {
    // flip-model のスワップチェーンは WS_EX_LAYERED と組めない。DXGI でだけ描くので
    // GDI のリダイレクトのサーフェスは要らない
    // SAFETY: 登録したクラスでウィンドウを作る。モニタの上にちょうど作るので WM_DPICHANGED は来ない
    unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
            CLASS_NAME,
            w!("shunsoku"),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(module),
            None,
        )
    }
}

fn create_surface(view: &MonitorView) -> Result<Surface> {
    let rect = view.shot.rect;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: (rect.right - rect.left) as u32,
        Height: (rect.bottom - rect.top) as u32,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        // 画素を等倍で出す（引き伸ばすとぼやける）
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        ..Default::default()
    };
    // SAFETY: このモニタのアダプタのデバイスで、このモニタのウィンドウに作る
    unsafe {
        let factory: IDXGIFactory2 = view
            .shot
            .device
            .cast::<IDXGIDevice>()?
            .GetAdapter()?
            .GetParent()?;
        let swap_chain =
            factory.CreateSwapChainForHwnd(&view.shot.device, view.hwnd, &desc, None, None)?;
        // Alt+Enter で全画面に切り替えさせない
        let _ = factory.MakeWindowAssociation(
            view.hwnd,
            DXGI_MWA_NO_ALT_ENTER | DXGI_MWA_NO_WINDOW_CHANGES,
        );
        let back_buffer = swap_chain.GetBuffer(0)?;
        Ok(Surface {
            swap_chain,
            back_buffer,
        })
    }
}
