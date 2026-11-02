//! マウスカーソルの合成。Desktop Duplication はカーソルを画面の画像に含めず、形と位置を別に返す。
//! カーソルを写す設定（`[general] capture_cursor`）が ON のとき、Win32 層（`grab`）が切り抜く前の
//! 画面の画像に、ここで重ねる。

use crate::geometry::{Point, Rect, Size};

/// カーソルの形の種類（`DXGI_OUTDUPL_POINTER_SHAPE_TYPE`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    /// 1 画素 1 ビットの AND マスクと XOR マスクを縦に重ねたもの（`height` は見える高さの 2 倍）。
    Monochrome,
    /// BGRA。アルファは premultiply していない。
    Color,
    /// BGRA。アルファが 0 の画素は色で置き換え、0xFF の画素は画面の色と XOR する。
    MaskedColor,
}

/// カーソルの形（`GetFramePointerShape` が返すもの）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub width: u32,
    /// `Monochrome` では 2 つのマスクを合わせた高さ。
    pub height: u32,
    /// 1 行のバイト数。
    pub pitch: usize,
    pub data: Vec<u8>,
}

impl Shape {
    /// 画面に出る大きさ。
    pub fn size(&self) -> Size {
        match self.kind {
            ShapeKind::Monochrome => Size::new(self.width, self.height / 2),
            ShapeKind::Color | ShapeKind::MaskedColor => Size::new(self.width, self.height),
        }
    }

    /// 形の (x, y) の画素を、下の画素 `under`（BGRA）に重ねた色。データが足りなければ `under` のまま。
    fn over(&self, x: usize, y: usize, under: [u8; 4]) -> [u8; 4] {
        match self.kind {
            ShapeKind::Monochrome => {
                let bit = |row: usize| {
                    let byte = self.data.get(row * self.pitch + x / 8)?;
                    Some(byte & (0x80 >> (x % 8)) != 0)
                };
                let height = self.height as usize / 2;
                let (Some(and), Some(xor)) = (bit(y), bit(y + height)) else {
                    return under;
                };
                let mut out = if and { under } else { [0, 0, 0, under[3]] };
                if xor {
                    for c in &mut out[..3] {
                        *c ^= 0xFF;
                    }
                }
                out
            }
            ShapeKind::Color => {
                let Some(src) = self.bgra(x, y) else {
                    return under;
                };
                let a = u32::from(src[3]);
                let mut out = under;
                for (o, s) in out[..3].iter_mut().zip(src) {
                    *o = ((u32::from(s) * a + u32::from(*o) * (255 - a) + 127) / 255) as u8;
                }
                out
            }
            ShapeKind::MaskedColor => {
                let Some(src) = self.bgra(x, y) else {
                    return under;
                };
                let mut out = under;
                for (o, s) in out[..3].iter_mut().zip(src) {
                    *o = if src[3] == 0 { s } else { *o ^ s };
                }
                out
            }
        }
    }

    fn bgra(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        let i = y * self.pitch + x * 4;
        self.data.get(i..i + 4)?.try_into().ok()
    }
}

/// カーソルの形と、その左上の位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub shape: Shape,
    /// 形の左上（ホットスポットではない）。`draw` の `origin` と同じ座標系。
    pub at: Point,
}

impl Cursor {
    /// カーソルが覆う範囲。
    pub fn bounds(&self) -> Rect {
        Rect::from_origin_size(self.at, self.shape.size())
    }

    /// BGRA の画素の列 `pixels`（左上が `origin`、大きさ `size`、1 行 `stride` バイト）にカーソルを重ねる。
    /// 範囲の外にはみ出す部分は描かない。画素のアルファは変えない。
    pub fn draw(&self, origin: Point, size: Size, stride: usize, pixels: &mut [u8]) {
        let area = Rect::from_origin_size(origin, size);
        let Some(hit) = self.bounds().intersect(&area) else {
            return;
        };
        for y in hit.top..hit.bottom {
            for x in hit.left..hit.right {
                let i = (y - origin.y) as usize * stride + (x - origin.x) as usize * 4;
                let Some(px) = pixels.get_mut(i..i + 4) else {
                    continue;
                };
                let under: [u8; 4] = [px[0], px[1], px[2], px[3]];
                let out = self.shape.over(
                    (x - self.at.x) as usize,
                    (y - self.at.y) as usize,
                    under,
                );
                px.copy_from_slice(&out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一色で塗った BGRA の画素の列（1 行は `width * 4` バイト）。
    fn filled(size: Size, bgra: [u8; 4]) -> Vec<u8> {
        bgra.repeat(size.width as usize * size.height as usize)
    }

    fn at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = (y * width + x) as usize * 4;
        pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn a_color_cursor_is_alpha_blended_and_clipped_to_the_area() {
        // 2x1 のカラーのカーソル: 左は不透明の赤、右は半透明の白
        let cursor = Cursor {
            shape: Shape {
                kind: ShapeKind::Color,
                width: 2,
                height: 1,
                pitch: 8,
                data: vec![0, 0, 255, 255, 255, 255, 255, 128],
            },
            at: Point::new(9, 5),
        };
        // スクリーン座標で (10,5) から 2x2 の灰色の領域。カーソルの左の画素は領域の外
        let size = Size::new(2, 2);
        let mut pixels = filled(size, [100, 100, 100, 255]);
        cursor.draw(Point::new(10, 5), size, 8, &mut pixels);
        // 100 + (255 - 100) * 128 / 255 = 177.8
        assert_eq!(at(&pixels, 2, 0, 0), [178, 178, 178, 255]);
        assert_eq!(at(&pixels, 2, 1, 0), [100, 100, 100, 255]);
        assert_eq!(at(&pixels, 2, 0, 1), [100, 100, 100, 255]);
    }

    #[test]
    fn a_masked_color_cursor_replaces_where_the_mask_is_off_and_xors_where_it_is_on() {
        let cursor = Cursor {
            shape: Shape {
                kind: ShapeKind::MaskedColor,
                width: 2,
                height: 1,
                pitch: 8,
                // 左: マスク 0 で青に置き換え。右: マスク 0xFF で 0xFF と XOR（反転）
                data: vec![255, 0, 0, 0, 255, 255, 255, 255],
            },
            at: Point::new(0, 0),
        };
        let size = Size::new(2, 1);
        let mut pixels = filled(size, [10, 20, 30, 255]);
        cursor.draw(Point::new(0, 0), size, 8, &mut pixels);
        assert_eq!(at(&pixels, 2, 0, 0), [255, 0, 0, 255]);
        assert_eq!(at(&pixels, 2, 1, 0), [245, 235, 225, 255]);
    }

    #[test]
    fn a_monochrome_cursor_applies_the_and_mask_then_the_xor_mask() {
        // 4x1 の白黒のカーソル。上の行が AND、下の行が XOR（1 画素 1 ビット、左が上位ビット）
        // 画素:    0      1      2      3
        // AND:     1      0      0      1
        // XOR:     0      0      1      1
        // 結果:  そのまま  黒     白     反転
        let cursor = Cursor {
            shape: Shape {
                kind: ShapeKind::Monochrome,
                width: 4,
                height: 2,
                pitch: 4,
                data: vec![0b1001_0000, 0, 0, 0, 0b0011_0000, 0, 0, 0],
            },
            at: Point::new(0, 0),
        };
        assert_eq!(cursor.bounds(), Rect::new(0, 0, 4, 1));
        // 1 行 20 バイト（画像の幅より広い stride）
        let size = Size::new(4, 1);
        let mut pixels = [10, 20, 30, 255].repeat(5);
        cursor.draw(Point::new(0, 0), size, 20, &mut pixels);
        assert_eq!(at(&pixels, 5, 0, 0), [10, 20, 30, 255]);
        assert_eq!(at(&pixels, 5, 1, 0), [0, 0, 0, 255]);
        assert_eq!(at(&pixels, 5, 2, 0), [255, 255, 255, 255]);
        assert_eq!(at(&pixels, 5, 3, 0), [245, 235, 225, 255]);
        assert_eq!(at(&pixels, 5, 4, 0), [10, 20, 30, 255]);
    }

    #[test]
    fn a_cursor_outside_the_area_changes_nothing() {
        let cursor = Cursor {
            shape: Shape {
                kind: ShapeKind::Color,
                width: 1,
                height: 1,
                pitch: 4,
                data: vec![0, 0, 255, 255],
            },
            at: Point::new(-1, 3),
        };
        let size = Size::new(2, 2);
        let mut pixels = filled(size, [1, 2, 3, 255]);
        cursor.draw(Point::new(0, 0), size, 8, &mut pixels);
        assert_eq!(pixels, filled(size, [1, 2, 3, 255]));
    }
}
