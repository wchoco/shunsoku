//! 設定 GUI の、入力で変わる状態（写しの中身ではないもの）。Win32 の層は通知を渡して、決まったことに従う。

/// 定型領域の詳細の欄が指す領域。結び直したら、次に欄に出すときはフォーカスのある欄も書き換える。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegionDetail {
    index: usize,
    /// 結び直してから、まだ欄に出していない。
    rebound: bool,
}

impl RegionDetail {
    /// 領域 `index` に結び直す。
    pub fn bind(&mut self, index: usize) {
        self.index = index;
        self.rebound = true;
    }

    /// 指している領域。
    pub fn index(&self) -> usize {
        self.index
    }

    /// 欄に出し始める。この回にどの欄を書き換えるかを返す。
    pub fn begin_fill(&mut self) -> FillPass {
        FillPass {
            rebound: std::mem::take(&mut self.rebound),
        }
    }
}

/// 写しを欄に出す 1 回。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillPass {
    rebound: bool,
}

impl FillPass {
    /// テキストの欄を書き換えるか。`focused` はフォーカスがあるか、`region_field` は定型領域の詳細の欄か。
    /// 打っている途中の欄は書き換えない（末尾の空白などを消さない）。ただし結び直した直後の詳細の欄は、
    /// 前の領域の値が残らないよう書き換える。
    pub fn overwrites(self, focused: bool, region_field: bool) -> bool {
        !focused || (self.rebound && region_field)
    }
}

/// 設定 GUI のキーの組み合わせの欄にフォーカスがある間、全体のホットキーの登録を外しておく（押したキーが
/// 欄に届くように）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HotkeyPause {
    paused: bool,
}

/// ホットキーの登録をどうするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseChange {
    /// 登録を外す。
    Pause,
    /// 登録し直す。
    Resume,
}

impl HotkeyPause {
    /// 外している間か。この間は、設定が変わってもホットキーを登録しない。
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// フォーカスが変わった。`in_chord_field` はキーの組み合わせの欄にあるか。
    pub fn focus_changed(&mut self, in_chord_field: bool) -> Option<PauseChange> {
        match (in_chord_field, self.paused) {
            (true, false) => {
                self.paused = true;
                Some(PauseChange::Pause)
            }
            (false, true) => {
                self.paused = false;
                Some(PauseChange::Resume)
            }
            _ => None,
        }
    }

    /// 設定 GUI が閉じた。
    pub fn closed(&mut self) -> Option<PauseChange> {
        self.focus_changed(false)
    }

    /// 適用で、登録できるかを確かめるため登録し直す。欄にフォーカスが残っていれば、次のフォーカスの通知で外し直す。
    pub fn reregistering(&mut self) {
        self.paused = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_binding_another_region_the_next_fill_overwrites_the_focused_region_field_once() {
        let mut detail = RegionDetail::default();
        detail.bind(2);
        assert_eq!(detail.index(), 2);

        let pass = detail.begin_fill();
        // 前の領域の値が残らないよう、フォーカスのある詳細の欄も書き換える
        assert!(pass.overwrites(true, true));
        // 全般の欄でフォーカスのあるものは、打っている途中なので書き換えない
        assert!(!pass.overwrites(true, false));
        assert!(pass.overwrites(false, true));
        assert!(pass.overwrites(false, false));

        // 次からは、打っている途中の詳細の欄も書き換えない（末尾の空白を消さない）
        let pass = detail.begin_fill();
        assert!(!pass.overwrites(true, true));
        assert!(pass.overwrites(false, true));
    }

    #[test]
    fn hotkeys_pause_while_a_chord_field_has_focus_and_resume_when_it_leaves_or_the_gui_closes() {
        let mut pause = HotkeyPause::default();
        assert!(!pause.is_paused());
        assert_eq!(pause.focus_changed(true), Some(PauseChange::Pause));
        assert!(pause.is_paused());
        // 欄から欄へ移っても外したまま
        assert_eq!(pause.focus_changed(true), None);
        assert_eq!(pause.focus_changed(false), Some(PauseChange::Resume));
        assert_eq!(pause.focus_changed(false), None);
        assert!(!pause.is_paused());

        assert_eq!(pause.closed(), None);
        pause.focus_changed(true);
        assert_eq!(pause.closed(), Some(PauseChange::Resume));
        assert!(!pause.is_paused());
    }

    #[test]
    fn applying_registers_again_and_a_field_still_focused_pauses_again() {
        let mut pause = HotkeyPause::default();
        pause.focus_changed(true);
        pause.reregistering();
        assert!(!pause.is_paused());
        assert_eq!(pause.focus_changed(true), Some(PauseChange::Pause));
    }
}
