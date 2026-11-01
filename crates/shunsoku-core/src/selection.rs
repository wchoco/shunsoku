//! 範囲選択の画面の状態機械（通常モード）。
//!
//! - 入力（`Input`）は Win32 層がマウスとキーのイベントから作る。座標はすべてスクリーン座標（物理ピクセル）。
//! - モニタの一覧（`MonitorInfo`）は Win32 層が集めて渡す。
//! - 範囲は、始点（押した画素、または `Space` を押したときのカーソルの画素）とカーソルの画素の両方を含む矩形。
//! - キーボードの操作（矢印・`Space`・`Ctrl+A`・`Ctrl+Shift+A`）もここで状態に変える。Win32 層はキーを
//!   `Input` にして渡し、矢印で動いた `cursor` に実際のマウスのカーソルを合わせるだけにする。
//! - 矢印で動かすカーソルは、どれかのモニタの画素に留める（隙間や画面の外には出ない）。

use crate::geometry::{Point, Rect};

/// 1 枚のモニタ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorInfo {
    /// スクリーン座標（物理ピクセル）での位置と大きさ。
    pub rect: Rect,
    /// 実効 DPI（100% で 96）。
    pub dpi: u32,
}

/// マウスのボタン。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// 矢印キーの向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// 矢印でカーソルを動かす量（物理ピクセル）。
const ARROW_STEP: i32 = 1;
/// Shift+矢印でカーソルを動かす量（物理ピクセル）。
const SHIFT_ARROW_STEP: i32 = 10;

/// 範囲選択の画面への入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Press {
        button: Button,
        at: Point,
    },
    Move {
        at: Point,
    },
    Release {
        button: Button,
        at: Point,
    },
    /// `Esc`。1 段戻る。
    Escape,
    /// 矢印（`shift` なら Shift+矢印）。カーソルを動かす。
    Arrow {
        direction: Direction,
        shift: bool,
    },
    /// `Space`。何も選んでいなければカーソルの画素を始点にし、選んでいれば範囲を確定する。
    Space,
    /// `Ctrl+A`。カーソルのあるモニタ全体を選んで確定する。
    SelectMonitor,
    /// `Ctrl+Shift+A`。全モニタを囲む矩形を選んで確定する（モニタの隙間は撮るときに透明になる）。
    SelectAllMonitors,
}

/// 状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// 何も選んでいない。
    Idle,
    /// 範囲を選んでいる。
    Selecting,
    /// 範囲が決まった（`Selection::rect`）。
    Confirmed,
    /// 中止した。
    Cancelled,
}

/// 範囲選択の状態。
#[derive(Debug, Clone)]
pub struct Selection {
    monitors: Vec<MonitorInfo>,
    state: State,
    /// 範囲の始点。`Selecting` と `Confirmed` のときだけ Some。
    anchor: Option<Point>,
    /// カーソルの画素。キーボードで動かす操作もこれを動かす。
    cursor: Point,
}

impl Selection {
    /// `cursor` は開いた時点のカーソルの位置。
    pub fn new(monitors: Vec<MonitorInfo>, cursor: Point) -> Selection {
        Selection {
            monitors,
            state: State::Idle,
            anchor: None,
            cursor,
        }
    }

    /// 入力を 1 つ受けて、受けたあとの状態を返す。確定・中止のあとの入力は無視する。
    pub fn handle(&mut self, input: Input) -> State {
        if matches!(self.state, State::Confirmed | State::Cancelled) {
            return self.state;
        }
        match input {
            Input::Press {
                button: Button::Left,
                at,
            } => {
                self.cursor = at;
                if self.state == State::Idle {
                    self.anchor = Some(at);
                    self.state = State::Selecting;
                }
            }
            Input::Move { at } => self.cursor = at,
            Input::Release {
                button: Button::Left,
                at,
            } => {
                self.cursor = at;
                if self.state == State::Selecting {
                    if self.anchor == Some(at) {
                        // 動かさずに離した（クリック）。何も選ばなかったことにする
                        self.reset();
                    } else {
                        self.state = State::Confirmed;
                    }
                }
            }
            Input::Press {
                button: Button::Right,
                ..
            }
            | Input::Escape => self.back(),
            Input::Release {
                button: Button::Right,
                ..
            } => {}
            Input::Arrow { direction, shift } => self.move_cursor(direction, shift),
            Input::Space => match self.state {
                State::Idle => {
                    self.anchor = Some(self.cursor);
                    self.state = State::Selecting;
                }
                // 始点と同じ画素でも、キーで 2 度押したなら 1 画素の範囲として確定する
                _ => self.state = State::Confirmed,
            },
            Input::SelectMonitor => {
                if let Some(rect) = self.monitor_at(self.cursor).map(|m| m.rect) {
                    self.confirm(rect);
                }
            }
            Input::SelectAllMonitors => {
                let all = self.monitors.iter().map(|m| m.rect).reduce(|a, b| {
                    Rect::new(
                        a.left.min(b.left),
                        a.top.min(b.top),
                        a.right.max(b.right),
                        a.bottom.max(b.bottom),
                    )
                });
                if let Some(rect) = all {
                    self.confirm(rect);
                }
            }
        }
        self.state
    }

    /// 1 段戻る。選んでいれば選び直し、何も選んでいなければ中止。
    fn back(&mut self) {
        match self.state {
            State::Selecting => self.reset(),
            State::Idle => self.state = State::Cancelled,
            State::Confirmed | State::Cancelled => {}
        }
    }

    /// 矢印でカーソルを動かす。
    fn move_cursor(&mut self, direction: Direction, shift: bool) {
        let step = if shift { SHIFT_ARROW_STEP } else { ARROW_STEP };
        let (dx, dy) = match direction {
            Direction::Left => (-step, 0),
            Direction::Right => (step, 0),
            Direction::Up => (0, -step),
            Direction::Down => (0, step),
        };
        let to = Point::new(self.cursor.x + dx, self.cursor.y + dy);
        if self.monitor_at(to).is_some() {
            self.cursor = to;
        } else if let Some(monitor) = self.monitor_at(self.cursor) {
            // 隙間や画面の外には出ず、いまのモニタの端で止まる
            let r = monitor.rect;
            self.cursor = Point::new(
                to.x.clamp(r.left, r.right - 1),
                to.y.clamp(r.top, r.bottom - 1),
            );
        }
    }

    /// `rect` を範囲にして確定する。
    fn confirm(&mut self, rect: Rect) {
        self.anchor = Some(rect.origin());
        self.cursor = Point::new(rect.right - 1, rect.bottom - 1);
        self.state = State::Confirmed;
    }

    /// `at` の画素を含むモニタ。
    fn monitor_at(&self, at: Point) -> Option<&MonitorInfo> {
        self.monitors.iter().find(|m| m.rect.contains(at))
    }

    /// 何も選んでいない状態に戻す（選び直し）。
    fn reset(&mut self) {
        self.anchor = None;
        self.state = State::Idle;
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// 選んでいる（決まった）範囲。何も選んでいなければ None。
    pub fn rect(&self) -> Option<Rect> {
        let a = self.anchor?;
        let c = self.cursor;
        Some(Rect::new(
            a.x.min(c.x),
            a.y.min(c.y),
            a.x.max(c.x) + 1,
            a.y.max(c.y) + 1,
        ))
    }

    /// カーソルの画素。
    pub fn cursor(&self) -> Point {
        self.cursor
    }

    pub fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }

    /// 範囲のモニタごとの部分。何も選んでいなければ空。
    pub fn parts(&self) -> Vec<Part> {
        self.rect()
            .map_or_else(Vec::new, |rect| split(rect, &self.monitors))
    }

    /// 範囲の左上の外側に出すサイズ（`1280 × 720`）。モニタをまたいでいれば、モニタごとのサイズを
    /// 左から `＋` でつなぐ。何も選んでいなければ None。
    pub fn size_label(&self) -> Option<String> {
        let rect = self.rect()?;
        let parts = split(rect, &self.monitors);
        let label = if parts.len() <= 1 {
            size_text(rect)
        } else {
            parts
                .iter()
                .map(|part| size_text(part.rect))
                .collect::<Vec<_>>()
                .join(" ＋ ")
        };
        Some(label)
    }
}

fn size_text(rect: Rect) -> String {
    format!("{} × {}", rect.width(), rect.height())
}

/// 範囲のうち、1 枚のモニタに重なる部分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// `MonitorInfo` の一覧での番号。
    pub monitor: usize,
    /// 重なる部分（スクリーン座標）。
    pub rect: Rect,
}

/// `rect` をモニタごとの部分に分ける。どのモニタにも重ならない部分は含めない。
/// 並びは左から右（同じ左端なら上から下）。
pub fn split(rect: Rect, monitors: &[MonitorInfo]) -> Vec<Part> {
    let mut parts: Vec<Part> = monitors
        .iter()
        .enumerate()
        .filter_map(|(monitor, info)| {
            info.rect
                .intersect(&rect)
                .map(|rect| Part { monitor, rect })
        })
        .collect();
    parts.sort_by_key(|part| (part.rect.left, part.rect.top));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    /// 1920×1080 @100% の 1 枚。
    fn one_monitor() -> Selection {
        Selection::new(
            vec![MonitorInfo {
                rect: Rect::new(0, 0, 1920, 1080),
                dpi: 96,
            }],
            p(0, 0),
        )
    }

    #[test]
    fn dragging_selects_both_end_pixels_and_releasing_confirms() {
        let mut s = one_monitor();
        assert_eq!(s.state(), State::Idle);
        s.handle(Input::Press {
            button: Button::Left,
            at: p(10, 20),
        });
        s.handle(Input::Move { at: p(109, 69) });
        assert_eq!(s.state(), State::Selecting);
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 110, 70)));
        let state = s.handle(Input::Release {
            button: Button::Left,
            at: p(109, 69),
        });
        assert_eq!(state, State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 110, 70)));
    }

    #[test]
    fn a_click_without_dragging_does_nothing() {
        let mut s = one_monitor();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(10, 20),
        });
        let state = s.handle(Input::Release {
            button: Button::Left,
            at: p(10, 20),
        });
        assert_eq!(state, State::Idle);
        assert_eq!(s.rect(), None);
    }

    fn start_drag(s: &mut Selection) {
        s.handle(Input::Press {
            button: Button::Left,
            at: p(10, 20),
        });
        s.handle(Input::Move { at: p(300, 200) });
    }

    #[test]
    fn escape_while_selecting_starts_over_then_escape_again_cancels() {
        let mut s = one_monitor();
        start_drag(&mut s);
        assert_eq!(s.handle(Input::Escape), State::Idle);
        assert_eq!(s.rect(), None);
        // 押したままのボタンを離しても、選び直しのまま
        assert_eq!(
            s.handle(Input::Release {
                button: Button::Left,
                at: p(300, 200),
            }),
            State::Idle
        );
        assert_eq!(s.handle(Input::Escape), State::Cancelled);
    }

    #[test]
    fn right_click_while_selecting_starts_over_then_right_click_again_cancels() {
        let mut s = one_monitor();
        start_drag(&mut s);
        let right = Input::Press {
            button: Button::Right,
            at: p(300, 200),
        };
        assert_eq!(s.handle(right), State::Idle);
        assert_eq!(s.rect(), None);
        assert_eq!(s.handle(right), State::Cancelled);
    }

    #[test]
    fn dragging_up_and_left_selects_the_same_rect() {
        let mut s = one_monitor();
        s.handle(Input::Press {
            button: Button::Left,
            at: p(109, 69),
        });
        s.handle(Input::Move { at: p(10, 20) });
        assert_eq!(s.rect(), Some(Rect::new(10, 20, 110, 70)));
    }

    /// 左に 100% の FHD（負の座標）、中央にプライマリの 150% の 4K、右に 100px の隙間を空けて 100% の FHD。
    fn three_monitors() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                rect: Rect::new(0, 0, 3840, 2160),
                dpi: 144,
            },
            MonitorInfo {
                rect: Rect::new(-1920, 0, 0, 1080),
                dpi: 96,
            },
            MonitorInfo {
                rect: Rect::new(3940, 0, 5860, 1080),
                dpi: 96,
            },
        ]
    }

    #[test]
    fn a_rect_inside_one_monitor_is_one_part() {
        let parts = split(Rect::new(100, 100, 1380, 820), &three_monitors());
        assert_eq!(
            parts,
            vec![Part {
                monitor: 0,
                rect: Rect::new(100, 100, 1380, 820),
            }]
        );
    }

    #[test]
    fn a_rect_across_a_negative_monitor_and_a_monitor_with_another_dpi_splits_left_to_right() {
        let parts = split(Rect::new(-100, 500, 200, 700), &three_monitors());
        assert_eq!(
            parts,
            vec![
                Part {
                    monitor: 1,
                    rect: Rect::new(-100, 500, 0, 700),
                },
                Part {
                    monitor: 0,
                    rect: Rect::new(0, 500, 200, 700),
                },
            ]
        );
    }

    #[test]
    fn the_gap_between_monitors_belongs_to_no_part() {
        let parts = split(Rect::new(3800, 100, 4000, 300), &three_monitors());
        assert_eq!(
            parts,
            vec![
                Part {
                    monitor: 0,
                    rect: Rect::new(3800, 100, 3840, 300),
                },
                Part {
                    monitor: 2,
                    rect: Rect::new(3940, 100, 4000, 300),
                },
            ]
        );
    }

    #[test]
    fn the_area_below_a_shorter_monitor_belongs_to_no_part() {
        let parts = split(Rect::new(-50, 1000, 50, 1200), &three_monitors());
        assert_eq!(
            parts,
            vec![
                Part {
                    monitor: 1,
                    rect: Rect::new(-50, 1000, 0, 1080),
                },
                Part {
                    monitor: 0,
                    rect: Rect::new(0, 1000, 50, 1200),
                },
            ]
        );
    }

    #[test]
    fn the_size_label_shows_the_physical_size_of_the_range() {
        let mut s = Selection::new(three_monitors(), p(0, 0));
        assert_eq!(s.size_label(), None);
        s.handle(Input::Press {
            button: Button::Left,
            at: p(100, 100),
        });
        s.handle(Input::Move { at: p(1379, 819) });
        assert_eq!(s.size_label().as_deref(), Some("1280 × 720"));
    }

    #[test]
    fn the_size_label_joins_the_size_on_each_monitor_with_plus() {
        let mut s = Selection::new(three_monitors(), p(0, 0));
        s.handle(Input::Press {
            button: Button::Left,
            at: p(-100, 500),
        });
        s.handle(Input::Move { at: p(199, 699) });
        assert_eq!(s.size_label().as_deref(), Some("100 × 200 ＋ 200 × 200"));
    }

    fn arrow(direction: Direction, shift: bool) -> Input {
        Input::Arrow { direction, shift }
    }

    #[test]
    fn an_arrow_moves_the_cursor_one_pixel_and_shift_arrow_ten_pixels() {
        let mut s = Selection::new(
            vec![MonitorInfo {
                rect: Rect::new(0, 0, 1920, 1080),
                dpi: 96,
            }],
            p(500, 500),
        );
        s.handle(arrow(Direction::Right, false));
        assert_eq!(s.cursor(), p(501, 500));
        s.handle(arrow(Direction::Down, true));
        assert_eq!(s.cursor(), p(501, 510));
        s.handle(arrow(Direction::Left, true));
        assert_eq!(s.cursor(), p(491, 510));
        s.handle(arrow(Direction::Up, false));
        assert_eq!(s.cursor(), p(491, 509));
        assert_eq!(s.state(), State::Idle);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn arrows_move_the_cursor_onto_an_adjacent_monitor_but_not_off_the_screens() {
        let mut s = Selection::new(three_monitors(), p(-1, 500));
        // 隣り合うモニタへは移れる
        s.handle(arrow(Direction::Right, false));
        assert_eq!(s.cursor(), p(0, 500));
        // 隙間には入らず、モニタの端で止まる
        let mut s = Selection::new(three_monitors(), p(3835, 100));
        s.handle(arrow(Direction::Right, true));
        assert_eq!(s.cursor(), p(3839, 100));
        // 画面の外には出ない
        let mut s = Selection::new(three_monitors(), p(-1915, 5));
        s.handle(arrow(Direction::Left, true));
        s.handle(arrow(Direction::Up, true));
        assert_eq!(s.cursor(), p(-1920, 0));
        // 短いモニタの下（どのモニタでもない部分）にも入らない
        let mut s = Selection::new(three_monitors(), p(-10, 1075));
        s.handle(arrow(Direction::Down, true));
        assert_eq!(s.cursor(), p(-10, 1079));
    }

    #[test]
    fn space_sets_the_start_then_arrows_extend_and_space_again_confirms() {
        let mut s = Selection::new(three_monitors(), p(100, 200));
        assert_eq!(s.handle(Input::Space), State::Selecting);
        assert_eq!(s.rect(), Some(Rect::new(100, 200, 101, 201)));
        s.handle(arrow(Direction::Right, true));
        s.handle(arrow(Direction::Right, false));
        s.handle(arrow(Direction::Down, true));
        assert_eq!(s.state(), State::Selecting);
        assert_eq!(s.rect(), Some(Rect::new(100, 200, 112, 211)));
        assert_eq!(s.handle(Input::Space), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(100, 200, 112, 211)));
    }

    #[test]
    fn space_after_moving_the_mouse_starts_at_the_mouse() {
        let mut s = one_monitor();
        s.handle(Input::Move { at: p(300, 400) });
        s.handle(Input::Space);
        s.handle(Input::Move { at: p(250, 350) });
        assert_eq!(s.handle(Input::Space), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(250, 350, 301, 401)));
    }

    #[test]
    fn escape_after_space_starts_over() {
        let mut s = one_monitor();
        s.handle(Input::Space);
        assert_eq!(s.handle(Input::Escape), State::Idle);
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn ctrl_a_confirms_the_whole_monitor_under_the_cursor() {
        let mut s = Selection::new(three_monitors(), p(-500, 300));
        assert_eq!(s.handle(Input::SelectMonitor), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(-1920, 0, 0, 1080)));
        assert_eq!(s.size_label().as_deref(), Some("1920 × 1080"));
    }

    #[test]
    fn ctrl_a_while_selecting_replaces_the_range_with_the_monitor() {
        let mut s = Selection::new(three_monitors(), p(0, 0));
        start_drag(&mut s);
        assert_eq!(s.handle(Input::SelectMonitor), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(0, 0, 3840, 2160)));
    }

    #[test]
    fn ctrl_shift_a_confirms_the_bounding_rect_of_all_monitors() {
        let mut s = Selection::new(three_monitors(), p(100, 100));
        assert_eq!(s.handle(Input::SelectAllMonitors), State::Confirmed);
        assert_eq!(s.rect(), Some(Rect::new(-1920, 0, 5860, 2160)));
        let parts = s.parts();
        assert_eq!(
            parts.iter().map(|part| part.monitor).collect::<Vec<_>>(),
            vec![1, 0, 2]
        );
    }

    #[test]
    fn inputs_after_cancelling_are_ignored() {
        let mut s = one_monitor();
        s.handle(Input::Escape);
        start_drag(&mut s);
        assert_eq!(s.state(), State::Cancelled);
        assert_eq!(s.rect(), None);
    }
}
