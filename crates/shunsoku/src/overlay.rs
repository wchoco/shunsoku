//! 範囲選択の画面: 固定された画面を、モニタごとの最前面ウィンドウに flip-model のスワップチェーンで出す
//! （ADR-0002）。描画は D3D11 と、その上で動く Direct2D で行い、GDI は使わない。
//!
//! - ウィンドウはモニタ（`grab::Shot`）ごとに 1 枚。位置と大きさはモニタの `rect`（物理ピクセル）と同じ。
//!   範囲選択の状態はスクリーン座標で 1 つだけ持ち、各ウィンドウはそのうち自分のモニタの部分を描く。
//! - キー入力は前面にした 1 枚にしか来ない。どのウィンドウのイベントも、ウィンドウごとではなく
//!   画面全体の状態に渡すこと。
//! - ウィンドウ プロシージャは `app` が持つ（Win32 層のモジュールは `app` を通してつながる）。
//! - 画面は暗くしない。範囲を選んでいる間は、固定された画面をバックバッファに写したうえに Direct2D で
//!   範囲のすぐ外側に細い枠（内側が白、外側が黒の 1 物理ピクセルずつ。どんな背景でも見えるように）を描き、
//!   範囲の左上の外側にサイズを出す。ウィンドウをハイライトしている間は、範囲の代わりにその矩形を
//!   半透明の青で塗って内側に枠を描き、名前とサイズを出す。Direct2D の資源は初めて範囲を描くときに
//!   作る（開くまでの時間を延ばさないため）。Direct2D の座標は 1 DIP = 1 物理ピクセル。
//! - 描くもの（範囲、ハイライト、つまみ、十字線、カーソルの横の座標、拡大鏡）は core の `Selection::view` が決める。
//!   十字線はカーソルのあるモニタにだけ、そのモニタいっぱいに引く。
//! - 拡大鏡（精密モード）は、カーソルのまわりの画素を固定された画面のテクスチャから CPU に読み、拡大した
//!   画素を 1 つずつ塗る。その下にカーソルの横の座標と色コードを付けて 1 まとまりにし、置き場所は
//!   core の `magnifier::place` で決める（カーソルのあるモニタの中で、端では反対側に回る）。
//! - drop で Direct2D の描画先、スワップチェーンの順に解放してからウィンドウを壊す。そのあと
//!   `grab::Gpu::trim` を呼ぶ。

use shunsoku_core::geometry::{Point, Rect};
use shunsoku_core::image::Fragment;
use shunsoku_core::magnifier;
use shunsoku_core::selection::{CursorShape, MonitorInfo, Part, Selection};
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
    CS_DBLCLKS, CreateWindowExW, DestroyWindow, GetCursorPos, IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW,
    IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, LoadCursorW, RegisterClassExW, SW_SHOW, SetCursor,
    SetForegroundWindow, ShowWindow, WNDCLASSEXW, WNDPROC, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{Interface, PCWSTR, Result, w};

use crate::grab::Shot;

/// 範囲選択の画面のウィンドウのクラス名。
const CLASS_NAME: PCWSTR = w!("shunsoku.overlay");

/// 範囲の枠の内側の線の色（白）。
const FRAME_INNER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
/// 範囲の枠の外側の線の色（黒）。
const FRAME_OUTER: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
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
/// ハイライトの枠の色（Windows の既定のアクセントに近い青）。
const HIGHLIGHT_FRAME: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.47,
    b: 0.84,
    a: 1.0,
};
/// ハイライトの中に重ねる色。
const HIGHLIGHT_FILL: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.47,
    b: 0.84,
    a: 0.15,
};
/// ハイライトの枠の太さ（DPI 100% のときの物理ピクセル。DPI に合わせて太くする）。
const HIGHLIGHT_WIDTH: f32 = 3.0;
/// サイズの表示の文字の大きさ・余白・範囲との間（DPI 100% のときの物理ピクセル。DPI に合わせて大きくする）。
const LABEL_FONT_SIZE: f32 = 12.0;
const LABEL_PADDING_X: f32 = 6.0;
const LABEL_PADDING_Y: f32 = 3.0;
const LABEL_GAP: f32 = 4.0;
/// カーソルの横の座標の表示を、カーソルの画素から右下へずらす量（DPI 100% のときの物理ピクセル）。
/// 十字のカーソルに重ならないように。
const CURSOR_LABEL_OFFSET: f32 = 20.0;
/// 画面の上端に出す文字（定型領域の登録）の、モニタの上端からの距離（96 DPI での物理ピクセル）。
const BANNER_TOP: f32 = 8.0;
/// 拡大鏡で、モニタの外の画素を塗る色。
const MAGNIFIER_OUTSIDE: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.25,
    g: 0.25,
    b: 0.25,
    a: 1.0,
};
/// 十字線の色。どんな背景でも見えるよう、半透明のマゼンタにする。
const CROSSHAIR: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 0.7,
};

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
        // 精密モードの調整の状態でダブルクリックを受ける
        style: CS_DBLCLKS,
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
    /// 各モニタの上端の中央に出す文字（`set_banner`）。
    banner: Option<String>,
    /// モニタをまたいだ範囲のサイズの、モニタごとのサイズのつなぎ（文言の `selection.size_plus`）。
    size_plus: String,
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
    frame_inner: ID2D1SolidColorBrush,
    frame_outer: ID2D1SolidColorBrush,
    label_back: ID2D1SolidColorBrush,
    label_text: ID2D1SolidColorBrush,
    highlight_frame: ID2D1SolidColorBrush,
    highlight_fill: ID2D1SolidColorBrush,
    crosshair: ID2D1SolidColorBrush,
    /// 拡大鏡の画素を塗る（色は塗るたびに変える）。
    pixel: ID2D1SolidColorBrush,
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

/// 1 回に描くもの。座標はスクリーン座標。
#[derive(Default)]
struct Scene {
    rect: Option<Rect>,
    size_label: Option<Label>,
    handles: Vec<Rect>,
    crosshair: Option<Point>,
    cursor_label: Option<Label>,
    /// ハイライトしているウィンドウの矩形。名前とサイズは `size_label` に入れる。
    highlight: Option<Rect>,
    /// 各モニタの上端に出す文字（`Overlay::set_banner`）。
    banners: Vec<Label>,
    magnifier: Option<Magnifier>,
}

impl Scene {
    fn is_empty(&self) -> bool {
        self.banners.is_empty()
            && self.rect.is_none()
            && self.crosshair.is_none()
            && self.cursor_label.is_none()
            && self.highlight.is_none()
            && self.magnifier.is_none()
    }
}

/// 拡大鏡と、その下に付ける表示。座標はスクリーン座標。
struct Magnifier {
    /// 拡大した画素と、その色（モニタの外は None）。
    cells: Vec<(Rect, Option<D2D1_COLOR_F>)>,
    /// カーソルの画素を拡大したもの（枠で囲む）。
    center: Rect,
    /// 拡大した画素の並び全体。
    area: Rect,
    /// 下に付ける座標と色コード。
    labels: Vec<Label>,
}

/// サイズや座標の表示。座標はスクリーン座標。
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
    /// `size_plus` はモニタをまたいだ範囲のサイズのつなぎ（文言の `selection.size_plus`）。
    pub fn open(shots: Vec<Shot>, module: HINSTANCE, size_plus: &str) -> Result<Overlay> {
        let mut overlay = Overlay {
            monitors: Vec::with_capacity(shots.len()),
            painter: None,
            banner: None,
            size_plus: size_plus.to_owned(),
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
            if let Err(e) = view.draw(None, &Scene::default()) {
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

    /// 各モニタの上端の中央に出す文字（定型領域の登録）。次の `redraw` から出す。
    pub fn set_banner(&mut self, text: Option<&str>) {
        self.banner = text.map(str::to_owned);
    }

    /// 全モニタのウィンドウを描き直す。描くもの（`Selection::view`）がなければ固定された画面を
    /// そのまま出し、あれば範囲の枠とサイズ、つまみ、十字線、カーソルの横の座標を出す。
    pub fn redraw(&mut self, selection: &Selection) {
        let view = selection.view();
        // ハイライトしていれば、範囲のサイズの代わりにウィンドウの名前とサイズを出す
        let text = match (&view.highlight, view.rect, &view.size_label) {
            (Some(highlight), _, _) => Some((
                highlight_label_at(selection, highlight.rect),
                highlight.label(),
            )),
            (None, Some(rect), Some(sizes)) => Some((rect, sizes.join(&self.size_plus))),
            _ => None,
        };
        let size_label = match text {
            Some((at, text)) => self.label(selection, at, &text).unwrap_or_else(|e| {
                log::warn!("could not lay out the size label: {e}");
                None
            }),
            None => None,
        };
        let magnifier = match view.magnifier {
            Some(at) => {
                let text = view.cursor_label.as_ref().map(|(_, text)| text.as_str());
                self.magnifier(selection, at, text).unwrap_or_else(|e| {
                    log::warn!("could not lay out the magnifier: {e}");
                    None
                })
            }
            None => None,
        };
        let banners = match self.banners() {
            Ok(banners) => banners,
            Err(e) => {
                log::warn!("could not lay out the banner: {e}");
                Vec::new()
            }
        };
        // 拡大鏡を出すなら、カーソルの横の座標は拡大鏡の下に付ける
        let cursor_label = match (&view.cursor_label, &magnifier) {
            (Some((at, text)), None) => {
                self.cursor_label(selection, *at, text).unwrap_or_else(|e| {
                    log::warn!("could not lay out the cursor label: {e}");
                    None
                })
            }
            _ => None,
        };
        let scene = Scene {
            banners,
            rect: view.rect,
            size_label,
            handles: view.handles,
            crosshair: view.crosshair,
            cursor_label,
            highlight: view.highlight.map(|h| h.rect),
            magnifier,
        };
        let factory = if scene.is_empty() {
            None
        } else {
            match self.painter() {
                Ok(painter) => Some(painter.factory.clone()),
                Err(e) => {
                    log::warn!("could not create Direct2D: {e}");
                    None
                }
            }
        };
        for view in &mut self.monitors {
            if let Err(e) = view.draw(factory.as_ref(), &scene) {
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
    /// そのモニタの上にはみ出すなら、範囲の内側の左上に置く。右にはみ出すなら、モニタの右端に寄せる。
    fn label(&mut self, selection: &Selection, rect: Rect, text: &str) -> Result<Option<Label>> {
        let monitor = monitor_at(selection, rect.origin());
        let dpi = monitor.map_or(96, |m| m.dpi);
        let gap = LABEL_GAP * dpi as f32 / 96.0;
        let (text, format, width, height) = self.measure(text, dpi)?;
        let mut left = rect.left as f32;
        if let Some(m) = monitor {
            left = left
                .min(m.rect.right as f32 - width)
                .max(m.rect.left as f32);
        }
        let mut top = rect.top as f32 - gap - height;
        if monitor.is_some_and(|m| top < m.rect.top as f32) {
            top = rect.top as f32 + gap;
        }
        Ok(Some(Label::new(
            text, format, left, top, width, height, dpi,
        )))
    }

    /// カーソルの横の座標の表示を、カーソルのあるモニタの DPI の大きさで、カーソルの右下に置く。
    /// そのモニタの右や下にはみ出すなら、カーソルの左や上に回す。
    fn cursor_label(
        &mut self,
        selection: &Selection,
        at: Point,
        text: &str,
    ) -> Result<Option<Label>> {
        let monitor = monitor_at(selection, at);
        let dpi = monitor.map_or(96, |m| m.dpi);
        let offset = CURSOR_LABEL_OFFSET * dpi as f32 / 96.0;
        let (text, format, width, height) = self.measure(text, dpi)?;
        let mut left = at.x as f32 + offset;
        let mut top = at.y as f32 + offset;
        if let Some(m) = monitor {
            if left + width > m.rect.right as f32 {
                left = at.x as f32 - offset - width;
            }
            if top + height > m.rect.bottom as f32 {
                top = at.y as f32 - offset - height;
            }
        }
        Ok(Some(Label::new(
            text, format, left, top, width, height, dpi,
        )))
    }

    /// `banner` を、モニタごとにその DPI の大きさで、上端の中央に置く。
    fn banners(&mut self) -> Result<Vec<Label>> {
        let Some(banner) = self.banner.clone() else {
            return Ok(Vec::new());
        };
        let monitors: Vec<(Rect, u32)> = self
            .monitors
            .iter()
            .map(|view| (to_rect(view.shot.rect), view.shot.dpi))
            .collect();
        let mut labels = Vec::with_capacity(monitors.len());
        for (rect, dpi) in monitors {
            let (text, format, width, height) = self.measure(&banner, dpi)?;
            let left = rect.left as f32 + (rect.width() as f32 - width) / 2.0;
            let top = rect.top as f32 + BANNER_TOP * dpi as f32 / 96.0;
            labels.push(Label::new(text, format, left, top, width, height, dpi));
        }
        Ok(labels)
    }

    /// 拡大鏡を、カーソルのあるモニタの DPI の大きさで組む。拡大した画素の下に `label`（カーソルの横の
    /// 座標）と色コードを並べ、全体を `magnifier::place` で置く。カーソルのあるモニタの画面を読めなければ
    /// （HDR・回転など）None。
    fn magnifier(
        &mut self,
        selection: &Selection,
        at: Point,
        label: Option<&str>,
    ) -> Result<Option<Magnifier>> {
        let Some(index) = selection
            .monitors()
            .iter()
            .position(|m| m.rect.contains(at))
        else {
            return Ok(None);
        };
        let monitor = selection.monitors()[index];
        let Some(view) = self.monitors.get(index) else {
            return Ok(None);
        };
        let source = magnifier::source(at);
        let Some(fragment) = source
            .intersect(&monitor.rect)
            .map(|area| view.shot.read_back(area))
            .transpose()?
            .flatten()
        else {
            return Ok(None);
        };
        let color_at = |p: Point| {
            let x = p.x - fragment.at.x;
            let y = p.y - fragment.at.y;
            let inside = (0..fragment.image.width() as i32).contains(&x)
                && (0..fragment.image.height() as i32).contains(&y);
            inside.then(|| fragment.image.pixel(x as u32, y as u32))
        };
        let Some(under) = color_at(at) else {
            return Ok(None);
        };

        let dpi = monitor.dpi;
        let scaled = |len: i32| (len * dpi as i32 + 48) / 96;
        let cell = scaled(magnifier::ZOOM).max(1);
        let side = cell * magnifier::SPAN;
        let mut texts = Vec::new();
        for text in label
            .map(str::to_owned)
            .into_iter()
            .chain([magnifier::color_code(under)])
        {
            texts.push(self.measure(&text, dpi)?);
        }
        let width = texts
            .iter()
            .map(|(_, _, w, _)| w.ceil() as i32)
            .fold(side, i32::max);
        let height = side
            + texts
                .iter()
                .map(|(_, _, _, h)| h.ceil() as i32)
                .sum::<i32>();
        let panel = magnifier::place(at, width, height, scaled(magnifier::OFFSET), monitor.rect);

        let area = Rect::new(panel.left, panel.top, panel.left + side, panel.top + side);
        let mut cells = Vec::with_capacity((magnifier::SPAN * magnifier::SPAN) as usize);
        let mut center = area;
        for row in 0..magnifier::SPAN {
            for col in 0..magnifier::SPAN {
                let pixel = Point::new(source.left + col, source.top + row);
                let left = area.left + col * cell;
                let top = area.top + row * cell;
                let rect = Rect::new(left, top, left + cell, top + cell);
                if pixel == at {
                    center = rect;
                }
                cells.push((rect, color_at(pixel).map(color_f)));
            }
        }
        let mut labels = Vec::with_capacity(texts.len());
        let mut top = area.bottom as f32;
        for (text, format, _, height) in texts {
            let height = height.ceil();
            labels.push(Label::new(
                text,
                format,
                panel.left as f32,
                top,
                width as f32,
                height,
                dpi,
            ));
            top += height;
        }
        Ok(Some(Magnifier {
            cells,
            center,
            area,
            labels,
        }))
    }

    /// `text` を `dpi` のモニタの大きさで測る。返すのは UTF-16 の文字列、書式、余白を含む幅と高さ。
    fn measure(&mut self, text: &str, dpi: u32) -> Result<(Vec<u16>, IDWriteTextFormat, f32, f32)> {
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
        let width = metrics.width.ceil() + LABEL_PADDING_X * scale * 2.0;
        let height = metrics.height.ceil() + LABEL_PADDING_Y * scale * 2.0;
        Ok((text, format, width, height))
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

impl Label {
    /// 背景の左上と大きさ（余白を含む）から作る。
    fn new(
        text: Vec<u16>,
        format: IDWriteTextFormat,
        left: f32,
        top: f32,
        width: f32,
        height: f32,
        dpi: u32,
    ) -> Label {
        let scale = dpi as f32 / 96.0;
        let back = D2D_RECT_F {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        let text_rect = D2D_RECT_F {
            left: back.left + LABEL_PADDING_X * scale,
            top: back.top + LABEL_PADDING_Y * scale,
            right: back.right - LABEL_PADDING_X * scale,
            bottom: back.bottom - LABEL_PADDING_Y * scale,
        };
        Label {
            text,
            format,
            back,
            text_rect,
        }
    }
}

/// BGRA の画素を Direct2D の色にする（描画先は sRGB の変換をしない UNORM なので、値がそのまま出る）。
fn color_f([b, g, r, _]: [u8; 4]) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    }
}

/// `at` の画素を含むモニタ。
fn monitor_at(selection: &Selection, at: Point) -> Option<MonitorInfo> {
    selection
        .monitors()
        .iter()
        .find(|m| m.rect.contains(at))
        .copied()
}

/// カーソルの形を当てる。
pub fn set_cursor(shape: CursorShape) {
    let id = match shape {
        CursorShape::Cross => IDC_CROSS,
        CursorShape::Move => IDC_SIZEALL,
        CursorShape::ResizeNwse => IDC_SIZENWSE,
        CursorShape::ResizeNesw => IDC_SIZENESW,
        CursorShape::ResizeNs => IDC_SIZENS,
        CursorShape::ResizeWe => IDC_SIZEWE,
    };
    // SAFETY: 共有のカーソルなので壊さなくてよい
    if let Ok(cursor) = unsafe { LoadCursorW(None, id) } {
        // SAFETY: UI スレッドで呼ぶ
        unsafe { SetCursor(Some(cursor)) };
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
    /// 固定された画面を写し、その上に `scene` を描く。`factory` がなければ `scene` は描かない。
    fn draw(&mut self, factory: Option<&ID2D1Factory1>, scene: &Scene) -> Result<()> {
        self.copy_shot()?;
        if let Some(factory) = factory
            && !scene.is_empty()
            && let Err(e) = self.draw_selection(factory, scene)
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

    /// 十字線、ハイライト、範囲の枠、つまみ、サイズ（ハイライトなら名前とサイズ）、カーソルの横の座標を出す。スクリーン座標からこのウィンドウの
    /// 座標に直して描く。
    fn draw_selection(&mut self, factory: &ID2D1Factory1, scene: &Scene) -> Result<()> {
        let monitor = to_rect(self.shot.rect);
        let highlight_width = (HIGHLIGHT_WIDTH * self.shot.dpi as f32 / 96.0)
            .round()
            .max(1.0) as i32;
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
        // 枠は範囲の外側に 1 画素ずつ、内側の白と外側の黒の 2 本（範囲の画素には重ねない）
        let lines = scene.rect.map(|rect| {
            [
                (rect.inflate(1), rect, &d2d.frame_inner),
                (rect.inflate(2), rect.inflate(1), &d2d.frame_outer),
            ]
        });
        let context = &d2d.context;
        // このモニタに重なる部分だけ塗る
        let fill = |r: Rect, brush: &ID2D1SolidColorBrush| {
            if let Some(r) = r.intersect(&monitor) {
                // SAFETY: BeginDraw と EndDraw の間で呼ぶ
                unsafe { context.FillRectangle(&local(&to_rect_f(r)), brush) };
            }
        };
        let text = |label: &Label| {
            // SAFETY: BeginDraw と EndDraw の間で呼ぶ
            unsafe {
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
        };
        // SAFETY: このモニタのバックバッファに描く。UI スレッドで呼ぶ
        unsafe {
            context.BeginDraw();
            context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        }
        // 十字線はカーソルのあるモニタにだけ、そのモニタいっぱいに引く
        if let Some(at) = scene.crosshair.filter(|&at| monitor.contains(at)) {
            fill(
                Rect::new(monitor.left, at.y, monitor.right, at.y + 1),
                &d2d.crosshair,
            );
            fill(
                Rect::new(at.x, monitor.top, at.x + 1, monitor.bottom),
                &d2d.crosshair,
            );
        }
        // ハイライトは中を薄く塗り、枠を内側に描く（最大化したウィンドウでもモニタの端で見えるように）
        if let Some(rect) = scene.highlight {
            fill(rect, &d2d.highlight_fill);
            let inner = rect.inflate(-highlight_width);
            if inner.is_empty() {
                fill(rect, &d2d.highlight_frame);
            } else {
                for band in ring(rect, inner) {
                    fill(band, &d2d.highlight_frame);
                }
            }
        }
        for (outer, inner, brush) in lines.into_iter().flatten() {
            for band in ring(outer, inner) {
                fill(band, brush);
            }
        }
        // つまみは黒で縁取った白い四角
        for &handle in &scene.handles {
            fill(handle.inflate(1), &d2d.frame_outer);
            fill(handle, &d2d.frame_inner);
        }
        if let Some(label) = &scene.size_label {
            text(label);
        }
        if let Some(label) = &scene.cursor_label {
            text(label);
        }
        // 上端の文字はこのモニタのものだけ（ほかのモニタの分は描いても外れる）
        for label in &scene.banners {
            let inside =
                monitor.contains(Point::new(label.back.left as i32, label.back.top as i32));
            if inside {
                text(label);
            }
        }
        if let Some(m) = &scene.magnifier {
            // 黒で縁取り、拡大した画素を塗り、カーソルの画素を白と黒の枠で囲む
            fill(m.area.inflate(1), &d2d.frame_outer);
            for &(cell, color) in &m.cells {
                // SAFETY: このデバイス コンテキストのブラシ
                unsafe { d2d.pixel.SetColor(&color.unwrap_or(MAGNIFIER_OUTSIDE)) };
                fill(cell, &d2d.pixel);
            }
            for band in ring(m.center.inflate(2), m.center.inflate(1)) {
                fill(band, &d2d.frame_outer);
            }
            for band in ring(m.center.inflate(1), m.center) {
                fill(band, &d2d.frame_inner);
            }
            for label in &m.labels {
                text(label);
            }
        }
        // SAFETY: BeginDraw と対
        unsafe { context.EndDraw(None, None) }
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
                frame_inner: context.CreateSolidColorBrush(&FRAME_INNER, None)?,
                frame_outer: context.CreateSolidColorBrush(&FRAME_OUTER, None)?,
                label_back: context.CreateSolidColorBrush(&LABEL_BACK, None)?,
                label_text: context.CreateSolidColorBrush(&LABEL_TEXT, None)?,
                highlight_frame: context.CreateSolidColorBrush(&HIGHLIGHT_FRAME, None)?,
                highlight_fill: context.CreateSolidColorBrush(&HIGHLIGHT_FILL, None)?,
                crosshair: context.CreateSolidColorBrush(&CROSSHAIR, None)?,
                pixel: context.CreateSolidColorBrush(&MAGNIFIER_OUTSIDE, None)?,
                _target: target,
                context,
            })
        }
    }
}

/// ハイライトの名前とサイズを置く基準の矩形。ハイライトのうちカーソルのあるモニタに見えている部分
/// （モニタをまたぐウィンドウや、画面の外にはみ出したウィンドウでも見えるように）。
fn highlight_label_at(selection: &Selection, rect: Rect) -> Rect {
    selection
        .monitors()
        .iter()
        .find(|m| m.rect.contains(selection.cursor()))
        .and_then(|m| m.rect.intersect(&rect))
        .unwrap_or(rect)
}

/// `outer` から `inner`（`outer` の内側）を除いた輪を、上・下・左・右の 4 つの帯で返す。
fn ring(outer: Rect, inner: Rect) -> [Rect; 4] {
    [
        Rect::new(outer.left, outer.top, outer.right, inner.top),
        Rect::new(outer.left, inner.bottom, outer.right, outer.bottom),
        Rect::new(outer.left, inner.top, inner.left, inner.bottom),
        Rect::new(inner.right, inner.top, outer.right, inner.bottom),
    ]
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
