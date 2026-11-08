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
//!   `step_choices`（足せる操作と見た目を指定する手）/ `add_step` / `move_step` / `remove_step`。見た目を指定する手の
//!   欄（`AppearanceItem`）は `appearance` / `set_appearance` で読み書きする（「変えない」は書かない）。ショートカットは
//!   `set_chord` の `RecipeShortcut`。検査（`recipe::plan`）で引っかかったもの（欄に打った範囲外の値を含む）は `mark` の
//!   `RecipeName` / `RecipeSteps` に出る。操作の並びの行の文言は `step_label`（値のある手は書いた値の要約）。

use std::collections::BTreeSet;

use super::{Problem, Region, Settings, SettingsItem, new_region};
use crate::geometry::Rect;
use crate::keys::Chord;
pub use crate::recipe::AppearanceItem;
use crate::recipe::{self, AppearanceStep, ItemValue, Recipe, Step, StepKey, Written};
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
            Mark::Problem(Problem::CloseNotLast) => "settings_gui.mark.close_not_last",
            Mark::Problem(Problem::OutOfRange) => "settings_gui.mark.out_of_range",
            Mark::Problem(Problem::BadValue) => "settings_gui.mark.bad_value",
            Mark::Problem(Problem::UnknownKey) => "settings_gui.mark.unknown_key",
            Mark::Problem(Problem::NotAppearance) => "settings_gui.mark.not_appearance",
            Mark::Problem(Problem::NoValues) => "settings_gui.mark.no_values",
            Mark::Problem(Problem::BorderOffWithColorOrWidth) => {
                "settings_gui.mark.border_off_with_color_or_width"
            }
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
        let len = self.draft.recipes.len();
        (len > 0).then(|| index.min(len - 1))
    }

    /// レシピ `index` の名前を `name`（前後の空白を除く）にする。`index` のレシピがなければ何もしない。
    pub fn set_recipe_name(&mut self, index: usize, name: &str) {
        if let Some(r) = self.draft.recipes.get_mut(index) {
            r.name = name.trim().to_owned();
        }
    }

    /// レシピ `index` の「操作を足す」で選べるもの。値のない操作（表の順）と、最後に見た目を指定する手。値のない
    /// 操作は、レシピに入れられる操作（`recipe::can_be_step`）のうち、`close` で終わるレシピには `close` を出さない
    /// （`add_step` は `close` の前に足すので、2 つ目の `close` は最後に置けない）。`index` のレシピがなければ空。
    pub fn step_choices(&self, index: usize) -> Vec<StepChoice> {
        let Some(r) = self.draft.recipes.get(index) else {
            return Vec::new();
        };
        let ends_with_last_only = r
            .steps
            .last()
            .is_some_and(|s| recipe::is_last_only(s.op_id(), OPERATIONS));
        OPERATIONS
            .iter()
            .filter(|op| recipe::can_be_step(op))
            .filter(|op| !(ends_with_last_only && op.in_recipe == InRecipe::Last))
            .map(|op| StepChoice::Op(op.id))
            .chain(std::iter::once(StepChoice::Appearance))
            .collect()
    }

    /// レシピ `index` に `choice` を足す。見た目を指定する手は、全部の項目が「変えない」（そのままでは
    /// `NoValues`）。レシピが `close` で終わっていれば、その前に入れる（`close` は最後にだけ置けるため）。入れた
    /// 位置を返す。`index` のレシピがなければ何もせず None。
    pub fn add_step(&mut self, index: usize, choice: impl Into<StepChoice>) -> Option<usize> {
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
            StepChoice::Appearance => Step::Appearance(AppearanceStep::default()),
        };
        steps.insert(at, step);
        Some(at)
    }

    /// レシピ `index` の手 `step` を隣と入れ替える（「上へ」「下へ」）。端か、手がなければ動かさない。移った
    /// 位置を返す。
    pub fn move_step(&mut self, index: usize, step: usize, to: Move) -> usize {
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
                other
            }
            None => step,
        }
    }

    /// レシピ `index` の手 `step` を消す。次に選ぶ位置（同じ位置、末尾を消したらその前。なくなれば None）を
    /// 返す。手がなければ何もせず None。
    pub fn remove_step(&mut self, index: usize, step: usize) -> Option<usize> {
        let steps = &mut self.draft.recipes.get_mut(index)?.steps;
        if step >= steps.len() {
            return None;
        }
        steps.remove(step);
        let len = steps.len();
        (len > 0).then(|| step.min(len - 1))
    }

    /// レシピ `index` の手 `step` が見た目を指定する手なら、その項目 `item` の欄に出す値。「変えない」は
    /// `Switch(None)` か空の `Text`。読めなかった値（範囲外など）は書いたまま出す。見た目を指定する手でなければ None。
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

    /// レシピ `index` の手 `step`（見た目を指定する手）の項目 `item` を、欄の入力 `input` にする。「変えない」
    /// （`Switch(None)`、空白だけの `Text`）は書かない。読めない文字（範囲外の数、`#RRGGBB` でない色）は書いたまま
    /// 残し、設定ファイルから読めなかった値と同じくレシピの問題にする（適用できない）。項目を書き直したら、その
    /// 項目の読めなかった値は消す。見た目を指定する手でないか、欄と違う種類の入力なら何もしない。
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

/// 「レシピ」タブの手の一覧の行の文言。値のない手は操作の名前（表にない id はそのまま）、値のある手は
/// 「見た目: 枠 #FF0000 3px」のように、書いた項目を決まった順（透明度、枠、枠の太さ、影）に並べた要約。`text` は文言のキーから
/// 文言を引く。
pub fn step_label(step: &Step, text: impl Fn(&'static str) -> &'static str) -> String {
    let values = match step {
        Step::Op(id) => {
            return OPERATIONS
                .iter()
                .find(|op| op.id == id)
                .map_or_else(|| id.clone(), |op| text(op.label_key).to_owned());
        }
        Step::Appearance(step) => step.values,
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

/// 見た目を指定する手の欄の値（`Editor::appearance` / `set_appearance`）。
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

/// 「操作を足す」で選べるもの（`Editor::step_choices`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepChoice {
    /// 値のない操作。
    Op(OperationId),
    /// 見た目を指定する手（値は足してから欄で入れる）。
    Appearance,
}

impl StepChoice {
    /// ドロップダウンに出す文言のキー（表にない id は None）。
    pub fn label_key(self) -> Option<&'static str> {
        let id = match self {
            StepChoice::Op(id) => id,
            StepChoice::Appearance => recipe::APPEARANCE,
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
            Mark::Problem(Problem::CloseNotLast),
            Mark::Problem(Problem::OutOfRange),
            Mark::Problem(Problem::BadValue),
            Mark::Problem(Problem::UnknownKey),
            Mark::Problem(Problem::NotAppearance),
            Mark::Problem(Problem::NoValues),
            Mark::Problem(Problem::BorderOffWithColorOrWidth),
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
            .chain(StepChoice::Appearance.label_key());
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
                StepChoice::Appearance => None,
            })
            .collect()
    }

    #[test]
    fn an_appearance_step_is_offered_last_and_starts_with_every_item_unchanged() {
        let mut editor = with_steps(&["copy", "close"]);
        let choices = editor.step_choices(0);
        assert_eq!(choices.last(), Some(&StepChoice::Appearance));
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
        // 項目のない手のままでは適用できない
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
        for id in ["copy", "zoom_in", "toggle_compact", "close"] {
            assert!(ids.contains(&id), "{id} in {ids:?}");
        }
        // モードを切り替える操作、引数を取る操作、ドラッグ専用の操作は出さない
        for id in ["trim", "annotate", "zoom", "opacity", "move"] {
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

    /// 操作の並びが `ids` のレシピ 1 つを持つ写し（ほかに適用を止める問題はない）。
    fn with_steps(ids: &[&str]) -> Editor {
        let mut settings = Settings::default();
        settings.hotkeys.select_capture = Some(chord("Ctrl+Shift+A"));
        settings.recipes.push(Recipe {
            name: "R".to_owned(),
            steps: ids.iter().map(|&id| id.into()).collect(),
            shortcut: None,
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

    /// レシピ 1 つに、全部の項目が「変えない」の見た目の手を 1 つ足した写し。
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
        // 値のない手には欄がない
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
            Some(Mark::Problem(Problem::CloseNotLast))
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
}
