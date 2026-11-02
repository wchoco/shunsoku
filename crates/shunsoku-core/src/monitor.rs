//! モニタの一覧と、モニタ相対の範囲をスクリーン座標に直す判定。
//!
//! 一覧（`MonitorInfo`）は Win32 層が撮るたびに集めて渡す。定型領域キャプチャ（`locate`）と CLI の `rect` は、
//! ここでモニタを探し、範囲がそのモニタに収まるかを決める。

use crate::geometry::Rect;

/// モニタ 1 枚。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// モニタの識別子。並びや配置を変えても同じモニタなら同じ値（Win32 層ではデバイスインターフェースのパス）。
    pub id: String,
    /// 表示用の名前。
    pub name: String,
    /// 位置と大きさ（スクリーン座標、物理ピクセル）。
    pub rect: Rect,
    /// 実効 DPI（100% で 96）。
    pub dpi: u32,
    /// メインモニタ。
    pub primary: bool,
}

/// モニタ相対の範囲を撮れないわけ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocateError {
    /// その識別子のモニタがない（外した、など）。
    MonitorNotFound,
    /// 範囲がモニタからはみ出す（解像度を下げた、など）。
    OutOfBounds,
}

impl MonitorInfo {
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
pub fn locate(monitors: &[MonitorInfo], id: &str, relative: Rect) -> Result<Located, LocateError> {
    let index = monitors
        .iter()
        .position(|m| m.id.eq_ignore_ascii_case(id))
        .ok_or(LocateError::MonitorNotFound)?;
    let screen = monitors[index].to_screen(relative)?;
    Ok(Located { index, screen })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: &str, rect: Rect, dpi: u32) -> MonitorInfo {
        MonitorInfo {
            id: id.to_owned(),
            name: String::new(),
            rect,
            dpi,
            primary: false,
        }
    }

    /// 1920x1080 のメインモニタと、その左に置いた 2560x1440 のモニタ。
    fn two_monitors() -> Vec<MonitorInfo> {
        vec![
            monitor("MAIN", Rect::new(0, 0, 1920, 1080), 96),
            monitor("LEFT", Rect::new(-2560, -360, 0, 1080), 144),
        ]
    }

    #[test]
    fn a_region_is_taken_relative_to_its_monitor() {
        let located = locate(&two_monitors(), "LEFT", Rect::new(100, 50, 400, 250));
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
            monitor("LEFT", Rect::new(0, 0, 2560, 1440), 144),
            monitor("MAIN", Rect::new(2560, 0, 4480, 1080), 96),
        ];
        assert_eq!(
            locate(&rearranged, "main", Rect::new(10, 20, 110, 220)),
            Ok(Located {
                index: 1,
                screen: Rect::new(2570, 20, 2670, 220),
            })
        );
    }

    #[test]
    fn a_missing_monitor_is_not_taken() {
        assert_eq!(
            locate(&two_monitors(), "GONE", Rect::new(0, 0, 10, 10)),
            Err(LocateError::MonitorNotFound)
        );
    }

    #[test]
    fn a_range_that_sticks_out_of_the_monitor_is_not_taken() {
        let monitors = two_monitors();
        // 右下の端までは収まる
        assert!(locate(&monitors, "MAIN", Rect::new(1820, 980, 1920, 1080)).is_ok());
        for outside in [
            Rect::new(1821, 980, 1921, 1080),
            Rect::new(1820, 981, 1920, 1081),
            Rect::new(-1, 0, 10, 10),
        ] {
            assert_eq!(
                locate(&monitors, "MAIN", outside),
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
            locate(&monitors, "MAIN", Rect::new(100, 100, 200, 200)),
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
            locate(&monitors, "MAIN", Rect::new(100, 100, 1280, 720)),
            Ok(Located {
                index: 0,
                screen: Rect::new(100, 100, 1280, 720),
            })
        );
        assert_eq!(
            locate(&monitors, "MAIN", Rect::new(100, 100, 1281, 720)),
            Err(LocateError::OutOfBounds)
        );
    }
}
