//! 描き手（`Renderer`）の使い回しのテスト。1 つの描き手で続けて描いた結果が、毎回新しく作った描き手で描いた結果と
//! 画素で一致することを確かめる（#165 のシーム 2）。

use super::*;

use shunsoku_core::annotate::Style;
use shunsoku_core::scrap::Border;

/// 一色の画像。`bgra` はアルファを掛けない BGRA。
fn solid(size: Size, bgra: [u8; 4]) -> Image {
    let pixels = bgra.repeat(size.width as usize * size.height as usize);
    Image::from_bgra(size, pixels).unwrap()
}

/// 左半分が `left`、右半分が透明な画像（透明な部分のある画像）。
fn half_transparent(size: Size, left: [u8; 4]) -> Image {
    let mut pixels = Vec::new();
    for _ in 0..size.height {
        for x in 0..size.width {
            let px = if x < size.width / 2 { left } else { [0; 4] };
            pixels.extend_from_slice(&px);
        }
    }
    Image::from_bgra(size, pixels).unwrap()
}

/// 線・矢印・フリーハンド・文字の書き込み。
fn annotations() -> Vec<Annotation> {
    let style = |color| Style { color, width: 3 };
    vec![
        Annotation {
            shape: Shape::Line {
                from: Point::new(2, 2),
                to: Point::new(30, 20),
            },
            style: style(Rgb(0xFF, 0, 0)),
        },
        Annotation {
            shape: Shape::Arrow {
                from: Point::new(5, 35),
                to: Point::new(30, 10),
            },
            style: style(Rgb(0, 0x80, 0xFF)),
        },
        Annotation {
            shape: Shape::Freehand {
                points: vec![
                    Point::new(10, 30),
                    Point::new(14, 26),
                    Point::new(20, 31),
                    Point::new(27, 24),
                ],
            },
            style: style(Rgb(0, 0xC0, 0)),
        },
        Annotation {
            shape: Shape::Text {
                at: Point::new(4, 8),
                text: "Ab\nあ".to_owned(),
                font: Font::default(),
            },
            style: style(Rgb(0x20, 0x20, 0x20)),
        },
    ]
}

/// 太い枠と影のある見た目。
fn framed() -> Appearance {
    Appearance {
        border: Border {
            width: 2,
            color: 0x30_60_90,
        },
        show_border: true,
        shadow: true,
        ..Appearance::default()
    }
}

/// `image` を書き込みと枠・影を付けて、表示の大きさ `display` で `renderer` で新しい描き先に描いた画素。
fn paint(
    renderer: &mut Renderer,
    image: &CachedImage,
    display: Size,
    hit_alpha: Option<u8>,
) -> Image {
    paint_on(&RefCell::new(None), renderer, image, display, hit_alpha)
}

/// `paint` と同じものを、描き先 `canvas` を使い回して（大きさが違えば作り直して）描いた画素。
fn paint_on(
    canvas: &RefCell<Option<Canvas>>,
    renderer: &mut Renderer,
    image: &CachedImage,
    display: Size,
    hit_alpha: Option<u8>,
) -> Image {
    let appearance = framed();
    let margin = appearance.margin() as u32;
    let window = Size::new(display.width + margin * 2, display.height + margin * 2);
    let canvas = Canvas::reuse(canvas, window).unwrap();
    let canvas = &*canvas;
    let annotations = annotations();
    let size = image.image().size();
    let overlays = Overlays {
        annotations: AnnotationLayer {
            annotations: &annotations,
            view: View {
                left: 0.0,
                top: 0.0,
                scale_x: f64::from(display.width) / f64::from(size.width),
                scale_y: f64::from(display.height) / f64::from(size.height),
            },
            selected: &[],
            selection_range: None,
            typing: None,
            scale: 1.0,
        },
        label: None,
        trim: None,
    };
    renderer
        .paint_scrap(canvas, image, display, &appearance, hit_alpha, overlays)
        .unwrap();
    canvas.to_image().unwrap()
}

#[test]
fn reused_renderer_paints_the_same_pixels_as_a_new_one() {
    let small = Size::new(48, 40);
    let shrunk = Size::new(72, 48);
    let enlarged = Size::new(80, 112);
    let hit_alpha = Appearance::default().hit_alpha();
    // 並び: 同じ画像（同じ作り置きで 2 回）→ 別の画像 → 大きさの違う画像（縮小して描く）→ 透明な部分のある
    // 画像（拡大して描き、透明な白紙スクラップの下敷きを敷く）
    let first = CachedImage::new(solid(small, [0x10, 0x80, 0xF0, 0xFF]));
    let other = CachedImage::new(solid(small, [0xF0, 0x20, 0x40, 0xFF]));
    let larger = CachedImage::new(solid(Size::new(96, 64), [0x60, 0xD0, 0x30, 0xFF]));
    let see_through = CachedImage::new(half_transparent(
        Size::new(40, 56),
        [0x00, 0x00, 0xFF, 0x80],
    ));
    let steps = [
        (&first, small, None),
        (&first, small, None),
        (&other, small, None),
        (&larger, shrunk, None),
        (&see_through, enlarged, Some(hit_alpha)),
    ];

    let mut reused = Renderer::new().unwrap();
    for (i, (image, display, alpha)) in steps.iter().enumerate() {
        let got = paint(&mut reused, image, *display, *alpha);
        let fresh = CachedImage::new(image.image().clone());
        let want = paint(&mut Renderer::new().unwrap(), &fresh, *display, *alpha);
        assert!(
            got == want,
            "step {i}: the reused renderer drew other pixels"
        );
        // 描けていること（画像の右下の隅は書き込みの外）
        let margin = framed().margin() as u32;
        let corner = got.pixel(margin + display.width - 2, margin + display.height - 2);
        let expected = match i {
            0 | 1 => [0x10, 0x80, 0xF0, 0xFF],
            2 => [0xF0, 0x20, 0x40, 0xFF],
            3 => [0x60, 0xD0, 0x30, 0xFF],
            // 透明な部分には当たり判定の下敷き（黒）だけが残る
            _ => [0, 0, 0, hit_alpha],
        };
        assert_eq!(corner, expected, "step {i}");
    }

    // 描き手を作り直したあと（`D2DERR_RECREATE_TARGET` で捨てたあとなど）も、同じ画像を新しい描き手で描ける
    let mut recreated = Renderer::new().unwrap();
    let got = paint(&mut recreated, &first, small, None);
    let fresh = CachedImage::new(first.image().clone());
    let want = paint(&mut Renderer::new().unwrap(), &fresh, small, None);
    assert!(got == want, "a recreated renderer drew other pixels");
}

#[test]
fn reused_canvas_paints_the_same_pixels_as_a_new_one() {
    let large = Size::new(96, 80);
    let small = Size::new(40, 32);
    let hit_alpha = Appearance::default().hit_alpha();
    // 並び: 大 → 同じ大きさで別の画像 → 小 → 大（作り直した描き先）→ 透明な部分のある画像（前に描いた不透明な
    // 画素が残らないこと）
    let first = CachedImage::new(solid(large, [0x10, 0x80, 0xF0, 0xFF]));
    let other = CachedImage::new(solid(large, [0xF0, 0x20, 0x40, 0xFF]));
    let see_through = CachedImage::new(half_transparent(large, [0x00, 0x00, 0xFF, 0x80]));
    let steps = [
        (&first, large, None),
        (&other, large, None),
        (&first, small, None),
        (&other, large, None),
        (&see_through, large, Some(hit_alpha)),
    ];

    let mut renderer = Renderer::new().unwrap();
    let canvas = RefCell::new(None);
    for (i, (image, display, alpha)) in steps.iter().enumerate() {
        let got = paint_on(&canvas, &mut renderer, image, *display, *alpha);
        let want = paint(&mut renderer, image, *display, *alpha);
        assert!(got == want, "step {i}: the reused canvas drew other pixels");
        let margin = framed().margin() as u32;
        assert_eq!(
            got.size(),
            Size::new(display.width + margin * 2, display.height + margin * 2),
            "step {i}"
        );
        // 画像の右下の隅（書き込みの外）
        let corner = got.pixel(margin + display.width - 2, margin + display.height - 2);
        let expected = match i {
            0 | 2 => [0x10, 0x80, 0xF0, 0xFF],
            1 | 3 => [0xF0, 0x20, 0x40, 0xFF],
            // 透明な部分には当たり判定の下敷き（黒）だけが残る
            _ => [0, 0, 0, hit_alpha],
        };
        assert_eq!(corner, expected, "step {i}");
    }
}

#[test]
fn a_canvas_borrowed_by_an_outer_paint_does_not_stop_a_nested_one() {
    // 出している間（`UpdateLayeredWindow` の中）に届いたメッセージで描き直すと、描き先はまだ借りられている
    let size = Size::new(48, 40);
    let image = CachedImage::new(solid(size, [0x10, 0x80, 0xF0, 0xFF]));
    let mut renderer = Renderer::new().unwrap();
    let canvas = RefCell::new(None);
    let want = paint_on(&canvas, &mut renderer, &image, size, None);

    let outer = Canvas::reuse(&canvas, want.size()).unwrap();
    let got = paint_on(&canvas, &mut renderer, &image, size, None);
    drop(outer);
    assert!(got == want, "the nested paint drew other pixels");
    // 外の描き直しが借りていた描き先は、そのあとも使い回せる
    let again = paint_on(&canvas, &mut renderer, &image, size, None);
    assert!(again == want, "the reused canvas drew other pixels");
}

#[test]
fn dropping_the_renderer_returns_the_bitmaps_kept_for_it() {
    // 描き手を作り直したとき（`D2DERR_RECREATE_TARGET` で捨てたときなど）、描き直していないスクラップにも古い
    // 描き手のビットマップを残さない
    let size = Size::new(48, 40);
    let drawn = CachedImage::new(solid(size, [0x10, 0x80, 0xF0, 0xFF]));
    let other = CachedImage::new(solid(size, [0xF0, 0x20, 0x40, 0xFF]));
    let mut renderer = Renderer::new().unwrap();
    paint(&mut renderer, &drawn, size, None);
    paint(&mut renderer, &other, size, None);
    assert!(drawn.keeps_bitmap() && other.keeps_bitmap());

    drop(renderer);
    assert!(
        !drawn.keeps_bitmap(),
        "the dropped renderer's bitmap is kept"
    );
    assert!(
        !other.keeps_bitmap(),
        "the dropped renderer's bitmap is kept"
    );
}
