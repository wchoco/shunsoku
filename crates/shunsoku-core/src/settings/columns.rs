//! 設定 GUI の一覧の列の幅（Win32 層は中身を測って渡し、結果を当てるだけ）。

/// 一覧の列の幅（ピクセル）。`needed` は列ごとの中身（見出しと全部の行の文言に余白を足したもの）の幅、
/// `available` は一覧の中の幅。
pub fn column_widths(needed: &[i32], available: i32) -> Vec<i32> {
    let mut widths = needed.to_vec();
    let rest = available - needed.iter().sum::<i32>();
    if let Some(last) = widths.last_mut() {
        *last += rest.max(0);
    }
    widths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_take_their_content_width_and_the_last_one_takes_the_rest() {
        assert_eq!(
            column_widths(&[120, 200, 90, 40], 500),
            vec![120, 200, 90, 90]
        );
    }

    #[test]
    fn columns_wider_than_the_list_keep_their_content_width() {
        // 「…」で略さず、一覧を横にスクロールさせる
        assert_eq!(
            column_widths(&[120, 200, 90, 270], 500),
            vec![120, 200, 90, 270]
        );
        assert_eq!(column_widths(&[], 500), Vec::<i32>::new());
    }
}
