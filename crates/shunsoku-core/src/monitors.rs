//! モニタの一覧と、モニタ相対の範囲をスクリーン座標に直す判定。
//!
//! 一覧（`Monitor`）は Win32 層が撮るたびに集めて渡す（抜き差しや配置の変更を拾うため）。
//!
//! - CLI の `monitors` の番号と、`capture rect --monitor <番号>` の範囲（`numbered`、`listing`、`locate_numbered`）。
//!   番号は位置の順（左から右、同じ左端なら上から下）に 1 から振る。抜き差しや配置の変更がなければ、
//!   起動し直しても同じ番号になる。
//! - 定型領域キャプチャの撮影の判定（`locate_by_id`）。定型領域はモニタを識別子で覚える。
//! - 範囲選択の画面で決めた範囲を、定型領域のモニタ相対の範囲にする（`relative_to`）。

use crate::geometry::Rect;

/// 1 枚のモニタ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    /// モニタの識別子。並びや配置を変えても同じモニタなら同じ値（Win32 層ではデバイスインターフェースのパス）。
    pub id: String,
    /// 名前（取れればモニタの製品名、取れなければデバイスの名前 `\\.\DISPLAY1`）。
    pub name: String,
    /// スクリーン座標（物理ピクセル）での位置と大きさ。
    pub rect: Rect,
    /// 実効 DPI（100% で 96）。
    pub dpi: u32,
    /// メインモニタ（スクリーン座標の原点を含むモニタ）か。
    pub primary: bool,
}

/// モニタ相対の範囲を撮れないわけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocateError {
    /// その識別子のモニタがない（外した、など）。
    MonitorNotFound,
    /// その番号のモニタがない。
    NoSuchMonitor(u32),
    /// メインモニタが見つからない。
    NoMainMonitor,
    /// 範囲がモニタからはみ出す（解像度を下げた、など）。
    OutOfBounds,
}

impl Monitor {
    /// モニタ相対の範囲 `relative` をスクリーン座標に直す。はみ出すなら `OutOfBounds`。
    pub fn to_screen(&self, relative: Rect) -> Result<Rect, LocateError> {
        let screen = relative.offset(self.rect.left, self.rect.top);
        let inside = !relative.is_empty()
            && relative.left >= 0
            && relative.top >= 0
            && screen.right <= self.rect.right
            && screen.bottom <= self.rect.bottom;
        if inside {
            Ok(screen)
        } else {
            Err(LocateError::OutOfBounds)
        }
    }
}

/// 番号の順に並べる。`n` 番のモニタは添字 `n - 1`。
pub fn numbered(mut monitors: Vec<Monitor>) -> Vec<Monitor> {
    monitors.sort_by_key(|m| (m.rect.left, m.rect.top));
    monitors
}

/// `monitors` の出力。1 行に 1 枚、番号の順に、番号・名前・解像度・拡大率（とメインモニタの印 `main`。文言の
/// `monitors.main`）をタブで区切る。
pub fn listing(monitors: &[Monitor], main: &str) -> String {
    let mut out = String::new();
    for (i, m) in numbered(monitors.to_vec()).iter().enumerate() {
        let size = m.rect.size();
        // 拡大率は DPI 96 を 100% とした値（Windows の設定と同じく整数の %）
        let scale = (u64::from(m.dpi) * 100 + 48) / 96;
        out.push_str(&format!(
            "{}\t{}\t{}x{}\t{scale}%",
            i + 1,
            m.name,
            size.width,
            size.height
        ));
        if m.primary {
            out.push('\t');
            out.push_str(main);
        }
        out.push('\n');
    }
    out
}

/// モニタ相対の範囲 `rect` を、`number` 番（None ならメインモニタ）のモニタのスクリーン座標にする
/// （CLI の `capture rect`）。範囲がそのモニタからはみ出すなら撮らない。
pub fn locate_numbered(
    monitors: &[Monitor],
    number: Option<u32>,
    rect: Rect,
) -> Result<Rect, LocateError> {
    let monitor = match number {
        Some(n) => numbered(monitors.to_vec())
            .into_iter()
            .nth(n.checked_sub(1).ok_or(LocateError::NoSuchMonitor(n))? as usize)
            .ok_or(LocateError::NoSuchMonitor(n))?,
        None => monitors
            .iter()
            .find(|m| m.primary)
            .cloned()
            .ok_or(LocateError::NoMainMonitor)?,
    };
    monitor.to_screen(rect)
}

/// 撮る場所。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Located {
    /// `monitors` の中の位置。
    pub index: usize,
    /// 撮る範囲（スクリーン座標、物理ピクセル）。
    pub screen: Rect,
}

/// 識別子 `id` のモニタを `monitors` から探し、そのモニタ相対の範囲 `relative` をスクリーン座標に直す
/// （定型領域キャプチャの撮影の判定）。識別子は大文字と小文字を区別しない。DPI は見ない（範囲は物理ピクセルなので、
/// 拡大率だけを変えても同じ画素を撮る）。
pub fn locate_by_id(
    monitors: &[Monitor],
    id: &str,
    relative: Rect,
) -> Result<Located, LocateError> {
    let index = monitors
        .iter()
        .position(|m| m.id.eq_ignore_ascii_case(id))
        .ok_or(LocateError::MonitorNotFound)?;
    let screen = monitors[index].to_screen(relative)?;
    Ok(Located { index, screen })
}

/// 範囲選択の画面で決めた範囲 `screen`（スクリーン座標）を、左上を含むモニタの相対の範囲にする（定型領域の
/// 登録と「範囲を選び直す」）。そのモニタの `monitors` の中の位置も返す。左上がどのモニタにもなければ None。
pub fn relative_to(monitors: &[Monitor], screen: Rect) -> Option<(usize, Rect)> {
    let index = monitors
        .iter()
        .position(|m| m.rect.contains(screen.origin()))?;
    let monitor = &monitors[index].rect;
    Some((index, screen.offset(-monitor.left, -monitor.top)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, rect: Rect, primary: bool) -> Monitor {
        Monitor {
            id: name.to_uppercase(),
            name: name.to_owned(),
            rect,
            dpi: 96,
            primary,
        }
    }

    /// 左にサブ（1920×1080）、真ん中にメイン（2560×1440）、メインの下にもう 1 枚。
    fn three() -> Vec<Monitor> {
        vec![
            monitor("main", Rect::new(0, 0, 2560, 1440), true),
            monitor("below", Rect::new(0, 1440, 1920, 2520), false),
            monitor("left", Rect::new(-1920, 0, 0, 1080), false),
        ]
    }

    #[test]
    fn a_chosen_range_belongs_to_the_monitor_holding_its_top_left_corner() {
        let monitors = three();
        // 左のモニタの中で選んだ範囲（スクリーン座標）は、そのモニタの左上からの位置になる
        let (index, relative) = relative_to(&monitors, Rect::new(-1800, 100, -1000, 700)).unwrap();
        assert_eq!(monitors[index].name, "left");
        assert_eq!(relative, Rect::new(120, 100, 920, 700));
        // 下のモニタ
        let (index, relative) = relative_to(&monitors, Rect::new(10, 1450, 110, 1550)).unwrap();
        assert_eq!(monitors[index].name, "below");
        assert_eq!(relative, Rect::new(10, 10, 110, 110));
    }

    #[test]
    fn a_range_starting_outside_every_monitor_has_no_monitor() {
        // メインの右下の外（下のモニタは 1920 までの幅）
        assert_eq!(
            relative_to(&three(), Rect::new(2000, 1500, 2100, 1600)),
            None
        );
    }

    #[test]
    fn monitors_are_numbered_from_1_left_to_right_then_top_to_bottom() {
        let names: Vec<String> = numbered(three()).into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["left", "main", "below"]);
    }

    #[test]
    fn the_listing_shows_number_name_resolution_and_scale_per_line() {
        let mut monitors = three();
        monitors[0].dpi = 144;
        monitors[2].dpi = 120;
        monitors[2].name = r"\\.\DISPLAY3".to_owned();
        assert_eq!(
            listing(&monitors, "メイン"),
            "1\t\\\\.\\DISPLAY3\t1920x1080\t125%\n\
             2\tmain\t2560x1440\t150%\tメイン\n\
             3\tbelow\t1920x1080\t100%\n"
        );
    }

    #[test]
    fn without_a_number_the_rect_is_relative_to_the_main_monitor() {
        assert_eq!(
            locate_numbered(&three(), None, Rect::new(10, 20, 110, 120)),
            Ok(Rect::new(10, 20, 110, 120))
        );
    }

    #[test]
    fn a_numbered_monitor_shifts_the_rect_to_screen_coordinates() {
        // 1 番は左のモニタ（原点 -1920, 0）、3 番はメインの下（原点 0, 1440）
        assert_eq!(
            locate_numbered(&three(), Some(1), Rect::new(0, 0, 100, 100)),
            Ok(Rect::new(-1920, 0, -1820, 100))
        );
        assert_eq!(
            locate_numbered(&three(), Some(3), Rect::new(1820, 980, 1920, 1080)),
            Ok(Rect::new(1820, 2420, 1920, 2520))
        );
    }

    #[test]
    fn a_rect_reaching_past_its_numbered_monitor_is_rejected() {
        let past = [
            Rect::new(1821, 0, 1921, 100),
            Rect::new(0, 1000, 100, 1081),
            Rect::new(-1, 0, 99, 100),
            Rect::new(0, -1, 100, 99),
        ];
        for rect in past {
            assert_eq!(
                locate_numbered(&three(), Some(1), rect),
                Err(LocateError::OutOfBounds),
                "{rect:?}"
            );
        }
    }

    #[test]
    fn a_missing_monitor_number_is_rejected() {
        assert_eq!(
            locate_numbered(&three(), Some(4), Rect::new(0, 0, 1, 1)),
            Err(LocateError::NoSuchMonitor(4))
        );
        assert_eq!(
            locate_numbered(&three(), Some(0), Rect::new(0, 0, 1, 1)),
            Err(LocateError::NoSuchMonitor(0))
        );
    }

    #[test]
    fn without_a_main_monitor_the_rect_is_rejected() {
        let monitors = vec![monitor("only", Rect::new(0, 0, 100, 100), false)];
        assert_eq!(
            locate_numbered(&monitors, None, Rect::new(0, 0, 1, 1)),
            Err(LocateError::NoMainMonitor)
        );
    }

    /// 1920x1080 のメインモニタと、その左に置いた 2560x1440 のモニタ。
    fn two_monitors() -> Vec<Monitor> {
        let mut left = monitor("left", Rect::new(-2560, -360, 0, 1080), false);
        left.dpi = 144;
        vec![monitor("main", Rect::new(0, 0, 1920, 1080), true), left]
    }

    #[test]
    fn a_region_is_taken_relative_to_its_monitor() {
        let located = locate_by_id(&two_monitors(), "LEFT", Rect::new(100, 50, 400, 250));
        assert_eq!(
            located,
            Ok(Located {
                index: 1,
                screen: Rect::new(-2460, -310, -2160, -110),
            })
        );
    }

    #[test]
    fn the_same_monitor_is_found_after_the_monitors_are_rearranged() {
        let rearranged = vec![
            monitor("left", Rect::new(0, 0, 2560, 1440), false),
            monitor("main", Rect::new(2560, 0, 4480, 1080), true),
        ];
        assert_eq!(
            locate_by_id(&rearranged, "main", Rect::new(10, 20, 110, 220)),
            Ok(Located {
                index: 1,
                screen: Rect::new(2570, 20, 2670, 220),
            })
        );
    }

    #[test]
    fn a_missing_monitor_is_not_taken() {
        assert_eq!(
            locate_by_id(&two_monitors(), "GONE", Rect::new(0, 0, 10, 10)),
            Err(LocateError::MonitorNotFound)
        );
    }

    #[test]
    fn a_range_that_sticks_out_of_the_monitor_is_not_taken() {
        let monitors = two_monitors();
        // 右下の端までは収まる
        assert!(locate_by_id(&monitors, "MAIN", Rect::new(1820, 980, 1920, 1080)).is_ok());
        for outside in [
            Rect::new(1821, 980, 1921, 1080),
            Rect::new(1820, 981, 1920, 1081),
            Rect::new(-1, 0, 10, 10),
        ] {
            assert_eq!(
                locate_by_id(&monitors, "MAIN", outside),
                Err(LocateError::OutOfBounds),
                "{outside:?}"
            );
        }
    }

    #[test]
    fn only_the_dpi_changing_still_takes_the_same_pixels() {
        let mut monitors = two_monitors();
        monitors[0].dpi = 192;
        assert_eq!(
            locate_by_id(&monitors, "MAIN", Rect::new(100, 100, 200, 200)),
            Ok(Located {
                index: 0,
                screen: Rect::new(100, 100, 200, 200),
            })
        );
    }

    #[test]
    fn a_lower_resolution_that_still_holds_the_range_takes_it() {
        let mut monitors = two_monitors();
        monitors[0].rect = Rect::new(0, 0, 1280, 720);
        assert_eq!(
            locate_by_id(&monitors, "MAIN", Rect::new(100, 100, 1280, 720)),
            Ok(Located {
                index: 0,
                screen: Rect::new(100, 100, 1280, 720),
            })
        );
        assert_eq!(
            locate_by_id(&monitors, "MAIN", Rect::new(100, 100, 1281, 720)),
            Err(LocateError::OutOfBounds)
        );
    }
}
