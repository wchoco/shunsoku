//! 書き込みの編集モードのツールバーの、Win32 に依らない部分: 並び（`Layout`）、当たり判定、色の並び
//! （`palette`）、置き場所（`place`）。

use super::{Annotations, EditState, Pick, Shape, Tool};
use crate::geometry::{Point, Rect, Size};
use crate::keys::Modifiers;
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

/// 文字の大きさの選択肢（元の画像の画素）。文字の道具の間（と打っている間）は、太さの欄がこれになる。
pub const FONT_SIZES: [u32; 4] = [12, 16, 24, 32];

/// ツールバーの道具の欄。選択と 6 種の描く道具、消しゴムを並べる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSlot {
    Select,
    Rect,
    Ellipse,
    Arrow,
    Line,
    Text,
    Freehand,
    Eraser,
}

impl ToolSlot {
    pub const ALL: [ToolSlot; 8] = [
        ToolSlot::Select,
        ToolSlot::Rect,
        ToolSlot::Ellipse,
        ToolSlot::Arrow,
        ToolSlot::Line,
        ToolSlot::Text,
        ToolSlot::Freehand,
        ToolSlot::Eraser,
    ];

    /// この欄で選ぶ道具。まだない道具なら None。
    pub fn tool(self) -> Option<Tool> {
        match self {
            ToolSlot::Rect => Some(Tool::Rect),
            ToolSlot::Ellipse => Some(Tool::Ellipse),
            ToolSlot::Arrow => Some(Tool::Arrow),
            ToolSlot::Line => Some(Tool::Line),
            ToolSlot::Freehand => Some(Tool::Freehand),
            ToolSlot::Select => Some(Tool::Select),
            ToolSlot::Text => Some(Tool::Text),
            ToolSlot::Eraser => Some(Tool::Eraser),
        }
    }
}

/// 編集モードのホイール 1 ノッチで選ぶもの。`modifiers` はそのとき押している修飾キー、`rotation` はノッチの
/// 向き（上が正）。修飾キーなしで道具、Shift で色（`colours` の順）、Ctrl で太さ（文字の道具か打っている間は
/// 文字の大きさ）を、下へ回すとツールバーの並びの次へ、上へ回すと前へ進め、端では反対の端へ回り込む。ほかの
/// 修飾キーつきと回っていない（0）なら None。割り当ては固定（`[mouse]` では変えない）。
pub fn wheel_pick(
    state: &EditState,
    colours: &[Rgb],
    modifiers: Modifiers,
    rotation: i32,
) -> Option<Pick> {
    let Modifiers {
        win: false,
        ctrl,
        alt: false,
        shift,
    } = modifiers
    else {
        return None;
    };
    let (color, size) = shown_color_and_font_size(state);
    match (ctrl, shift) {
        (false, false) => {
            let tools: Vec<Tool> = ToolSlot::ALL.iter().filter_map(|s| s.tool()).collect();
            step(&tools, &state.tool, rotation).map(Pick::Tool)
        }
        (false, true) => step(colours, &color, rotation).map(Pick::Color),
        (true, false) if shows_font_sizes(state) => {
            step_size(&FONT_SIZES, size, rotation).map(Pick::FontSize)
        }
        (true, false) => step_size(&WIDTHS, state.style.width, rotation).map(Pick::Width),
        (true, true) => None,
    }
}

/// ツールバーで選ばれている色と文字の大きさ。文字を打っている間は、打っている文字のもの（打ち直しなら、その
/// 文字の）。
fn shown_color_and_font_size(state: &EditState) -> (Rgb, u32) {
    let typing = state.typing();
    let color = typing.as_ref().map_or(state.style.color, |a| a.style.color);
    let size = match typing.map(|a| a.shape) {
        Some(Shape::Text { font, .. }) => font.size,
        _ => state.font.size,
    };
    (color, size)
}

/// 太さの欄に文字の大きさを並べるか（文字の道具を選んでいるか、文字を打っている間）。
fn shows_font_sizes(state: &EditState) -> bool {
    state.tool == Tool::Text || state.is_typing()
}

/// 小さい順の選択肢 `sizes` で、`current` から `rotation` の向きに 1 つ進んだもの。下へは次に大きいもの、上へは
/// 次に小さいもの（`current` が選択肢になくても）。端を越えたら反対の端へ回り込む。
fn step_size(sizes: &[u32], current: u32, rotation: i32) -> Option<u32> {
    let (first, last) = (*sizes.first()?, *sizes.last()?);
    match rotation.signum() {
        -1 => Some(
            sizes
                .iter()
                .copied()
                .find(|&s| s > current)
                .unwrap_or(first),
        ),
        1 => Some(
            sizes
                .iter()
                .copied()
                .rev()
                .find(|&s| s < current)
                .unwrap_or(last),
        ),
        _ => None,
    }
}

/// `items` の中の `current` から、`rotation` の向きに 1 つ進んだもの（下へは次、上へは前）。端では反対の端へ
/// 回り込む。`current` が `items` になければ、下へは先頭、上へは末尾。
fn step<T: Copy + PartialEq>(items: &[T], current: &T, rotation: i32) -> Option<T> {
    let len = items.len();
    let at = items.iter().position(|x| x == current);
    let next = match (rotation.signum(), at) {
        (-1, Some(i)) => (i + 1) % len,
        (-1, None) => 0,
        (1, Some(i)) => (i + len - 1) % len,
        (1, None) => len.checked_sub(1)?,
        _ => return None,
    };
    items.get(next).copied()
}

/// ツールバーのボタン。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Tool(ToolSlot),
    Color(Rgb),
    Width(u32),
    /// 文字の大きさ。
    FontSize(u32),
    /// 元に戻す。
    Undo,
    /// やり直す。
    Redo,
    /// 削除（選んでいる書き込みを消す）。
    Delete,
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
            Button::FontSize(size) => Some(Pick::FontSize(size)),
            Button::Undo | Button::Redo | Button::Delete | Button::Done | Button::Grip => None,
        }
    }

    /// 押せるか（押せないものは灰色で描く）。まだない道具、書き込みを選んでいないときの削除、戻す・
    /// やり直すものがないときの元に戻す・やり直すは押せない。
    pub fn is_enabled(self, state: &EditState, annotations: &Annotations) -> bool {
        match self {
            Button::Tool(slot) => slot.tool().is_some(),
            Button::Undo => annotations.can_undo(),
            Button::Redo => annotations.can_redo(),
            Button::Delete => !state.selected().is_empty(),
            _ => true,
        }
    }

    /// いま選ばれているか（`state` の道具・色・太さ・文字の大きさと同じ）。文字を打っている間の色と大きさは、
    /// 打っている文字のもの（打ち直しなら、その文字の）。
    pub fn is_current(self, state: &EditState) -> bool {
        let (color, size) = shown_color_and_font_size(state);
        match self.pick() {
            Some(Pick::Tool(tool)) => state.tool == tool,
            Some(Pick::Color(c)) => color == c,
            Some(Pick::Width(width)) => state.style.width == width,
            Some(Pick::FontSize(s)) => size == s,
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

/// ツールバーの並び: 左から道具、色、太さ（文字の道具では文字の大きさ）、元に戻す・やり直す・削除・完了、端に
/// 移動用のつまみ。座標はツールバーの左上が原点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    size: Size,
    buttons: Vec<(Button, Rect)>,
}

impl Layout {
    /// `colours`（`palette`）を並べた、太さを選ぶ並び。`scale` はスクラップのモニタの DPI / 96。
    pub fn new(colours: &[Rgb], scale: f64) -> Layout {
        Layout::with_sizes(colours, scale, false)
    }

    /// `state` に合わせた並び。文字の道具を選んでいるか文字を打っている間は、太さの代わりに文字の大きさを
    /// 同じ場所に並べる。
    pub fn for_state(colours: &[Rgb], scale: f64, state: &EditState) -> Layout {
        Layout::with_sizes(colours, scale, shows_font_sizes(state))
    }

    fn with_sizes(colours: &[Rgb], scale: f64, font_sizes: bool) -> Layout {
        let sizes: Vec<Button> = if font_sizes {
            FONT_SIZES.iter().map(|&s| Button::FontSize(s)).collect()
        } else {
            WIDTHS.iter().map(|&w| Button::Width(w)).collect()
        };
        let px = |v: f64| (v * scale).round() as i32;
        let (padding, button, spacing, group) =
            (px(PADDING), px(BUTTON), px(SPACING), px(GROUP_SPACING));
        let groups: [Vec<Button>; 4] = [
            ToolSlot::ALL.iter().map(|&s| Button::Tool(s)).collect(),
            colours.iter().map(|&c| Button::Color(c)).collect(),
            sizes,
            vec![Button::Undo, Button::Redo, Button::Delete, Button::Done],
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
    use crate::annotate::Style;

    const PLAIN: Modifiers = Modifiers {
        win: false,
        ctrl: false,
        alt: false,
        shift: false,
    };
    const DOWN: i32 = -1;
    const UP: i32 = 1;

    fn editing(tool: Tool) -> EditState {
        let mut state = EditState::new(Style::default());
        state.tool = tool;
        state
    }

    #[test]
    fn the_wheel_steps_through_the_tools_in_the_toolbar_order() {
        let state = editing(Tool::Rect);
        assert_eq!(
            wheel_pick(&state, &PALETTE, PLAIN, DOWN),
            Some(Pick::Tool(Tool::Ellipse))
        );
        assert_eq!(
            wheel_pick(&state, &PALETTE, PLAIN, UP),
            Some(Pick::Tool(Tool::Select))
        );
    }

    #[test]
    fn the_tools_wrap_around_at_either_end() {
        assert_eq!(
            wheel_pick(&editing(Tool::Freehand), &PALETTE, PLAIN, DOWN),
            Some(Pick::Tool(Tool::Eraser))
        );
        assert_eq!(
            wheel_pick(&editing(Tool::Eraser), &PALETTE, PLAIN, DOWN),
            Some(Pick::Tool(Tool::Select))
        );
        assert_eq!(
            wheel_pick(&editing(Tool::Select), &PALETTE, PLAIN, UP),
            Some(Pick::Tool(Tool::Eraser))
        );
    }

    #[test]
    fn the_eraser_button_comes_last_among_the_tools_and_picks_the_eraser() {
        assert_eq!(ToolSlot::ALL.last(), Some(&ToolSlot::Eraser));
        assert_eq!(
            Button::Tool(ToolSlot::Eraser).pick(),
            Some(Pick::Tool(Tool::Eraser))
        );
        assert!(Button::Tool(ToolSlot::Eraser).is_current(&editing(Tool::Eraser)));
    }

    #[test]
    fn shift_and_the_wheel_steps_through_the_palette_and_wraps_around() {
        let shift = Modifiers {
            shift: true,
            ..PLAIN
        };
        let mut state = editing(Tool::Rect);
        state.style.color = PALETTE[0];
        assert_eq!(
            wheel_pick(&state, &PALETTE, shift, DOWN),
            Some(Pick::Color(PALETTE[1]))
        );
        assert_eq!(
            wheel_pick(&state, &PALETTE, shift, UP),
            Some(Pick::Color(PALETTE[7]))
        );
    }

    const CTRL: Modifiers = Modifiers {
        win: false,
        ctrl: true,
        alt: false,
        shift: false,
    };

    #[test]
    fn ctrl_and_the_wheel_steps_through_the_widths_and_wraps_around() {
        let mut state = editing(Tool::Arrow);
        state.style.width = 3;
        assert_eq!(
            wheel_pick(&state, &PALETTE, CTRL, DOWN),
            Some(Pick::Width(5))
        );
        state.style.width = 8;
        assert_eq!(
            wheel_pick(&state, &PALETTE, CTRL, DOWN),
            Some(Pick::Width(2))
        );
    }

    #[test]
    fn ctrl_and_the_wheel_steps_through_the_font_sizes_with_the_text_tool() {
        let mut state = editing(Tool::Text);
        state.font.size = 16;
        assert_eq!(
            wheel_pick(&state, &PALETTE, CTRL, DOWN),
            Some(Pick::FontSize(24))
        );
        state.font.size = 12;
        assert_eq!(
            wheel_pick(&state, &PALETTE, CTRL, UP),
            Some(Pick::FontSize(32))
        );
    }

    #[test]
    fn a_width_that_is_not_a_choice_steps_to_the_nearest_choice_in_that_direction() {
        // 設定の太さは 1〜64 のどれでもよい
        let mut state = editing(Tool::Rect);
        state.style.width = 4;
        assert_eq!(
            wheel_pick(&state, &PALETTE, CTRL, DOWN),
            Some(Pick::Width(5))
        );
        assert_eq!(wheel_pick(&state, &PALETTE, CTRL, UP), Some(Pick::Width(3)));
        state.style.width = 64;
        assert_eq!(
            wheel_pick(&state, &PALETTE, CTRL, DOWN),
            Some(Pick::Width(2))
        );
        state.style.width = 1;
        assert_eq!(wheel_pick(&state, &PALETTE, CTRL, UP), Some(Pick::Width(8)));
    }

    #[test]
    fn while_retyping_the_wheel_steps_from_the_colour_and_size_of_the_text_being_typed() {
        use crate::annotate::{EditInput, Pick};
        use crate::keys::{Chord, Key};
        let mut state = editing(Tool::Text);
        let mut annotations = Annotations::default();
        let at = Point::new(10, 10);
        state.input(EditInput::Pick(Pick::Color(PALETTE[3])), &mut annotations);
        state.input(EditInput::Pick(Pick::FontSize(32)), &mut annotations);
        state.input(EditInput::Press(at), &mut annotations);
        state.input(EditInput::Char('a'), &mut annotations);
        state.input(
            EditInput::Key(Chord::plain(Key::known(0x1B))),
            &mut annotations,
        );
        state.input(EditInput::Pick(Pick::Color(PALETTE[0])), &mut annotations);
        state.input(EditInput::Pick(Pick::FontSize(16)), &mut annotations);
        state.input(EditInput::DoubleClick(Point::new(12, 12)), &mut annotations);
        assert_eq!(state.retyping(), Some(0));
        let shift = Modifiers {
            shift: true,
            ..PLAIN
        };
        assert_eq!(
            wheel_pick(&state, &PALETTE, shift, DOWN),
            Some(Pick::Color(PALETTE[4]))
        );
        assert_eq!(
            wheel_pick(&state, &PALETTE, CTRL, DOWN),
            Some(Pick::FontSize(12))
        );
    }

    #[test]
    fn the_wheel_picks_nothing_with_other_modifiers() {
        let state = editing(Tool::Rect);
        for modifiers in [
            Modifiers { alt: true, ..PLAIN },
            Modifiers { win: true, ..PLAIN },
            Modifiers {
                shift: true,
                ..CTRL
            },
        ] {
            assert_eq!(wheel_pick(&state, &PALETTE, modifiers, DOWN), None);
        }
    }

    #[test]
    fn a_colour_that_is_not_in_the_palette_steps_to_either_end_of_it() {
        let shift = Modifiers {
            shift: true,
            ..PLAIN
        };
        let mut state = editing(Tool::Rect);
        state.style.color = Rgb(1, 2, 3);
        assert_eq!(
            wheel_pick(&state, &PALETTE, shift, DOWN),
            Some(Pick::Color(PALETTE[0]))
        );
        assert_eq!(
            wheel_pick(&state, &PALETTE, shift, UP),
            Some(Pick::Color(PALETTE[7]))
        );
    }

    #[test]
    fn a_colour_or_width_from_the_wheel_restyles_the_selected_annotation_and_can_be_undone() {
        use crate::annotate::EditInput;
        let mut state = editing(Tool::Rect);
        state.style = Style {
            color: PALETTE[0],
            width: 3,
        };
        let mut annotations = Annotations::default();
        state.input(EditInput::Press(Point::new(10, 10)), &mut annotations);
        state.input(
            EditInput::Move {
                at: Point::new(50, 50),
                shift: false,
            },
            &mut annotations,
        );
        state.input(EditInput::Release, &mut annotations);
        state.input(EditInput::Pick(Pick::Tool(Tool::Select)), &mut annotations);
        state.input(EditInput::Press(Point::new(10, 30)), &mut annotations);
        state.input(EditInput::Release, &mut annotations);
        assert_eq!(state.selected(), [0]);
        let shift = Modifiers {
            shift: true,
            ..PLAIN
        };
        for (modifiers, expected) in [
            (
                shift,
                Style {
                    color: PALETTE[1],
                    width: 3,
                },
            ),
            (
                CTRL,
                Style {
                    color: PALETTE[1],
                    width: 5,
                },
            ),
        ] {
            let pick = wheel_pick(&state, &PALETTE, modifiers, DOWN).unwrap();
            state.input(EditInput::Pick(pick), &mut annotations);
            assert_eq!(annotations.items()[0].style, expected);
        }
        state.input(EditInput::Undo, &mut annotations);
        state.input(EditInput::Undo, &mut annotations);
        assert_eq!(
            annotations.items()[0].style,
            Style {
                color: PALETTE[0],
                width: 3,
            }
        );
    }

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
    fn the_toolbar_lines_up_tools_colours_widths_history_done_and_the_grip_at_the_end() {
        let layout = Layout::new(&PALETTE, 1.0);
        let order: Vec<Button> = layout.buttons().iter().map(|&(b, _)| b).collect();
        let mut expected: Vec<Button> = ToolSlot::ALL.iter().map(|&s| Button::Tool(s)).collect();
        expected.extend(PALETTE.iter().map(|&c| Button::Color(c)));
        expected.extend(WIDTHS.iter().map(|&w| Button::Width(w)));
        expected.extend([
            Button::Undo,
            Button::Redo,
            Button::Delete,
            Button::Done,
            Button::Grip,
        ]);
        assert_eq!(order, expected);
        // 8 + 8 + 4 + 4 個の 28px のボタン、間 2px、組の間 10px、つまみ 14px、外の余白 4px
        assert_eq!(
            layout.size(),
            Size::new(4 + 24 * 28 + 20 * 2 + 4 * 10 + 14 + 4, 36)
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
        // 色の組は、道具の組から 10px 空けて始まる（8 個目の右端 32+7*30=242、そこから 10px）
        assert_eq!(layout.hit(Point::new(245, 20)), None);
        assert_eq!(
            layout.hit(Point::new(252, 20)),
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
        let annotations = crate::annotate::Annotations::default();
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
        assert_eq!(
            Button::Tool(ToolSlot::Text).pick(),
            Some(Pick::Tool(Tool::Text))
        );
        assert!(Button::Tool(ToolSlot::Text).is_enabled(&state, &annotations));
        assert!(Button::Tool(ToolSlot::Arrow).is_enabled(&state, &annotations));
        assert_eq!(Button::Done.pick(), None);
        assert_eq!(Button::Grip.pick(), None);
        assert!(
            Button::Done.is_enabled(&state, &annotations)
                && Button::Grip.is_enabled(&state, &annotations)
        );
    }

    #[test]
    fn the_select_button_picks_the_select_tool_and_delete_is_enabled_only_with_a_selection() {
        use crate::annotate::{Annotations, EditInput};
        let select = Button::Tool(ToolSlot::Select);
        assert_eq!(select.pick(), Some(Pick::Tool(Tool::Select)));
        let mut state = EditState::new(crate::annotate::Style::default());
        let mut annotations = Annotations::default();
        assert!(select.is_enabled(&state, &annotations));
        assert!(!Button::Delete.is_enabled(&state, &annotations));
        assert_eq!(Button::Delete.pick(), None);
        // 線を描いて選ぶ
        state.input(EditInput::Press(Point::new(0, 0)), &mut annotations);
        state.input(
            EditInput::Move {
                at: Point::new(20, 0),
                shift: false,
            },
            &mut annotations,
        );
        state.input(EditInput::Release, &mut annotations);
        state.input(EditInput::Pick(Pick::Tool(Tool::Select)), &mut annotations);
        assert!(select.is_current(&state));
        state.input(EditInput::Press(Point::new(10, 0)), &mut annotations);
        state.input(EditInput::Release, &mut annotations);
        assert!(Button::Delete.is_enabled(&state, &annotations));
    }

    #[test]
    fn undo_and_redo_are_pressable_only_when_there_is_something_to_take_back_or_redo() {
        use crate::annotate::{Annotations, EditInput};
        let mut state = EditState::new(crate::annotate::Style::default());
        let mut annotations = Annotations::default();
        let enabled = |state: &EditState, annotations: &Annotations| {
            (
                Button::Undo.is_enabled(state, annotations),
                Button::Redo.is_enabled(state, annotations),
            )
        };
        assert_eq!(enabled(&state, &annotations), (false, false));
        state.input(EditInput::Press(Point::new(0, 0)), &mut annotations);
        state.input(
            EditInput::Move {
                at: Point::new(20, 0),
                shift: false,
            },
            &mut annotations,
        );
        state.input(EditInput::Release, &mut annotations);
        assert_eq!(enabled(&state, &annotations), (true, false));
        state.input(EditInput::Undo, &mut annotations);
        assert_eq!(enabled(&state, &annotations), (false, true));
        assert_eq!(Button::Undo.pick(), None);
        assert!(!Button::Undo.is_current(&state));
    }

    #[test]
    fn with_the_text_tool_the_widths_turn_into_font_sizes_in_the_same_place() {
        use crate::annotate::{Annotations, EditInput};
        let mut state = EditState::new(crate::annotate::Style::default());
        let mut annotations = Annotations::default();
        let drawing = Layout::for_state(&PALETTE, 1.0, &state);
        assert_eq!(drawing, Layout::new(&PALETTE, 1.0));
        state.input(EditInput::Pick(Pick::Tool(Tool::Text)), &mut annotations);
        let text = Layout::for_state(&PALETTE, 1.0, &state);
        assert_eq!(text.size(), drawing.size());
        let swapped: Vec<_> = drawing
            .buttons()
            .iter()
            .zip(text.buttons())
            .filter(|(a, b)| a != b)
            .map(|(&(a, ar), &(b, br))| (a, b, ar == br))
            .collect();
        assert_eq!(
            swapped,
            WIDTHS
                .iter()
                .zip(FONT_SIZES)
                .map(|(&w, s)| (Button::Width(w), Button::FontSize(s), true))
                .collect::<Vec<_>>()
        );
        assert_eq!(Button::FontSize(24).pick(), Some(Pick::FontSize(24)));
        assert!(Button::FontSize(16).is_current(&state));
        state.input(EditInput::Pick(Pick::FontSize(24)), &mut annotations);
        assert!(Button::FontSize(24).is_current(&state));
        assert!(!Button::FontSize(16).is_current(&state));
        // 描く道具に戻せば太さに戻る
        state.input(EditInput::Pick(Pick::Tool(Tool::Rect)), &mut annotations);
        assert_eq!(Layout::for_state(&PALETTE, 1.0, &state), drawing);
    }
}
