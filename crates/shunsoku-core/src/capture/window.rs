//! ウィンドウキャプチャで WGC が使えなかったとき（UAC で昇格したウィンドウなど）の後退（spec #34、#64）。
//!
//! ウィンドウの矩形のうちモニタに写っている部分を Desktop Duplication の画面から切り抜く。
//! 他のウィンドウに隠れた部分は、隠しているウィンドウが写る。

use crate::geometry::Rect;

/// `window`（スクリーン座標）のうち、どれかのモニタ（`monitors`）に重なる部分を囲む矩形。
/// どのモニタにも重ならなければ None。
pub fn visible_bounds(window: Rect, monitors: &[Rect]) -> Option<Rect> {
    monitors
        .iter()
        .filter_map(|m| window.intersect(m))
        .reduce(|a, b| {
            Rect::new(
                a.left.min(b.left),
                a.top.min(b.top),
                a.right.max(b.right),
                a.bottom.max(b.bottom),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_sticking_out_of_the_monitor_is_cut_to_the_monitor() {
        let monitor = Rect::new(0, 0, 1920, 1080);
        let window = Rect::new(-100, 900, 500, 1300);
        assert_eq!(
            visible_bounds(window, &[monitor]),
            Some(Rect::new(0, 900, 500, 1080))
        );
    }

    #[test]
    fn a_window_across_two_monitors_keeps_the_parts_on_both() {
        // 左のモニタは右のより低い。ウィンドウは右のモニタの上にはみ出している
        let left = Rect::new(-1920, 200, 0, 1280);
        let right = Rect::new(0, 0, 2560, 1440);
        let window = Rect::new(-300, -50, 400, 600);
        assert_eq!(
            visible_bounds(window, &[left, right]),
            Some(Rect::new(-300, 0, 400, 600))
        );
    }

    #[test]
    fn a_window_off_every_monitor_has_nothing_to_take() {
        let monitor = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            visible_bounds(Rect::new(-32000, -32000, -31840, -31972), &[monitor]),
            None
        );
        assert_eq!(visible_bounds(Rect::new(0, 0, 0, 0), &[monitor]), None);
    }
}
