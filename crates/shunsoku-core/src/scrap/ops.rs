//! スクラップ操作の表（ADR-0005）。
//!
//! スクラップ操作は静的な表 `OPERATIONS` に 1 行足せば、右クリックメニュー（`menu`）とショートカットの
//! 対応表（`Shortcuts`）とマウスの対応表（`MouseBindings`）の全部に出る。`run` は `ScrapModel` を変え、
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
//!
//! 書き込み（`annotate`）は書き込みの編集モード（`Mode::Editing`）に入る。もう一度選ぶと抜ける。編集モードの
//! 間は「書き込み」のほかの操作の `can_execute` を false にし、メニューでは「書き込み」にチェックを付ける
//! （`OperationDescriptor::checked`）。編集モードの間の入力は表を通さず `ScrapModel::edit_input` が受ける。
//!
//! 見た目（透明度・枠・影）は、一覧を `choices` に持つ `opacity` / `border` / `shadow` と、`Ctrl+ホイール` の
//! `opacity_up` / `opacity_down` に分ける。どれもスクラップごとの上書き（`ScrapModel::appearance_overrides`）を
//! 変える。いまの見た目は全体の既定値に上書きを重ねて決まるので、既定値を `OpContext::appearance` と `menu` の
//! 引数で受け取る。メニューでは 3 つの一覧を「見た目」の子メニュー（`FOLDERS`、`MenuEntry::Folder`）にまとめる。
//! 上書きを全部消す `reset_appearance`（見た目を既定に戻す）は値のない操作で、同じ子メニューの区切りの下に置く。
//!
//! 見た目を指定（`appearance`）は、設定ファイルのレシピに値のある手（`{ op = "appearance", border_color = ... }`）
//! として書いたときだけ動く（ADR-0008）。書いた値は `OpContext::values` で渡り、書いた項目だけを上書きする。
//! `takes_values` の操作はショートカット・マウス・「スクラップ操作」タブの行のどれにも出さない。
//!
//! 背景色（`background`）は白紙スクラップだけの操作で、普通のスクラップのメニューには出さない（`BLANK_ONLY`）。
//! メニューではツールバーと同じ 8 色と「その他の色…」の子メニューにし、レシピでは値のある手
//! （`{ op = "background", color = "#RRGGBB" }`）で呼ぶ。普通のスクラップでは `can_execute` が false なので、
//! レシピはそこで止まる（ADR-0007）。背景色は見た目ではなく中身なので、`reset_appearance` では戻らない。

use std::collections::BTreeMap;

use crate::annotate::toolbar::PALETTE;
use crate::annotate::{Carryover, EditInput, Font, Style};
use crate::geometry::Point;
use crate::keys::{Chord, Key, Modifiers};
use crate::recipe::{Plan, StepValues};
use crate::scrap::{
    Appearance, AppearanceOverrides, Background, BorderOverride, Mode, ScrapModel, ZoomAnchor,
};
use crate::settings::{self, Conflict, MouseOverrides, OPACITY_RANGE, Rgb, ShortcutOverrides};

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
    /// レシピの値のある手の値（`OpContext::values`）を引数に取る操作（`appearance`、`background`。ADR-0008）。
    /// ショートカットとマウスの対応表にも、値のない手にも出さない。
    pub takes_values: bool,
    /// レシピのどこに置けるか（ADR-0007）。引数のある操作とドラッグ専用の操作は `Never` にする。
    pub in_recipe: InRecipe,
    /// 右クリックメニューに出すか。キーやマウスで足りる操作（移動、拡大・縮小）は出さず、メニューを spec の
    /// 並びに保つ。
    pub in_menu: bool,
    /// いま実行できるか。できない操作はメニューでグレー表示にし、ショートカットでも実行しない。
    pub can_execute: fn(&ScrapModel) -> bool,
    /// メニューの項目にチェックを付けるか（いまその状態にいる。書き込みの編集モードなど）。None なら付けない。
    pub checked: Option<fn(&ScrapModel) -> bool>,
    /// 引数の選択肢（倍率の一覧など）。Some の操作はショートカットを持たず、メニューでは子メニューになる。
    /// 選ばれた選択肢の `Choice::value` は `OpContext::choice` で `run` に渡る。
    /// 引数は全体の見た目の既定値（チェックをいまの見た目に付けるため）。
    pub choices: Option<fn(&ScrapModel, &Appearance) -> Vec<Choice>>,
    pub run: fn(&mut ScrapModel, &OpContext) -> Vec<Effect>,
}

impl OperationDescriptor {
    /// ショートカットとマウスの割り当てを設定で変えられるか。引数のない操作だけ。
    pub(crate) fn takes_binding(&self) -> bool {
        self.choices.is_none() && !self.drag_only && !self.takes_values
    }
}

/// スクラップ操作をレシピのどこに置けるか（`OperationDescriptor::in_recipe`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InRecipe {
    /// どこにでも置ける。
    Anywhere,
    /// 最後にだけ置ける（`close`。閉じたあとの手は実行できない）。
    Last,
    /// 置けない。引数を取る操作と、モードを切り替えて入力を奪う操作（`trim`、`annotate`）。
    Never,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MouseGesture {
    LeftDrag,
    MiddleClick,
    /// ホイールを上（奥）へ 1 ノッチ。修飾キーなし。
    WheelUp,
    /// ホイールを下（手前）へ 1 ノッチ。修飾キーなし。
    WheelDown,
    /// Ctrl を押しながらホイールを上（奥）へ 1 ノッチ。
    CtrlWheelUp,
    /// Ctrl を押しながらホイールを下（手前）へ 1 ノッチ。
    CtrlWheelDown,
    /// 左ボタンのダブルクリック（`WM_LBUTTONDBLCLK`）。
    DoubleClick,
}

impl MouseGesture {
    /// ホイールの回転（`rotation`、上が正）と、そのとき押している修飾キーから決まるホイールの操作。修飾キーなしと
    /// Ctrl だけなら上下の向きで決め、ほかの修飾キーつきと回っていない（0）なら None（対応がない）。
    pub fn wheel(modifiers: Modifiers, rotation: i32) -> Option<MouseGesture> {
        let Modifiers {
            win: false,
            ctrl,
            alt: false,
            shift: false,
        } = modifiers
        else {
            return None;
        };
        let up = match rotation.signum() {
            1 => true,
            -1 => false,
            _ => return None,
        };
        Some(match (ctrl, up) {
            (false, true) => MouseGesture::WheelUp,
            (false, false) => MouseGesture::WheelDown,
            (true, true) => MouseGesture::CtrlWheelUp,
            (true, false) => MouseGesture::CtrlWheelDown,
        })
    }

    /// 設定 GUI に出す文言のキー（`mouse_gesture.*`）。
    pub fn label_key(self) -> &'static str {
        match self {
            MouseGesture::LeftDrag => "mouse_gesture.left_drag",
            MouseGesture::MiddleClick => "mouse_gesture.middle_click",
            MouseGesture::WheelUp => "mouse_gesture.wheel_up",
            MouseGesture::WheelDown => "mouse_gesture.wheel_down",
            MouseGesture::CtrlWheelUp => "mouse_gesture.ctrl_wheel_up",
            MouseGesture::CtrlWheelDown => "mouse_gesture.ctrl_wheel_down",
            MouseGesture::DoubleClick => "mouse_gesture.double_click",
        }
    }

    /// 設定ファイル（`[mouse]`）での名前。
    const NAMES: [(MouseGesture, &'static str); 7] = [
        (MouseGesture::LeftDrag, "LeftDrag"),
        (MouseGesture::MiddleClick, "MiddleClick"),
        (MouseGesture::WheelUp, "WheelUp"),
        (MouseGesture::WheelDown, "WheelDown"),
        (MouseGesture::CtrlWheelUp, "Ctrl+WheelUp"),
        (MouseGesture::CtrlWheelDown, "Ctrl+WheelDown"),
        (MouseGesture::DoubleClick, "DoubleClick"),
    ];
}

impl std::fmt::Display for MouseGesture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = MouseGesture::NAMES
            .iter()
            .find(|(g, _)| g == self)
            .map_or("?", |(_, name)| name);
        f.write_str(name)
    }
}

/// 名前の表にないマウスの操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownGesture;

impl std::str::FromStr for MouseGesture {
    type Err = UnknownGesture;

    /// 大文字と小文字は区別しない。`Ctrl+` の前後の空白は無視する（`Ctrl + WheelUp`）。
    fn from_str(s: &str) -> Result<MouseGesture, UnknownGesture> {
        let normalized: String = s.split('+').map(str::trim).collect::<Vec<_>>().join("+");
        MouseGesture::NAMES
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(&normalized))
            .map(|&(g, _)| g)
            .ok_or(UnknownGesture)
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpContext {
    /// 全体の見た目の既定値（設定ファイルの `[scrap]`）。見た目を変える操作が、いまの見た目を知るのに使う。
    pub appearance: Appearance,
    /// 左ドラッグで前回からカーソルが動いた量（物理ピクセル）。
    pub drag: Point,
    /// 選ばれた選択肢の `Choice::value`。
    pub choice: Option<i32>,
    /// マウスで呼んだときのカーソルの位置（スクリーン座標）。ホイールの拡大縮小の基準点になる。
    /// キーやメニューから呼んだときは None（スクラップの中心が基準）。
    pub cursor: Option<Point>,
    /// 書き込みの色と太さの既定値（設定ファイルの `[annotation]`）。書き込みの編集モードに入るときに使う。
    pub annotation: Style,
    /// 前の編集モードで選んだ道具・色・太さ（アプリが動いている間だけ、すべてのスクラップで引き継ぐ）。
    pub carryover: Carryover,
    /// 文字の書き込みの書体と大きさの既定値（設定ファイルの `[annotation]`）。
    pub font: Font,
    /// レシピの値のある手に書いた値（`takes_values` の操作に渡す）。ほかから呼んだときは None。
    pub values: Option<StepValues>,
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
    /// スクラップファイル（`crate::scrap_file`）の保存先のダイアログを出し、選ばれた場所に書く。ほかの保存と違い、
    /// 表示する画像ではなく元の画像・トリム・書き込みをそのまま書く（`ScrapFile::of`）。
    ShowScrapFileSaveDialog,
    /// トリムの範囲の選択を始めた（`Mode::Trimming`）。キーとマウスを `ScrapModel::trim_input` に渡し、範囲を描く。
    EnterTrimMode,
    /// 書き込みの編集モードに入った（`Mode::Editing`）。キーとマウスを `ScrapModel::edit_input` に渡す。
    EnterEditMode,
    /// 書き込みの編集モードを抜けた。
    LeaveEditMode,
    /// 背景色の「その他の色…」: 色の選択ダイアログを出し、選ばれた色を `background` の選択肢として実行する。
    ShowBackgroundDialog,
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
const E: Chord = Chord::plain(Key::known(0x45));

/// ふつうのモード（`Mode::Normal`）なら、いつでも実行できる。トリムの範囲を選んでいる間と書き込みの
/// 編集モードの間は実行できない。
const fn normal(model: &ScrapModel) -> bool {
    matches!(model.mode, Mode::Normal)
}

/// トリムしていて、畳んでいなければ実行できる（トリムの範囲を選んでいる間は除く）。
fn trimmed(model: &ScrapModel) -> bool {
    not_compact(model) && model.is_trimmed()
}

/// コンパクト表示に畳んでいないときだけ実行できる。畳んでいる間に実行できるのは移動・閉じる・コピーと、
/// 広げる操作だけにする（spec #33）。
fn not_compact(model: &ScrapModel) -> bool {
    normal(model) && !model.is_compact()
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

fn zoom_choices(model: &ScrapModel, _: &Appearance) -> Vec<Choice> {
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

/// 見た目を変えたあとの `Effect`。
fn redrawn(changed: bool) -> Vec<Effect> {
    if changed {
        vec![Effect::Redraw]
    } else {
        Vec::new()
    }
}

/// 透明度の刻み（%）。`Ctrl+ホイール` の 1 ノッチと、一覧の間隔。
const OPACITY_STEP: u8 = 10;

/// 透明度の上書きを `opacity`（範囲に収める）にする。いまの透明度と同じなら false。
fn set_opacity(model: &mut ScrapModel, defaults: &Appearance, opacity: u8) -> bool {
    let opacity = opacity.clamp(*OPACITY_RANGE.start(), *OPACITY_RANGE.end());
    if model.appearance(defaults).opacity == opacity {
        return false;
    }
    model.appearance_overrides.opacity = Some(opacity);
    true
}

/// 1 段濃くする（10% の倍数のうち、いまより大きい最初のもの）。
fn opacity_up(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let now = model.appearance(&context.appearance).opacity;
    let next = (now / OPACITY_STEP + 1).saturating_mul(OPACITY_STEP);
    redrawn(set_opacity(model, &context.appearance, next))
}

/// 1 段薄くする（10% の倍数のうち、いまより小さい最初のもの）。
fn opacity_down(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let now = model.appearance(&context.appearance).opacity;
    let next = (now.saturating_sub(1) / OPACITY_STEP) * OPACITY_STEP;
    redrawn(set_opacity(model, &context.appearance, next))
}

fn opacity_choices(model: &ScrapModel, defaults: &Appearance) -> Vec<Choice> {
    let now = model.appearance(defaults).opacity;
    (1..=10)
        .rev()
        .map(|n| n * OPACITY_STEP)
        .map(|percent| Choice {
            label: ChoiceLabel::Percent(i32::from(percent)),
            value: i32::from(percent),
            checked: now == percent,
        })
        .collect()
}

/// 透明度の一覧から選んだ透明度にする。
fn opacity_to(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let Some(choice) = context.choice else {
        return Vec::new();
    };
    let percent = u8::try_from(choice.max(0)).unwrap_or(u8::MAX);
    redrawn(set_opacity(model, &context.appearance, percent))
}

/// 枠の一覧の「枠なし」の値。色の値（`0xRRGGBB`）は 0 以上なので重ならない。
const NO_BORDER: i32 = -1;

/// 枠の一覧で設定の色に続けて並べる色（文言のキー、`0xRRGGBB`）。設定の色と同じものは並べない。
const BORDER_COLORS: [(&str, u32); 5] = [
    ("scrap.border.gray", 0x80_80_80),
    ("scrap.border.black", 0x00_00_00),
    ("scrap.border.white", 0xFF_FF_FF),
    ("scrap.border.red", 0xFF_00_00),
    ("scrap.border.blue", 0x00_00_FF),
];

/// 枠の一覧: 「なし」、設定の色（設定で枠を出さなくても）、`BORDER_COLORS` の順。
fn border_choices(model: &ScrapModel, defaults: &Appearance) -> Vec<Choice> {
    let now = model.appearance(defaults).shown_border().map(|b| b.color);
    let configured = defaults.border.color;
    let none = Choice {
        label: ChoiceLabel::Key("scrap.border.none"),
        value: NO_BORDER,
        checked: now.is_none(),
    };
    let others = BORDER_COLORS
        .iter()
        .copied()
        .filter(|&(_, color)| color != configured);
    let colors = std::iter::once(("scrap.border.default", configured))
        .chain(others)
        .map(|(key, color)| Choice {
            label: ChoiceLabel::Key(key),
            value: color as i32,
            checked: now == Some(color),
        });
    std::iter::once(none).chain(colors).collect()
}

/// 枠の一覧から選んだ枠にする（`NO_BORDER` なら枠なし、ほかはその色の枠）。
fn border_to(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let Some(choice) = context.choice else {
        return Vec::new();
    };
    let color = u32::try_from(choice).ok().map(|c| c & 0xFF_FF_FF);
    if model
        .appearance(&context.appearance)
        .shown_border()
        .map(|b| b.color)
        == color
    {
        return Vec::new();
    }
    let border = color.map_or(BorderOverride::Hidden, BorderOverride::Color);
    model.appearance_overrides.border = Some(border);
    vec![Effect::Redraw]
}

fn shadow_choices(model: &ScrapModel, defaults: &Appearance) -> Vec<Choice> {
    let now = model.appearance(defaults).shadow;
    [("scrap.shadow.on", true), ("scrap.shadow.off", false)]
        .map(|(key, shadow)| Choice {
            label: ChoiceLabel::Key(key),
            value: i32::from(shadow),
            checked: now == shadow,
        })
        .to_vec()
}

/// 影の一覧から選んだ有無にする（1 であり、0 でなし）。
fn shadow_to(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let Some(choice) = context.choice else {
        return Vec::new();
    };
    let shadow = choice != 0;
    if model.appearance(&context.appearance).shadow == shadow {
        return Vec::new();
    }
    model.appearance_overrides.shadow = Some(shadow);
    vec![Effect::Redraw]
}

/// レシピの値のある手に書いた項目だけを、スクラップごとの上書きに重ねる（ADR-0008）。書かなかった項目は今の
/// 値のまま。枠の色か太さを書けば枠も出す。見た目が変わらなければ何もしない（手は成功として次へ進む）。
fn set_appearance(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let Some(StepValues::Appearance(values)) = context.values else {
        return Vec::new();
    };
    let before = model.appearance(&context.appearance);
    let overrides = &mut model.appearance_overrides;
    if let Some(opacity) = values.opacity {
        overrides.opacity = Some(opacity);
    }
    if let Some(shadow) = values.shadow {
        overrides.shadow = Some(shadow);
    }
    if let Some(width) = values.border_width {
        overrides.border_width = Some(width);
    }
    // 枠を出す（色か太さを書いたときも）なら、全体の既定値で枠を出していても、枠を出す上書きを入れる（あとで
    // 既定値が枠を出さなくなっても、この手で決めた枠は残す）。色を書かなければ今の色のまま
    let shows = values.border == Some(true)
        || values.border_color.is_some()
        || values.border_width.is_some();
    if values.border == Some(false) {
        overrides.border = Some(BorderOverride::Hidden);
    } else if shows {
        let color = values
            .border_color
            .map_or(before.border.color, settings::Rgb::to_rrggbb);
        overrides.border = Some(BorderOverride::Color(color));
    }
    redrawn(model.appearance(&context.appearance) != before)
}

/// 見た目のスクラップごとの上書きを全部消し、全体の既定値の見た目に戻す。見た目が変わったときだけ描き直す。
fn reset_appearance(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let before = model.appearance(&context.appearance);
    model.appearance_overrides = AppearanceOverrides::default();
    redrawn(model.appearance(&context.appearance) != before)
}

/// 背景色の一覧の文言のキー（`PALETTE` と同じ順）。
const BACKGROUND_LABELS: [&str; 8] = [
    "scrap.background.red",
    "scrap.background.orange",
    "scrap.background.yellow",
    "scrap.background.green",
    "scrap.background.blue",
    "scrap.background.purple",
    "scrap.background.black",
    "scrap.background.white",
];

/// 背景色の一覧の「その他の色…」の値。色の値（`0xRRGGBB`）は 0 以上なので重ならない。
pub const OTHER_BACKGROUND: i32 = -1;

/// 白紙スクラップで、畳んでいなければ実行できる。
fn blank_and_not_compact(model: &ScrapModel) -> bool {
    not_compact(model) && model.background().is_some()
}

/// 背景色の一覧: ツールバーと同じ 8 色（`PALETTE`）と「その他の色…」。並びにない色なら「その他の色…」に
/// チェックを付ける。
fn background_choices(model: &ScrapModel, _: &Appearance) -> Vec<Choice> {
    let now = model.background();
    let listed = PALETTE.map(|c| Some(Background::Color(c)));
    let colours = PALETTE
        .iter()
        .zip(BACKGROUND_LABELS)
        .map(|(&colour, key)| Choice {
            label: ChoiceLabel::Key(key),
            value: colour.to_rrggbb() as i32,
            checked: now == Some(Background::Color(colour)),
        });
    let other = Choice {
        label: ChoiceLabel::Key("scrap.background.other"),
        value: OTHER_BACKGROUND,
        checked: now.is_some() && !listed.contains(&now),
    };
    colours.chain(std::iter::once(other)).collect()
}

/// 背景色を、レシピの手の値か、一覧から選んだ色にする。「その他の色…」なら色の選択ダイアログを出す。
/// 背景色が変わったときだけ描き直す（変わらなければ、手は成功として次へ進む）。
fn background_to(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    let background = match (context.values, context.choice) {
        (Some(StepValues::Background(background)), _) => background,
        (_, Some(OTHER_BACKGROUND)) => return vec![Effect::ShowBackgroundDialog],
        (_, Some(choice)) => match u32::try_from(choice) {
            Ok(rrggbb) => Background::Color(Rgb::from_rrggbb(rrggbb)),
            Err(_) => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    redrawn(model.set_background(background))
}

/// コンパクト表示に畳む、または広げる。畳む基準点は拡大縮小と同じ（マウスならカーソル、ほかは中心）。
fn toggle_compact(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    if model.is_compact() {
        model.leave_compact();
    } else {
        model.enter_compact(zoom_anchor(context));
    }
    vec![Effect::Redraw]
}

fn save_as(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::ShowSaveDialog]
}

fn save_scrap_file(_: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    vec![Effect::ShowScrapFileSaveDialog]
}

fn trim(model: &mut ScrapModel, _: &OpContext) -> Vec<Effect> {
    if model.start_trim() {
        vec![Effect::EnterTrimMode]
    } else {
        Vec::new()
    }
}

/// 書き込みの編集モードの間と、ふつうのモードで畳んでいないときに実行できる（入るか抜けるか）。
fn can_annotate(model: &ScrapModel) -> bool {
    model.is_editing() || not_compact(model)
}

fn editing(model: &ScrapModel) -> bool {
    model.is_editing()
}

/// 書き込みの編集モードに入る。入っていれば抜ける（描きかけは書き込みにする）。
fn annotate(model: &mut ScrapModel, context: &OpContext) -> Vec<Effect> {
    if model.is_editing() {
        model.edit_input(EditInput::Leave);
        vec![Effect::LeaveEditMode]
    } else if model.start_editing_with(
        context
            .carryover
            .start_with(context.annotation, context.font.clone()),
    ) {
        vec![Effect::EnterEditMode]
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: false,
        can_execute: normal,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: normal,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
        choices: None,
        run: save_as,
    },
    OperationDescriptor {
        id: "save_scrap_file",
        label_key: "scrap.op.save_scrap_file",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::Output,
        order: 40,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
        choices: None,
        run: save_scrap_file,
    },
    OperationDescriptor {
        id: "trim",
        label_key: "scrap.op.trim",
        default_shortcut: &[CTRL_T],
        default_mouse: None,
        group: Group::Edit,
        order: 10,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Never,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: trimmed,
        checked: None,
        choices: None,
        run: untrim,
    },
    OperationDescriptor {
        id: "annotate",
        label_key: "scrap.op.annotate",
        default_shortcut: &[E],
        default_mouse: None,
        group: Group::Edit,
        order: 30,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Never,
        in_menu: true,
        can_execute: can_annotate,
        checked: Some(editing),
        choices: None,
        run: annotate,
    },
    OperationDescriptor {
        id: "background",
        label_key: "scrap.op.background",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::Edit,
        order: 40,
        drag_only: false,
        takes_values: true,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: blank_and_not_compact,
        checked: None,
        choices: Some(background_choices),
        run: background_to,
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
        takes_values: false,
        in_recipe: InRecipe::Never,
        in_menu: false,
        can_execute: normal,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Never,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: false,
        can_execute: not_compact,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: false,
        can_execute: not_compact,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: false,
        can_execute: not_compact,
        checked: None,
        choices: None,
        run: zoom_actual,
    },
    OperationDescriptor {
        id: "opacity",
        label_key: "scrap.op.opacity",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::View,
        order: 20,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Never,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
        choices: Some(opacity_choices),
        run: opacity_to,
    },
    OperationDescriptor {
        id: "opacity_up",
        label_key: "scrap.op.opacity_up",
        default_shortcut: &[],
        default_mouse: Some(MouseGesture::CtrlWheelUp),
        group: Group::View,
        order: 21,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: false,
        can_execute: not_compact,
        checked: None,
        choices: None,
        run: opacity_up,
    },
    OperationDescriptor {
        id: "opacity_down",
        label_key: "scrap.op.opacity_down",
        default_shortcut: &[],
        default_mouse: Some(MouseGesture::CtrlWheelDown),
        group: Group::View,
        order: 22,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: false,
        can_execute: not_compact,
        checked: None,
        choices: None,
        run: opacity_down,
    },
    OperationDescriptor {
        id: "border",
        label_key: "scrap.op.border",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::View,
        order: 23,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Never,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
        choices: Some(border_choices),
        run: border_to,
    },
    OperationDescriptor {
        id: "shadow",
        label_key: "scrap.op.shadow",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::View,
        order: 24,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Never,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
        choices: Some(shadow_choices),
        run: shadow_to,
    },
    OperationDescriptor {
        id: "reset_appearance",
        label_key: "scrap.op.reset_appearance",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::View,
        order: 25,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: not_compact,
        checked: None,
        choices: None,
        run: reset_appearance,
    },
    OperationDescriptor {
        id: "appearance",
        label_key: "scrap.op.set_appearance",
        default_shortcut: &[],
        default_mouse: None,
        group: Group::View,
        order: 26,
        drag_only: false,
        takes_values: true,
        in_recipe: InRecipe::Anywhere,
        in_menu: false,
        can_execute: not_compact,
        checked: None,
        choices: None,
        run: set_appearance,
    },
    OperationDescriptor {
        id: "toggle_compact",
        label_key: "scrap.op.toggle_compact",
        default_shortcut: &[],
        default_mouse: Some(MouseGesture::DoubleClick),
        group: Group::View,
        order: 30,
        drag_only: false,
        takes_values: false,
        in_recipe: InRecipe::Anywhere,
        in_menu: true,
        can_execute: normal,
        checked: None,
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
        takes_values: false,
        in_recipe: InRecipe::Last,
        in_menu: true,
        can_execute: normal,
        checked: None,
        choices: None,
        run: close,
    },
];

/// id から表の行を引く。
pub fn find(id: &str) -> Option<&'static OperationDescriptor> {
    OPERATIONS.iter().find(|op| op.id == id)
}

/// 設定 GUI の「スクラップ操作」タブの 1 行。
#[derive(Debug, Clone, Copy)]
pub struct BindingRow {
    pub op: &'static OperationDescriptor,
    /// ショートカットの欄があれば、その既定（複数のこともある。空なら既定なし）。None なら欄がない
    /// （引数のある操作）。
    pub default_shortcut: Option<&'static [Chord]>,
}

impl BindingRow {
    /// マウスの割り当てのドロップダウンの候補（表示の順）。左ドラッグは移動（`move`）専用で、移動の候補は
    /// 左ドラッグだけ（変えられない）。ほかの操作には左ドラッグを出さない。
    pub fn mouse_choices(&self) -> &'static [MouseGesture] {
        if self.op.drag_only {
            &[MouseGesture::LeftDrag]
        } else {
            &[
                MouseGesture::WheelUp,
                MouseGesture::WheelDown,
                MouseGesture::CtrlWheelUp,
                MouseGesture::CtrlWheelDown,
                MouseGesture::MiddleClick,
                MouseGesture::DoubleClick,
            ]
        }
    }
}

/// 設定 GUI の「スクラップ操作」タブの行の一覧。`ops`（ふつうは `OPERATIONS`）の表の順で、ショートカットか
/// マウスを割り当てられる操作の行を作る。引数の選択肢を持つ操作（`choices` が Some）はメニュー専用なので
/// 行にしない。レシピの値のある手からだけ呼ぶ操作（`takes_values`）も行にしない。左ドラッグの移動（`drag_only`）は行にするが、ショートカットの欄はない。
pub fn binding_rows(ops: &'static [OperationDescriptor]) -> Vec<BindingRow> {
    ops.iter()
        .filter(|op| op.choices.is_none() && !op.takes_values)
        .map(|op| BindingRow {
            op,
            default_shortcut: op.takes_binding().then_some(op.default_shortcut),
        })
        .collect()
}

/// ショートカットの対応表。表の既定値に、設定ファイルの上書き（`[shortcuts]`）と、検査を通ったレシピの
/// ショートカット（`[[recipes]]`）を重ねて作る。
///
/// - 上書きは操作の id ごとに 1 つ。Some ならその操作の既定のショートカットを全部それ 1 つに置き換え、
///   None（設定ファイルの `""`）なら割り当てを外す。書かなかった操作は既定値のまま。
/// - 表にない id と、引数のある操作（`choices` が Some）への上書きは無視し、`ignored_overrides` に載せる
///   （設定の通知で項目名を出すため。`settings::keymap` が `SettingsIssue` にする）。
/// - 同じキーの組み合わせが 2 つ以上の操作かレシピに付いたら、どれにも付けず `conflicts` に載せる
///   （同じ範囲の重なりの規則 `settings::assign`。ホットキー同士と同じ）。
/// - スクラップにフォーカスがあるときだけ引く（ホットキーと違って OS には登録しない）。
#[derive(Debug, Clone)]
pub struct Shortcuts {
    bindings: BTreeMap<Chord, Bound>,
    /// ショートカットの付いたレシピ（`Bound::Recipe` の添字）。
    recipes: Vec<Plan>,
    ignored: Vec<String>,
    conflicts: Vec<Conflict<ShortcutOwner>>,
}

/// ショートカットに付いたもの。
#[derive(Debug, Clone, Copy)]
enum Bound {
    Operation(&'static OperationDescriptor),
    /// `Shortcuts::recipes` の添字。
    Recipe(usize),
}

/// ショートカットの持ち主（重なりの知らせに使う）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutOwner {
    Operation(OperationId),
    /// レシピの `Settings::recipes` の中の位置。
    Recipe(usize),
}

impl Shortcuts {
    /// `ops`（ふつうは `OPERATIONS`）の既定値に `overrides` を重ねる。レシピは付けない。
    pub fn new(ops: &'static [OperationDescriptor], overrides: &ShortcutOverrides) -> Shortcuts {
        Shortcuts::with_recipes(ops, overrides, &[])
    }

    /// `new` に、検査を通ったレシピ（`recipe::plan`）のショートカットを足す。
    pub fn with_recipes(
        ops: &'static [OperationDescriptor],
        overrides: &ShortcutOverrides,
        plans: &[Plan],
    ) -> Shortcuts {
        let bindable = |id: &str| ops.iter().any(|op| op.id == id && op.takes_binding());
        let ignored = overrides
            .keys()
            .filter(|id| !bindable(id))
            .cloned()
            .collect();
        let candidates = ops.iter().filter(|op| op.takes_binding()).flat_map(|op| {
            let chords = match overrides.get(op.id) {
                Some(chord) => chord.as_slice(),
                None => op.default_shortcut,
            };
            chords
                .iter()
                .map(move |&chord| (chord, Bound::Operation(op)))
        });
        let recipes: Vec<Plan> = plans
            .iter()
            .filter(|plan| plan.shortcut.is_some())
            .cloned()
            .collect();
        let recipe_candidates = recipes
            .iter()
            .enumerate()
            .filter_map(|(i, plan)| Some((plan.shortcut?, Bound::Recipe(i))));
        let assigned = settings::assign(candidates.chain(recipe_candidates));
        let owner = |bound: &Bound| match *bound {
            Bound::Operation(op) => ShortcutOwner::Operation(op.id),
            Bound::Recipe(i) => ShortcutOwner::Recipe(recipes[i].index),
        };
        let conflicts = assigned
            .conflicts
            .iter()
            .map(|c| Conflict {
                chord: c.chord,
                owners: c.owners.iter().map(owner).collect(),
            })
            .collect();
        Shortcuts {
            bindings: assigned.bound,
            recipes,
            ignored,
            conflicts,
        }
    }

    /// キーの組み合わせに付いた操作。
    pub fn operation(&self, chord: Chord) -> Option<&'static OperationDescriptor> {
        match self.bindings.get(&chord)? {
            Bound::Operation(op) => Some(op),
            Bound::Recipe(_) => None,
        }
    }

    /// キーの組み合わせに付いたレシピ。
    pub fn recipe(&self, chord: Chord) -> Option<&Plan> {
        match self.bindings.get(&chord)? {
            Bound::Operation(_) => None,
            Bound::Recipe(i) => self.recipes.get(*i),
        }
    }

    /// キーの組み合わせに付いたものの持ち主。
    pub fn owner(&self, chord: Chord) -> Option<ShortcutOwner> {
        Some(match *self.bindings.get(&chord)? {
            Bound::Operation(op) => ShortcutOwner::Operation(op.id),
            Bound::Recipe(i) => ShortcutOwner::Recipe(self.recipes[i].index),
        })
    }

    /// 操作に付いたキーの組み合わせ。
    pub fn chords(&self, id: &str) -> Vec<Chord> {
        self.bindings
            .iter()
            .filter(|(_, bound)| matches!(bound, Bound::Operation(op) if op.id == id))
            .map(|(&chord, _)| chord)
            .collect()
    }

    /// 無視した上書きの id（表にない、または引数のある操作）。
    pub fn ignored_overrides(&self) -> &[String] {
        &self.ignored
    }

    /// 2 つ以上の操作かレシピに付いたため、どれにも付けなかったキーの組み合わせと、その持ち主。
    pub fn conflicts(&self) -> &[Conflict<ShortcutOwner>] {
        &self.conflicts
    }
}

/// マウスの対応表。表の既定値に、設定ファイルの上書き（`[mouse]`）を重ねて作る。規則は `Shortcuts` と同じ。
///
/// - 上書きは操作の id ごとに 1 つ。Some ならその操作のマウスの操作をそれに置き換え、None（設定ファイルの
///   `""`）なら割り当てを外す。書かなかった操作は既定値のまま。
/// - 付けられるのは引数のない操作だけ（ショートカットと同じ）。表にない id、引数のある操作（`choices` が
///   Some、`drag_only`）への上書き、候補にない値（引数のない操作への `LeftDrag`）は無視し、
///   `ignored_overrides` に載せる。左ドラッグは移動（`move`）に付いたまま変えられない。
/// - ホイールの上下は別々のマウスの操作で、別々の操作（`zoom_in` と `zoom_out` など）に 1 つずつ付く。
///   上下をまとめて引数のある操作に付ける書き方はない。入れ替えるなら上と下を両方書く。
/// - 同じマウスの操作が 2 つ以上の操作に付いたら、どれにも付けず `conflicts` に載せる（表の既定値から
///   来たものも数える）。
#[derive(Debug, Clone)]
pub struct MouseBindings {
    bindings: BTreeMap<MouseGesture, &'static OperationDescriptor>,
    ignored: Vec<String>,
    conflicts: Vec<MouseConflict>,
}

/// 2 つ以上の操作に付いたため、どれにも付けなかったマウスの操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseConflict {
    pub gesture: MouseGesture,
    /// 重なった操作（表の順）。
    pub owners: Vec<OperationId>,
}

impl MouseBindings {
    /// `ops`（ふつうは `OPERATIONS`）の既定値に `overrides` を重ねる。
    pub fn new(ops: &'static [OperationDescriptor], overrides: &MouseOverrides) -> MouseBindings {
        // 左ドラッグはドラッグの量を取る操作（`drag_only`）のもので、引数のない操作の候補にはない
        let accepts = |id: &str, gesture: Option<MouseGesture>| {
            gesture != Some(MouseGesture::LeftDrag)
                && ops.iter().any(|op| op.id == id && op.takes_binding())
        };
        let ignored = overrides
            .iter()
            .filter(|&(id, &gesture)| !accepts(id, gesture))
            .map(|(id, _)| id.clone())
            .collect();
        let mut grouped: BTreeMap<MouseGesture, Vec<&'static OperationDescriptor>> =
            BTreeMap::new();
        for op in ops {
            let gesture = match overrides.get(op.id) {
                Some(&gesture) if accepts(op.id, gesture) => gesture,
                _ => op.default_mouse,
            };
            if let Some(gesture) = gesture {
                grouped.entry(gesture).or_default().push(op);
            }
        }
        let mut bindings = BTreeMap::new();
        let mut conflicts = Vec::new();
        for (gesture, owners) in grouped {
            match owners.as_slice() {
                [op] => {
                    bindings.insert(gesture, *op);
                }
                _ => conflicts.push(MouseConflict {
                    gesture,
                    owners: owners.iter().map(|op| op.id).collect(),
                }),
            }
        }
        MouseBindings {
            bindings,
            ignored,
            conflicts,
        }
    }

    /// マウスの操作に付いた操作。
    pub fn operation(&self, gesture: MouseGesture) -> Option<&'static OperationDescriptor> {
        self.bindings.get(&gesture).copied()
    }

    /// 無視した上書きの id。
    pub fn ignored_overrides(&self) -> &[String] {
        &self.ignored
    }

    /// 2 つ以上の操作に付いたため、どれにも付けなかったマウスの操作と、その操作。
    pub fn conflicts(&self) -> &[MouseConflict] {
        &self.conflicts
    }
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
    /// 操作をまとめた子メニュー（「見た目」など。`FOLDERS`）。それ自体は操作ではない。
    Folder(Folder),
    Separator,
}

/// 操作をまとめた子メニュー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub label_key: &'static str,
    pub entries: Vec<MenuEntry>,
}

impl Folder {
    /// 中の操作が 1 つでも実行できるか。false なら親の項目をグレー表示にする。
    pub fn is_enabled(&self) -> bool {
        self.entries.iter().any(|entry| match entry {
            MenuEntry::Item(item) => item.enabled,
            MenuEntry::Submenu(submenu) => submenu.enabled,
            MenuEntry::Folder(folder) => folder.is_enabled(),
            MenuEntry::Separator => false,
        })
    }
}

/// メニューで子メニューにまとめる操作（文言のキー、区切りで分けた操作の id の並び）。まとめた操作は、最初の
/// 操作の位置に並ぶ。子メニューの中では、違う並びの操作の間に区切りを入れる。
const FOLDERS: &[(&str, &[&[OperationId]])] = &[(
    "scrap.op.appearance",
    &[&["opacity", "border", "shadow"], &["reset_appearance"]],
)];

/// 白紙スクラップのメニューにだけ出す操作。普通のスクラップでは、グレー表示にもせずに出さない。
const BLANK_ONLY: &[OperationId] = &["background"];

/// `id` をまとめる子メニューの文言のキーと、その中の区切りで分けた並びの番号。
fn folder_of(id: OperationId) -> Option<(&'static str, usize)> {
    FOLDERS.iter().find_map(|&(label_key, sections)| {
        sections
            .iter()
            .position(|ids| ids.contains(&id))
            .map(|section| (label_key, section))
    })
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
    /// チェックを付ける（`OperationDescriptor::checked`）。
    pub checked: bool,
}

/// 右クリックメニューを `ops`（ふつうは `OPERATIONS`）から作る。`group` の宣言の順、その中は `order` の順に並べ、
/// `group` の変わり目に区切りを入れる。ドラッグでしか呼べない操作と `in_menu` が false の操作は出さない。
/// 引数のある操作（`choices`）は子メニュー（`MenuEntry::Submenu`）にし、`FOLDERS` の操作は `MenuEntry::Folder`
/// にまとめる。白紙スクラップだけの操作（`BLANK_ONLY`）は、白紙スクラップのときだけ出す。`defaults` は全体の見た目の既定値（見た目の一覧のチェックに使う）。
pub fn menu(
    ops: &'static [OperationDescriptor],
    model: &ScrapModel,
    shortcuts: &Shortcuts,
    defaults: &Appearance,
) -> Vec<MenuEntry> {
    let mut listed: Vec<&OperationDescriptor> = ops
        .iter()
        .filter(|op| op.in_menu && !op.drag_only)
        .filter(|op| model.background().is_some() || !BLANK_ONLY.contains(&op.id))
        .collect();
    listed.sort_by_key(|op| (op.group, op.order));
    let mut entries = Vec::new();
    let mut group = None;
    // 直前に子メニューへ入れた操作の、区切りで分けた並びの番号。
    let mut folder_section = None;
    for op in listed {
        if group.is_some_and(|g| g != op.group) {
            entries.push(MenuEntry::Separator);
        }
        group = Some(op.group);
        let enabled = (op.can_execute)(model);
        let entry = match op.choices {
            Some(choices) => MenuEntry::Submenu(Submenu {
                id: op.id,
                label_key: op.label_key,
                enabled,
                choices: choices(model, defaults),
            }),
            None => MenuEntry::Item(MenuItem {
                id: op.id,
                label_key: op.label_key,
                enabled,
                shortcut: shortcuts.chords(op.id).first().copied(),
                checked: op.checked.is_some_and(|checked| checked(model)),
            }),
        };
        let Some((label_key, section)) = folder_of(op.id) else {
            entries.push(entry);
            folder_section = None;
            continue;
        };
        match entries.last_mut() {
            Some(MenuEntry::Folder(folder)) if folder.label_key == label_key => {
                if folder_section != Some(section) {
                    folder.entries.push(MenuEntry::Separator);
                }
                folder.entries.push(entry);
            }
            _ => entries.push(MenuEntry::Folder(Folder {
                label_key,
                entries: vec![entry],
            })),
        }
        folder_section = Some(section);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{Carryover, EditOutput, Font, Pick, Style, Tool};
    use crate::geometry::{Point, Rect, Size};
    use crate::image::Image;
    use crate::recipe::AppearanceValues;
    use crate::scrap::{
        Appearance, Background, Border, Mode, ScrapModel, TrimInput, ZOOM_ACTUAL, ZOOM_MAX,
        ZOOM_MIN, ZoomAnchor,
    };
    use crate::settings::{Conflict, Rgb};

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
            ("save_scrap_file", Effect::ShowScrapFileSaveDialog),
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
        let mouse = MouseBindings::new(OPERATIONS, &MouseOverrides::new());
        let id = |g| mouse_bound(&mouse, g);
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
        assert!(model.is_trimming());
        trim(&mut model);
        assert_eq!(model.mode, Mode::Normal);
        assert!(model.is_trimmed());
    }

    fn editing_context() -> OpContext {
        OpContext {
            annotation: Style {
                color: Rgb(0, 0x80, 0xFF),
                width: 5,
            },
            ..OpContext::default()
        }
    }

    #[test]
    fn annotate_enters_the_edit_mode_with_the_configured_style_and_leaves_it_when_run_again() {
        let mut model = scrap();
        let effects = execute(op("annotate"), &mut model, &editing_context());
        assert_eq!(effects, vec![Effect::EnterEditMode]);
        let Mode::Editing(state) = &model.mode else {
            panic!("not editing: {:?}", model.mode);
        };
        assert_eq!(state.style, editing_context().annotation);
        let effects = execute(op("annotate"), &mut model, &editing_context());
        assert_eq!(effects, vec![Effect::LeaveEditMode]);
        assert_eq!(model.mode, Mode::Normal);
    }

    #[test]
    fn the_colour_and_width_chosen_on_one_scrap_carry_over_to_the_next_scrap() {
        let defaults = editing_context();
        let mut carryover = Carryover::default();
        let mut first = scrap();
        let context = OpContext {
            carryover,
            ..defaults.clone()
        };
        execute(op("annotate"), &mut first, &context);
        let blue = Rgb(0x1E, 0x88, 0xE5);
        for pick in [Pick::Color(blue), Pick::Width(8)] {
            let outputs = first.edit_input(EditInput::Pick(pick));
            assert!(outputs.contains(&EditOutput::Picked(pick)));
            carryover.record(&outputs);
        }
        let chosen = Style {
            color: blue,
            width: 8,
        };
        assert_eq!(first.edit_state().map(|s| s.style), Some(chosen));
        execute(op("annotate"), &mut first, &context);
        assert_eq!(first.edit_state(), None);

        // 別のスクラップで入っても前回の値。設定の既定値（defaults.annotation）には戻らない
        let mut second = scrap();
        let context = OpContext {
            carryover,
            ..defaults.clone()
        };
        execute(op("annotate"), &mut second, &context);
        assert_eq!(second.edit_state().map(|s| s.style), Some(chosen));
    }

    #[test]
    fn edit_mode_types_text_in_the_configured_font() {
        let mut model = scrap();
        let font = Font {
            family: "Meiryo".to_owned(),
            size: 24,
        };
        let context = OpContext {
            font: font.clone(),
            ..editing_context()
        };
        execute(op("annotate"), &mut model, &context);
        assert_eq!(model.edit_state().map(|s| s.font.clone()), Some(font));
    }

    #[test]
    fn edit_mode_starts_with_the_drawing_tool_used_last_on_any_scrap() {
        let defaults = editing_context();
        let mut carryover = Carryover::default();
        let mut first = scrap();
        let context = OpContext {
            carryover,
            ..defaults.clone()
        };
        execute(op("annotate"), &mut first, &context);
        assert_eq!(first.edit_state().map(|s| s.tool), Some(Tool::Rect));
        // ツールバーで楕円、続けて数字キーの 4 で線
        carryover.record(&first.edit_input(EditInput::Pick(Pick::Tool(Tool::Ellipse))));
        let four = Chord::plain(Key::known(0x34));
        carryover.record(&first.edit_input(EditInput::Key(four)));
        execute(op("annotate"), &mut first, &context);

        let mut second = scrap();
        let context = OpContext {
            carryover,
            ..defaults.clone()
        };
        execute(op("annotate"), &mut second, &context);
        assert_eq!(second.edit_state().map(|s| s.tool), Some(Tool::Line));
    }

    #[test]
    fn only_the_width_carries_over_when_only_the_width_was_chosen() {
        let mut carryover = Carryover::default();
        carryover.record(&[EditOutput::Picked(Pick::Width(2))]);
        let state = carryover.start(editing_context().annotation);
        assert_eq!(
            state.style,
            Style {
                color: editing_context().annotation.color,
                width: 2
            }
        );
        assert_eq!(state.tool, Tool::Rect);
    }

    #[test]
    fn only_annotate_can_run_while_editing() {
        let mut model = scrap();
        trim(&mut model);
        execute(op("annotate"), &mut model, &editing_context());
        for op in OPERATIONS {
            assert_eq!((op.can_execute)(&model), op.id == "annotate", "{}", op.id);
        }
        let before = model.clone();
        assert_eq!(
            execute(op("move_left"), &mut model, &OpContext::default()),
            vec![]
        );
        assert_eq!(model, before);
    }

    #[test]
    fn annotate_cannot_run_while_folded_or_trimming() {
        let mut model = scrap();
        model.enter_compact(ZoomAnchor::Center);
        assert!(!(op("annotate").can_execute)(&model));
        model.leave_compact();
        execute(op("trim"), &mut model, &OpContext::default());
        assert!(!(op("annotate").can_execute)(&model));
    }

    #[test]
    fn the_menu_checks_annotate_while_editing_and_greys_out_the_rest() {
        let shortcuts = Shortcuts::new(OPERATIONS, &ShortcutOverrides::new());
        let mut model = scrap();
        let items = |model: &ScrapModel| -> Vec<(OperationId, bool, bool)> {
            menu(OPERATIONS, model, &shortcuts, &Appearance::default())
                .into_iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Item(item) => Some((item.id, item.enabled, item.checked)),
                    _ => None,
                })
                .collect()
        };
        assert!(items(&model).iter().all(|&(_, _, checked)| !checked));
        execute(op("annotate"), &mut model, &editing_context());
        for (id, enabled, checked) in items(&model) {
            assert_eq!(enabled, id == "annotate", "{id}");
            assert_eq!(checked, id == "annotate", "{id}");
        }
    }

    #[test]
    fn the_appearance_folder_is_greyed_out_while_editing() {
        let shortcuts = Shortcuts::new(OPERATIONS, &ShortcutOverrides::new());
        let mut model = scrap();
        let folder = |model: &ScrapModel| -> Folder {
            menu(OPERATIONS, model, &shortcuts, &Appearance::default())
                .into_iter()
                .find_map(|entry| match entry {
                    MenuEntry::Folder(folder) => Some(folder),
                    _ => None,
                })
                .unwrap()
        };
        assert!(folder(&model).is_enabled());
        execute(op("annotate"), &mut model, &editing_context());
        assert!(!folder(&model).is_enabled());
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

    /// 全体の既定値を `defaults` にして呼んだときの引数。
    fn with_defaults(defaults: Appearance) -> OpContext {
        OpContext {
            appearance: defaults,
            ..OpContext::default()
        }
    }

    fn checked_values(id: &str, model: &ScrapModel, defaults: &Appearance) -> Vec<i32> {
        (op(id).choices.expect("has choices"))(model, defaults)
            .iter()
            .filter(|c| c.checked)
            .map(|c| c.value)
            .collect()
    }

    #[test]
    fn ctrl_wheel_changes_the_opacity_by_10_within_10_to_100() {
        let mouse = MouseBindings::new(OPERATIONS, &MouseOverrides::new());
        let id = |g| mouse_bound(&mouse, g);
        assert_eq!(id(MouseGesture::CtrlWheelUp), Some("opacity_up"));
        assert_eq!(id(MouseGesture::CtrlWheelDown), Some("opacity_down"));
        let defaults = Appearance::default();
        let mut model = scrap();
        let context = with_defaults(defaults);
        // 既定値の 100% より上はない
        assert_eq!(execute(op("opacity_up"), &mut model, &context), vec![]);
        assert_eq!(model.appearance_overrides.opacity, None);
        let mut seen = Vec::new();
        while execute(op("opacity_down"), &mut model, &context) == [Effect::Redraw] {
            seen.push(model.appearance(&defaults).opacity);
        }
        assert_eq!(seen, [90, 80, 70, 60, 50, 40, 30, 20, 10]);
        assert_eq!(execute(op("opacity_down"), &mut model, &context), vec![]);
        assert_eq!(
            execute(op("opacity_up"), &mut model, &context),
            vec![Effect::Redraw]
        );
        assert_eq!(model.appearance(&defaults).opacity, 20);
        // 既定値が刻みの間なら、次の刻みに合わせる
        let mut model = scrap();
        let context = with_defaults(Appearance {
            opacity: 85,
            ..defaults
        });
        execute(op("opacity_up"), &mut model, &context);
        assert_eq!(model.appearance_overrides.opacity, Some(90));
        let mut model = scrap();
        execute(op("opacity_down"), &mut model, &context);
        assert_eq!(model.appearance_overrides.opacity, Some(80));
    }

    #[test]
    fn the_opacity_list_checks_the_actual_opacity_and_choosing_overrides_it() {
        let defaults = Appearance {
            opacity: 70,
            ..Appearance::default()
        };
        let choices = (op("opacity").choices.unwrap())(&scrap(), &defaults);
        assert_eq!(
            choices.iter().map(|c| c.label).collect::<Vec<_>>(),
            [100, 90, 80, 70, 60, 50, 40, 30, 20, 10].map(ChoiceLabel::Percent)
        );
        assert_eq!(checked_values("opacity", &scrap(), &defaults), [70]);
        let mut model = scrap();
        let chosen = OpContext {
            choice: Some(30),
            ..with_defaults(defaults)
        };
        assert_eq!(
            execute(op("opacity"), &mut model, &chosen),
            vec![Effect::Redraw]
        );
        assert_eq!(model.appearance(&defaults).opacity, 30);
        assert_eq!(checked_values("opacity", &model, &defaults), [30]);
        // 範囲の外の値は範囲に収める。選択肢なしなら何もしない
        let out = OpContext {
            choice: Some(5),
            ..with_defaults(defaults)
        };
        execute(op("opacity"), &mut model, &out);
        assert_eq!(model.appearance(&defaults).opacity, 10);
        assert_eq!(
            execute(op("opacity"), &mut model, &with_defaults(defaults)),
            vec![]
        );
    }

    #[test]
    fn the_border_list_offers_none_and_colors_and_checks_the_actual_border() {
        let defaults = Appearance::default();
        let choices = (op("border").choices.unwrap())(&scrap(), &defaults);
        assert_eq!(
            choices
                .iter()
                .map(|c| (c.label, c.value))
                .collect::<Vec<_>>(),
            [
                (ChoiceLabel::Key("scrap.border.none"), -1),
                (ChoiceLabel::Key("scrap.border.default"), 0x80_80_80),
                (ChoiceLabel::Key("scrap.border.black"), 0x00_00_00),
                (ChoiceLabel::Key("scrap.border.white"), 0xFF_FF_FF),
                (ChoiceLabel::Key("scrap.border.red"), 0xFF_00_00),
                (ChoiceLabel::Key("scrap.border.blue"), 0x00_00_FF),
            ]
        );
        // 既定値は 1px の灰色
        assert_eq!(checked_values("border", &scrap(), &defaults), [0x80_80_80]);
        let no_border = Appearance {
            show_border: false,
            ..defaults
        };
        assert_eq!(checked_values("border", &scrap(), &no_border), [-1]);
    }

    #[test]
    fn the_border_list_puts_the_configured_color_first_without_repeating_it() {
        let red = Appearance {
            border: Border {
                width: 2,
                color: 0xFF_00_00,
            },
            ..Appearance::default()
        };
        let choices = (op("border").choices.unwrap())(&scrap(), &red);
        assert_eq!(
            choices
                .iter()
                .map(|c| (c.label, c.value))
                .collect::<Vec<_>>(),
            [
                (ChoiceLabel::Key("scrap.border.none"), -1),
                (ChoiceLabel::Key("scrap.border.default"), 0xFF_00_00),
                (ChoiceLabel::Key("scrap.border.gray"), 0x80_80_80),
                (ChoiceLabel::Key("scrap.border.black"), 0x00_00_00),
                (ChoiceLabel::Key("scrap.border.white"), 0xFF_FF_FF),
                (ChoiceLabel::Key("scrap.border.blue"), 0x00_00_FF),
            ]
        );
        assert_eq!(checked_values("border", &scrap(), &red), [0xFF_00_00]);
        // 決まった色にない色でも、設定の色なら先頭に出してチェックを付ける
        let custom = Appearance {
            border: Border {
                width: 2,
                color: 0x12_34_56,
            },
            ..Appearance::default()
        };
        assert_eq!(checked_values("border", &scrap(), &custom), [0x12_34_56]);
    }

    #[test]
    fn a_border_added_to_a_scrap_uses_the_configured_width_even_when_the_settings_hide_it() {
        let defaults = Appearance {
            border: Border {
                width: 3,
                color: 0x80_80_80,
            },
            show_border: false,
            ..Appearance::default()
        };
        let mut model = scrap();
        let choose = OpContext {
            choice: Some(0x00_00_FF),
            ..with_defaults(defaults)
        };
        assert_eq!(
            execute(op("border"), &mut model, &choose),
            vec![Effect::Redraw]
        );
        assert_eq!(
            model.appearance(&defaults).shown_border(),
            Some(Border {
                width: 3,
                color: 0x00_00_FF,
            })
        );
    }

    #[test]
    fn choosing_a_border_changes_only_this_scrap_and_redraws() {
        let defaults = Appearance::default();
        let mut model = scrap();
        let choose = |value| OpContext {
            choice: Some(value),
            ..with_defaults(defaults)
        };
        assert_eq!(
            execute(op("border"), &mut model, &choose(-1)),
            vec![Effect::Redraw]
        );
        assert_eq!(model.appearance(&defaults).shown_border(), None);
        assert_eq!(checked_values("border", &model, &defaults), [-1]);
        assert_eq!(
            execute(op("border"), &mut model, &choose(0xFF_00_00)),
            vec![Effect::Redraw]
        );
        assert_eq!(
            model.appearance(&defaults).shown_border(),
            Some(Border {
                width: 1,
                color: 0xFF_00_00,
            })
        );
        // 同じものを選び直しても変わらない
        assert_eq!(
            execute(op("border"), &mut model, &choose(0xFF_00_00)),
            vec![]
        );
        assert_eq!(scrap().appearance(&defaults), defaults);
    }

    #[test]
    fn the_shadow_list_offers_on_and_off_and_choosing_switches_it() {
        let defaults = Appearance::default();
        let choices = (op("shadow").choices.unwrap())(&scrap(), &defaults);
        assert_eq!(
            choices
                .iter()
                .map(|c| (c.label, c.value))
                .collect::<Vec<_>>(),
            [
                (ChoiceLabel::Key("scrap.shadow.on"), 1),
                (ChoiceLabel::Key("scrap.shadow.off"), 0),
            ]
        );
        assert_eq!(checked_values("shadow", &scrap(), &defaults), [1]);
        let mut model = scrap();
        let off = OpContext {
            choice: Some(0),
            ..with_defaults(defaults)
        };
        assert_eq!(
            execute(op("shadow"), &mut model, &off),
            vec![Effect::Redraw]
        );
        assert!(!model.appearance(&defaults).shadow);
        assert_eq!(checked_values("shadow", &model, &defaults), [0]);
        assert_eq!(execute(op("shadow"), &mut model, &off), vec![]);
    }

    #[test]
    fn resetting_the_appearance_drops_every_override_and_keeps_the_image_in_place() {
        let defaults = Appearance::default();
        let mut model = scrap();
        let context = |choice| OpContext {
            choice: Some(choice),
            ..with_defaults(defaults)
        };
        execute(op("opacity"), &mut model, &context(40));
        execute(op("border"), &mut model, &context(0xFF_00_00));
        execute(op("shadow"), &mut model, &context(0));
        assert_ne!(model.appearance(&defaults), defaults);

        assert_eq!(
            execute(op("reset_appearance"), &mut model, &with_defaults(defaults)),
            vec![Effect::Redraw]
        );
        assert_eq!(model.appearance(&defaults), defaults);
        assert_eq!(model.window_rect(&defaults), scrap().window_rect(&defaults));
        assert_eq!(model.position, scrap().position);
    }

    #[test]
    fn a_border_width_step_shows_that_width_and_the_menu_changes_only_the_color() {
        let defaults = Appearance {
            show_border: false,
            ..Appearance::default()
        };
        let mut model = scrap();
        let width = |w| OpContext {
            values: Some(StepValues::Appearance(AppearanceValues {
                border_width: Some(w),
                ..AppearanceValues::default()
            })),
            ..with_defaults(defaults)
        };
        // 太さを書けば、既定値で枠を出さなくても枠が出る
        assert_eq!(
            execute(op("appearance"), &mut model, &width(4)),
            vec![Effect::Redraw]
        );
        assert_eq!(
            model.appearance(&defaults).shown_border(),
            Some(Border {
                width: 4,
                color: 0x80_80_80
            })
        );
        assert_eq!(execute(op("appearance"), &mut model, &width(4)), vec![]);
        let choose = |choice| OpContext {
            choice: Some(choice),
            ..with_defaults(defaults)
        };
        execute(op("border"), &mut model, &choose(0xFF_00_00));
        assert_eq!(
            model.appearance(&defaults).shown_border(),
            Some(Border {
                width: 4,
                color: 0xFF_00_00
            })
        );
        execute(op("border"), &mut model, &choose(NO_BORDER));
        execute(op("border"), &mut model, &choose(0x00_00_FF));
        assert_eq!(
            model.appearance(&defaults).shown_border(),
            Some(Border {
                width: 4,
                color: 0x00_00_FF
            })
        );
        // 既定に戻すと太さの上書きも消える
        execute(op("reset_appearance"), &mut model, &with_defaults(defaults));
        execute(op("border"), &mut model, &choose(0x00_00_FF));
        assert_eq!(
            model.appearance(&defaults).shown_border(),
            Some(Border {
                width: 1,
                color: 0x00_00_FF
            })
        );
    }

    #[test]
    fn a_step_that_writes_the_border_keeps_it_shown_when_the_default_stops_showing_it() {
        let shown = Appearance::default();
        let hidden = Appearance {
            show_border: false,
            ..shown
        };
        for values in [
            AppearanceValues {
                border_width: Some(3),
                ..AppearanceValues::default()
            },
            AppearanceValues {
                border: Some(true),
                ..AppearanceValues::default()
            },
            AppearanceValues {
                border_color: Some(settings::Rgb(0x80, 0x80, 0x80)),
                ..AppearanceValues::default()
            },
        ] {
            let mut model = scrap();
            let context = OpContext {
                values: Some(StepValues::Appearance(values)),
                ..with_defaults(shown)
            };
            execute(op("appearance"), &mut model, &context);

            // 全体の既定値で枠を出さなくなっても、手で書いた枠は出たまま
            assert_eq!(
                model.appearance(&hidden).shown_border(),
                Some(Border {
                    width: values.border_width.unwrap_or(1),
                    color: 0x80_80_80
                }),
                "{values:?}"
            );
        }
    }

    #[test]
    fn resetting_an_appearance_already_at_the_defaults_changes_nothing() {
        let defaults = Appearance::default();
        let mut model = scrap();
        assert_eq!(
            execute(op("reset_appearance"), &mut model, &with_defaults(defaults)),
            vec![]
        );
        assert_eq!(model, scrap());
    }

    #[test]
    fn resetting_the_appearance_has_no_default_shortcut_and_takes_one_from_the_settings_file() {
        assert_eq!(op("reset_appearance").default_shortcut, &[]);
        let (settings, issues) = settings::load(
            r#"version = 1
[shortcuts]
reset_appearance = "Ctrl+Shift+D"
"#,
        );
        assert_eq!(issues, vec![]);
        let keymap = settings::keymap(&settings, OPERATIONS);
        assert_eq!(keymap.issues, vec![]);
        assert_eq!(
            bound(&keymap.shortcuts, "Ctrl+Shift+D"),
            Some("reset_appearance")
        );
    }

    #[test]
    fn the_appearance_lists_take_no_shortcut() {
        let overrides = ShortcutOverrides::from([
            ("opacity".to_owned(), Some(chord("O"))),
            ("border".to_owned(), Some(chord("B"))),
            ("shadow".to_owned(), Some(chord("H"))),
            ("opacity_up".to_owned(), Some(chord("Ctrl+Up"))),
        ]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        assert_eq!(
            shortcuts.ignored_overrides(),
            ["border", "opacity", "shadow"]
        );
        assert_eq!(bound(&shortcuts, "Ctrl+Up"), Some("opacity_up"));
    }

    #[test]
    fn setting_the_appearance_by_values_is_only_for_recipe_steps() {
        let overrides = ShortcutOverrides::from([("appearance".to_owned(), Some(chord("Q")))]);
        let shortcuts = Shortcuts::new(OPERATIONS, &overrides);
        assert_eq!(shortcuts.ignored_overrides(), ["appearance"]);
        assert_eq!(bound(&shortcuts, "Q"), None);
        let mouse = MouseBindings::new(
            OPERATIONS,
            &MouseOverrides::from([("appearance".to_owned(), Some(MouseGesture::WheelUp))]),
        );
        assert_eq!(mouse.ignored_overrides(), ["appearance"]);
        assert!(
            binding_rows(OPERATIONS)
                .iter()
                .all(|row| row.op.id != "appearance")
        );
        fn ids(entries: &[MenuEntry]) -> Vec<&'static str> {
            entries
                .iter()
                .flat_map(|e| match e {
                    MenuEntry::Item(item) => vec![item.id],
                    MenuEntry::Submenu(sub) => vec![sub.id],
                    MenuEntry::Folder(folder) => ids(&folder.entries),
                    MenuEntry::Separator => vec![],
                })
                .collect()
        }
        let entries = menu(OPERATIONS, &scrap(), &shortcuts, &Appearance::default());
        assert!(!ids(&entries).contains(&"appearance"));
        // 値なしで呼んでも何もしない
        let mut model = scrap();
        assert_eq!(
            execute(op("appearance"), &mut model, &OpContext::default()),
            vec![]
        );
        assert_eq!(model, scrap());
    }

    #[test]
    fn a_double_click_folds_around_the_cursor_and_toggles_back() {
        let mouse = MouseBindings::new(OPERATIONS, &MouseOverrides::new());
        let id = |g| mouse_bound(&mouse, g);
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
        model.enter_compact(ZoomAnchor::Center);
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
        model.enter_compact(ZoomAnchor::Center);
        let enabled: Vec<(OperationId, bool)> =
            menu(OPERATIONS, &model, &shortcuts, &Appearance::default())
                .into_iter()
                .flat_map(|entry| match entry {
                    MenuEntry::Folder(folder) => folder.entries,
                    other => vec![other],
                })
                .filter_map(|entry| match entry {
                    MenuEntry::Item(item) => Some((item.id, item.enabled)),
                    MenuEntry::Submenu(submenu) => Some((submenu.id, submenu.enabled)),
                    MenuEntry::Folder(_) | MenuEntry::Separator => None,
                })
                .collect();
        assert_eq!(
            enabled,
            [
                ("copy", true),
                ("save_default", false),
                ("save_as", false),
                ("save_scrap_file", false),
                ("trim", false),
                ("untrim", false),
                ("annotate", false),
                ("zoom", false),
                ("opacity", false),
                ("border", false),
                ("shadow", false),
                ("reset_appearance", false),
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
        fn opacity_choices(_: &ScrapModel, _: &Appearance) -> Vec<Choice> {
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
            checked: false,
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
            menu(OPERATIONS, &scrap(), &shortcuts, &Appearance::default()),
            vec![
                item("copy", true, Some("Ctrl+C")),
                item("save_default", true, Some("Ctrl+S")),
                item("save_as", true, Some("Ctrl+Shift+S")),
                item("save_scrap_file", true, None),
                MenuEntry::Separator,
                item("trim", true, Some("Ctrl+T")),
                item("untrim", false, None),
                item("annotate", true, Some("E")),
                MenuEntry::Separator,
                MenuEntry::Submenu(Submenu {
                    id: "zoom",
                    label_key: "scrap.op.zoom",
                    enabled: true,
                    choices: zoom_list,
                }),
                MenuEntry::Folder(Folder {
                    label_key: "scrap.op.appearance",
                    entries: ["opacity", "border", "shadow"]
                        .map(|id| MenuEntry::Submenu(Submenu {
                            id,
                            label_key: op(id).label_key,
                            enabled: true,
                            choices: (op(id).choices.unwrap())(&scrap(), &Appearance::default()),
                        }))
                        .into_iter()
                        .chain([MenuEntry::Separator, item("reset_appearance", true, None),])
                        .collect(),
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
        let checked: Vec<i32> = choices(&model, &Appearance::default())
            .iter()
            .filter(|c| c.checked)
            .map(|c| c.value)
            .collect();
        assert_eq!(checked, [200]);
        model.set_zoom(125, ZoomAnchor::Center);
        assert!(
            choices(&model, &Appearance::default())
                .iter()
                .all(|c| !c.checked)
        );
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
        fn levels(_: &ScrapModel, _: &Appearance) -> Vec<Choice> {
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
                checked: None,
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
                choices: levels(&scrap(), &Appearance::default()),
            })
        };
        assert_eq!(
            menu(TABLE, &scrap(), &shortcuts, &Appearance::default()),
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
        let entries = menu(OPERATIONS, &scrap(), &shortcuts, &Appearance::default());
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
                takes_values: false,
                ..test_op("drag", Group::Move, 0, &[])
            },
            OperationDescriptor {
                can_execute: never,
                checked: None,
                ..test_op("a", Group::Move, 10, &[])
            },
        ];
        let shortcuts = Shortcuts::new(TABLE, &ShortcutOverrides::new());
        assert_eq!(
            menu(TABLE, &scrap(), &shortcuts, &Appearance::default()),
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
                owners: vec![
                    ShortcutOwner::Operation("move_left"),
                    ShortcutOwner::Operation("close")
                ],
            }]
        );
    }

    fn mouse_bound(mouse: &MouseBindings, gesture: MouseGesture) -> Option<OperationId> {
        mouse.operation(gesture).map(|op| op.id)
    }

    #[test]
    fn without_overrides_the_mouse_bindings_are_the_table_defaults() {
        let mouse = MouseBindings::new(OPERATIONS, &MouseOverrides::new());
        let bound = |g| mouse_bound(&mouse, g);
        assert_eq!(bound(MouseGesture::LeftDrag), Some("move"));
        assert_eq!(bound(MouseGesture::MiddleClick), Some("close"));
        assert_eq!(bound(MouseGesture::WheelUp), Some("zoom_in"));
        assert_eq!(bound(MouseGesture::WheelDown), Some("zoom_out"));
        assert_eq!(bound(MouseGesture::CtrlWheelUp), Some("opacity_up"));
        assert_eq!(bound(MouseGesture::CtrlWheelDown), Some("opacity_down"));
        assert_eq!(bound(MouseGesture::DoubleClick), Some("toggle_compact"));
        assert!(mouse.ignored_overrides().is_empty());
        assert!(mouse.conflicts().is_empty());
    }

    #[test]
    fn mouse_overrides_swap_double_click_and_middle_click() {
        let overrides = MouseOverrides::from([
            ("close".to_owned(), Some(MouseGesture::DoubleClick)),
            ("toggle_compact".to_owned(), Some(MouseGesture::MiddleClick)),
        ]);
        let mouse = MouseBindings::new(OPERATIONS, &overrides);
        assert_eq!(
            mouse_bound(&mouse, MouseGesture::DoubleClick),
            Some("close")
        );
        assert_eq!(
            mouse_bound(&mouse, MouseGesture::MiddleClick),
            Some("toggle_compact")
        );
        assert_eq!(mouse_bound(&mouse, MouseGesture::WheelUp), Some("zoom_in"));
        assert!(mouse.conflicts().is_empty());
    }

    #[test]
    fn an_empty_mouse_override_removes_the_binding_of_that_operation_only() {
        let overrides = MouseOverrides::from([("zoom_in".to_owned(), None)]);
        let mouse = MouseBindings::new(OPERATIONS, &overrides);
        assert_eq!(mouse_bound(&mouse, MouseGesture::WheelUp), None);
        assert_eq!(
            mouse_bound(&mouse, MouseGesture::WheelDown),
            Some("zoom_out")
        );
    }

    #[test]
    fn mouse_overrides_for_unknown_or_argument_taking_operations_are_ignored_and_reported() {
        let overrides = MouseOverrides::from([
            ("fly".to_owned(), Some(MouseGesture::WheelUp)),
            ("move".to_owned(), None),
            ("zoom".to_owned(), Some(MouseGesture::DoubleClick)),
            ("close".to_owned(), Some(MouseGesture::CtrlWheelUp)),
        ]);
        let mouse = MouseBindings::new(OPERATIONS, &overrides);
        assert_eq!(mouse.ignored_overrides(), ["fly", "move", "zoom"]);
        // 無視した上書きは既定値を変えない
        assert_eq!(mouse_bound(&mouse, MouseGesture::LeftDrag), Some("move"));
        assert_eq!(
            mouse_bound(&mouse, MouseGesture::DoubleClick),
            Some("toggle_compact")
        );
        // 無視しなかった上書きは効く（opacity_up の既定と重なる）
        assert_eq!(mouse_bound(&mouse, MouseGesture::CtrlWheelUp), None);
        assert_eq!(mouse_bound(&mouse, MouseGesture::MiddleClick), None);
    }

    #[test]
    fn left_drag_is_not_a_candidate_for_operations_without_arguments() {
        // 左ドラッグはドラッグの量を取る移動だけのもの。引数のない操作には付けられない
        let overrides = MouseOverrides::from([("close".to_owned(), Some(MouseGesture::LeftDrag))]);
        let mouse = MouseBindings::new(OPERATIONS, &overrides);
        assert_eq!(mouse.ignored_overrides(), ["close"]);
        assert_eq!(mouse_bound(&mouse, MouseGesture::LeftDrag), Some("move"));
        assert_eq!(
            mouse_bound(&mouse, MouseGesture::MiddleClick),
            Some("close")
        );
        assert!(mouse.conflicts().is_empty());
    }

    #[test]
    fn wheel_directions_are_bound_one_by_one() {
        // 上下は別々の操作に付く。上だけ書き換えれば、下は既定値のまま
        let overrides =
            MouseOverrides::from([("opacity_up".to_owned(), Some(MouseGesture::WheelUp))]);
        let mouse = MouseBindings::new(OPERATIONS, &overrides);
        assert_eq!(mouse_bound(&mouse, MouseGesture::CtrlWheelUp), None);
        assert_eq!(
            mouse_bound(&mouse, MouseGesture::CtrlWheelDown),
            Some("opacity_down")
        );
        // 既定の zoom_in と重なるので、どちらにも付かない
        assert_eq!(mouse_bound(&mouse, MouseGesture::WheelUp), None);
        assert_eq!(
            mouse.conflicts(),
            [MouseConflict {
                gesture: MouseGesture::WheelUp,
                owners: vec!["zoom_in", "opacity_up"],
            }]
        );
    }

    #[test]
    fn execute_runs_only_operations_that_can_execute() {
        fn never(_: &ScrapModel) -> bool {
            false
        }
        static BLOCKED: OperationDescriptor = OperationDescriptor {
            can_execute: never,
            checked: None,
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

    #[test]
    fn every_choice_and_folder_has_a_label() {
        let catalog =
            crate::i18n::Catalog::parse(include_str!("../../../../locales/ja.toml")).unwrap();
        for (key, _) in FOLDERS {
            assert_ne!(catalog.text(key), *key);
        }
        for op in OPERATIONS {
            let Some(choices) = op.choices else { continue };
            for choice in choices(&scrap(), &Appearance::default()) {
                if let ChoiceLabel::Key(key) = choice.label {
                    assert_ne!(catalog.text(key), key, "{}", op.id);
                }
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
            takes_values: false,
            in_recipe: InRecipe::Anywhere,
            in_menu: true,
            can_execute: normal,
            checked: None,
            choices: None,
            run: nothing,
        }
    }

    #[test]
    fn the_wheel_maps_to_a_gesture_only_without_modifiers_or_with_ctrl_alone() {
        let none = Modifiers::default();
        let ctrl = Modifiers { ctrl: true, ..none };
        assert_eq!(MouseGesture::wheel(none, 120), Some(MouseGesture::WheelUp));
        assert_eq!(MouseGesture::wheel(none, -1), Some(MouseGesture::WheelDown));
        assert_eq!(
            MouseGesture::wheel(ctrl, 1),
            Some(MouseGesture::CtrlWheelUp)
        );
        assert_eq!(
            MouseGesture::wheel(ctrl, -120),
            Some(MouseGesture::CtrlWheelDown)
        );
        assert_eq!(MouseGesture::wheel(none, 0), None);
        for other in [
            Modifiers {
                shift: true,
                ..none
            },
            Modifiers { alt: true, ..none },
            Modifiers { win: true, ..none },
            Modifiers {
                shift: true,
                ..ctrl
            },
        ] {
            assert_eq!(MouseGesture::wheel(other, 120), None, "{other:?}");
        }
    }

    #[test]
    fn the_binding_rows_follow_the_table_and_only_operations_without_arguments_take_a_shortcut() {
        let rows = binding_rows(OPERATIONS);
        let ids: Vec<&str> = rows.iter().map(|row| row.op.id).collect();
        // 引数のある操作（choices が Some）は行にしない。左ドラッグの移動は行にするがショートカットの欄はない
        assert_eq!(
            ids,
            [
                "copy",
                "save_default",
                "save_as",
                "save_scrap_file",
                "trim",
                "untrim",
                "annotate",
                "move_left",
                "move_right",
                "move_up",
                "move_down",
                "move_left_10",
                "move_right_10",
                "move_up_10",
                "move_down_10",
                "move",
                "zoom_in",
                "zoom_out",
                "zoom_actual",
                "opacity_up",
                "opacity_down",
                "reset_appearance",
                "toggle_compact",
                "close",
            ]
        );
        let row = |id: &str| rows.iter().find(|row| row.op.id == id).unwrap();
        assert_eq!(row("move").default_shortcut, None);
        assert_eq!(row("untrim").default_shortcut, Some(&[][..]));
        assert_eq!(
            row("copy").default_shortcut,
            Some(&["Ctrl+C".parse::<Chord>().unwrap()][..])
        );
        assert_eq!(row("zoom_in").default_shortcut.unwrap().len(), 3);
        assert_eq!(row("zoom_out").default_shortcut.unwrap().len(), 2);
    }

    #[test]
    fn left_drag_is_a_mouse_choice_only_for_move() {
        let rows = binding_rows(OPERATIONS);
        for row in &rows {
            let choices = row.mouse_choices();
            if row.op.id == "move" {
                assert_eq!(choices, [MouseGesture::LeftDrag]);
            } else {
                assert_eq!(
                    choices,
                    [
                        MouseGesture::WheelUp,
                        MouseGesture::WheelDown,
                        MouseGesture::CtrlWheelUp,
                        MouseGesture::CtrlWheelDown,
                        MouseGesture::MiddleClick,
                        MouseGesture::DoubleClick,
                    ],
                    "{}",
                    row.op.id
                );
            }
            // 表の既定値は候補に入っている
            if let Some(gesture) = row.op.default_mouse {
                assert!(choices.contains(&gesture), "{}", row.op.id);
            }
        }
    }

    #[test]
    fn every_mouse_gesture_has_a_label() {
        let catalog =
            crate::i18n::Catalog::parse(include_str!("../../../../locales/ja.toml")).unwrap();
        for (gesture, _) in MouseGesture::NAMES {
            let key = gesture.label_key();
            assert_ne!(catalog.text(key), key, "{gesture}");
        }
    }

    fn blank(colour: Rgb) -> ScrapModel {
        ScrapModel::blank(
            Point::new(0, 0),
            Size::new(40, 30),
            Background::Color(colour),
        )
    }

    fn choose(value: i32) -> OpContext {
        OpContext {
            choice: Some(value),
            ..OpContext::default()
        }
    }

    /// メニューの「背景色」の子メニュー。出ていなければ None。
    fn background_menu(model: &ScrapModel) -> Option<Submenu> {
        let shortcuts = Shortcuts::new(OPERATIONS, &ShortcutOverrides::new());
        menu(OPERATIONS, model, &shortcuts, &Appearance::default())
            .into_iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu(submenu) if submenu.id == "background" => Some(submenu),
                _ => None,
            })
    }

    #[test]
    fn only_a_blank_scrap_shows_the_background_in_its_menu() {
        assert!(background_menu(&blank(Rgb(0xFF, 0xFF, 0xFF))).is_some());
        assert_eq!(background_menu(&scrap()), None);
    }

    #[test]
    fn the_background_menu_lists_the_toolbar_colours_and_other_colours_checking_the_current_one() {
        let white = Rgb(0xFF, 0xFF, 0xFF);
        let submenu = background_menu(&blank(white)).unwrap();
        assert!(submenu.enabled);
        let values: Vec<i32> = submenu.choices.iter().map(|c| c.value).collect();
        assert_eq!(
            values,
            [
                0xE5_39_35, 0xFB_8C_00, 0xFD_D8_35, 0x43_A0_47, 0x1E_88_E5, 0x8E_24_AA, 0x21_21_21,
                0xFF_FF_FF, -1
            ]
        );
        let checked: Vec<i32> = submenu
            .choices
            .iter()
            .filter(|c| c.checked)
            .map(|c| c.value)
            .collect();
        assert_eq!(checked, [0xFF_FF_FF]);
        assert_eq!(
            submenu.choices.last().unwrap().label,
            ChoiceLabel::Key("scrap.background.other")
        );
        // 並びにない色なら「その他の色…」にチェックを付ける
        let custom = background_menu(&blank(Rgb(1, 2, 3))).unwrap();
        let checked: Vec<i32> = custom
            .choices
            .iter()
            .filter(|c| c.checked)
            .map(|c| c.value)
            .collect();
        assert_eq!(checked, [-1]);
    }

    #[test]
    fn choosing_a_background_colour_repaints_the_blank_scrap_with_it() {
        let mut model = blank(Rgb(0xFF, 0xFF, 0xFF));
        assert_eq!(
            execute(op("background"), &mut model, &choose(0x1E_88_E5)),
            vec![Effect::Redraw]
        );
        assert_eq!(
            model.background(),
            Some(Background::Color(Rgb(0x1E, 0x88, 0xE5)))
        );
        assert_eq!(
            model.blank_image().unwrap().pixel(0, 0),
            [0xE5, 0x88, 0x1E, 0xFF]
        );
        // 同じ色なら何もしない
        assert_eq!(
            execute(op("background"), &mut model, &choose(0x1E_88_E5)),
            vec![]
        );
    }

    #[test]
    fn other_colours_asks_for_a_colour_with_a_dialog() {
        let mut model = blank(Rgb(0xFF, 0xFF, 0xFF));
        let before = model.clone();
        assert_eq!(
            execute(op("background"), &mut model, &choose(-1)),
            vec![Effect::ShowBackgroundDialog]
        );
        assert_eq!(model, before);
    }

    #[test]
    fn the_background_of_a_captured_scrap_cannot_be_changed() {
        let mut model = scrap();
        let before = model.clone();
        assert!(!(op("background").can_execute)(&model));
        assert_eq!(
            execute(op("background"), &mut model, &choose(0xFF_00_00)),
            vec![]
        );
        assert_eq!(model, before);
    }

    #[test]
    fn the_background_cannot_be_changed_while_folded_or_editing() {
        let mut model = blank(Rgb(0xFF, 0xFF, 0xFF));
        model.enter_compact(ZoomAnchor::Center);
        assert!(!(op("background").can_execute)(&model));
        model.leave_compact();
        execute(op("annotate"), &mut model, &editing_context());
        assert!(!(op("background").can_execute)(&model));
    }

    #[test]
    fn resetting_the_appearance_keeps_the_background() {
        let defaults = Appearance::default();
        let mut model = blank(Rgb(0xFF, 0xFF, 0xFF));
        execute(op("background"), &mut model, &choose(0x21_21_21));
        execute(op("shadow"), &mut model, &choose(0));
        execute(op("reset_appearance"), &mut model, &with_defaults(defaults));
        assert_eq!(model.appearance(&defaults), defaults);
        assert_eq!(
            model.background(),
            Some(Background::Color(Rgb(0x21, 0x21, 0x21)))
        );
    }

    #[test]
    fn the_background_takes_no_shortcut_and_has_no_row_of_its_own() {
        let background = op("background");
        assert_eq!(background.default_shortcut, &[]);
        assert!(!background.takes_binding());
        assert!(
            !binding_rows(OPERATIONS)
                .iter()
                .any(|row| row.op.id == "background")
        );
    }

    #[test]
    fn every_background_choice_has_a_label() {
        let catalog =
            crate::i18n::Catalog::parse(include_str!("../../../../locales/ja.toml")).unwrap();
        for choice in background_menu(&blank(Rgb(0xFF, 0xFF, 0xFF)))
            .unwrap()
            .choices
        {
            let ChoiceLabel::Key(key) = choice.label else {
                panic!("{choice:?}");
            };
            assert_ne!(catalog.text(key), key);
        }
    }
}
