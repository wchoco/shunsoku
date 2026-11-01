//! 画像。BGRA（1 画素 4 バイト、B, G, R, A の順）、アルファは premultiply しない、上の行から並べる。
//!
//! キャプチャでは、モニタごとに切り抜いた断片（`Fragment`）を `Image::compose` で 1 枚にする。
//! どのモニタにも当たらない部分は透明（アルファ 0）になる。
//! 保存画像は `write_png` で PNG に書き出す。

use crate::geometry::{Point, Rect, Size};

/// BGRA の画像。
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    size: Size,
    pixels: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 画素は大きいので出さない
        f.debug_struct("Image").field("size", &self.size).finish()
    }
}

/// 1 画素のバイト数。
const BPP: usize = 4;

/// 画像の 1 片と、その左上の位置（スクリーン座標）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub at: Point,
    pub image: Image,
}

impl Image {
    /// 全体が透明な画像。
    pub fn transparent(size: Size) -> Image {
        Image {
            size,
            pixels: vec![0; size.width as usize * size.height as usize * BPP],
        }
    }

    /// BGRA のバイト列から作る。長さが合わなければ None。
    pub fn from_bgra(size: Size, pixels: Vec<u8>) -> Option<Image> {
        (pixels.len() == size.width as usize * size.height as usize * BPP)
            .then_some(Image { size, pixels })
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn width(&self) -> u32 {
        self.size.width
    }

    pub fn height(&self) -> u32 {
        self.size.height
    }

    /// BGRA のバイト列。1 行は `width * 4` バイト。
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// (x, y) の画素の BGRA。
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.offset(x, y);
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.size.width as usize + x as usize) * BPP
    }

    /// アルファを premultiply した BGRA のバイト列（Direct2D のビットマップと `UpdateLayeredWindow` 用）。
    pub fn to_premultiplied(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for px in out.chunks_exact_mut(BPP) {
            let a = u16::from(px[3]);
            if a == 255 {
                continue;
            }
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
        out
    }

    /// premultiplied の BGRA（`render` で描いた DIB）から作る。色をアルファで割り戻す。長さが合わなければ None。
    pub fn from_premultiplied(size: Size, mut pixels: Vec<u8>) -> Option<Image> {
        if pixels.len() != size.width as usize * size.height as usize * BPP {
            return None;
        }
        for px in pixels.chunks_exact_mut(BPP) {
            let a = u16::from(px[3]);
            if a == 255 {
                continue;
            }
            for c in &mut px[..3] {
                // 透明（a == 0）の画素の色は 0 にする
                *c = (u16::from(*c) * 255 + a / 2)
                    .checked_div(a)
                    .map_or(0, |v| v.min(255) as u8);
            }
        }
        Some(Image { size, pixels })
    }

    /// PNG（8 bit の RGBA）に書き出す。
    pub fn write_png(&self, out: impl std::io::Write) -> Result<(), png::EncodingError> {
        let mut encoder = png::Encoder::new(out, self.size.width, self.size.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        let mut rgba = self.pixels.clone();
        for px in rgba.chunks_exact_mut(BPP) {
            px.swap(0, 2);
        }
        writer.write_image_data(&rgba)?;
        writer.finish()
    }

    /// PNG のバイト列。
    pub fn to_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        self.write_png(&mut out)?;
        Ok(out)
    }

    /// 1 行 `stride` バイトで並んだ BGRX（画面から書き戻した画像）から作る。アルファはすべて 255 にする。
    /// `data` が足りないか、`stride` が 1 行より短ければ None。
    pub fn from_opaque_rows(size: Size, stride: usize, data: &[u8]) -> Option<Image> {
        let row = size.width as usize * BPP;
        let height = size.height as usize;
        if stride < row || (height > 0 && data.len() < stride * (height - 1) + row) {
            return None;
        }
        let mut pixels = Vec::with_capacity(row * height);
        for y in 0..height {
            pixels.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        for alpha in pixels.iter_mut().skip(3).step_by(BPP) {
            *alpha = 255;
        }
        Some(Image { size, pixels })
    }

    /// `rect`（画像の中の座標）の部分を切り抜く。画像からはみ出した部分は捨てる。
    pub fn crop(&self, rect: Rect) -> Image {
        let whole = Rect::from_origin_size(Point::default(), self.size);
        let Some(area) = whole.intersect(&rect) else {
            return Image::transparent(Size::default());
        };
        let mut out = Image::transparent(area.size());
        out.copy_from(self, area, Point::default());
        out
    }

    /// 断片を `bounds`（スクリーン座標）の 1 枚の画像に並べる。断片のない部分は透明。
    /// `bounds` からはみ出した部分は捨てる。
    pub fn compose(bounds: Rect, fragments: &[Fragment]) -> Image {
        let mut out = Image::transparent(bounds.size());
        for fragment in fragments {
            let placed = Rect::from_origin_size(fragment.at, fragment.image.size());
            let Some(area) = placed.intersect(&bounds) else {
                continue;
            };
            out.copy_from(
                &fragment.image,
                area.offset(-placed.left, -placed.top),
                Point::new(area.left - bounds.left, area.top - bounds.top),
            );
        }
        out
    }

    /// `src` の `from`（`src` の中の座標）を、自分の `to` を左上として写す。範囲は呼び出し側で収めておく。
    fn copy_from(&mut self, src: &Image, from: Rect, to: Point) {
        let len = from.width() as usize * BPP;
        for row in 0..from.height() {
            let s = src.offset(from.left as u32, from.top as u32 + row);
            let d = self.offset(to.x as u32, to.y as u32 + row);
            self.pixels[d..d + len].copy_from_slice(&src.pixels[s..s + len]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [0, 0, 255, 255];
    const BLUE: [u8; 4] = [255, 0, 0, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];

    fn filled(width: u32, height: u32, bgra: [u8; 4]) -> Image {
        let pixels = bgra.repeat(width as usize * height as usize);
        Image::from_bgra(Size::new(width, height), pixels).unwrap()
    }

    fn rows(image: &Image) -> Vec<Vec<[u8; 4]>> {
        (0..image.height())
            .map(|y| (0..image.width()).map(|x| image.pixel(x, y)).collect())
            .collect()
    }

    #[test]
    fn composing_fragments_leaves_the_gap_between_them_transparent() {
        // 左のモニタが x < 0、右のモニタが x >= 2 で、間の 0..2 にはモニタがない
        let image = Image::compose(
            Rect::new(-1, 10, 3, 12),
            &[
                Fragment {
                    at: Point::new(-1, 10),
                    image: filled(1, 2, RED),
                },
                Fragment {
                    at: Point::new(2, 10),
                    image: filled(1, 2, BLUE),
                },
            ],
        );
        assert_eq!(image.size(), Size::new(4, 2));
        assert_eq!(
            rows(&image),
            vec![vec![RED, CLEAR, CLEAR, BLUE], vec![RED, CLEAR, CLEAR, BLUE]]
        );
    }

    #[test]
    fn composing_leaves_the_area_below_a_shorter_fragment_transparent() {
        let image = Image::compose(
            Rect::new(0, 0, 2, 2),
            &[
                Fragment {
                    at: Point::new(0, 0),
                    image: filled(1, 1, RED),
                },
                Fragment {
                    at: Point::new(1, 0),
                    image: filled(1, 2, BLUE),
                },
            ],
        );
        assert_eq!(rows(&image), vec![vec![RED, BLUE], vec![CLEAR, BLUE]]);
    }

    /// 画素ごとに違う値の 3×2 の画像。(x, y) の B が 10*y + x。
    fn numbered() -> Image {
        let mut pixels = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                pixels.extend([10 * y + x, 0, 0, 255]);
            }
        }
        Image::from_bgra(Size::new(3, 2), pixels).unwrap()
    }

    fn blues(image: &Image) -> Vec<Vec<u8>> {
        rows(image)
            .into_iter()
            .map(|row| row.into_iter().map(|p| p[0]).collect())
            .collect()
    }

    #[test]
    fn cropping_keeps_only_the_given_rect() {
        let image = numbered().crop(Rect::new(1, 0, 3, 2));
        assert_eq!(blues(&image), vec![vec![1, 2], vec![11, 12]]);
    }

    #[test]
    fn cropping_outside_the_image_keeps_only_the_overlap() {
        let image = numbered().crop(Rect::new(2, -5, 10, 1));
        assert_eq!(blues(&image), vec![vec![2]]);
        assert_eq!(
            numbered().crop(Rect::new(5, 5, 6, 6)).size(),
            Size::new(0, 0)
        );
    }

    #[test]
    fn rows_read_back_from_the_screen_are_made_opaque_and_their_padding_dropped() {
        // 1 行 2 画素（8 バイト）に 4 バイトの詰め物。画面の画像のアルファは当てにならない
        let data = [
            1, 2, 3, 0, 4, 5, 6, 7, 99, 99, 99, 99, //
            8, 9, 10, 11, 12, 13, 14, 0, 99, 99, 99, 99,
        ];
        let image = Image::from_opaque_rows(Size::new(2, 2), 12, &data).unwrap();
        assert_eq!(
            rows(&image),
            vec![
                vec![[1, 2, 3, 255], [4, 5, 6, 255]],
                vec![[8, 9, 10, 255], [12, 13, 14, 255]],
            ]
        );
    }

    #[test]
    fn premultiplying_scales_colors_by_alpha() {
        let image = Image::from_bgra(
            Size::new(3, 1),
            vec![100, 200, 50, 128, 1, 2, 3, 255, 9, 9, 9, 0],
        )
        .unwrap();
        assert_eq!(
            image.to_premultiplied(),
            vec![50, 100, 25, 128, 1, 2, 3, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn rows_shorter_than_the_size_are_rejected() {
        assert_eq!(Image::from_opaque_rows(Size::new(2, 2), 8, &[0; 15]), None);
        assert_eq!(Image::from_opaque_rows(Size::new(2, 2), 4, &[0; 16]), None);
    }
    /// PNG を読み戻して BGRA の画像にする。
    fn decode_png(bytes: &[u8]) -> Image {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        buf.truncate(info.buffer_size());
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Image::from_bgra(Size::new(info.width, info.height), buf).unwrap()
    }

    #[test]
    fn a_written_png_reads_back_with_the_same_pixels_including_transparent_ones() {
        let image = Image::from_bgra(
            Size::new(3, 2),
            vec![
                1, 2, 3, 255, 0, 0, 0, 0, 40, 50, 60, 128, //
                255, 0, 0, 255, 0, 255, 0, 255, 7, 8, 9, 0,
            ],
        )
        .unwrap();
        let png = image.to_png().unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let back = decode_png(&png);
        assert_eq!(back.size(), Size::new(3, 2));
        assert_eq!(rows(&back), rows(&image));
    }

    #[test]
    fn premultiplied_pixels_are_turned_back_into_straight_alpha() {
        // 描いた DIB（premultiplied）から戻す。不透明と透明はそのまま、半透明は色を割り戻す
        let image = Image::from_premultiplied(
            Size::new(3, 1),
            vec![1, 2, 3, 255, 50, 100, 25, 128, 9, 9, 9, 0],
        )
        .unwrap();
        assert_eq!(
            rows(&image),
            vec![vec![[1, 2, 3, 255], [100, 199, 50, 128], [0, 0, 0, 0]]]
        );
        assert_eq!(Image::from_premultiplied(Size::new(2, 1), vec![0; 4]), None);
    }
}
