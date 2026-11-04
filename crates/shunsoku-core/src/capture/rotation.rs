//! 回転したモニタの画像を、画面で見えている向きに直す対応（#132）。
//!
//! Desktop Duplication は、回転したモニタの画像を回転前の向き（縦置きなら横長）で返す。Win32 層は
//! 取得の直後に GPU 上でこの対応どおりに回し直す（`crates/shunsoku/src/rotate.rs`）。以後、モニタの画像は
//! いつも画面で見えている向き。

use crate::geometry::{Point, Size};

/// モニタの回転の向き（`DXGI_MODE_ROTATION`）。`UNSPECIFIED` は `Identity` と同じに扱う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    Identity,
    Rotate90,
    Rotate180,
    Rotate270,
}

impl Rotation {
    /// 回転前の画像の大きさ `raw` から、画面で見えている向きにしたときの大きさ。
    pub fn upright_size(self, raw: Size) -> Size {
        match self {
            Rotation::Identity | Rotation::Rotate180 => raw,
            Rotation::Rotate90 | Rotation::Rotate270 => Size::new(raw.height, raw.width),
        }
    }

    /// 画面で見えている向きの画素 `upright` が、回転前の画像（大きさ `raw`）のどの画素にあたるか。
    ///
    /// 90° は回転前の画像を時計回りに 90° 回したもの、270° は反時計回りに 90° 回したもの。
    /// 対応は `upright` について 1 次（GPU では係数にして使う）なので、画像の外の点も同じ式で返す。
    pub fn source_pixel(self, raw: Size, upright: Point) -> Point {
        let (w, h) = (raw.width as i32, raw.height as i32);
        let Point { x, y } = upright;
        match self {
            Rotation::Identity => Point::new(x, y),
            Rotation::Rotate90 => Point::new(y, h - 1 - x),
            Rotation::Rotate180 => Point::new(w - 1 - x, h - 1 - y),
            Rotation::Rotate270 => Point::new(w - 1 - y, x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回転前の 3×2 の画像。
    ///
    /// ```text
    /// a b c
    /// d e f
    /// ```
    const RAW: [[char; 3]; 2] = [['a', 'b', 'c'], ['d', 'e', 'f']];

    /// `RAW` を `rotation` で画面で見えている向きにした画像（行の並び）。
    fn upright(rotation: Rotation) -> Vec<String> {
        let raw = Size::new(3, 2);
        let size = rotation.upright_size(raw);
        (0..size.height as i32)
            .map(|y| {
                (0..size.width as i32)
                    .map(|x| {
                        let p = rotation.source_pixel(raw, Point::new(x, y));
                        RAW[p.y as usize][p.x as usize]
                    })
                    .collect()
            })
            .collect()
    }

    // 期待値は画面で見えている向きから手で書く。90° は回転前の画像を時計回りに 90° 回したもの
    // （Microsoft の Desktop Duplication のサンプルの `DXGI_MODE_ROTATION_ROTATE90` の扱いと同じ）。

    #[test]
    fn identity_keeps_the_image() {
        assert_eq!(upright(Rotation::Identity), ["abc", "def"]);
    }

    #[test]
    fn rotate90_turns_the_image_clockwise() {
        assert_eq!(upright(Rotation::Rotate90), ["da", "eb", "fc"]);
    }

    #[test]
    fn rotate180_turns_the_image_upside_down() {
        assert_eq!(upright(Rotation::Rotate180), ["fed", "cba"]);
    }

    #[test]
    fn rotate270_turns_the_image_counterclockwise() {
        assert_eq!(upright(Rotation::Rotate270), ["cf", "be", "ad"]);
    }

    #[test]
    fn quarter_turns_swap_width_and_height() {
        let raw = Size::new(2560, 1440);
        assert_eq!(Rotation::Identity.upright_size(raw), raw);
        assert_eq!(Rotation::Rotate180.upright_size(raw), raw);
        assert_eq!(Rotation::Rotate90.upright_size(raw), Size::new(1440, 2560));
        assert_eq!(Rotation::Rotate270.upright_size(raw), Size::new(1440, 2560));
    }
}
