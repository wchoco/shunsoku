//! レシピ（名前を付けた、スクラップ操作の並び）。ADR-0007。キャプチャで始まるレシピは ADR-0010。

use crate::keys::Chord;
use crate::scrap::ops::{Effect, InRecipe, OpContext, OperationDescriptor, OperationId};
use crate::scrap::{Background, ScrapModel};
use crate::settings::{BORDER_WIDTH_RANGE, OPACITY_RANGE, Region, Rgb};

/// 設定ファイルの `[[recipes]]` の 1 つ。中身の検査はまだしていない（書いたまま）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recipe {
    pub name: String,
    /// 操作の並び。
    pub steps: Vec<Step>,
    /// スクラップのショートカット。キャプチャで始まらないレシピだけが持てる。
    pub shortcut: Option<Chord>,
    /// ホットキー。キャプチャで始まるレシピだけが持てる（ADR-0010）。
    pub hotkey: Option<Chord>,
}

/// キャプチャ系の操作（ADR-0010）。レシピの最初にだけ置け、スクラップを作る。スクラップに対する操作ではないので、
/// スクラップ操作の表（`scrap::ops::OPERATIONS`）には入れない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// 範囲選択キャプチャ。範囲選択の画面で選んだ範囲か、クリックしたウィンドウを撮る。
    Select,
    /// ウィンドウキャプチャ。いまアクティブなウィンドウを撮る。
    ActiveWindow,
    /// 白紙スクラップを作る。範囲選択の画面で位置と大きさを選ぶ（画面は取り込まない）。
    Blank,
    /// 定型領域キャプチャ。パラメータで選んだ定型領域（`[[regions]]`）を撮る（`Step::CaptureRegion`）。
    Region,
}

impl Capture {
    /// キャプチャ系の操作のすべて（「追加」の一覧の順）。
    pub const ALL: [Capture; 4] = [
        Capture::Select,
        Capture::ActiveWindow,
        Capture::Blank,
        Capture::Region,
    ];

    /// レシピの `steps` に書く id。
    pub fn id(self) -> &'static str {
        match self {
            Capture::Select => "select_capture",
            Capture::ActiveWindow => "active_window_capture",
            Capture::Blank => "blank_scrap",
            Capture::Region => "capture_region",
        }
    }

    /// `id` のキャプチャ系の操作。
    pub fn from_id(id: &str) -> Option<Capture> {
        Capture::ALL.into_iter().find(|c| c.id() == id)
    }

    /// 設定 GUI の操作の名前の文言のキー。
    pub fn label_key(self) -> &'static str {
        match self {
            Capture::Select => "recipe.capture.select_capture",
            Capture::ActiveWindow => "recipe.capture.active_window_capture",
            Capture::Blank => "recipe.capture.blank_scrap",
            Capture::Region => "recipe.capture.capture_region",
        }
    }
}

/// 最初にだけ置ける操作（キャプチャ系）の id か。最後にだけ置ける操作（`is_last_only`）と対になる。
pub fn is_first_only(id: &str) -> bool {
    Capture::from_id(id).is_some()
}

/// 検査を通ったキャプチャ系の操作。パラメータを添えるもの（定型領域キャプチャ）は、その値も持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedCapture {
    /// 範囲選択キャプチャ（`Capture::Select`）。
    Select,
    /// ウィンドウキャプチャ（`Capture::ActiveWindow`）。
    ActiveWindow,
    /// 白紙スクラップを作る（`Capture::Blank`）。
    Blank,
    /// 定型領域キャプチャ（`Capture::Region`）と、撮る定型領域の名前（`Region::name`）。
    Region(String),
}

/// 書き込みの編集モードに入る操作の id（スクラップ操作 `annotate`）。レシピの最後では、入るだけで抜けない。
pub const ANNOTATE: &str = "annotate";

/// 見た目を指定する操作の id（設定ファイルの `op = "appearance"`）。ADR-0008。
pub const APPEARANCE: &str = "appearance";

/// 背景色を指定する操作の id（設定ファイルの `op = "background"`）。白紙スクラップだけで動く。
pub const BACKGROUND: &str = "background";

/// レシピの操作 1 つ（設定ファイルの `steps` の要素）。中身の検査はまだしていない（書いたまま）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// パラメータのない操作。スクラップ操作の id（設定ファイルでは文字列）。
    Op(String),
    /// パラメータを添えた操作。見た目を指定する操作（`APPEARANCE`）と、その値（設定ファイルではインライン表）。
    Appearance(AppearanceStep),
    /// パラメータを添えた操作。背景色を指定する操作（`BACKGROUND`）と、その色（設定ファイルではインライン表）。
    Background(BackgroundStep),
    /// パラメータを添えた操作。定型領域キャプチャ（`Capture::Region`）と、撮る定型領域の名前（設定ファイルでは
    /// インライン表）。
    CaptureRegion(CaptureRegionStep),
}

impl Step {
    /// 呼ぶ操作の id。
    pub fn op_id(&self) -> &str {
        match self {
            Step::Op(id) => id,
            Step::Appearance(_) => APPEARANCE,
            Step::Background(_) => BACKGROUND,
            Step::CaptureRegion(_) => Capture::Region.id(),
        }
    }

    /// 見た目を指定する操作に添えたパラメータ。ほかの操作は None。
    pub fn values(&self) -> Option<&AppearanceStep> {
        match self {
            Step::Op(_) | Step::Background(_) | Step::CaptureRegion(_) => None,
            Step::Appearance(values) => Some(values),
        }
    }
}

impl From<&str> for Step {
    /// パラメータのない操作。
    fn from(id: &str) -> Step {
        Step::Op(id.to_owned())
    }
}

/// 見た目を指定する操作（ADR-0008）。書いたまま（範囲と矛盾の検査は `plan` がする）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppearanceStep {
    /// 読めた項目。
    pub values: AppearanceValues,
    /// 読めなかった項目（書いた順）。どれか 1 つでもあれば、その操作はレシピの問題になる。
    pub rejected: Vec<Rejected>,
}

/// 背景色を指定する操作（`{ op = "background", color = "#RRGGBB" }`）。書いたまま（検査は `plan` がする）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackgroundStep {
    /// 読めた色（`Background` の書き方。設定ファイルの `[annotation] background` と同じ）。書いていないか
    /// 読めなければ None。
    pub color: Option<Background>,
    /// 読めなかった項目（書いた順）。どれか 1 つでもあれば、その操作はレシピの問題になる。
    pub rejected: Vec<Rejected>,
}

impl BackgroundStep {
    /// `color` に書いた値を読む。読めなければ色を「書いていない」にして、そのわけを返す。
    pub fn set_color(&mut self, written: Written) -> Result<(), Rejection> {
        self.color = None;
        let Written::Text(s) = written else {
            return Err(Rejection::BadValue);
        };
        self.color = Some(s.parse().map_err(|_| Rejection::BadValue)?);
        Ok(())
    }
}

/// 定型領域キャプチャ（`{ op = "capture_region", region = "<名前>" }`）。書いたまま（定型領域があるかの検査は
/// `plan` がする）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CaptureRegionStep {
    /// 撮る定型領域の名前（`Region::name`）。書いていないか読めなければ None。
    pub region: Option<String>,
    /// 読めなかった項目（書いた順）。どれか 1 つでもあれば、その操作はレシピの問題になる。
    pub rejected: Vec<Rejected>,
}

/// 検査を通った操作に添えたパラメータ。実行するときに `OpContext::values` で操作へ渡す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepValues {
    /// 見た目を指定する操作（`APPEARANCE`）。
    Appearance(AppearanceValues),
    /// 背景色を指定する操作（`BACKGROUND`）。
    Background(Background),
}

/// 見た目を指定する操作に添えたパラメータ。書いた項目だけが Some。キーと値の範囲は設定ファイルの `[scrap]` と同じ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppearanceValues {
    /// 透明度（%、`settings::OPACITY_RANGE`）。
    pub opacity: Option<u8>,
    /// 枠を出すか。
    pub border: Option<bool>,
    /// 枠の色。書けば枠も出す。
    pub border_color: Option<Rgb>,
    /// 枠の太さ（物理ピクセル、`settings::BORDER_WIDTH_RANGE`）。書けば枠も出す。キーと値は `[scrap] border_width`
    /// と同じで、物理ピクセルのまま持つわけも同じ（`AppearanceOverrides::border_width`）。
    pub border_width: Option<u32>,
    /// 影の有無。
    pub shadow: Option<bool>,
}

impl AppearanceValues {
    /// 項目を 1 つも書いていない。
    pub fn is_empty(&self) -> bool {
        *self == AppearanceValues::default()
    }

    /// 項目 `item` に書いた値。「変えない」は None。
    pub fn get(&self, item: AppearanceItem) -> Option<ItemValue> {
        match item {
            AppearanceItem::Border => self.border.map(ItemValue::Switch),
            AppearanceItem::BorderColor => self.border_color.map(ItemValue::Color),
            AppearanceItem::BorderWidth => self.border_width.map(ItemValue::Number),
            AppearanceItem::Opacity => self.opacity.map(|o| ItemValue::Number(u32::from(o))),
            AppearanceItem::Shadow => self.shadow.map(ItemValue::Switch),
        }
    }

    /// 項目 `item` を、書いた値 `written` にする。値の型と範囲は設定ファイルの `[scrap]` と同じ。読めなければ
    /// 項目を「変えない」にして、そのわけを返す。設定ファイルの読み込みと設定 GUI の欄の両方がこれで読む。
    pub fn set(&mut self, item: AppearanceItem, written: Written) -> Result<(), Rejection> {
        self.clear(item);
        match (item, written) {
            (AppearanceItem::Border, Written::Bool(on)) => self.border = Some(on),
            (AppearanceItem::Shadow, Written::Bool(on)) => self.shadow = Some(on),
            (AppearanceItem::BorderColor, Written::Text(s)) => {
                self.border_color = Some(s.parse().map_err(|_| Rejection::BadValue)?);
            }
            (AppearanceItem::BorderWidth, Written::Integer(n)) => {
                self.border_width = Some(ranged(n, BORDER_WIDTH_RANGE)?);
            }
            (AppearanceItem::Opacity, Written::Integer(n)) => {
                self.opacity = Some(ranged(n, OPACITY_RANGE)?);
            }
            (
                AppearanceItem::Border
                | AppearanceItem::Shadow
                | AppearanceItem::BorderColor
                | AppearanceItem::BorderWidth
                | AppearanceItem::Opacity,
                _,
            ) => return Err(Rejection::BadValue),
        }
        Ok(())
    }

    /// 項目 `item` を「変えない」にする。
    pub fn clear(&mut self, item: AppearanceItem) {
        match item {
            AppearanceItem::Border => self.border = None,
            AppearanceItem::BorderColor => self.border_color = None,
            AppearanceItem::BorderWidth => self.border_width = None,
            AppearanceItem::Opacity => self.opacity = None,
            AppearanceItem::Shadow => self.shadow = None,
        }
    }
}

/// 整数 `n` が `range` に入っていればその値。入っていなければ `OutOfRange`。
fn ranged<T: TryFrom<i64> + PartialOrd>(
    n: i64,
    range: std::ops::RangeInclusive<T>,
) -> Result<T, Rejection> {
    T::try_from(n)
        .ok()
        .filter(|n| range.contains(n))
        .ok_or(Rejection::OutOfRange)
}

/// 見た目を指定する操作の項目。並びは設定ファイルに書くキーの順で、設定 GUI の欄の順。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceItem {
    /// 枠を出すか（`border`）。
    Border,
    /// 枠の色（`border_color`）。
    BorderColor,
    /// 枠の太さ（`border_width`）。
    BorderWidth,
    /// 透明度（`opacity`）。
    Opacity,
    /// 影の有無（`shadow`）。
    Shadow,
}

impl AppearanceItem {
    /// 項目の並び（設定ファイルのキーの順、設定 GUI の欄の順）。
    pub const ALL: [AppearanceItem; 5] = [
        AppearanceItem::Border,
        AppearanceItem::BorderColor,
        AppearanceItem::BorderWidth,
        AppearanceItem::Opacity,
        AppearanceItem::Shadow,
    ];

    /// 設定ファイルのパラメータのキー。
    pub fn key(self) -> &'static str {
        match self {
            AppearanceItem::Border => "border",
            AppearanceItem::BorderColor => "border_color",
            AppearanceItem::BorderWidth => "border_width",
            AppearanceItem::Opacity => "opacity",
            AppearanceItem::Shadow => "shadow",
        }
    }

    /// 設定ファイルのパラメータのキー `key` の項目。
    pub fn from_key(key: &str) -> Option<AppearanceItem> {
        AppearanceItem::ALL
            .into_iter()
            .find(|item| item.key() == key)
    }

    /// オン・オフの項目か（ほかは色か数）。
    pub fn is_switch(self) -> bool {
        match self {
            AppearanceItem::Border | AppearanceItem::Shadow => true,
            AppearanceItem::BorderColor | AppearanceItem::BorderWidth | AppearanceItem::Opacity => {
                false
            }
        }
    }
}

/// 項目に書いた値（`AppearanceValues::get`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemValue {
    /// オン・オフ（枠・影）。
    Switch(bool),
    /// 色（枠の色）。
    Color(Rgb),
    /// 数（枠の太さ・透明度）。
    Number(u32),
}

/// 項目に書いたまま値（`AppearanceValues::set`）。設定ファイルの値の型か、設定 GUI の欄の入力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written<'a> {
    Integer(i64),
    Bool(bool),
    Text(&'a str),
    /// ほかの型（小数、配列など）。
    Other,
}

impl Written<'_> {
    /// 設定ファイルに書く形。ほかの型は書けないので None。
    pub fn toml(self) -> Option<String> {
        match self {
            Written::Integer(n) => Some(n.to_string()),
            Written::Bool(on) => Some(on.to_string()),
            Written::Text(s) => Some(toml_edit::Value::from(s).to_string()),
            Written::Other => None,
        }
    }
}

/// 見た目を指定する操作に書いたキー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKey {
    /// `op`。
    Op,
    /// 見た目の項目のキー。
    Item(AppearanceItem),
    /// 背景色を指定する操作の色（`color`）。
    Color,
    /// 定型領域キャプチャの定型領域の名前（`region`）。
    Region,
    /// 知らないキー。
    Unknown(String),
}

impl StepKey {
    /// 設定ファイルに書くキー。
    pub fn as_str(&self) -> &str {
        match self {
            StepKey::Op => "op",
            StepKey::Item(item) => item.key(),
            StepKey::Color => "color",
            StepKey::Region => "region",
            StepKey::Unknown(key) => key,
        }
    }
}

/// 見た目を指定する操作の、読めなかった項目 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub key: StepKey,
    /// 書いた値（TOML の書き方のまま）。書き戻すときにそのまま書く。`op` を書いていないときは None。
    pub value: Option<String>,
    pub reason: Rejection,
}

/// 項目が読めなかったわけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// 範囲外の数。
    OutOfRange,
    /// 値の型か書式の誤り（数でない透明度、`#RRGGBB` でない色など）。
    BadValue,
    /// 知らないキー。
    UnknownKey,
    /// `op` が `appearance` でも `background` でもない（書いていないときも）。
    NotAppearance,
}

/// 検査を通った操作 1 つ。
#[derive(Debug, Clone)]
pub struct PlannedStep {
    pub op: &'static OperationDescriptor,
    /// 操作に添えたパラメータ。パラメータのない操作は None。実行するときに `OpContext::values` で操作へ渡す。
    pub values: Option<StepValues>,
}

/// 検査を通ったレシピ。キャプチャで始まるレシピはホットキーから、ほかのレシピはショートカットから実行できる。
#[derive(Debug, Clone)]
pub struct Plan {
    /// `Settings::recipes` の中の位置。
    pub index: usize,
    pub name: String,
    /// 最初に置いたキャプチャ系の操作。キャプチャで始まらないレシピは None。
    pub capture: Option<PlannedCapture>,
    /// スクラップ操作の並び（キャプチャ系の操作を除く）。
    pub steps: Vec<PlannedStep>,
    pub shortcut: Option<Chord>,
    pub hotkey: Option<Chord>,
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
    /// 最後にだけ置ける操作（`InRecipe::Last`。`close`、`annotate`）が最後以外にある。2 つ置いたときも、前の
    /// ほうが最後以外になるのでこれ。
    LastOnlyNotLast,
    /// 最初にだけ置ける操作（キャプチャ系。`is_first_only`）が最初以外にある。`LastOnlyNotLast` と対になる。
    FirstOnlyNotFirst,
    /// キャプチャで始まるレシピに `shortcut` がある（ホットキーで呼ぶ）。
    ShortcutOnCapture,
    /// キャプチャで始まるレシピに `hotkey` がない。
    NoHotkey,
    /// キャプチャで始まらないレシピに `hotkey` がある（ショートカットで呼ぶ）。
    HotkeyWithoutCapture,
    /// 操作に添えたパラメータが範囲外。書いたキー。
    OutOfRange(String),
    /// 操作に添えたパラメータの型か書式の誤り。書いたキー。
    BadValue(String),
    /// パラメータを添えた操作の知らないキー。
    UnknownKey(String),
    /// パラメータを添えた操作の `op` が `appearance` でも `background` でもない（書いていないときも）。
    NotAppearance,
    /// パラメータを添えた操作に、項目を 1 つも書いていない。
    NoValues,
    /// パラメータを添えた操作に、`border = false` と枠の色か太さを一緒に書いた。
    BorderOffWithColorOrWidth,
    /// 定型領域キャプチャの定型領域が `[[regions]]` にない。書いた名前。
    NoSuchRegion(String),
}

/// レシピの欄（問題が載る場所）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Steps,
    Shortcut,
    Hotkey,
}

impl Problem {
    /// 問題が載る欄。キーの食い違いは、設定 GUI に出ている欄（キャプチャで始まるならホットキー、ほかはショートカット）
    /// に載せる。
    pub fn field(&self) -> Field {
        match self {
            Problem::EmptyName | Problem::DuplicateName => Field::Name,
            Problem::NoHotkey | Problem::ShortcutOnCapture => Field::Hotkey,
            Problem::HotkeyWithoutCapture => Field::Shortcut,
            _ => Field::Steps,
        }
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
/// 認める。定型領域キャプチャの定型領域は `regions`（`Settings::regions`）から探す。
pub fn plan(
    recipes: &[Recipe],
    ops: &'static [OperationDescriptor],
    regions: &[Region],
) -> Checked {
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
        let mut capture = None;
        for (position, step) in recipe.steps.iter().enumerate() {
            if let Some(c) = Capture::from_id(step.op_id())
                && matches!(step, Step::Op(_) | Step::CaptureRegion(_))
            {
                if position == 0 {
                    capture = match c {
                        Capture::Select => Some(PlannedCapture::Select),
                        Capture::ActiveWindow => Some(PlannedCapture::ActiveWindow),
                        Capture::Blank => Some(PlannedCapture::Blank),
                        Capture::Region => check_capture_region(step, regions, &mut problems)
                            .map(PlannedCapture::Region),
                    };
                } else if !problems.contains(&Problem::FirstOnlyNotFirst) {
                    problems.push(Problem::FirstOnlyNotFirst);
                }
                continue;
            }
            let values = match step {
                Step::Op(_) => None,
                Step::Appearance(step) => Some(StepValues::Appearance(check_appearance(
                    step,
                    &mut problems,
                ))),
                Step::Background(step) => Some(StepValues::Background(check_background(
                    step,
                    &mut problems,
                ))),
                // キャプチャ系の操作は上で除いている
                Step::CaptureRegion(_) => None,
            };
            // パラメータを添えた操作は値を取る操作（`takes_values`）を、パラメータのない操作はレシピに入れられる操作を呼ぶ
            let fits = |op: &OperationDescriptor| match values {
                Some(_) => op.takes_values,
                None => can_be_step(op),
            };
            match ops.iter().find(|op| op.id == step.op_id() && fits(op)) {
                Some(op) => steps.push(PlannedStep { op, values }),
                None => problems.push(Problem::NotAStep(step.op_id().to_owned())),
            }
        }
        if let Some((_, before_last)) = recipe.steps.split_last()
            && before_last.iter().any(|s| is_last_only(s.op_id(), ops))
        {
            problems.push(Problem::LastOnlyNotLast);
        }
        // キャプチャで始まるレシピはホットキーで、ほかのレシピはショートカットで呼ぶ
        let starts_with_capture = recipe
            .steps
            .first()
            .is_some_and(|s| is_first_only(s.op_id()));
        if starts_with_capture {
            if recipe.shortcut.is_some() {
                problems.push(Problem::ShortcutOnCapture);
            }
            if recipe.hotkey.is_none() {
                problems.push(Problem::NoHotkey);
            }
        } else if recipe.hotkey.is_some() {
            problems.push(Problem::HotkeyWithoutCapture);
        }
        if problems.is_empty() {
            checked.plans.push(Plan {
                index,
                name: recipe.name.clone(),
                capture,
                steps,
                shortcut: recipe.shortcut,
                hotkey: recipe.hotkey,
            });
        } else {
            checked
                .problems
                .extend(problems.into_iter().map(|p| (index, p)));
        }
    }
    checked
}

/// キャプチャで始まるレシピを実行できなかったわけ（`launch`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CannotLaunch {
    /// その位置に、キャプチャで始まるレシピがもうない。
    Gone,
    /// レシピに問題がある（定型領域を消したあとなど）。
    Problems(Vec<Problem>),
}

/// ホットキーを受けた時点の設定（`recipes`、`regions`）で検査した、`index` 番目の（`Settings::recipes` の中の位置）
/// キャプチャで始まるレシピの実行（ADR-0010）。設定を変えたあとで実行できなければ、そのわけを返す。
pub fn launch(
    recipes: &[Recipe],
    ops: &'static [OperationDescriptor],
    regions: &[Region],
    index: usize,
) -> Result<Launch, CannotLaunch> {
    let checked = plan(recipes, ops, regions);
    if let Some(plan) = checked.plans.iter().find(|plan| plan.index == index) {
        return Launch::of(plan).ok_or(CannotLaunch::Gone);
    }
    let problems: Vec<Problem> = checked
        .problems
        .into_iter()
        .filter(|(i, _)| *i == index)
        .map(|(_, problem)| problem)
        .collect();
    if problems.is_empty() {
        Err(CannotLaunch::Gone)
    } else {
        Err(CannotLaunch::Problems(problems))
    }
}

/// 読めなかった項目を問題にして `problems` に足す。
fn check_rejected(rejected: &[Rejected], problems: &mut Vec<Problem>) {
    for r in rejected {
        problems.push(match r.reason {
            Rejection::OutOfRange => Problem::OutOfRange(r.key.as_str().to_owned()),
            Rejection::BadValue => Problem::BadValue(r.key.as_str().to_owned()),
            Rejection::UnknownKey => Problem::UnknownKey(r.key.as_str().to_owned()),
            Rejection::NotAppearance => Problem::NotAppearance,
        });
    }
}

/// 定型領域キャプチャ（`step`）を検査し、問題を `problems` に足す。撮る定型領域の名前を返す（問題があっても、
/// 読めていれば）。パラメータのない書き方（`"capture_region"`）は、定型領域を選んでいないのと同じ。
fn check_capture_region(
    step: &Step,
    regions: &[Region],
    problems: &mut Vec<Problem>,
) -> Option<String> {
    let Step::CaptureRegion(step) = step else {
        problems.push(Problem::NoValues);
        return None;
    };
    check_rejected(&step.rejected, problems);
    match &step.region {
        Some(name) if !regions.iter().any(|r| &r.name == name) => {
            problems.push(Problem::NoSuchRegion(name.clone()));
        }
        Some(_) => {}
        None if step.rejected.iter().any(|r| r.key == StepKey::Region) => {}
        None => problems.push(Problem::NoValues),
    }
    step.region.clone()
}

/// 背景色を指定する操作を検査し、問題を `problems` に足す。読めた色を返す（読めなければ既定の白。問題がある
/// ので実行はしない）。
fn check_background(step: &BackgroundStep, problems: &mut Vec<Problem>) -> Background {
    check_rejected(&step.rejected, problems);
    let wrote_color = step.color.is_some() || step.rejected.iter().any(|r| r.key == StepKey::Color);
    if !wrote_color {
        problems.push(Problem::NoValues);
    }
    step.color.unwrap_or_default()
}

/// 見た目を指定する操作のパラメータを検査し、問題を `problems` に足す。読めた値を返す（問題があっても）。
fn check_appearance(step: &AppearanceStep, problems: &mut Vec<Problem>) -> AppearanceValues {
    check_rejected(&step.rejected, problems);
    let values = step.values;
    let wrote_an_item = !values.is_empty() || step.rejected.iter().any(|r| r.key != StepKey::Op);
    if !wrote_an_item {
        problems.push(Problem::NoValues);
    }
    if values.border == Some(false)
        && (values.border_color.is_some() || values.border_width.is_some())
    {
        problems.push(Problem::BorderOffWithColorOrWidth);
    }
    values
}

/// 1 つの操作の結果。Win32 層がその操作の `Effect` を実行して決め、`Run::settle` で返す。
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
    /// 1 つの操作の `Effect` ごとの結果をまとめた、その操作の結果。いちばん悪いもの（失敗 → 取り消し → 成功）。
    /// `Effect` がなければ成功。
    pub fn of(outcomes: impl IntoIterator<Item = Outcome>) -> Outcome {
        outcomes.into_iter().max().unwrap_or(Outcome::Succeeded)
    }
}

/// レシピが止まったわけ。どれも、その操作（id）より後ろの操作は実行していない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// その操作は今は実行できなかった（`can_execute` が偽）。実行していない。
    CannotRun(OperationId),
    /// その操作が失敗した。
    Failed(OperationId),
    /// その操作が取り消された。
    Cancelled(OperationId),
}

/// `Run::advance` の結果。
#[derive(Debug, Clone)]
pub enum Advance {
    /// 次の操作を実行した（`ScrapModel` を変えた）。Win32 層は `effects` を実行し、その結果を `Run::settle` で
    /// 返す。結果を返すまで、次の操作へは進まない。
    Step {
        op: &'static OperationDescriptor,
        effects: Vec<Effect>,
    },
    /// その操作の結果をまだ受けていない（何もしていない）。
    Waiting(OperationId),
    /// 全部の操作が成功した。
    Finished,
    /// 止まった。
    Stopped(Stop),
}

/// 操作は id で比べる（`OperationDescriptor` は関数を持つので比べない）。
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

/// 走っているレシピの進み具合（ADR-0007）。1 つずつ実行し、その操作の結果を受けてから次の操作へ進む。
///
/// Win32 層は `advance` で操作を 1 つ実行させ、返った `Effect` を実行して、その結果を `settle` で返す。結果が
/// あとから届く `Effect`（裏のスレッドの保存など）は、届くまで `Run` を持っておき、届いたら `settle` してから
/// `advance` を続ける。`Effect` を実行する前にスクラップが閉じられたら、`Run` を捨てる。
#[derive(Debug, Clone)]
pub struct Run {
    steps: Vec<PlannedStep>,
    /// 次に実行する操作の位置。
    next: usize,
    state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 次の操作を実行できる。
    Ready,
    /// 実行した操作（`next - 1`）の結果を待っている。
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

    /// 次の操作へ進む。前の操作の結果を待っている間と、止まったあとは何もしない。
    pub fn advance(&mut self, model: &mut ScrapModel, context: &OpContext) -> Advance {
        match self.state {
            State::Stopped(stop) => return Advance::Stopped(stop),
            State::Waiting => return Advance::Waiting(self.steps[self.next - 1].op.id),
            State::Ready => {}
        }
        let Some(&PlannedStep { op, values }) = self.steps.get(self.next) else {
            return Advance::Finished;
        };
        if !(op.can_execute)(model) {
            let stop = Stop::CannotRun(op.id);
            self.state = State::Stopped(stop);
            return Advance::Stopped(stop);
        }
        self.next += 1;
        self.state = State::Waiting;
        // レシピの「編集モードに入る」は入るだけ。もう編集モードなら何もしない（右クリックメニューやショートカットの
        // ように抜けはしない）
        if op.id == ANNOTATE && model.is_editing() {
            return Advance::Step {
                op,
                effects: Vec::new(),
            };
        }
        let context = OpContext {
            values,
            ..context.clone()
        };
        Advance::Step {
            op,
            effects: (op.run)(model, &context),
        }
    }

    /// 結果を待っている操作の id。
    pub fn waiting_for(&self) -> Option<OperationId> {
        (self.state == State::Waiting).then(|| self.steps[self.next - 1].op.id)
    }

    /// 終えたか止まったあとに、`model` で書き込みの編集モードに入るか。レシピの途中では、白紙スクラップを作っても
    /// 編集モードに入らない（続きの操作を止めないため）。終えたとき（`Advance::Finished`）か止まったとき
    /// （`Advance::Stopped`）に、何も書かれていない透明な白紙（`ScrapModel::starts_in_edit_mode`）が残っていて、まだ
    /// 編集モードでなければ入る（見えずにつかめるだけの物を残さないため）。
    pub fn enters_edit_mode(&self, model: &ScrapModel) -> bool {
        let ended = match self.state {
            State::Stopped(_) => true,
            State::Ready => self.next >= self.steps.len(),
            State::Waiting => false,
        };
        ended && model.starts_in_edit_mode() && !model.is_editing()
    }

    /// 実行した操作の結果を受ける。成功なら次の操作へ進めるようにし、失敗か取り消しなら止める。結果を待って
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

/// キャプチャで始まるレシピの実行（ADR-0010）。キャプチャ・出す前に当てる操作・残りの操作の 3 つに分けて持つ。
///
/// Win32 層はホットキーを受けたら `Launch::of` で作り、`capture` のキャプチャを行う。キャプチャをやめたら
/// （範囲選択の画面の `Esc`）捨てるだけで、何も作らない。できた `ScrapModel` に `captured` で出す前に当てる操作を
/// 当ててから最初に出し、返った `Run` を、ショートカットから呼んだレシピと同じく 1 つずつ進める（ADR-0007）。
#[derive(Debug, Clone)]
pub struct Launch {
    capture: PlannedCapture,
    /// キャプチャの直後に続く、見た目（`APPEARANCE`）と背景色（`BACKGROUND`）の操作。
    before_show: Vec<PlannedStep>,
    /// 残りの操作（出す前に当てる操作のあと、最初のほかの操作から先）。
    rest: Vec<PlannedStep>,
}

/// 出す前に当てる操作を当てた結果（`Launch::captured`）。
#[derive(Debug, Clone)]
pub struct Shown {
    /// 出したあとに進める残りの操作。出す前に当てる操作が今は実行できなければ、止まっている。
    pub run: Run,
    /// 出す前に当てた操作の `Effect` のうち、出したあとに実行するもの。描き直し（`Effect::Redraw`）は、出すとき
    /// に描くので除く。
    pub effects: Vec<Effect>,
}

impl Launch {
    /// キャプチャで始まるレシピ `plan` の実行。キャプチャで始まらなければ None。
    pub fn of(plan: &Plan) -> Option<Launch> {
        let capture = plan.capture.clone()?;
        let shown_with = |s: &PlannedStep| s.op.id == APPEARANCE || s.op.id == BACKGROUND;
        let split = plan
            .steps
            .iter()
            .position(|s| !shown_with(s))
            .unwrap_or(plan.steps.len());
        let (before_show, rest) = plan.steps.split_at(split);
        Some(Launch {
            capture,
            before_show: before_show.to_vec(),
            rest: rest.to_vec(),
        })
    }

    /// 最初に行うキャプチャ。
    pub fn capture(&self) -> &PlannedCapture {
        &self.capture
    }

    /// キャプチャでできた、まだ出していない `model` に、出す前に当てる操作を並べた順に当てる。今は実行できない操作
    /// （撮ったスクラップへの背景色など）があれば、その前で止め、残りの操作も実行しない。
    pub fn captured(self, model: &mut ScrapModel, context: &OpContext) -> Shown {
        let mut effects = Vec::new();
        for PlannedStep { op, values } in self.before_show {
            if !(op.can_execute)(model) {
                return Shown {
                    run: Run {
                        steps: self.rest,
                        next: 0,
                        state: State::Stopped(Stop::CannotRun(op.id)),
                    },
                    effects,
                };
            }
            let context = OpContext {
                values,
                ..context.clone()
            };
            effects.extend(
                (op.run)(model, &context)
                    .into_iter()
                    .filter(|e| *e != Effect::Redraw),
            );
        }
        Shown {
            run: Run::new(self.rest),
            effects,
        }
    }
}

/// あとから届く結果（裏のスレッドの保存）を待っているレシピ。待たせるのは 1 つだけ。
///
/// Win32 層は、結果を待つ操作を実行したら `Run` をここに預け、結果の知らせ（`ticket`。保存先など）が届いたら
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
    use crate::scrap::ops::{Effect, OPERATIONS, OpContext};
    use crate::scrap::{Appearance, Background, Border, ScrapModel};
    use crate::settings::{
        self, Keymap, Rgb, Settings, SettingsIssue, SettingsItem, ValidationIssue,
    };

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
            ..Recipe::default()
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
            &[],
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
        // trim はモードを変える。zoom は選択肢を持ち、move はドラッグ専用。fly は表にない（別の
        // レシピの名前も同じ）
        for step in ["trim", "zoom", "move", "fly", "コピーして閉じる"] {
            let checked = plan(
                &[
                    recipe("コピーして閉じる", &["copy", "close"]),
                    recipe("r", &["copy", step]),
                ],
                OPERATIONS,
                &[],
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
            &[],
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
            &[],
        );
        assert_eq!(
            checked.problems,
            vec![(0, Problem::LastOnlyNotLast), (1, Problem::LastOnlyNotLast)]
        );
        assert_eq!(
            checked.plans.iter().map(|p| p.index).collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn annotate_goes_only_at_the_end_and_not_with_close() {
        let checked = plan(
            &[
                recipe("a", &["annotate", "copy"]),
                recipe("b", &["copy", "close", "annotate"]),
                recipe("c", &["annotate", "close"]),
                recipe("d", &["annotate", "annotate"]),
                recipe("e", &["copy", "annotate"]),
            ],
            OPERATIONS,
            &[],
        );
        assert_eq!(
            checked.problems,
            vec![
                (0, Problem::LastOnlyNotLast),
                (1, Problem::LastOnlyNotLast),
                (2, Problem::LastOnlyNotLast),
                (3, Problem::LastOnlyNotLast),
            ]
        );
        assert_eq!(
            checked.plans.iter().map(|p| p.index).collect::<Vec<_>>(),
            [4]
        );
    }

    fn capture_recipe(name: &str, steps: &[&str], hotkey: &str) -> Recipe {
        Recipe {
            hotkey: Some(chord(hotkey)),
            ..recipe(name, steps)
        }
    }

    #[test]
    fn a_capture_goes_only_first_and_is_taken_out_of_the_scrap_steps() {
        let checked = plan(
            &[
                capture_recipe("撮ってコピー", &["select_capture", "copy"], "Ctrl+Alt+C"),
                capture_recipe("撮るだけ", &["select_capture"], "Ctrl+Alt+D"),
                recipe("a", &["copy", "select_capture"]),
                capture_recipe("b", &["select_capture", "select_capture"], "Ctrl+Alt+E"),
            ],
            OPERATIONS,
            &[],
        );
        assert_eq!(
            checked.problems,
            vec![
                (2, Problem::FirstOnlyNotFirst),
                (3, Problem::FirstOnlyNotFirst)
            ]
        );
        assert_eq!(
            checked.plans.iter().map(|p| p.index).collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(checked.plans[0].capture, Some(PlannedCapture::Select));
        assert_eq!(ids(&checked.plans[0]), ["copy"]);
        assert_eq!(checked.plans[0].hotkey, Some(chord("Ctrl+Alt+C")));
        assert_eq!(checked.plans[1].capture, Some(PlannedCapture::Select));
        assert_eq!(ids(&checked.plans[1]), Vec::<&str>::new());
    }

    #[test]
    fn a_capture_recipe_takes_a_hotkey_and_other_recipes_take_a_shortcut() {
        let with_both = Recipe {
            shortcut: Some(chord("Ctrl+Q")),
            ..capture_recipe("a", &["select_capture", "copy"], "Ctrl+Alt+C")
        };
        let no_hotkey = recipe("b", &["select_capture", "copy"]);
        let hotkey_on_scrap_recipe = capture_recipe("c", &["copy"], "Ctrl+Alt+D");
        let checked = plan(
            &[with_both, no_hotkey, hotkey_on_scrap_recipe],
            OPERATIONS,
            &[],
        );
        assert_eq!(
            checked.problems,
            vec![
                (0, Problem::ShortcutOnCapture),
                (1, Problem::NoHotkey),
                (2, Problem::HotkeyWithoutCapture),
            ]
        );
        assert!(checked.plans.is_empty());
        assert!(is_first_only("select_capture"));
        assert!(!is_first_only("copy"));
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
            &[],
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
                    settings::Problem::LastOnlyNotLast
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
        let checked = plan(&[recipe("r", steps)], OPERATIONS, &[]);
        assert_eq!(checked.problems, vec![]);
        Run::new(checked.plans[0].steps.clone())
    }

    fn model() -> ScrapModel {
        ScrapModel::new(Point::new(0, 0), Size::new(300, 200))
    }

    /// 次の操作を実行し、その操作の id と Effect を返す。`Advance::Step` でなければ panic。
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
        // 結果を受けるまで次の操作へ進まない
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
    fn annotate_at_the_end_enters_the_edit_mode_after_the_steps_before_it() {
        let mut run = run_of(&["zoom_in", "annotate"]);
        let mut model = model();
        assert_eq!(step(&mut run, &mut model).0, "zoom_in");
        run.settle(Outcome::Succeeded);
        assert_eq!(
            step(&mut run, &mut model),
            ("annotate", vec![Effect::EnterEditMode])
        );
        assert!(model.is_editing());
        run.settle(Outcome::Succeeded);
        assert!(matches!(
            run.advance(&mut model, &OpContext::default()),
            Advance::Finished
        ));
    }

    #[test]
    fn annotate_at_the_end_only_enters_and_does_nothing_when_already_editing() {
        let mut run = run_of(&["annotate"]);
        let mut model = model();
        model.start_editing(crate::annotate::Style::default());
        assert_eq!(step(&mut run, &mut model), ("annotate", vec![]));
        assert!(model.is_editing());
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
            ("save_scrap_file", Effect::ShowScrapFileSaveDialog),
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
            (
                "save_scrap_file",
                Outcome::Cancelled,
                Stop::Cancelled("save_scrap_file"),
            ),
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

    /// `steps = [...]` の 1 つのレシピの `Run`。
    fn run_of_steps(steps: &str) -> Run {
        let checked = checked_steps(steps);
        assert_eq!(checked.problems, vec![], "{steps}");
        Run::new(checked.plans[0].steps.clone())
    }

    /// 全体の既定値: 透明度 100%、枠なし（設定の色は灰）、影あり。
    fn no_border_context() -> OpContext {
        OpContext {
            appearance: Appearance {
                show_border: false,
                ..Appearance::default()
            },
            ..OpContext::default()
        }
    }

    fn step_in(
        run: &mut Run,
        model: &mut ScrapModel,
        context: &OpContext,
    ) -> (&'static str, Vec<Effect>) {
        match run.advance(model, context) {
            Advance::Step { op, effects } => (op.id, effects),
            other => panic!("not a step: {other:?}"),
        }
    }

    #[test]
    fn a_border_color_step_shows_a_border_of_that_color_then_copies() {
        let mut run =
            run_of_steps(r##"[{ op = "appearance", border_color = "#FF0000" }, "copy"]"##);
        let context = no_border_context();
        let mut scrap = model();
        assert_eq!(
            step_in(&mut run, &mut scrap, &context),
            ("appearance", vec![Effect::Redraw])
        );
        assert_eq!(
            scrap.appearance(&context.appearance).shown_border(),
            Some(Border {
                width: 1,
                color: 0xFF_00_00
            })
        );
        run.settle(Outcome::Succeeded);
        assert_eq!(
            step_in(&mut run, &mut scrap, &context),
            ("copy", vec![Effect::CopyToClipboard])
        );
    }

    #[test]
    fn a_step_with_values_changes_only_the_items_it_writes() {
        let context = no_border_context();
        let mut scrap = model();
        let mut first = run_of_steps(
            r##"[{ op = "appearance", opacity = 40, border_color = "#0000FF", shadow = false }]"##,
        );
        step_in(&mut first, &mut scrap, &context);
        let mut run = run_of_steps(r#"[{ op = "appearance", opacity = 70 }]"#);
        step_in(&mut run, &mut scrap, &context);
        assert_eq!(
            scrap.appearance(&context.appearance),
            Appearance {
                opacity: 70,
                border: Border {
                    width: 1,
                    color: 0x00_00_FF
                },
                show_border: true,
                shadow: false,
            }
        );
        // 枠を出すだけなら今の色のまま。隠してから出すと、設定の色になる
        let mut run = run_of_steps(r#"[{ op = "appearance", border = true }]"#);
        assert_eq!(step_in(&mut run, &mut scrap, &context).1, vec![]);
        let mut run = run_of_steps(r#"[{ op = "appearance", border = false }]"#);
        step_in(&mut run, &mut scrap, &context);
        assert_eq!(scrap.appearance(&context.appearance).shown_border(), None);
        let mut run = run_of_steps(r#"[{ op = "appearance", border = true }]"#);
        assert_eq!(
            step_in(&mut run, &mut scrap, &context).1,
            vec![Effect::Redraw]
        );
        assert_eq!(
            scrap.appearance(&context.appearance).shown_border(),
            Some(Border {
                width: 1,
                color: 0x80_80_80
            })
        );
    }

    #[test]
    fn a_step_with_values_that_changes_nothing_still_goes_on_to_the_next_step() {
        let context = no_border_context();
        let mut scrap = model();
        let mut run =
            run_of_steps(r#"[{ op = "appearance", opacity = 100, shadow = true }, "copy"]"#);
        let (_, effects) = step_in(&mut run, &mut scrap, &context);
        assert_eq!(effects, vec![]);
        run.settle(Outcome::of([]));
        assert_eq!(
            step_in(&mut run, &mut scrap, &context),
            ("copy", vec![Effect::CopyToClipboard])
        );
    }

    #[test]
    fn a_step_with_values_does_not_run_on_a_compact_scrap() {
        let mut run = run_of_steps(r#"[{ op = "appearance", shadow = false }, "copy"]"#);
        let mut scrap = model();
        assert!(scrap.enter_compact(crate::scrap::ZoomAnchor::Center));
        assert_eq!(
            run.advance(&mut scrap, &OpContext::default()),
            Advance::Stopped(Stop::CannotRun("appearance"))
        );
    }

    #[test]
    fn resetting_the_appearance_goes_anywhere_and_moves_on_even_when_nothing_changes() {
        let checked = plan(
            &[
                recipe("戻してコピー", &["reset_appearance", "copy"]),
                recipe("コピーして戻す", &["copy", "reset_appearance"]),
            ],
            OPERATIONS,
            &[],
        );
        assert_eq!(checked.problems, vec![]);
        assert_eq!(checked.plans.len(), 2);

        // もう既定の見た目なので何も変わらないが、次の操作へ進む
        let mut run = Run::new(checked.plans[0].steps.clone());
        let mut model = model();
        assert_eq!(step(&mut run, &mut model), ("reset_appearance", vec![]));
        run.settle(Outcome::Succeeded);
        assert_eq!(
            step(&mut run, &mut model),
            ("copy", vec![Effect::CopyToClipboard])
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

    /// `steps = [...]` の 1 つのレシピを読んで検査する。
    fn checked_steps(steps: &str) -> Checked {
        let (settings, issues) = settings::load(&format!(
            "version = 1\n[[recipes]]\nname = \"r\"\nsteps = {steps}\n"
        ));
        assert_eq!(issues, vec![], "{steps}");
        plan(&settings.recipes, OPERATIONS, &[])
    }

    #[test]
    fn a_step_with_appearance_values_becomes_a_plan_with_those_values() {
        let checked = checked_steps(
            r##"[{ op = "appearance", opacity = 50, border = true, border_color = "#ff0000", border_width = 3, shadow = false }, "copy"]"##,
        );
        assert_eq!(checked.problems, vec![]);
        let steps = &checked.plans[0].steps;
        assert_eq!(ids(&checked.plans[0]), ["appearance", "copy"]);
        assert_eq!(
            steps[0].values,
            Some(StepValues::Appearance(AppearanceValues {
                opacity: Some(50),
                border: Some(true),
                border_color: Some(Rgb(0xFF, 0, 0)),
                border_width: Some(3),
                shadow: Some(false),
            }))
        );
        assert_eq!(steps[1].values, None);
    }

    #[test]
    fn mistakes_in_a_step_with_values_make_the_recipe_an_error() {
        for (steps, problems) in [
            (
                r#"[{ op = "appearance", opacity = 5 }]"#,
                vec![Problem::OutOfRange("opacity".into())],
            ),
            (
                r#"[{ op = "appearance", opacity = 101 }]"#,
                vec![Problem::OutOfRange("opacity".into())],
            ),
            (
                r#"[{ op = "appearance", opacity = "50" }]"#,
                vec![Problem::BadValue("opacity".into())],
            ),
            (
                r#"[{ op = "appearance", border_color = "red" }]"#,
                vec![Problem::BadValue("border_color".into())],
            ),
            (
                r##"[{ op = "appearance", border_color = "#FF00" }]"##,
                vec![Problem::BadValue("border_color".into())],
            ),
            (
                r#"[{ op = "appearance", shadow = 1 }]"#,
                vec![Problem::BadValue("shadow".into())],
            ),
            (
                r#"[{ op = "appearance", border = "true" }]"#,
                vec![Problem::BadValue("border".into())],
            ),
            (
                r##"[{ op = "appearance", shadow = true, colour = "#FF0000" }]"##,
                vec![Problem::UnknownKey("colour".into())],
            ),
            (
                r#"[{ op = "appearance", border_width = 0 }]"#,
                vec![Problem::OutOfRange("border_width".into())],
            ),
            (
                r#"[{ op = "appearance", border_width = 17 }]"#,
                vec![Problem::OutOfRange("border_width".into())],
            ),
            (
                r#"[{ op = "appearance", border_width = 2.5 }]"#,
                vec![Problem::BadValue("border_width".into())],
            ),
            (
                r#"[{ op = "copy", shadow = true }]"#,
                vec![Problem::NotAppearance],
            ),
            (r#"[{ shadow = true }]"#, vec![Problem::NotAppearance]),
            (r#"[{ op = "appearance" }]"#, vec![Problem::NoValues]),
            (
                r##"[{ op = "appearance", border = false, border_color = "#FF0000" }]"##,
                vec![Problem::BorderOffWithColorOrWidth],
            ),
            (
                r#"[{ op = "appearance", border = false, border_width = 3 }]"#,
                vec![Problem::BorderOffWithColorOrWidth],
            ),
            // パラメータのない操作として appearance は書けない（値がないと実行できない）
            (
                r#"["appearance"]"#,
                vec![Problem::NotAStep("appearance".into())],
            ),
        ] {
            let checked = checked_steps(steps);
            assert_eq!(
                checked.problems,
                problems.into_iter().map(|p| (0, p)).collect::<Vec<_>>(),
                "{steps}"
            );
            assert!(checked.plans.is_empty(), "{steps}");
        }
    }

    fn white_blank() -> ScrapModel {
        ScrapModel::blank(Point::new(0, 0), Size::new(20, 10), Background::WHITE)
    }

    #[test]
    fn a_background_step_becomes_a_plan_with_that_colour() {
        let checked = checked_steps(r##"[{ op = "background", color = "#1e88e5" }, "copy"]"##);
        assert_eq!(checked.problems, vec![]);
        assert_eq!(ids(&checked.plans[0]), ["background", "copy"]);
        assert_eq!(
            checked.plans[0].steps[0].values,
            Some(StepValues::Background(Background::Color(Rgb(
                0x1E, 0x88, 0xE5
            ))))
        );
    }

    #[test]
    fn a_background_step_can_make_a_written_blank_scrap_transparent_then_goes_on() {
        let checked = checked_steps(r#"[{ op = "background", color = "transparent" }, "copy"]"#);
        assert_eq!(checked.problems, vec![]);
        assert_eq!(
            checked.plans[0].steps[0].values,
            Some(StepValues::Background(Background::Transparent))
        );
        let mut run = run_of_steps(r#"[{ op = "background", color = "transparent" }, "copy"]"#);
        let mut scrap = ScrapModel::blank(
            Point::new(0, 0),
            Size::new(20, 10),
            Background::Color(Rgb(0x21, 0x21, 0x21)),
        );
        scrap.start_editing(crate::annotate::Style::default());
        scrap.edit_input(crate::annotate::EditInput::Press(Point::new(1, 1)));
        scrap.edit_input(crate::annotate::EditInput::Move {
            at: Point::new(8, 8),
            shift: false,
        });
        scrap.edit_input(crate::annotate::EditInput::Leave);
        assert_eq!(
            step(&mut run, &mut scrap),
            ("background", vec![Effect::Redraw])
        );
        assert_eq!(scrap.background(), Some(Background::Transparent));
        run.settle(Outcome::Succeeded);
        assert_eq!(
            step(&mut run, &mut scrap),
            ("copy", vec![Effect::CopyToClipboard])
        );
    }

    /// `steps = [...]` のキャプチャで始まるレシピ 1 つの `Launch`。
    fn launch_of(steps: &str) -> Launch {
        let (settings, issues) = settings::load(&format!(
            "version = 1\n[[recipes]]\nname = \"r\"\nsteps = {steps}\nhotkey = \"Ctrl+Alt+C\"\n"
        ));
        assert_eq!(issues, vec![], "{steps}");
        let checked = plan(&settings.recipes, OPERATIONS, &[]);
        assert_eq!(checked.problems, vec![], "{steps}");
        Launch::of(&checked.plans[0]).expect("starts with a capture")
    }

    #[test]
    fn only_a_recipe_starting_with_a_capture_is_launched() {
        let checked = checked_steps(r#"["copy"]"#);
        assert!(Launch::of(&checked.plans[0]).is_none());
        assert_eq!(
            launch_of(r#"["select_capture"]"#).capture(),
            &PlannedCapture::Select
        );
    }

    #[test]
    fn the_appearance_right_after_a_capture_is_applied_before_showing_and_the_rest_runs_after() {
        let launch = launch_of(
            r##"["select_capture", { op = "appearance", border_color = "#FF0000" }, { op = "appearance", shadow = false }, "copy", { op = "appearance", opacity = 50 }, "close"]"##,
        );
        let context = no_border_context();
        let mut scrap = model();
        let Shown { mut run, effects } = launch.captured(&mut scrap, &context);
        // 出すときに描くので、描き直しは返さない
        assert_eq!(effects, vec![]);
        assert_eq!(
            scrap.appearance(&context.appearance),
            Appearance {
                opacity: 100,
                border: Border {
                    width: 1,
                    color: 0xFF_00_00
                },
                show_border: true,
                shadow: false,
            }
        );
        // ほかの操作が出てきたら、そこから先（後ろの見た目も）は出したあとに 1 つずつ実行する
        assert_eq!(
            step_in(&mut run, &mut scrap, &context),
            ("copy", vec![Effect::CopyToClipboard])
        );
        run.settle(Outcome::Succeeded);
        assert_eq!(
            step_in(&mut run, &mut scrap, &context),
            ("appearance", vec![Effect::Redraw])
        );
        assert_eq!(scrap.appearance(&context.appearance).opacity, 50);
        run.settle(Outcome::Succeeded);
        assert_eq!(step_in(&mut run, &mut scrap, &context).0, "close");
        run.settle(Outcome::Succeeded);
        assert_eq!(run.advance(&mut scrap, &context), Advance::Finished);
    }

    #[test]
    fn the_background_right_after_a_capture_is_applied_before_showing_a_blank_scrap() {
        let launch = launch_of(
            r##"["select_capture", { op = "background", color = "#212121" }, { op = "appearance", opacity = 50 }, "copy"]"##,
        );
        let context = no_border_context();
        let mut scrap = white_blank();
        let Shown { mut run, effects } = launch.captured(&mut scrap, &context);
        assert_eq!(effects, vec![]);
        assert_eq!(
            scrap.background(),
            Some(Background::Color(Rgb(0x21, 0x21, 0x21)))
        );
        assert_eq!(scrap.appearance(&context.appearance).opacity, 50);
        assert_eq!(
            step_in(&mut run, &mut scrap, &context),
            ("copy", vec![Effect::CopyToClipboard])
        );
    }

    #[test]
    fn a_step_before_showing_that_cannot_run_stops_the_recipe_once_shown() {
        // 背景色は白紙スクラップだけで動く。範囲選択キャプチャで撮ったスクラップでは動かない
        let launch = launch_of(
            r##"["select_capture", { op = "appearance", opacity = 50 }, { op = "background", color = "#212121" }, "copy"]"##,
        );
        let context = no_border_context();
        let mut scrap = model();
        let Shown { mut run, effects } = launch.captured(&mut scrap, &context);
        assert_eq!(effects, vec![]);
        assert_eq!(scrap.appearance(&context.appearance).opacity, 50);
        assert_eq!(
            run.advance(&mut scrap, &context),
            Advance::Stopped(Stop::CannotRun("background"))
        );
    }

    #[test]
    fn a_background_step_with_a_misspelt_transparent_makes_the_recipe_an_error() {
        let checked = checked_steps(r#"[{ op = "background", color = "Transparent" }]"#);
        assert_eq!(checked.problems.len(), 1, "{:?}", checked.problems);
        assert!(checked.plans.is_empty());
    }

    #[test]
    fn a_background_step_repaints_a_blank_scrap_then_goes_on() {
        let mut run = run_of_steps(r##"[{ op = "background", color = "#212121" }, "copy"]"##);
        let mut scrap = white_blank();
        assert_eq!(
            step(&mut run, &mut scrap),
            ("background", vec![Effect::Redraw])
        );
        assert_eq!(
            scrap.background(),
            Some(Background::Color(Rgb(0x21, 0x21, 0x21)))
        );
        run.settle(Outcome::Succeeded);
        assert_eq!(
            step(&mut run, &mut scrap),
            ("copy", vec![Effect::CopyToClipboard])
        );
    }

    #[test]
    fn a_background_step_already_of_that_colour_still_goes_on() {
        let mut run = run_of_steps(r##"[{ op = "background", color = "#FFFFFF" }, "copy"]"##);
        let mut scrap = white_blank();
        assert_eq!(step(&mut run, &mut scrap), ("background", vec![]));
        run.settle(Outcome::of([]));
        assert_eq!(
            step(&mut run, &mut scrap),
            ("copy", vec![Effect::CopyToClipboard])
        );
    }

    #[test]
    fn a_background_step_stops_the_recipe_on_a_captured_scrap() {
        let mut run = run_of_steps(r##"[{ op = "background", color = "#000000" }, "copy"]"##);
        let mut scrap = model();
        let before = scrap.clone();
        assert_eq!(
            run.advance(&mut scrap, &OpContext::default()),
            Advance::Stopped(Stop::CannotRun("background"))
        );
        assert_eq!(scrap, before);
        assert_eq!(
            run.advance(&mut scrap, &OpContext::default()),
            Advance::Stopped(Stop::CannotRun("background"))
        );
    }

    #[test]
    fn mistakes_in_a_background_step_make_the_recipe_an_error() {
        for (steps, problems) in [
            (
                r#"[{ op = "background", color = "white" }]"#,
                vec![Problem::BadValue("color".into())],
            ),
            (
                r##"[{ op = "background", color = "#FFF" }]"##,
                vec![Problem::BadValue("color".into())],
            ),
            (
                r#"[{ op = "background", color = 0xFFFFFF }]"#,
                vec![Problem::BadValue("color".into())],
            ),
            (
                r##"[{ op = "background", color = "#FFFFFF", opacity = 50 }]"##,
                vec![Problem::UnknownKey("opacity".into())],
            ),
            (r#"[{ op = "background" }]"#, vec![Problem::NoValues]),
            // パラメータのない操作として background は書けない（色がないと実行できない）
            (
                r#"["background"]"#,
                vec![Problem::NotAStep("background".into())],
            ),
        ] {
            let checked = checked_steps(steps);
            assert_eq!(
                checked.problems,
                problems.into_iter().map(|p| (0, p)).collect::<Vec<_>>(),
                "{steps}"
            );
            assert!(checked.plans.is_empty(), "{steps}");
        }
    }

    #[test]
    fn a_recipe_with_a_wrong_step_with_values_takes_no_shortcut_and_marks_its_steps() {
        let src = "version = 1\n[[recipes]]\nname = \"r\"\nsteps = [{ op = \"appearance\", opacity = 500 }, \"copy\"]\nshortcut = \"Ctrl+Shift+R\"\n";
        let (settings, issues) = settings::load(src);
        assert_eq!(issues, vec![]);
        let keymap = settings::keymap(&settings, OPERATIONS);
        assert_eq!(bound_recipe(&keymap, "Ctrl+Shift+R"), None);
        assert_eq!(keymap.issues, vec![invalid("recipes[1].steps")]);
        assert!(settings::validate(&settings).contains(&issue(
            SettingsItem::RecipeSteps(0),
            settings::Problem::OutOfRange
        )));
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
                    hotkey: None,
                },
                Recipe {
                    name: "二度コピー".into(),
                    steps: vec!["copy".into(), "copy".into()],
                    shortcut: None,
                    hotkey: None,
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
                hotkey: None,
            }]
        );
    }

    #[test]
    fn window_captures_and_blank_scraps_go_only_first_and_take_a_hotkey() {
        let checked = plan(
            &[
                capture_recipe("窓", &["active_window_capture", "copy"], "Ctrl+Alt+W"),
                capture_recipe("白紙", &["blank_scrap"], "Ctrl+Alt+N"),
                recipe("a", &["copy", "blank_scrap"]),
                recipe("b", &["active_window_capture"]),
            ],
            OPERATIONS,
            &[],
        );
        assert_eq!(
            checked.problems,
            vec![(2, Problem::FirstOnlyNotFirst), (3, Problem::NoHotkey)]
        );
        assert_eq!(checked.plans[0].capture, Some(PlannedCapture::ActiveWindow));
        assert_eq!(ids(&checked.plans[0]), ["copy"]);
        assert_eq!(checked.plans[1].capture, Some(PlannedCapture::Blank));
        assert_eq!(
            Capture::ALL.map(Capture::label_key),
            [
                "recipe.capture.select_capture",
                "recipe.capture.active_window_capture",
                "recipe.capture.blank_scrap",
                "recipe.capture.capture_region",
            ]
        );
    }

    fn transparent_blank() -> ScrapModel {
        ScrapModel::blank(Point::new(0, 0), Size::new(20, 10), Background::Transparent)
    }

    /// `model` に 1 本の線を書く（編集モードに入って書き、抜ける）。
    fn write_a_line(model: &mut ScrapModel) {
        model.start_editing(crate::annotate::Style::default());
        model.edit_input(crate::annotate::EditInput::Press(Point::new(1, 1)));
        model.edit_input(crate::annotate::EditInput::Move {
            at: Point::new(8, 8),
            shift: false,
        });
        model.edit_input(crate::annotate::EditInput::Leave);
        assert!(!model.is_editing());
    }

    #[test]
    fn a_blank_scrap_recipe_paints_the_background_before_showing_and_edits_only_at_the_end() {
        let launch = launch_of(
            r#"["blank_scrap", { op = "background", color = "transparent" }, "copy", "zoom_in"]"#,
        );
        assert_eq!(launch.capture(), &PlannedCapture::Blank);
        let context = no_border_context();
        let mut scrap = white_blank();
        let Shown { mut run, effects } = launch.captured(&mut scrap, &context);
        assert_eq!(effects, vec![]);
        assert_eq!(scrap.background(), Some(Background::Transparent));
        // 途中では、透明な白紙でも編集モードに入らない
        assert!(!run.enters_edit_mode(&scrap));
        assert_eq!(step_in(&mut run, &mut scrap, &context).0, "copy");
        assert!(!scrap.is_editing());
        assert!(!run.enters_edit_mode(&scrap));
        run.settle(Outcome::Succeeded);
        assert!(!run.enters_edit_mode(&scrap));
        assert_eq!(step_in(&mut run, &mut scrap, &context).0, "zoom_in");
        run.settle(Outcome::Succeeded);
        assert_eq!(run.advance(&mut scrap, &context), Advance::Finished);
        assert!(!scrap.is_editing());
        assert!(run.enters_edit_mode(&scrap));
    }

    #[test]
    fn a_recipe_that_stops_on_an_empty_transparent_blank_enters_the_edit_mode() {
        for outcome in [Outcome::Failed, Outcome::Cancelled] {
            let mut run = run_of(&["copy", "zoom_in"]);
            let mut scrap = transparent_blank();
            step(&mut run, &mut scrap);
            run.settle(outcome);
            assert!(matches!(
                run.advance(&mut scrap, &OpContext::default()),
                Advance::Stopped(_)
            ));
            assert!(run.enters_edit_mode(&scrap), "{outcome:?}");
        }
        // 出す前に当てる操作が今は実行できずに止まったときも
        let launch = launch_of(
            r#"["select_capture", { op = "background", color = "transparent" }, "copy"]"#,
        );
        let mut scrap = model();
        let Shown { run, .. } = launch.captured(&mut scrap, &OpContext::default());
        assert!(!run.enters_edit_mode(&scrap));
    }

    #[test]
    fn the_edit_mode_at_the_end_is_only_for_an_empty_transparent_blank_not_yet_editing() {
        let finished = |model: &mut ScrapModel, steps: &[&str]| {
            let mut run = run_of(steps);
            loop {
                match run.advance(model, &OpContext::default()) {
                    Advance::Step { .. } => run.settle(Outcome::Succeeded),
                    Advance::Finished => return run,
                    other => panic!("{other:?}"),
                }
            }
        };
        let mut transparent = transparent_blank();
        assert!(finished(&mut transparent, &["copy"]).enters_edit_mode(&transparent));
        let mut white = white_blank();
        assert!(!finished(&mut white, &["copy"]).enters_edit_mode(&white));
        let mut image = model();
        assert!(!finished(&mut image, &["copy"]).enters_edit_mode(&image));
        let mut written = transparent_blank();
        write_a_line(&mut written);
        assert!(!finished(&mut written, &["copy"]).enters_edit_mode(&written));
        // 最後の「編集モードに入る」でもう入っている
        let mut editing = transparent_blank();
        let run = finished(&mut editing, &["copy", "annotate"]);
        assert!(editing.is_editing());
        assert!(!run.enters_edit_mode(&editing));
    }

    /// 定型領域「左上」と、`steps = [...]` のキャプチャで始まるレシピ 1 つを読む。
    fn with_a_region(steps: &str) -> Settings {
        let (settings, issues) = settings::load(&format!(
            "version = 1\n[[regions]]\nname = \"左上\"\nmonitor = \"m\"\nx = 0\ny = 0\nwidth = 10\nheight = 10\n\
             [[recipes]]\nname = \"r\"\nsteps = {steps}\nhotkey = \"Ctrl+Alt+C\"\n"
        ));
        assert_eq!(issues, vec![], "{steps}");
        settings
    }

    fn checked_with_a_region(steps: &str) -> Checked {
        let settings = with_a_region(steps);
        plan(&settings.recipes, OPERATIONS, &settings.regions)
    }

    #[test]
    fn a_capture_recipe_whose_region_is_gone_by_the_hotkey_cannot_be_launched() {
        let settings = with_a_region(r#"[{ op = "capture_region", region = "左上" }, "copy"]"#);
        let launched = launch(&settings.recipes, OPERATIONS, &settings.regions, 0);
        assert_eq!(
            launched.map(|l| l.capture().clone()),
            Ok(PlannedCapture::Region("左上".to_owned()))
        );
        // 設定を変えて定型領域を消したあと
        assert_eq!(
            launch(&settings.recipes, OPERATIONS, &[], 0).err(),
            Some(CannotLaunch::Problems(vec![Problem::NoSuchRegion(
                "左上".to_owned()
            )]))
        );
        // レシピを消したあと
        assert_eq!(
            launch(&settings.recipes, OPERATIONS, &settings.regions, 1).err(),
            Some(CannotLaunch::Gone)
        );
    }

    #[test]
    fn a_fixed_region_capture_names_the_region_it_captures() {
        let checked =
            checked_with_a_region(r#"[{ op = "capture_region", region = "左上" }, "copy"]"#);
        assert_eq!(checked.problems, vec![]);
        let launch = Launch::of(&checked.plans[0]).expect("starts with a capture");
        assert_eq!(launch.capture(), &PlannedCapture::Region("左上".to_owned()));
        assert_eq!(ids(&checked.plans[0]), ["copy"]);
    }

    #[test]
    fn a_fixed_region_capture_of_a_missing_or_unnamed_region_makes_the_recipe_an_error() {
        for (steps, problem) in [
            (
                r#"[{ op = "capture_region", region = "右下" }]"#,
                Problem::NoSuchRegion("右下".into()),
            ),
            (r#"[{ op = "capture_region" }]"#, Problem::NoValues),
            (r#"["capture_region"]"#, Problem::NoValues),
            (
                r#"[{ op = "capture_region", region = 1 }]"#,
                Problem::BadValue("region".into()),
            ),
            (
                r##"[{ op = "capture_region", region = "左上", color = "#000000" }]"##,
                Problem::UnknownKey("color".into()),
            ),
            (
                r#"[{ op = "capture_region", region = "左上" }, { op = "capture_region", region = "左上" }]"#,
                Problem::FirstOnlyNotFirst,
            ),
        ] {
            let checked = checked_with_a_region(steps);
            assert_eq!(checked.problems, vec![(0, problem)], "{steps}");
            assert!(checked.plans.is_empty(), "{steps}");
        }
    }

    #[test]
    fn a_fixed_region_capture_of_a_missing_region_blocks_apply() {
        let settings = with_a_region(r#"[{ op = "capture_region", region = "右下" }]"#);
        assert!(settings::validate(&settings).contains(&issue(
            SettingsItem::RecipeSteps(0),
            settings::Problem::NoSuchRegion
        )));
    }

    #[test]
    fn a_fixed_region_capture_is_written_back_as_it_was_read() {
        let settings = with_a_region(
            r#"[{ op = "capture_region", region = "左上" }, { op = "capture_region", region = "左上", size = 1 }]"#,
        );
        let written = settings::rewrite("version = 1\n", &settings).unwrap();
        let (again, _) = settings::load(&written);
        assert_eq!(again.recipes, settings.recipes, "{written}");
        assert!(
            written.contains(r#"{ op = "capture_region", region = "左上" }"#),
            "{written}"
        );
    }
}
