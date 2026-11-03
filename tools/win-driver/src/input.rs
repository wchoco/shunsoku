//! マウスの入力の組み立てのうち、Win32 によらない部分。

/// ドラッグで通る点の数（始点を除く）。前回の道具（`drv.cs`）と同じ 30。
pub const DRAG_STEPS: i32 = 30;

/// `from` から `to` までのドラッグで、カーソルを順に置く点（始点と終点を含む）。
/// 入力の前に、全部の点の下のウィンドウを確かめる。
pub fn drag_path(from: (i32, i32), to: (i32, i32), steps: i32) -> Vec<(i32, i32)> {
    let steps = steps.max(1);
    (0..=steps)
        .map(|i| {
            (
                from.0 + (to.0 - from.0) * i / steps,
                from.1 + (to.1 - from.1) * i / steps,
            )
        })
        .collect()
}

/// ホイールの 1 目盛り（`WHEEL_DELTA`）。
pub const WHEEL_DELTA: i32 = 120;

/// ホイールを `notches` 目盛り回すときの、1 回ずつの量。正は奥へ（上へ）、負は手前へ。
pub fn wheel_steps(notches: i32) -> Vec<i32> {
    let one = if notches > 0 {
        WHEEL_DELTA
    } else {
        -WHEEL_DELTA
    };
    vec![one; notches.unsigned_abs() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_starts_at_the_start_and_ends_at_the_end() {
        assert_eq!(
            drag_path((100, 200), (-100, 260), 4),
            vec![(100, 200), (50, 215), (0, 230), (-50, 245), (-100, 260)]
        );
    }

    #[test]
    fn a_drag_has_at_least_the_two_ends() {
        assert_eq!(drag_path((1, 2), (3, 4), 0), vec![(1, 2), (3, 4)]);
    }

    #[test]
    fn wheel_notches_are_sent_one_at_a_time() {
        assert_eq!(wheel_steps(3), vec![120, 120, 120]);
        assert_eq!(wheel_steps(-2), vec![-120, -120]);
        assert!(wheel_steps(0).is_empty());
    }
}
