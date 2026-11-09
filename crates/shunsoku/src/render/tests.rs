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

/// `annotations` を書き込みに重ねて（選んでいるものの印は `selected`。キャレット・範囲選択の枠・トリム・倍率は
/// なし）、枠と影を付けて描き先 `canvas` に描き、返った値と描き先の画素を返す。画像の 1 画素は表示で `zoom` 倍。
fn paint_drawing(
    canvas: &RefCell<Option<Canvas>>,
    renderer: &mut Renderer,
    image: &CachedImage,
    zoom: u32,
    hit_alpha: Option<u8>,
    annotations: &[Annotation],
    selected: &[Rect],
) -> (Painted, Image) {
    let appearance = framed();
    let size = image.image().size();
    let display = Size::new(size.width * zoom, size.height * zoom);
    let margin = appearance.margin() as u32;
    let window = Size::new(display.width + margin * 2, display.height + margin * 2);
    let canvas = Canvas::reuse(canvas, window).unwrap();
    let canvas = &*canvas;
    let overlays = Overlays {
        annotations: AnnotationLayer {
            annotations,
            view: View {
                left: 0.0,
                top: 0.0,
                scale_x: f64::from(zoom),
                scale_y: f64::from(zoom),
            },
            selected,
            selection_range: None,
            typing: None,
            scale: 1.0,
        },
        label: None,
        trim: None,
    };
    let painted = renderer
        .paint_scrap(canvas, image, display, &appearance, hit_alpha, overlays)
        .unwrap();
    (painted, canvas.to_image().unwrap())
}

/// `paint_drawing` と同じものを、新しい描き先にウィンドウ全体で描いた画素。
fn paint_whole(
    renderer: &mut Renderer,
    image: &CachedImage,
    zoom: u32,
    hit_alpha: Option<u8>,
    annotations: &[Annotation],
    selected: &[Rect],
) -> Image {
    let fresh = RefCell::new(None);
    let (painted, image) = paint_drawing(
        &fresh,
        renderer,
        image,
        zoom,
        hit_alpha,
        annotations,
        selected,
    );
    assert_eq!(painted.dirty, Some(whole_window(&image)), "a new canvas");
    image
}

/// ウィンドウ全体の矩形。
fn whole_window(image: &Image) -> Rect {
    Rect::from_origin_size(Point::default(), image.size())
}

fn stroke(shape: Shape, color: Rgb, width: u32) -> Annotation {
    Annotation {
        shape,
        style: Style { color, width },
    }
}

fn freehand(points: &[(i32, i32)], width: u32) -> Annotation {
    let points = points.iter().map(|&(x, y)| Point::new(x, y)).collect();
    stroke(Shape::Freehand { points }, Rgb(0xE0, 0x20, 0x20), width)
}

fn arrow(from: (i32, i32), to: (i32, i32), width: u32) -> Annotation {
    let (from, to) = (Point::new(from.0, from.1), Point::new(to.0, to.1));
    stroke(Shape::Arrow { from, to }, Rgb(0x10, 0x60, 0xE0), width)
}

/// 書き込みを足していく並び（ドラッグ中のフリーハンド・矢印・線・矩形・楕円と、元に戻す、変わらない描き直し）。
/// 太い線、矢印の頭、丸めた端が、前の書き込みと画像の端の近くに来るようにしてある（画像は 120×80）。
fn drawing_steps() -> Vec<Vec<Annotation>> {
    let mut steps: Vec<Vec<Annotation>> = vec![vec![]];
    // 太いフリーハンドを 1 点ずつ伸ばす（前の線と交わり、画像の左上の端で丸めた端が切れる）
    let trail = [
        (1, 1),
        (20, 6),
        (34, 30),
        (12, 40),
        (8, 18),
        (40, 12),
        (60, 50),
    ];
    // （描いているフリーハンドはいつも 2 点から。core の `EditState`）
    for n in 2..=trail.len() {
        steps.push(vec![freehand(&trail[..n], 11)]);
    }
    let done = freehand(&trail, 11);
    // 太い矢印の先を動かす（頭が前の線に重なり、画像の右の端にかかる）
    for to in [(70, 20), (90, 35), (118, 10), (50, 25)] {
        steps.push(vec![done.clone(), arrow((80, 70), to, 9)]);
    }
    let pointed = arrow((80, 70), (50, 25), 9);
    // 太い線を伸ばす（丸めた端が画像の左下の外に出る）
    for to in [(30, 60), (5, 79), (-10, 90)] {
        let line = Shape::Line {
            from: Point::new(100, 40),
            to: Point::new(to.0, to.1),
        };
        steps.push(vec![
            done.clone(),
            pointed.clone(),
            stroke(line, Rgb(0x20, 0xA0, 0x40), 13),
        ]);
    }
    // 矩形と楕円を広げる（画像の右下の端にかかる）
    for (corner, end) in [((70, 60), (20, 20)), ((119, 79), (60, 70))] {
        let rect = Shape::Rect {
            from: Point::new(40, 45),
            to: Point::new(corner.0, corner.1),
        };
        let ellipse = Shape::Ellipse {
            from: Point::new(25, 15),
            to: Point::new(end.0, end.1),
        };
        steps.push(vec![
            done.clone(),
            pointed.clone(),
            stroke(rect, Rgb(0x20, 0xA0, 0x40), 7),
            stroke(ellipse, Rgb(0xA0, 0x20, 0xA0), 6),
        ]);
    }
    // 元に戻す（最後の 2 つを消す）
    steps.push(vec![done.clone(), pointed.clone()]);
    // 変わったものがない
    steps.push(vec![done, pointed]);
    steps
}

#[test]
fn a_dirty_rect_paints_the_same_pixels_as_the_whole_window() {
    let size = Size::new(120, 80);
    let hit_alpha = Appearance::default().hit_alpha();
    let opaque = CachedImage::new(solid(size, [0x10, 0x80, 0xF0, 0xFF]));
    let see_through = CachedImage::new(half_transparent(size, [0x00, 0x00, 0xFF, 0x80]));
    // 等倍・拡大・透明な白紙スクラップの下敷き
    let cases = [
        (&opaque, 1, None),
        (&opaque, 3, None),
        (&see_through, 2, Some(hit_alpha)),
    ];
    let steps = drawing_steps();
    for (c, (image, zoom, alpha)) in cases.into_iter().enumerate() {
        let mut renderer = Renderer::new().unwrap();
        let canvas = RefCell::new(None);
        let mut partial = 0;
        for (i, annotations) in steps.iter().enumerate() {
            let (painted, got) =
                paint_drawing(&canvas, &mut renderer, image, zoom, alpha, annotations, &[]);
            let want = paint_whole(&mut renderer, image, zoom, alpha, annotations, &[]);
            assert!(
                got == want,
                "case {c} step {i}: the dirty rect drew other pixels"
            );
            let window = whole_window(&want);
            match (i, painted.dirty) {
                (0, dirty) => assert_eq!(dirty, Some(window), "case {c}: the first paint"),
                (i, dirty) if i == steps.len() - 1 => {
                    assert_eq!(dirty, None, "case {c}: nothing changed")
                }
                (_, Some(dirty)) if dirty != window => {
                    assert!(
                        window.intersect(&dirty) == Some(dirty),
                        "case {c} step {i}: {dirty:?} is out of the window"
                    );
                    partial += 1;
                }
                _ => {}
            }
        }
        // ドラッグ中は、ウィンドウの一部だけを描き直して渡す
        assert!(
            partial >= steps.len() - 3,
            "case {c}: only {partial} paints were partial"
        );
    }
}

#[test]
fn marks_text_and_other_looks_paint_the_whole_window() {
    let size = Size::new(48, 40);
    let image = CachedImage::new(solid(size, [0x10, 0x80, 0xF0, 0xFF]));
    let other = CachedImage::new(solid(size, [0xF0, 0x20, 0x40, 0xFF]));
    let line = freehand(&[(4, 4), (30, 20)], 5);
    let longer = freehand(&[(4, 4), (30, 20), (40, 30)], 5);
    let text = stroke(
        Shape::Text {
            at: Point::new(6, 24),
            text: "Ab".to_owned(),
            font: Font::default(),
        },
        Rgb(0x20, 0x20, 0x20),
        3,
    );
    let marked = [line.bounds()];
    let no_marks: &[Rect] = &[];
    // 並び: 選択の印 → 印を外す → 文字を足す → 別の画像 → 別の倍率 → 別の当たり判定の下敷き。どれも前と比べて
    // 範囲を正しく出せないので、ウィンドウ全体を描き直す
    /// 画像・倍率・当たり判定の下敷き・書き込み・選択の印。
    type Step<'a> = (
        &'a CachedImage,
        u32,
        Option<u8>,
        Vec<Annotation>,
        &'a [Rect],
    );
    let steps: [Step; 7] = [
        (&image, 1, None, vec![line.clone()], no_marks),
        (&image, 1, None, vec![line.clone()], &marked),
        (&image, 1, None, vec![longer.clone()], no_marks),
        (
            &image,
            1,
            None,
            vec![longer.clone(), text.clone()],
            no_marks,
        ),
        (
            &other,
            1,
            None,
            vec![longer.clone(), text.clone()],
            no_marks,
        ),
        (&other, 2, None, vec![longer.clone()], no_marks),
        (&other, 2, Some(1), vec![longer.clone()], no_marks),
    ];
    let mut renderer = Renderer::new().unwrap();
    let canvas = RefCell::new(None);
    for (i, (image, zoom, alpha, annotations, selected)) in steps.iter().enumerate() {
        let (painted, got) = paint_drawing(
            &canvas,
            &mut renderer,
            image,
            *zoom,
            *alpha,
            annotations,
            selected,
        );
        let want = paint_whole(&mut renderer, image, *zoom, *alpha, annotations, selected);
        assert!(got == want, "step {i}: drew other pixels");
        assert_eq!(painted.dirty, Some(whole_window(&got)), "step {i}");
    }

    // 出せなかったとき（`Canvas::forget`）は、次はウィンドウ全体を描き直す
    let (_, got) = paint_drawing(&canvas, &mut renderer, &other, 2, Some(1), &[], no_marks);
    Canvas::reuse(&canvas, got.size()).unwrap().forget();
    let (painted, got) = paint_drawing(
        &canvas,
        &mut renderer,
        &other,
        2,
        Some(1),
        &[line],
        no_marks,
    );
    assert_eq!(painted.dirty, Some(whole_window(&got)), "after forget");
}
