//! 範囲選択キャプチャのホットキーが未設定のときの促し方（初回起動の流れ）。

use super::Settings;

/// 未設定かを見る時点。
#[derive(Debug, Clone, Copy)]
pub enum When<'a> {
    /// 起動した。
    Startup,
    /// 実行中に設定ファイルを読み込み直した。`before` は読み込み直す前の設定。
    Reload { before: &'a Settings },
    /// 設定 GUI を閉じた（未設定のままでは OK・適用を押せないので、キャンセルで閉じたことになる）。
    GuiClosed,
}

/// どう促すか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// 設定 GUI を「キャプチャ」タブで開き、範囲選択キャプチャの欄にフォーカスを置く。
    OpenGui,
    /// 未設定をトレイの通知で知らせる。
    Notify,
}

/// `when` の時点の設定 `settings` で、未設定をどう促すか。促さないなら None。
pub fn prompt(when: When, settings: &Settings) -> Option<Prompt> {
    let unset = |s: &Settings| s.hotkeys.select_capture.is_none();
    if !unset(settings) {
        return None;
    }
    match when {
        When::Startup => Some(Prompt::OpenGui),
        // 空になったときだけ。前から空なら、起動時か GUI を閉じたときに知らせてある
        When::Reload { before } => (!unset(before)).then_some(Prompt::Notify),
        When::GuiClosed => Some(Prompt::Notify),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_hotkey() -> Settings {
        let mut s = Settings::default();
        s.hotkeys.select_capture = Some("Ctrl+Shift+A".parse().unwrap());
        s
    }

    #[test]
    fn startup_without_hotkey_opens_the_gui() {
        assert_eq!(
            prompt(When::Startup, &Settings::default()),
            Some(Prompt::OpenGui)
        );
    }

    #[test]
    fn startup_with_hotkey_prompts_nothing() {
        assert_eq!(prompt(When::Startup, &with_hotkey()), None);
    }

    #[test]
    fn reload_that_empties_the_hotkey_only_notifies() {
        let before = with_hotkey();
        assert_eq!(
            prompt(When::Reload { before: &before }, &Settings::default()),
            Some(Prompt::Notify)
        );
    }

    #[test]
    fn reload_that_was_already_empty_prompts_nothing() {
        let before = Settings::default();
        assert_eq!(
            prompt(When::Reload { before: &before }, &Settings::default()),
            None
        );
    }

    #[test]
    fn reload_that_keeps_the_hotkey_prompts_nothing() {
        let before = with_hotkey();
        assert_eq!(
            prompt(When::Reload { before: &before }, &with_hotkey()),
            None
        );
    }

    #[test]
    fn closing_the_gui_without_hotkey_notifies() {
        assert_eq!(
            prompt(When::GuiClosed, &Settings::default()),
            Some(Prompt::Notify)
        );
    }

    #[test]
    fn closing_the_gui_with_hotkey_prompts_nothing() {
        assert_eq!(prompt(When::GuiClosed, &with_hotkey()), None);
    }
}
