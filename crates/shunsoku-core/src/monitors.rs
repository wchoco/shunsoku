//! モニタの一覧と番号。CLI の `monitors` の番号と、`capture rect --monitor <番号>` の範囲の判定（spec #34）。
//!
//! 一覧は Win32 層が集めて渡す。番号は位置の順（左から右、同じ左端なら上から下）に 1 から振る。
//! 抜き差しや配置の変更がなければ、起動し直しても同じ番号になる。

use crate::geometry::Rect;

/// 1 枚のモニタ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    /// 名前（取れればモニタの製品名、取れなければデバイスの名前 `\\.\DISPLAY1`）。
    pub name: String,
    /// スクリーン座標（物理ピクセル）での位置と大きさ。
    pub rect: Rect,
    /// 実効 DPI（100% で 96）。
    pub dpi: u32,
    /// メインモニタ（スクリーン座標の原点を含むモニタ）か。
    pub primary: bool,
}

/// 番号の順に並べる。`n` 番のモニタは添字 `n - 1`。
pub fn numbered(mut monitors: Vec<Monitor>) -> Vec<Monitor> {
    monitors.sort_by_key(|m| (m.rect.left, m.rect.top));
    monitors
}

/// `monitors` の出力。1 行に 1 枚、番号の順に、番号・名前・解像度・拡大率（とメインモニタの印）をタブで区切る。
pub fn listing(monitors: &[Monitor]) -> String {
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
            out.push_str("\tメイン");
        }
        out.push('\n');
    }
    out
}

/// `locate` で範囲を決められない理由（どれも CLI の終了コード 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocateError {
    /// その番号のモニタがない。
    NoSuchMonitor(u32),
    /// メインモニタが見つからない。
    NoMainMonitor,
    /// 範囲が 1 枚のモニタに収まらない。
    OutOfMonitor,
}

/// モニタ相対の範囲 `rect` を、`number` 番（None ならメインモニタ）のモニタのスクリーン座標にする。
/// 範囲がそのモニタからはみ出すなら撮らない。
pub fn locate(monitors: &[Monitor], number: Option<u32>, rect: Rect) -> Result<Rect, LocateError> {
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
    let size = monitor.rect.size();
    let inside = rect.left >= 0
        && rect.top >= 0
        && !rect.is_empty()
        && i64::from(rect.right) <= i64::from(size.width)
        && i64::from(rect.bottom) <= i64::from(size.height);
    if !inside {
        return Err(LocateError::OutOfMonitor);
    }
    Ok(rect.offset(monitor.rect.left, monitor.rect.top))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, rect: Rect, primary: bool) -> Monitor {
        Monitor {
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
            listing(&monitors),
            "1\t\\\\.\\DISPLAY3\t1920x1080\t125%\n\
             2\tmain\t2560x1440\t150%\tメイン\n\
             3\tbelow\t1920x1080\t100%\n"
        );
    }

    #[test]
    fn without_a_number_the_rect_is_relative_to_the_main_monitor() {
        assert_eq!(
            locate(&three(), None, Rect::new(10, 20, 110, 120)),
            Ok(Rect::new(10, 20, 110, 120))
        );
    }

    #[test]
    fn a_numbered_monitor_shifts_the_rect_to_screen_coordinates() {
        // 1 番は左のモニタ（原点 -1920, 0）、3 番はメインの下（原点 0, 1440）
        assert_eq!(
            locate(&three(), Some(1), Rect::new(0, 0, 100, 100)),
            Ok(Rect::new(-1920, 0, -1820, 100))
        );
        assert_eq!(
            locate(&three(), Some(3), Rect::new(1820, 980, 1920, 1080)),
            Ok(Rect::new(1820, 2420, 1920, 2520))
        );
    }

    #[test]
    fn a_rect_reaching_past_its_monitor_is_rejected() {
        let past = [
            Rect::new(1821, 0, 1921, 100),
            Rect::new(0, 1000, 100, 1081),
            Rect::new(-1, 0, 99, 100),
            Rect::new(0, -1, 100, 99),
        ];
        for rect in past {
            assert_eq!(
                locate(&three(), Some(1), rect),
                Err(LocateError::OutOfMonitor),
                "{rect:?}"
            );
        }
    }

    #[test]
    fn a_missing_monitor_number_is_rejected() {
        assert_eq!(
            locate(&three(), Some(4), Rect::new(0, 0, 1, 1)),
            Err(LocateError::NoSuchMonitor(4))
        );
    }

    #[test]
    fn without_a_main_monitor_the_rect_is_rejected() {
        let monitors = vec![monitor("only", Rect::new(0, 0, 100, 100), false)];
        assert_eq!(
            locate(&monitors, None, Rect::new(0, 0, 1, 1)),
            Err(LocateError::NoMainMonitor)
        );
    }
}
