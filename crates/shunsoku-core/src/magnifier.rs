//! 精密モードの拡大鏡。カーソルのまわりの画素を拡大して見せ、カーソルの下の色を出す。

use crate::geometry::{Point, Rect};

/// 拡大して見せる範囲の一辺（画素）。奇数にして、カーソルの画素を真ん中に置く。
pub const SPAN: i32 = 15;
/// 拡大の倍率。拡大した 1 画素の一辺は、96 DPI でこの物理ピクセル（モニタの DPI で換算する）。
pub const ZOOM: i32 = 8;
/// 拡大鏡をカーソルの画素から離す量（96 DPI での物理ピクセル）。十字のカーソルに重ならないように。
pub const OFFSET: i32 = 20;

/// 拡大して見せる範囲（スクリーン座標）。カーソルの画素を真ん中にした `SPAN` × `SPAN` 画素。
pub fn source(cursor: Point) -> Rect {
    let half = SPAN / 2;
    Rect::new(
        cursor.x - half,
        cursor.y - half,
        cursor.x - half + SPAN,
        cursor.y - half + SPAN,
    )
}

/// 色コード。BGRA の画素を `#RRGGBB   rgb(r, g, b)` にする（アルファは見ない）。
pub fn color_code(bgra: [u8; 4]) -> String {
    let [b, g, r, _] = bgra;
    format!("#{r:02X}{g:02X}{b:02X}   rgb({r}, {g}, {b})")
}

/// 拡大鏡（と、その下に付ける表示）の置き場所。大きさは `width` × `height`（物理ピクセル）。
///
/// カーソルの画素から右下へ `offset` だけ離して置き、`monitor`（カーソルのあるモニタ）の右や下に
/// はみ出すなら、カーソルの左や上へ回す。モニタはまたがない。
pub fn place(cursor: Point, width: i32, height: i32, offset: i32, monitor: Rect) -> Rect {
    let left = beside(cursor.x, width, offset, monitor.left, monitor.right);
    let top = beside(cursor.y, height, offset, monitor.top, monitor.bottom);
    Rect::new(left, top, left + width, top + height)
}

/// 1 軸での置き場所の始まり。`at` の後ろ（右・下）に置き、`end` を越えるなら前（左・上）に回す。
/// どちらにも収まらなければ `start..end` の中に押し戻す（収まらないほど大きければ `start` から）。
fn beside(at: i32, len: i32, offset: i32, start: i32, end: i32) -> i32 {
    let after = at + offset;
    let begin = if after + len > end {
        at - offset - len
    } else {
        after
    };
    begin.min(end - len).max(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    const MONITOR: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    #[test]
    fn it_sits_below_right_of_the_cursor_away_from_the_edges() {
        assert_eq!(
            place(p(500, 400), 120, 150, 20, MONITOR),
            Rect::new(520, 420, 640, 570)
        );
    }

    #[test]
    fn near_the_right_edge_it_goes_to_the_left_of_the_cursor() {
        assert_eq!(
            place(p(1800, 400), 120, 150, 20, MONITOR),
            Rect::new(1660, 420, 1780, 570)
        );
    }

    #[test]
    fn near_the_bottom_edge_it_goes_above_the_cursor() {
        assert_eq!(
            place(p(500, 1000), 120, 150, 20, MONITOR),
            Rect::new(520, 830, 640, 980)
        );
    }

    #[test]
    fn in_the_bottom_right_corner_it_goes_above_left_of_the_cursor() {
        assert_eq!(
            place(p(1919, 1079), 120, 150, 20, MONITOR),
            Rect::new(1779, 909, 1899, 1059)
        );
    }

    #[test]
    fn in_the_top_left_corner_it_stays_below_right_of_the_cursor() {
        assert_eq!(
            place(p(0, 0), 120, 150, 20, MONITOR),
            Rect::new(20, 20, 140, 170)
        );
    }

    #[test]
    fn in_the_top_right_and_bottom_left_corners_it_turns_only_the_overflowing_way() {
        assert_eq!(
            place(p(1919, 0), 120, 150, 20, MONITOR),
            Rect::new(1779, 20, 1899, 170)
        );
        assert_eq!(
            place(p(0, 1079), 120, 150, 20, MONITOR),
            Rect::new(20, 909, 140, 1059)
        );
    }

    #[test]
    fn on_a_monitor_left_of_the_primary_it_does_not_cross_into_the_primary() {
        let left_monitor = Rect::new(-1280, 0, 0, 1024);
        assert_eq!(
            place(p(-60, 500), 120, 150, 20, left_monitor),
            Rect::new(-200, 520, -80, 670)
        );
        assert_eq!(
            place(p(-1280, 0), 120, 150, 20, left_monitor),
            Rect::new(-1260, 20, -1140, 170)
        );
    }

    #[test]
    fn when_neither_side_fits_it_is_pushed_back_inside_the_monitor() {
        let small = Rect::new(100, 100, 300, 300);
        assert_eq!(
            place(p(200, 200), 120, 150, 20, small),
            Rect::new(100, 100, 220, 250)
        );
    }

    #[test]
    fn on_a_monitor_smaller_than_the_magnifier_it_starts_at_the_top_left_of_the_monitor() {
        let tiny = Rect::new(0, 0, 100, 100);
        assert_eq!(
            place(p(50, 50), 120, 150, 20, tiny),
            Rect::new(0, 0, 120, 150)
        );
    }

    #[test]
    fn it_magnifies_15_by_15_pixels_centred_on_the_cursor() {
        assert_eq!(source(p(100, 200)), Rect::new(93, 193, 108, 208));
        assert_eq!(source(p(-1, 0)), Rect::new(-8, -7, 7, 8));
    }

    #[test]
    fn the_colour_code_shows_hex_and_rgb() {
        // BGRA の並びで渡す
        assert_eq!(
            color_code([0x3c, 0x2b, 0x1a, 0xff]),
            "#1A2B3C   rgb(26, 43, 60)"
        );
        assert_eq!(color_code([0, 0, 0, 0xff]), "#000000   rgb(0, 0, 0)");
    }
}
