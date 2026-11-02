//! 書き込みの編集モードのツールバーの、Win32 に依らない部分: 並び（`Layout`）、当たり判定、色の並び
//! （`palette`）、置き場所（`place`）。

use super::{EditState, Pick, Tool};
use crate::geometry::{Point, Rect, Size};
use crate::settings::Rgb;

/// ツールバーに並べる固定の色（赤・橙・黄・緑・青・紫・黒・白）。
pub const PALETTE: [Rgb; 8] = [
    Rgb(0xE5, 0x39, 0x35),
    Rgb(0xFB, 0x8C, 0x00),
    Rgb(0xFD, 0xD8, 0x35),
    Rgb(0x43, 0xA0, 0x47),
    Rgb(0x1E, 0x88, 0xE5),
    Rgb(0x8E, 0x24, 0xAA),
    Rgb(0x21, 0x21, 0x21),
    Rgb(0xFF, 0xFF, 0xFF),
];

/// ツールバーに並べる色。設定の既定色 `default` が `PALETTE` になければ先頭に足す。
pub fn palette(default: Rgb) -> Vec<Rgb> {
    let mut colours = Vec::with_capacity(PALETTE.len() + 1);
    if !PALETTE.contains(&default) {
        colours.push(default);
    }
    colours.extend(PALETTE);
    colours
}

/// ツールバー（大きさ `size`）の左上。スクラップの画像の矩形 `anchor` の直下に `gap` 空けて、左を揃えて置く。
/// 作業領域 `work` の下に余地がなければ上に、上にもなければ画像の下端の内側に重ねる。横は作業領域に収める。
pub fn place(anchor: Rect, size: Size, work: Rect, gap: i32) -> Point {
    let width = size.width as i32;
    let height = size.height as i32;
    // 作業領域より広ければ左に寄せる
    let x = anchor.left.min(work.right - width).max(work.left);
    let below = anchor.bottom + gap;
    let above = anchor.top - gap - height;
    let y = if below + height <= work.bottom {
        below
    } else if above >= work.top {
        above
    } else {
        anchor.bottom.min(work.bottom) - gap - height
    };
    Point::new(x, y)
}

/// 太さの選択肢（元の画像の画素）。
pub const WIDTHS: [u32; 4] = [2, 3, 5, 8];

/// ツールバーの道具の欄。選択と 6 種の道具を、まだ描けないものも含めて並べる（描けないものは押せない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSlot {
    Select,
    Rect,
    Ellipse,
    Arrow,
    Line,
    Text,
    Freehand,
}

impl ToolSlot {
    pub const ALL: [ToolSlot; 7] = [
        ToolSlot::Select,
        ToolSlot::Rect,
        ToolSlot::Ellipse,
        ToolSlot::Arrow,
        ToolSlot::Line,
        ToolSlot::Text,
        ToolSlot::Freehand,
    ];

    /// この欄で選ぶ道具。まだない道具なら None。
    pub fn tool(self) -> Option<Tool> {
        match self {
            ToolSlot::Rect => Some(Tool::Rect),
            ToolSlot::Ellipse => Some(Tool::Ellipse),
            ToolSlot::Arrow => Some(Tool::Arrow),
            ToolSlot::Line => Some(Tool::Line),
            ToolSlot::Freehand => Some(Tool::Freehand),
            // 選択（#86）と文字（#87）
            ToolSlot::Select | ToolSlot::Text => None,
        }
    }
}

/// ツールバーのボタン。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Tool(ToolSlot),
    Color(Rgb),
    Width(u32),
    /// 完了（編集モードを抜ける）。
    Done,
    /// 移動用のつまみ。ドラッグでスクラップごと動かす。
    Grip,
}

impl Button {
    /// 押したときに選ぶもの。道具・色・太さでなければ（まだない道具も）None。
    pub fn pick(self) -> Option<Pick> {
        match self {
            Button::Tool(slot) => slot.tool().map(Pick::Tool),
            Button::Color(color) => Some(Pick::Color(color)),
            Button::Width(width) => Some(Pick::Width(width)),
            Button::Done | Button::Grip => None,
        }
    }

    /// 押せるか。まだない道具は押せない（灰色で描く）。
    pub fn is_enabled(self) -> bool {
        !matches!(self, Button::Tool(slot) if slot.tool().is_none())
    }

    /// いま選ばれているか（`state` の道具・色・太さと同じ）。
    pub fn is_current(self, state: &EditState) -> bool {
        match self.pick() {
            Some(Pick::Tool(tool)) => state.tool == tool,
            Some(Pick::Color(color)) => state.style.color == color,
            Some(Pick::Width(width)) => state.style.width == width,
            None => false,
        }
    }
}

/// ツールバーの寸法（DPI 96 のときの物理ピクセル）。
const PADDING: f64 = 4.0;
const BUTTON: f64 = 28.0;
const SPACING: f64 = 2.0;
const GROUP_SPACING: f64 = 10.0;
const GRIP_WIDTH: f64 = 14.0;

/// ツールバーの並び: 左から道具、色、太さ、完了、端に移動用のつまみ。座標はツールバーの左上が原点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    size: Size,
    buttons: Vec<(Button, Rect)>,
}

impl Layout {
    /// `colours`（`palette`）を並べた並び。`scale` はスクラップのモニタの DPI / 96。
    pub fn new(colours: &[Rgb], scale: f64) -> Layout {
        let px = |v: f64| (v * scale).round() as i32;
        let (padding, button, spacing, group) =
            (px(PADDING), px(BUTTON), px(SPACING), px(GROUP_SPACING));
        let groups: [Vec<Button>; 4] = [
            ToolSlot::ALL.iter().map(|&s| Button::Tool(s)).collect(),
            colours.iter().map(|&c| Button::Color(c)).collect(),
            WIDTHS.iter().map(|&w| Button::Width(w)).collect(),
            vec![Button::Done],
        ];
        let mut buttons = Vec::new();
        let mut x = padding;
        for (i, group_buttons) in groups.iter().enumerate() {
            if i > 0 {
                x += group - spacing;
            }
            for &b in group_buttons {
                buttons.push((b, Rect::new(x, padding, x + button, padding + button)));
                x += button + spacing;
            }
        }
        x += group - spacing;
        let grip = Rect::new(x, padding, x + px(GRIP_WIDTH), padding + button);
        buttons.push((Button::Grip, grip));
        let width = grip.right + padding;
        Layout {
            size: Size::new(width as u32, (button + padding * 2) as u32),
            buttons,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// ボタンと、その矩形。
    pub fn buttons(&self) -> &[(Button, Rect)] {
        &self.buttons
    }

    /// ツールバーの中の点 `at` にあるボタン。ボタンの間なら None。
    pub fn hit(&self, at: Point) -> Option<Button> {
        self.buttons
            .iter()
            .find(|(_, rect)| rect.contains(at))
            .map(|&(b, _)| b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_puts_a_default_colour_that_is_not_in_it_first() {
        assert_eq!(palette(Rgb(0xE5, 0x39, 0x35)), PALETTE.to_vec());
        let custom = Rgb(0x12, 0x34, 0x56);
        let colours = palette(custom);
        assert_eq!(colours.len(), PALETTE.len() + 1);
        assert_eq!(colours[0], custom);
        assert_eq!(&colours[1..], &PALETTE);
    }

    const WORK: Rect = Rect::new(0, 0, 1920, 1040);
    const BAR: Size = Size::new(400, 36);

    #[test]
    fn the_toolbar_goes_right_below_the_scrap_with_the_left_edges_lined_up() {
        let anchor = Rect::new(100, 200, 700, 600);
        assert_eq!(place(anchor, BAR, WORK, 6), Point::new(100, 606));
    }

    #[test]
    fn the_toolbar_goes_above_the_scrap_when_there_is_no_room_below() {
        let anchor = Rect::new(100, 600, 700, 1010);
        assert_eq!(place(anchor, BAR, WORK, 6), Point::new(100, 558));
    }

    #[test]
    fn the_toolbar_overlaps_the_bottom_of_the_scrap_when_there_is_no_room_above_or_below() {
        let anchor = Rect::new(100, 20, 700, 1020);
        assert_eq!(place(anchor, BAR, WORK, 6), Point::new(100, 978));
        // 下が作業領域の外まではみ出していれば、作業領域の下端の内側
        let anchor = Rect::new(100, 20, 700, 1200);
        assert_eq!(place(anchor, BAR, WORK, 6), Point::new(100, 998));
    }

    #[test]
    fn the_toolbar_stays_inside_the_work_area_sideways() {
        let anchor = Rect::new(1800, 200, 2400, 600);
        assert_eq!(place(anchor, BAR, WORK, 6), Point::new(1520, 606));
        let anchor = Rect::new(-300, 200, 300, 600);
        assert_eq!(place(anchor, BAR, WORK, 6), Point::new(0, 606));
        // 作業領域が他のモニタ（負の座標）でも同じ
        let work = Rect::new(-1920, 0, 0, 1040);
        let anchor = Rect::new(-200, 200, 400, 600);
        assert_eq!(place(anchor, BAR, work, 6), Point::new(-400, 606));
    }

    #[test]
    fn the_toolbar_lines_up_tools_colours_widths_done_and_the_grip_at_the_end() {
        let layout = Layout::new(&PALETTE, 1.0);
        let order: Vec<Button> = layout.buttons().iter().map(|&(b, _)| b).collect();
        let mut expected: Vec<Button> = ToolSlot::ALL.iter().map(|&s| Button::Tool(s)).collect();
        expected.extend(PALETTE.iter().map(|&c| Button::Color(c)));
        expected.extend(WIDTHS.iter().map(|&w| Button::Width(w)));
        expected.extend([Button::Done, Button::Grip]);
        assert_eq!(order, expected);
        // 7 + 8 + 4 + 1 個の 28px のボタン、間 2px、組の間 10px、つまみ 14px、外の余白 4px
        assert_eq!(
            layout.size(),
            Size::new(4 + 20 * 28 + 16 * 2 + 4 * 10 + 14 + 4, 36)
        );
    }

    #[test]
    fn a_point_on_a_button_hits_it_and_a_point_between_buttons_hits_nothing() {
        let layout = Layout::new(&PALETTE, 1.0);
        assert_eq!(
            layout.hit(Point::new(4, 4)),
            Some(Button::Tool(ToolSlot::Select))
        );
        assert_eq!(
            layout.hit(Point::new(40, 20)),
            Some(Button::Tool(ToolSlot::Rect))
        );
        assert_eq!(layout.hit(Point::new(32, 20)), None);
        assert_eq!(layout.hit(Point::new(2, 20)), None);
        let right = layout.size().width as i32;
        assert_eq!(layout.hit(Point::new(right - 6, 20)), Some(Button::Grip));
        // 色の組は、道具の組から 10px 空けて始まる（7 個目の右端 32+6*30=212、そこから 10px）
        assert_eq!(layout.hit(Point::new(215, 20)), None);
        assert_eq!(
            layout.hit(Point::new(222, 20)),
            Some(Button::Color(PALETTE[0]))
        );
    }

    #[test]
    fn the_toolbar_grows_with_the_dpi_of_the_scrap() {
        let layout = Layout::new(&PALETTE, 1.5);
        assert_eq!(layout.size().height, 54);
        assert_eq!(
            layout.hit(Point::new(47, 47)),
            Some(Button::Tool(ToolSlot::Select))
        );
    }

    #[test]
    fn buttons_pick_tools_colours_and_widths_and_show_what_is_current() {
        let state = EditState::new(crate::annotate::Style {
            color: PALETTE[4],
            width: 5,
        });
        assert_eq!(
            Button::Tool(ToolSlot::Rect).pick(),
            Some(Pick::Tool(Tool::Rect))
        );
        assert!(Button::Tool(ToolSlot::Rect).is_current(&state));
        assert!(Button::Color(PALETTE[4]).is_current(&state));
        assert!(!Button::Color(PALETTE[0]).is_current(&state));
        assert!(Button::Width(5).is_current(&state));
        assert!(!Button::Width(3).is_current(&state));
        assert_eq!(
            Button::Tool(ToolSlot::Freehand).pick(),
            Some(Pick::Tool(Tool::Freehand))
        );
        assert!(!Button::Tool(ToolSlot::Ellipse).is_current(&state));
        // 選択（#86）と文字（#87）はまだない
        assert!(!Button::Tool(ToolSlot::Select).is_enabled());
        assert!(!Button::Tool(ToolSlot::Text).is_enabled());
        assert!(Button::Tool(ToolSlot::Arrow).is_enabled());
        assert_eq!(Button::Done.pick(), None);
        assert_eq!(Button::Grip.pick(), None);
        assert!(Button::Done.is_enabled() && Button::Grip.is_enabled());
    }
}
