//! 範囲選択の画面でのウィンドウのハイライト（Shift でウィンドウ、Shift+Ctrl で部品）。
//!
//! - ウィンドウの一覧（`WindowInfo`）は Win32 層が範囲選択の画面を開いた時点で集めて渡す。
//! - 一覧の並びは Z オーダーの手前から奥。部品（子ウィンドウ）も同じ一覧に入れ、`parent` で親を指す。
//!   兄弟の間の並びも手前から奥にする（`EnumWindows` / `EnumChildWindows` の順）。
//! - スクラップとタスクバーは対象から外す（その下のウィンドウが対象になる）。

use crate::geometry::{Point, Rect};

/// ウィンドウの識別子（Win32 層の `HWND` の値）。core では中身を見ない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub isize);

/// ウィンドウの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Normal,
    /// shunsoku のスクラップ。写り込みは許すが、ハイライトの対象にはしない。
    Scrap,
    /// タスクバー。
    Taskbar,
}

/// 1 つのウィンドウ（または部品）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: WindowId,
    /// スクリーン座標（物理ピクセル）。最上位のウィンドウは WGC が撮る範囲（`DWMWA_EXTENDED_FRAME_BOUNDS`）。
    pub rect: Rect,
    /// タイトル。部品なら、そのテキストかクラス名。
    pub name: String,
    /// 親のウィンドウ。最上位なら None。
    pub parent: Option<WindowId>,
    pub kind: WindowKind,
}

/// ハイライトしているもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Highlight {
    /// 撮る最上位のウィンドウ（部品でも、その最上位のウィンドウ）。
    pub window: WindowId,
    /// ハイライトする範囲（スクリーン座標）。部品なら、最上位のウィンドウの中に切り詰めたもの。
    pub rect: Rect,
    /// 部品なら Some（`rect` の部品）。
    pub part: Option<WindowId>,
    /// 出す名前。部品なら「ウィンドウ名 › 部品名」。
    pub name: String,
}

impl Highlight {
    /// 出す文字（名前と物理ピクセルのサイズ）。
    pub fn label(&self) -> String {
        format!(
            "{}  {} × {}",
            self.name,
            self.rect.width(),
            self.rect.height()
        )
    }
}

/// `at` の下のハイライトの対象。`parts` なら最上位のウィンドウの中の、いちばん奥の部品まで降りる。
pub(super) fn find(windows: &[WindowInfo], at: Point, parts: bool) -> Option<Highlight> {
    // スクラップとタスクバーは飛ばして、その奥のウィンドウを選ぶ
    let top = windows
        .iter()
        .find(|w| w.parent.is_none() && w.kind == WindowKind::Normal && w.rect.contains(at))?;
    let mut highlight = Highlight {
        window: top.id,
        rect: top.rect,
        part: None,
        name: top.name.clone(),
    };
    if !parts {
        return Some(highlight);
    }
    // 子のうち手前でカーソルの下にあるものへ、降りられなくなるまで降りる。
    // 撮るのは最上位のウィンドウなので、その外にはみ出した部分は対象にしない
    let mut current = top;
    while let Some(child) = windows.iter().find(|w| {
        w.parent == Some(current.id)
            && w.rect
                .intersect(&top.rect)
                .is_some_and(|visible| visible.contains(at))
    }) {
        current = child;
    }
    if current.id != top.id {
        highlight.part = Some(current.id);
        highlight.rect = current.rect.intersect(&top.rect).unwrap_or(top.rect);
        highlight.name = format!("{} › {}", top.name, current.name);
    }
    Some(highlight)
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn p(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    fn window(id: isize, rect: Rect, name: &str) -> WindowInfo {
        WindowInfo {
            id: WindowId(id),
            rect,
            name: name.to_owned(),
            parent: None,
            kind: WindowKind::Normal,
        }
    }

    /// 1920×1080 の 1 枚に、手前から順に重なったウィンドウを並べた画面。
    fn screen(windows: Vec<WindowInfo>, cursor: Point) -> Selection {
        let mut s = Selection::new(
            vec![MonitorInfo {
                rect: Rect::new(0, 0, 1920, 1080),
                dpi: 96,
            }],
            cursor,
        );
        s.set_windows(windows);
        s
    }

    fn shift() -> Input {
        Input::Modifiers {
            shift: true,
            ctrl: false,
        }
    }

    #[test]
    fn shift_highlights_the_frontmost_window_under_the_cursor() {
        let mut s = screen(
            vec![
                window(1, Rect::new(500, 500, 900, 800), "メモ帳"),
                window(2, Rect::new(100, 100, 1380, 820), "ブラウザ"),
            ],
            p(200, 200),
        );
        assert_eq!(s.highlight(), None);
        s.handle(shift());
        let h = s.highlight().expect("highlighted");
        assert_eq!(h.window, WindowId(2));
        assert_eq!(h.rect, Rect::new(100, 100, 1380, 820));
        assert_eq!(h.label(), "ブラウザ  1280 × 720");
        // 手前のウィンドウの上に動かせば、そちらになる
        s.handle(Input::Move { at: p(600, 600) });
        assert_eq!(s.highlight().map(|h| h.window), Some(WindowId(1)));
    }

    #[test]
    fn scraps_and_the_taskbar_are_skipped_for_the_window_behind_them() {
        let scrap = WindowInfo {
            kind: WindowKind::Scrap,
            ..window(1, Rect::new(150, 150, 400, 400), "shunsoku")
        };
        let taskbar = WindowInfo {
            kind: WindowKind::Taskbar,
            ..window(2, Rect::new(0, 1032, 1920, 1080), "")
        };
        let mut s = screen(
            vec![
                scrap,
                taskbar,
                window(3, Rect::new(100, 100, 1380, 1080), "ブラウザ"),
            ],
            p(200, 200),
        );
        s.handle(shift());
        assert_eq!(s.highlight().map(|h| h.window), Some(WindowId(3)));
        s.handle(Input::Move { at: p(200, 1050) });
        assert_eq!(s.highlight().map(|h| h.window), Some(WindowId(3)));
        // タスクバーの奥に何もなければ、何もハイライトしない
        s.handle(Input::Move { at: p(1500, 1050) });
        assert_eq!(s.highlight(), None);
    }

    fn part(id: isize, parent: isize, rect: Rect, name: &str) -> WindowInfo {
        WindowInfo {
            parent: Some(WindowId(parent)),
            ..window(id, rect, name)
        }
    }

    /// ウィンドウ 1 の中に、サイドバー（2）と本文（3）。本文の中にボタン（4）。
    fn window_with_parts(cursor: Point) -> Selection {
        screen(
            vec![
                window(1, Rect::new(100, 100, 1100, 800), "エディタ"),
                part(2, 1, Rect::new(100, 130, 300, 800), "サイドバー"),
                part(3, 1, Rect::new(300, 130, 1100, 800), "本文"),
                part(4, 3, Rect::new(1000, 700, 1080, 730), "OK"),
                window(5, Rect::new(0, 0, 1920, 1080), "デスクトップ"),
            ],
            cursor,
        )
    }

    #[test]
    fn shift_ctrl_highlights_the_innermost_part_under_the_cursor() {
        let mut s = window_with_parts(p(1010, 710));
        s.handle(Input::Modifiers {
            shift: true,
            ctrl: true,
        });
        let h = s.highlight().expect("highlighted");
        assert_eq!(h.window, WindowId(1));
        assert_eq!(h.part, Some(WindowId(4)));
        assert_eq!(h.rect, Rect::new(1000, 700, 1080, 730));
        assert_eq!(h.label(), "エディタ › OK  80 × 30");
        s.handle(Input::Move { at: p(200, 400) });
        assert_eq!(
            s.highlight().map(|h| h.label()).as_deref(),
            Some("エディタ › サイドバー  200 × 670")
        );
        // Ctrl を離すとウィンドウ全体に戻る
        s.handle(shift());
        assert_eq!(
            s.highlight().map(|h| (h.part, h.rect)),
            Some((None, Rect::new(100, 100, 1100, 800)))
        );
    }

    #[test]
    fn shift_ctrl_outside_any_part_highlights_the_window_itself() {
        // タイトルバー（どの部品の上でもない）
        let mut s = window_with_parts(p(500, 110));
        s.handle(Input::Modifiers {
            shift: true,
            ctrl: true,
        });
        assert_eq!(
            s.highlight().map(|h| (h.window, h.part)),
            Some((WindowId(1), None))
        );
    }

    #[test]
    fn a_part_sticking_out_of_its_window_is_cut_to_the_window() {
        let mut s = screen(
            vec![
                window(1, Rect::new(100, 100, 500, 500), "ビューア"),
                part(2, 1, Rect::new(50, 300, 600, 700), "キャンバス"),
            ],
            p(200, 400),
        );
        s.handle(Input::Modifiers {
            shift: true,
            ctrl: true,
        });
        assert_eq!(
            s.highlight().map(|h| h.rect),
            Some(Rect::new(100, 300, 500, 500))
        );
    }

    fn press(at: Point) -> Input {
        Input::Press {
            button: Button::Left,
            at,
        }
    }

    fn release(at: Point) -> Input {
        Input::Release {
            button: Button::Left,
            at,
        }
    }

    #[test]
    fn a_click_with_shift_chooses_the_highlighted_window() {
        let mut s = window_with_parts(p(500, 400));
        s.handle(shift());
        s.handle(press(p(500, 400)));
        assert_eq!(s.handle(release(p(500, 400))), State::Confirmed);
        let chosen = s.chosen_window().expect("chosen");
        assert_eq!(chosen.window, WindowId(1));
        assert_eq!(chosen.rect, Rect::new(100, 100, 1100, 800));
        assert_eq!(s.rect(), None);
    }

    #[test]
    fn a_click_with_shift_ctrl_chooses_the_highlighted_part() {
        let mut s = window_with_parts(p(1010, 710));
        s.handle(Input::Modifiers {
            shift: true,
            ctrl: true,
        });
        s.handle(press(p(1010, 710)));
        assert_eq!(s.handle(release(p(1010, 710))), State::Confirmed);
        assert_eq!(
            s.chosen_window().map(|h| (h.window, h.part)),
            Some((WindowId(1), Some(WindowId(4))))
        );
    }

    #[test]
    fn enter_with_shift_chooses_the_highlighted_window() {
        let mut s = window_with_parts(p(500, 400));
        s.handle(shift());
        assert_eq!(s.handle(Input::Enter), State::Confirmed);
        assert_eq!(s.chosen_window().map(|h| h.window), Some(WindowId(1)));
    }

    #[test]
    fn enter_or_a_click_without_shift_chooses_nothing() {
        let mut s = window_with_parts(p(500, 400));
        assert_eq!(s.handle(Input::Enter), State::Idle);
        s.handle(press(p(500, 400)));
        assert_eq!(s.handle(release(p(500, 400))), State::Idle);
        assert_eq!(s.chosen_window(), None);
    }

    #[test]
    fn dragging_with_shift_selects_a_range_instead() {
        let mut s = window_with_parts(p(500, 400));
        s.handle(shift());
        s.handle(press(p(500, 400)));
        s.handle(Input::Move { at: p(599, 449) });
        // ドラッグを始めたら、ハイライトは消えて範囲が出る
        assert_eq!(s.highlight(), None);
        assert_eq!(s.handle(release(p(599, 449))), State::Confirmed);
        assert_eq!(s.chosen_window(), None);
        assert_eq!(s.rect(), Some(Rect::new(500, 400, 600, 450)));
    }

    #[test]
    fn a_click_with_shift_where_no_window_is_highlighted_does_nothing() {
        let mut s = screen(
            vec![window(1, Rect::new(100, 100, 500, 500), "ビューア")],
            p(800, 800),
        );
        s.handle(shift());
        s.handle(press(p(800, 800)));
        assert_eq!(s.handle(release(p(800, 800))), State::Idle);
        assert_eq!(s.handle(Input::Enter), State::Idle);
    }

    #[test]
    fn ctrl_alone_highlights_nothing() {
        let mut s = window_with_parts(p(1010, 710));
        s.handle(Input::Modifiers {
            shift: false,
            ctrl: true,
        });
        assert_eq!(s.highlight(), None);
    }

    #[test]
    fn the_view_shows_the_highlight_instead_of_the_range() {
        let mut s = window_with_parts(p(500, 400));
        s.handle(shift());
        s.handle(press(p(500, 400)));
        let view = s.view();
        assert_eq!(view.highlight.map(|h| h.window), Some(WindowId(1)));
        assert_eq!((view.rect, view.size_label), (None, None));
    }

    #[test]
    fn shift_enter_while_adjusting_confirms_the_range_not_a_window() {
        let mut s = window_with_parts(p(500, 400)).with_mode(Mode::Precise);
        s.handle(press(p(500, 400)));
        s.handle(Input::Move { at: p(599, 449) });
        assert_eq!(s.handle(release(p(599, 449))), State::Adjusting);
        s.handle(shift());
        assert_eq!(s.highlight(), None);
        assert_eq!(s.handle(Input::Enter), State::Confirmed);
        assert_eq!(s.chosen_window(), None);
        assert_eq!(s.rect(), Some(Rect::new(500, 400, 600, 450)));
    }
}
