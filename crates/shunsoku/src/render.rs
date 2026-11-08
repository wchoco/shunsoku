//! スクラップの描画: 画像に枠と影を付けて、GDI の DIB（`Canvas`）に Direct2D で描く。
//!
//! - 描いた DIB は premultiplied BGRA。`UpdateLayeredWindow` にそのまま渡せる。
//! - Direct2D の座標は 1 DIP = 1 物理ピクセル（DPI 96 固定）。画像は倍率を掛けた表示の大きさ
//!   （`ScrapModel::display_size`）に伸び縮みさせて描く。拡大と等倍は補間せず（画素が見えるように）、
//!   縮小は線形補間で描く。
//! - 倍率を変えた直後は、画像の左上に倍率（`ZoomLabel`）を重ねて描く。
//! - トリムの範囲を選んでいる間は、選んだ範囲の外を暗くし、範囲の縁に線を引く（`TrimOverlay`）。範囲選択の
//!   画面の精密モードと同じく、つまみ、十字線（画像の矩形の中だけ）、拡大鏡と座標・サイズ・色コードも描く。
//!   拡大鏡の画素は表示している画像（`ScrapModel::shown_image`）から読み、ウィンドウの中に収まるように置く。
//! - 書き込み（`annotate::Annotation`）は画像の上、トリムの範囲の上の重ねものの下に、画像の矩形の中だけに
//!   描く（`draw_annotations`）。座標の換算は core の `annotate::View` で、表示なら `ScrapModel::annotation_view`、
//!   保存とコピーなら `ScrapModel::output_view` を渡す。画像は補間せずに描くが、書き込みはアンチエイリアスする。
//! - 文字の書き込み（`Shape::Text`）は DirectWrite で、左上を書き込みの点の画素の左上に合わせ、行の高さを core の
//!   見積もり（`annotate::LINE_HEIGHT`）に揃えて描く。打っている文字（`TypingLayer`）は、IME で変換中の文字に
//!   下線を引いてつなげ、キャレットを描く（その矩形を IME の窓の位置に使う）。
//! - スクラップの画像のビットマップは、画像と一緒に作り置きにする（`CachedImage`）。画像が変わらない描き直しでは、
//!   画像全体の premultiplied への変換とビットマップの作り直しをしない。
//! - レンダー ターゲットはソフトウェア（`D2D1_RENDER_TARGET_TYPE_SOFTWARE`）。CPU で DIB に直接描く。
//! - 透明度はここでは掛けない（レイヤード ウィンドウの `SourceConstantAlpha` で掛ける）。
//! - 透明な白紙スクラップは、画像の下に当たり判定のための見えないほど小さいアルファ（`Appearance::hit_alpha`）を
//!   敷く。コピーと保存の画像（`render_image`）には敷かないので、透明な部分はアルファ 0 のまま。
//! - 書き込みの編集モードのツールバー（`toolbar_window`）も、ここで `paint_toolbar` で描く。並びは core の
//!   `annotate::toolbar::Layout`。アイコンは Direct2D の線と図形で描く（字形のフォントに頼らない）。
//! - コピーと保存の画像は、`render_image` で画像と書き込みを画像の大きさに描いてから作る（表示と同じ描き方に
//!   するため。ADR-0006）。

use std::cell::{RefCell, RefMut};
use std::ffi::c_void;
use std::ops::Deref;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

use shunsoku_core::annotate::toolbar::{Button, Layout, ToolSlot};
use shunsoku_core::annotate::{
    Annotation, Annotations, EditState, Font, LINE_HEIGHT, Shape, View, arrow_head,
};
use shunsoku_core::geometry::{Point, Rect, Size};
use shunsoku_core::image::Image;
use shunsoku_core::magnifier;
use shunsoku_core::scrap::{Appearance, SHADOW_WIDTH};
use shunsoku_core::settings::Rgb;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_INTERPOLATION_MODE,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_PROPERTIES, D2D1_CAP_STYLE_ROUND, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_LINE_JOIN_ROUND,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1Bitmap, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget,
    ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_HIT_TEST_METRICS,
    DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_METRICS,
    DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory,
    IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
};
use windows::core::{Result, w};
use windows_numerics::Vector2;

#[cfg(test)]
mod tests;

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

/// トリムの拡大鏡の、画像の外の画素の色。
const TRIM_MAGNIFIER_OUTSIDE: u32 = 0x40_40_40;
/// トリムの拡大鏡の下の文字の大きさと余白（DPI 96 のときの物理ピクセル）。
const TRIM_LABEL_FONT: f32 = 12.0;
const TRIM_LABEL_PADDING_X: f32 = 6.0;
const TRIM_LABEL_PADDING_Y: f32 = 3.0;

/// トリムの範囲を選んでいる間に画像に重ねるもの（core の `TrimView` をウィンドウの中の座標にしたもの）。
#[derive(Debug, Clone, PartialEq)]
pub struct TrimOverlay {
    /// 選んだ範囲（ウィンドウの中の座標）。まだ選んでいなければ None（画像全体を暗くする）。
    pub selection: Option<Rect>,
    /// 範囲のつまみ（ウィンドウの中の座標）。
    pub handles: Vec<Rect>,
    /// 十字線を通す点（ウィンドウの中の座標）と色（`[selection] crosshair_color`）。画像の矩形の中だけに引く。
    pub crosshair: Option<(Point, Rgb)>,
    /// 拡大鏡。
    pub magnifier: Option<TrimMagnifier>,
    /// 拡大鏡と文字の大きさの倍率（ウィンドウのあるモニタの DPI / 96）。
    pub scale: f32,
}

/// トリムの拡大鏡。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimMagnifier {
    /// カーソルの位置（ウィンドウの中の座標）。拡大鏡はこの横に置く。
    pub cursor: Point,
    /// 拡大して見せる画素（表示している画像の画素）。
    pub pixel: Point,
    /// 拡大した画素の下に出す座標とサイズ（色コードはその下に足す）。
    pub label: Option<String>,
}

/// 画像に重ねる書き込み。
#[derive(Debug, Clone, Copy)]
pub struct AnnotationLayer<'a> {
    /// 描く順（後のものほど上）。
    pub annotations: &'a [Annotation],
    /// 元の画像の画素から、画像の左上を原点とした座標への変換。
    pub view: View,
    /// 選んでいる書き込みの外接矩形（元の画像の画素、`Annotation::bounds`）。それぞれに選択の印を描く。
    /// 保存とコピーには描かない（空）。
    pub selected: &'a [Rect],
    /// 範囲で選んでいる間の範囲（元の画像の画素、`EditState::marquee`）。あれば描く。保存とコピーには描かない
    /// （None）。
    pub selection_range: Option<Rect>,
    /// 打っている文字。保存とコピーには描かない（None）。
    pub typing: Option<TypingLayer<'a>>,
    /// 選択の印とキャレットの大きさの倍率（ウィンドウのあるモニタの DPI / 96）。書き込みそのものは画像の画素で
    /// 描くので、これを掛けない。
    pub scale: f32,
}

/// 打っている文字と、IME で変換中の文字。
#[derive(Debug, Clone, Copy)]
pub struct TypingLayer<'a> {
    /// 打っている文字の書き込み（`Shape::Text`。`EditState::typing`）。
    pub annotation: &'a Annotation,
    /// 変換中の文字（打っている文字の後ろにつなげて、下線を引いて描く）。
    pub composition: &'a str,
    /// 変換中の文字の中のキャレットの位置（UTF-16 の単位）。
    pub cursor: usize,
}

/// スクラップの画像に重ねるもの。下から書き込み・トリムの範囲・倍率の順に描く。
#[derive(Debug, Clone, Copy)]
pub struct Overlays<'a> {
    pub annotations: AnnotationLayer<'a>,
    /// あれば、画像の左上に倍率を出す。
    pub label: Option<ZoomLabel>,
    /// あれば、トリムの範囲を出す。
    pub trim: Option<&'a TrimOverlay>,
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

    /// 使い回す描き先 `slot` を大きさ `size` で返す。同じ大きさのものがあればそのまま（中身は描くときに
    /// `Renderer` が消す）、なければ前のものを返してから作り直す。作れなければ `slot` は空になる。
    ///
    /// `slot` がほかの描き直しに借りられていれば（出している間に届いたメッセージで描き直したとき）、使い捨ての
    /// 描き先を作って返す（`slot` には触れない）。
    ///
    /// `UpdateLayeredWindow` は中身を写し取るので、出し終えた描き先は次の描き直しで使ってよい。
    pub fn reuse(slot: &RefCell<Option<Canvas>>, size: Size) -> Result<CanvasLease<'_>> {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return Ok(CanvasLease::Once(Canvas::new(size)?));
        };
        if slot.as_ref().is_none_or(|canvas| canvas.size != size) {
            // 大きな DIB を 2 枚同時に持たないよう、先に返す
            *slot = None;
            *slot = Some(Canvas::new(size)?);
        }
        Ok(CanvasLease::Reused(RefMut::map(slot, |slot| {
            slot.as_mut().expect("filled above")
        })))
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

/// `Canvas::reuse` が返す描き先。使い回すもの（借りている間はほかの描き直しに使わせない）か、使い捨てのもの。
pub enum CanvasLease<'a> {
    Reused(RefMut<'a, Canvas>),
    Once(Canvas),
}

impl Deref for CanvasLease<'_> {
    type Target = Canvas;

    fn deref(&self) -> &Canvas {
        match self {
            CanvasLease::Reused(canvas) => canvas,
            CanvasLease::Once(canvas) => canvas,
        }
    }
}

/// スクラップに表示する画像と、描き手（`Renderer`）がその画像から作った Direct2D のビットマップの作り置き。
///
/// 描き直しのたびに画像全体を premultiplied にしてビットマップを作り直さないよう、初めて描いたときに作って
/// 持っておく。画像はあとから差し替えられない（変わったら新しく作る）ので、作り置きが別の画像のものになることは
/// ない。ビットマップは作ったレンダー ターゲットでしか使えないので、作った描き手の番号（`Renderer::id`）も持ち、
/// 描き手が作り直されていたら（`D2DERR_RECREATE_TARGET` などで捨てたあと）古いものを捨てて作り直す。
/// 作り置きはこの値と一緒に捨てる（スクラップを閉じたら返る）。描き手を捨てたときも、その描き手で作った作り置きは
/// 次に描くのを待たずに捨てる（`Kept`）。
pub struct CachedImage {
    image: Image,
    bitmap: Rc<BitmapSlot>,
}

/// 作り置きのビットマップと、それを作った描き手の番号（`Kept::id`）。
type BitmapSlot = RefCell<Option<(u64, ID2D1Bitmap)>>;

/// 描き手が作り置きにさせたビットマップの控え。描き手を捨てるとき（`Renderer` の drop）、ここから辿って捨てる。
struct Kept {
    /// 描き手ごとに違う番号。作り置きのビットマップを作った描き手を見分ける。
    id: u64,
    /// 作り置きにさせた `CachedImage` の置き場。閉じたスクラップのものは、次に作り置きを足すときに除く。
    slots: RefCell<Vec<Weak<BitmapSlot>>>,
}

impl Kept {
    fn new() -> Kept {
        Kept {
            id: RENDERER_IDS.fetch_add(1, Ordering::Relaxed),
            slots: RefCell::new(Vec::new()),
        }
    }

    fn remember(&self, slot: &Rc<BitmapSlot>) {
        let mut slots = self.slots.borrow_mut();
        slots.retain(|slot| slot.strong_count() > 0);
        slots.push(Rc::downgrade(slot));
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        for slot in self.slots.get_mut().iter().filter_map(Weak::upgrade) {
            // ほかの描き手が作り直したものは残す
            if let Ok(mut slot) = slot.try_borrow_mut()
                && slot.as_ref().is_some_and(|(id, _)| *id == self.id)
            {
                *slot = None;
            }
        }
    }
}

impl CachedImage {
    pub fn new(image: Image) -> CachedImage {
        CachedImage {
            image,
            bitmap: Rc::new(RefCell::new(None)),
        }
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    /// 作り置きを捨てて画像だけを返す。
    pub fn into_image(self) -> Image {
        self.image
    }

    /// ビットマップを作り置きにしているか。
    #[cfg(test)]
    fn keeps_bitmap(&self) -> bool {
        self.bitmap.borrow().is_some()
    }

    /// 画像を `dest` に伸び縮みさせて描く。縮めるときだけ線形補間にする。空の画像は描かない。
    /// `kept` があればその描き手の作り置きを使い（なければ作って控えさせる）、なければその場で作って捨てる
    /// （コピーと保存の使い捨ての描き手で、スクラップの描き手の作り置きを追い出さないため）。
    fn draw(&self, target: &ID2D1RenderTarget, dest: Rect, kept: Option<&Kept>) -> Result<()> {
        let size = self.image.size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let bitmap = match kept {
            Some(kept) => self.bitmap(target, kept)?,
            None => create_bitmap(target, &self.image)?,
        };
        draw_bitmap(target, &bitmap, size, dest);
        Ok(())
    }

    /// `kept` の描き手で使えるビットマップ。なければ（初めて描くとき、描き手が作り直されたとき）作る。
    fn bitmap(&self, target: &ID2D1RenderTarget, kept: &Kept) -> Result<ID2D1Bitmap> {
        let mut cached = self.bitmap.borrow_mut();
        if let Some((id, bitmap)) = cached.as_ref()
            && *id == kept.id
        {
            return Ok(bitmap.clone());
        }
        // 別の描き手のものは、作る前に捨てる（2 枚ぶん持たない）
        *cached = None;
        let bitmap = create_bitmap(target, &self.image)?;
        *cached = Some((kept.id, bitmap.clone()));
        kept.remember(&self.bitmap);
        Ok(bitmap)
    }
}

/// 描き手の番号の払い出し（`Kept::id`）。
static RENDERER_IDS: AtomicU64 = AtomicU64::new(0);

/// Canvas に描く Direct2D の DC レンダー ターゲット。
pub struct Renderer {
    /// この描き手で作ったビットマップの作り置き（`CachedImage`）の控え。描き手と一緒に捨て、作り置きも捨てさせる。
    /// レンダー ターゲットより先に捨てる（フィールドは書いた順に drop される）。
    kept: Kept,
    factory: ID2D1Factory,
    target: ID2D1DCRenderTarget,
    /// 倍率の表示を書く DirectWrite。初めて表示するときに作る。
    dwrite: Option<IDWriteFactory>,
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            // GPU で描いて DIB に読み戻すより、CPU で DIB に直接描く方が速い（#168、docs/perf.md の書き込みの
            // 更新の表）。GPU のないマシンでも同じ種類で描ける。
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
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
                kept: Kept::new(),
                factory,
                target,
                dwrite: None,
            })
        }
    }

    /// スクラップのウィンドウの中身を描く。`canvas` はウィンドウの大きさ（`ScrapModel::window_rect`）で、
    /// 画像は余白（`Appearance::margin`）の内側に `display`（`ScrapModel::display_size`）の大きさで置く。
    /// 画像のビットマップは `image` の作り置きを使う（なければ作る）。
    /// 画像の上に `overlays` を重ねる。文字を打っていれば、キャレットの矩形（ウィンドウの中の座標）を返す。
    /// `hit_alpha` があれば（透明な白紙スクラップ）、画像の下にそのアルファの黒を敷き、透明な部分のクリックが
    /// 下のウィンドウに抜けないようにする（`Appearance::hit_alpha`）。
    pub fn paint_scrap(
        &mut self,
        canvas: &Canvas,
        image: &CachedImage,
        display: Size,
        appearance: &Appearance,
        hit_alpha: Option<u8>,
        overlays: Overlays,
    ) -> Result<Option<Rect>> {
        let Overlays {
            annotations,
            label,
            trim,
        } = overlays;
        let margin = appearance.margin();
        let image_rect = Rect::from_origin_size(Point::new(margin, margin), display);
        let magnified = trim.and_then(|t| t.magnifier.as_ref().map(|m| (m, t.scale)));
        let dwrite = if label.is_some() || magnified.is_some() {
            Some(self.dwrite()?)
        } else {
            None
        };
        let window = Rect::from_origin_size(Point::default(), canvas.size());
        let kept = &self.kept;
        let mut caret = None;
        self.paint(canvas, |target| {
            // SAFETY: BeginDraw と EndDraw の間で呼ぶ
            unsafe {
                let mut edge = image_rect;
                if let Some(border) = appearance.shown_border() {
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
            if let Some(alpha) = hit_alpha {
                // SAFETY: BeginDraw と EndDraw の間で呼ぶ
                unsafe {
                    let brush =
                        target.CreateSolidColorBrush(&rgb(0, f32::from(alpha) / 255.0), None)?;
                    target.FillRectangle(&to_rect_f(image_rect), &brush);
                }
            }
            image.draw(target, image_rect, Some(kept))?;
            caret = draw_annotations(target, annotations, image_rect)?;
            if let Some(trim) = trim {
                draw_trim_overlay(target, image_rect, trim)?;
            }
            if let (Some(label), Some(dwrite)) = (label, &dwrite) {
                draw_zoom_label(target, dwrite, image_rect, label)?;
            }
            match (magnified, &dwrite) {
                (Some((m, scale)), Some(dwrite)) => {
                    draw_trim_magnifier(target, dwrite, &image.image, window, m, scale)
                }
                _ => Ok(()),
            }
        })?;
        Ok(caret)
    }

    /// 書き込みのツールバーを描く。`canvas` は `layout` の大きさ。`state` の道具・色・太さのボタンを選んだ
    /// 見た目にする。`scale` はスクラップのモニタの DPI / 96。
    pub fn paint_toolbar(
        &self,
        canvas: &Canvas,
        layout: &Layout,
        state: &EditState,
        annotations: &Annotations,
        scale: f32,
    ) -> Result<()> {
        let factory = &self.factory;
        self.paint(canvas, |target| {
            draw_toolbar(target, factory, layout, state, annotations, scale)
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

    /// 画像と書き込みを画像の大きさの `canvas` に描く（コピーや保存で、表示と同じ描き方を使うため）。
    pub fn paint_image(
        &self,
        canvas: &Canvas,
        image: &Image,
        annotations: AnnotationLayer,
    ) -> Result<()> {
        let rect = Rect::from_origin_size(Point::default(), image.size());
        self.paint(canvas, |target| {
            draw_image(target, image, rect)?;
            draw_annotations(target, annotations, rect).map(|_| ())
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

/// コピーと保存の画像を作る: 画像と書き込みを画像の画素の大きさに、表示と同じ描き方で描く。
pub fn render_image(image: &Image, annotations: AnnotationLayer) -> Result<Image> {
    if image.width() == 0 || image.height() == 0 {
        return Ok(image.clone());
    }
    let canvas = Canvas::new(image.size())?;
    Renderer::new()?.paint_image(&canvas, image, annotations)?;
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
    let bitmap = create_bitmap(target, image)?;
    draw_bitmap(target, &bitmap, size, dest);
    Ok(())
}

/// 大きさ `size` の画像のビットマップを `dest` に伸び縮みさせて描く。縮めるときだけ線形補間にする。
fn draw_bitmap(target: &ID2D1RenderTarget, bitmap: &ID2D1Bitmap, size: Size, dest: Rect) {
    let interpolation: D2D1_BITMAP_INTERPOLATION_MODE =
        if dest.width() < size.width || dest.height() < size.height {
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
        } else {
            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        };
    let dest = to_rect_f(dest);
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ
    unsafe { target.DrawBitmap(bitmap, Some(&dest), 1.0, interpolation, None) };
}

/// 画像全体を premultiplied にして、`target` で使えるビットマップを作る。画像は空でないこと。
fn create_bitmap(target: &ID2D1RenderTarget, image: &Image) -> Result<ID2D1Bitmap> {
    let size = image.size();
    let pixels = image.to_premultiplied();
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    // SAFETY: pixels は CreateBitmap が写し終えるまで生きている
    unsafe {
        target.CreateBitmap(
            D2D_SIZE_U {
                width: size.width,
                height: size.height,
            },
            Some(pixels.as_ptr().cast()),
            size.width * 4,
            &props,
        )
    }
}

/// 書き込みを、`image_rect` の左上を原点に `layer.view` で置いて描く。画像の矩形の外（トリムで隠した部分など）は
/// 描かない。線は画素の中心を通るので、奇数の太さを等倍で描くと画素に揃う。線・矢印・フリーハンドの端と
/// 折れ目は丸める。矢印の頭（core の `arrow_head`）は塗りつぶした三角形で、軸は頭の底辺まで引く。
/// 打っている文字（`layer.typing`）は書き込みの上に描き、そのキャレットの矩形（描く先の座標）を返す。
fn draw_annotations(
    target: &ID2D1RenderTarget,
    layer: AnnotationLayer,
    image_rect: Rect,
) -> Result<Option<Rect>> {
    if (layer.annotations.is_empty() && layer.typing.is_none() && layer.selection_range.is_none())
        || image_rect.is_empty()
    {
        return Ok(None);
    }
    let (dx, dy) = (f64::from(image_rect.left), f64::from(image_rect.top));
    let at = |p: Point| {
        let (x, y) = layer.view.point(p);
        (x + dx, y + dy)
    };
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ。Push と Pop は対にする
    unsafe {
        target.PushAxisAlignedClip(&to_rect_f(image_rect), D2D1_ANTIALIAS_MODE_ALIASED);
        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        let drawn = (|| {
            let factory = target.GetFactory()?;
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    ..Default::default()
                },
                None,
            )?;
            let brush = target.CreateSolidColorBrush(&rgb(0, 1.0), None)?;
            // 文字があるときだけ DirectWrite を使う（共有のファクトリなので、作るのは 1 度だけ）
            let has_text = layer.typing.is_some()
                || layer
                    .annotations
                    .iter()
                    .any(|a| matches!(a.shape, Shape::Text { .. }));
            let dwrite: Option<IDWriteFactory> = if has_text {
                // レイヤード ウィンドウは画素ごとのアルファを持つので、ClearType ではなくグレースケールで描く
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                Some(DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?)
            } else {
                None
            };
            let pen = Pen {
                target,
                factory: &factory,
                dwrite: dwrite.as_ref(),
                brush: &brush,
                round: &round,
                view: layer.view,
                origin: (dx, dy),
            };
            for annotation in layer.annotations {
                brush.SetColor(&color_of(annotation));
                let width = layer.view.width(annotation.style.width);
                pen.draw(&annotation.shape, &at, width)?;
            }
            match layer.typing {
                Some(typing) => {
                    brush.SetColor(&color_of(typing.annotation));
                    pen.draw_typing(typing, layer.scale)
                }
                None => Ok(None),
            }
        })();
        target.PopAxisAlignedClip();
        // 選択の印は画像の端の書き込みでも見えるよう、画像の矩形で切らない
        let marked = drawn.and_then(|caret| {
            for bounds in layer.selected {
                let (x0, y0) = at(Point::new(bounds.left, bounds.top));
                let (x1, y1) = at(Point::new(bounds.right, bounds.bottom));
                draw_selection_mark(target, (x0, y0, x1, y1), layer.scale)?;
            }
            if let Some(range) = layer.selection_range {
                // 範囲は画素の矩形なので、左上の画素の左上の角から右下の画素の右下の角まで
                let (x0, y0) = at(Point::new(range.left, range.top));
                let (x1, y1) = at(Point::new(range.right, range.bottom));
                let half = (layer.view.scale_x / 2.0, layer.view.scale_y / 2.0);
                draw_selection_range(
                    target,
                    (x0 - half.0, y0 - half.1, x1 - half.0, y1 - half.1),
                    layer.scale,
                )?;
            }
            Ok(caret)
        });
        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        marked
    }
}

/// 選択の印の色と、外接矩形から外へ空ける幅・角の四角の大きさ・白い縁と青い枠の太さ（96 DPI での画素。モニタの
/// DPI で換算する）。
const SELECTION_COLOR: u32 = 0x1E_88_E5;
const SELECTION_PAD: f32 = 3.0;
const SELECTION_HANDLE: f32 = 5.0;
const SELECTION_HALO: f32 = 3.0;
const SELECTION_LINE: f32 = 1.0;

/// 選んでいる書き込みの印: 外接矩形（描く先の座標 `(left, top, right, bottom)`）を少し広げた青い枠と、
/// 角の白い四角。どんな色の上でも見えるように、枠は白い縁の上に引く。大きさは `scale`（DPI / 96）倍にする。
fn draw_selection_mark(
    target: &ID2D1RenderTarget,
    (left, top, right, bottom): (f64, f64, f64, f64),
    scale: f32,
) -> Result<()> {
    let scale = scale.max(1.0);
    let pad = SELECTION_PAD * scale;
    let line = SELECTION_LINE * scale;
    let frame = D2D_RECT_F {
        left: left.min(right) as f32 - pad,
        top: top.min(bottom) as f32 - pad,
        right: left.max(right) as f32 + pad,
        bottom: top.max(bottom) as f32 + pad,
    };
    let half = SELECTION_HANDLE * scale / 2.0;
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ（draw_annotations）
    unsafe {
        let brush = &target.CreateSolidColorBrush(&rgb(0xFF_FF_FF, 1.0), None)?;
        target.DrawRectangle(&frame, brush, SELECTION_HALO * scale, None);
        brush.SetColor(&rgb(SELECTION_COLOR, 1.0));
        target.DrawRectangle(&frame, brush, line, None);
        for (x, y) in [
            (frame.left, frame.top),
            (frame.right, frame.top),
            (frame.left, frame.bottom),
            (frame.right, frame.bottom),
        ] {
            let handle = D2D_RECT_F {
                left: x - half,
                top: y - half,
                right: x + half,
                bottom: y + half,
            };
            brush.SetColor(&rgb(0xFF_FF_FF, 1.0));
            target.FillRectangle(&handle, brush);
            brush.SetColor(&rgb(SELECTION_COLOR, 1.0));
            target.DrawRectangle(&handle, brush, line, None);
        }
    }
    Ok(())
}

/// 範囲で選んでいる間の範囲（描く先の座標）。選択の印と同じ色の枠と、薄く塗った中。
fn draw_selection_range(
    target: &ID2D1RenderTarget,
    (left, top, right, bottom): (f64, f64, f64, f64),
    scale: f32,
) -> Result<()> {
    let frame = D2D_RECT_F {
        left: left as f32,
        top: top as f32,
        right: right as f32,
        bottom: bottom as f32,
    };
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ（draw_annotations）
    unsafe {
        let brush =
            &target.CreateSolidColorBrush(&rgb(SELECTION_COLOR, SELECTION_RANGE_FILL), None)?;
        target.FillRectangle(&frame, brush);
        brush.SetColor(&rgb(SELECTION_COLOR, 1.0));
        target.DrawRectangle(&frame, brush, SELECTION_LINE * scale.max(1.0), None);
    }
    Ok(())
}

/// 範囲で選んでいる間の範囲の中を塗る不透明度。
const SELECTION_RANGE_FILL: f32 = 0.15;

/// 書き込みの色（不透明）。
fn color_of(annotation: &Annotation) -> D2D1_COLOR_F {
    rgb_color(annotation.style.color)
}

/// 打っている文字のキャレットの太さ（96 DPI での画素。`AnnotationLayer::scale` で換算する）。
const CARET_WIDTH: f32 = 2.0;

/// 書き込みを 1 つずつ描く道具一式。BeginDraw と EndDraw の間だけ使う。
struct Pen<'a> {
    target: &'a ID2D1RenderTarget,
    factory: &'a ID2D1Factory,
    /// 文字を描く DirectWrite。文字の書き込みがなければ None。
    dwrite: Option<&'a IDWriteFactory>,
    brush: &'a ID2D1SolidColorBrush,
    /// 端と折れ目を丸める線の形。
    round: &'a ID2D1StrokeStyle,
    /// 元の画像の画素から、画像の左上を原点とした座標への変換と、描く先での画像の左上。
    view: View,
    origin: (f64, f64),
}

impl Pen<'_> {
    /// `shape` を描く。`at` は元の画像の画素 → 描く先の座標、`width` は描く先での太さ。
    fn draw(&self, shape: &Shape, at: &dyn Fn(Point) -> (f64, f64), width: f64) -> Result<()> {
        let (target, brush) = (self.target, self.brush);
        let stroke = width as f32;
        // SAFETY: Pen は BeginDraw と EndDraw の間だけ作られる（draw_annotations）
        unsafe {
            match shape {
                Shape::Rect { from, to } => {
                    let ((x0, y0), (x1, y1)) = (at(*from), at(*to));
                    let rect = D2D_RECT_F {
                        left: x0.min(x1) as f32,
                        top: y0.min(y1) as f32,
                        right: x0.max(x1) as f32,
                        bottom: y0.max(y1) as f32,
                    };
                    target.DrawRectangle(&rect, brush, stroke, None);
                }
                Shape::Ellipse { from, to } => {
                    let ((x0, y0), (x1, y1)) = (at(*from), at(*to));
                    let ellipse = D2D1_ELLIPSE {
                        point: vec2(((x0 + x1) / 2.0, (y0 + y1) / 2.0)),
                        radiusX: ((x1 - x0).abs() / 2.0) as f32,
                        radiusY: ((y1 - y0).abs() / 2.0) as f32,
                    };
                    target.DrawEllipse(&ellipse, brush, stroke, None);
                }
                Shape::Line { from, to } => {
                    target.DrawLine(vec2(at(*from)), vec2(at(*to)), brush, stroke, self.round);
                }
                Shape::Arrow { from, to } => {
                    let (from, to) = (at(*from), at(*to));
                    let Some(head) = arrow_head(from, to, width) else {
                        return Ok(());
                    };
                    target.DrawLine(vec2(from), vec2(head.base), brush, stroke, self.round);
                    let path = self.factory.CreatePathGeometry()?;
                    let sink = path.Open()?;
                    sink.BeginFigure(vec2(head.tip), D2D1_FIGURE_BEGIN_FILLED);
                    sink.AddLines(&[vec2(head.left), vec2(head.right)]);
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    sink.Close()?;
                    target.FillGeometry(&path, brush, None);
                }
                Shape::Freehand { points } => {
                    let Some((first, rest)) = points.split_first() else {
                        return Ok(());
                    };
                    let path = self.factory.CreatePathGeometry()?;
                    let sink = path.Open()?;
                    sink.BeginFigure(vec2(at(*first)), D2D1_FIGURE_BEGIN_HOLLOW);
                    let rest: Vec<Vector2> = rest.iter().map(|p| vec2(at(*p))).collect();
                    sink.AddLines(&rest);
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    sink.Close()?;
                    target.DrawGeometry(&path, brush, stroke, self.round);
                }
                Shape::Text { at, text, font } => {
                    if let Some((layout, origin)) = self.text_layout(*at, text, font)? {
                        target.DrawTextLayout(origin, &layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
        }
        Ok(())
    }

    /// `text` を `font` で `at` の画素の左上から描く並び（DirectWrite）と、描く先での左上。DirectWrite が
    /// なければ None。大きさは表示の倍率に合わせて伸び縮みさせる。
    fn text_layout(
        &self,
        at: Point,
        text: &str,
        font: &Font,
    ) -> Result<Option<(IDWriteTextLayout, Vector2)>> {
        let Some(dwrite) = self.dwrite else {
            return Ok(None);
        };
        let size = self.view.width(font.size).max(1.0) as f32;
        let family: Vec<u16> = font.family.encode_utf16().chain([0]).collect();
        let text: Vec<u16> = text.encode_utf16().collect();
        let (x, y) = self.view.point(at);
        let origin = Vector2 {
            X: (x - self.view.scale_x / 2.0 + self.origin.0) as f32,
            Y: (y - self.view.scale_y / 2.0 + self.origin.1) as f32,
        };
        // SAFETY: 文字列はこの関数の中で生きている（family は 0 で終わる）
        unsafe {
            let format = dwrite.CreateTextFormat(
                windows::core::PCWSTR(family.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!(""),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            // 行の高さを core の見積もり（当たり判定の矩形）に揃える。ベースラインは行の高さの 8 割
            let line = size * LINE_HEIGHT as f32;
            format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, line * 0.8)?;
            let layout = dwrite.CreateTextLayout(&text, &format, 1.0e6, 1.0e6)?;
            Ok(Some((layout, origin)))
        }
    }

    /// 打っている文字を、変換中の文字に下線を引いてつなげて描き、キャレットを描く（太さは `scale` 倍）。
    /// キャレットの矩形（描く先の座標）を返す。
    fn draw_typing(&self, typing: TypingLayer, scale: f32) -> Result<Option<Rect>> {
        let Shape::Text { at, text, font } = &typing.annotation.shape else {
            return Ok(None);
        };
        let typed = text.encode_utf16().count();
        let composed = typing.composition.encode_utf16().count();
        let whole = format!("{text}{}", typing.composition);
        let Some((layout, origin)) = self.text_layout(*at, &whole, font)? else {
            return Ok(None);
        };
        let caret_at = typed + typing.cursor.min(composed);
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        // SAFETY: Pen は BeginDraw と EndDraw の間だけ作られる（draw_annotations）
        unsafe {
            if composed > 0 {
                layout.SetUnderline(
                    true,
                    DWRITE_TEXT_RANGE {
                        startPosition: typed as u32,
                        length: composed as u32,
                    },
                )?;
            }
            layout.HitTestTextPosition(caret_at as u32, false, &mut x, &mut y, &mut metrics)?;
            self.target
                .DrawTextLayout(origin, &layout, self.brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            let caret = D2D_RECT_F {
                left: origin.X + x,
                top: origin.Y + y,
                right: origin.X + x + CARET_WIDTH * scale.max(1.0),
                bottom: origin.Y + y + metrics.height.max(1.0),
            };
            self.target.FillRectangle(&caret, self.brush);
            Ok(Some(Rect::new(
                caret.left.floor() as i32,
                caret.top.floor() as i32,
                caret.right.ceil() as i32,
                caret.bottom.ceil() as i32,
            )))
        }
    }
}

/// Direct2D の点。
fn vec2((x, y): (f64, f64)) -> Vector2 {
    Vector2 {
        X: x as f32,
        Y: y as f32,
    }
}

/// トリムの範囲の外（画像の中だけ）を暗くし、範囲の縁に 1px の線を引く。十字線（画像の中だけ）とつまみを重ねる。
fn draw_trim_overlay(
    target: &ID2D1RenderTarget,
    image_rect: Rect,
    trim: &TrimOverlay,
) -> Result<()> {
    // 範囲の縁の線、十字線、つまみの縁取りは、DPI によらず物理ピクセルで 1px にする（範囲選択の画面の精密
    // モードと同じ）。画素の境目を指す細い線なので、太らせると選んでいる画素が隠れる。つまみの大きさは core が
    // DPI で換算して渡す
    let selection = trim.selection.and_then(|r| r.intersect(&image_rect));
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ
    unsafe {
        let shade = target.CreateSolidColorBrush(&rgb(0, TRIM_SHADE_ALPHA), None)?;
        let edge = target.CreateSolidColorBrush(&rgb(TRIM_EDGE_COLOR, 1.0), None)?;
        match selection {
            None => target.FillRectangle(&to_rect_f(image_rect), &shade),
            Some(selection) => {
                for strip in ring(image_rect, selection) {
                    if !strip.is_empty() {
                        target.FillRectangle(&to_rect_f(strip), &shade);
                    }
                }
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
        }
        if let Some((at, color)) = trim.crosshair {
            let brush = target.CreateSolidColorBrush(&rgb_color(color), None)?;
            let lines = [
                Rect::new(image_rect.left, at.y, image_rect.right, at.y + 1),
                Rect::new(at.x, image_rect.top, at.x + 1, image_rect.bottom),
            ];
            for line in lines.iter().filter_map(|l| l.intersect(&image_rect)) {
                target.FillRectangle(&to_rect_f(line), &brush);
            }
        }
        // つまみは黒で縁取った白い四角
        let outline = target.CreateSolidColorBrush(&rgb(0, 1.0), None)?;
        for &handle in &trim.handles {
            target.FillRectangle(&to_rect_f(handle.inflate(1)), &outline);
            target.FillRectangle(&to_rect_f(handle), &edge);
        }
    }
    Ok(())
}

/// トリムの拡大鏡: カーソルの下の画素のまわりを拡大し、その下に座標とサイズ、色コードを並べる。置き場所は
/// core の `magnifier::place` で、ウィンドウ（`window`）の中に収める。
fn draw_trim_magnifier(
    target: &ID2D1RenderTarget,
    dwrite: &IDWriteFactory,
    image: &Image,
    window: Rect,
    m: &TrimMagnifier,
    scale: f32,
) -> Result<()> {
    let color_at = |p: Point| {
        let inside =
            (0..image.width() as i32).contains(&p.x) && (0..image.height() as i32).contains(&p.y);
        inside.then(|| image.pixel(p.x as u32, p.y as u32))
    };
    let Some(under) = color_at(m.pixel) else {
        return Ok(());
    };
    let scaled = |len: i32| (len as f32 * scale).round() as i32;
    let cell = scaled(magnifier::ZOOM).max(1);
    let side = cell * magnifier::SPAN;
    let texts: Vec<String> = m
        .label
        .clone()
        .into_iter()
        .chain([magnifier::color_code(under)])
        .collect();
    let font_scale = scale.max(1.0);
    let pad_x = TRIM_LABEL_PADDING_X * font_scale;
    let pad_y = TRIM_LABEL_PADDING_Y * font_scale;
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ。文字列と書式はこの関数の中で生きている
    unsafe {
        let format = dwrite.CreateTextFormat(
            w!("Segoe UI"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            TRIM_LABEL_FONT * font_scale,
            w!(""),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let mut measured = Vec::with_capacity(texts.len());
        for text in &texts {
            let text: Vec<u16> = text.encode_utf16().collect();
            let layout = dwrite.CreateTextLayout(&text, &format, 4096.0, 1024.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let width = (metrics.width + pad_x * 2.0).ceil() as i32;
            let height = (metrics.height + pad_y * 2.0).ceil() as i32;
            measured.push((text, width, height));
        }
        let width = measured.iter().map(|&(_, w, _)| w).fold(side, i32::max);
        let height = side + measured.iter().map(|&(_, _, h)| h).sum::<i32>();
        let panel = magnifier::place(m.cursor, width, height, scaled(magnifier::OFFSET), window);
        let area = Rect::new(panel.left, panel.top, panel.left + side, panel.top + side);

        let black = target.CreateSolidColorBrush(&rgb(0, 1.0), None)?;
        let white = target.CreateSolidColorBrush(&rgb(0xFF_FF_FF, 1.0), None)?;
        let cell_brush = target.CreateSolidColorBrush(&rgb(0, 1.0), None)?;
        target.FillRectangle(&to_rect_f(area.inflate(1)), &black);
        let source = magnifier::source(m.pixel);
        let mut center = area;
        for row in 0..magnifier::SPAN {
            for col in 0..magnifier::SPAN {
                let at = Point::new(source.left + col, source.top + row);
                let left = area.left + col * cell;
                let top = area.top + row * cell;
                let rect = Rect::new(left, top, left + cell, top + cell);
                if at == m.pixel {
                    center = rect;
                }
                let color = match color_at(at) {
                    Some([b, g, r, _]) => rgb_color(Rgb(r, g, b)),
                    None => rgb(TRIM_MAGNIFIER_OUTSIDE, 1.0),
                };
                cell_brush.SetColor(&color);
                target.FillRectangle(&to_rect_f(rect), &cell_brush);
            }
        }
        for band in ring(center.inflate(2), center.inflate(1)) {
            target.FillRectangle(&to_rect_f(band), &black);
        }
        for band in ring(center.inflate(1), center) {
            target.FillRectangle(&to_rect_f(band), &white);
        }
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let back = target.CreateSolidColorBrush(&rgb(0, ZOOM_LABEL_BACK_ALPHA), None)?;
        let mut top = area.bottom as f32;
        for (text, _, height) in measured {
            let rect = D2D_RECT_F {
                left: panel.left as f32,
                top,
                right: (panel.left + width) as f32,
                bottom: top + height as f32,
            };
            target.FillRectangle(&rect, &back);
            let text_rect = D2D_RECT_F {
                left: rect.left + pad_x,
                top: rect.top + pad_y,
                right: rect.right - pad_x,
                bottom: rect.bottom - pad_y,
            };
            target.DrawText(
                &text,
                &format,
                &text_rect,
                &white,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            top += height as f32;
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

/// ツールバーの色。
const TOOLBAR_BACK: u32 = 0xF7_F7_F7;
const TOOLBAR_EDGE: u32 = 0xBD_BD_BD;
const TOOLBAR_ICON: u32 = 0x33_33_33;
const TOOLBAR_DISABLED: u32 = 0xB4_B4_B4;
const TOOLBAR_CURRENT_BACK: u32 = 0xD6_E8_FA;
const TOOLBAR_CURRENT_EDGE: u32 = 0x1E_88_E5;
const TOOLBAR_DONE: u32 = 0x002E_7D32;
const TOOLBAR_SWATCH_EDGE: u32 = 0x80_80_80;

/// ツールバーの背景とボタンを描く。
fn draw_toolbar(
    target: &ID2D1RenderTarget,
    factory: &ID2D1Factory,
    layout: &Layout,
    state: &EditState,
    annotations: &Annotations,
    scale: f32,
) -> Result<()> {
    let whole = Rect::from_origin_size(Point::default(), layout.size());
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ
    unsafe {
        let brush = target.CreateSolidColorBrush(&rgb(TOOLBAR_BACK, 1.0), None)?;
        target.FillRectangle(&to_rect_f(whole), &brush);
        brush.SetColor(&rgb(TOOLBAR_EDGE, 1.0));
        for strip in ring(whole, whole.inflate(-1)) {
            target.FillRectangle(&to_rect_f(strip), &brush);
        }
        for &(button, rect) in layout.buttons() {
            if button.is_current(state) {
                brush.SetColor(&rgb(TOOLBAR_CURRENT_BACK, 1.0));
                target.FillRectangle(&to_rect_f(rect), &brush);
                brush.SetColor(&rgb(TOOLBAR_CURRENT_EDGE, 1.0));
                for strip in ring(rect, rect.inflate(-1)) {
                    target.FillRectangle(&to_rect_f(strip), &brush);
                }
            }
        }
        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        let drawn = (|| {
            for &(button, rect) in layout.buttons() {
                let icon = if button.is_enabled(state, annotations) {
                    TOOLBAR_ICON
                } else {
                    TOOLBAR_DISABLED
                };
                brush.SetColor(&rgb(icon, 1.0));
                draw_toolbar_button(target, factory, &brush, button, rect, scale)?;
            }
            Ok(())
        })();
        target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        drawn
    }
}

/// ボタン 1 つのアイコンを `brush` の色で描く（完了・色・つまみは自分の色）。
fn draw_toolbar_button(
    target: &ID2D1RenderTarget,
    factory: &ID2D1Factory,
    brush: &ID2D1SolidColorBrush,
    button: Button,
    rect: Rect,
    scale: f32,
) -> Result<()> {
    // アイコンを描く枠（ボタンの内側）の中の (0..1, 0..1) の点
    let inset = 7.0 * scale;
    let (left, top) = (rect.left as f32 + inset, rect.top as f32 + inset);
    let width = rect.width() as f32 - inset * 2.0;
    let height = rect.height() as f32 - inset * 2.0;
    let at = |u: f32, v: f32| Vector2 {
        X: left + u * width,
        Y: top + v * height,
    };
    let stroke = 1.5 * scale;
    let lines = |points: &[(f32, f32)], closed: bool, brush: &ID2D1Brush, stroke: f32| {
        let points: Vec<_> = points.iter().map(|&(u, v)| at(u, v)).collect();
        polyline(target, factory, &points, closed, brush, stroke)
    };
    let plain: &ID2D1Brush = brush;
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ
    unsafe {
        match button {
            Button::Tool(ToolSlot::Select) => lines(
                &[
                    (0.22, 0.02),
                    (0.22, 0.86),
                    (0.42, 0.68),
                    (0.56, 0.98),
                    (0.70, 0.92),
                    (0.56, 0.62),
                    (0.82, 0.62),
                ],
                true,
                plain,
                stroke,
            )?,
            Button::Tool(ToolSlot::Rect) => {
                let r = D2D_RECT_F {
                    left,
                    top: at(0.0, 0.18).Y,
                    right: left + width,
                    bottom: at(0.0, 0.82).Y,
                };
                target.DrawRectangle(&r, brush, stroke, None);
            }
            Button::Tool(ToolSlot::Ellipse) => {
                let ellipse = D2D1_ELLIPSE {
                    point: at(0.5, 0.5),
                    radiusX: width * 0.5,
                    radiusY: height * 0.36,
                };
                target.DrawEllipse(&ellipse, brush, stroke, None);
            }
            Button::Tool(ToolSlot::Arrow) => {
                lines(&[(0.05, 0.95), (0.95, 0.05)], false, plain, stroke)?;
                lines(
                    &[(0.45, 0.05), (0.95, 0.05), (0.95, 0.55)],
                    false,
                    plain,
                    stroke,
                )?;
            }
            Button::Tool(ToolSlot::Line) => {
                lines(&[(0.05, 0.95), (0.95, 0.05)], false, plain, stroke)?;
            }
            Button::Tool(ToolSlot::Text) => {
                let thick = 2.0 * scale;
                lines(&[(0.1, 0.08), (0.9, 0.08)], false, plain, thick)?;
                lines(&[(0.5, 0.08), (0.5, 0.95)], false, plain, thick)?;
            }
            Button::Tool(ToolSlot::Freehand) => lines(
                &[
                    (0.0, 0.75),
                    (0.15, 0.35),
                    (0.3, 0.25),
                    (0.42, 0.55),
                    (0.55, 0.8),
                    (0.7, 0.72),
                    (0.82, 0.38),
                    (1.0, 0.22),
                ],
                false,
                plain,
                stroke,
            )?,
            Button::Tool(ToolSlot::Eraser) => {
                // 斜めに置いた消しゴム（先の 1/4 に仕切り）と、その下の線
                lines(
                    &[(0.30, 0.92), (0.02, 0.64), (0.58, 0.08), (0.86, 0.36)],
                    true,
                    plain,
                    stroke,
                )?;
                lines(&[(0.16, 0.50), (0.44, 0.78)], false, plain, stroke)?;
                lines(&[(0.30, 0.92), (0.98, 0.92)], false, plain, stroke)?;
            }
            Button::Color(color) => {
                let swatch = to_rect_f(rect.inflate(-(6.0 * scale).round() as i32));
                let fill = target.CreateSolidColorBrush(&rgb_color(color), None)?;
                target.FillRectangle(&swatch, &fill);
                // 白でも見えるように縁を引く
                fill.SetColor(&rgb(TOOLBAR_SWATCH_EDGE, 1.0));
                target.DrawRectangle(&swatch, &fill, 1.0, None);
            }
            Button::Width(w) => {
                // 太さはスクラップの画素なので、DPI の倍率だけを掛けた太さの線で見せる
                let thick = (w as f32 * scale).min(height);
                let y = top + height / 2.0;
                let from = Vector2 { X: left, Y: y };
                let to = Vector2 {
                    X: left + width,
                    Y: y,
                };
                target.DrawLine(from, to, brush, thick, None);
            }
            Button::FontSize(size) => {
                // 文字の大きさは「A」の高さで見せる（いちばん大きい 32 で枠いっぱい。小さくても読める高さは残す）
                let h = (size as f32 / 32.0).clamp(0.3, 1.0);
                let (bottom, top) = (0.5 + h / 2.0, 0.5 - h / 2.0);
                let half = h * 0.4;
                lines(
                    &[(0.5 - half, bottom), (0.5, top), (0.5 + half, bottom)],
                    false,
                    plain,
                    stroke,
                )?;
                let bar = bottom - h * 0.38;
                let bar_half = half * 0.62;
                lines(
                    &[(0.5 - bar_half, bar), (0.5 + bar_half, bar)],
                    false,
                    plain,
                    stroke,
                )?;
            }
            Button::Undo | Button::Redo => {
                // 曲がった矢印: 元に戻すは左へ、やり直すは右へ戻ってくる（左右を映した形）
                let mirror = |(u, v): (f32, f32)| match button {
                    Button::Redo => (1.0 - u, v),
                    _ => (u, v),
                };
                let curve: Vec<_> = [
                    (0.12, 0.42),
                    (0.55, 0.42),
                    (0.78, 0.48),
                    (0.92, 0.64),
                    (0.88, 0.82),
                    (0.72, 0.94),
                    (0.5, 0.96),
                ]
                .map(mirror)
                .to_vec();
                lines(&curve, false, plain, stroke)?;
                let head: Vec<_> = [(0.36, 0.16), (0.12, 0.42), (0.36, 0.68)]
                    .map(mirror)
                    .to_vec();
                lines(&head, false, plain, stroke)?;
            }
            Button::Delete => {
                // ごみ箱: ふた、取っ手、胴、縦の筋
                lines(&[(0.05, 0.2), (0.95, 0.2)], false, plain, stroke)?;
                lines(
                    &[(0.35, 0.2), (0.38, 0.04), (0.62, 0.04), (0.65, 0.2)],
                    false,
                    plain,
                    stroke,
                )?;
                lines(
                    &[(0.15, 0.2), (0.22, 0.98), (0.78, 0.98), (0.85, 0.2)],
                    false,
                    plain,
                    stroke,
                )?;
                lines(&[(0.4, 0.38), (0.42, 0.8)], false, plain, stroke)?;
                lines(&[(0.6, 0.38), (0.58, 0.8)], false, plain, stroke)?;
            }
            Button::Done => {
                let done = target.CreateSolidColorBrush(&rgb(TOOLBAR_DONE, 1.0), None)?;
                let done: &ID2D1Brush = &done;
                lines(
                    &[(0.05, 0.55), (0.38, 0.85), (0.95, 0.15)],
                    false,
                    done,
                    2.2 * scale,
                )?;
            }
            Button::Grip => {
                let dot = target.CreateSolidColorBrush(&rgb(TOOLBAR_DISABLED, 1.0), None)?;
                let cx = rect.left as f32 + rect.width() as f32 / 2.0;
                let cy = rect.top as f32 + rect.height() as f32 / 2.0;
                let step = 4.5 * scale;
                for dx in [-0.5, 0.5] {
                    for dy in [-1.0, 0.0, 1.0] {
                        let ellipse = D2D1_ELLIPSE {
                            point: Vector2 {
                                X: cx + dx * step,
                                Y: cy + dy * step,
                            },
                            radiusX: 1.4 * scale,
                            radiusY: 1.4 * scale,
                        };
                        target.FillEllipse(&ellipse, &dot);
                    }
                }
            }
        }
    }
    Ok(())
}

/// 点を順に結んだ線を引く。`closed` なら最後の点から最初の点へも結ぶ。
fn polyline(
    target: &ID2D1RenderTarget,
    factory: &ID2D1Factory,
    points: &[Vector2],
    closed: bool,
    brush: &ID2D1Brush,
    stroke: f32,
) -> Result<()> {
    let Some((&first, rest)) = points.split_first() else {
        return Ok(());
    };
    // SAFETY: BeginDraw と EndDraw の間で呼ぶ。シンクは Close してから描く
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(first, D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLines(rest);
        sink.EndFigure(if closed {
            D2D1_FIGURE_END_CLOSED
        } else {
            D2D1_FIGURE_END_OPEN
        });
        sink.Close()?;
        target.DrawGeometry(&path, brush, stroke, None);
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

/// 設定の色を、不透明の色にする。
fn rgb_color(color: Rgb) -> D2D1_COLOR_F {
    rgb(color.to_rrggbb(), 1.0)
}

fn to_rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.left as f32,
        top: r.top as f32,
        right: r.right as f32,
        bottom: r.bottom as f32,
    }
}
