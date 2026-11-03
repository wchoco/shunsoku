//! 設定 GUI の見た目と操作の判断（Win32 層はメッセージの値を渡し、結果を当てるだけ）。

use crate::keys::Modifiers;

/// Ctrl+Tab / Ctrl+Shift+Tab でタブを移る向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStep {
    Next,
    Previous,
}

impl TabStep {
    /// 押されたキー `vk`（仮想キーコード）と押さえている修飾キー `held` から、タブを移る向きを決める。
    pub fn from_key_down(vk: u16, held: Modifiers) -> Option<TabStep> {
        const TAB: u16 = 0x09;
        if vk != TAB || !held.ctrl || held.alt || held.win {
            return None;
        }
        Some(if held.shift {
            TabStep::Previous
        } else {
            TabStep::Next
        })
    }

    /// `count` 個のタブの `current` 番目から 1 つ移った先。端では反対の端へ回る。
    pub fn from(self, current: usize, count: usize) -> usize {
        if count == 0 {
            return current;
        }
        match self {
            TabStep::Next => (current + 1) % count,
            TabStep::Previous => (current + count - 1) % count,
        }
    }
}

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

    const TAB: u16 = 0x09;

    fn held(ctrl: bool, shift: bool, alt: bool) -> Modifiers {
        Modifiers {
            win: false,
            ctrl,
            alt,
            shift,
        }
    }

    #[test]
    fn ctrl_tab_moves_to_the_next_tab_and_ctrl_shift_tab_to_the_previous() {
        assert_eq!(
            TabStep::from_key_down(TAB, held(true, false, false)),
            Some(TabStep::Next)
        );
        assert_eq!(
            TabStep::from_key_down(TAB, held(true, true, false)),
            Some(TabStep::Previous)
        );
    }

    #[test]
    fn other_keys_leave_the_tab_alone() {
        // Tab だけ・Shift+Tab は欄の移動（ダイアログに任せる）
        assert_eq!(TabStep::from_key_down(TAB, held(false, false, false)), None);
        assert_eq!(TabStep::from_key_down(TAB, held(false, true, false)), None);
        // Ctrl+Alt+Tab は OS のウィンドウの切り替え
        assert_eq!(TabStep::from_key_down(TAB, held(true, false, true)), None);
        // Ctrl+A などほかのキー
        assert_eq!(TabStep::from_key_down(0x41, held(true, false, false)), None);
        let win = Modifiers {
            win: true,
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert_eq!(TabStep::from_key_down(TAB, win), None);
    }

    #[test]
    fn stepping_wraps_around_at_both_ends() {
        assert_eq!(TabStep::Next.from(0, 6), 1);
        assert_eq!(TabStep::Next.from(5, 6), 0);
        assert_eq!(TabStep::Previous.from(3, 6), 2);
        assert_eq!(TabStep::Previous.from(0, 6), 5);
        // タブがなければ動かない
        assert_eq!(TabStep::Next.from(0, 0), 0);
    }

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
