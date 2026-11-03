//! 設定 GUI が編集する設定の写し（`Editor`）。
//!
//! - 開いた時点の設定を base として持ち、GUI の欄は写し（`draft`）だけを変える。ファイルに書くのは適用のとき。
//! - 適用できるか（OK・適用のボタン）と、欄の横の印（`Mark`）はここで決める。適用の可否は `validate` による。
//! - 適用で書けたら `applied` で写しを新しい base にし、登録できなかったホットキーを覚える。
//! - キーの組み合わせの欄（ホットキー・ショートカット）は `chord` / `set_chord` で `SettingsItem` を指して読み書きする。
//!   ほかの項目は `draft_mut` で写しを直接変える。
//! - 「スクラップ操作」タブの行（`scrap::ops::BindingRow`）は、上書きがなければ表の既定を出す（`shortcut` /
//!   `mouse`）。ショートカットの欄にキーを入れたら、その 1 つで上書きする（`set_chord`）。マウスは候補から選ぶ
//!   （`set_mouse`。既定と同じなら上書きを消す）。行の「既定に戻す」は両方の上書きを消す（`reset_binding`）。

use std::collections::BTreeSet;

use super::{Problem, Settings, SettingsItem};
use crate::keys::Chord;
use crate::scrap::ops::{BindingRow, MouseGesture};

/// 設定 GUI の写し。
#[derive(Debug, Clone)]
pub struct Editor {
    base: Settings,
    draft: Settings,
    /// 最後の適用で登録できなかったホットキー。写しの値が base と同じ間だけ印にする。
    not_registered: BTreeSet<SettingsItem>,
}

/// 欄の横に出す印。1 つの欄には優先の高い 1 つだけを出す（適用を止める問題 → 登録できなかった → 警告）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// `validate` が見つけた問題。
    Problem(Problem),
    /// 適用したが、ほかのアプリが使っていてホットキーを登録できなかった。
    NotRegistered,
}

impl Mark {
    /// 適用を止める印か。
    pub fn blocks_apply(self) -> bool {
        match self {
            Mark::Problem(p) => p.blocks_apply(),
            Mark::NotRegistered => false,
        }
    }

    /// 印の文言のキー（`settings_gui.mark.*`）。
    pub fn label_key(self) -> &'static str {
        match self {
            Mark::NotRegistered => "settings_gui.mark.not_registered",
            Mark::Problem(Problem::Unset) => "settings_gui.mark.unset",
            Mark::Problem(Problem::Conflict(_)) => "settings_gui.mark.conflict",
            Mark::Problem(Problem::MouseConflict(_)) => "settings_gui.mark.mouse_conflict",
            Mark::Problem(Problem::Overlap(_)) => "settings_gui.mark.overlap",
            Mark::Problem(Problem::EmptyName) => "settings_gui.mark.empty_name",
            Mark::Problem(Problem::DuplicateName) => "settings_gui.mark.duplicate_name",
        }
    }
}

/// 「スクラップ操作」タブのショートカットの欄に出す値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutValue {
    /// 上書きがない。表の既定（複数のこともあり、空なら割り当てなし）をまとめて表示だけする。
    Defaults(&'static [Chord]),
    /// 上書きがある。None は割り当てを外した（設定ファイルの `""`）。
    Set(Option<Chord>),
}

/// OK と適用のボタンを押せるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Buttons {
    pub ok: bool,
    pub apply: bool,
}

impl Editor {
    /// 今の設定 `current` を base にして開く。
    pub fn new(current: Settings) -> Editor {
        Editor {
            draft: current.clone(),
            base: current,
            not_registered: BTreeSet::new(),
        }
    }

    /// 写しを直接変える。キーの組み合わせの欄は `set_chord` を使う。
    pub fn draft_mut(&mut self) -> &mut Settings {
        &mut self.draft
    }

    /// 適用で `written` を書けた。写しを新しい base にし、登録できなかったホットキー `not_registered` を覚える。
    pub fn applied(
        &mut self,
        written: Settings,
        not_registered: impl IntoIterator<Item = SettingsItem>,
    ) {
        self.draft = written.clone();
        self.base = written;
        self.not_registered = not_registered.into_iter().collect();
    }

    /// キーの組み合わせの欄 `item` の写しの値。キーの組み合わせの項目でなければ None。
    pub fn chord(&self, item: &SettingsItem) -> Option<Chord> {
        chord_of(&self.draft, item)
    }

    /// 欄 `item` に出す印。
    pub fn mark(&self, item: &SettingsItem) -> Option<Mark> {
        let problems: Vec<Problem> = super::validate(&self.draft)
            .into_iter()
            .filter(|i| &i.item == item)
            .map(|i| i.problem)
            .collect();
        if let Some(&p) = problems.iter().find(|p| p.blocks_apply()) {
            return Some(Mark::Problem(p));
        }
        if self.not_registered.contains(item)
            && chord_of(&self.draft, item) == chord_of(&self.base, item)
        {
            return Some(Mark::NotRegistered);
        }
        problems.first().map(|&p| Mark::Problem(p))
    }

    /// 開いた時点（か最後に適用した時点）の設定。
    pub fn base(&self) -> &Settings {
        &self.base
    }

    /// 写し。
    pub fn draft(&self) -> &Settings {
        &self.draft
    }

    /// キーの組み合わせの欄 `item` に `chord` を入れる。None は空（割り当てなし）。キーの組み合わせの項目で
    /// なければ何もしない。
    pub fn set_chord(&mut self, item: &SettingsItem, chord: Option<Chord>) {
        if let Some(slot) = chord_slot(&mut self.draft, item) {
            *slot = chord;
        }
    }

    /// 行 `row` のショートカットの欄に出す値。
    pub fn shortcut(&self, row: &BindingRow) -> ShortcutValue {
        match self.draft.shortcuts.get(row.op.id) {
            Some(&chord) => ShortcutValue::Set(chord),
            None => ShortcutValue::Defaults(row.default_shortcut.unwrap_or_default()),
        }
    }

    /// 行 `row` のマウスの割り当て。上書きが候補にない値なら、スクラップと同じく表の既定にする。
    pub fn mouse(&self, row: &BindingRow) -> Option<MouseGesture> {
        match self.draft.mouse.get(row.op.id) {
            Some(&gesture) if gesture.is_none_or(|g| row.mouse_choices().contains(&g)) => gesture,
            _ => row.op.default_mouse,
        }
    }

    /// 行 `row` のマウスの割り当てを `gesture`（None は割り当てなし）にする。表の既定と同じなら上書きを消す。
    pub fn set_mouse(&mut self, row: &BindingRow, gesture: Option<MouseGesture>) {
        if gesture == row.op.default_mouse {
            self.draft.mouse.remove(row.op.id);
        } else {
            self.draft.mouse.insert(row.op.id.to_owned(), gesture);
        }
    }

    /// 行 `row` のショートカットとマウスの上書きを消し、表の既定に戻す。
    pub fn reset_binding(&mut self, row: &BindingRow) {
        self.draft.shortcuts.remove(row.op.id);
        self.draft.mouse.remove(row.op.id);
    }

    /// 行 `row` にショートカットかマウスの上書きがあるか（「既定に戻す」を押せるか）。
    pub fn is_overridden(&self, row: &BindingRow) -> bool {
        self.draft.shortcuts.contains_key(row.op.id) || self.draft.mouse.contains_key(row.op.id)
    }

    /// OK と適用を押せるか。OK は適用を止める問題がないとき、適用はそれに加えて写しが base と違うとき。
    pub fn buttons(&self) -> Buttons {
        let ok = !super::validate(&self.draft)
            .iter()
            .any(super::ValidationIssue::blocks_apply);
        Buttons {
            ok,
            apply: ok && self.draft != self.base,
        }
    }
}

/// 設定 `settings` の、キーの組み合わせの項目 `item` の値。
fn chord_of(settings: &Settings, item: &SettingsItem) -> Option<Chord> {
    match item {
        SettingsItem::SelectCaptureHotkey => settings.hotkeys.select_capture,
        SettingsItem::RestoreLastHotkey => settings.hotkeys.restore_last,
        SettingsItem::ActiveWindowCaptureHotkey => settings.hotkeys.active_window_capture,
        SettingsItem::RegionHotkey(i) => settings.regions.get(*i).and_then(|r| r.hotkey),
        SettingsItem::Shortcut(id) => settings.shortcuts.get(id).copied().flatten(),
        SettingsItem::Mouse(_) | SettingsItem::RegionName(_) => None,
    }
}

/// 写しの中の、キーの組み合わせの項目の置き場所。
fn chord_slot<'a>(
    settings: &'a mut Settings,
    item: &SettingsItem,
) -> Option<&'a mut Option<Chord>> {
    match item {
        SettingsItem::SelectCaptureHotkey => Some(&mut settings.hotkeys.select_capture),
        SettingsItem::RestoreLastHotkey => Some(&mut settings.hotkeys.restore_last),
        SettingsItem::ActiveWindowCaptureHotkey => {
            Some(&mut settings.hotkeys.active_window_capture)
        }
        SettingsItem::RegionHotkey(i) => settings.regions.get_mut(*i).map(|r| &mut r.hotkey),
        SettingsItem::Shortcut(id) => Some(settings.shortcuts.entry(id.clone()).or_default()),
        SettingsItem::Mouse(_) | SettingsItem::RegionName(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Chord;
    use crate::scrap::ops::{BindingRow, MouseGesture, OPERATIONS, binding_rows};
    use crate::settings::{Settings, SettingsItem};

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn the_select_capture_hotkey_set_in_the_copy_enables_ok_and_apply_but_leaves_the_base() {
        let mut editor = Editor::new(Settings::default());
        assert_eq!(
            editor.buttons(),
            Buttons {
                ok: false,
                apply: false
            }
        );

        editor.set_chord(
            &SettingsItem::SelectCaptureHotkey,
            Some(chord("Ctrl+Shift+A")),
        );

        assert_eq!(
            editor.buttons(),
            Buttons {
                ok: true,
                apply: true
            }
        );
        assert_eq!(
            editor.draft().hotkeys.select_capture,
            Some(chord("Ctrl+Shift+A"))
        );
        assert_eq!(editor.base().hotkeys.select_capture, None);
    }

    #[test]
    fn an_empty_select_capture_hotkey_is_marked_unset() {
        let editor = Editor::new(Settings::default());
        assert_eq!(
            editor.mark(&SettingsItem::SelectCaptureHotkey),
            Some(Mark::Problem(Problem::Unset))
        );
        assert_eq!(editor.mark(&SettingsItem::RestoreLastHotkey), None);
    }

    #[test]
    fn hotkeys_overlapping_each_other_mark_both_fields_and_block_ok_and_apply() {
        let mut editor = Editor::new(Settings::default());
        editor.set_chord(&SettingsItem::SelectCaptureHotkey, Some(chord("Ctrl+Q")));
        editor.set_chord(&SettingsItem::RestoreLastHotkey, Some(chord("Ctrl+Q")));

        let conflict = Some(Mark::Problem(Problem::Conflict(chord("Ctrl+Q"))));
        assert_eq!(editor.mark(&SettingsItem::SelectCaptureHotkey), conflict);
        assert_eq!(editor.mark(&SettingsItem::RestoreLastHotkey), conflict);
        assert_eq!(
            editor.buttons(),
            Buttons {
                ok: false,
                apply: false
            }
        );
    }

    #[test]
    fn a_hotkey_overlapping_a_shortcut_is_only_a_warning() {
        let mut editor = Editor::new(Settings::default());
        // Ctrl+C は copy の既定のショートカット
        editor.set_chord(&SettingsItem::SelectCaptureHotkey, Some(chord("Ctrl+C")));

        assert_eq!(
            editor.mark(&SettingsItem::SelectCaptureHotkey),
            Some(Mark::Problem(Problem::Overlap(chord("Ctrl+C"))))
        );
        assert!(!Mark::Problem(Problem::Overlap(chord("Ctrl+C"))).blocks_apply());
        assert_eq!(
            editor.buttons(),
            Buttons {
                ok: true,
                apply: true
            }
        );
    }

    #[test]
    fn applying_makes_the_copy_the_new_base_and_marks_hotkeys_that_could_not_be_registered() {
        let mut editor = Editor::new(Settings::default());
        editor.set_chord(
            &SettingsItem::SelectCaptureHotkey,
            Some(chord("Ctrl+Shift+A")),
        );
        let written = editor.draft().clone();

        editor.applied(written.clone(), [SettingsItem::SelectCaptureHotkey]);

        assert_eq!(editor.base(), &written);
        assert_eq!(
            editor.buttons(),
            Buttons {
                ok: true,
                apply: false
            }
        );
        assert_eq!(
            editor.mark(&SettingsItem::SelectCaptureHotkey),
            Some(Mark::NotRegistered)
        );
    }

    #[test]
    fn the_not_registered_mark_goes_away_once_the_field_holds_another_key() {
        let mut editor = Editor::new(Settings::default());
        editor.set_chord(
            &SettingsItem::SelectCaptureHotkey,
            Some(chord("Ctrl+Shift+A")),
        );
        editor.applied(editor.draft().clone(), [SettingsItem::SelectCaptureHotkey]);

        editor.set_chord(
            &SettingsItem::SelectCaptureHotkey,
            Some(chord("Ctrl+Shift+B")),
        );
        assert_eq!(editor.mark(&SettingsItem::SelectCaptureHotkey), None);

        editor.set_chord(
            &SettingsItem::SelectCaptureHotkey,
            Some(chord("Ctrl+Shift+A")),
        );
        assert_eq!(
            editor.mark(&SettingsItem::SelectCaptureHotkey),
            Some(Mark::NotRegistered)
        );

        // 次に適用して登録できたら消える
        editor.applied(editor.draft().clone(), []);
        assert_eq!(editor.mark(&SettingsItem::SelectCaptureHotkey), None);
    }

    #[test]
    fn a_problem_that_blocks_apply_is_shown_before_a_registration_failure_and_a_warning() {
        let mut editor = Editor::new(Settings::default());
        editor.set_chord(&SettingsItem::SelectCaptureHotkey, Some(chord("Ctrl+C")));
        editor.set_chord(&SettingsItem::RestoreLastHotkey, Some(chord("Ctrl+Q")));
        editor.applied(
            editor.draft().clone(),
            [
                SettingsItem::SelectCaptureHotkey,
                SettingsItem::RestoreLastHotkey,
            ],
        );
        // Ctrl+C はショートカットとの重なり（警告）。登録の失敗を先に出す
        assert_eq!(
            editor.mark(&SettingsItem::SelectCaptureHotkey),
            Some(Mark::NotRegistered)
        );

        editor.set_chord(
            &SettingsItem::ActiveWindowCaptureHotkey,
            Some(chord("Ctrl+Q")),
        );
        assert_eq!(
            editor.mark(&SettingsItem::RestoreLastHotkey),
            Some(Mark::Problem(Problem::Conflict(chord("Ctrl+Q"))))
        );
    }

    #[test]
    fn chord_fields_read_the_copy() {
        let mut settings = Settings::default();
        settings.hotkeys.select_capture = Some(chord("Win+Shift+S"));
        let mut editor = Editor::new(settings);
        assert_eq!(
            editor.chord(&SettingsItem::SelectCaptureHotkey),
            Some(chord("Win+Shift+S"))
        );
        editor.set_chord(&SettingsItem::RestoreLastHotkey, Some(chord("Ctrl+Alt+Z")));
        assert_eq!(
            editor.chord(&SettingsItem::RestoreLastHotkey),
            Some(chord("Ctrl+Alt+Z"))
        );
        assert_eq!(editor.chord(&SettingsItem::Mouse("close".into())), None);
    }

    #[test]
    fn every_mark_has_a_text_in_the_bundled_locale() {
        let catalog =
            crate::i18n::Catalog::parse(include_str!("../../../../locales/ja.toml")).unwrap();
        let c = chord("Ctrl+Q");
        for mark in [
            Mark::NotRegistered,
            Mark::Problem(Problem::Unset),
            Mark::Problem(Problem::Conflict(c)),
            Mark::Problem(Problem::MouseConflict(
                crate::scrap::ops::MouseGesture::WheelUp,
            )),
            Mark::Problem(Problem::Overlap(c)),
            Mark::Problem(Problem::EmptyName),
            Mark::Problem(Problem::DuplicateName),
        ] {
            let key = mark.label_key();
            assert_ne!(catalog.text(key), key, "{mark:?}");
        }
    }

    fn row(id: &str) -> BindingRow {
        binding_rows(OPERATIONS)
            .into_iter()
            .find(|row| row.op.id == id)
            .unwrap()
    }

    #[test]
    fn several_default_shortcuts_are_shown_until_a_key_overrides_them_and_come_back_on_reset() {
        let mut settings = Settings::default();
        settings.hotkeys.select_capture = Some(chord("Ctrl+Shift+A"));
        let mut editor = Editor::new(settings);
        let zoom_in = row("zoom_in");
        let ShortcutValue::Defaults(defaults) = editor.shortcut(&zoom_in) else {
            panic!("zoom_in should show its defaults");
        };
        assert_eq!(defaults.len(), 3);
        assert!(!editor.is_overridden(&zoom_in));

        let item = SettingsItem::Shortcut("zoom_in".into());
        editor.set_chord(&item, Some(chord("Ctrl+Up")));
        assert_eq!(
            editor.shortcut(&zoom_in),
            ShortcutValue::Set(Some(chord("Ctrl+Up")))
        );
        assert!(editor.is_overridden(&zoom_in));
        assert!(editor.buttons().apply);

        editor.reset_binding(&zoom_in);
        assert_eq!(editor.shortcut(&zoom_in), ShortcutValue::Defaults(defaults));
        assert_eq!(editor.draft(), editor.base());
    }

    #[test]
    fn a_shortcut_unbound_in_the_file_is_shown_as_unset() {
        let mut settings = Settings::default();
        settings.shortcuts.insert("copy".into(), None);
        let editor = Editor::new(settings);
        assert_eq!(editor.shortcut(&row("copy")), ShortcutValue::Set(None));
        assert!(editor.is_overridden(&row("copy")));
    }

    #[test]
    fn the_same_shortcut_on_two_operations_marks_both_rows_and_blocks_apply() {
        let mut settings = Settings::default();
        settings.hotkeys.select_capture = Some(chord("Ctrl+Shift+A"));
        let mut editor = Editor::new(settings);
        editor.set_chord(
            &SettingsItem::Shortcut("trim".into()),
            Some(chord("Ctrl+Q")),
        );
        editor.set_chord(
            &SettingsItem::Shortcut("close".into()),
            Some(chord("Ctrl+Q")),
        );

        let conflict = Some(Mark::Problem(Problem::Conflict(chord("Ctrl+Q"))));
        assert_eq!(
            editor.mark(&SettingsItem::Shortcut("trim".into())),
            conflict
        );
        assert_eq!(
            editor.mark(&SettingsItem::Shortcut("close".into())),
            conflict
        );
        assert!(!editor.buttons().ok);
    }

    #[test]
    fn the_mouse_choice_shows_the_default_and_choosing_the_default_again_drops_the_override() {
        let mut editor = Editor::new(Settings::default());
        let close = row("close");
        assert_eq!(editor.mouse(&close), Some(MouseGesture::MiddleClick));

        editor.set_mouse(&close, Some(MouseGesture::DoubleClick));
        assert_eq!(editor.mouse(&close), Some(MouseGesture::DoubleClick));
        assert!(editor.is_overridden(&close));

        editor.set_mouse(&close, None);
        assert_eq!(editor.mouse(&close), None);

        editor.set_mouse(&close, Some(MouseGesture::MiddleClick));
        assert_eq!(editor.mouse(&close), Some(MouseGesture::MiddleClick));
        assert!(!editor.is_overridden(&close));
        assert_eq!(editor.draft(), editor.base());
    }

    #[test]
    fn a_mouse_override_outside_the_choices_shows_the_default() {
        let mut settings = Settings::default();
        settings
            .mouse
            .insert("close".into(), Some(MouseGesture::LeftDrag));
        let editor = Editor::new(settings);
        assert_eq!(editor.mouse(&row("close")), Some(MouseGesture::MiddleClick));
    }

    #[test]
    fn the_same_mouse_choice_on_two_operations_marks_both_rows() {
        let mut settings = Settings::default();
        settings.hotkeys.select_capture = Some(chord("Ctrl+Shift+A"));
        let mut editor = Editor::new(settings);
        // toggle_compact の既定はダブルクリック
        editor.set_mouse(&row("close"), Some(MouseGesture::DoubleClick));

        let conflict = Some(Mark::Problem(Problem::MouseConflict(
            MouseGesture::DoubleClick,
        )));
        assert_eq!(editor.mark(&SettingsItem::Mouse("close".into())), conflict);
        assert_eq!(
            editor.mark(&SettingsItem::Mouse("toggle_compact".into())),
            conflict
        );
        assert!(!editor.buttons().ok);

        editor.reset_binding(&row("close"));
        assert_eq!(editor.mark(&SettingsItem::Mouse("close".into())), None);
    }
}
