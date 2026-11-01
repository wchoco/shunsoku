//! スクラップの描画: 画像に枠と影を付けて、GDI の DIB（`Canvas`）に Direct2D で描く。
//!
//! - 描いた DIB は premultiplied BGRA。`UpdateLayeredWindow` にそのまま渡せる。
//! - Direct2D の座標は 1 DIP = 1 物理ピクセル（DPI 96 固定）。画像は倍率を掛けた表示の大きさ
//!   （`ScrapModel::display_size`）に伸び縮みさせて描く。拡大と等倍は補間せず（画素が見えるように）、
//!   縮小は線形補間で描く。
//! - 倍率を変えた直後は、画像の左上に倍率（`ZoomLabel`）を重ねて描く。
//! - トリムの範囲を選んでいる間は、選んだ範囲の外を暗くし、範囲の縁に線を引く（`TrimOverlay`）。
//! - 透明度はここでは掛けない（レイヤード ウィンドウの `SourceConstantAlpha` で掛ける）。
//! - コピーと保存の画像は、`render_image` で画像だけを画像の大きさに描いてから作る（表示と同じ描き方にするため。
//!   M1 では書き込みがないので、元の画像と同じになる）。

use std::ffi::c_void;

use shunsoku_core::geometry::{Point, Rect, Size};
use shunsoku_core::image::Image;
use shunsoku_core::scrap::{Appearance, SHADOW_WIDTH};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::core::{Result, w};

/// 影の濃さ（いちばん内側の不透明度）。外側ほど薄くする。
const SHADOW_ALPHA: f32 = 0.22;

/// 倍率の表示の文字の大きさ・余白・画像の角からの距離（DPI 96 のときの物理ピクセル）。
const ZOOM_LABEL_FONT: f32 = 14.0;
const ZOOM_LABEL_PADDING_X: f32 = 6.0;
const ZOOM_LABEL_PADDING_Y: f32 = 3.0;
const ZOOM_LABEL_INSET: f32 = 6.0;
/// 倍率の表示の背景の不透明度。
const ZOOM_LABEL_BACK_ALPHA: f32 = 0.65;

/// トリムの範囲の外を暗くする黒の不透明度。
const TRIM_SHADE_ALPHA: f32 = 0.45;
/// トリムの範囲の縁の線の色。
const TRIM_EDGE_COLOR: u32 = 0xFF_FF_FF;

/// トリムの範囲を選んでいる間に画像に重ねるもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimOverlay {
    /// 選んだ範囲（ウィンドウの中の座標）。まだ選んでいなければ None（画像全体を暗くする）。
    pub selection: Option<Rect>,
}

/// 画像に重ねる倍率の表示。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomLabel {
    /// 倍率（%）。
    pub percent: u16,
    /// 文字の大きさの倍率（ウィンドウのあるモニタの DPI / 96）。
    pub scale: f32,
}

/// 描き先の DIB（上の行から並ぶ 32bit BGRA）と、それを選んだメモリ DC。
pub struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    /// DIB の画素（`bitmap` が生きている間だけ有効）。
    bits: *const u8,
    size: Size,
}

impl Canvas {
    pub fn new(size: Size) -> Result<Canvas> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.width as i32,
                // 負の高さで上の行から並べる
                biHeight: -(size.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: 作った DC とビットマップは drop で返す
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return Err(windows::core::Error::from_thread());
            }
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            {
                Ok(bitmap) => bitmap,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(e);
                }
            };
            let old = SelectObject(dc, bitmap.into());
            Ok(Canvas {
                dc,
                bitmap,
                old,
                bits: bits.cast_const().cast(),
                size,
            })
        }
    }

    /// DIB を選んだメモリ DC（`UpdateLayeredWindow` の描き元）。
    pub fn dc(&self) -> HDC {
        self.dc
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// 描いた中身（premultiplied）を、アルファを割り戻した画像にする。
    pub fn to_image(&self) -> Option<Image> {
        let len = self.size.width as usize * self.size.height as usize * 4;
        if self.bits.is_null() {
            return None;
        }
        // SAFETY: bits は CreateDIBSection が返した、上の行から並ぶ 32bit の DIB で、len バイトある。
        // GdiFlush で GDI の描き込みを終えてから読む
        let pixels = unsafe {
            let _ = GdiFlush();
            std::slice::from_raw_parts(self.bits, len).to_vec()
        };
        Image::from_premultiplied(self.size, pixels)
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: new で作ったものを返す
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Canvas に描く Direct2D の DC レンダー ターゲット。
pub struct Renderer {
    _factory: ID2D1Factory,
    target: ID2D1DCRenderTarget,
    /// 倍率の表示を書く DirectWrite。初めて表示するときに作る。
    dwrite: Option<IDWriteFactory>,
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        // SAFETY: 引数はこの関数の中で生きている
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let target = factory.CreateDCRenderTarget(&props)?;
            Ok(Renderer {
                _factory: factory,
                target,
                dwrite: None,
            })
        }
    }

    /// スクラップのウィンドウの中身を描く。`canvas` はウィンドウの大きさ（`ScrapModel::window_rect`）で、
    /// 画像は余白（`Appearance::margin`）の内側に `display`（`ScrapModel::display_size`）の大きさで置く。
    /// `label` があれば、画像の左上に倍率を重ねる。`trim` があれば、トリムの範囲を重ねる。
    pub fn paint_scrap(
        &mut self,
        canvas: &Canvas,
        image: &Image,
        display: Size,
        appearance: &Appearance,
        label: Option<ZoomLabel>,
        trim: Option<TrimOverlay>,
    ) -> Result<()> {
        let margin = appearance.margin();
        let image_rect = Rect::from_origin_size(Point::new(margin, margin), display);
        let dwrite = match label {
            Some(_) => Some(self.dwrite()?),
            None => None,
        };
        self.paint(canvas, |target| {
            // SAFETY: BeginDraw と EndDraw の間で呼ぶ
            unsafe {
                let mut edge = image_rect;
                if let Some(border) = appearance.border {
                    let width = border.width as i32;
                    let color = rgb(border.color, 1.0);
                    let brush = target.CreateSolidColorBrush(&color, None)?;
                    for _ in 0..width {
                        let outer = edge.inflate(1);
                        for strip in ring(outer, edge) {
                            target.FillRectangle(&to_rect_f(strip), &brush);
                        }
                        edge = outer;
                    }
                }
                if appearance.shadow {
                    let brush = target.CreateSolidColorBrush(&rgb(0, 0.0), None)?;
                    for i in 0..SHADOW_WIDTH {
                        // 外側ほど薄く（2 乗で落とす）
                        let t = 1.0 - i as f32 / SHADOW_WIDTH as f32;
                        brush.SetColor(&rgb(0, SHADOW_ALPHA * t * t));
                        let outer = edge.inflate(1);
                        for strip in ring(outer, edge) {
                            target.FillRectangle(&to_rect_f(strip), &brush);
                        }
                        edge = outer;
                    }
                }
            }
            draw_image(target, image, image_rect)?;
            if let Some(trim) = trim {
                draw_trim_overlay(target, image_rect, trim)?;
            }
            match (label, dwrite) {
                (Some(label), Some(dwrite)) => draw_zoom_label(target, &dwrite, image_rect, label),
                _ => Ok(()),
            }
        })
    }

    fn dwrite(&mut self) -> Result<IDWriteFactory> {
        if let Some(dwrite) = &self.dwrite {
            return Ok(dwrite.clone());
        }
        // SAFETY: 引数は定数
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        self.dwrite = Some(dwrite.clone());
        Ok(dwrite)
    }

    /// 画像だけを画像の大きさの `canvas` に描く（コピーや保存で、表示と同じ描き方を使うため）。
    pub fn paint_image(&self, canvas: &Canvas, image: &Image) -> Result<()> {
        self.paint(canvas, |target| {
            draw_image(
                target,
                image,
                Rect::from_origin_size(Point::default(), image.size()),
            )
        })
    }

    /// `canvas` を透明で塗ってから `draw` で描く。
    fn paint(
        &self,
        canvas: &Canvas,
        draw: impl FnOnce(&ID2D1RenderTarget) -> Result<()>,
    ) -> Result<()> {
        let size = canvas.size();
        let rect = RECT {
            left: 0,
            top: 0,
            right: size.width as i32,
            bottom: size.height as i32,
        };
        let target: &ID2D1RenderTarget = &self.target;
        // SAFETY: canvas の DC は描き終わるまで生きている
        unsafe {
            self.target.BindDC(canvas.dc(), &rect)?;
            target.BeginDraw();
            target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            target.Clear(Some(&rgb(0, 0.0)));
            let drawn = draw(target);
            let ended = target.EndDraw(None, None);
            drawn.and(ended)
        }
    }
}

/// コピーと保存の画像を作る: 画像だけを画像の画素の大きさに、表示と同じ描き方で描く。
pub fn render_image(image: &Image) -> Result<Image> {
    if image.width() == 0 || image.height() == 0 {
        return Ok(image.clone());
    }
    let canvas = Canvas::new(image.size())?;
    Renderer::new()?.paint_image(&canvas, image)?;
    canvas
        .to_image()
        .ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_UNEXPECTED))
}

/// 画像を `dest` に伸び縮みさせて描く。縮めるときだけ線形補間にする。
fn draw_image(target: &ID2D1RenderTarget, image: &Image, dest: Rect) -> Result<()> {
    let size = image.size();
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let interpolation: D2D1_BITMAP_INTERPOLATION_MODE =
        if dest.width() < size.width || dest.height() < size.height {
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
        } else {
            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        };
    let pixels = image.to_premultiplied();
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let dest = to_rect_f(dest);
    // SAFETY: pixels は CreateBitmap が写し終えるまで生きている
    unsafe {
        let bitmap = target.CreateBitmap(
            D2D_SIZE_U {
                width: size.width,
                height: size.height,
            },
            Some(pixels.as_ptr().cast()),
            size.width * 4,
            &props,
        )?;
        target.DrawBitmap(&bitmap, Some(&dest), 1.0, interpolation, None);
    }
    Ok(())
}

/// トリムの範囲の外（画像の中だけ）を暗くし、範囲の縁に 1px の線を引く。
fn draw_trim_overlay(
    target: &ID2D1RenderTarget,
    image_rect: Rect,
    trim: TrimOverlay,
) -> Result<()> {
    let selection = trim.selection.and_then(|r| r.intersect(&image_rect));
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ
    unsafe {
        let shade = target.CreateSolidColorBrush(&rgb(0, TRIM_SHADE_ALPHA), None)?;
        let Some(selection) = selection else {
            target.FillRectangle(&to_rect_f(image_rect), &shade);
            return Ok(());
        };
        for strip in ring(image_rect, selection) {
            if !strip.is_empty() {
                target.FillRectangle(&to_rect_f(strip), &shade);
            }
        }
        let edge = target.CreateSolidColorBrush(&rgb(TRIM_EDGE_COLOR, 1.0), None)?;
        // 範囲の内側に線を引く（画像の端でも見えるように）
        let inner = selection.inflate(-1);
        if inner.is_empty() {
            target.FillRectangle(&to_rect_f(selection), &edge);
        } else {
            for strip in ring(selection, inner) {
                target.FillRectangle(&to_rect_f(strip), &edge);
            }
        }
    }
    Ok(())
}

/// 倍率（`150%` など）を、半透明の黒い背景に白い文字で、画像の左上の内側に描く。
fn draw_zoom_label(
    target: &ID2D1RenderTarget,
    dwrite: &IDWriteFactory,
    image_rect: Rect,
    label: ZoomLabel,
) -> Result<()> {
    let scale = label.scale.max(1.0);
    let text: Vec<u16> = format!("{}%", label.percent).encode_utf16().collect();
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ。文字列と書式はこの関数の中で生きている
    unsafe {
        let format = dwrite.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            ZOOM_LABEL_FONT * scale,
            w!(""),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let layout = dwrite.CreateTextLayout(&text, &format, 4096.0, 1024.0)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics)?;
        let pad_x = ZOOM_LABEL_PADDING_X * scale;
        let pad_y = ZOOM_LABEL_PADDING_Y * scale;
        let left = image_rect.left as f32 + ZOOM_LABEL_INSET * scale;
        let top = image_rect.top as f32 + ZOOM_LABEL_INSET * scale;
        let back = D2D_RECT_F {
            left,
            top,
            right: left + metrics.width.ceil() + pad_x * 2.0,
            bottom: top + metrics.height.ceil() + pad_y * 2.0,
        };
        let text_rect = D2D_RECT_F {
            left: back.left + pad_x,
            top: back.top + pad_y,
            right: back.right - pad_x,
            bottom: back.bottom - pad_y,
        };
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let back_brush = target.CreateSolidColorBrush(&rgb(0, ZOOM_LABEL_BACK_ALPHA), None)?;
        target.FillRectangle(&back, &back_brush);
        let text_brush = target.CreateSolidColorBrush(&rgb(0xFF_FF_FF, 1.0), None)?;
        target.DrawText(
            &text,
            &format,
            &text_rect,
            &text_brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

/// `outer` から `inner` を除いた枠を、重ならない 4 本の帯にする。
fn ring(outer: Rect, inner: Rect) -> [Rect; 4] {
    [
        Rect::new(outer.left, outer.top, outer.right, inner.top),
        Rect::new(outer.left, inner.bottom, outer.right, outer.bottom),
        Rect::new(outer.left, inner.top, inner.left, inner.bottom),
        Rect::new(inner.right, inner.top, outer.right, inner.bottom),
    ]
}

/// `0xRRGGBB` と不透明度から色を作る。
fn rgb(color: u32, alpha: f32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((color >> shift) & 0xff) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: alpha,
    }
}

fn to_rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left as f32,
        top: r.top as f32,
        right: r.right as f32,
        bottom: r.bottom as f32,
    }
}
