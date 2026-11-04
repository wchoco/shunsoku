//! レシピ（名前を付けた、スクラップ操作の並び）。ADR-0007。

use crate::keys::Chord;
use crate::scrap::ScrapModel;
use crate::scrap::ops::{Effect, InRecipe, OpContext, OperationDescriptor, OperationId};

/// 設定ファイルの `[[recipes]]` の 1 つ。中身の検査はまだしていない（書いたまま）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recipe {
    pub name: String,
    /// 手の並び。
    pub steps: Vec<Step>,
    pub shortcut: Option<Chord>,
}

/// 値のある手の操作の id（設定ファイルの `op = "appearance"`）。ADR-0008。
pub const APPEARANCE: &str = "appearance";

/// レシピの手 1 つ（設定ファイルの `steps` の要素）。中身の検査はまだしていない（書いたまま）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// 値のない手。スクラップ操作の id（設定ファイルでは文字列）。
    Op(String),
    /// 値のある手。見た目を指定する操作（`APPEARANCE`）と、その値（設定ファイルではインライン表）。
    Appearance(AppearanceStep),
}

impl Step {
    /// 手が呼ぶ操作の id。
    pub fn op_id(&self) -> &str {
        match self {
            Step::Op(id) => id,
            Step::Appearance(_) => APPEARANCE,
        }
    }

    /// 手に書いた値。値のない手は None。
    pub fn values(&self) -> Option<&AppearanceStep> {
        match self {
            Step::Op(_) => None,
            Step::Appearance(values) => Some(values),
        }
    }
}

impl From<&str> for Step {
    /// 値のない手。
    fn from(id: &str) -> Step {
        Step::Op(id.to_owned())
    }
}

/// 見た目を指定する手の値（ADR-0008）。書いた項目だけを持つ。項目（透明度・枠・枠の色・枠の太さ・影）は、
/// 見た目を指定する操作を足すときに足す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppearanceStep {}

/// 検査を通った手 1 つ。
#[derive(Debug, Clone)]
pub struct PlannedStep {
    pub op: &'static OperationDescriptor,
    /// 手に書いた値。値のない手は None。
    pub values: Option<AppearanceStep>,
}

/// 検査を通ったレシピ。ショートカットから実行できる。
#[derive(Debug, Clone)]
pub struct Plan {
    /// `Settings::recipes` の中の位置。
    pub index: usize,
    pub name: String,
    pub steps: Vec<PlannedStep>,
    pub shortcut: Option<Chord>,
}

/// レシピの問題（設定のエラー）。問題のあるレシピは実行できず、ショートカットにも付けない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// 名前が空（空白だけを含む）。
    EmptyName,
    /// 名前がほかのレシピと同じ。同じ名前のすべてのレシピに付ける。
    DuplicateName,
    /// 操作の並びが空。
    NoSteps,
    /// レシピに入れられない操作（表にない、`InRecipe::Never`、別のレシピ）。書いた id。
    NotAStep(String),
    /// 最後にだけ置ける操作（`InRecipe::Last`。`close`）が最後以外にある。
    CloseNotLast,
}

impl Problem {
    /// 問題が名前の欄にあるか（ないなら操作の並びの欄）。
    pub fn is_about_name(&self) -> bool {
        matches!(self, Problem::EmptyName | Problem::DuplicateName)
    }
}

/// レシピの検査の結果（`plan`）。
#[derive(Debug, Clone, Default)]
pub struct Checked {
    /// 検査を通ったレシピ（設定の順）。
    pub plans: Vec<Plan>,
    /// 問題と、そのレシピの `Settings::recipes` の中の位置（設定の順）。
    pub problems: Vec<(usize, Problem)>,
}

/// レシピに入れられる操作か。引数のない操作（ショートカットを持てるもの）のうち、`InRecipe::Never` でないもの。
pub fn can_be_step(op: &OperationDescriptor) -> bool {
    op.takes_binding() && op.in_recipe != InRecipe::Never
}

/// 最後にだけ置ける操作（`InRecipe::Last`）の id か。`ops` は `plan` と同じ表。
pub fn is_last_only(id: &str, ops: &[OperationDescriptor]) -> bool {
    ops.iter()
        .any(|op| op.id == id && op.in_recipe == InRecipe::Last)
}

/// レシピを検査し、`ops`（ふつうは `scrap::ops::OPERATIONS`）の行に結び付ける。同じ操作を 2 回以上入れるのは
/// 認める。
pub fn plan(recipes: &[Recipe], ops: &'static [OperationDescriptor]) -> Checked {
    let mut checked = Checked::default();
    for (index, recipe) in recipes.iter().enumerate() {
        let mut problems = Vec::new();
        if recipe.name.trim().is_empty() {
            problems.push(Problem::EmptyName);
        } else if recipes
            .iter()
            .enumerate()
            .any(|(j, other)| j != index && other.name == recipe.name)
        {
            problems.push(Problem::DuplicateName);
        }
        if recipe.steps.is_empty() {
            problems.push(Problem::NoSteps);
        }
        let mut steps = Vec::with_capacity(recipe.steps.len());
        for step in &recipe.steps {
            let id = step.op_id();
            match ops.iter().find(|op| op.id == id && can_be_step(op)) {
                Some(op) => steps.push(PlannedStep {
                    op,
                    values: step.values().copied(),
                }),
                None => problems.push(Problem::NotAStep(id.to_owned())),
            }
        }
        if let Some((_, before_last)) = recipe.steps.split_last()
            && before_last.iter().any(|s| is_last_only(s.op_id(), ops))
        {
            problems.push(Problem::CloseNotLast);
        }
        if problems.is_empty() {
            checked.plans.push(Plan {
                index,
                name: recipe.name.clone(),
                steps,
                shortcut: recipe.shortcut,
            });
        } else {
            checked
                .problems
                .extend(problems.into_iter().map(|p| (index, p)));
        }
    }
    checked
}

/// 1 手の結果。Win32 層がその手の `Effect` を実行して決め、`Run::settle` で返す。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    /// 成功した。結果を返さない `Effect`（移動、描き直しなど）もこれ。
    #[default]
    Succeeded,
    /// 取り消した（名前を付けて保存のダイアログなど）。通知は出さない。
    Cancelled,
    /// 失敗した。通知は `Effect` を実行した側が出す。
    Failed,
}

impl Outcome {
    /// 1 手の `Effect` ごとの結果をまとめた、その手の結果。いちばん悪いもの（失敗 → 取り消し → 成功）。
    /// `Effect` がなければ成功。
    pub fn of(outcomes: impl IntoIterator<Item = Outcome>) -> Outcome {
        outcomes.into_iter().max().unwrap_or(Outcome::Succeeded)
    }
}

/// レシピが止まったわけ。どれも、その手（id）より後ろの手は実行していない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// その手は今は実行できなかった（`can_execute` が偽）。実行していない。
    CannotRun(OperationId),
    /// その手が失敗した。
    Failed(OperationId),
    /// その手が取り消された。
    Cancelled(OperationId),
}

/// `Run::advance` の結果。
#[derive(Debug, Clone)]
pub enum Advance {
    /// 次の手を実行した（`ScrapModel` を変えた）。Win32 層は `effects` を実行し、その結果を `Run::settle` で
    /// 返す。結果を返すまで、次の手へは進まない。
    Step {
        op: &'static OperationDescriptor,
        effects: Vec<Effect>,
    },
    /// その手の結果をまだ受けていない（何もしていない）。
    Waiting(OperationId),
    /// 全部の手が成功した。
    Finished,
    /// 止まった。
    Stopped(Stop),
}

/// 手は id で比べる（`OperationDescriptor` は関数を持つので比べない）。
impl PartialEq for Advance {
    fn eq(&self, other: &Advance) -> bool {
        match (self, other) {
            (
                Advance::Step { op, effects },
                Advance::Step {
                    op: other_op,
                    effects: other_effects,
                },
            ) => op.id == other_op.id && effects == other_effects,
            (Advance::Waiting(a), Advance::Waiting(b)) => a == b,
            (Advance::Finished, Advance::Finished) => true,
            (Advance::Stopped(a), Advance::Stopped(b)) => a == b,
            _ => false,
        }
    }
}

/// 走っているレシピの進み具合（ADR-0007）。1 手ずつ実行し、その手の結果を受けてから次の手へ進む。
///
/// Win32 層は `advance` で手を 1 つ実行させ、返った `Effect` を実行して、その結果を `settle` で返す。結果が
/// あとから届く `Effect`（裏のスレッドの保存など）は、届くまで `Run` を持っておき、届いたら `settle` してから
/// `advance` を続ける。`Effect` を実行する前にスクラップが閉じられたら、`Run` を捨てる。
#[derive(Debug, Clone)]
pub struct Run {
    steps: Vec<PlannedStep>,
    /// 次に実行する手の位置。
    next: usize,
    state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 次の手を実行できる。
    Ready,
    /// 実行した手（`next - 1`）の結果を待っている。
    Waiting,
    Stopped(Stop),
}

impl Run {
    pub fn new(steps: Vec<PlannedStep>) -> Run {
        Run {
            steps,
            next: 0,
            state: State::Ready,
        }
    }

    /// 次の手へ進む。前の手の結果を待っている間と、止まったあとは何もしない。
    pub fn advance(&mut self, model: &mut ScrapModel, context: &OpContext) -> Advance {
        match self.state {
            State::Stopped(stop) => return Advance::Stopped(stop),
            State::Waiting => return Advance::Waiting(self.steps[self.next - 1].op.id),
            State::Ready => {}
        }
        let Some(op) = self.steps.get(self.next).map(|s| s.op) else {
            return Advance::Finished;
        };
        if !(op.can_execute)(model) {
            let stop = Stop::CannotRun(op.id);
            self.state = State::Stopped(stop);
            return Advance::Stopped(stop);
        }
        self.next += 1;
        self.state = State::Waiting;
        Advance::Step {
            op,
            effects: (op.run)(model, context),
        }
    }

    /// 結果を待っている手の id。
    pub fn waiting_for(&self) -> Option<OperationId> {
        (self.state == State::Waiting).then(|| self.steps[self.next - 1].op.id)
    }

    /// 実行した手の結果を受ける。成功なら次の手へ進めるようにし、失敗か取り消しなら止める。結果を待って
    /// いなければ何もしない。
    pub fn settle(&mut self, outcome: Outcome) {
        let Some(id) = self.waiting_for() else {
            return;
        };
        self.state = match outcome {
            Outcome::Succeeded => State::Ready,
            Outcome::Failed => State::Stopped(Stop::Failed(id)),
            Outcome::Cancelled => State::Stopped(Stop::Cancelled(id)),
        };
    }
}

/// あとから届く結果（裏のスレッドの保存）を待っているレシピ。待たせるのは 1 つだけ。
///
/// Win32 層は、結果を待つ手を実行したら `Run` をここに預け、結果の知らせ（`ticket`。保存先など）が届いたら
/// `resume` で取り出して `Run::settle` し、`Run::advance` を続ける。待っている間にそのスクラップを閉じたら
/// `discard_for`、ほかのレシピを始めたら `discard` で捨てる（捨てたレシピは、結果が届いても進めない）。
/// `S` はスクラップを、`T` は結果の知らせを見分けるもの。
#[derive(Debug, Clone)]
pub struct Parked<S, T> {
    waiting: Option<(S, T, Run)>,
}

impl<S, T> Default for Parked<S, T> {
    fn default() -> Parked<S, T> {
        Parked { waiting: None }
    }
}

impl<S: PartialEq, T: PartialEq> Parked<S, T> {
    /// `scrap` のレシピ `run` を、`ticket` の結果が届くまで待たせる。前に待っていたレシピがあれば捨てて返す。
    pub fn park(&mut self, scrap: S, ticket: T, run: Run) -> Option<Run> {
        self.waiting
            .replace((scrap, ticket, run))
            .map(|(_, _, run)| run)
    }

    /// `ticket` の結果を待っていたレシピを取り出す（スクラップと一緒に）。待っていなければ None。
    pub fn resume(&mut self, ticket: &T) -> Option<(S, Run)> {
        match &self.waiting {
            Some((_, waiting, _)) if waiting == ticket => {
                self.waiting.take().map(|(scrap, _, run)| (scrap, run))
            }
            _ => None,
        }
    }

    /// `scrap` のレシピが待っていれば捨てて返す。
    pub fn discard_for(&mut self, scrap: &S) -> Option<Run> {
        match &self.waiting {
            Some((waiting, _, _)) if waiting == scrap => self.discard(),
            _ => None,
        }
    }

    /// 待っているレシピを捨てて返す。
    pub fn discard(&mut self) -> Option<Run> {
        self.waiting.take().map(|(_, _, run)| run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Point, Size};
    use crate::scrap::ScrapModel;
    use crate::scrap::ops::{Effect, OPERATIONS, OpContext};
    use crate::settings::{self, Keymap, Settings, SettingsIssue, SettingsItem, ValidationIssue};

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    fn invalid(item: &str) -> SettingsIssue {
        SettingsIssue::Invalid {
            item: item.to_owned(),
        }
    }

    fn recipe(name: &str, steps: &[&str]) -> Recipe {
        Recipe {
            name: name.into(),
            steps: steps.iter().map(|&s| s.into()).collect(),
            shortcut: None,
        }
    }

    fn ids(plan: &Plan) -> Vec<&'static str> {
        plan.steps.iter().map(|s| s.op.id).collect()
    }

    #[test]
    fn a_recipe_of_operations_without_arguments_becomes_a_plan_and_may_repeat_one() {
        let checked = plan(
            &[
                recipe("コピーして閉じる", &["copy", "close"]),
                recipe("二度", &["copy", "zoom_in", "copy"]),
            ],
            OPERATIONS,
        );
        assert_eq!(checked.problems, vec![]);
        assert_eq!(checked.plans.len(), 2);
        assert_eq!(checked.plans[0].index, 0);
        assert_eq!(ids(&checked.plans[0]), ["copy", "close"]);
        assert_eq!(checked.plans[1].index, 1);
        assert_eq!(ids(&checked.plans[1]), ["copy", "zoom_in", "copy"]);
    }

    #[test]
    fn steps_that_cannot_go_in_a_recipe_make_it_an_error() {
        // trim と annotate はモードを変える。zoom は選択肢を持ち、move はドラッグ専用。fly は表にない（別の
        // レシピの名前も同じ）
        for step in [
            "trim",
            "annotate",
            "zoom",
            "move",
            "fly",
            "コピーして閉じる",
        ] {
            let checked = plan(
                &[
                    recipe("コピーして閉じる", &["copy", "close"]),
                    recipe("r", &["copy", step]),
                ],
                OPERATIONS,
            );
            assert_eq!(
                checked.problems,
                vec![(1, Problem::NotAStep(step.to_owned()))],
                "{step}"
            );
            assert_eq!(
                checked.plans.iter().map(|p| p.index).collect::<Vec<_>>(),
                [0],
                "{step}"
            );
        }
    }

    #[test]
    fn saves_go_in_a_recipe() {
        let checked = plan(
            &[
                recipe("保存して閉じる", &["save_default", "close"]),
                recipe("名前を付けて保存して閉じる", &["save_as", "close"]),
            ],
            OPERATIONS,
        );
        assert_eq!(checked.problems, vec![]);
        assert_eq!(ids(&checked.plans[0]), ["save_default", "close"]);
        assert_eq!(ids(&checked.plans[1]), ["save_as", "close"]);
    }

    #[test]
    fn close_goes_only_at_the_end() {
        let checked = plan(
            &[
                recipe("a", &["close", "copy"]),
                recipe("b", &["close", "close"]),
                recipe("c", &["close"]),
            ],
            OPERATIONS,
        );
        assert_eq!(
            checked.problems,
            vec![(0, Problem::CloseNotLast), (1, Problem::CloseNotLast)]
        );
        assert_eq!(
            checked.plans.iter().map(|p| p.index).collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn empty_names_empty_steps_and_shared_names_are_errors() {
        let checked = plan(
            &[
                recipe(" ", &["copy"]),
                recipe("空", &[]),
                recipe("同じ", &["copy"]),
                recipe("同じ", &["close"]),
                recipe("", &[]),
            ],
            OPERATIONS,
        );
        assert_eq!(
            checked.problems,
            vec![
                (0, Problem::EmptyName),
                (1, Problem::NoSteps),
                (2, Problem::DuplicateName),
                (3, Problem::DuplicateName),
                (4, Problem::EmptyName),
                (4, Problem::NoSteps),
            ]
        );
        assert!(checked.plans.is_empty());
    }

    fn keymap_of(src: &str) -> Keymap {
        let (settings, issues) = settings::load(src);
        assert_eq!(issues, vec![], "{src}");
        settings::keymap(&settings, OPERATIONS)
    }

    fn bound_recipe(keymap: &Keymap, s: &str) -> Option<String> {
        keymap
            .shortcuts
            .recipe(chord(s))
            .map(|plan| plan.name.clone())
    }

    const COPY_THEN_CLOSE: &str =
        "version = 1\n[[recipes]]\nname = \"cc\"\nsteps = [\"copy\", \"close\"]\n";

    #[test]
    fn a_recipe_shortcut_runs_the_recipe_and_not_an_operation() {
        let keymap = keymap_of(&format!("{COPY_THEN_CLOSE}shortcut = \"Ctrl+Shift+C\"\n"));
        assert_eq!(keymap.issues, vec![]);
        assert_eq!(bound_recipe(&keymap, "Ctrl+Shift+C"), Some("cc".to_owned()));
        assert!(keymap.shortcuts.operation(chord("Ctrl+Shift+C")).is_none());
        assert_eq!(
            keymap.shortcuts.recipe(chord("Ctrl+Shift+C")).map(|p| p
                .steps
                .iter()
                .map(|s| s.op.id)
                .collect::<Vec<_>>()),
            Some(vec!["copy", "close"])
        );
        // 操作のショートカットはそのまま
        assert_eq!(
            keymap.shortcuts.operation(chord("Ctrl+C")).map(|op| op.id),
            Some("copy")
        );
        assert_eq!(bound_recipe(&keymap, "Ctrl+C"), None);
    }

    #[test]
    fn a_recipe_shortcut_on_an_operation_or_another_recipe_disables_both() {
        let keymap = keymap_of(&format!(
            "{COPY_THEN_CLOSE}shortcut = \"Ctrl+C\"\n[[recipes]]\nname = \"a\"\nsteps = [\"copy\"]\nshortcut = \"Q\"\n[[recipes]]\nname = \"b\"\nsteps = [\"copy\"]\nshortcut = \"Q\"\n"
        ));
        assert_eq!(bound_recipe(&keymap, "Ctrl+C"), None);
        assert!(keymap.shortcuts.operation(chord("Ctrl+C")).is_none());
        assert_eq!(bound_recipe(&keymap, "Q"), None);
        assert_eq!(
            keymap.issues,
            vec![
                SettingsIssue::Conflict {
                    chord: chord("Q"),
                    items: vec![
                        SettingsItem::RecipeShortcut(1),
                        SettingsItem::RecipeShortcut(2)
                    ],
                },
                SettingsIssue::Conflict {
                    chord: chord("Ctrl+C"),
                    items: vec![
                        SettingsItem::Shortcut("copy".into()),
                        SettingsItem::RecipeShortcut(0)
                    ],
                },
            ]
        );
    }

    #[test]
    fn a_hotkey_on_a_recipe_shortcut_is_only_a_warning() {
        let keymap = keymap_of(&format!(
            "{COPY_THEN_CLOSE}shortcut = \"Ctrl+Q\"\n[hotkeys]\nselect_capture = \"Ctrl+Q\"\n"
        ));
        assert_eq!(bound_recipe(&keymap, "Ctrl+Q"), Some("cc".to_owned()));
        assert_eq!(
            keymap.issues,
            vec![SettingsIssue::Overlap {
                chord: chord("Ctrl+Q"),
                hotkey: SettingsItem::SelectCaptureHotkey,
                shortcut: SettingsItem::RecipeShortcut(0),
            }]
        );
    }

    #[test]
    fn a_recipe_with_problems_is_named_as_invalid_and_takes_no_shortcut() {
        let keymap = keymap_of(
            "version = 1\n[[recipes]]\nname = \"\"\nsteps = [\"close\", \"trim\"]\nshortcut = \"Ctrl+C\"\n",
        );
        assert_eq!(bound_recipe(&keymap, "Ctrl+C"), None);
        // 操作と重ならない（問題のあるレシピは割り当てに加わらない）
        assert_eq!(
            keymap.shortcuts.operation(chord("Ctrl+C")).map(|op| op.id),
            Some("copy")
        );
        assert_eq!(
            keymap.issues,
            vec![invalid("recipes[1].name"), invalid("recipes[1].steps")]
        );
    }

    fn issue(item: SettingsItem, problem: settings::Problem) -> ValidationIssue {
        ValidationIssue { item, problem }
    }

    #[test]
    fn validate_blocks_apply_on_recipes_with_problems_and_shared_shortcuts() {
        let mut s = Settings::default();
        s.hotkeys.select_capture = Some(chord("Ctrl+Shift+A"));
        s.recipes = vec![
            recipe("", &["copy"]),
            recipe("同じ", &["close", "copy"]),
            recipe("同じ", &[]),
            recipe("t", &["trim"]),
            Recipe {
                shortcut: Some(chord("Ctrl+C")),
                ..recipe("ok", &["copy"])
            },
        ];
        let issues = settings::validate(&s);
        let conflict = settings::Problem::Conflict(chord("Ctrl+C"));
        assert_eq!(
            issues,
            vec![
                issue(SettingsItem::Shortcut("copy".into()), conflict),
                issue(SettingsItem::RecipeShortcut(4), conflict),
                issue(SettingsItem::RecipeName(0), settings::Problem::EmptyName),
                issue(
                    SettingsItem::RecipeName(1),
                    settings::Problem::DuplicateName
                ),
                issue(
                    SettingsItem::RecipeSteps(1),
                    settings::Problem::CloseNotLast
                ),
                issue(
                    SettingsItem::RecipeName(2),
                    settings::Problem::DuplicateName
                ),
                issue(SettingsItem::RecipeSteps(2), settings::Problem::NoSteps),
                issue(SettingsItem::RecipeSteps(3), settings::Problem::NotAStep),
            ]
        );
        assert!(issues.iter().all(ValidationIssue::blocks_apply));
    }

    #[test]
    fn changed_recipes_rebuild_the_shortcuts() {
        let old = Settings::default();
        let mut new = old.clone();
        new.recipes.push(recipe("a", &["copy"]));
        let changes = settings::diff(&old, &new);
        assert!(changes.shortcuts && !changes.is_empty());
    }

    fn run_of(steps: &[&str]) -> Run {
        let checked = plan(&[recipe("r", steps)], OPERATIONS);
        assert_eq!(checked.problems, vec![]);
        Run::new(checked.plans[0].steps.clone())
    }

    fn model() -> ScrapModel {
        ScrapModel::new(Point::new(0, 0), Size::new(300, 200))
    }

    /// 次の手を実行し、その手の id と Effect を返す。手でなければ panic。
    fn step(run: &mut Run, model: &mut ScrapModel) -> (&'static str, Vec<Effect>) {
        match run.advance(model, &OpContext::default()) {
            Advance::Step { op, effects } => (op.id, effects),
            other => panic!("not a step: {other:?}"),
        }
    }

    #[test]
    fn copy_then_close_runs_one_step_at_a_time_on_success() {
        let mut run = run_of(&["copy", "close"]);
        let mut model = model();
        assert_eq!(
            step(&mut run, &mut model),
            ("copy", vec![Effect::CopyToClipboard])
        );
        // 結果を受けるまで次の手へ進まない
        assert_eq!(run.waiting_for(), Some("copy"));
        assert!(matches!(
            run.advance(&mut model, &OpContext::default()),
            Advance::Waiting("copy")
        ));
        run.settle(Outcome::Succeeded);
        assert_eq!(run.waiting_for(), None);
        assert_eq!(
            step(&mut run, &mut model),
            ("close", vec![Effect::MoveToTrash])
        );
        run.settle(Outcome::Succeeded);
        assert!(matches!(
            run.advance(&mut model, &OpContext::default()),
            Advance::Finished
        ));
    }

    #[test]
    fn a_failed_or_cancelled_step_stops_the_recipe_before_close() {
        for (outcome, stop) in [
            (Outcome::Failed, Stop::Failed("copy")),
            (Outcome::Cancelled, Stop::Cancelled("copy")),
        ] {
            let mut run = run_of(&["copy", "close"]);
            let mut model = model();
            step(&mut run, &mut model);
            run.settle(outcome);
            assert_eq!(
                run.advance(&mut model, &OpContext::default()),
                Advance::Stopped(stop)
            );
            // 止まったまま
            assert_eq!(
                run.advance(&mut model, &OpContext::default()),
                Advance::Stopped(stop)
            );
        }
    }

    #[test]
    fn save_then_close_closes_only_after_the_save_succeeds() {
        for (save, effect) in [
            ("save_default", Effect::SaveDefault),
            ("save_as", Effect::ShowSaveDialog),
        ] {
            let mut run = run_of(&[save, "close"]);
            let mut model = model();
            assert_eq!(step(&mut run, &mut model), (save, vec![effect]));
            // 書き込みの知らせが届くまでは閉じない
            assert_eq!(
                run.advance(&mut model, &OpContext::default()),
                Advance::Waiting(save)
            );
            run.settle(Outcome::Succeeded);
            assert_eq!(
                step(&mut run, &mut model),
                ("close", vec![Effect::MoveToTrash])
            );
        }
    }

    #[test]
    fn a_failed_or_cancelled_save_leaves_the_scrap_open() {
        for (save, outcome, stop) in [
            (
                "save_default",
                Outcome::Failed,
                Stop::Failed("save_default"),
            ),
            ("save_as", Outcome::Failed, Stop::Failed("save_as")),
            ("save_as", Outcome::Cancelled, Stop::Cancelled("save_as")),
        ] {
            let mut run = run_of(&[save, "close"]);
            let mut model = model();
            step(&mut run, &mut model);
            run.settle(outcome);
            assert_eq!(
                run.advance(&mut model, &OpContext::default()),
                Advance::Stopped(stop)
            );
        }
    }

    /// 保存を待っているレシピ。スクラップは 1 から、保存の知らせは "a.png" などで見分ける。
    fn parked_save(scrap: u32, ticket: &str) -> Parked<u32, String> {
        let mut run = run_of(&["save_default", "close"]);
        step(&mut run, &mut model());
        let mut parked = Parked::default();
        assert!(parked.park(scrap, ticket.to_owned(), run).is_none());
        parked
    }

    #[test]
    fn the_save_result_resumes_the_recipe_waiting_for_it() {
        let mut parked = parked_save(1, "a.png");
        // 別の保存の知らせでは進めない
        assert!(parked.resume(&"b.png".to_owned()).is_none());
        let (scrap, mut run) = parked.resume(&"a.png".to_owned()).unwrap();
        assert_eq!(scrap, 1);
        assert_eq!(run.waiting_for(), Some("save_default"));
        run.settle(Outcome::Succeeded);
        assert_eq!(
            step(&mut run, &mut model()),
            ("close", vec![Effect::MoveToTrash])
        );
        // 1 度しか進めない
        assert!(parked.resume(&"a.png".to_owned()).is_none());
    }

    #[test]
    fn closing_the_scrap_discards_the_recipe_waiting_on_it() {
        let mut parked = parked_save(1, "a.png");
        assert!(parked.discard_for(&2).is_none());
        assert!(parked.discard_for(&1).is_some());
        assert!(parked.resume(&"a.png".to_owned()).is_none());
    }

    #[test]
    fn starting_another_recipe_discards_the_waiting_one() {
        let mut parked = parked_save(1, "a.png");
        assert!(parked.discard().is_some());
        assert!(parked.discard().is_none());
        assert!(parked.resume(&"a.png".to_owned()).is_none());
        // 待たせるのも 1 つだけ。前のは捨てて返す
        let mut parked = parked_save(1, "a.png");
        let mut run = run_of(&["save_as"]);
        step(&mut run, &mut model());
        assert!(parked.park(2, "b.png".to_owned(), run).is_some());
        assert!(parked.resume(&"a.png".to_owned()).is_none());
        assert_eq!(parked.resume(&"b.png".to_owned()).map(|(s, _)| s), Some(2));
    }

    #[test]
    fn a_step_that_cannot_run_now_stops_the_recipe_without_running_it() {
        let mut run = run_of(&["zoom_in", "copy"]);
        let mut model = model();
        assert!(model.start_trim());
        let before = model.clone();
        assert_eq!(
            run.advance(&mut model, &OpContext::default()),
            Advance::Stopped(Stop::CannotRun("zoom_in"))
        );
        assert_eq!(model, before);
    }

    #[test]
    fn steps_change_the_model_like_a_shortcut_would() {
        let mut run = run_of(&["zoom_in", "zoom_in"]);
        let mut model = model();
        let (_, effects) = step(&mut run, &mut model);
        assert_eq!(effects, vec![Effect::Redraw, Effect::ShowZoom]);
        run.settle(Outcome::Succeeded);
        step(&mut run, &mut model);
        run.settle(Outcome::Succeeded);
        assert!(model.zoom() > 100);
        assert_eq!(
            run.advance(&mut model, &OpContext::default()),
            Advance::Finished
        );
    }

    #[test]
    fn the_outcome_of_a_step_is_its_worst_effect() {
        use Outcome::*;
        assert_eq!(Outcome::of([]), Succeeded);
        assert_eq!(Outcome::of([Succeeded, Succeeded]), Succeeded);
        assert_eq!(Outcome::of([Succeeded, Cancelled]), Cancelled);
        assert_eq!(Outcome::of([Cancelled, Failed, Succeeded]), Failed);
    }

    #[test]
    fn recipes_are_read_in_file_order() {
        let (settings, issues) = settings::load(
            r#"version = 1
[[recipes]]
name = "コピーして閉じる"
steps = ["copy", "close"]
shortcut = "Ctrl+Shift+C"
[[recipes]]
name = "二度コピー"
steps = ["copy", "copy"]
"#,
        );
        assert_eq!(issues, vec![]);
        assert_eq!(
            settings.recipes,
            vec![
                Recipe {
                    name: "コピーして閉じる".into(),
                    steps: vec!["copy".into(), "close".into()],
                    shortcut: Some(chord("Ctrl+Shift+C")),
                },
                Recipe {
                    name: "二度コピー".into(),
                    steps: vec!["copy".into(), "copy".into()],
                    shortcut: None,
                },
            ]
        );
    }

    #[test]
    fn unreadable_recipe_items_are_named_and_the_recipe_is_dropped_or_left_unbound() {
        let (settings, issues) = settings::load(
            r#"version = 1
[[recipes]]
name = 1
steps = ["copy"]
[[recipes]]
name = "a"
steps = "copy"
[[recipes]]
name = "b"
steps = ["copy"]
shortcut = "Ctrl+Nope"
"#,
        );
        assert_eq!(
            issues,
            vec![
                invalid("recipes[1].name"),
                invalid("recipes[2].steps"),
                invalid("recipes[3].shortcut"),
            ]
        );
        assert_eq!(
            settings.recipes,
            vec![Recipe {
                name: "b".into(),
                steps: vec!["copy".into()],
                shortcut: None,
            }]
        );
    }
}
