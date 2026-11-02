//! 書き込み（スクラップの上に描き加える図形や文字）と、書き込みの編集モードの状態（ADR-0006）。
//!
//! 書き込みは元の画像（トリム前）の画素の座標で持ち、画像には焼き込まない。スクラップの書き込みは
//! `ScrapModel::annotations` に、編集モードの間だけの状態（描きかけの図形、選んでいる道具と色と太さ）は
//! `scrap::Mode::Editing` の `EditState` に置く。モードを抜けても書き込みは残る。
//!
//! 編集モードへの入力は、スクラップ操作の表を通さず `ScrapModel::edit_input` が受ける。`ScrapModel` が
//! スクリーン座標を元の画像の画素へ直してから `EditState::input` に渡すので、ここは座標の換算を知らない。
//! 結果は `EditOutput` で返し、Win32 が要る仕事（描き直しなど）は `app` がする。
//!
//! 表示と保存・コピーは、どちらも `View`（元の画像の画素 → 描く先の座標）を通して同じ描画処理で描く。
//!
//! 道具は矩形・楕円・矢印・線・フリーハンド（`Tool`）で、編集モードの間は数字キーで選ぶ（`Tool::from_key`。
//! 5 は文字のために空けてある）。`Shift` を押しながら描くと、矩形は正方形、楕円は円、線と矢印は 45° 刻みに
//! なる（`Draft::end`）。道具を足すときは `Tool` と `Shape` に行を足し、`Draft::shape` と番号の表を足す。

use crate::geometry::Point;
use crate::keys::{Chord, Key, Modifiers};
use crate::settings::{AnnotationDefaults, Rgb};

/// 書き込みの色と太さ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub color: Rgb,
    /// 太さ（元の画像の画素）。拡大縮小すると表示も一緒に太く・細くなる。
    pub width: u32,
}

impl From<AnnotationDefaults> for Style {
    fn from(defaults: AnnotationDefaults) -> Style {
        Style {
            color: defaults.color,
            width: defaults.width,
        }
    }
}

impl Default for Style {
    fn default() -> Style {
        Style::from(AnnotationDefaults::default())
    }
}

/// 書き込みの形。座標は元の画像の画素（その画素の中心を通って描く）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// 矩形。`from` と `to` はドラッグの両端（向きはそのまま）で、対角の 2 つの角。
    Rect { from: Point, to: Point },
    /// 楕円。`from` と `to` は外接する矩形の対角の 2 つの角（`Rect` と同じ）。
    Ellipse { from: Point, to: Point },
    /// 矢印。`from` から `to` へ引き、頭は `to` の側に 1 つ（大きさは太さに比例する。`arrow_head`）。
    Arrow { from: Point, to: Point },
    /// 線。`from` から `to` へ。
    Line { from: Point, to: Point },
    /// フリーハンド。通った点（2 つ以上）を順に線で結ぶ。滑らかにはせず、点を間引くだけ（`FREEHAND_STEP`）。
    Freehand { points: Vec<Point> },
}

/// 1 つの書き込み。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub shape: Shape,
    pub style: Style,
}

/// 1 枚のスクラップの書き込み。描いた順に並べ、後のものほど上に描く。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Annotations {
    items: Vec<Annotation>,
}

impl Annotations {
    /// 描いた順の書き込み。
    pub fn items(&self) -> &[Annotation] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn push(&mut self, annotation: Annotation) {
        self.items.push(annotation);
    }
}

/// 編集モードで左ドラッグが描くもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Rect,
    Ellipse,
    Arrow,
    Line,
    Freehand,
}

impl Tool {
    /// 数字キー（上の段とテンキー、修飾キーなし）で選ぶ道具。番号は 1 から矩形・楕円・矢印・線・文字・
    /// フリーハンドの順で、キーは固定（割り当て直しの対象外）。
    fn from_key(chord: Chord) -> Option<Tool> {
        if chord.modifiers != Modifiers::default() {
            return None;
        }
        let n = match chord.key.vk() {
            vk @ 0x31..=0x39 => vk - 0x30,
            vk @ 0x61..=0x69 => vk - 0x60,
            _ => return None,
        };
        match n {
            1 => Some(Tool::Rect),
            2 => Some(Tool::Ellipse),
            3 => Some(Tool::Arrow),
            4 => Some(Tool::Line),
            // 5 は文字（#87）
            6 => Some(Tool::Freehand),
            _ => None,
        }
    }
}

/// 描きかけの図形。左ボタンを押してから離すまで。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    from: Point,
    /// カーソルのいる画素（`Shift` の制約をかける前）。
    to: Point,
    /// 最後に動いたときに `Shift` を押していたか。
    shift: bool,
    /// フリーハンドで残す点（`from` から、`FREEHAND_STEP` 以上離れるごとに 1 つ）。`to` は含まない。
    trail: Vec<Point>,
}

impl Draft {
    fn new(at: Point) -> Draft {
        Draft {
            from: at,
            to: at,
            shift: false,
            trail: vec![at],
        }
    }

    /// カーソルが `at` に動いた。フリーハンドの点は、最後に残した点から `FREEHAND_STEP` 以上離れたら残す。
    fn move_to(&mut self, at: Point, shift: bool) {
        self.to = at;
        self.shift = shift;
        let last = self.trail.last().copied().unwrap_or(self.from);
        let (dx, dy) = (i64::from(at.x - last.x), i64::from(at.y - last.y));
        if dx * dx + dy * dy >= FREEHAND_STEP * FREEHAND_STEP {
            self.trail.push(at);
        }
    }

    /// いまの道具で作る図形。長さ 0（押した画素から動いていない）なら None で、描かずに捨てる。
    fn shape(&self, tool: Tool) -> Option<Shape> {
        if tool == Tool::Freehand {
            let mut points = self.trail.clone();
            if points.last() != Some(&self.to) {
                points.push(self.to);
            }
            return (points.len() >= 2).then_some(Shape::Freehand { points });
        }
        let (from, to) = (self.from, self.end(tool));
        if from == to {
            return None;
        }
        match tool {
            Tool::Rect => Some(Shape::Rect { from, to }),
            Tool::Ellipse => Some(Shape::Ellipse { from, to }),
            Tool::Arrow => Some(Shape::Arrow { from, to }),
            Tool::Line => Some(Shape::Line { from, to }),
            Tool::Freehand => None,
        }
    }

    /// `Shift` の制約をかけた終わりの点。矩形と楕円は長いほうの辺にそろえた正方形にする。線と矢印は
    /// いちばん近い 45° 刻みの向きにする（斜めは縦と横の長さの平均）。
    fn end(&self, tool: Tool) -> Point {
        if !self.shift {
            return self.to;
        }
        let (dx, dy) = (self.to.x - self.from.x, self.to.y - self.from.y);
        match tool {
            Tool::Rect | Tool::Ellipse => {
                let side = dx.abs().max(dy.abs());
                Point::new(self.from.x + side * sign(dx), self.from.y + side * sign(dy))
            }
            Tool::Freehand => self.to,
            Tool::Arrow | Tool::Line => {
                let (ax, ay) = (f64::from(dx.abs()), f64::from(dy.abs()));
                let (dx, dy) = if ay < ax * TAN_22_5 {
                    (dx, 0)
                } else if ax < ay * TAN_22_5 {
                    (0, dy)
                } else {
                    let d = (dx.abs() + dy.abs() + 1) / 2;
                    (d * sign(dx), d * sign(dy))
                };
                Point::new(self.from.x + dx, self.from.y + dy)
            }
        }
    }
}

/// フリーハンドで点を残す間隔（元の画像の画素）。
const FREEHAND_STEP: i64 = 2;

/// 45° 刻みの境（22.5°）の傾き。
const TAN_22_5: f64 = 0.414_213_562_373_095_1;

/// 0 を正に数える符号（まっすぐなドラッグの正方形は右下へ広げる）。
fn sign(d: i32) -> i32 {
    if d < 0 { -1 } else { 1 }
}

/// 書き込みの編集モードの間だけの状態（`scrap::Mode::Editing`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditState {
    pub tool: Tool,
    /// これから描く書き込みの色と太さ。入ったときは設定の `[annotation]` の値。
    pub style: Style,
    draft: Option<Draft>,
}

impl EditState {
    pub fn new(style: Style) -> EditState {
        EditState {
            tool: Tool::default(),
            style,
            draft: None,
        }
    }

    /// 描きかけの書き込み（ドラッグ中だけ）。長さ 0 なら None。
    pub fn draft(&self) -> Option<Annotation> {
        let shape = self.draft.as_ref()?.shape(self.tool)?;
        Some(Annotation {
            shape,
            style: self.style,
        })
    }

    /// 入力を受けて、`annotations` を変える。座標は元の画像の画素（`ScrapModel::edit_input` が直したもの）。
    pub fn input(&mut self, input: EditInput, annotations: &mut Annotations) -> Vec<EditOutput> {
        match input {
            EditInput::Press(at) => {
                self.draft = Some(Draft::new(at));
                vec![]
            }
            EditInput::Move { at, shift } => match &mut self.draft {
                Some(draft) if (draft.to, draft.shift) != (at, shift) => {
                    draft.move_to(at, shift);
                    vec![EditOutput::Redraw]
                }
                _ => vec![],
            },
            EditInput::Release => self.commit(annotations),
            EditInput::Key(chord) if chord == ESCAPE => {
                let mut outputs = self.commit(annotations);
                outputs.push(EditOutput::Left);
                outputs
            }
            EditInput::Key(chord) => match Tool::from_key(chord) {
                Some(tool) => self.pick(tool, annotations),
                None => vec![],
            },
            EditInput::Leave => {
                let mut outputs = self.commit(annotations);
                outputs.push(EditOutput::Left);
                outputs
            }
        }
    }

    /// 道具を `tool` に替える。描きかけがあれば、前の道具のまま書き込みにしてから替える。
    fn pick(&mut self, tool: Tool, annotations: &mut Annotations) -> Vec<EditOutput> {
        let mut outputs = self.commit(annotations);
        self.tool = tool;
        outputs.push(EditOutput::ToolChanged);
        outputs
    }

    /// 描きかけを書き込みにする。長さ 0 なら捨てる。
    fn commit(&mut self, annotations: &mut Annotations) -> Vec<EditOutput> {
        let Some(draft) = self.draft.take() else {
            return vec![];
        };
        let Some(shape) = draft.shape(self.tool) else {
            return vec![];
        };
        annotations.push(Annotation {
            shape,
            style: self.style,
        });
        vec![EditOutput::Redraw]
    }
}

/// `Esc`。編集モードを抜ける。
const ESCAPE: Chord = Chord::plain(Key::known(0x1B));

/// 書き込みの編集モードへの入力。座標は、`ScrapModel::edit_input` に渡すときはスクリーン座標（物理ピクセル）、
/// `EditState::input` が受けるときは元の画像の画素。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditInput {
    /// 左ボタンを押した（ダブルクリックの 2 回目も）。
    Press(Point),
    /// カーソルが動いた。押している間だけ描きかけを変える。`shift` は `Shift` を押しているか（図形の制約）。
    Move { at: Point, shift: bool },
    /// 左ボタンを離した、またはマウスを捕まえていられなくなった。描きかけを書き込みにする。
    Release,
    /// キーを押した。`Esc` で抜け、数字キーで道具を選ぶ。ほかのキーは受けない（何も返さない）。
    Key(Chord),
    /// 抜ける（メニューの「書き込み」、ほかのウィンドウへのフォーカスの移動、スクラップを閉じる前）。
    Leave,
}

impl EditInput {
    /// 座標を `f` で直した入力。
    pub fn map_point(self, f: impl FnOnce(Point) -> Point) -> EditInput {
        match self {
            EditInput::Press(at) => EditInput::Press(f(at)),
            EditInput::Move { at, shift } => EditInput::Move { at: f(at), shift },
            other => other,
        }
    }
}

/// 編集モードへの入力の結果。空なら入力を受けなかった（キーなら Windows の既定の処理に回す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutput {
    /// 書き込みか描きかけが変わった。描き直す。
    Redraw,
    /// 道具が替わった（`EditState::tool`）。同じ道具を選び直しても返す。
    ToolChanged,
    /// 編集モードを抜けた。
    Left,
}

/// 矢印の頭（三角形）。座標は描く先のもの。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowHead {
    /// 矢印の先（終点）。
    pub tip: (f64, f64),
    /// 三角形の底辺の中点。矢印の軸はここまで引く（太い線の端が頭の先からはみ出さないように）。
    pub base: (f64, f64),
    /// 底辺の両端。矢印の向きに向かって左と右。
    pub left: (f64, f64),
    pub right: (f64, f64),
}

/// 太さ 1 あたりの矢印の頭の長さと、底辺の半分の幅。
const ARROW_HEAD_LENGTH: f64 = 4.0;
const ARROW_HEAD_HALF_WIDTH: f64 = 2.0;

/// `from` から `to` への矢印（描く先の座標と太さ）の頭。大きさは太さに比例し、矢印が頭より短ければ頭を
/// 矢印の長さに縮める。長さ 0 なら None。
pub fn arrow_head(from: (f64, f64), to: (f64, f64), width: f64) -> Option<ArrowHead> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return None;
    }
    let (ux, uy) = (dx / length, dy / length);
    let head = (width * ARROW_HEAD_LENGTH).min(length);
    let half = head * ARROW_HEAD_HALF_WIDTH / ARROW_HEAD_LENGTH;
    let base = (to.0 - ux * head, to.1 - uy * head);
    // 矢印の向きを右に 90° 回した向き（画面の座標は y が下向き）
    let (nx, ny) = (-uy, ux);
    Some(ArrowHead {
        tip: to,
        base,
        left: (base.0 - nx * half, base.1 - ny * half),
        right: (base.0 + nx * half, base.1 + ny * half),
    })
}

/// 書き込みを描く先への変換。元の画像の画素 `p` の中心を、描く先の `((p.x - left + 0.5) * scale_x,
/// (p.y - top + 0.5) * scale_y)` に置く。太さには `scale_x` を掛ける。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// 描く先の左上に来る、元の画像の画素の位置。
    pub left: f64,
    pub top: f64,
    /// 元の画像の 1 画素の、描く先での長さ。
    pub scale_x: f64,
    pub scale_y: f64,
}

impl View {
    /// 元の画像の画素 `p` の中心の、描く先での座標。
    pub fn point(&self, p: Point) -> (f64, f64) {
        (
            (f64::from(p.x) - self.left + 0.5) * self.scale_x,
            (f64::from(p.y) - self.top + 0.5) * self.scale_y,
        )
    }

    /// 太さ `width`（元の画像の画素）の、描く先での太さ。
    pub fn width(&self, width: u32) -> f64 {
        f64::from(width) * self.scale_x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Rgb;

    fn p(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    fn digit(n: u16) -> EditInput {
        EditInput::Key(Chord::plain(Key::known(0x30 + n)))
    }

    const BLUE: Style = Style {
        color: Rgb(0x1E, 0x88, 0xE5),
        width: 5,
    };

    /// `from` から `to` へ 1 回でドラッグして離す。
    fn drag(state: &mut EditState, annotations: &mut Annotations, from: Point, to: Point) {
        state.input(EditInput::Press(from), annotations);
        state.input(
            EditInput::Move {
                at: to,
                shift: false,
            },
            annotations,
        );
        state.input(EditInput::Release, annotations);
    }

    /// `Shift` を押しながら `from` から `to` へドラッグして離した書き込みの形。
    fn shift_drag(tool: Tool, from: Point, to: Point) -> Option<Shape> {
        let mut state = EditState::new(BLUE);
        state.tool = tool;
        let mut annotations = Annotations::default();
        state.input(EditInput::Press(from), &mut annotations);
        state.input(
            EditInput::Move {
                at: to,
                shift: true,
            },
            &mut annotations,
        );
        state.input(EditInput::Release, &mut annotations);
        annotations.items().first().map(|a| a.shape.clone())
    }

    #[test]
    fn shift_makes_the_rect_a_square_and_the_ellipse_a_circle_on_the_longer_side() {
        assert_eq!(
            shift_drag(Tool::Rect, p(10, 20), p(40, 30)),
            Some(Shape::Rect {
                from: p(10, 20),
                to: p(40, 50),
            })
        );
        // 左上へのドラッグは左上へ広がる
        assert_eq!(
            shift_drag(Tool::Ellipse, p(50, 50), p(45, 20)),
            Some(Shape::Ellipse {
                from: p(50, 50),
                to: p(20, 20),
            })
        );
        // 縦か横にまっすぐなドラッグも正方形になる
        assert_eq!(
            shift_drag(Tool::Rect, p(10, 10), p(10, 30)),
            Some(Shape::Rect {
                from: p(10, 10),
                to: p(30, 30),
            })
        );
    }

    #[test]
    fn the_second_number_key_picks_the_ellipse_and_a_drag_draws_one_with_the_style() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        assert_ne!(state.input(digit(2), &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Ellipse);
        drag(&mut state, &mut annotations, p(10, 20), p(40, 30));
        assert_eq!(
            annotations.items(),
            &[Annotation {
                shape: Shape::Ellipse {
                    from: p(10, 20),
                    to: p(40, 30),
                },
                style: BLUE,
            }]
        );
    }

    #[test]
    fn the_third_and_fourth_number_keys_draw_an_arrow_and_a_line_from_start_to_end() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(3), &mut annotations);
        assert_eq!(state.tool, Tool::Arrow);
        drag(&mut state, &mut annotations, p(40, 30), p(10, 20));
        state.input(digit(4), &mut annotations);
        assert_eq!(state.tool, Tool::Line);
        drag(&mut state, &mut annotations, p(1, 2), p(3, 4));
        let shapes: Vec<_> = annotations
            .items()
            .iter()
            .map(|a| a.shape.clone())
            .collect();
        assert_eq!(
            shapes,
            vec![
                Shape::Arrow {
                    from: p(40, 30),
                    to: p(10, 20),
                },
                Shape::Line {
                    from: p(1, 2),
                    to: p(3, 4),
                },
            ]
        );
        assert!(annotations.items().iter().all(|a| a.style == BLUE));
    }

    #[test]
    fn shift_snaps_lines_and_arrows_to_the_nearest_45_degrees() {
        let line = |from, to| Shape::Line { from, to };
        // ほぼ横は横、ほぼ縦は縦
        assert_eq!(
            shift_drag(Tool::Line, p(10, 10), p(50, 18)),
            Some(line(p(10, 10), p(50, 10)))
        );
        assert_eq!(
            shift_drag(Tool::Line, p(10, 10), p(4, -30)),
            Some(line(p(10, 10), p(10, -30)))
        );
        // 斜めは縦と横の平均の長さで 45°
        assert_eq!(
            shift_drag(Tool::Line, p(10, 10), p(-20, 34)),
            Some(line(p(10, 10), p(-17, 37)))
        );
        assert_eq!(
            shift_drag(Tool::Arrow, p(0, 0), p(30, 20)),
            Some(Shape::Arrow {
                from: p(0, 0),
                to: p(25, 25),
            })
        );
    }

    #[test]
    fn the_sixth_number_key_draws_freehand_keeping_only_points_a_step_apart_and_the_last() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(6), &mut annotations);
        assert_eq!(state.tool, Tool::Freehand);
        state.input(EditInput::Press(p(0, 0)), &mut annotations);
        for (at, shift) in [
            (p(1, 0), false),
            (p(2, 0), true),
            (p(3, 0), false),
            (p(4, 1), false),
            (p(4, 2), false),
        ] {
            state.input(EditInput::Move { at, shift }, &mut annotations);
        }
        let points = vec![p(0, 0), p(2, 0), p(4, 1), p(4, 2)];
        // 描きかけも同じ点で描く
        assert_eq!(
            state.draft().map(|a| a.shape),
            Some(Shape::Freehand {
                points: points.clone(),
            })
        );
        state.input(EditInput::Release, &mut annotations);
        assert_eq!(
            annotations.items(),
            &[Annotation {
                shape: Shape::Freehand { points },
                style: BLUE,
            }]
        );
    }

    #[test]
    fn a_freehand_stroke_that_ends_where_it_started_without_leaving_is_dropped() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(6), &mut annotations);
        state.input(EditInput::Press(p(5, 5)), &mut annotations);
        state.input(
            EditInput::Move {
                at: p(6, 5),
                shift: false,
            },
            &mut annotations,
        );
        state.input(
            EditInput::Move {
                at: p(5, 5),
                shift: false,
            },
            &mut annotations,
        );
        assert_eq!(state.input(EditInput::Release, &mut annotations), vec![]);
        assert!(annotations.is_empty());
    }

    #[test]
    fn the_fifth_number_key_is_kept_for_text_and_other_keys_are_not_taken() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(3), &mut annotations);
        assert_eq!(state.input(digit(5), &mut annotations), vec![]);
        assert_eq!(state.input(digit(7), &mut annotations), vec![]);
        // 修飾キーつきの数字は道具を選ばない
        let ctrl_1 = EditInput::Key(Chord::plain(Key::known(0x31)).ctrl());
        assert_eq!(state.input(ctrl_1, &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Arrow);
        // テンキーでも選べる
        let numpad_1 = EditInput::Key(Chord::plain(Key::known(0x61)));
        assert_ne!(state.input(numpad_1, &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Rect);
    }

    #[test]
    fn switching_the_tool_mid_drag_keeps_what_was_drawn_with_the_old_tool() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(EditInput::Press(p(0, 0)), &mut annotations);
        state.input(
            EditInput::Move {
                at: p(9, 9),
                shift: false,
            },
            &mut annotations,
        );
        assert_eq!(
            state.input(digit(4), &mut annotations),
            vec![EditOutput::Redraw, EditOutput::ToolChanged]
        );
        assert_eq!(
            annotations.items()[0].shape,
            Shape::Rect {
                from: p(0, 0),
                to: p(9, 9),
            }
        );
        assert_eq!(state.draft(), None);
    }

    #[test]
    fn the_arrow_head_sits_on_the_end_and_grows_with_the_width() {
        assert_eq!(
            arrow_head((0.0, 0.0), (100.0, 0.0), 3.0),
            Some(ArrowHead {
                tip: (100.0, 0.0),
                base: (88.0, 0.0),
                left: (88.0, -6.0),
                right: (88.0, 6.0),
            })
        );
        let head = arrow_head((10.0, 50.0), (10.0, 10.0), 2.0).unwrap();
        assert_eq!((head.tip, head.base), ((10.0, 10.0), (10.0, 18.0)));
        assert_eq!((head.left, head.right), ((6.0, 18.0), (14.0, 18.0)));
        // 頭より短い矢印は、頭を矢印の長さに縮める
        let head = arrow_head((0.0, 0.0), (5.0, 0.0), 3.0).unwrap();
        assert_eq!((head.base, head.left), ((0.0, 0.0), (0.0, -2.5)));
        assert_eq!(arrow_head((1.0, 1.0), (1.0, 1.0), 3.0), None);
    }
}
