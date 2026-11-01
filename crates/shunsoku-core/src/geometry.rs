//! スクリーン座標（物理ピクセル）の点と矩形。範囲選択の状態、切り抜き、スクラップの位置はすべてこれで持つ。

/// 点。スクリーン座標なら、プライマリ モニタの左上が原点で、左や上のモニタでは負になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }
}

/// 大きさ（物理ピクセル）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Size {
        Size { width, height }
    }
}

/// 矩形。`right` と `bottom` は含まない（Win32 の `RECT` と同じ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// 左上と大きさから作る。
    pub fn from_origin_size(origin: Point, size: Size) -> Rect {
        Rect {
            left: origin.x,
            top: origin.y,
            right: origin.x.saturating_add_unsigned(size.width),
            bottom: origin.y.saturating_add_unsigned(size.height),
        }
    }

    pub fn origin(&self) -> Point {
        Point::new(self.left, self.top)
    }

    pub fn width(&self) -> u32 {
        self.right.saturating_sub(self.left).max(0) as u32
    }

    pub fn height(&self) -> u32 {
        self.bottom.saturating_sub(self.top).max(0) as u32
    }

    pub fn size(&self) -> Size {
        Size::new(self.width(), self.height())
    }

    pub fn is_empty(&self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    pub fn contains(&self, p: Point) -> bool {
        (self.left..self.right).contains(&p.x) && (self.top..self.bottom).contains(&p.y)
    }

    /// 重なる部分。重ならなければ None。
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let r = Rect {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (!r.is_empty()).then_some(r)
    }

    /// 平行移動した矩形。
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    /// 四方に `by` だけ広げた矩形。
    pub fn inflate(&self, by: i32) -> Rect {
        Rect::new(
            self.left - by,
            self.top - by,
            self.right + by,
            self.bottom + by,
        )
    }
}
