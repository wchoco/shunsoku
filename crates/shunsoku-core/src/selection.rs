//! 範囲選択の画面の状態機械（通常モード）。
//!
//! - 入力（`Input`）は Win32 層がマウスとキーのイベントから作る。座標はすべてスクリーン座標（物理ピクセル）。
//! - モニタの一覧（`MonitorInfo`）は Win32 層が集めて渡す。
//! - 範囲は、始点（押した画素）とカーソルの画素の両方を含む矩形。

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

/// 範囲選択の画面への入力。キーボードの操作（矢印、`Space`、`Ctrl+A`）はここに足す。
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

    #[test]
    fn inputs_after_cancelling_are_ignored() {
        let mut s = one_monitor();
        s.handle(Input::Escape);
        start_drag(&mut s);
        assert_eq!(s.state(), State::Cancelled);
        assert_eq!(s.rect(), None);
    }
}
