//! 範囲選択の画面: 固定された画面を、モニタごとの最前面ウィンドウに flip-model のスワップチェーンで出す
//! （ADR-0002）。描画は D3D11 と、その上で動く Direct2D で行い、GDI は使わない。
//!
//! - ウィンドウはモニタ（`grab::Shot`）ごとに 1 枚。位置と大きさはモニタの `rect`（物理ピクセル）と同じ。
//!   範囲選択の状態はスクリーン座標で 1 つだけ持ち、各ウィンドウはそのうち自分のモニタの部分を描く。
//! - キー入力は前面にした 1 枚にしか来ない。どのウィンドウのイベントも、ウィンドウごとではなく
//!   画面全体の状態に渡すこと。
//! - ウィンドウ プロシージャは `app` が持つ（Win32 層のモジュールは `app` を通してつながる）。
//! - 範囲を選んでいる間は、固定された画面をバックバッファに写したうえに Direct2D で範囲の外を暗くし、
//!   範囲の左上の外側にサイズを出す。Direct2D の資源は初めて範囲を描くときに作る（開くまでの時間を
//!   延ばさないため）。Direct2D の座標は 1 DIP = 1 物理ピクセル。
//! - drop で Direct2D の描画先、スワップチェーンの順に解放してからウィンドウを壊す。そのあと
//!   `grab::Gpu::trim` を呼ぶ。

use shunsoku_core::geometry::{Point, Rect};
use shunsoku_core::image::Fragment;
use shunsoku_core::selection::{MonitorInfo, Part, Selection};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1CreateFactory, ID2D1Bitmap1, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_TEXTURE2D_DESC, ID3D11Device, ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MWA_NO_ALT_ENTER, DXGI_MWA_NO_WINDOW_CHANGES, DXGI_PRESENT, DXGI_SCALING_NONE,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetCursorPos, IDC_CROSS, LoadCursorW, RegisterClassExW,
    SW_SHOW, SetCursor, SetForegroundWindow, ShowWindow, WNDCLASSEXW, WNDPROC,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Interface, PCWSTR, Result, w};

use crate::grab::Shot;

/// 範囲選択の画面のウィンドウのクラス名。
const CLASS_NAME: PCWSTR = w!("shunsoku.overlay");

/// 範囲の外を暗くする色（黒の半透明）。
const DIM: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.5,
};
/// サイズの表示の背景の色。
const LABEL_BACK: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.75,
};
/// サイズの表示の文字の色。
const LABEL_TEXT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
/// サイズの表示の文字の大きさ・余白・範囲との間（DPI 100% のときの物理ピクセル。DPI に合わせて大きくする）。
const LABEL_FONT_SIZE: f32 = 12.0;
const LABEL_PADDING_X: f32 = 6.0;
const LABEL_PADDING_Y: f32 = 3.0;
const LABEL_GAP: f32 = 4.0;

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
    /// 範囲を描くための資源。初めて範囲を描くときに作る。
    painter: Option<Painter>,
}

/// 1 モニタ分のウィンドウと、そこに出す固定された画面。
struct MonitorView {
    hwnd: HWND,
    shot: Shot,
    /// drop でウィンドウより先に解放する。
    surface: Option<Surface>,
}

/// フィールドの順に解放される。Direct2D の描画先（バックバッファを参照する）をスワップチェーンより先に。
struct Surface {
    d2d: Option<D2dTarget>,
    back_buffer: ID3D11Texture2D,
    swap_chain: IDXGISwapChain1,
}

/// バックバッファに描く Direct2D のデバイス コンテキストとブラシ。
struct D2dTarget {
    dim: ID2D1SolidColorBrush,
    label_back: ID2D1SolidColorBrush,
    label_text: ID2D1SolidColorBrush,
    _target: ID2D1Bitmap1,
    context: ID2D1DeviceContext,
}

/// Direct2D と DirectWrite のファクトリ。
struct Painter {
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    /// サイズの表示の書式と、それを作ったときの DPI。
    format: Option<(u32, IDWriteTextFormat)>,
}

/// サイズの表示。座標はスクリーン座標。
struct Label {
    text: Vec<u16>,
    format: IDWriteTextFormat,
    back: D2D_RECT_F,
    text_rect: D2D_RECT_F,
}

impl Overlay {
    /// モニタごとにウィンドウとスワップチェーンを作り、固定された画面を出す。全モニタで DWM に
    /// 反映される（`DwmFlush` が返る）まで待つ。
    ///
    /// 作ったウィンドウにはメッセージが届くので、呼ぶ間はアプリの状態を借りないこと。
    pub fn open(shots: Vec<Shot>, module: HINSTANCE) -> Result<Overlay> {
        let mut overlay = Overlay {
            monitors: Vec::with_capacity(shots.len()),
            painter: None,
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
        // まだ範囲はないので Direct2D は作らない
        for view in &mut overlay.monitors {
            if let Err(e) = view.draw(None, None, None) {
                log::warn!("could not draw the overlay: {e}");
            }
        }
        // SAFETY: カーソルが動く前から十字にする。DWM への反映を待つ
        unsafe {
            if let Ok(cursor) = LoadCursorW(None, IDC_CROSS) {
                SetCursor(Some(cursor));
            }
            let _ = DwmFlush();
        }
        Ok(overlay)
    }

    /// 開いた時点の範囲選択の状態。モニタの番号は `read_back` に渡す `Part::monitor` と同じ。
    pub fn new_selection(&self) -> Selection {
        let monitors = self
            .monitors
            .iter()
            .map(|view| MonitorInfo {
                rect: to_rect(view.shot.rect),
                dpi: view.shot.dpi,
            })
            .collect();
        let mut cursor = POINT::default();
        // SAFETY: cursor はこの関数の中で生きている。取れなければ原点のまま
        let _ = unsafe { GetCursorPos(&mut cursor) };
        Selection::new(monitors, Point::new(cursor.x, cursor.y))
    }

    /// 全モニタのウィンドウを描き直す。範囲がなければ固定された画面をそのまま出し、
    /// 範囲があればその外を暗くしてサイズを出す。
    pub fn redraw(&mut self, selection: &Selection) {
        let rect = selection.rect();
        let label = match rect {
            Some(rect) => self.label(selection, rect).unwrap_or_else(|e| {
                log::warn!("could not lay out the size label: {e}");
                None
            }),
            None => None,
        };
        let factory = match rect {
            Some(_) => match self.painter() {
                Ok(painter) => Some(painter.factory.clone()),
                Err(e) => {
                    log::warn!("could not create Direct2D: {e}");
                    None
                }
            },
            None => None,
        };
        for view in &mut self.monitors {
            if let Err(e) = view.draw(factory.as_ref(), rect, label.as_ref()) {
                log::warn!("could not draw the overlay: {e}");
            }
        }
    }

    /// 範囲のうち 1 枚のモニタに重なる部分を CPU に書き戻す。出せないモニタなら None。
    pub fn read_back(&self, part: &Part) -> Result<Option<Fragment>> {
        match self.monitors.get(part.monitor) {
            Some(view) => view.shot.read_back(part.rect),
            None => Ok(None),
        }
    }

    /// サイズの表示を、範囲の左上があるモニタの DPI の大きさで、範囲の左上の外側に置く。
    /// そのモニタの上にはみ出すなら、範囲の内側の左上に置く。
    fn label(&mut self, selection: &Selection, rect: Rect) -> Result<Option<Label>> {
        let Some(text) = selection.size_label() else {
            return Ok(None);
        };
        let monitor = selection
            .monitors()
            .iter()
            .find(|m| m.rect.contains(rect.origin()))
            .copied();
        let dpi = monitor.map_or(96, |m| m.dpi);
        let scale = dpi as f32 / 96.0;
        let painter = self.painter()?;
        let format = painter.format(dpi)?;
        let text: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: 文字列と書式はこの関数の中で生きている
        let metrics = unsafe {
            let layout = painter
                .dwrite
                .CreateTextLayout(&text, &format, 4096.0, 1024.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            metrics
        };
        let pad_x = LABEL_PADDING_X * scale;
        let pad_y = LABEL_PADDING_Y * scale;
        let gap = LABEL_GAP * scale;
        let width = metrics.width.ceil() + pad_x * 2.0;
        let height = metrics.height.ceil() + pad_y * 2.0;
        let left = rect.left as f32;
        let mut top = rect.top as f32 - gap - height;
        if monitor.is_some_and(|m| top < m.rect.top as f32) {
            top = rect.top as f32 + gap;
        }
        let back = D2D_RECT_F {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        let text_rect = D2D_RECT_F {
            left: back.left + pad_x,
            top: back.top + pad_y,
            right: back.right - pad_x,
            bottom: back.bottom - pad_y,
        };
        Ok(Some(Label {
            text,
            format,
            back,
            text_rect,
        }))
    }

    fn painter(&mut self) -> Result<&mut Painter> {
        if self.painter.is_none() {
            // SAFETY: 引数は定数
            let (factory, dwrite) = unsafe {
                (
                    D2D1CreateFactory::<ID2D1Factory1>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                    DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)?,
                )
            };
            self.painter = Some(Painter {
                factory,
                dwrite,
                format: None,
            });
        }
        Ok(self.painter.as_mut().expect("created above"))
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

impl Painter {
    /// `dpi` のモニタ用のサイズの表示の書式。DPI が変わらなければ作り直さない。
    fn format(&mut self, dpi: u32) -> Result<IDWriteTextFormat> {
        if let Some((cached, format)) = &self.format
            && *cached == dpi
        {
            return Ok(format.clone());
        }
        let size = LABEL_FONT_SIZE * dpi as f32 / 96.0;
        // SAFETY: 文字列は 'static
        let format = unsafe {
            let format = self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ja-jp"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format
        };
        self.format = Some((dpi, format.clone()));
        Ok(format)
    }
}

impl MonitorView {
    /// 固定された画面を写し、範囲（`rect`）があれば範囲の外を暗くしてサイズを出す。
    /// `factory` がなければ範囲は描かない。
    fn draw(
        &mut self,
        factory: Option<&ID2D1Factory1>,
        rect: Option<Rect>,
        label: Option<&Label>,
    ) -> Result<()> {
        self.copy_shot()?;
        if let (Some(factory), Some(rect)) = (factory, rect)
            && let Err(e) = self.draw_selection(factory, rect, label)
        {
            // 描画先を作り直せば直ることがある（D2DERR_RECREATE_TARGET など）。次に描くときに作り直す
            log::warn!("could not draw the selection: {e}");
            if let Some(surface) = &mut self.surface {
                surface.d2d = None;
            }
        }
        match &self.surface {
            // SAFETY: このモニタのスワップチェーン
            Some(surface) => unsafe { surface.swap_chain.Present(0, DXGI_PRESENT(0)).ok() },
            None => Ok(()),
        }
    }

    /// 範囲の外を暗くし、サイズを出す。スクリーン座標からこのウィンドウの座標に直して描く。
    fn draw_selection(
        &mut self,
        factory: &ID2D1Factory1,
        rect: Rect,
        label: Option<&Label>,
    ) -> Result<()> {
        let monitor = to_rect(self.shot.rect);
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };
        if surface.d2d.is_none() {
            surface.d2d = Some(D2dTarget::new(
                factory,
                &self.shot.device,
                &surface.swap_chain,
            )?);
        }
        let Some(d2d) = &surface.d2d else {
            return Ok(());
        };
        let (ox, oy) = (monitor.left as f32, monitor.top as f32);
        let local = |r: &D2D_RECT_F| D2D_RECT_F {
            left: r.left - ox,
            top: r.top - oy,
            right: r.right - ox,
            bottom: r.bottom - oy,
        };
        // 範囲の外 = 範囲の上・下・左・右の 4 つの帯。このモニタに重なる部分だけ塗る
        let outside = [
            Rect::new(monitor.left, monitor.top, monitor.right, rect.top),
            Rect::new(monitor.left, rect.bottom, monitor.right, monitor.bottom),
            Rect::new(monitor.left, rect.top, rect.left, rect.bottom),
            Rect::new(rect.right, rect.top, monitor.right, rect.bottom),
        ];
        let context = &d2d.context;
        // SAFETY: このモニタのバックバッファに描く。UI スレッドで呼ぶ
        unsafe {
            context.BeginDraw();
            context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            for band in outside.iter().filter_map(|b| b.intersect(&monitor)) {
                context.FillRectangle(&local(&to_rect_f(band)), &d2d.dim);
            }
            if let Some(label) = label {
                context.FillRectangle(&local(&label.back), &d2d.label_back);
                context.DrawText(
                    &label.text,
                    &label.format,
                    &local(&label.text_rect),
                    &d2d.label_text,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            context.EndDraw(None, None)
        }
    }

    /// 固定された画面をバックバッファに写す。
    fn copy_shot(&self) -> Result<()> {
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
        }
        Ok(())
    }
}

impl Drop for MonitorView {
    fn drop(&mut self) {
        // Direct2D の描画先、バックバッファとスワップチェーンを、ウィンドウより先に解放する
        drop(self.surface.take());
        // SAFETY: 自分で作ったウィンドウを、作ったスレッドで壊す
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl D2dTarget {
    /// スワップチェーンのバックバッファに描く Direct2D のデバイス コンテキストを作る。1 DIP = 1 物理ピクセル。
    fn new(
        factory: &ID2D1Factory1,
        device: &ID3D11Device,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<D2dTarget> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        // SAFETY: このモニタのアダプタのデバイス（BGRA_SUPPORT 付きで作った）と、そのスワップチェーン
        unsafe {
            let d2d_device = factory.CreateDevice(&device.cast::<IDXGIDevice>()?)?;
            let context = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let surface: IDXGISurface = swap_chain.GetBuffer(0)?;
            let target = context.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            context.SetTarget(&target);
            context.SetDpi(96.0, 96.0);
            Ok(D2dTarget {
                dim: context.CreateSolidColorBrush(&DIM, None)?,
                label_back: context.CreateSolidColorBrush(&LABEL_BACK, None)?,
                label_text: context.CreateSolidColorBrush(&LABEL_TEXT, None)?,
                _target: target,
                context,
            })
        }
    }
}

/// マウスのメッセージの座標（クライアント座標、符号付き 16 bit）をスクリーン座標にする。
/// 左や上のモニタでは負になる。
pub fn screen_point(hwnd: HWND, lparam: LPARAM) -> Point {
    let mut point = POINT {
        x: i32::from(lparam.0 as u16 as i16),
        y: i32::from((lparam.0 >> 16) as u16 as i16),
    };
    // SAFETY: point はこの関数の中で生きている
    let _ = unsafe { ClientToScreen(hwnd, &mut point) };
    Point::new(point.x, point.y)
}

fn to_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

fn to_rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left as f32,
        top: r.top as f32,
        right: r.right as f32,
        bottom: r.bottom as f32,
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
            d2d: None,
            back_buffer,
            swap_chain,
        })
    }
}
