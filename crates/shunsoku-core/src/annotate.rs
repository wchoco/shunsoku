//! 書き込み（スクラップの上に描き加える図形や文字）と、書き込みの編集モードの状態（ADR-0006）。
//!
//! 書き込みは元の画像（トリム前）の画素の座標で持ち、画像には焼き込まない。スクラップの書き込みは
//! `ScrapModel::annotations` に、編集モードの間だけの状態（描きかけの図形、選んでいる道具と色と太さ）は
//! `scrap::Mode::Editing` の `EditState` に置く。モードを抜けても書き込みは残る。
//!
//! 編集モードへの入力は、スクラップ操作の表を通さず `ScrapModel::edit_input` が受ける。`ScrapModel` が
//! スクリーン座標を元の画像の画素へ直してから `EditState::input` に渡すので、ここは座標の換算を知らない。
//! 結果は `EditOutput` で返し、Win32 が要る仕事（描き直しなど）は `app` がする。
//!
//! 表示と保存・コピーは、どちらも `View`（元の画像の画素 → 描く先の座標）を通して同じ描画処理で描く。
//!
//! 道具は今のところ矩形（`Tool::Rect`）だけ。道具を足すときは `Tool` と `Shape` に行を足し、`Draft` から
//! 図形を作るところ（`Draft::shape`）を足す。

pub mod toolbar;

use crate::geometry::Point;
use crate::keys::{Chord, Key};
use crate::settings::{AnnotationDefaults, Rgb};

/// 書き込みの色と太さ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub color: Rgb,
    /// 太さ（元の画像の画素）。拡大縮小すると表示も一緒に太く・細くなる。
    pub width: u32,
}

impl From<AnnotationDefaults> for Style {
    fn from(defaults: AnnotationDefaults) -> Style {
        Style {
            color: defaults.color,
            width: defaults.width,
        }
    }
}

impl Default for Style {
    fn default() -> Style {
        Style::from(AnnotationDefaults::default())
    }
}

/// 書き込みの形。座標は元の画像の画素（その画素の中心を通って描く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// 矩形。`from` と `to` はドラッグの両端（向きはそのまま）で、対角の 2 つの角。
    Rect { from: Point, to: Point },
}

/// 1 つの書き込み。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Annotation {
    pub shape: Shape,
    pub style: Style,
}

/// 1 枚のスクラップの書き込み。描いた順に並べ、後のものほど上に描く。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Annotations {
    items: Vec<Annotation>,
}

impl Annotations {
    /// 描いた順の書き込み。
    pub fn items(&self) -> &[Annotation] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn push(&mut self, annotation: Annotation) {
        self.items.push(annotation);
    }
}

/// 編集モードで左ドラッグが描くもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Rect,
}

/// 描きかけの図形。左ボタンを押してから離すまで。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Draft {
    from: Point,
    to: Point,
}

impl Draft {
    /// いまの道具で作る図形。長さ 0（押した画素から動いていない）なら None で、描かずに捨てる。
    fn shape(&self, tool: Tool) -> Option<Shape> {
        if self.from == self.to {
            return None;
        }
        Some(match tool {
            Tool::Rect => Shape::Rect {
                from: self.from,
                to: self.to,
            },
        })
    }
}

/// 書き込みの編集モードの間だけの状態（`scrap::Mode::Editing`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditState {
    pub tool: Tool,
    /// これから描く書き込みの色と太さ。入ったときは設定の `[annotation]` の値。
    pub style: Style,
    draft: Option<Draft>,
}

impl EditState {
    pub fn new(style: Style) -> EditState {
        EditState {
            tool: Tool::default(),
            style,
            draft: None,
        }
    }

    /// 描きかけの書き込み（ドラッグ中だけ）。長さ 0 なら None。
    pub fn draft(&self) -> Option<Annotation> {
        let shape = self.draft?.shape(self.tool)?;
        Some(Annotation {
            shape,
            style: self.style,
        })
    }

    /// 入力を受けて、`annotations` を変える。座標は元の画像の画素（`ScrapModel::edit_input` が直したもの）。
    pub fn input(&mut self, input: EditInput, annotations: &mut Annotations) -> Vec<EditOutput> {
        match input {
            EditInput::Press(at) => {
                self.draft = Some(Draft { from: at, to: at });
                vec![]
            }
            EditInput::Move(at) => match &mut self.draft {
                Some(draft) if draft.to != at => {
                    draft.to = at;
                    vec![EditOutput::Redraw]
                }
                _ => vec![],
            },
            EditInput::Release => self.commit(annotations),
            EditInput::Pick(pick) => {
                self.pick(pick);
                vec![EditOutput::Picked(pick)]
            }
            EditInput::Key(chord) if chord == ESCAPE => {
                let mut outputs = self.commit(annotations);
                outputs.push(EditOutput::Left);
                outputs
            }
            EditInput::Key(_) => vec![],
            EditInput::Leave => {
                let mut outputs = self.commit(annotations);
                outputs.push(EditOutput::Left);
                outputs
            }
        }
    }

    /// ツールバーで選んだものを、これから描くものにする。
    fn pick(&mut self, pick: Pick) {
        match pick {
            Pick::Tool(tool) => self.tool = tool,
            Pick::Color(color) => self.style.color = color,
            Pick::Width(width) => self.style.width = width,
        }
    }

    /// 描きかけを書き込みにする。長さ 0 なら捨てる。
    fn commit(&mut self, annotations: &mut Annotations) -> Vec<EditOutput> {
        let Some(draft) = self.draft.take() else {
            return vec![];
        };
        let Some(shape) = draft.shape(self.tool) else {
            return vec![];
        };
        annotations.push(Annotation {
            shape,
            style: self.style,
        });
        vec![EditOutput::Redraw]
    }
}

/// `Esc`。編集モードを抜ける。
const ESCAPE: Chord = Chord::plain(Key::known(0x1B));

/// 書き込みの編集モードへの入力。座標は、`ScrapModel::edit_input` に渡すときはスクリーン座標（物理ピクセル）、
/// `EditState::input` が受けるときは元の画像の画素。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditInput {
    /// 左ボタンを押した（ダブルクリックの 2 回目も）。
    Press(Point),
    /// カーソルが動いた。押している間だけ描きかけを変える。
    Move(Point),
    /// 左ボタンを離した、またはマウスを捕まえていられなくなった。描きかけを書き込みにする。
    Release,
    /// キーを押した。`Esc` で抜ける。ほかのキーは受けない（何も返さない）。
    Key(Chord),
    /// ツールバー（か道具の切り替えのキー）で道具・色・太さを選んだ。
    Pick(Pick),
    /// 抜ける（メニューの「書き込み」、ほかのウィンドウへのフォーカスの移動、スクラップを閉じる前）。
    Leave,
}

impl EditInput {
    /// 座標を `f` で直した入力。
    pub fn map_point(self, f: impl FnOnce(Point) -> Point) -> EditInput {
        match self {
            EditInput::Press(at) => EditInput::Press(f(at)),
            EditInput::Move(at) => EditInput::Move(f(at)),
            other => other,
        }
    }
}

/// 編集モードへの入力の結果。空なら入力を受けなかった（キーなら Windows の既定の処理に回す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutput {
    /// 書き込みか描きかけが変わった。描き直す。
    Redraw,
    /// 編集モードを抜けた。
    Left,
    /// 道具・色・太さを選んだ。ツールバーを描き直し、`Carryover::record` で次の編集モードに引き継ぐ。
    Picked(Pick),
}

/// 編集モードで選べる、これから描くものの道具・色・太さ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Tool(Tool),
    Color(Rgb),
    Width(u32),
}

/// 前の編集モードで選んだ道具・色・太さ。アプリが動いている間、すべてのスクラップで引き継ぐ（設定ファイルには
/// 書き戻さない）。色と太さは選ぶまでは設定の既定値に従う（設定を変えればそれが効く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Carryover {
    /// 最後に使った描く道具。最初は矩形。
    tool: Tool,
    color: Option<Rgb>,
    width: Option<u32>,
}

impl Carryover {
    /// 編集モードの始まりの状態。`defaults` は設定の `[annotation]`。
    pub fn start(&self, defaults: Style) -> EditState {
        let mut state = EditState::new(Style {
            color: self.color.unwrap_or(defaults.color),
            width: self.width.unwrap_or(defaults.width),
        });
        state.tool = self.tool;
        state
    }

    /// 編集モードの出力のうち、選んだもの（`EditOutput::Picked`）を覚える。
    pub fn record(&mut self, outputs: &[EditOutput]) {
        for output in outputs {
            match *output {
                EditOutput::Picked(Pick::Tool(tool)) => self.tool = tool,
                EditOutput::Picked(Pick::Color(color)) => self.color = Some(color),
                EditOutput::Picked(Pick::Width(width)) => self.width = Some(width),
                _ => {}
            }
        }
    }
}

/// 書き込みを描く先への変換。元の画像の画素 `p` の中心を、描く先の `((p.x - left + 0.5) * scale_x,
/// (p.y - top + 0.5) * scale_y)` に置く。太さには `scale_x` を掛ける。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// 描く先の左上に来る、元の画像の画素の位置。
    pub left: f64,
    pub top: f64,
    /// 元の画像の 1 画素の、描く先での長さ。
    pub scale_x: f64,
    pub scale_y: f64,
}

impl View {
    /// 元の画像の画素 `p` の中心の、描く先での座標。
    pub fn point(&self, p: Point) -> (f64, f64) {
        (
            (f64::from(p.x) - self.left + 0.5) * self.scale_x,
            (f64::from(p.y) - self.top + 0.5) * self.scale_y,
        )
    }

    /// 太さ `width`（元の画像の画素）の、描く先での太さ。
    pub fn width(&self, width: u32) -> f64 {
        f64::from(width) * self.scale_x
    }
}
