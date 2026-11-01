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
//!
//! 拡大縮小も同じ理由で、引数のない拡大（`zoom_in`）・縮小（`zoom_out`）・元のサイズに戻す（`zoom_actual`）と、
//! 倍率の一覧を `choices` に持つ親（`zoom`）に分ける。`choices` を持つ操作はメニューで子メニューになり、選んだ
//! 値が `OpContext::choice` で渡る。親はメニュー専用、ほかはキーとマウス専用（`in_menu: false`）にして、
//! メニューを spec #33 の並び（`Group` の宣言の順）に保つ。ホイールから呼ぶときは `OpContext::cursor` に
//! カーソルの位置が入り、そこが拡大縮小の基準点になる（キーとメニューからは中心）。
//!
//! トリム（`trim`）は範囲の選択を始めるだけで、選んでいる間（`Mode::Trimming`）の入力は表を通さず
//! `ScrapModel::trim_input` が受ける。その間はどの操作の `can_execute` も false にする。

use std::collections::BTreeMap;

use crate::geometry::Point;
use crate::keys::{Chord, Key};
use crate::scrap::{Mode, ScrapModel, ZoomAnchor};
use crate::settings::{self, Conflict, ShortcutOverrides};

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
    /// 右クリックメニューに出すか。キーやマウスで足りる操作（移動、拡大・縮小）は出さず、メニューを spec の
    /// 並びに保つ。
    pub in_menu: bool,
    /// いま実行できるか。できない操作はメニューでグレー表示にし、ショートカットでも実行しない。
    pub can_execute: fn(&ScrapModel) -> bool,
    /// 引数の選択肢（倍率の一覧など）。Some の操作はショートカットを持たず、メニューでは子メニューになる。
    /// 選ばれた選択肢の `Choice::value` は `OpContext::choice` で `run` に渡る。
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
    /// ホイールを上（奥）へ 1 ノッチ。修飾キーなし。
    WheelUp,
    /// ホイールを下（手前）へ 1 ノッチ。修飾キーなし。
    WheelDown,
    /// 左ボタンのダブルクリック（`WM_LBUTTONDBLCLK`）。
    DoubleClick,
}

/// メニューの区切りのまとまり。宣言の順にメニューに並ぶ（spec #33: コピー / 保存 ─ トリム / 書き込み ─
/// 拡大縮小 / 見た目 / コンパクト表示 ─ 閉じる）。操作のない `Group` は区切りも出ない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// コピーと保存。メニューのいちばん上。
    Output,
    /// トリム・トリムを解除（M2）と書き込み（M4）。
    Edit,
    /// 拡大縮小・見た目・コンパクト表示。
    View,
    /// キーとドラッグの移動（メニューには出さない）。
    Move,
    /// 閉じる。メニューのいちばん下。
    Close,
}

/// 引数のある操作の選択肢 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: ChoiceLabel,
    pub value: i32,
    /// いまの状態がこの選択肢か（メニューでチェックを付ける）。
    pub checked: bool,
}

/// 選択肢の文言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceLabel {
    /// 文言のキー（`locales/ja.toml`）。
    Key(&'static str),
    /// 百分率（`50%` のように出す）。
    Percent(i32),
}

/// 操作を呼び出したときの引数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpContext {
    /// 左ドラッグで前回からカーソルが動いた量（物理ピクセル）。
    pub drag: Point,
    /// 選ばれた選択肢の `Choice::value`。
    pub choice: Option<i32>,
    /// マウスで呼んだときのカーソルの位置（スクリーン座標）。ホイールの拡大縮小の基準点になる。
    /// キーやメニューから呼んだときは None（スクラップの中心が基準）。
    pub cursor: Option<Point>,
}

/// 操作の結果、Win32 層がすること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// `ScrapModel` のとおりに出し直す（位置と中身）。
    Redraw,
    /// スクラップを閉じて、閉じる直前の状態のままごみ箱に移す（上限が 0 なら破棄する）。
    MoveToTrash,
    /// 画像をクリップボードに置く。コピー・保存の画像は、どれも表示する画像（`ScrapModel::shown_image`、
    /// トリムしていればトリム後）。
    CopyToClipboard,
    /// 画像を全体設定の保存先に、全体のファイル名テンプレートの名前で保存する（ダイアログなし）。
    SaveDefault,
    /// 倍率（`ScrapModel::zoom`）を少しの間だけスクラップの上に出す。
    ShowZoom,
    /// 「名前を付けて保存」のダイアログを出し、選ばれた場所に保存する。初期フォルダとファイル名は `SaveDefault` と同じ。
    ShowSaveDialog,
    /// トリムの範囲の選択を始めた（`Mode::Trimming`）。キーとマウスを `ScrapModel::trim_input` に渡し、範囲を描く。
    EnterTrimMode,
}

const ESC: Chord = Chord::plain(Key::known(0x1B));
const LEFT: Chord = Chord::plain(Key::known(0x25));
const UP: Chord = Chord::plain(Key::known(0x26));
const RIGHT: Chord = Chord::plain(Key::known(0x27));
const DOWN: Chord = Chord::plain(Key::known(0x28));
const CTRL_C: Chord = Chord::plain(Key::known(0x43)).ctrl();
const CTRL_S: Chord = Chord::plain(Key::known(0x53)).ctrl();
/// `VK_OEM_PLUS`（US 配列の `=` / `+`、JIS 配列の `;` / `+`）。`+` は Shift と打つ配列があるので両方に付ける。
const CTRL_PLUS: Chord = Chord::plain(Key::known(0xBB)).ctrl();
const CTRL_MINUS: Chord = Chord::plain(Key::known(0xBD)).ctrl();
const CTRL_NUMPAD_ADD: Chord = Chord::plain(Key::known(0x6B)).ctrl();
const CTRL_NUMPAD_SUBTRACT: Chord = Chord::plain(Key::known(0x6D)).ctrl();
const CTRL_0: Chord = Chord::plain(Key::known(0x30)).ctrl();
const CTRL_SHIFT_S: Chord = CTRL_S.shift();
const CTRL_T: Chord = Chord::plain(Key::known(0x54)).ctrl();

/// トリムの範囲を選んでいる間（`Mode::Trimming`）のほかは、いつでも実行できる。
const fn always(model: &ScrapModel) -> bool {
    !matches!(model.mode, Mode::Trimming(_))
}

/// トリムしていて、畳んでいなければ実行できる（トリムの範囲を選んでいる間は除く）。
fn trimmed(model: &ScrapModel) -> bool {
    unfolded(model) && model.is_trimmed()
}

/// コンパクト表示に畳んでいないときだけ実行できる。畳んでいる間に実行できるのは移動・閉じる・コピーと、
/// 広げる操作だけにする（spec #33）。
fn unfolded(model: &ScrapModel) -> bool {
    always(model) && !model.is_compact()
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
    vec![Effect::MoveToTrash]
}

fn copy(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::CopyToClipboard]
}

fn save_default(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::SaveDefault]
}

/// 拡大縮小の基準点。マウスから呼んだらカーソルの位置、キーやメニューからならスクラップの中心。
fn zoom_anchor(context: &OpContext) -> ZoomAnchor {
    context.cursor.map_or(ZoomAnchor::Center, ZoomAnchor::At)
}

/// 倍率を変えたあとの `Effect`。変わらなくても（範囲の端）、いまの倍率は出す。
fn zoomed(changed: bool) -> Vec<Effect> {
    if changed {
        vec![Effect::Redraw, Effect::ShowZoom]
    } else {
        vec![Effect::ShowZoom]
    }
}

fn zoom_in(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    zoomed(model.zoom_in(zoom_anchor(context)))
}

fn zoom_out(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    zoomed(model.zoom_out(zoom_anchor(context)))
}

/// 右クリックメニューの倍率の一覧（%）。
const ZOOM_LIST: [u16; 10] = [10, 25, 50, 75, 100, 150, 200, 300, 400, 800];

fn zoom_choices(model: &ScrapModel) -> Vec<Choice> {
    ZOOM_LIST
        .iter()
        .map(|&percent| Choice {
            label: ChoiceLabel::Percent(i32::from(percent)),
            value: i32::from(percent),
            checked: model.zoom() == percent,
        })
        .collect()
}

/// 倍率の一覧から選んだ倍率にする（中心が基準）。
fn zoom_to(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let Some(choice) = context.choice else {
        return Vec::new();
    };
    let percent = u16::try_from(choice.max(0)).unwrap_or(u16::MAX);
    zoomed(model.set_zoom(percent, zoom_anchor(context)))
}

fn zoom_actual(model: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    if model.reset_zoom() {
        vec![Effect::Redraw]
    } else {
        Vec::new()
    }
}

/// コンパクト表示に畳む、または広げる。畳む基準点は拡大縮小と同じ（マウスならカーソル、ほかは中心）。
fn toggle_compact(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    if model.is_compact() {
        model.unfold();
    } else {
        model.fold(zoom_anchor(context));
    }
    vec![Effect::Redraw]
}

fn save_as(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::ShowSaveDialog]
}

fn trim(model: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    if model.start_trim() {
        vec![Effect::EnterTrimMode]
    } else {
        Vec::new()
    }
}

fn untrim(model: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    if model.untrim() {
        vec![Effect::Redraw]
    } else {
        Vec::new()
    }
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
        in_menu: false,
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
        in_menu: true,
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
        in_menu: true,
        can_execute: unfolded,
        choices: None,
        run: save_default,
    },
    OperationDescriptor {
        id: "save_as",
        label_key: "scrap.op.save_as",
        default_shortcut: &[CTRL_SHIFT_S],
        default_mouse: None,
        group: Group::Output,
        order: 30,
        drag_only: false,
        in_menu: true,
        can_execute: unfolded,
        choices: None,
        run: save_as,
    },
    OperationDescriptor {
        id: "trim",
        label_key: "scrap.op.trim",
        default_shortcut: &[CTRL_T],
        default_mouse: None,
        group: Group::Edit,
        order: 10,
        drag_only: false,
        in_menu: true,
        can_execute: unfolded,
        choices: None,
        run: trim,
    },
    OperationDescriptor {
        id: "untrim",
        label_key: "scrap.op.untrim",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::Edit,
        order: 20,
        drag_only: false,
        in_menu: true,
        can_execute: trimmed,
        choices: None,
        run: untrim,
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
        in_menu: false,
        can_execute: always,
        choices: None,
        run: drag,
    },
    OperationDescriptor {
        id: "zoom",
        label_key: "scrap.op.zoom",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::View,
        order: 10,
        drag_only: false,
        in_menu: true,
        can_execute: unfolded,
        choices: Some(zoom_choices),
        run: zoom_to,
    },
    OperationDescriptor {
        id: "zoom_in",
        label_key: "scrap.op.zoom_in",
        default_shortcut: &[CTRL_PLUS, CTRL_PLUS.shift(), CTRL_NUMPAD_ADD],
        default_mouse: Some(MouseGesture::WheelUp),
        group: Group::View,
        order: 11,
        drag_only: false,
        in_menu: false,
        can_execute: unfolded,
        choices: None,
        run: zoom_in,
    },
    OperationDescriptor {
        id: "zoom_out",
        label_key: "scrap.op.zoom_out",
        default_shortcut: &[CTRL_MINUS, CTRL_NUMPAD_SUBTRACT],
        default_mouse: Some(MouseGesture::WheelDown),
        group: Group::View,
        order: 12,
        drag_only: false,
        in_menu: false,
        can_execute: unfolded,
        choices: None,
        run: zoom_out,
    },
    OperationDescriptor {
        id: "zoom_actual",
        label_key: "scrap.op.zoom_actual",
        default_shortcut: &[CTRL_0],
        default_mouse: None,
        group: Group::View,
        order: 13,
        drag_only: false,
        in_menu: false,
        can_execute: unfolded,
        choices: None,
        run: zoom_actual,
    },
    OperationDescriptor {
        id: "toggle_compact",
        label_key: "scrap.op.toggle_compact",
        default_shortcut: &[],
        default_mouse: Some(MouseGesture::DoubleClick),
        group: Group::View,
        order: 30,
        drag_only: false,
        in_menu: true,
        can_execute: always,
        choices: None,
        run: toggle_compact,
    },
    OperationDescriptor {
        id: "close",
        label_key: "scrap.op.close",
        default_shortcut: &[ESC],
        default_mouse: Some(MouseGesture::MiddleClick),
        group: Group::Close,
        order: 10,
        drag_only: false,
        in_menu: true,
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
///   （設定の通知で項目名を出すため。`settings::keymap` が `SettingsIssue` にする）。
/// - 同じキーの組み合わせが 2 つ以上の操作に付いたら、どれにも付けず `conflicts` に載せる
///   （同じ範囲の重なりの規則 `settings::assign`。ホットキー同士と同じ）。
/// - スクラップにフォーカスがあるときだけ引く（ホットキーと違って OS には登録しない）。
#[derive(Debug, Clone)]
pub struct Shortcuts {
    bindings: BTreeMap<Chord, &'static OperationDescriptor>,
    ignored: Vec<String>,
    conflicts: Vec<Conflict<OperationId>>,
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
        let candidates = ops.iter().filter(|op| op.takes_shortcut()).flat_map(|op| {
            let chords = match overrides.get(op.id) {
                Some(chord) => chord.as_slice(),
                None => op.default_shortcut,
            };
            chords.iter().map(move |&chord| (chord, op))
        });
        let assigned = settings::assign(candidates);
        let conflicts = assigned
            .conflicts
            .into_iter()
            .map(|c| Conflict {
                chord: c.chord,
                owners: c.owners.iter().map(|op| op.id).collect(),
            })
            .collect();
        Shortcuts {
            bindings: assigned.bound,
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

    /// 2 つ以上の操作に付いたため、どれにも付けなかったキーの組み合わせと、その操作。
    pub fn conflicts(&self) -> &[Conflict<OperationId>] {
        &self.conflicts
    }
}

/// マウスの操作に付いた操作（表の既定値。まだ割り当て直せない）。
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
    /// 引数のある操作（`choices` が Some）。選択肢を子メニューに並べる。
    Submenu(Submenu),
    Separator,
}

/// 子メニューに展開する操作。選んだ選択肢の `value` を `OpContext::choice` に入れて実行する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submenu {
    pub id: OperationId,
    pub label_key: &'static str,
    /// false なら親の項目をグレー表示にする。
    pub enabled: bool,
    pub choices: Vec<Choice>,
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
/// `group` の変わり目に区切りを入れる。ドラッグでしか呼べない操作と `in_menu` が false の操作は出さない。
/// 引数のある操作（`choices`）は子メニュー（`MenuEntry::Submenu`）にする。
pub fn menu(
    ops: &'static [OperationDescriptor],
    model: &ScrapModel,
    shortcuts: &Shortcuts,
) -> Vec<MenuEntry> {
    let mut listed: Vec<&OperationDescriptor> = ops
        .iter()
        .filter(|op| op.in_menu && !op.drag_only)
        .collect();
    listed.sort_by_key(|op| (op.group, op.order));
    let mut entries = Vec::new();
    let mut group = None;
    for op in listed {
        if group.is_some_and(|g| g != op.group) {
            entries.push(MenuEntry::Separator);
        }
        group = Some(op.group);
        let enabled = (op.can_execute)(model);
        entries.push(match op.choices {
            Some(choices) => MenuEntry::Submenu(Submenu {
                id: op.id,
                label_key: op.label_key,
                enabled,
                choices: choices(model),
            }),
            None => MenuEntry::Item(MenuItem {
                id: op.id,
                label_key: op.label_key,
                enabled,
                shortcut: shortcuts.chords(op.id).first().copied(),
            }),
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Point, Rect, Size};
    use crate::image::Image;
    use crate::scrap::{Mode, ScrapModel, TrimInput, ZOOM_ACTUAL, ZOOM_MAX, ZOOM_MIN, ZoomAnchor};
    use crate::settings::Conflict;

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
    fn close_moves_the_scrap_to_the_trash_without_changing_it() {
        let mut model = scrap();
        let effects = (op("close").run)(&mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::MoveToTrash]);
        assert_eq!(model, scrap());
    }

    #[test]
    fn copy_and_save_hand_the_image_to_the_win32_layer_without_changing_the_scrap() {
        for (id, effect) in [
            ("copy", Effect::CopyToClipboard),
            ("save_default", Effect::SaveDefault),
            ("save_as", Effect::ShowSaveDialog),
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
            "save_as",
            "close",
        ] {
            assert!((op(id).can_execute)(&scrap()), "{id}");
        }
    }

    #[test]
    fn keys_zoom_in_and_out_around_the_center_and_show_the_zoom() {
        let mut model = ScrapModel::new(Point::new(0, 0), Size::new(400, 200));
        let effects = execute(op("zoom_in"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::Redraw, Effect::ShowZoom]);
        assert_eq!(model.zoom(), 125);
        assert_eq!(model.image_rect(), Rect::new(-50, -25, 450, 225));
        let effects = execute(op("zoom_out"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::Redraw, Effect::ShowZoom]);
        assert_eq!(model.image_rect(), Rect::new(0, 0, 400, 200));
    }

    #[test]
    fn the_wheel_zooms_around_the_cursor() {
        let mut model = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        let at_cursor = OpContext {
            cursor: Some(Point::new(40, 20)),
            ..OpContext::default()
        };
        let id = |g| mouse_operation(OPERATIONS, g).map(|op| op.id);
        assert_eq!(id(MouseGesture::WheelUp), Some("zoom_in"));
        assert_eq!(id(MouseGesture::WheelDown), Some("zoom_out"));
        execute(op("zoom_in"), &mut model, &at_cursor);
        assert_eq!(model.image_rect(), Rect::new(-10, -5, 365, 245));
    }

    #[test]
    fn zooming_past_the_limit_only_shows_the_zoom() {
        let mut model = scrap();
        model.set_zoom(ZOOM_MAX, ZoomAnchor::Center);
        let before = model.clone();
        let effects = execute(op("zoom_in"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::ShowZoom]);
        assert_eq!(model, before);
        model.set_zoom(ZOOM_MIN, ZoomAnchor::Center);
        let effects = execute(op("zoom_out"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::ShowZoom]);
    }

    #[test]
    fn zoom_actual_returns_to_actual_pixels_around_the_center() {
        let mut model = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        model.set_zoom(200, ZoomAnchor::At(Point::new(0, 0)));
        let effects = execute(op("zoom_actual"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::Redraw]);
        assert_eq!(model.zoom(), ZOOM_ACTUAL);
        assert_eq!(model.image_rect(), Rect::new(150, 100, 450, 300));
        // 等倍のままなら何もしない
        let effects = execute(op("zoom_actual"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![]);
    }

    /// `scrap()` を (10, 20)〜(59, 59) の画素だけにトリムする。
    fn trim(model: &mut ScrapModel) {
        model.trim_input(TrimInput::Press(Point::new(-90, 70)));
        model.trim_input(TrimInput::Move(Point::new(-41, 109)));
        model.trim_input(TrimInput::Release);
        model.trim_input(TrimInput::Confirm);
    }

    #[test]
    fn trim_enters_the_trim_mode() {
        let mut model = scrap();
        let effects = execute(op("trim"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::EnterTrimMode]);
        assert!(matches!(model.mode, Mode::Trimming(_)));
        trim(&mut model);
        assert_eq!(model.mode, Mode::Normal);
        assert!(model.is_trimmed());
    }

    #[test]
    fn no_operation_can_run_while_trimming() {
        let mut model = scrap();
        execute(op("trim"), &mut model, &OpContext::default());
        for op in OPERATIONS {
            assert!(!(op.can_execute)(&model), "{}", op.id);
        }
        let before = model.clone();
        assert_eq!(
            execute(
                find("move_left").unwrap(),
                &mut model,
                &OpContext::default()
            ),
            vec![]
        );
        assert_eq!(model, before);
    }

    #[test]
    fn untrim_can_run_only_on_a_trimmed_scrap_and_returns_the_whole_image() {
        let mut model = scrap();
        assert!(!(op("untrim").can_execute)(&model));
        execute(op("trim"), &mut model, &OpContext::default());
        trim(&mut model);
        assert!((op("untrim").can_execute)(&model));
        let effects = execute(op("untrim"), &mut model, &OpContext::default());
        assert_eq!(effects, vec![Effect::Redraw]);
        assert_eq!(model, scrap());
        assert!(!(op("untrim").can_execute)(&model));
    }

    #[test]
    fn copy_and_save_hand_over_the_trimmed_image() {
        let size = Size::new(300, 200);
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend([x as u8, y as u8, 0, 0xFF]);
            }
        }
        let source = Image::from_bgra(size, pixels).unwrap();
        let mut model = scrap();
        execute(op("trim"), &mut model, &OpContext::default());
        trim(&mut model);
        for (id, effect) in [
            ("copy", Effect::CopyToClipboard),
            ("save_default", Effect::SaveDefault),
            ("save_as", Effect::ShowSaveDialog),
        ] {
            assert_eq!(
                execute(op(id), &mut model, &OpContext::default()),
                vec![effect],
                "{id}"
            );
        }
        // Win32 層が渡す画像（`Effect` の説明のとおり、表示する画像）
        let handed = model.shown_image(&source);
        assert_eq!(handed, source.crop(Rect::new(10, 20, 60, 60)));
    }

    #[test]
    fn a_double_click_folds_around_the_cursor_and_toggles_back() {
        let id = |g| mouse_operation(OPERATIONS, g).map(|op| op.id);
        assert_eq!(id(MouseGesture::DoubleClick), Some("toggle_compact"));
        let mut model = scrap();
        let at_cursor = OpContext {
            cursor: Some(Point::new(-90, 60)),
            ..OpContext::default()
        };
        let effects = execute(op("toggle_compact"), &mut model, &at_cursor);
        assert_eq!(effects, vec![Effect::Redraw]);
        assert!(model.is_compact());
        assert_eq!(model.image_rect(), Rect::new(-122, 28, -58, 92));
        let effects = execute(op("toggle_compact"), &mut model, &at_cursor);
        assert_eq!(effects, vec![Effect::Redraw]);
        assert_eq!(model, scrap());
    }

    #[test]
    fn the_menu_folds_around_the_center() {
        let mut model = scrap();
        execute(op("toggle_compact"), &mut model, &OpContext::default());
        assert_eq!(model.image_rect(), Rect::new(18, 118, 82, 182));
    }

    #[test]
    fn only_moving_closing_copying_and_unfolding_run_while_compact() {
        let mut model = scrap();
        model.fold(ZoomAnchor::Center);
        let runnable: Vec<OperationId> = OPERATIONS
            .iter()
            .filter(|op| (op.can_execute)(&model))
            .map(|op| op.id)
            .collect();
        assert_eq!(
            runnable,
            [
                "copy",
                "move_left",
                "move_right",
                "move_up",
                "move_down",
                "move_left_10",
                "move_right_10",
                "move_up_10",
                "move_down_10",
                "move",
                "toggle_compact",
                "close",
            ]
        );
        // 実行できない操作は呼んでも何もしない
        let before = model.clone();
        assert_eq!(
            execute(op("zoom_in"), &mut model, &OpContext::default()),
            vec![]
        );
        assert_eq!(model, before);
    }

    #[test]
    fn the_menu_greys_out_what_cannot_run_while_compact() {
        let shortcuts = Shortcuts::new(OPERATIONS, &ShortcutOverrides::new());
        let mut model = scrap();
        model.fold(ZoomAnchor::Center);
        let enabled: Vec<(OperationId, bool)> = menu(OPERATIONS, &model, &shortcuts)
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some((item.id, item.enabled)),
                MenuEntry::Submenu(submenu) => Some((submenu.id, submenu.enabled)),
                MenuEntry::Separator => None,
            })
            .collect();
        assert_eq!(
            enabled,
            [
                ("copy", true),
                ("save_default", false),
                ("save_as", false),
                ("trim", false),
                ("untrim", false),
                ("zoom", false),
                ("toggle_compact", true),
                ("close", true),
            ]
        );
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
        assert_eq!(bound(&shortcuts, "Ctrl+Plus"), Some("zoom_in"));
        assert_eq!(bound(&shortcuts, "Ctrl+Shift+Plus"), Some("zoom_in"));
        assert_eq!(bound(&shortcuts, "Ctrl+NumpadAdd"), Some("zoom_in"));
        assert_eq!(bound(&shortcuts, "Ctrl+Minus"), Some("zoom_out"));
        assert_eq!(bound(&shortcuts, "Ctrl+NumpadSubtract"), Some("zoom_out"));
        assert_eq!(bound(&shortcuts, "Ctrl+0"), Some("zoom_actual"));
        assert_eq!(bound(&shortcuts, "Ctrl+Shift+S"), Some("save_as"));
        assert_eq!(bound(&shortcuts, "Ctrl+T"), Some("trim"));
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
                label: ChoiceLabel::Percent(50),
                value: 50,
                checked: false,
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

    fn zoom_choice(percent: i32, checked: bool) -> Choice {
        Choice {
            label: ChoiceLabel::Percent(percent),
            value: percent,
            checked,
        }
    }

    #[test]
    fn the_menu_follows_the_spec_order_with_the_zoom_list_as_a_submenu() {
        let shortcuts = Shortcuts::new(OPERATIONS, &ShortcutOverrides::new());
        let zoom_list = [10, 25, 50, 75, 100, 150, 200, 300, 400, 800]
            .map(|p| zoom_choice(p, p == 100))
            .to_vec();
        assert_eq!(
            menu(OPERATIONS, &scrap(), &shortcuts),
            vec![
                item("copy", true, Some("Ctrl+C")),
                item("save_default", true, Some("Ctrl+S")),
                item("save_as", true, Some("Ctrl+Shift+S")),
                MenuEntry::Separator,
                item("trim", true, Some("Ctrl+T")),
                item("untrim", false, None),
                MenuEntry::Separator,
                MenuEntry::Submenu(Submenu {
                    id: "zoom",
                    label_key: "scrap.op.zoom",
                    enabled: true,
                    choices: zoom_list,
                }),
                item("toggle_compact", true, None),
                MenuEntry::Separator,
                item("close", true, Some("Esc")),
            ]
        );
    }

    #[test]
    fn the_zoom_list_checks_the_current_zoom_and_nothing_when_it_is_not_listed() {
        let choices = op("zoom").choices.expect("zoom has choices");
        let mut model = scrap();
        model.set_zoom(200, ZoomAnchor::Center);
        let checked: Vec<i32> = choices(&model)
            .iter()
            .filter(|c| c.checked)
            .map(|c| c.value)
            .collect();
        assert_eq!(checked, [200]);
        model.set_zoom(125, ZoomAnchor::Center);
        assert!(choices(&model).iter().all(|c| !c.checked));
    }

    #[test]
    fn choosing_from_the_zoom_list_sets_that_zoom_around_the_center() {
        let mut model = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        let chosen = OpContext {
            choice: Some(200),
            ..OpContext::default()
        };
        let effects = execute(op("zoom"), &mut model, &chosen);
        assert_eq!(effects, vec![Effect::Redraw, Effect::ShowZoom]);
        assert_eq!(model.zoom(), 200);
        assert_eq!(model.image_rect(), Rect::new(-150, -100, 450, 300));
        // 選択肢なしで呼ばれたら何もしない
        assert_eq!(
            execute(op("zoom"), &mut model, &OpContext::default()),
            vec![]
        );
        assert_eq!(model.zoom(), 200);
    }

    #[test]
    fn the_zoom_list_takes_no_shortcut() {
        let overrides = ShortcutOverrides::from([("zoom".to_owned(), Some(chord("Z")))]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        assert_eq!(bound(&shortcuts, "Z"), None);
        assert_eq!(shortcuts.chords("zoom"), vec![]);
        assert_eq!(shortcuts.ignored_overrides(), ["zoom"]);
    }

    #[test]
    fn an_operation_with_choices_becomes_a_submenu_with_its_checks() {
        fn never(_: &ScrapModel) -> bool {
            false
        }
        fn levels(_: &ScrapModel) -> Vec<Choice> {
            vec![
                Choice {
                    label: ChoiceLabel::Key("low"),
                    value: 1,
                    checked: false,
                },
                Choice {
                    label: ChoiceLabel::Key("high"),
                    value: 2,
                    checked: true,
                },
            ]
        }
        static TABLE: &[OperationDescriptor] = &[
            OperationDescriptor {
                choices: Some(levels),
                ..test_op("level", Group::View, 10, &[])
            },
            OperationDescriptor {
                choices: Some(levels),
                can_execute: never,
                ..test_op("blocked", Group::View, 20, &[])
            },
            OperationDescriptor {
                in_menu: false,
                ..test_op("hidden", Group::View, 30, &[])
            },
            test_op("plain", Group::Close, 10, &[]),
        ];
        let shortcuts = Shortcuts::new(TABLE, &ShortcutOverrides::new());
        let submenu = |id, enabled| {
            MenuEntry::Submenu(Submenu {
                id,
                label_key: id,
                enabled,
                choices: levels(&scrap()),
            })
        };
        assert_eq!(
            menu(TABLE, &scrap(), &shortcuts),
            vec![
                submenu("level", true),
                submenu("blocked", false),
                MenuEntry::Separator,
                item("plain", true, None),
            ]
        );
    }

    #[test]
    fn the_menu_shows_overridden_shortcuts() {
        let overrides = ShortcutOverrides::from([
            ("close".to_owned(), Some(chord("Ctrl+W"))),
            ("copy".to_owned(), None),
        ]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        let entries = menu(OPERATIONS, &scrap(), &shortcuts);
        assert_eq!(entries.first(), Some(&item("copy", true, None)));
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
        assert_eq!(
            shortcuts.conflicts(),
            [Conflict {
                chord: chord("Left"),
                owners: vec!["move_left", "close"],
            }]
        );
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
            in_menu: true,
            can_execute: always,
            choices: None,
            run: nothing,
        }
    }
}
