//! スクラップの描画: 画像に枠と影を付けて、GDI の DIB（`Canvas`）に Direct2D で描く。
//!
//! - 描いた DIB は premultiplied BGRA。`UpdateLayeredWindow` にそのまま渡せる。
//! - Direct2D の座標は 1 DIP = 1 物理ピクセル（DPI 96 固定）。画像は等倍で、補間せずに描く。
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
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_PROPERTIES, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::core::Result;

/// 影の濃さ（いちばん内側の不透明度）。外側ほど薄くする。
const SHADOW_ALPHA: f32 = 0.22;

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
            })
        }
    }

    /// スクラップのウィンドウの中身を描く。`canvas` はウィンドウの大きさ（`ScrapModel::window_rect`）で、
    /// 画像は余白（`Appearance::margin`）の内側に等倍で置く。
    pub fn paint_scrap(
        &self,
        canvas: &Canvas,
        image: &Image,
        appearance: &Appearance,
    ) -> Result<()> {
        let margin = appearance.margin();
        let image_rect = Rect::from_origin_size(Point::new(margin, margin), image.size());
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
            draw_image(target, image, image_rect.origin())
        })
    }

    /// 画像だけを画像の大きさの `canvas` に描く（コピーや保存で、表示と同じ描き方を使うため）。
    pub fn paint_image(&self, canvas: &Canvas, image: &Image) -> Result<()> {
        self.paint(canvas, |target| draw_image(target, image, Point::default()))
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

/// 画像を `at` を左上として等倍で描く。
fn draw_image(target: &ID2D1RenderTarget, image: &Image, at: Point) -> Result<()> {
    let size = image.size();
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let pixels = image.to_premultiplied();
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let dest = to_rect_f(Rect::from_origin_size(at, size));
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
        target.DrawBitmap(
            &bitmap,
            Some(&dest),
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            None,
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
