//! スクラップ操作の表（ADR-0005）。
//!
//! スクラップ操作は静的な表 `OPERATIONS` に 1 行足せば、右クリックメニュー（`menu`）とショートカットの
//! 対応表（`Shortcuts`）とマウスの対応（`mouse_operation`）の全部に出る。`run` は `ScrapModel` を変え、
//! Win32 が要る仕事（描き直し、閉じる、クリップボード、保存など）は実行せずに `Effect` として返す。
//! `Effect` を実行するのは Win32 層の `app`。操作を足すときは、表に行を足し、新しい `Effect` があれば
//! `app` でそれを実行する。
//!
//! 移動は、キーごとに引数のない操作に分ける（`move_left`、`move_left_10` など）。ショートカットは引数の
//! ない操作にだけ付き、設定ファイルの上書きは操作の id ごとに 1 つなので、こうすると矢印と Shift+矢印を
//! 1 つずつ割り当て直せる。左ドラッグの移動（`move`）はドラッグの量を `OpContext` で受け取るので、
//! ショートカットを持たず、メニューにも出さない。

use std::collections::BTreeMap;

use crate::geometry::Point;
use crate::keys::{Chord, Key};
use crate::scrap::ScrapModel;
use crate::settings::ShortcutOverrides;

/// スクラップ操作の id。設定ファイルの `[shortcuts]` のキーにもなる。
pub type OperationId = &'static str;

/// スクラップ操作 1 つ。
pub struct OperationDescriptor {
    pub id: OperationId,
    /// メニューに出す文言のキー（`locales/ja.toml`）。
    pub label_key: &'static str,
    /// 既定のショートカット。引数のある操作（`choices` が Some）には付けない。
    pub default_shortcut: &'static [Chord],
    pub default_mouse: Option<MouseGesture>,
    /// メニューの区切り。違う `group` の間に区切りを入れる。
    pub group: Group,
    /// `group` の中の並び（小さいほうが上）。
    pub order: u16,
    /// ドラッグの量（`OpContext::drag`）を引数に取る操作。ドラッグでしか呼べないので、メニューにも
    /// ショートカットの対応表にも出さない。
    pub drag_only: bool,
    /// いま実行できるか。できない操作はメニューでグレー表示にし、ショートカットでも実行しない。
    pub can_execute: fn(&ScrapModel) -> bool,
    /// 引数の選択肢（M2 の透明度など）。Some の操作はショートカットを持たない。
    pub choices: Option<fn(&ScrapModel) -> Vec<Choice>>,
    pub run: fn(&mut ScrapModel, &OpContext) -> Vec<Effect>,
}

impl OperationDescriptor {
    /// ショートカットを付けられるか。引数のない操作だけに付く。
    fn takes_shortcut(&self) -> bool {
        self.choices.is_none() && !self.drag_only
    }
}

impl std::fmt::Debug for OperationDescriptor {
    /// 関数のフィールドは出さない（id で分かる）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationDescriptor")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// マウスの操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseGesture {
    LeftDrag,
    MiddleClick,
}

/// メニューの区切りのまとまり。宣言の順にメニューに並ぶ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// コピーと保存。メニューのいちばん上。
    Output,
    Move,
    /// 閉じる。メニューのいちばん下。
    Close,
}

/// 引数のある操作の選択肢 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label_key: &'static str,
    pub value: i32,
}

/// 操作を呼び出したときの引数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpContext {
    /// 左ドラッグで前回からカーソルが動いた量（物理ピクセル）。
    pub drag: Point,
    /// 選ばれた選択肢の `Choice::value`。
    pub choice: Option<i32>,
}

/// 操作の結果、Win32 層がすること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// `ScrapModel` のとおりに出し直す（位置と中身）。
    Redraw,
    /// スクラップを閉じる。M1 は破棄する（M2 でごみ箱への移動に置き換える）。
    Close,
    /// 画像をクリップボードに置く。
    CopyToClipboard,
    /// 画像を全体設定の保存先に、全体のファイル名テンプレートの名前で保存する（ダイアログなし）。
    SaveDefault,
}

const ESC: Chord = Chord::plain(Key::known(0x1B));
const LEFT: Chord = Chord::plain(Key::known(0x25));
const UP: Chord = Chord::plain(Key::known(0x26));
const RIGHT: Chord = Chord::plain(Key::known(0x27));
const DOWN: Chord = Chord::plain(Key::known(0x28));
const CTRL_C: Chord = Chord::plain(Key::known(0x43)).ctrl();
const CTRL_S: Chord = Chord::plain(Key::known(0x53)).ctrl();

const fn always(_: &ScrapModel) -> bool {
    true
}

/// `(dx, dy)` だけ動かす操作の `run`。
macro_rules! move_by {
    ($dx:expr, $dy:expr) => {{
        fn run(model: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
            move_position(model, $dx, $dy)
        }
        run
    }};
}

fn move_position(model: &mut ScrapModel, dx: i32, dy: i32) -> Vec<Effect> {
    if dx == 0 && dy == 0 {
        return Vec::new();
    }
    let p = model.position;
    model.position = Point::new(p.x.saturating_add(dx), p.y.saturating_add(dy));
    vec![Effect::Redraw]
}

fn drag(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    move_position(model, context.drag.x, context.drag.y)
}

fn close(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::Close]
}

fn copy(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::CopyToClipboard]
}

fn save_default(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::SaveDefault]
}

/// キーで動かす操作の行。
const fn keyboard_move(
    id: OperationId,
    label_key: &'static str,
    shortcut: &'static [Chord],
    order: u16,
    run: fn(&mut ScrapModel, &OpContext) -> Vec<Effect>,
) -> OperationDescriptor {
    OperationDescriptor {
        id,
        label_key,
        default_shortcut: shortcut,
        default_mouse: None,
        group: Group::Move,
        order,
        drag_only: false,
        can_execute: always,
        choices: None,
        run,
    }
}

/// スクラップ操作の表。
pub static OPERATIONS: &[OperationDescriptor] = &[
    OperationDescriptor {
        id: "copy",
        label_key: "scrap.op.copy",
        default_shortcut: &[CTRL_C],
        default_mouse: None,
        group: Group::Output,
        order: 10,
        drag_only: false,
        can_execute: always,
        choices: None,
        run: copy,
    },
    OperationDescriptor {
        id: "save_default",
        label_key: "scrap.op.save_default",
        default_shortcut: &[CTRL_S],
        default_mouse: None,
        group: Group::Output,
        order: 20,
        drag_only: false,
        can_execute: always,
        choices: None,
        run: save_default,
    },
    keyboard_move(
        "move_left",
        "scrap.op.move_left",
        &[LEFT],
        10,
        move_by!(-1, 0),
    ),
    keyboard_move(
        "move_right",
        "scrap.op.move_right",
        &[RIGHT],
        20,
        move_by!(1, 0),
    ),
    keyboard_move("move_up", "scrap.op.move_up", &[UP], 30, move_by!(0, -1)),
    keyboard_move(
        "move_down",
        "scrap.op.move_down",
        &[DOWN],
        40,
        move_by!(0, 1),
    ),
    keyboard_move(
        "move_left_10",
        "scrap.op.move_left_10",
        &[LEFT.shift()],
        50,
        move_by!(-10, 0),
    ),
    keyboard_move(
        "move_right_10",
        "scrap.op.move_right_10",
        &[RIGHT.shift()],
        60,
        move_by!(10, 0),
    ),
    keyboard_move(
        "move_up_10",
        "scrap.op.move_up_10",
        &[UP.shift()],
        70,
        move_by!(0, -10),
    ),
    keyboard_move(
        "move_down_10",
        "scrap.op.move_down_10",
        &[DOWN.shift()],
        80,
        move_by!(0, 10),
    ),
    OperationDescriptor {
        id: "move",
        label_key: "scrap.op.move",
        default_shortcut: &[],
        default_mouse: Some(MouseGesture::LeftDrag),
        group: Group::Move,
        order: 0,
        drag_only: true,
        can_execute: always,
        choices: None,
        run: drag,
    },
    OperationDescriptor {
        id: "close",
        label_key: "scrap.op.close",
        default_shortcut: &[ESC],
        default_mouse: Some(MouseGesture::MiddleClick),
        group: Group::Close,
        order: 10,
        drag_only: false,
        can_execute: always,
        choices: None,
        run: close,
    },
];

/// id から表の行を引く。
pub fn find(id: &str) -> Option<&'static OperationDescriptor> {
    OPERATIONS.iter().find(|op| op.id == id)
}

/// ショートカットの対応表。表の既定値に、設定ファイルの上書き（`[shortcuts]`）を重ねて作る。
///
/// - 上書きは操作の id ごとに 1 つ。Some ならその操作の既定のショートカットを全部それ 1 つに置き換え、
///   None（設定ファイルの `""`）なら割り当てを外す。書かなかった操作は既定値のまま。
/// - 表にない id と、引数のある操作（`choices` が Some）への上書きは無視し、`ignored_overrides` に載せる
///   （設定の通知で項目名を出すため。通知するのは設定の監視のチケット #46）。
/// - 同じキーの組み合わせが 2 つ以上の操作に付いたら、どれにも付けず `conflicts` に載せる
///   （どちらが効くか分からない状態を避ける。spec #32）。
/// - スクラップにフォーカスがあるときだけ引く（ホットキーと違って OS には登録しない）。
#[derive(Debug, Clone)]
pub struct Shortcuts {
    bindings: BTreeMap<Chord, &'static OperationDescriptor>,
    ignored: Vec<String>,
    conflicts: Vec<Chord>,
}

impl Shortcuts {
    /// `ops`（ふつうは `OPERATIONS`）の既定値に `overrides` を重ねる。
    pub fn new(ops: &'static [OperationDescriptor], overrides: &ShortcutOverrides) -> Shortcuts {
        let bindable = |id: &str| ops.iter().any(|op| op.id == id && op.takes_shortcut());
        let ignored = overrides
            .keys()
            .filter(|id| !bindable(id))
            .cloned()
            .collect();
        let mut candidates: BTreeMap<Chord, Vec<&'static OperationDescriptor>> = BTreeMap::new();
        for op in ops.iter().filter(|op| op.takes_shortcut()) {
            let chords = match overrides.get(op.id) {
                Some(chord) => chord.as_slice(),
                None => op.default_shortcut,
            };
            for &chord in chords {
                candidates.entry(chord).or_default().push(op);
            }
        }
        let mut bindings = BTreeMap::new();
        let mut conflicts = Vec::new();
        for (chord, ops) in candidates {
            match ops.as_slice() {
                [op] => {
                    bindings.insert(chord, *op);
                }
                _ => conflicts.push(chord),
            }
        }
        Shortcuts {
            bindings,
            ignored,
            conflicts,
        }
    }

    /// キーの組み合わせに付いた操作。
    pub fn operation(&self, chord: Chord) -> Option<&'static OperationDescriptor> {
        self.bindings.get(&chord).copied()
    }

    /// 操作に付いたキーの組み合わせ。
    pub fn chords(&self, id: &str) -> Vec<Chord> {
        self.bindings
            .iter()
            .filter(|(_, op)| op.id == id)
            .map(|(&chord, _)| chord)
            .collect()
    }

    /// 無視した上書きの id（表にない、または引数のある操作）。
    pub fn ignored_overrides(&self) -> &[String] {
        &self.ignored
    }

    /// 2 つ以上の操作に付いたため、どれにも付けなかったキーの組み合わせ。
    pub fn conflicts(&self) -> &[Chord] {
        &self.conflicts
    }
}

/// マウスの操作に付いた操作（表の既定値。M1 では割り当て直せない）。
pub fn mouse_operation(
    ops: &'static [OperationDescriptor],
    gesture: MouseGesture,
) -> Option<&'static OperationDescriptor> {
    ops.iter().find(|op| op.default_mouse == Some(gesture))
}

/// 操作を実行する。いま実行できなければ何もしない（`Effect` なし）。ショートカット、マウス、メニューの
/// どこから呼んでもこれを通す。
pub fn execute(
    op: &OperationDescriptor,
    model: &mut ScrapModel,
    context: &OpContext,
) -> Vec<Effect> {
    if (op.can_execute)(model) {
        (op.run)(model, context)
    } else {
        Vec::new()
    }
}

/// 右クリックメニューの 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

/// 右クリックメニューの項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub id: OperationId,
    pub label_key: &'static str,
    /// false ならグレー表示（消さずに並びを保つ）。
    pub enabled: bool,
    /// 項目の右に出すショートカット。複数あれば最初の 1 つ。
    pub shortcut: Option<Chord>,
}

/// 右クリックメニューを `ops`（ふつうは `OPERATIONS`）から作る。`group` の宣言の順、その中は `order` の順に並べ、
/// `group` の変わり目に区切りを入れる。ドラッグでしか呼べない操作は出さない。
///
/// 引数のある操作（`choices`）を子メニューにするのは、最初にそういう操作を足すチケット（M2）で行う。
pub fn menu(
    ops: &'static [OperationDescriptor],
    model: &ScrapModel,
    shortcuts: &Shortcuts,
) -> Vec<MenuEntry> {
    let mut listed: Vec<&OperationDescriptor> = ops.iter().filter(|op| !op.drag_only).collect();
    listed.sort_by_key(|op| (op.group, op.order));
    let mut entries = Vec::new();
    let mut group = None;
    for op in listed {
        if group.is_some_and(|g| g != op.group) {
            entries.push(MenuEntry::Separator);
        }
        group = Some(op.group);
        entries.push(MenuEntry::Item(MenuItem {
            id: op.id,
            label_key: op.label_key,
            enabled: (op.can_execute)(model),
            shortcut: shortcuts.chords(op.id).first().copied(),
        }));
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Point, Size};
    use crate::scrap::ScrapModel;

    fn scrap() -> ScrapModel {
        ScrapModel::new(Point::new(-100, 50), Size::new(300, 200))
    }

    fn op(id: &str) -> &'static OperationDescriptor {
        find(id).unwrap_or_else(|| panic!("{id} is not in OPERATIONS"))
    }

    #[test]
    fn arrow_operations_move_the_scrap_by_1px_and_10px_and_redraw() {
        for (id, dx, dy) in [
            ("move_left", -1, 0),
            ("move_right", 1, 0),
            ("move_up", 0, -1),
            ("move_down", 0, 1),
            ("move_left_10", -10, 0),
            ("move_right_10", 10, 0),
            ("move_up_10", 0, -10),
            ("move_down_10", 0, 10),
        ] {
            let mut model = scrap();
            let effects = (op(id).run)(&mut model, &OpContext::default());
            assert_eq!(effects, vec![Effect::Redraw], "{id}");
            assert_eq!(model.position, Point::new(-100 + dx, 50 + dy), "{id}");
            assert_eq!(model.image_size, Size::new(300, 200), "{id}");
        }
    }

    #[test]
    fn dragging_moves_the_scrap_by_the_cursor_movement() {
        let mut model = scrap();
        let drag = OpContext {
            drag: Point::new(25, -7),
            ..OpContext::default()
        };
        assert_eq!((op("move").run)(&mut model, &drag), vec![Effect::Redraw]);
        assert_eq!(model.position, Point::new(-75, 43));

        // 動いていなければ描き直さない
        assert_eq!((op("move").run)(&mut model, &OpContext::default()), vec![]);
        assert_eq!(model.position, Point::new(-75, 43));
    }

    #[test]
    fn close_closes_the_scrap_without_changing_it() {
        let mut model = scrap();
        let effects = (op("close").run)(&mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::Close]);
        assert_eq!(model, scrap());
    }

    #[test]
    fn copy_and_save_hand_the_image_to_the_win32_layer_without_changing_the_scrap() {
        for (id, effect) in [
            ("copy", Effect::CopyToClipboard),
            ("save_default", Effect::SaveDefault),
        ] {
            let mut model = scrap();
            let effects = execute(op(id), &mut model, &OpContext::default());
            assert_eq!(effects, vec![effect], "{id}");
            assert_eq!(model, scrap(), "{id}");
        }
    }

    #[test]
    fn move_and_close_can_always_run_on_a_normal_scrap() {
        for id in [
            "move",
            "move_left",
            "move_down_10",
            "copy",
            "save_default",
            "close",
        ] {
            assert!((op(id).can_execute)(&scrap()), "{id}");
        }
    }

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    fn bound(shortcuts: &Shortcuts, s: &str) -> Option<OperationId> {
        shortcuts.operation(chord(s)).map(|op| op.id)
    }

    #[test]
    fn default_shortcuts_come_from_the_table() {
        let shortcuts = Shortcuts::new(OPERATIONS, &ShortcutOverrides::new());
        assert_eq!(bound(&shortcuts, "Left"), Some("move_left"));
        assert_eq!(bound(&shortcuts, "Right"), Some("move_right"));
        assert_eq!(bound(&shortcuts, "Up"), Some("move_up"));
        assert_eq!(bound(&shortcuts, "Down"), Some("move_down"));
        assert_eq!(bound(&shortcuts, "Shift+Left"), Some("move_left_10"));
        assert_eq!(bound(&shortcuts, "Shift+Right"), Some("move_right_10"));
        assert_eq!(bound(&shortcuts, "Shift+Up"), Some("move_up_10"));
        assert_eq!(bound(&shortcuts, "Shift+Down"), Some("move_down_10"));
        assert_eq!(bound(&shortcuts, "Esc"), Some("close"));
        assert_eq!(bound(&shortcuts, "Ctrl+C"), Some("copy"));
        assert_eq!(bound(&shortcuts, "Ctrl+S"), Some("save_default"));
        assert_eq!(bound(&shortcuts, "Ctrl+Left"), None);
        assert_eq!(shortcuts.chords("close"), vec![chord("Esc")]);
        assert_eq!(shortcuts.chords("move"), vec![]);
        assert!(shortcuts.ignored_overrides().is_empty());
    }

    #[test]
    fn overrides_replace_or_remove_the_default_shortcut_of_that_operation_only() {
        let overrides = ShortcutOverrides::from([
            ("close".to_owned(), Some(chord("Ctrl+W"))),
            ("move_left".to_owned(), None),
        ]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        assert_eq!(bound(&shortcuts, "Ctrl+W"), Some("close"));
        assert_eq!(bound(&shortcuts, "Esc"), None);
        assert_eq!(bound(&shortcuts, "Left"), None);
        assert_eq!(shortcuts.chords("move_left"), vec![]);
        assert_eq!(bound(&shortcuts, "Shift+Left"), Some("move_left_10"));
        assert_eq!(bound(&shortcuts, "Right"), Some("move_right"));
    }

    #[test]
    fn overrides_for_unknown_or_argument_taking_operations_are_ignored_and_reported() {
        let overrides = ShortcutOverrides::from([
            ("fly".to_owned(), Some(chord("F"))),
            ("move".to_owned(), Some(chord("M"))),
            ("close".to_owned(), Some(chord("Q"))),
        ]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        assert_eq!(bound(&shortcuts, "F"), None);
        assert_eq!(bound(&shortcuts, "M"), None);
        assert_eq!(bound(&shortcuts, "Q"), Some("close"));
        assert_eq!(shortcuts.ignored_overrides(), ["fly", "move"]);
    }

    #[test]
    fn an_operation_with_choices_gets_no_shortcut() {
        fn opacity_choices(_: &ScrapModel) -> Vec<Choice> {
            vec![Choice {
                label_key: "opacity.50",
                value: 50,
            }]
        }
        static TABLE: &[OperationDescriptor] = &[OperationDescriptor {
            choices: Some(opacity_choices),
            ..test_op("opacity", Group::Move, 10, &[ESC])
        }];
        let shortcuts = Shortcuts::new(TABLE, &ShortcutOverrides::new());
        assert_eq!(bound(&shortcuts, "Esc"), None);
        let overrides = ShortcutOverrides::from([("opacity".to_owned(), Some(chord("O")))]);
        let shortcuts = Shortcuts::new(TABLE, &overrides);
        assert_eq!(bound(&shortcuts, "O"), None);
        assert_eq!(shortcuts.ignored_overrides(), ["opacity"]);
    }

    fn item(id: OperationId, enabled: bool, shortcut: Option<&str>) -> MenuEntry {
        MenuEntry::Item(MenuItem {
            id,
            label_key: find(id).map_or(id, |op| op.label_key),
            enabled,
            shortcut: shortcut.map(chord),
        })
    }

    #[test]
    fn the_menu_lists_the_operations_by_group_and_order_with_their_shortcuts() {
        let shortcuts = Shortcuts::new(OPERATIONS, &ShortcutOverrides::new());
        assert_eq!(
            menu(OPERATIONS, &scrap(), &shortcuts),
            vec![
                item("copy", true, Some("Ctrl+C")),
                item("save_default", true, Some("Ctrl+S")),
                MenuEntry::Separator,
                item("move_left", true, Some("Left")),
                item("move_right", true, Some("Right")),
                item("move_up", true, Some("Up")),
                item("move_down", true, Some("Down")),
                item("move_left_10", true, Some("Shift+Left")),
                item("move_right_10", true, Some("Shift+Right")),
                item("move_up_10", true, Some("Shift+Up")),
                item("move_down_10", true, Some("Shift+Down")),
                MenuEntry::Separator,
                item("close", true, Some("Esc")),
            ]
        );
    }

    #[test]
    fn the_menu_shows_overridden_shortcuts() {
        let overrides = ShortcutOverrides::from([
            ("close".to_owned(), Some(chord("Ctrl+W"))),
            ("move_left".to_owned(), None),
        ]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        let entries = menu(OPERATIONS, &scrap(), &shortcuts);
        assert!(entries.contains(&item("move_left", true, None)));
        assert_eq!(entries.last(), Some(&item("close", true, Some("Ctrl+W"))));
    }

    #[test]
    fn the_menu_is_sorted_and_separated_by_group_greys_out_and_skips_drag_only_operations() {
        fn never(_: &ScrapModel) -> bool {
            false
        }
        static TABLE: &[OperationDescriptor] = &[
            test_op("z", Group::Close, 10, &[]),
            test_op("b", Group::Move, 20, &[]),
            OperationDescriptor {
                drag_only: true,
                ..test_op("drag", Group::Move, 0, &[])
            },
            OperationDescriptor {
                can_execute: never,
                ..test_op("a", Group::Move, 10, &[])
            },
        ];
        let shortcuts = Shortcuts::new(TABLE, &ShortcutOverrides::new());
        assert_eq!(
            menu(TABLE, &scrap(), &shortcuts),
            vec![
                item("a", false, None),
                item("b", true, None),
                MenuEntry::Separator,
                item("z", true, None),
            ]
        );
    }

    #[test]
    fn two_operations_on_the_same_chord_are_both_disabled() {
        let overrides = ShortcutOverrides::from([("close".to_owned(), Some(chord("Left")))]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        assert_eq!(bound(&shortcuts, "Left"), None);
        assert_eq!(shortcuts.chords("close"), vec![]);
        assert_eq!(shortcuts.chords("move_left"), vec![]);
        assert_eq!(shortcuts.conflicts(), [chord("Left")]);
    }

    #[test]
    fn mouse_gestures_map_to_move_and_close() {
        let id = |g| mouse_operation(OPERATIONS, g).map(|op| op.id);
        assert_eq!(id(MouseGesture::LeftDrag), Some("move"));
        assert_eq!(id(MouseGesture::MiddleClick), Some("close"));
    }

    #[test]
    fn execute_runs_only_operations_that_can_execute() {
        fn never(_: &ScrapModel) -> bool {
            false
        }
        static BLOCKED: OperationDescriptor = OperationDescriptor {
            can_execute: never,
            run: move_by!(5, 5),
            ..test_op("blocked", Group::Move, 0, &[])
        };
        let mut model = scrap();
        assert_eq!(execute(&BLOCKED, &mut model, &OpContext::default()), vec![]);
        assert_eq!(model, scrap());
        assert_eq!(
            execute(op("move_right"), &mut model, &OpContext::default()),
            vec![Effect::Redraw]
        );
        assert_eq!(model.position, Point::new(-99, 50));
    }

    #[test]
    fn every_operation_has_a_unique_id_a_label_and_displayable_shortcuts() {
        let catalog =
            crate::i18n::Catalog::parse(include_str!("../../../../locales/ja.toml")).unwrap();
        for (i, op) in OPERATIONS.iter().enumerate() {
            assert!(
                OPERATIONS[..i].iter().all(|other| other.id != op.id),
                "duplicate id {}",
                op.id
            );
            assert_ne!(catalog.text(op.label_key), op.label_key, "{}", op.id);
            for chord in op.default_shortcut {
                // Key::known で作ったキーが名前の表にある（設定ファイルに書け、表示できる）
                assert_eq!(chord.to_string().parse::<Chord>(), Ok(*chord), "{}", op.id);
            }
        }
    }

    /// 表の組み立てを確かめるための行。
    const fn test_op(
        id: OperationId,
        group: Group,
        order: u16,
        default_shortcut: &'static [Chord],
    ) -> OperationDescriptor {
        fn nothing(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
            Vec::new()
        }
        OperationDescriptor {
            id,
            label_key: id,
            default_shortcut,
            default_mouse: None,
            group,
            order,
            drag_only: false,
            can_execute: always,
            choices: None,
            run: nothing,
        }
    }
}
