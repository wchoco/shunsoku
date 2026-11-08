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
//! - 「定型領域」タブの一覧の追加・複製・削除・並べ替えは `add_region` / `duplicate_region` / `remove_region` /
//!   `move_region`。「範囲を選び直す」は `set_region_range`。GUI を開いている間にトレイから登録した定型領域は
//!   `new_registered_region` で名前を決め、書いたら `registered` で base と写しに足す。詳細の欄は `draft_mut` で変える（名前の空・重なりは `mark` の `RegionName` に出る）。
//! - 「レシピ」タブの一覧の追加・削除は `add_recipe` / `remove_recipe`、名前は `set_recipe_name`。操作の並びは
//!   `step_choices`（足せる操作と、見た目・背景色を指定する操作）/ `add_step` / `move_step` / `remove_step`。選んでいる操作に
//!   出すパラメータの欄の種類は `param_fields`（パラメータのない操作は None）。足したら、`add_step` が返す位置の
//!   操作を選ぶ。見た目を指定する操作の
//!   欄（`AppearanceItem`）は `appearance` / `set_appearance` で、背景色を指定する操作の色は `background` / `set_background` で読み書きする（「変えない」は書かない）。ショートカットは
//!   `set_chord` の `RecipeShortcut`、キャプチャで始まるレシピのホットキーは `RecipeHotkey`（どちらの欄を出すかは
//!   `key_item`。最初の操作がキャプチャ系かどうかが変わったら、両方のキーを消す）。検査（`recipe::plan`）で引っかかったもの（欄に打った範囲外の値を含む）は `mark` の
//!   `RecipeName` / `RecipeSteps` に出る。操作の並びの行の文言は `step_label`（パラメータを添えた操作はパラメータの要約）。

use std::collections::BTreeSet;

use super::{Problem, Region, Settings, SettingsItem, new_region};
use crate::geometry::Rect;
use crate::keys::Chord;
pub use crate::recipe::AppearanceItem;
pub use crate::recipe::Capture;
use crate::recipe::{
    self, AppearanceStep, BackgroundStep, CaptureRegionStep, ItemValue, Recipe, Step, StepKey,
    Written,
};
use crate::scrap::Background;
use crate::scrap::ops::{BindingRow, InRecipe, MouseGesture, OPERATIONS, OperationId};

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
            Mark::Problem(Problem::NoSteps) => "settings_gui.mark.no_steps",
            Mark::Problem(Problem::NotAStep) => "settings_gui.mark.not_a_step",
            Mark::Problem(Problem::LastOnlyNotLast) => "settings_gui.mark.last_only_not_last",
            Mark::Problem(Problem::FirstOnlyNotFirst) => "settings_gui.mark.first_only_not_first",
            Mark::Problem(Problem::ShortcutOnCapture) => "settings_gui.mark.shortcut_on_capture",
            Mark::Problem(Problem::HotkeyWithoutCapture) => {
                "settings_gui.mark.hotkey_without_capture"
            }
            Mark::Problem(Problem::OutOfRange) => "settings_gui.mark.out_of_range",
            Mark::Problem(Problem::BadValue) => "settings_gui.mark.bad_value",
            Mark::Problem(Problem::UnknownKey) => "settings_gui.mark.unknown_key",
            Mark::Problem(Problem::NotAppearance) => "settings_gui.mark.not_appearance",
            Mark::Problem(Problem::NoValues) => "settings_gui.mark.no_values",
            Mark::Problem(Problem::BorderOffWithColorOrWidth) => {
                "settings_gui.mark.border_off_with_color_or_width"
            }
            Mark::Problem(Problem::NoSuchRegion) => "settings_gui.mark.no_such_region",
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
    /// なければ何もしない。レシピのショートカットかホットキーを入れたら、もう片方（設定 GUI に出ていない欄。
    /// 設定ファイルに両方を書いたときだけ値がある）を消す。
    pub fn set_chord(&mut self, item: &SettingsItem, chord: Option<Chord>) {
        if let Some(slot) = chord_slot(&mut self.draft, item) {
            *slot = chord;
        }
        let other = match *item {
            SettingsItem::RecipeShortcut(i) => self.draft.recipes.get_mut(i).map(|r| &mut r.hotkey),
            SettingsItem::RecipeHotkey(i) => self.draft.recipes.get_mut(i).map(|r| &mut r.shortcut),
            _ => None,
        };
        if let Some(other) = other {
            *other = None;
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

    /// 定型領域を末尾に足す（「追加」）。名前は `"{base} N"`（`new_region`）。足した位置を返す。
    pub fn add_region(&mut self, base: &str, monitor: &str, rect: Rect) -> usize {
        let region = new_region(&self.draft, base, monitor, rect);
        self.draft.regions.push(region);
        self.forget_region_marks();
        self.draft.regions.len() - 1
    }

    /// 定型領域 `index` のモニタと範囲（モニタ相対）を置き換える（「範囲を選び直す」）。ほかの項目と、ファイルの
    /// 要素との結び付き（`loaded_name`）はそのまま。`index` の定型領域がなければ何もしない。
    pub fn set_region_range(&mut self, index: usize, monitor: &str, rect: Rect) {
        if let Some(region) = self.draft.regions.get_mut(index) {
            region.monitor = monitor.to_owned();
            region.rect = rect;
        }
    }

    /// GUI を開いている間に、トレイの「定型領域を追加…」で登録する定型領域を作る（`new_region`）。名前は、適用済みの
    /// 設定 `applied` と写しのどちらの定型領域とも重ならない `"{base} N"`。
    pub fn new_registered_region(
        &self,
        applied: &Settings,
        base: &str,
        monitor: &str,
        rect: Rect,
    ) -> Region {
        let mut taken = applied.clone();
        taken.regions.extend(self.draft.regions.iter().cloned());
        // 保存先とファイル名テンプレートの初期値は、適用済みの全体設定（`taken` の general）
        new_region(&taken, base, monitor, rect)
    }

    /// トレイの「定型領域を追加…」でファイルに書いた定型領域 `region` を、base と写しの末尾に足す（ファイルには
    /// もうあるので、適用で足し直さない）。写しでの位置を返す。
    pub fn registered(&mut self, region: Region) -> usize {
        self.base.regions.push(region.clone());
        self.draft.regions.push(region);
        self.draft.regions.len() - 1
    }

    /// 定型領域 `index` を写して、その後ろに入れる（「複製」）。名前は `name`（ほかと重なれば `"{name} N"`、
    /// N は 2 から）。ホットキーは写さない（重なって適用できなくなるため）。入れた位置を返す。
    pub fn duplicate_region(&mut self, index: usize, name: &str) -> usize {
        let used = |n: &str| self.draft.regions.iter().any(|r| r.name == n);
        let name = std::iter::once(name.to_owned())
            .chain((2..).map(|n: u64| format!("{name} {n}")))
            .find(|n| !used(n))
            .expect("some number is unused");
        let copy = Region {
            name,
            hotkey: None,
            loaded_name: None,
            ..self.draft.regions[index].clone()
        };
        self.draft.regions.insert(index + 1, copy);
        self.forget_region_marks();
        index + 1
    }

    /// 写しの定型領域 `index` の、撮ったものの出し方の欄（保存先・ファイル名テンプレート・保存する・スクラップに
    /// する）を使えるか。白紙スクラップにする定型領域は撮らず何も保存しないので、使えない。`index` の定型領域が
    /// なければ false。
    pub fn region_uses_capture_outputs(&self, index: usize) -> bool {
        self.draft
            .regions
            .get(index)
            .is_some_and(|region| !region.blank_scrap)
    }

    /// 定型領域 `index` を消す（「削除」）。次に選ぶ位置（同じ位置、末尾を消したらその前。なくなれば None）を返す。
    pub fn remove_region(&mut self, index: usize) -> Option<usize> {
        self.draft.regions.remove(index);
        self.forget_region_marks();
        let len = self.draft.regions.len();
        (len > 0).then(|| index.min(len - 1))
    }

    /// 定型領域 `index` を隣と入れ替える（「上へ」「下へ」）。端なら動かさない。移った位置を返す。
    pub fn move_region(&mut self, index: usize, to: Move) -> usize {
        let other = match to {
            Move::Up => index.checked_sub(1),
            Move::Down => Some(index + 1).filter(|&i| i < self.draft.regions.len()),
        };
        let Some(other) = other else {
            return index;
        };
        self.draft.regions.swap(index, other);
        self.forget_region_marks();
        other
    }

    /// 定型領域の位置が変わったので、位置で覚えた「登録できなかった」印を忘れる。
    fn forget_region_marks(&mut self) {
        self.not_registered
            .retain(|item| !matches!(item, SettingsItem::RegionHotkey(_)));
    }

    /// レシピを末尾に足す（「追加」）。名前はどのレシピとも重ならない `"{base} N"`（N は 1 から）。操作の並びと
    /// ショートカットは空。足した位置を返す。
    pub fn add_recipe(&mut self, base: &str) -> usize {
        let used = |n: &str| self.draft.recipes.iter().any(|r| r.name == n);
        let name = (1..)
            .map(|n: u64| format!("{base} {n}"))
            .find(|n| !used(n))
            .expect("some number is unused");
        self.draft.recipes.push(Recipe {
            name,
            ..Recipe::default()
        });
        self.draft.recipes.len() - 1
    }

    /// レシピ `index` を消す（「削除」）。次に選ぶ位置（同じ位置、末尾を消したらその前。なくなれば None）を返す。
    /// `index` のレシピがなければ何もせず None。
    pub fn remove_recipe(&mut self, index: usize) -> Option<usize> {
        if index >= self.draft.recipes.len() {
            return None;
        }
        self.draft.recipes.remove(index);
        // ホットキーは位置で指すので、登録できなかった印は位置がずれたら当たらない
        self.not_registered
            .retain(|item| !matches!(item, SettingsItem::RecipeHotkey(_)));
        let len = self.draft.recipes.len();
        (len > 0).then(|| index.min(len - 1))
    }

    /// レシピ `index` の名前を `name`（前後の空白を除く）にする。`index` のレシピがなければ何もしない。
    pub fn set_recipe_name(&mut self, index: usize, name: &str) {
        if let Some(r) = self.draft.recipes.get_mut(index) {
            r.name = name.trim().to_owned();
        }
    }

    /// レシピ `index` の「操作を足す」で選べるもの。最初にキャプチャ系（`Capture::ALL` の順）、次にパラメータの
    /// ない操作（表の順）、最後に見た目を指定する操作と背景色を指定する操作。
    /// パラメータのない操作は、レシピに入れられる操作（`recipe::can_be_step`）。最後にだけ置ける操作（`close`、
    /// `annotate`）で終わるレシピには最後にだけ置ける操作を出さない（`add_step` はその前に足すので、2 つ目は最後に
    /// 置けない）。`index` のレシピがなければ空。
    pub fn step_choices(&self, index: usize) -> Vec<StepChoice> {
        let Some(r) = self.draft.recipes.get(index) else {
            return Vec::new();
        };
        let ends_with_last_only = r
            .steps
            .last()
            .is_some_and(|s| recipe::is_last_only(s.op_id(), OPERATIONS));
        // キャプチャ系は、操作の位置にかかわらず出す（最初以外に足せば `FirstOnlyNotFirst` の印が出る）
        let captures = Capture::ALL.into_iter().map(StepChoice::Capture);
        captures
            .chain(
                OPERATIONS
                    .iter()
                    .filter(|op| recipe::can_be_step(op))
                    .filter(|op| !(ends_with_last_only && op.in_recipe == InRecipe::Last))
                    .map(|op| StepChoice::Op(op.id)),
            )
            .chain([StepChoice::Appearance, StepChoice::Background])
            .collect()
    }

    /// レシピ `index` に `choice` を足す。見た目を指定する操作は、全部の項目が「変えない」、背景色を指定する操作は
    /// 色を選んでいない（どちらもそのままでは `NoValues`）。レシピが最後にだけ置ける操作（`close`、`annotate`）で終わっていれば、その前に入れる。入れた
    /// 位置を返す。`index` のレシピがなければ何もせず None。
    pub fn add_step(&mut self, index: usize, choice: impl Into<StepChoice>) -> Option<usize> {
        let first = self.starts_with_capture(index);
        let steps = &mut self.draft.recipes.get_mut(index)?.steps;
        let at = if steps
            .last()
            .is_some_and(|s| recipe::is_last_only(s.op_id(), OPERATIONS))
        {
            steps.len() - 1
        } else {
            steps.len()
        };
        let step = match choice.into() {
            StepChoice::Op(id) => id.into(),
            StepChoice::Capture(Capture::Region) => {
                Step::CaptureRegion(CaptureRegionStep::default())
            }
            StepChoice::Capture(c) => c.id().into(),
            StepChoice::Appearance => Step::Appearance(AppearanceStep::default()),
            StepChoice::Background => Step::Background(BackgroundStep::default()),
        };
        steps.insert(at, step);
        self.first_step_changed(index, first);
        Some(at)
    }

    /// レシピ `index` の最初の操作がキャプチャ系か。
    fn starts_with_capture(&self, index: usize) -> bool {
        self.draft
            .recipes
            .get(index)
            .and_then(|r| r.steps.first())
            .is_some_and(|s| recipe::is_first_only(s.op_id()))
    }

    /// 操作の並びを変えたあと、最初の操作がキャプチャ系かどうか（変える前は `before`）が変わったら、
    /// ショートカットとホットキーを消す（欄が入れ替わるため）。
    fn first_step_changed(&mut self, index: usize, before: bool) {
        if self.starts_with_capture(index) != before
            && let Some(r) = self.draft.recipes.get_mut(index)
        {
            r.shortcut = None;
            r.hotkey = None;
        }
    }

    /// レシピ `index` のキーの欄（ショートカットの欄の場所に出す欄）。最初の操作がキャプチャ系なら
    /// `RecipeHotkey`、ほかは `RecipeShortcut`。`index` のレシピがなければ None。
    pub fn key_item(&self, index: usize) -> Option<SettingsItem> {
        self.draft.recipes.get(index)?;
        Some(if self.starts_with_capture(index) {
            SettingsItem::RecipeHotkey(index)
        } else {
            SettingsItem::RecipeShortcut(index)
        })
    }

    /// レシピ `index` の操作 `step` を隣と入れ替える（「上へ」「下へ」）。端か、操作がなければ動かさない。移った
    /// 位置を返す。
    pub fn move_step(&mut self, index: usize, step: usize, to: Move) -> usize {
        let first = self.starts_with_capture(index);
        let Some(r) = self.draft.recipes.get_mut(index) else {
            return step;
        };
        let other = match to {
            Move::Up => step.checked_sub(1),
            Move::Down => Some(step + 1),
        };
        match other.filter(|&o| o < r.steps.len() && step < r.steps.len()) {
            Some(other) => {
                r.steps.swap(step, other);
                self.first_step_changed(index, first);
                other
            }
            None => step,
        }
    }

    /// レシピ `index` の操作 `step` を消す。次に選ぶ位置（同じ位置、末尾を消したらその前。なくなれば None）を
    /// 返す。操作がなければ何もせず None。
    pub fn remove_step(&mut self, index: usize, step: usize) -> Option<usize> {
        let first = self.starts_with_capture(index);
        let steps = &mut self.draft.recipes.get_mut(index)?.steps;
        if step >= steps.len() {
            return None;
        }
        steps.remove(step);
        let len = steps.len();
        self.first_step_changed(index, first);
        (len > 0).then(|| step.min(len - 1))
    }

    /// レシピ `index` の操作 `step` を選んでいる間に出すパラメータの欄。パラメータの欄のない操作か、操作が
    /// なければ None（欄を出さない）。
    pub fn param_fields(&self, index: usize, step: usize) -> Option<ParamFields> {
        match self.draft.recipes.get(index)?.steps.get(step)? {
            Step::Appearance(_) => Some(ParamFields::Appearance),
            Step::Background(_) => Some(ParamFields::Background),
            Step::CaptureRegion(_) => Some(ParamFields::Region),
            Step::Op(_) => None,
        }
    }

    /// レシピ `index` の操作 `step` が定型領域キャプチャなら、選べる定型領域の名前（写しの定型領域の順）と、選んで
    /// いる位置（選んでいないか、名前が読めなければ None）。写しにない名前を選んでいれば、それも末尾に足して選んで
    /// おく（表示だけ）。定型領域キャプチャでなければ None。
    pub fn capture_region(&self, index: usize, step: usize) -> Option<RegionChoices> {
        let Step::CaptureRegion(step) = self.draft.recipes.get(index)?.steps.get(step)? else {
            return None;
        };
        let mut names: Vec<String> = self.draft.regions.iter().map(|r| r.name.clone()).collect();
        let selected = step.region.as_ref().map(|chosen| {
            names.iter().position(|n| n == chosen).unwrap_or_else(|| {
                names.push(chosen.clone());
                names.len() - 1
            })
        });
        Some(RegionChoices { names, selected })
    }

    /// レシピ `index` の操作 `step`（定型領域キャプチャ）で撮る定型領域を、名前 `name` の定型領域にする。設定
    /// ファイルから読めなかった名前は消す。定型領域キャプチャでなければ何もしない。
    pub fn set_capture_region(&mut self, index: usize, step: usize, name: &str) {
        let Some(Step::CaptureRegion(step)) = self
            .draft
            .recipes
            .get_mut(index)
            .and_then(|r| r.steps.get_mut(step))
        else {
            return;
        };
        step.rejected.retain(|r| r.key != StepKey::Region);
        step.region = Some(name.to_owned());
    }

    /// レシピ `index` の操作 `step` が背景色を指定する操作なら、その色（選んでいないか、設定ファイルに書いた色が
    /// 読めなければ None）。背景色を指定する操作でなければ None。
    pub fn background(&self, index: usize, step: usize) -> Option<Option<Background>> {
        match self.draft.recipes.get(index)?.steps.get(step)? {
            Step::Background(step) => Some(step.color),
            Step::Op(_) | Step::Appearance(_) | Step::CaptureRegion(_) => None,
        }
    }

    /// レシピ `index` の操作 `step`（背景色を指定する操作）の色を `color` にする（「選ぶ…」で選んだ色か、「透明」の
    /// チェックで `Transparent`。チェックを外したら None で、色を選ぶまで `NoValues`）。設定ファイルから読めなかった
    /// 色は消す。背景色を指定する操作でなければ何もしない。
    pub fn set_background(&mut self, index: usize, step: usize, color: Option<Background>) {
        let Some(Step::Background(step)) = self
            .draft
            .recipes
            .get_mut(index)
            .and_then(|r| r.steps.get_mut(step))
        else {
            return;
        };
        step.rejected.retain(|r| r.key != StepKey::Color);
        step.color = color;
    }

    /// レシピ `index` の操作 `step` が見た目を指定する操作なら、その項目 `item` の欄に出す値。「変えない」は
    /// `Switch(None)` か空の `Text`。読めなかった値（範囲外など）は書いたまま出す。見た目を指定する操作でなければ None。
    pub fn appearance(
        &self,
        index: usize,
        step: usize,
        item: AppearanceItem,
    ) -> Option<AppearanceInput> {
        let Some(Step::Appearance(step)) = self.draft.recipes.get(index)?.steps.get(step) else {
            return None;
        };
        let text = match step.values.get(item) {
            Some(ItemValue::Switch(on)) => return Some(AppearanceInput::Switch(Some(on))),
            None if item.is_switch() => return Some(AppearanceInput::Switch(None)),
            Some(ItemValue::Color(color)) => Some(color.to_string()),
            Some(ItemValue::Number(n)) => Some(n.to_string()),
            None => None,
        };
        let rejected = || {
            step.rejected
                .iter()
                .find(|r| r.key == StepKey::Item(item))
                .and_then(|r| r.value.as_deref())
                .map(unquoted)
        };
        Some(AppearanceInput::Text(
            text.or_else(rejected).unwrap_or_default(),
        ))
    }

    /// レシピ `index` の操作 `step`（見た目を指定する操作）の項目 `item` を、欄の入力 `input` にする。「変えない」
    /// （`Switch(None)`、空白だけの `Text`）は書かない。読めない文字（範囲外の数、`#RRGGBB` でない色）は書いたまま
    /// 残し、設定ファイルから読めなかった値と同じくレシピの問題にする（適用できない）。項目を書き直したら、その
    /// 項目の読めなかった値は消す。見た目を指定する操作でないか、欄と違う種類の入力なら何もしない。
    pub fn set_appearance(
        &mut self,
        index: usize,
        step: usize,
        item: AppearanceItem,
        input: AppearanceInput,
    ) {
        let Some(Step::Appearance(step)) = self
            .draft
            .recipes
            .get_mut(index)
            .and_then(|r| r.steps.get_mut(step))
        else {
            return;
        };
        if item.is_switch() != matches!(input, AppearanceInput::Switch(_)) {
            return;
        }
        step.rejected.retain(|r| r.key != StepKey::Item(item));
        let written = match &input {
            AppearanceInput::Switch(on) => on.map(Written::Bool),
            AppearanceInput::Text(s) => {
                let s = s.trim();
                // 数の欄の文字は、設定ファイルの整数と同じに読む（整数でなければ型の誤り）
                match s.parse() {
                    _ if s.is_empty() => None,
                    Ok(n) if item.number_range().is_some() => Some(Written::Integer(n)),
                    _ => Some(Written::Text(s)),
                }
            }
        };
        let Some(written) = written else {
            step.values.clear(item);
            return;
        };
        if let Err(reason) = step.values.set(item, written) {
            // 書いたまま残す
            step.rejected.push(recipe::Rejected {
                key: StepKey::Item(item),
                value: written.toml(),
                reason,
            });
        }
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

/// 「レシピ」タブの操作の一覧の行の文言。パラメータのない操作は操作の名前（表にない id はそのまま）、パラメータを添えた操作は
/// 「見た目: 枠 #FF0000 3px」のように、書いた項目を決まった順（透明度、枠、枠の太さ、影）に並べた要約。`text` は文言のキーから
/// 文言を引く。
pub fn step_label(step: &Step, text: impl Fn(&'static str) -> &'static str) -> String {
    let values = match step {
        Step::Op(id) => {
            if let Some(c) = Capture::from_id(id) {
                return text(c.label_key()).to_owned();
            }
            return OPERATIONS
                .iter()
                .find(|op| op.id == id)
                .map_or_else(|| id.clone(), |op| text(op.label_key).to_owned());
        }
        Step::Appearance(step) => step.values,
        Step::CaptureRegion(step) => {
            let label = text(Capture::Region.label_key());
            return match &step.region {
                Some(name) => text("settings_gui.recipes.step.capture_region")
                    .replace("{capture}", label)
                    .replace("{region}", name),
                None => label.to_owned(),
            };
        }
        Step::Background(step) => {
            return match step.color {
                Some(Background::Transparent) => text("settings_gui.recipes.step.background")
                    .replace("{color}", text("scrap.background.transparent")),
                Some(color) => text("settings_gui.recipes.step.background")
                    .replace("{color}", &color.to_string()),
                None => text("scrap.op.background").to_owned(),
            };
        }
    };
    let mut items: Vec<String> = Vec::new();
    if let Some(opacity) = values.opacity {
        items.push(
            text("settings_gui.recipes.step.opacity").replace("{percent}", &opacity.to_string()),
        );
    }
    match (values.border, values.border_color, values.border_width) {
        (_, Some(color), _) => items.push(
            text("settings_gui.recipes.step.border_color").replace("{color}", &color.to_string()),
        ),
        (Some(false), None, _) => {
            items.push(text("settings_gui.recipes.step.border_off").to_owned());
        }
        (Some(true), None, _) | (None, None, Some(_)) => {
            items.push(text("settings_gui.recipes.step.border_on").to_owned());
        }
        (None, None, None) => {}
    }
    if let Some(width) = values.border_width {
        items.push(
            text("settings_gui.recipes.step.border_width").replace("{width}", &width.to_string()),
        );
    }
    match values.shadow {
        Some(true) => items.push(text("settings_gui.recipes.step.shadow_on").to_owned()),
        Some(false) => items.push(text("settings_gui.recipes.step.shadow_off").to_owned()),
        None => {}
    }
    if items.is_empty() {
        return text("scrap.op.set_appearance").to_owned();
    }
    text("settings_gui.recipes.step.appearance").replace("{items}", &items.join(" "))
}

impl AppearanceItem {
    /// 欄の見出しの文言のキー。
    pub fn label_key(self) -> &'static str {
        match self {
            AppearanceItem::Border => "settings_gui.recipes.appearance.border",
            AppearanceItem::BorderColor => "settings_gui.recipes.appearance.border_color",
            AppearanceItem::BorderWidth => "settings_gui.recipes.appearance.border_width",
            AppearanceItem::Opacity => "settings_gui.recipes.appearance.opacity",
            AppearanceItem::Shadow => "settings_gui.recipes.appearance.shadow",
        }
    }

    /// 数の欄なら、その範囲（上下のボタンに付ける）。
    pub fn number_range(self) -> Option<(u32, u32)> {
        match self {
            AppearanceItem::BorderWidth => Some((
                *super::BORDER_WIDTH_RANGE.start(),
                *super::BORDER_WIDTH_RANGE.end(),
            )),
            AppearanceItem::Opacity => Some((
                u32::from(*super::OPACITY_RANGE.start()),
                u32::from(*super::OPACITY_RANGE.end()),
            )),
            AppearanceItem::Border | AppearanceItem::BorderColor | AppearanceItem::Shadow => None,
        }
    }
}

/// 見た目を指定する操作の欄の値（`Editor::appearance` / `set_appearance`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppearanceInput {
    /// オン・オフの欄（枠・影）。None は「変えない」。
    Switch(Option<bool>),
    /// 文字の欄（色・太さ・透明度）。空は「変えない」。
    Text(String),
}

/// オン・オフの欄の選択肢（ドロップダウンの順）と文言のキー。先頭の「変えない」が既定。
pub const SWITCH_CHOICES: [(Option<bool>, &str); 3] = [
    (None, UNCHANGED_KEY),
    (Some(true), "settings_gui.recipes.appearance.on"),
    (Some(false), "settings_gui.recipes.appearance.off"),
];

/// 「変えない」の文言のキー。文字の欄では、空のときに薄く出す。
pub const UNCHANGED_KEY: &str = "settings_gui.recipes.appearance.unchanged";

/// 読めなかった値（TOML の書き方）を欄に出す文字にする。文字列なら中身、ほかはそのまま。
fn unquoted(written: &str) -> String {
    match written.parse::<toml_edit::Value>() {
        Ok(toml_edit::Value::String(s)) => s.value().clone(),
        _ => written.to_owned(),
    }
}

/// レシピのタブで、選んでいる操作に添えて出すパラメータの欄の種類（`Editor::param_fields`）。Win32 層は、
/// 種類ごとの欄のまとまりを、選んでいる操作の種類のときだけ出す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamFields {
    /// 見た目（透明度・枠・影。`Editor::appearance` / `set_appearance`）。
    Appearance,
    /// 背景色（「選ぶ…」と「透明」。`Editor::background` / `set_background`）。
    Background,
    /// 定型領域キャプチャの定型領域（ドロップダウン。`Editor::capture_region` / `set_capture_region`）。
    Region,
}

/// 定型領域キャプチャの欄のドロップダウン（`Editor::capture_region`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionChoices {
    /// 定型領域の名前（ドロップダウンの順）。
    pub names: Vec<String>,
    /// 選んでいる位置。選んでいなければ None。
    pub selected: Option<usize>,
}

/// 「操作を足す」で選べるもの（`Editor::step_choices`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepChoice {
    /// 値のない操作。
    Op(OperationId),
    /// 見た目を指定する操作（パラメータは足してから欄で入れる）。
    Appearance,
    /// 背景色を指定する操作（色は足してから欄で選ぶ）。
    Background,
    /// キャプチャ系の操作（ADR-0010。最初にだけ置ける）。
    Capture(Capture),
}

impl StepChoice {
    /// ドロップダウンに出す文言のキー（表にない id は None）。
    pub fn label_key(self) -> Option<&'static str> {
        let id = match self {
            StepChoice::Op(id) => id,
            StepChoice::Appearance => recipe::APPEARANCE,
            StepChoice::Background => recipe::BACKGROUND,
            StepChoice::Capture(c) => return Some(c.label_key()),
        };
        OPERATIONS
            .iter()
            .find(|op| op.id == id)
            .map(|op| op.label_key)
    }
}

impl From<OperationId> for StepChoice {
    fn from(id: OperationId) -> StepChoice {
        StepChoice::Op(id)
    }
}

/// 定型領域を動かす向き（`Editor::move_region`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
}

/// ドロップダウンの項目と、選んでおく位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices<T> {
    pub items: Vec<T>,
    pub selected: usize,
}

/// 候補 `candidates`（昇順）から選ぶドロップダウンに、今の値 `current` を出す。設定ファイルに候補の外の値が
/// 書かれていれば、それも順の位置に足して選んでおく（表示だけ。ほかを選べばその値になる）。
pub fn choices<T: Ord + Clone>(candidates: &[T], current: &T) -> Choices<T> {
    let mut items = candidates.to_vec();
    let selected = match items.binary_search(current) {
        Ok(i) => i,
        Err(i) => {
            items.insert(i, current.clone());
            i
        }
    };
    Choices { items, selected }
}

/// 設定 `settings` の、キーの組み合わせの項目 `item` の値。
fn chord_of(settings: &Settings, item: &SettingsItem) -> Option<Chord> {
    match item {
        SettingsItem::SelectCaptureHotkey => settings.hotkeys.select_capture,
        SettingsItem::RestoreLastHotkey => settings.hotkeys.restore_last,
        SettingsItem::ActiveWindowCaptureHotkey => settings.hotkeys.active_window_capture,
        SettingsItem::BlankScrapHotkey => settings.hotkeys.blank_scrap,
        SettingsItem::RegionHotkey(i) => settings.regions.get(*i).and_then(|r| r.hotkey),
        SettingsItem::Shortcut(id) => settings.shortcuts.get(id).copied().flatten(),
        SettingsItem::RecipeShortcut(i) => settings.recipes.get(*i).and_then(|r| r.shortcut),
        SettingsItem::RecipeHotkey(i) => settings.recipes.get(*i).and_then(|r| r.hotkey),
        SettingsItem::Mouse(_)
        | SettingsItem::RegionName(_)
        | SettingsItem::RecipeName(_)
        | SettingsItem::RecipeSteps(_) => None,
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
        SettingsItem::BlankScrapHotkey => Some(&mut settings.hotkeys.blank_scrap),
        SettingsItem::RegionHotkey(i) => settings.regions.get_mut(*i).map(|r| &mut r.hotkey),
        SettingsItem::Shortcut(id) => Some(settings.shortcuts.entry(id.clone()).or_default()),
        SettingsItem::RecipeShortcut(i) => settings.recipes.get_mut(*i).map(|r| &mut r.shortcut),
        SettingsItem::RecipeHotkey(i) => settings.recipes.get_mut(*i).map(|r| &mut r.hotkey),
        SettingsItem::Mouse(_)
        | SettingsItem::RegionName(_)
        | SettingsItem::RecipeName(_)
        | SettingsItem::RecipeSteps(_) => None,
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
    fn a_value_among_the_candidates_is_selected_in_place() {
        assert_eq!(
            choices(&[2, 3, 5, 8], &5),
            Choices {
                items: vec![2, 3, 5, 8],
                selected: 2
            }
        );
    }

    #[test]
    fn a_value_outside_the_candidates_is_shown_in_order_and_selected() {
        assert_eq!(
            choices(&[12, 16, 24, 32], &20),
            Choices {
                items: vec![12, 16, 20, 24, 32],
                selected: 2
            }
        );
        assert_eq!(choices(&[2, 3, 5, 8], &64).selected, 4);
        let fonts = ["Meiryo".to_owned(), "Yu Gothic UI".to_owned()];
        assert_eq!(
            choices(&fonts, &"Not Installed".to_owned()).items,
            ["Meiryo", "Not Installed", "Yu Gothic UI"]
        );
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
            Mark::Problem(Problem::NoSteps),
            Mark::Problem(Problem::NotAStep),
            Mark::Problem(Problem::LastOnlyNotLast),
            Mark::Problem(Problem::OutOfRange),
            Mark::Problem(Problem::BadValue),
            Mark::Problem(Problem::UnknownKey),
            Mark::Problem(Problem::NotAppearance),
            Mark::Problem(Problem::NoValues),
            Mark::Problem(Problem::BorderOffWithColorOrWidth),
            Mark::Problem(Problem::NoSuchRegion),
        ] {
            let key = mark.label_key();
            assert_ne!(catalog.text(key), key, "{mark:?}");
        }
    }

    #[test]
    fn every_appearance_field_and_choice_has_a_text_in_the_bundled_locale() {
        let catalog =
            crate::i18n::Catalog::parse(include_str!("../../../../locales/ja.toml")).unwrap();
        let keys = AppearanceItem::ALL
            .map(AppearanceItem::label_key)
            .into_iter()
            .chain(SWITCH_CHOICES.map(|(_, key)| key))
            .chain([UNCHANGED_KEY])
            .chain(StepChoice::Appearance.label_key())
            .chain(StepChoice::Background.label_key());
        for key in keys {
            assert_ne!(catalog.text(key), key);
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

    /// 範囲選択キャプチャのホットキーが入った（適用を止める問題のない）設定に、定型領域 `names` を足したもの。
    fn with_regions(names: &[&str]) -> Settings {
        let mut s = Settings::default();
        s.hotkeys.select_capture = Some(chord("Ctrl+Shift+A"));
        for name in names {
            let region = crate::settings::new_region(&s, "x", "M1", Rect::new(0, 0, 10, 10));
            s.regions.push(crate::settings::Region {
                name: (*name).to_owned(),
                loaded_name: Some((*name).to_owned()),
                ..region
            });
        }
        s
    }

    fn region_names(editor: &Editor) -> Vec<&str> {
        editor
            .draft()
            .regions
            .iter()
            .map(|r| r.name.as_str())
            .collect()
    }

    #[test]
    fn an_added_region_gets_the_next_free_number_and_goes_last() {
        let mut editor = Editor::new(with_regions(&["a", "領域 1"]));

        let i = editor.add_region("領域", "M2", Rect::new(0, 0, 640, 480));

        assert_eq!(i, 2);
        assert_eq!(region_names(&editor), ["a", "領域 1", "領域 2"]);
        let added = &editor.draft().regions[2];
        assert_eq!(
            (added.monitor.as_str(), added.rect),
            ("M2", Rect::new(0, 0, 640, 480))
        );
        assert_eq!(added.loaded_name, None);
        assert!(editor.buttons().apply);
        assert_eq!(editor.base().regions.len(), 2);
    }

    #[test]
    fn a_reselected_range_replaces_only_the_monitor_and_range_in_the_copy() {
        let mut settings = with_regions(&["a", "b"]);
        settings.regions[1].hotkey = Some(chord("Ctrl+Alt+2"));
        let mut editor = Editor::new(settings.clone());

        editor.set_region_range(1, "M2", Rect::new(5, 6, 105, 206));

        let b = &editor.draft().regions[1];
        assert_eq!(
            (b.name.as_str(), b.monitor.as_str(), b.rect, b.hotkey),
            (
                "b",
                "M2",
                Rect::new(5, 6, 105, 206),
                Some(chord("Ctrl+Alt+2"))
            )
        );
        // ファイルの要素との結び付きは残す（適用で同じ [[regions]] の値を書き換える）
        assert_eq!(b.loaded_name.as_deref(), Some("b"));
        assert_eq!(editor.draft().regions[0], settings.regions[0]);
        // ファイルに書くのは適用のとき
        assert_eq!(editor.base(), &settings);
        assert!(editor.buttons().apply);
    }

    #[test]
    fn reselecting_a_region_that_is_gone_changes_nothing() {
        let mut editor = Editor::new(with_regions(&["a"]));

        editor.set_region_range(3, "M2", Rect::new(5, 6, 105, 206));

        assert_eq!(editor.draft(), editor.base());
    }

    #[test]
    fn a_region_registered_from_the_tray_avoids_names_only_in_the_copy() {
        let applied = with_regions(&["領域 1"]);
        let mut editor = Editor::new(applied.clone());
        // GUI で足して、まだ適用していない
        editor.add_region("領域", "M1", Rect::new(0, 0, 10, 10));

        let region = editor.new_registered_region(&applied, "領域", "M2", Rect::new(1, 2, 3, 4));

        assert_eq!(region.name, "領域 3");
        assert_eq!(
            (region.monitor.as_str(), region.rect),
            ("M2", Rect::new(1, 2, 3, 4))
        );
    }

    #[test]
    fn a_region_written_from_the_tray_joins_both_the_base_and_the_copy() {
        let mut editor = Editor::new(with_regions(&["a"]));
        editor.add_region("領域", "M1", Rect::new(0, 0, 10, 10));
        let mut written = with_regions(&["領域 2"]).regions.remove(0);
        written.monitor = "M2".to_owned();

        let i = editor.registered(written.clone());

        assert_eq!(i, 2);
        assert_eq!(region_names(&editor), ["a", "領域 1", "領域 2"]);
        assert_eq!(editor.draft().regions[2], written);
        // ファイルにはもう書いたので、base にも入れる（適用で足し直さない）
        assert_eq!(
            editor.base().regions.last().map(|r| r.name.as_str()),
            Some("領域 2")
        );
        assert_eq!(editor.base().regions.len(), 2);
    }

    #[test]
    fn a_duplicate_goes_after_its_original_without_the_hotkey() {
        let mut settings = with_regions(&["a", "b"]);
        settings.regions[0].hotkey = Some(chord("Ctrl+Alt+1"));
        settings.regions[0].scrap = true;
        let mut editor = Editor::new(settings);

        let i = editor.duplicate_region(0, "a のコピー");
        let j = editor.duplicate_region(0, "a のコピー");

        assert_eq!((i, j), (1, 1));
        assert_eq!(
            region_names(&editor),
            ["a", "a のコピー 2", "a のコピー", "b"]
        );
        let copy = &editor.draft().regions[2];
        // 同じホットキーは重なって適用できなくなるので、写さない
        assert_eq!(copy.hotkey, None);
        assert!(copy.scrap);
        assert_eq!(copy.loaded_name, None);
        assert!(editor.buttons().ok);
    }

    #[test]
    fn a_blank_scrap_region_cannot_use_its_save_and_scrap_items_and_copies_the_choice() {
        let mut editor = Editor::new(with_regions(&["a", "b"]));
        assert!(editor.region_uses_capture_outputs(0));

        editor.draft_mut().regions[0].blank_scrap = true;

        assert!(!editor.region_uses_capture_outputs(0));
        assert!(editor.region_uses_capture_outputs(1));
        assert!(!editor.region_uses_capture_outputs(2));
        let i = editor.duplicate_region(0, "a のコピー");
        assert!(!editor.region_uses_capture_outputs(i));
        // 白紙スクラップにしても、適用は止めない
        assert!(editor.buttons().ok);
    }

    #[test]
    fn a_removed_region_leaves_the_next_one_selected() {
        let mut editor = Editor::new(with_regions(&["a", "b", "c"]));

        assert_eq!(editor.remove_region(1), Some(1));
        assert_eq!(region_names(&editor), ["a", "c"]);
        assert_eq!(editor.remove_region(1), Some(0));
        assert_eq!(editor.remove_region(0), None);
        assert_eq!(region_names(&editor), Vec::<&str>::new());
    }

    #[test]
    fn a_moved_region_swaps_with_its_neighbour_and_stops_at_the_ends() {
        let mut editor = Editor::new(with_regions(&["a", "b", "c"]));

        assert_eq!(editor.move_region(0, Move::Down), 1);
        assert_eq!(region_names(&editor), ["b", "a", "c"]);
        assert_eq!(editor.move_region(1, Move::Up), 0);
        assert_eq!(editor.move_region(0, Move::Up), 0);
        assert_eq!(editor.move_region(2, Move::Down), 2);
        assert_eq!(region_names(&editor), ["a", "b", "c"]);
        assert!(!editor.buttons().apply);
    }

    #[test]
    fn region_names_must_be_filled_and_unique_before_applying() {
        let mut editor = Editor::new(with_regions(&["a", "b"]));

        editor.draft_mut().regions[1].name = "a".into();
        assert_eq!(
            editor.mark(&SettingsItem::RegionName(1)),
            Some(Mark::Problem(Problem::DuplicateName))
        );
        assert!(!editor.buttons().ok);
        editor.draft_mut().regions[1].name = String::new();
        assert_eq!(
            editor.mark(&SettingsItem::RegionName(1)),
            Some(Mark::Problem(Problem::EmptyName))
        );
        assert!(!editor.buttons().ok);
    }
    fn recipe_names(editor: &Editor) -> Vec<&str> {
        editor
            .draft()
            .recipes
            .iter()
            .map(|r| r.name.as_str())
            .collect()
    }

    #[test]
    fn an_added_recipe_gets_the_next_free_number_goes_last_and_needs_steps() {
        let mut editor = Editor::new(Settings::default());
        assert_eq!(editor.add_recipe("レシピ"), 0);
        assert_eq!(editor.add_recipe("レシピ"), 1);
        editor.draft_mut().recipes[0].name = "コピー".to_owned();
        assert_eq!(editor.add_recipe("レシピ"), 2);
        assert_eq!(recipe_names(&editor), ["コピー", "レシピ 2", "レシピ 1"]);
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(2)),
            Some(Mark::Problem(Problem::NoSteps))
        );
        assert_eq!(editor.draft().recipes[2].shortcut, None);
    }
    #[test]
    fn a_removed_recipe_leaves_the_next_one_selected() {
        let mut editor = Editor::new(Settings::default());
        for _ in 0..3 {
            editor.add_recipe("R");
        }
        assert_eq!(editor.remove_recipe(1), Some(1));
        assert_eq!(recipe_names(&editor), ["R 1", "R 3"]);
        assert_eq!(editor.remove_recipe(1), Some(0));
        assert_eq!(editor.remove_recipe(0), None);
        assert!(editor.draft().recipes.is_empty());
        assert_eq!(editor.remove_recipe(0), None);
    }

    fn steps(editor: &Editor, index: usize) -> Vec<&str> {
        editor.draft().recipes[index]
            .steps
            .iter()
            .map(recipe::Step::op_id)
            .collect()
    }

    fn choice_ids(editor: &Editor, index: usize) -> Vec<&'static str> {
        editor
            .step_choices(index)
            .iter()
            .filter_map(|c| match c {
                StepChoice::Op(id) => Some(*id),
                StepChoice::Appearance | StepChoice::Background | StepChoice::Capture(_) => None,
            })
            .collect()
    }

    #[test]
    fn an_appearance_step_is_offered_once_and_starts_with_every_item_unchanged() {
        let mut editor = with_steps(&["copy", "close"]);
        let choices = editor.step_choices(0);
        assert_eq!(
            choices
                .iter()
                .filter(|c| **c == StepChoice::Appearance)
                .count(),
            1
        );

        assert_eq!(editor.add_step(0, StepChoice::Appearance), Some(1));

        assert_eq!(steps(&editor, 0), ["copy", "appearance", "close"]);
        assert_eq!(
            editor.draft().recipes[0].steps[1],
            Step::Appearance(recipe::AppearanceStep::default())
        );
        // 項目を 1 つも書いていない操作のままでは適用できない
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::NoValues))
        );
        assert!(!editor.buttons().ok);
    }

    #[test]
    fn only_operations_a_recipe_can_take_are_offered_as_steps() {
        let mut editor = Editor::new(Settings::default());
        editor.add_recipe("R");
        let ids = choice_ids(&editor, 0);
        for id in ["copy", "zoom_in", "toggle_compact", "close", "annotate"] {
            assert!(ids.contains(&id), "{id} in {ids:?}");
        }
        // トリム、引数を取る操作、ドラッグ専用の操作は出さない
        for id in ["trim", "zoom", "opacity", "move"] {
            assert!(!ids.contains(&id), "{id} in {ids:?}");
        }
    }

    #[test]
    fn a_step_added_after_close_goes_before_it_and_close_is_offered_only_once() {
        let mut editor = Editor::new(Settings::default());
        editor.add_recipe("R");
        assert_eq!(editor.add_step(0, "close"), Some(0));
        assert!(!choice_ids(&editor, 0).contains(&"close"));
        assert_eq!(editor.add_step(0, "copy"), Some(0));
        assert_eq!(editor.add_step(0, "copy"), Some(1));
        assert_eq!(steps(&editor, 0), ["copy", "copy", "close"]);
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert_eq!(editor.add_step(1, "copy"), None);
    }

    #[test]
    fn a_recipe_ending_with_one_last_only_step_is_not_offered_the_other() {
        // 最後にだけ置ける操作は 1 つまで。足すとその前に入るので、「閉じる」と「書き込み」は一緒に出さない
        for (last, other) in [("close", "annotate"), ("annotate", "close")] {
            let mut editor = Editor::new(Settings::default());
            editor.add_recipe("R");
            editor.add_step(0, last);
            let ids = choice_ids(&editor, 0);
            assert!(!ids.contains(&last) && !ids.contains(&other), "{ids:?}");
            assert!(ids.contains(&"copy"), "{ids:?}");
        }
    }

    /// 操作の並びが `ids` のレシピ 1 つを持つ写し（ほかに適用を止める問題はない）。
    fn with_steps(ids: &[&str]) -> Editor {
        let mut settings = Settings::default();
        settings.hotkeys.select_capture = Some(chord("Ctrl+Shift+A"));
        settings.recipes.push(Recipe {
            name: "R".to_owned(),
            steps: ids.iter().map(|&id| id.into()).collect(),
            shortcut: None,
            hotkey: None,
        });
        Editor::new(settings)
    }

    /// `steps = [...]` のレシピ 1 つを設定ファイルから読んだ写し（ほかに適用を止める問題はない）。
    fn with_steps_read(steps: &str) -> Editor {
        let (settings, issues) = super::super::load(&format!(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Shift+A\"\n[[recipes]]\nname = \"R\"\nsteps = {steps}\n"
        ));
        assert_eq!(issues, vec![], "{steps}");
        Editor::new(settings)
    }

    fn labels(editor: &Editor) -> Vec<String> {
        let catalog = crate::i18n::catalog(crate::i18n::Language::Japanese);
        editor.draft().recipes[0]
            .steps
            .iter()
            .map(|s| step_label(s, |key| catalog.text(key)))
            .collect()
    }

    #[test]
    fn the_step_list_names_a_transparent_background_in_words() {
        let editor = with_steps_read(
            r##"[{ op = "background", color = "transparent" }, { op = "background", color = "#ffffff" }]"##,
        );
        assert_eq!(labels(&editor), ["背景色: 透明", "背景色: #FFFFFF"]);
    }

    #[test]
    fn the_step_list_sums_up_a_step_with_values() {
        let editor = with_steps_read(
            r##"["copy", { op = "appearance", border_color = "#ff0000" }, { op = "appearance", opacity = 50, border = false, shadow = true }, { op = "appearance", border = true, shadow = false }, { op = "appearance", border_width = 3, border_color = "#ff0000" }, { op = "appearance", shadow = true, border_width = 16 }, "nope"]"##,
        );
        assert_eq!(
            labels(&editor),
            [
                "コピー",
                "見た目: 枠 #FF0000",
                "見た目: 透明度 50% 枠なし 影",
                "見た目: 枠 影なし",
                "見た目: 枠 #FF0000 3px",
                "見た目: 枠 16px 影",
                "nope",
            ]
        );
    }

    #[test]
    fn a_wrong_step_with_values_marks_the_steps_and_blocks_applying() {
        let editor = with_steps_read(r#"[{ op = "appearance", opacity = 50 }]"#);
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert!(editor.buttons().ok);
        for (steps, problem) in [
            (
                r#"[{ op = "appearance", opacity = 5 }]"#,
                Problem::OutOfRange,
            ),
            (
                r#"[{ op = "appearance", opacity = "x" }]"#,
                Problem::BadValue,
            ),
            (r#"[{ op = "appearance", width = 3 }]"#, Problem::UnknownKey),
            (
                r#"[{ op = "copy", shadow = true }]"#,
                Problem::NotAppearance,
            ),
            (r#"[{ op = "appearance" }]"#, Problem::NoValues),
            (
                r##"[{ op = "appearance", border = false, border_color = "#000000" }]"##,
                Problem::BorderOffWithColorOrWidth,
            ),
        ] {
            let editor = with_steps_read(steps);
            assert_eq!(
                editor.mark(&SettingsItem::RecipeSteps(0)),
                Some(Mark::Problem(problem)),
                "{steps}"
            );
            assert!(!editor.buttons().ok, "{steps}");
        }
    }

    /// レシピ 1 つに、全部の項目が「変えない」の見た目を指定する操作を 1 つ足した写し。
    fn with_an_appearance_step() -> Editor {
        let mut editor = with_steps(&["copy"]);
        editor.add_step(0, StepChoice::Appearance);
        editor
    }

    fn fields(editor: &Editor, step: usize) -> Vec<Option<AppearanceInput>> {
        AppearanceItem::ALL
            .iter()
            .map(|&item| editor.appearance(0, step, item))
            .collect()
    }

    #[test]
    fn the_fields_of_a_new_appearance_step_are_all_unchanged() {
        let editor = with_an_appearance_step();
        let unchanged = |item: AppearanceItem| {
            Some(if item.is_switch() {
                AppearanceInput::Switch(None)
            } else {
                AppearanceInput::Text(String::new())
            })
        };
        assert_eq!(
            fields(&editor, 1),
            AppearanceItem::ALL.map(unchanged).to_vec()
        );
        // パラメータのない操作には欄がない
        assert_eq!(fields(&editor, 0), vec![None; 5]);
        assert_eq!(editor.appearance(0, 2, AppearanceItem::Border), None);
    }

    #[test]
    fn values_set_in_the_fields_go_into_the_copy_and_the_step_list() {
        let mut editor = with_an_appearance_step();
        let text = |s: &str| AppearanceInput::Text(s.to_owned());
        editor.set_appearance(
            0,
            1,
            AppearanceItem::Border,
            AppearanceInput::Switch(Some(true)),
        );
        editor.set_appearance(0, 1, AppearanceItem::BorderColor, text(" #ff0000 "));
        editor.set_appearance(0, 1, AppearanceItem::BorderWidth, text("3"));
        editor.set_appearance(0, 1, AppearanceItem::Opacity, text("50"));
        editor.set_appearance(
            0,
            1,
            AppearanceItem::Shadow,
            AppearanceInput::Switch(Some(false)),
        );

        assert_eq!(
            editor.draft().recipes[0].steps[1]
                .values()
                .map(|s| s.values),
            Some(recipe::AppearanceValues {
                opacity: Some(50),
                border: Some(true),
                border_color: Some(crate::settings::Rgb(0xFF, 0, 0)),
                border_width: Some(3),
                shadow: Some(false),
            })
        );
        assert_eq!(
            fields(&editor, 1),
            [
                Some(AppearanceInput::Switch(Some(true))),
                Some(text("#FF0000")),
                Some(text("3")),
                Some(text("50")),
                Some(AppearanceInput::Switch(Some(false))),
            ]
        );
        assert_eq!(
            labels(&editor)[1],
            "見た目: 透明度 50% 枠 #FF0000 3px 影なし"
        );
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert!(editor.buttons().apply);

        // 「変えない」に戻すと、その項目は書かない
        editor.set_appearance(0, 1, AppearanceItem::Shadow, AppearanceInput::Switch(None));
        editor.set_appearance(0, 1, AppearanceItem::BorderColor, text(""));
        editor.set_appearance(0, 1, AppearanceItem::Opacity, text(" "));
        assert_eq!(labels(&editor)[1], "見た目: 枠 3px");
        assert_eq!(
            editor.appearance(0, 1, AppearanceItem::BorderColor),
            Some(text(""))
        );
    }

    #[test]
    fn values_the_fields_cannot_take_stay_as_typed_and_block_applying() {
        let text = |s: &str| AppearanceInput::Text(s.to_owned());
        for (item, typed, problem) in [
            (AppearanceItem::Opacity, "5", Problem::OutOfRange),
            (AppearanceItem::Opacity, "101", Problem::OutOfRange),
            (AppearanceItem::BorderWidth, "17", Problem::OutOfRange),
            (AppearanceItem::BorderWidth, "0", Problem::OutOfRange),
            (AppearanceItem::BorderWidth, "x", Problem::BadValue),
            (AppearanceItem::BorderColor, "#FF00", Problem::BadValue),
        ] {
            let mut editor = with_an_appearance_step();
            editor.set_appearance(
                0,
                1,
                AppearanceItem::Shadow,
                AppearanceInput::Switch(Some(true)),
            );

            editor.set_appearance(0, 1, item, text(typed));

            assert_eq!(editor.appearance(0, 1, item), Some(text(typed)), "{typed}");
            assert_eq!(
                editor.mark(&SettingsItem::RecipeSteps(0)),
                Some(Mark::Problem(problem)),
                "{typed}"
            );
            assert!(!editor.buttons().ok, "{typed}");

            // 読める値に直せば、問題は消える
            editor.set_appearance(0, 1, item, text(""));
            assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None, "{typed}");
            assert!(editor.buttons().ok, "{typed}");
        }
    }

    #[test]
    fn no_border_with_a_border_color_or_width_blocks_applying() {
        let mut editor = with_an_appearance_step();
        let text = |s: &str| AppearanceInput::Text(s.to_owned());
        editor.set_appearance(
            0,
            1,
            AppearanceItem::Border,
            AppearanceInput::Switch(Some(false)),
        );
        assert!(editor.buttons().ok);

        editor.set_appearance(0, 1, AppearanceItem::BorderWidth, text("2"));

        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::BorderOffWithColorOrWidth))
        );
        assert!(!editor.buttons().ok);
        editor.set_appearance(0, 1, AppearanceItem::Border, AppearanceInput::Switch(None));
        assert!(editor.buttons().ok);
    }

    #[test]
    fn a_value_read_from_the_file_that_was_out_of_range_shows_as_written_until_overwritten() {
        let mut editor = with_steps_read(
            r#"[{ op = "appearance", opacity = 500, border_color = "red", shadow = true }]"#,
        );
        let text = |s: &str| AppearanceInput::Text(s.to_owned());
        assert_eq!(
            editor.appearance(0, 0, AppearanceItem::Opacity),
            Some(text("500"))
        );
        assert_eq!(
            editor.appearance(0, 0, AppearanceItem::BorderColor),
            Some(text("red"))
        );
        assert!(!editor.buttons().ok);

        editor.set_appearance(0, 0, AppearanceItem::Opacity, text("50"));
        editor.set_appearance(0, 0, AppearanceItem::BorderColor, text("#00ff00"));

        // 書き直した項目の読めなかった値は残らない
        assert_eq!(
            editor.draft().recipes[0].steps[0]
                .values()
                .unwrap()
                .rejected,
            []
        );
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert_eq!(labels(&editor), ["見た目: 透明度 50% 枠 #00FF00 影"]);
    }

    #[test]
    fn a_step_made_in_the_fields_is_written_as_an_inline_table_and_reads_back_the_same() {
        let mut editor = with_an_appearance_step();
        let text = |s: &str| AppearanceInput::Text(s.to_owned());
        editor.set_appearance(0, 1, AppearanceItem::BorderColor, text("#ff0000"));
        editor.set_appearance(0, 1, AppearanceItem::Opacity, text("40"));
        editor.set_appearance(
            0,
            1,
            AppearanceItem::Shadow,
            AppearanceInput::Switch(Some(false)),
        );
        assert!(editor.buttons().apply);

        let written = super::super::write::rewrite("version = 1\n", editor.draft()).unwrap();

        assert!(
            written.contains(
                r##"steps = ["copy", { op = "appearance", border_color = "#FF0000", opacity = 40, shadow = false }]"##
            ),
            "{written}"
        );
        let (read, issues) = super::super::load(&written);
        assert_eq!(issues, vec![]);
        assert_eq!(read.recipes, editor.draft().recipes);
    }

    #[test]
    fn fields_of_a_step_without_values_or_of_a_missing_step_change_nothing() {
        let mut editor = with_an_appearance_step();
        let before = editor.draft().clone();
        editor.set_appearance(
            0,
            0,
            AppearanceItem::Opacity,
            AppearanceInput::Text("50".into()),
        );
        editor.set_appearance(
            0,
            5,
            AppearanceItem::Opacity,
            AppearanceInput::Text("50".into()),
        );
        editor.set_appearance(
            3,
            1,
            AppearanceItem::Opacity,
            AppearanceInput::Text("50".into()),
        );
        // 欄と違う種類の入力は捨てる
        editor.set_appearance(
            0,
            1,
            AppearanceItem::Opacity,
            AppearanceInput::Switch(Some(true)),
        );
        editor.set_appearance(
            0,
            1,
            AppearanceItem::Border,
            AppearanceInput::Text("true".into()),
        );
        assert_eq!(editor.draft(), &before);
    }

    #[test]
    fn setting_the_appearance_is_not_offered_as_a_step_without_values() {
        let editor = with_steps(&["copy"]);
        assert!(!choice_ids(&editor, 0).contains(&"appearance"));
    }

    #[test]
    fn a_moved_step_swaps_with_its_neighbour_and_stops_at_the_ends() {
        let mut editor = with_steps(&["copy", "zoom_in", "close"]);
        assert_eq!(editor.move_step(0, 1, Move::Up), 0);
        assert_eq!(steps(&editor, 0), ["zoom_in", "copy", "close"]);
        assert_eq!(editor.move_step(0, 0, Move::Up), 0);
        assert_eq!(editor.move_step(0, 2, Move::Down), 2);
        assert_eq!(steps(&editor, 0), ["zoom_in", "copy", "close"]);
        // close を最後から動かすと、適用できない印が付く
        assert_eq!(editor.move_step(0, 2, Move::Up), 1);
        assert_eq!(steps(&editor, 0), ["zoom_in", "close", "copy"]);
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::LastOnlyNotLast))
        );
        assert_eq!(editor.move_step(1, 0, Move::Down), 0);
    }

    #[test]
    fn a_removed_step_leaves_the_next_one_selected() {
        let mut editor = with_steps(&["copy", "zoom_in", "close"]);
        assert_eq!(editor.remove_step(0, 1), Some(1));
        assert_eq!(steps(&editor, 0), ["copy", "close"]);
        assert_eq!(editor.remove_step(0, 1), Some(0));
        assert_eq!(editor.remove_step(0, 0), None);
        assert!(steps(&editor, 0).is_empty());
        assert_eq!(editor.remove_step(0, 0), None);
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::NoSteps))
        );
    }

    #[test]
    fn recipe_names_are_trimmed_and_must_be_filled_and_unique_before_applying() {
        let mut editor = with_steps(&["copy"]);
        editor.add_recipe("R");
        editor.add_step(1, "close");
        editor.set_recipe_name(1, "  R  ");
        assert_eq!(recipe_names(&editor), ["R", "R"]);
        let duplicate = Some(Mark::Problem(Problem::DuplicateName));
        assert_eq!(editor.mark(&SettingsItem::RecipeName(0)), duplicate);
        assert_eq!(editor.mark(&SettingsItem::RecipeName(1)), duplicate);
        assert!(!editor.buttons().ok);

        editor.set_recipe_name(1, " ");
        assert_eq!(
            editor.mark(&SettingsItem::RecipeName(1)),
            Some(Mark::Problem(Problem::EmptyName))
        );
        editor.set_recipe_name(1, "閉じる");
        assert_eq!(editor.mark(&SettingsItem::RecipeName(1)), None);
        assert!(editor.buttons().ok);
    }

    #[test]
    fn only_operations_with_parameters_show_parameter_fields() {
        let editor = with_steps_read(
            r##"["copy", { op = "appearance", opacity = 50 }, { op = "background", color = "#ffffff" }]"##,
        );
        assert_eq!(editor.param_fields(0, 0), None);
        assert_eq!(editor.param_fields(0, 1), Some(ParamFields::Appearance));
        assert_eq!(editor.param_fields(0, 2), Some(ParamFields::Background));
        // 選んでいる操作やレシピがない
        assert_eq!(editor.param_fields(0, 3), None);
        assert_eq!(editor.param_fields(1, 0), None);
    }

    #[test]
    fn an_added_appearance_operation_is_the_one_to_select_and_shows_its_fields() {
        let mut editor = with_steps(&["copy", "close"]);
        // `close` の前に入るので、選ぶのは末尾ではなく入った位置
        let added = editor.add_step(0, StepChoice::Appearance).unwrap();
        assert_eq!(added, 1);
        assert_eq!(editor.param_fields(0, added), Some(ParamFields::Appearance));

        let added = editor.add_step(0, "copy").unwrap();
        assert_eq!(editor.param_fields(0, added), None);
    }

    #[test]
    fn a_background_operation_is_offered_last_and_an_added_one_is_selected_with_its_fields() {
        let mut editor = with_steps(&["copy", "close"]);
        let choices = editor.step_choices(0);
        assert_eq!(
            choices[choices.len() - 2..],
            [StepChoice::Appearance, StepChoice::Background]
        );

        // `close` の前に入る。色はまだ選んでいない（そのままでは適用できない）
        let added = editor.add_step(0, StepChoice::Background).unwrap();
        assert_eq!(added, 1);
        assert_eq!(steps(&editor, 0), ["copy", "background", "close"]);
        assert_eq!(editor.param_fields(0, added), Some(ParamFields::Background));
        assert_eq!(editor.background(0, added), Some(None));
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::NoValues))
        );
        assert!(!editor.buttons().ok);
    }

    #[test]
    fn a_background_chosen_in_the_fields_is_written_as_a_background_step_and_reads_back_the_same() {
        let mut editor = with_steps(&["copy"]);
        editor.add_step(0, StepChoice::Background);
        editor.add_step(0, StepChoice::Background);
        // 「選ぶ…」で選んだ色と、「透明」のチェック
        editor.set_background(
            0,
            1,
            Some(Background::Color(super::super::Rgb(0x1E, 0x88, 0xE5))),
        );
        editor.set_background(0, 2, Some(Background::Transparent));
        assert_eq!(
            editor.background(0, 1),
            Some(Some(Background::Color(super::super::Rgb(0x1E, 0x88, 0xE5))))
        );
        assert_eq!(editor.background(0, 2), Some(Some(Background::Transparent)));
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert!(editor.buttons().apply);

        let written = super::super::write::rewrite("version = 1\n", editor.draft()).unwrap();

        assert!(
            written.contains(
                r##"steps = ["copy", { op = "background", color = "#1E88E5" }, { op = "background", color = "transparent" }]"##
            ),
            "{written}"
        );
        let (read, issues) = super::super::load(&written);
        assert_eq!(issues, vec![]);
        assert_eq!(read.recipes, editor.draft().recipes);
        let reopened = Editor::new(read);
        assert_eq!(
            reopened.background(0, 2),
            Some(Some(Background::Transparent))
        );
    }

    #[test]
    fn unchecking_transparent_leaves_the_colour_to_be_chosen() {
        let mut editor = with_steps_read(r#"[{ op = "background", color = "transparent" }]"#);
        editor.set_background(0, 0, None);
        assert_eq!(editor.background(0, 0), Some(None));
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::NoValues))
        );
    }

    #[test]
    fn choosing_a_background_replaces_a_colour_that_could_not_be_read() {
        let (settings, _) = super::super::load(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Shift+A\"\n[[recipes]]\nname = \"R\"\nsteps = [{ op = \"background\", color = \"white\" }]\n",
        );
        let mut editor = Editor::new(settings);
        assert_eq!(editor.background(0, 0), Some(None));
        assert!(editor.mark(&SettingsItem::RecipeSteps(0)).is_some());

        editor.set_background(0, 0, Some(Background::WHITE));

        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert!(editor.buttons().ok);
    }

    #[test]
    fn choosing_a_background_for_another_operation_changes_nothing() {
        let mut editor = with_steps_read(r#"["copy", { op = "appearance", opacity = 50 }]"#);
        let before = editor.draft().clone();
        editor.set_background(0, 0, Some(Background::Transparent));
        editor.set_background(0, 1, Some(Background::Transparent));
        editor.set_background(0, 2, Some(Background::Transparent));
        editor.set_background(1, 0, Some(Background::Transparent));
        assert_eq!(editor.draft(), &before);
        assert_eq!(editor.background(0, 1), None);
    }

    #[test]
    fn recipe_shortcuts_overlapping_an_operation_are_marked_and_block_applying() {
        let mut editor = with_steps(&["copy"]);
        editor.set_chord(&SettingsItem::RecipeShortcut(0), Some(chord("Ctrl+C")));
        assert_eq!(
            editor.chord(&SettingsItem::RecipeShortcut(0)),
            Some(chord("Ctrl+C"))
        );
        assert!(matches!(
            editor.mark(&SettingsItem::RecipeShortcut(0)),
            Some(Mark::Problem(Problem::Conflict(_)))
        ));
        assert!(!editor.buttons().ok);
    }

    fn captures(editor: &Editor, index: usize) -> Vec<Capture> {
        editor
            .step_choices(index)
            .iter()
            .filter_map(|c| match c {
                StepChoice::Capture(c) => Some(*c),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn captures_are_always_offered_first_and_named_in_the_step_list() {
        for ids in [&[][..], &["copy"], &["copy", "close"], &["select_capture"]] {
            let editor = with_steps(ids);
            assert_eq!(
                captures(&editor, 0),
                [Capture::Select, Capture::Region],
                "{ids:?}"
            );
            assert_eq!(
                editor.step_choices(0)[0],
                StepChoice::Capture(Capture::Select),
                "{ids:?}"
            );
        }
        assert_eq!(
            StepChoice::Capture(Capture::Select).label_key(),
            Some("recipe.capture.select_capture")
        );
        let editor = with_steps(&["select_capture", "copy"]);
        assert_eq!(labels(&editor), ["範囲選択キャプチャ", "コピー"]);
    }

    #[test]
    fn a_capture_added_after_the_first_step_is_a_problem_until_it_is_first() {
        let mut editor = with_steps(&["copy", "close"]);
        // ほかの操作と同じく、最後にだけ置ける操作の前に入る
        assert_eq!(
            editor.add_step(0, StepChoice::Capture(Capture::Select)),
            Some(1)
        );
        assert_eq!(steps(&editor, 0), ["copy", "select_capture", "close"]);
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::FirstOnlyNotFirst))
        );
        assert!(!editor.buttons().ok);
        editor.move_step(0, 1, Move::Up);
        editor.set_chord(&SettingsItem::RecipeHotkey(0), Some(chord("Ctrl+Alt+C")));
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert!(editor.buttons().ok);
    }

    #[test]
    fn the_key_field_follows_the_first_step_and_switching_clears_the_key() {
        let mut editor = with_steps(&["copy"]);
        assert_eq!(editor.key_item(0), Some(SettingsItem::RecipeShortcut(0)));
        editor.set_chord(&SettingsItem::RecipeShortcut(0), Some(chord("Ctrl+Q")));
        editor.add_step(0, StepChoice::Capture(Capture::Select));
        // 2 つ目に足しただけでは最初の操作は変わらない
        assert_eq!(editor.key_item(0), Some(SettingsItem::RecipeShortcut(0)));
        assert_eq!(
            editor.chord(&SettingsItem::RecipeShortcut(0)),
            Some(chord("Ctrl+Q"))
        );

        editor.move_step(0, 1, Move::Up);
        assert_eq!(editor.key_item(0), Some(SettingsItem::RecipeHotkey(0)));
        assert_eq!(editor.chord(&SettingsItem::RecipeShortcut(0)), None);
        assert_eq!(
            editor.mark(&SettingsItem::RecipeHotkey(0)),
            Some(Mark::Problem(Problem::Unset))
        );
        editor.set_chord(&SettingsItem::RecipeHotkey(0), Some(chord("Ctrl+Alt+C")));
        assert_eq!(editor.mark(&SettingsItem::RecipeHotkey(0)), None);
        assert!(editor.buttons().ok);

        // 並べ替えで最初がキャプチャのままなら、キーは残る
        editor.add_step(0, "close");
        editor.move_step(0, 2, Move::Up);
        assert_eq!(
            editor.chord(&SettingsItem::RecipeHotkey(0)),
            Some(chord("Ctrl+Alt+C"))
        );

        editor.remove_step(0, 0);
        assert_eq!(editor.key_item(0), Some(SettingsItem::RecipeShortcut(0)));
        assert_eq!(editor.chord(&SettingsItem::RecipeHotkey(0)), None);
        assert_eq!(editor.key_item(1), None);
    }

    #[test]
    fn a_capture_recipe_hotkey_overlapping_another_hotkey_is_marked() {
        let mut editor = with_steps(&["select_capture"]);
        editor.set_chord(&SettingsItem::RecipeHotkey(0), Some(chord("Ctrl+Shift+A")));
        assert!(matches!(
            editor.mark(&SettingsItem::RecipeHotkey(0)),
            Some(Mark::Problem(Problem::Conflict(_)))
        ));
        assert!(matches!(
            editor.mark(&SettingsItem::SelectCaptureHotkey),
            Some(Mark::Problem(Problem::Conflict(_)))
        ));
        assert!(!editor.buttons().ok);
    }

    #[test]
    fn entering_the_shown_key_clears_the_hidden_one_written_in_the_file() {
        let (settings, _) = super::super::load(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Shift+A\"\n[[recipes]]\nname = \"R\"\nsteps = [\"select_capture\"]\nshortcut = \"Ctrl+Q\"\nhotkey = \"Ctrl+Alt+C\"\n",
        );
        let mut editor = Editor::new(settings);
        assert_eq!(
            editor.mark(&SettingsItem::RecipeHotkey(0)),
            Some(Mark::Problem(Problem::ShortcutOnCapture))
        );
        editor.set_chord(&SettingsItem::RecipeHotkey(0), Some(chord("Ctrl+Alt+C")));
        assert_eq!(editor.chord(&SettingsItem::RecipeShortcut(0)), None);
        assert_eq!(editor.mark(&SettingsItem::RecipeHotkey(0)), None);
        assert!(editor.buttons().ok);
    }

    /// 定型領域「右」「左」（この順）と、`steps = [...]` のキャプチャで始まるレシピ 1 つを読んだ写し。
    fn with_regions_read(steps: &str) -> Editor {
        let region = |name: &str| {
            format!(
                "[[regions]]\nname = \"{name}\"\nmonitor = \"m\"\nx = 0\ny = 0\nwidth = 10\nheight = 10\n"
            )
        };
        let (settings, issues) = super::super::load(&format!(
            "version = 1\n[hotkeys]\nselect_capture = \"Ctrl+Shift+A\"\n{}{}[[recipes]]\nname = \"R\"\nsteps = {steps}\nhotkey = \"Ctrl+Alt+C\"\n",
            region("右"),
            region("左"),
        ));
        assert_eq!(issues, vec![], "{steps}");
        Editor::new(settings)
    }

    #[test]
    fn a_fixed_region_capture_is_added_without_a_region_and_offers_the_regions_in_order() {
        let mut editor = with_regions_read("[]");
        assert_eq!(captures(&editor, 0), [Capture::Select, Capture::Region]);
        let added = editor
            .add_step(0, StepChoice::Capture(Capture::Region))
            .unwrap();
        assert_eq!(steps(&editor, 0), ["capture_region"]);
        assert_eq!(editor.param_fields(0, added), Some(ParamFields::Region));
        assert_eq!(
            editor.capture_region(0, added),
            Some(RegionChoices {
                names: vec!["右".to_owned(), "左".to_owned()],
                selected: None,
            })
        );
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::NoValues))
        );
        assert!(!editor.buttons().ok);
    }

    #[test]
    fn a_region_chosen_in_the_field_is_written_as_a_fixed_region_capture_and_reads_back_the_same() {
        let mut editor = with_regions_read("[]");
        editor.add_step(0, StepChoice::Capture(Capture::Region));
        editor.add_step(0, "copy");
        editor.set_capture_region(0, 0, "左");
        editor.set_chord(&SettingsItem::RecipeHotkey(0), Some(chord("Ctrl+Alt+C")));
        assert_eq!(
            editor.capture_region(0, 0).map(|c| c.selected),
            Some(Some(1))
        );
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert!(editor.buttons().ok);
        assert_eq!(labels(&editor), ["定型領域キャプチャ: 左", "コピー"]);

        let written = super::super::write::rewrite("version = 1\n", editor.draft()).unwrap();

        assert!(
            written.contains(r#"steps = [{ op = "capture_region", region = "左" }, "copy"]"#),
            "{written}"
        );
        let (read, issues) = super::super::load(&written);
        assert_eq!(issues, vec![]);
        assert_eq!(read.recipes, editor.draft().recipes);
    }

    #[test]
    fn a_fixed_region_capture_of_a_missing_region_shows_that_name_and_is_a_problem_until_another_is_chosen()
     {
        let mut editor = with_regions_read(r#"[{ op = "capture_region", region = "中" }]"#);
        assert_eq!(
            editor.capture_region(0, 0),
            Some(RegionChoices {
                names: vec!["右".to_owned(), "左".to_owned(), "中".to_owned()],
                selected: Some(2),
            })
        );
        assert_eq!(
            editor.mark(&SettingsItem::RecipeSteps(0)),
            Some(Mark::Problem(Problem::NoSuchRegion))
        );
        assert!(!editor.buttons().ok);

        editor.set_capture_region(0, 0, "右");

        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
        assert!(editor.buttons().ok);
        // 定型領域キャプチャでない操作は何も変えない
        let before = editor.draft().clone();
        editor.set_capture_region(0, 1, "左");
        editor.set_capture_region(1, 0, "左");
        assert_eq!(editor.draft(), &before);
    }

    #[test]
    fn a_fixed_region_capture_of_a_region_value_that_could_not_be_read_is_replaced_by_choosing_one()
    {
        let mut editor = with_regions_read(r#"[{ op = "capture_region", region = 1 }]"#);
        assert_eq!(editor.capture_region(0, 0).map(|c| c.selected), Some(None));
        assert!(editor.mark(&SettingsItem::RecipeSteps(0)).is_some());
        editor.set_capture_region(0, 0, "左");
        assert_eq!(editor.mark(&SettingsItem::RecipeSteps(0)), None);
    }
}
