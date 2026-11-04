//! 範囲の調整: つまみ（`Handle`）・範囲の中のドラッグ・矢印キーで、決めた範囲を直す。
//!
//! 範囲選択の画面の精密モード（`selection`）とスクラップのトリム（`scrap`）が共有する。どちらも範囲を
//! 1 つの座標系（範囲選択はスクリーン座標、トリムは表示している画像の画素）で持ち、ここの関数で動かす。
//! 当たり判定（`hit`）とつまみの見た目（`handle_rect`）は画面の上の矩形で行い、大きさは 96 DPI での値
//! （`HANDLE_SIDE`、`HANDLE_GRIP`）を呼び手が DPI で換算する（`scaled` に渡す関数）。

use crate::geometry::{Point, Rect};

/// 範囲の大きさを変えるつまみ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    /// すべて。角を先に並べる（重なったら角を選ぶ）。
    pub const ALL: [Handle; 8] = [
        Handle::TopLeft,
        Handle::TopRight,
        Handle::BottomRight,
        Handle::BottomLeft,
        Handle::Top,
        Handle::Right,
        Handle::Bottom,
        Handle::Left,
    ];

    /// 動かす辺（左、上、右、下）。
    const fn edges(self) -> (bool, bool, bool, bool) {
        match self {
            Handle::TopLeft => (true, true, false, false),
            Handle::Top => (false, true, false, false),
            Handle::TopRight => (false, true, true, false),
            Handle::Right => (false, false, true, false),
            Handle::BottomRight => (false, false, true, true),
            Handle::Bottom => (false, false, false, true),
            Handle::BottomLeft => (true, false, false, true),
            Handle::Left => (true, false, false, false),
        }
    }

    /// つまみの中心（範囲の枠の上の点）。右と下は範囲に含まない側の座標。
    pub fn center(self, r: Rect) -> Point {
        let cx = r.left + (r.right - r.left) / 2;
        let cy = r.top + (r.bottom - r.top) / 2;
        let x = match self {
            Handle::TopLeft | Handle::BottomLeft | Handle::Left => r.left,
            Handle::Top | Handle::Bottom => cx,
            Handle::TopRight | Handle::Right | Handle::BottomRight => r.right,
        };
        let y = match self {
            Handle::TopLeft | Handle::Top | Handle::TopRight => r.top,
            Handle::Left | Handle::Right => cy,
            Handle::BottomLeft | Handle::Bottom | Handle::BottomRight => r.bottom,
        };
        Point::new(x, y)
    }
}

/// 矢印キーの向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// カーソルの形。Win32 層が `WM_SETCURSOR` で当てる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// 十字（既定）。
    Cross,
    /// 範囲の中（ドラッグで動かす）。
    Move,
    /// 左上・右下のつまみ。
    ResizeNwse,
    /// 右上・左下のつまみ。
    ResizeNesw,
    /// 上・下のつまみ。
    ResizeNs,
    /// 左・右のつまみ。
    ResizeWe,
}

/// つまみの一辺（96 DPI での物理ピクセル。モニタの DPI で換算する）。
pub const HANDLE_SIDE: i32 = 7;
/// つまみをつかめる、見た目より外側の幅（96 DPI での物理ピクセル）。
pub const HANDLE_GRIP: i32 = 3;

/// 矢印で動かす量。
const ARROW_STEP: i32 = 1;
/// Shift+矢印で動かす量。
const SHIFT_ARROW_STEP: i32 = 10;

/// つかんだもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grab {
    /// つまみ。None なら範囲の中（位置を動かす）。
    pub handle: Option<Handle>,
    /// 押した点。
    pub from: Point,
    /// 押したときの範囲。
    pub start: Rect,
}

impl Grab {
    /// つかんだものを `at` まで引いたあとの範囲。大きさは 1 画素より小さくせず、`bounds` の外には出さない。
    pub fn dragged(&self, at: Point, bounds: Rect) -> Rect {
        let (dx, dy) = (at.x - self.from.x, at.y - self.from.y);
        let r = self.start;
        let Some(handle) = self.handle else {
            return keep_inside(r.offset(dx, dy), bounds);
        };
        let b = bounds;
        let (left, top, right, bottom) = handle.edges();
        let mut out = r;
        if left {
            out.left = (r.left + dx).clamp(b.left.min(r.right - 1), r.right - 1);
        }
        if top {
            out.top = (r.top + dy).clamp(b.top.min(r.bottom - 1), r.bottom - 1);
        }
        if right {
            out.right = (r.right + dx).clamp(r.left + 1, b.right.max(r.left + 1));
        }
        if bottom {
            out.bottom = (r.bottom + dy).clamp(r.top + 1, b.bottom.max(r.top + 1));
        }
        out
    }

    /// つかんでいる間のカーソルの形。
    pub fn cursor_shape(&self) -> CursorShape {
        cursor_shape(Some(self.handle))
    }
}

/// `rect` の `at` にあるもの。つまみなら Some(Some)、範囲の中なら Some(None)、外なら None。
/// `scaled` は 96 DPI での長さを、その点のある画面の物理ピクセルにする。
pub fn hit(rect: Rect, at: Point, scaled: impl Fn(i32, Point) -> i32) -> Option<Option<Handle>> {
    let found = Handle::ALL.into_iter().find(|&handle| {
        let grip = scaled(HANDLE_GRIP, handle.center(rect));
        handle_rect(rect, handle, &scaled)
            .inflate(grip)
            .contains(at)
    });
    match found {
        Some(handle) => Some(Some(handle)),
        None => rect.contains(at).then_some(None),
    }
}

/// つまみの見た目の矩形。大きさは中心のある画面の DPI で換算する（`scaled`）。
pub fn handle_rect(rect: Rect, handle: Handle, scaled: impl Fn(i32, Point) -> i32) -> Rect {
    let c = handle.center(rect);
    let side = scaled(HANDLE_SIDE, c);
    let left = c.x - side / 2;
    let top = c.y - side / 2;
    Rect::new(left, top, left + side, top + side)
}

/// すべてのつまみの見た目の矩形。
pub fn handle_rects(rect: Rect, scaled: impl Fn(i32, Point) -> i32) -> Vec<Rect> {
    Handle::ALL
        .into_iter()
        .map(|handle| handle_rect(rect, handle, &scaled))
        .collect()
}

/// `hit` で見つけたものの上でのカーソルの形。つまみなら大きさを変える形、範囲の中なら動かす形。
pub fn cursor_shape(target: Option<Option<Handle>>) -> CursorShape {
    match target {
        None => CursorShape::Cross,
        Some(None) => CursorShape::Move,
        Some(Some(Handle::TopLeft | Handle::BottomRight)) => CursorShape::ResizeNwse,
        Some(Some(Handle::TopRight | Handle::BottomLeft)) => CursorShape::ResizeNesw,
        Some(Some(Handle::Top | Handle::Bottom)) => CursorShape::ResizeNs,
        Some(Some(Handle::Left | Handle::Right)) => CursorShape::ResizeWe,
    }
}

/// 矢印（`shift` なら Shift+矢印）で `rect` を動かした範囲。`bounds` の外には出さない。
pub fn moved(rect: Rect, direction: Direction, shift: bool, bounds: Rect) -> Rect {
    let (dx, dy) = step(direction, shift);
    keep_inside(rect.offset(dx, dy), bounds)
}

/// Ctrl+矢印（`shift` なら Ctrl+Shift+矢印）で `rect` の右下の辺を動かした範囲。大きさは 1 画素より
/// 小さくせず、`bounds` の外には広げない。
pub fn resized(rect: Rect, direction: Direction, shift: bool, bounds: Rect) -> Rect {
    let (dx, dy) = step(direction, shift);
    let b = bounds;
    let right = (rect.right + dx).clamp(rect.left + 1, b.right.max(rect.left + 1));
    let bottom = (rect.bottom + dy).clamp(rect.top + 1, b.bottom.max(rect.top + 1));
    Rect::new(rect.left, rect.top, right, bottom)
}

/// 矢印 1 回で動かす量。
pub fn step(direction: Direction, shift: bool) -> (i32, i32) {
    let step = if shift { SHIFT_ARROW_STEP } else { ARROW_STEP };
    match direction {
        Direction::Left => (-step, 0),
        Direction::Right => (step, 0),
        Direction::Up => (0, -step),
        Direction::Down => (0, step),
    }
}

/// 大きさを変えずに、`rect` を `bounds` の中に戻す。
pub fn keep_inside(rect: Rect, bounds: Rect) -> Rect {
    let b = bounds;
    let dx = if rect.left < b.left {
        b.left - rect.left
    } else if rect.right > b.right {
        (b.right - rect.right).max(b.left - rect.left)
    } else {
        0
    };
    let dy = if rect.top < b.top {
        b.top - rect.top
    } else if rect.bottom > b.bottom {
        (b.bottom - rect.bottom).max(b.top - rect.top)
    } else {
        0
    };
    rect.offset(dx, dy)
}
