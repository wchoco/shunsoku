//! 設定 GUI で Ctrl+Tab / Ctrl+Shift+Tab を押したときのタブの移動（Win32 層はキーの値を渡し、結果を当てるだけ）。

use crate::keys::{Modifiers, VK_TAB};

/// Ctrl+Tab / Ctrl+Shift+Tab でタブを移る向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabStep {
    Next,
    Previous,
}

impl TabStep {
    /// 押されたキー `vk`（仮想キーコード）と押さえている修飾キー `held` から、タブを移る向きを決める。
    pub fn from_key_down(vk: u16, held: Modifiers) -> Option<TabStep> {
        if vk != VK_TAB || !held.ctrl || held.alt || held.win {
            return None;
        }
        Some(if held.shift {
            TabStep::Previous
        } else {
            TabStep::Next
        })
    }

    /// `count` 個のタブの `current` 番目から 1 つ移った先の番目。端では反対の端へ回る。
    pub fn next_index(self, current: usize, count: usize) -> usize {
        if count == 0 {
            return current;
        }
        match self {
            TabStep::Next => (current + 1) % count,
            TabStep::Previous => (current + count - 1) % count,
        }
    }
}

/// Ctrl+Tab / Ctrl+Shift+Tab でタブを移したあと、フォーカスを置く先。標準のプロパティシートに合わせる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepFocus {
    /// 新しいページの最初の欄へ移す。
    FirstField,
    /// 今の場所（タブの見出しか、OK などページの外のボタン）のまま。
    Stay,
}

impl StepFocus {
    /// 移す前にフォーカスがページの中（欄やページのボタン）にあったか（`in_page`）から決める。ページの中に
    /// いたら、隠れるページにフォーカスを残さず、新しいページの最初の欄へ移す。タブの見出しや、どのページでも
    /// 見えている OK・キャンセル・適用にいたら、そのままにする。
    pub fn after(in_page: bool) -> StepFocus {
        if in_page {
            StepFocus::FirstField
        } else {
            StepFocus::Stay
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            TabStep::from_key_down(VK_TAB, held(true, false, false)),
            Some(TabStep::Next)
        );
        assert_eq!(
            TabStep::from_key_down(VK_TAB, held(true, true, false)),
            Some(TabStep::Previous)
        );
    }

    #[test]
    fn other_keys_leave_the_tab_alone() {
        // Tab だけ・Shift+Tab は欄の移動（ダイアログに任せる）
        assert_eq!(
            TabStep::from_key_down(VK_TAB, held(false, false, false)),
            None
        );
        assert_eq!(
            TabStep::from_key_down(VK_TAB, held(false, true, false)),
            None
        );
        // Ctrl+Alt+Tab は OS のウィンドウの切り替え
        assert_eq!(
            TabStep::from_key_down(VK_TAB, held(true, false, true)),
            None
        );
        // Ctrl+A などほかのキー
        assert_eq!(TabStep::from_key_down(0x41, held(true, false, false)), None);
        let win = Modifiers {
            win: true,
            ctrl: true,
            alt: false,
            shift: false,
        };
        assert_eq!(TabStep::from_key_down(VK_TAB, win), None);
    }

    #[test]
    fn after_stepping_from_a_field_the_focus_goes_to_the_first_field_of_the_new_page() {
        assert_eq!(StepFocus::after(true), StepFocus::FirstField);
    }

    #[test]
    fn after_stepping_from_the_tabs_or_the_dialog_buttons_the_focus_stays() {
        assert_eq!(StepFocus::after(false), StepFocus::Stay);
    }

    #[test]
    fn stepping_wraps_around_at_both_ends() {
        assert_eq!(TabStep::Next.next_index(0, 6), 1);
        assert_eq!(TabStep::Next.next_index(5, 6), 0);
        assert_eq!(TabStep::Previous.next_index(3, 6), 2);
        assert_eq!(TabStep::Previous.next_index(0, 6), 5);
        // タブがなければ動かない
        assert_eq!(TabStep::Next.next_index(0, 0), 0);
    }
}
