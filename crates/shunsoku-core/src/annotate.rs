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
//! 道具は矩形・楕円・矢印・線・文字・フリーハンド（`Tool`）で、編集モードの間は数字キーで選ぶ
//! （`Tool::from_key`）。`Shift` を押しながら描くと、矩形は正方形、楕円は円、線と矢印は 45° 刻みに
//! なる（`Draft::end`）。道具を足すときは `Tool` と `Shape` に行を足し、`Draft::shape` と番号の表を足す。
//!
//! 選択の道具（`Tool::Select`、`V`）では、クリックでいちばん上の書き込みを 1 つ選ぶ（`Annotation::hits`。
//! 矩形と楕円は縁だけが当たる）。選んだ書き込みはドラッグと矢印キーで動かし、色と太さを選べばそれにも当てはめ、
//! `Delete` / `Backspace` / ツールバーの削除で消す。書き込みを変えるのは `Annotations` の `push` / `translate` /
//! `restyle` / `remove` だけで、ひとまとまりの変更（ドラッグは押してから離すまで）がそれぞれ履歴の 1 件になる。
//!
//! 消しゴム（`Tool::Eraser`、`E` と 7 の数字キー）では、押したまま動かして、なぞって触れた書き込みを丸ごと消す
//! （当たり方は選択と同じ `Annotation::hits`）。押しただけなら、そこにあるいちばん上のものだけを消す。なぞって
//! いる間にすぐ消し、押してから離すまでに消したものを履歴の 1 件にまとめる（`Annotations::erase`）。
//!
//! 履歴（元に戻す / やり直す）は `Annotations` が書き込みと一緒に持つ（`mode` の外なので、モードを抜けても、
//! ごみ箱から戻しても残る）。編集モードの間、`Ctrl+Z` で元に戻し、`Ctrl+Y` か `Ctrl+Shift+Z` でやり直す。
//! 元に戻す・やり直すと選択は外れる。新しい変更を積むと、やり直しの履歴は消える。
//!
//! 文字の道具（`Tool::Text`）では、クリックした画素を左上にして文字を打つ（`Typing`）。打っている間の文字は
//! `EditInput::Char` で 1 文字ずつ受け（日本語入力の確定した文字も `app` がここに渡す。変換中の文字は `app` が
//! 持って描くだけ）、`Enter` で改行、`Backspace` で 1 文字消す。打っている所の外のクリック、`Esc`、道具の
//! 切り替え、モードを抜けるときに確定して書き込みにする（空なら何も残さない）。描いた文字はダブルクリック
//! （`EditInput::DoubleClick`）で打ち直せる。足すのも打ち直すのも、確定したときに履歴の 1 件になる。
//! 文字は書いたときの書体と大きさ（`Font`）を持つ。文字の大きさと当たり判定の矩形は書体を知らずに見積もる
//! （`text_bounds`）ので、実際に描いた文字の大きさとは少しずれる。

pub mod toolbar;

use crate::geometry::{Point, Rect};
use crate::keys::{Chord, Key, Modifiers};
use crate::settings::{AnnotationDefaults, Rgb};

/// 書き込みの色と太さ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub color: Rgb,
    /// 太さ（元の画像の画素）。拡大縮小すると表示も一緒に太く・細くなる。
    pub width: u32,
}

impl From<&AnnotationDefaults> for Style {
    fn from(defaults: &AnnotationDefaults) -> Style {
        Style {
            color: defaults.color,
            width: defaults.width,
        }
    }
}

impl Default for Style {
    fn default() -> Style {
        Style::from(&AnnotationDefaults::default())
    }
}

/// 文字の書き込みの書体と大きさ。書き込みは書いたときのものを持つ（設定を変えても変わらない）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    /// 書体の名前。入っていなければ、描くときに DirectWrite が代わりの書体で描く。
    pub family: String,
    /// 大きさ（元の画像の画素）。拡大縮小すると表示も一緒に大きく・小さくなる。
    pub size: u32,
}

impl From<&AnnotationDefaults> for Font {
    fn from(defaults: &AnnotationDefaults) -> Font {
        Font {
            family: defaults.font.clone(),
            size: defaults.font_size,
        }
    }
}

impl Default for Font {
    fn default() -> Font {
        Font::from(&AnnotationDefaults::default())
    }
}

/// 書き込みの形。座標は元の画像の画素（その画素の中心を通って描く）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// 矩形。`from` と `to` はドラッグの両端（向きはそのまま）で、対角の 2 つの角。
    Rect { from: Point, to: Point },
    /// 楕円。`from` と `to` は外接する矩形の対角の 2 つの角（`Rect` と同じ）。
    Ellipse { from: Point, to: Point },
    /// 矢印。`from` から `to` へ引き、頭は `to` の側に 1 つ（大きさは太さに比例する。`arrow_head`）。
    Arrow { from: Point, to: Point },
    /// 線。`from` から `to` へ。
    Line { from: Point, to: Point },
    /// フリーハンド。通った点（2 つ以上）を順に線で結ぶ。滑らかにはせず、点を間引くだけ（`FREEHAND_STEP`）。
    Freehand { points: Vec<Point> },
    /// 文字。`at` の画素の左上を文字の左上にして、`text`（`\n` で改行）を背景なし・折り返しなしで書く。
    /// 色は `Style::color` で、太さは使わない。
    Text { at: Point, text: String, font: Font },
}

impl Shape {
    /// `(dx, dy)` 画素ずらす。
    fn translate(&mut self, dx: i32, dy: i32) {
        let shift = |p: &mut Point| *p = Point::new(p.x.saturating_add(dx), p.y.saturating_add(dy));
        match self {
            Shape::Rect { from, to }
            | Shape::Ellipse { from, to }
            | Shape::Arrow { from, to }
            | Shape::Line { from, to } => {
                shift(from);
                shift(to);
            }
            Shape::Freehand { points } => points.iter_mut().for_each(shift),
            Shape::Text { at, .. } => shift(at),
        }
    }
}

/// 文字が占める矩形の見積もり（元の画像の画素）。左上は `at` で、幅は最も長い行（全角の文字は大きさと同じ、
/// ほかは `NARROW_CHAR` 倍）、高さは行の数 × `LINE_HEIGHT` 倍。空の文字でも 1 行分の高さと少しの幅を持つ。
pub fn text_bounds(at: Point, text: &str, size: u32) -> Rect {
    let size = f64::from(size);
    let width = text
        .split('\n')
        .map(|line| {
            line.chars()
                .map(|c| if is_wide(c) { size } else { size * NARROW_CHAR })
                .sum::<f64>()
        })
        .fold(size / 2.0, f64::max);
    let lines = text.split('\n').count() as f64;
    Rect::new(
        at.x,
        at.y,
        at.x.saturating_add(width.ceil() as i32),
        at.y.saturating_add((lines * size * LINE_HEIGHT).ceil() as i32),
    )
}

/// 半角の文字の幅（文字の大きさに対する割合）の見積もり。
const NARROW_CHAR: f64 = 0.55;

/// 行の高さ（文字の大きさに対する割合）の見積もり。
pub const LINE_HEIGHT: f64 = 1.35;

/// 全角（東アジアの幅が広い文字）か。CJK・かな・ハングル・全角形・絵文字のおおよその範囲。
fn is_wide(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD
    )
}

/// 1 つの書き込み。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub shape: Shape,
    pub style: Style,
}

impl Annotation {
    /// 元の画像の画素 `at` が、この書き込みの線の上（太さの半分と `slop` 画素の内）にあるか。矩形と楕円は
    /// 縁だけで、中は当たらない（囲んだものの上の書き込みを選べるように）。文字は線ではないので、見積もった
    /// 矩形（`bounds`）を `slop` 広げた中ならどこでも当たる。
    pub fn hits(&self, at: Point, slop: u32) -> bool {
        if let Shape::Text { .. } = self.shape {
            let slop = i32::try_from(slop).unwrap_or(i32::MAX);
            return self.bounds().inflate(slop).contains(at);
        }
        let reach = f64::from(self.style.width) / 2.0 + f64::from(slop);
        let at = (f64::from(at.x), f64::from(at.y));
        let near = |points: &[(f64, f64)], closed: bool| {
            let n = points.len();
            let segments = if closed { n } else { n.saturating_sub(1) };
            (0..segments).any(|i| segment_distance(at, points[i], points[(i + 1) % n]) <= reach)
        };
        match &self.shape {
            Shape::Rect { from, to } => {
                let (a, b) = (xy(*from), xy(*to));
                near(&[a, (b.0, a.1), b, (a.0, b.1)], true)
            }
            Shape::Ellipse { from, to } => {
                let (a, b) = (xy(*from), xy(*to));
                let (cx, cy) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                let (rx, ry) = ((b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
                let outline: Vec<_> = (0..ELLIPSE_SEGMENTS)
                    .map(|i| {
                        let t = f64::from(i) * std::f64::consts::TAU / f64::from(ELLIPSE_SEGMENTS);
                        (cx + rx * t.cos(), cy + ry * t.sin())
                    })
                    .collect();
                near(&outline, true)
            }
            Shape::Line { from, to } => near(&[xy(*from), xy(*to)], false),
            Shape::Arrow { from, to } => {
                let width = f64::from(self.style.width);
                near(&[xy(*from), xy(*to)], false)
                    || arrow_head(xy(*from), xy(*to), width).is_some_and(|head| {
                        let corners = [head.tip, head.left, head.right];
                        in_triangle(at, corners) || near(&corners, true)
                    })
            }
            Shape::Freehand { points } => {
                let points: Vec<_> = points.iter().map(|&p| xy(p)).collect();
                near(&points, false)
            }
            // 文字は上で見た
            Shape::Text { .. } => false,
        }
    }
}

/// 楕円の当たり判定で、縁を何本の線分で近似するか。
const ELLIPSE_SEGMENTS: u16 = 64;

/// 点 `p` が三角形 `t` の中にあるか（縁も含む）。
fn in_triangle(p: (f64, f64), t: [(f64, f64); 3]) -> bool {
    let side = |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let (d0, d1, d2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    let negative = d0 < 0.0 || d1 < 0.0 || d2 < 0.0;
    let positive = d0 > 0.0 || d1 > 0.0 || d2 > 0.0;
    !(negative && positive)
}

impl Annotation {
    /// 外接矩形（元の画像の画素。選択の印を描く場所）。辺は端の画素の中心から太さの半分（切り上げ）だけ外で、
    /// 矢印は頭の三角形も含む。文字は見積もった矩形（`text_bounds`）。
    pub fn bounds(&self) -> Rect {
        let half = self.style.width.div_ceil(2) as i32;
        let points: Vec<Point> = match &self.shape {
            Shape::Rect { from, to }
            | Shape::Ellipse { from, to }
            | Shape::Arrow { from, to }
            | Shape::Line { from, to } => vec![*from, *to],
            Shape::Freehand { points } => points.clone(),
            Shape::Text { at, text, font } => return text_bounds(*at, text, font.size),
        };
        let mut bounds = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let mut cover = |x: f64, y: f64, pad: i32| {
            bounds.left = bounds.left.min(x.floor() as i32 - pad);
            bounds.top = bounds.top.min(y.floor() as i32 - pad);
            bounds.right = bounds.right.max(x.ceil() as i32 + pad);
            bounds.bottom = bounds.bottom.max(y.ceil() as i32 + pad);
        };
        for p in &points {
            cover(f64::from(p.x), f64::from(p.y), half);
        }
        if let Shape::Arrow { from, to } = &self.shape
            && let Some(head) = arrow_head(xy(*from), xy(*to), f64::from(self.style.width))
        {
            for (x, y) in [head.tip, head.left, head.right] {
                cover(x, y, 0);
            }
        }
        bounds
    }

    /// 元の画像の画素の範囲 `area`（トリムで見えている部分）に、少しでも映るか。外接矩形（`bounds`）で見るので、
    /// 図形の種類ごとの手は要らない。
    pub fn is_visible_in(&self, area: Rect) -> bool {
        self.bounds().intersect(&area).is_some()
    }
}

fn xy(p: Point) -> (f64, f64) {
    (f64::from(p.x), f64::from(p.y))
}

/// 点 `p` から線分 `a`–`b` までの距離。
fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length2 = dx * dx + dy * dy;
    let t = if length2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0)
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// 1 枚のスクラップの書き込みと、その元に戻す / やり直すの履歴。描いた順に並べ、後のものほど上に描く。
///
/// 書き込みを変えるのは private な `push` / `translate` / `restyle` / `remove`（と、ひとまとまりの変更を
/// 続けて足す `drag`）だけで、それぞれが履歴に 1 件を積み、やり直しの履歴を捨てる。履歴は書き込みと一緒に
/// `ScrapModel` の `mode` の外にあるので、編集モードを抜けても、ごみ箱から戻しても残る。上限はない。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Annotations {
    items: Vec<Annotation>,
    /// 元に戻せる変更。最後のものが最新。
    undo: Vec<Change>,
    /// やり直せる変更（元に戻したもの）。最後のものが最も新しく戻したもの。
    redo: Vec<Change>,
}

/// 履歴の 1 件。`items` の `index` の位置への変更で、逆向きに当てれば元に戻る。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// `index` に足した。
    Add {
        index: usize,
        annotation: Annotation,
    },
    /// `index` から消した。
    Remove {
        index: usize,
        annotation: Annotation,
    },
    /// `index` を `before` から `after` に変えた（移動、色や太さの変更）。
    Replace {
        index: usize,
        before: Annotation,
        after: Annotation,
    },
    /// 続けて当てた変更のまとまり（消しゴムの 1 回のなぞり）。順に当て、元に戻すときは逆の順に戻す。
    Many(Vec<Change>),
}

impl Change {
    /// `items` に当てる。`forward` が false なら逆向き（元に戻す）。
    fn apply(&self, items: &mut Vec<Annotation>, forward: bool) {
        match (self, forward) {
            (Change::Add { index, annotation }, true)
            | (Change::Remove { index, annotation }, false) => {
                items.insert(*index, annotation.clone());
            }
            (Change::Add { index, .. }, false) | (Change::Remove { index, .. }, true) => {
                items.remove(*index);
            }
            (Change::Replace { index, after, .. }, true) => items[*index] = after.clone(),
            (Change::Replace { index, before, .. }, false) => items[*index] = before.clone(),
            (Change::Many(changes), true) => {
                for change in changes {
                    change.apply(items, true);
                }
            }
            (Change::Many(changes), false) => {
                for change in changes.iter().rev() {
                    change.apply(items, false);
                }
            }
        }
    }
}

impl Annotations {
    /// 描いた順の書き込み。
    pub fn items(&self) -> &[Annotation] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 元に戻せるか（戻す変更があるか）。
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// やり直せるか。
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// 最後の変更を元に戻す。戻すものがなければ false。
    fn undo(&mut self) -> bool {
        let Some(change) = self.undo.pop() else {
            return false;
        };
        change.apply(&mut self.items, false);
        self.redo.push(change);
        true
    }

    /// 最後に元に戻した変更をやり直す。やり直すものがなければ false。
    fn redo(&mut self) -> bool {
        let Some(change) = self.redo.pop() else {
            return false;
        };
        change.apply(&mut self.items, true);
        self.undo.push(change);
        true
    }

    /// `change` を当てて履歴に積む。新しい変更なので、やり直しの履歴は捨てる。
    fn record(&mut self, change: Change) {
        change.apply(&mut self.items, true);
        self.undo.push(change);
        self.redo.clear();
    }

    fn push(&mut self, annotation: Annotation) {
        self.record(Change::Add {
            index: self.items.len(),
            annotation,
        });
    }

    /// `index` の書き込みを消す。
    fn remove(&mut self, index: usize) -> Option<Annotation> {
        let annotation = self.items.get(index)?.clone();
        self.record(Change::Remove {
            index,
            annotation: annotation.clone(),
        });
        Some(annotation)
    }

    /// 消しゴムで `index` の書き込みを消す。`continuing`（同じなぞりで前にも消した）なら直前の履歴に
    /// まとめるので、なぞり全体が 1 件になる。
    fn erase(&mut self, index: usize, continuing: bool) {
        let Some(annotation) = self.items.get(index).cloned() else {
            return;
        };
        let change = Change::Remove { index, annotation };
        match self.undo.last_mut() {
            Some(Change::Many(changes)) if continuing => {
                change.apply(&mut self.items, true);
                changes.push(change);
            }
            _ => self.record(Change::Many(vec![change])),
        }
    }

    /// `index` の書き込みを `after` に置き換える。同じなら何もせずに false。
    fn replace(&mut self, index: usize, after: Annotation) -> bool {
        match self.items.get(index) {
            Some(before) if *before != after => {
                let before = before.clone();
                self.record(Change::Replace {
                    index,
                    before,
                    after,
                });
                true
            }
            _ => false,
        }
    }

    /// `index` の書き込みの色と太さを `style` にする。変わったら true。
    fn restyle(&mut self, index: usize, style: Style) -> bool {
        let Some(annotation) = self.items.get(index) else {
            return false;
        };
        let after = Annotation {
            style,
            ..annotation.clone()
        };
        self.replace(index, after)
    }

    /// `index` の書き込みを `(dx, dy)` 画素ずらす。動いたら true。
    fn translate(&mut self, index: usize, dx: i32, dy: i32) -> bool {
        let Some(mut after) = self.items.get(index).cloned() else {
            return false;
        };
        after.shape.translate(dx, dy);
        self.replace(index, after)
    }

    /// ドラッグの続きで `index` の書き込みを `(dx, dy)` 画素ずらす。直前の履歴（ドラッグの最初の `translate`）に
    /// まとめるので、ドラッグ全体が 1 件になる。直前の履歴が同じ書き込みの変更でなければ `translate` と同じ。
    fn drag(&mut self, index: usize, dx: i32, dy: i32) {
        match self.undo.last_mut() {
            Some(Change::Replace {
                index: i, after, ..
            }) if *i == index => {
                after.shape.translate(dx, dy);
                self.items[index].shape.translate(dx, dy);
            }
            _ => {
                self.translate(index, dx, dy);
            }
        }
    }
}

/// 編集モードで左ドラッグが描くもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// 選択。クリックで書き込みを 1 つ選び、ドラッグで動かす（描かない）。
    Select,
    #[default]
    Rect,
    Ellipse,
    Arrow,
    Line,
    /// 文字。クリックした所から文字を打つ。
    Text,
    Freehand,
    /// 消しゴム。押したまま動かして、なぞって触れた書き込みを丸ごと消す（描かない）。
    Eraser,
}

impl Tool {
    /// 数字キー（上の段とテンキー、修飾キーなし）で選ぶ道具。番号は 1 から矩形・楕円・矢印・線・文字・
    /// フリーハンド・消しゴムの順で、キーは固定（割り当て直しの対象外）。`V` で選択、`E` で消しゴム。
    fn from_key(chord: Chord) -> Option<Tool> {
        if chord.modifiers != Modifiers::default() {
            return None;
        }
        let n = match chord.key.vk() {
            VK_V => return Some(Tool::Select),
            VK_E => return Some(Tool::Eraser),
            vk @ 0x31..=0x39 => vk - 0x30,
            vk @ 0x61..=0x69 => vk - 0x60,
            _ => return None,
        };
        match n {
            1 => Some(Tool::Rect),
            2 => Some(Tool::Ellipse),
            3 => Some(Tool::Arrow),
            4 => Some(Tool::Line),
            5 => Some(Tool::Text),
            6 => Some(Tool::Freehand),
            7 => Some(Tool::Eraser),
            _ => None,
        }
    }
}

/// 描きかけの図形。左ボタンを押してから離すまで。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    from: Point,
    /// カーソルのいる画素（`Shift` の制約をかける前）。
    to: Point,
    /// 最後に動いたときに `Shift` を押していたか。
    shift: bool,
    /// フリーハンドで残す点（`from` から、`FREEHAND_STEP` 以上離れるごとに 1 つ）。`to` は含まない。
    trail: Vec<Point>,
}

impl Draft {
    fn new(at: Point) -> Draft {
        Draft {
            from: at,
            to: at,
            shift: false,
            trail: vec![at],
        }
    }

    /// カーソルが `at` に動いた。フリーハンドの点は、最後に残した点から `FREEHAND_STEP` 以上離れたら残す。
    fn move_to(&mut self, at: Point, shift: bool) {
        self.to = at;
        self.shift = shift;
        let last = self.trail.last().copied().unwrap_or(self.from);
        let (dx, dy) = (i64::from(at.x - last.x), i64::from(at.y - last.y));
        if dx * dx + dy * dy >= FREEHAND_STEP * FREEHAND_STEP {
            self.trail.push(at);
        }
    }

    /// いまの道具で作る図形。長さ 0（押した画素から動いていない）なら None で、描かずに捨てる。
    fn shape(&self, tool: Tool) -> Option<Shape> {
        if tool == Tool::Freehand {
            let mut points = self.trail.clone();
            if points.last() != Some(&self.to) {
                points.push(self.to);
            }
            return (points.len() >= 2).then_some(Shape::Freehand { points });
        }
        let (from, to) = (self.from, self.end(tool));
        if from == to {
            return None;
        }
        match tool {
            Tool::Rect => Some(Shape::Rect { from, to }),
            Tool::Ellipse => Some(Shape::Ellipse { from, to }),
            Tool::Arrow => Some(Shape::Arrow { from, to }),
            Tool::Line => Some(Shape::Line { from, to }),
            Tool::Select | Tool::Text | Tool::Freehand | Tool::Eraser => None,
        }
    }

    /// `Shift` の制約をかけた終わりの点。矩形と楕円は長いほうの辺にそろえた正方形にする。線と矢印は
    /// いちばん近い 45° 刻みの向きにする（斜めは縦と横の長さの平均）。
    fn end(&self, tool: Tool) -> Point {
        if !self.shift {
            return self.to;
        }
        let (dx, dy) = (self.to.x - self.from.x, self.to.y - self.from.y);
        match tool {
            Tool::Rect | Tool::Ellipse => {
                let side = dx.abs().max(dy.abs());
                Point::new(self.from.x + side * sign(dx), self.from.y + side * sign(dy))
            }
            Tool::Select | Tool::Text | Tool::Freehand | Tool::Eraser => self.to,
            Tool::Arrow | Tool::Line => {
                let (ax, ay) = (f64::from(dx.abs()), f64::from(dy.abs()));
                let (dx, dy) = if ay < ax * TAN_22_5 {
                    (dx, 0)
                } else if ax < ay * TAN_22_5 {
                    (0, dy)
                } else {
                    let d = (dx.abs() + dy.abs() + 1) / 2;
                    (d * sign(dx), d * sign(dy))
                };
                Point::new(self.from.x + dx, self.from.y + dy)
            }
        }
    }
}

/// フリーハンドで点を残す間隔（元の画像の画素）。
const FREEHAND_STEP: i64 = 2;

/// 45° 刻みの境（22.5°）の傾き。
const TAN_22_5: f64 = 0.414_213_562_373_095_1;

/// 0 を正に数える符号（まっすぐなドラッグの正方形は右下へ広げる）。
fn sign(d: i32) -> i32 {
    if d < 0 { -1 } else { 1 }
}

/// 書き込みの編集モードの間だけの状態（`scrap::Mode::Editing`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditState {
    pub tool: Tool,
    /// これから描く書き込みの色と太さ。入ったときは設定の `[annotation]` の値。
    pub style: Style,
    draft: Option<Draft>,
    /// 選んでいる書き込み（`Annotations::items` の位置）。選べるのは 1 回に 1 つだけ。
    selected: Option<usize>,
    /// 選んだ書き込みをドラッグで動かしている間（押してから離すまで）。
    grab: Option<Grab>,
    /// これから打つ文字の書体と大きさ。入ったときは設定の `[annotation]` の値。
    pub font: Font,
    /// 打っている文字（文字の道具でクリックしてから確定するまで）。
    typing: Option<Box<Typing>>,
    /// 消しゴムでなぞっている間（押してから離すまで）。
    erasing: Option<Erasing>,
}

/// 編集モードへの入力 1 回の間の、スクラップの表示から決まる値（`ScrapModel::edit_input` が入れる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EditContext {
    /// 当たり判定を線の太さの外に広げる画素数（`Annotation::hits` の `slop`、元の画像の画素）。表示の倍率から
    /// 決める（画面での広さを揃えるため）。
    pub hit_slop: u32,
    /// 見えている範囲（元の画像の画素。トリムの範囲か画像全体）。外にある書き込みは選ばない。None なら絞らない。
    pub visible: Option<Rect>,
}

impl EditContext {
    /// 書き込み `a` が見えている範囲に映るか（範囲を絞っていなければ常に映る）。
    fn shows(&self, a: &Annotation) -> bool {
        self.visible.is_none_or(|area| a.is_visible_in(area))
    }

    /// `at` が書き込み `a` に当たるか（見えていないものには当たらない）。
    fn hits(&self, a: &Annotation, at: Point) -> bool {
        self.shows(a) && a.hits(at, self.hit_slop)
    }
}

/// 打っている文字。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Typing {
    /// 文字の左上の画素。
    at: Point,
    /// 打った文字（確定したもの。日本語入力の変換中の文字は含まない）。
    text: String,
    font: Font,
    style: Style,
    /// 打ち直している書き込みの位置。新しく打つなら None。
    retyping: Option<usize>,
    /// 日本語入力の変換中の文字。打った文字の続きに描く。
    composition: Option<Composition>,
    /// `WM_CHAR` で来たサロゲート ペアの前半（後半が来たら 1 文字にして打つ）。
    high_surrogate: Option<u16>,
}

/// 日本語入力の変換中の文字と、その中のキャレットの位置（UTF-16 の単位）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composition {
    pub text: String,
    pub cursor: usize,
}

impl Typing {
    /// 文字 `c` を打つ。制御文字は捨てる。
    fn push(&mut self, c: char) -> Vec<EditOutput> {
        if c.is_control() {
            return vec![];
        }
        self.text.push(c);
        vec![EditOutput::Redraw]
    }

    /// `WM_CHAR` の UTF-16 の 1 単位を打つ。サロゲート ペアは 2 つ揃ってから 1 文字にし、揃わないものは捨てる。
    fn push_utf16(&mut self, unit: u16) -> Vec<EditOutput> {
        let c = match unit {
            0xD800..=0xDBFF => {
                self.high_surrogate = Some(unit);
                return vec![];
            }
            0xDC00..=0xDFFF => match self.high_surrogate.take() {
                Some(high) => char::decode_utf16([high, unit]).next().and_then(|c| c.ok()),
                None => None,
            },
            _ => {
                self.high_surrogate = None;
                char::from_u32(u32::from(unit))
            }
        };
        c.map_or_else(Vec::new, |c| self.push(c))
    }

    fn annotation(&self) -> Annotation {
        Annotation {
            shape: Shape::Text {
                at: self.at,
                text: self.text.clone(),
                font: self.font.clone(),
            },
            style: self.style,
        }
    }
}

/// 選んだ書き込みのドラッグ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Grab {
    /// 最後のカーソルの画素。
    last: Point,
    /// もう動かしたか。最初に動かしたときに履歴を 1 件足し、あとはそこにまとめる。
    moved: bool,
}

/// 消しゴムのなぞり。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Erasing {
    /// 最後のカーソルの画素。
    last: Point,
    /// もう消したか。最初に消したときに履歴を 1 件足し、あとはそこにまとめる。
    erased: bool,
}

impl EditState {
    pub fn new(style: Style) -> EditState {
        EditState {
            tool: Tool::default(),
            style,
            draft: None,
            selected: None,
            grab: None,
            font: Font::default(),
            typing: None,
            erasing: None,
        }
    }

    /// 文字を打っている間か。
    pub fn is_typing(&self) -> bool {
        self.typing.is_some()
    }

    /// 打っている文字（打った分だけ。空のこともある）。描くのと、日本語入力の窓の場所を決めるのに使う。
    pub fn typing(&self) -> Option<Annotation> {
        self.typing.as_deref().map(Typing::annotation)
    }

    /// 日本語入力の変換中の文字（打っている文字の続きに描く）。
    pub fn composition(&self) -> Option<&Composition> {
        self.typing.as_ref()?.composition.as_ref()
    }

    /// 打ち直している書き込みの位置（`Annotations::items` の添え字）。打ち終えるまで、元のものは描かない。
    pub fn retyping(&self) -> Option<usize> {
        self.typing.as_ref()?.retyping
    }

    /// 選んでいる書き込みが見えている範囲の外へ出たら、選択を外す（見えないものを動かしたり消したりしないため）。
    fn drop_hidden_selection(&mut self, annotations: &Annotations, cx: &EditContext) {
        let hidden = self
            .selected
            .and_then(|i| annotations.items().get(i))
            .is_some_and(|a| !cx.shows(a));
        if hidden {
            self.selected = None;
            self.grab = None;
        }
    }

    /// 選んでいる書き込みの位置（`Annotations::items` の添え字）。
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// 描きかけの書き込み（ドラッグ中だけ）。長さ 0 なら None。
    pub fn draft(&self) -> Option<Annotation> {
        let shape = self.draft.as_ref()?.shape(self.tool)?;
        Some(Annotation {
            shape,
            style: self.style,
        })
    }

    /// 入力を受けて、`annotations` を変える。座標は元の画像の画素（`ScrapModel::edit_input` が直したもの）。
    /// 文字を打っている間に、文字・日本語入力・カーソルの移動のほかの入力が来たら、変換中の文字を確定したことに
    /// してから受ける（`EditOutput::CompositionTaken`）。当たり判定は広げず、見えている範囲は絞らない
    /// （`EditContext::default()` の `input_in`）。
    pub fn input(&mut self, input: EditInput, annotations: &mut Annotations) -> Vec<EditOutput> {
        self.input_in(input, annotations, &EditContext::default())
    }

    /// `input` の、当たり判定の広さと見えている範囲を `cx` にしたもの。
    pub fn input_in(
        &mut self,
        input: EditInput,
        annotations: &mut Annotations,
        cx: &EditContext,
    ) -> Vec<EditOutput> {
        let mut outputs = self.take_composition(&input);
        let input = match self.type_input(input, annotations, cx) {
            Ok(typed) => {
                outputs.extend(typed);
                return outputs;
            }
            Err(input) => input,
        };
        outputs.extend(self.input_not_typing(input, annotations, cx));
        outputs
    }

    /// `input` の前に、変換中の文字を打った文字に入れる（`input` のとき）。
    fn take_composition(&mut self, input: &EditInput) -> Vec<EditOutput> {
        if matches!(
            input,
            EditInput::Char(_)
                | EditInput::Utf16(_)
                | EditInput::Composition { .. }
                | EditInput::Move { .. }
                | EditInput::Release
        ) {
            return vec![];
        }
        let Some(typing) = self.typing.as_mut() else {
            return vec![];
        };
        let Some(composition) = typing.composition.take() else {
            return vec![];
        };
        for c in composition.text.chars() {
            typing.push(c);
        }
        vec![EditOutput::CompositionTaken]
    }

    /// 文字を打っていないときの入力（打っている間でも、打つ文字への入力でないもの）。
    fn input_not_typing(
        &mut self,
        input: EditInput,
        annotations: &mut Annotations,
        cx: &EditContext,
    ) -> Vec<EditOutput> {
        match input {
            EditInput::Press(at) if self.tool == Tool::Text => self.start_typing(at, None),
            EditInput::Press(at) if self.tool == Tool::Select => {
                self.select_at(at, annotations, cx)
            }
            EditInput::Press(at) if self.tool == Tool::Eraser => {
                self.start_erasing(at, annotations, cx)
            }
            EditInput::Press(at) => {
                self.draft = Some(Draft::new(at));
                vec![]
            }
            EditInput::Move { at, .. } if self.grab.is_some() => {
                self.drag_selected(at, annotations, cx)
            }
            EditInput::Move { at, .. } if self.erasing.is_some() => {
                self.erase_to(at, annotations, cx)
            }
            EditInput::Move { at, shift } => match &mut self.draft {
                Some(draft) if (draft.to, draft.shift) != (at, shift) => {
                    draft.move_to(at, shift);
                    vec![EditOutput::Redraw]
                }
                _ => vec![],
            },
            EditInput::Release => {
                self.grab = None;
                self.erasing = None;
                self.commit(annotations)
            }
            EditInput::Pick(pick) => self.pick(pick, annotations),
            EditInput::Char(_) | EditInput::Utf16(_) | EditInput::Composition { .. } => vec![],
            // 消しゴムでは文字を打ち直さず、ふつうの押下と同じに消す
            EditInput::DoubleClick(at) if self.tool == Tool::Eraser => {
                self.start_erasing(at, annotations, cx)
            }
            EditInput::DoubleClick(at) => match self.text_at(at, annotations, cx) {
                Some(index) => self.retype(index, annotations),
                None => self.input_in(EditInput::Press(at), annotations, cx),
            },
            EditInput::Key(chord) if chord == ESCAPE => {
                let mut outputs = self.commit(annotations);
                outputs.push(EditOutput::Left);
                outputs
            }
            EditInput::Delete => self.delete_selected(annotations),
            EditInput::Undo => self.step_history(annotations, Annotations::undo),
            EditInput::Redo => self.step_history(annotations, Annotations::redo),
            EditInput::Key(chord) if chord == UNDO => {
                self.step_history(annotations, Annotations::undo)
            }
            EditInput::Key(chord) if REDO.contains(&chord) => {
                self.step_history(annotations, Annotations::redo)
            }
            EditInput::Key(chord) if chord == DELETE || chord == BACKSPACE => {
                self.delete_selected(annotations)
            }
            EditInput::Key(chord) => {
                if let Some((dx, dy)) = nudge(chord) {
                    return self.nudge(dx, dy, annotations, cx);
                }
                match Tool::from_key(chord) {
                    Some(tool) => self.pick(Pick::Tool(tool), annotations),
                    None => vec![],
                }
            }
            EditInput::Leave => {
                let mut outputs = self.commit(annotations);
                outputs.push(EditOutput::Left);
                outputs
            }
        }
    }

    /// 文字を打っている間の入力。打つ文字への入力ならその結果を、ほかの入力と同じに扱うなら（打っていない
    /// ときも）`input` をそのまま返す。
    fn type_input(
        &mut self,
        input: EditInput,
        annotations: &mut Annotations,
        cx: &EditContext,
    ) -> Result<Vec<EditOutput>, EditInput> {
        let Some(typing) = self.typing.as_mut() else {
            return Err(input);
        };
        let inside = |at: Point| typing.annotation().hits(at, cx.hit_slop);
        let outputs = match input {
            EditInput::Char(c) => typing.push(c),
            EditInput::Utf16(unit) => typing.push_utf16(unit),
            EditInput::Composition { result, composing } => {
                for c in result.chars() {
                    typing.push(c);
                }
                typing.composition = composing;
                vec![EditOutput::Redraw]
            }
            EditInput::Key(chord) if chord == ESCAPE => self.finish_typing(annotations),
            EditInput::Key(chord) if ENTER.contains(&chord) => {
                typing.text.push('\n');
                vec![EditOutput::Redraw]
            }
            EditInput::Key(chord) if chord == BACKSPACE => match typing.text.pop() {
                Some(_) => vec![EditOutput::Redraw],
                None => vec![],
            },
            // 元に戻す・やり直すは、ツールバーのものと同じに確定してから（打ち直している位置が履歴でずれないため）
            EditInput::Key(chord) if chord == UNDO || REDO.contains(&chord) => {
                let mut outputs = self.finish_typing(annotations);
                outputs.extend(self.input_in(input, annotations, cx));
                outputs
            }
            // ほかのキー（数字キーの道具、矢印、Delete など）は、打つ間は受けない
            EditInput::Key(_) => vec![],
            EditInput::Press(at) if inside(at) => vec![],
            // 打った文字の上のダブルクリックは、単語を選んだりせず何もしない
            EditInput::DoubleClick(at) if !typing.text.is_empty() && inside(at) => {
                vec![]
            }
            // 外のクリックは確定だけ（そこから描いたり選んだりはしない）
            EditInput::Press(_) => self.finish_typing(annotations),
            EditInput::Pick(Pick::Color(color)) => {
                typing.style.color = color;
                self.style.color = color;
                vec![EditOutput::Redraw, EditOutput::Picked(Pick::Color(color))]
            }
            EditInput::Pick(Pick::FontSize(size)) => {
                typing.font.size = size;
                self.font.size = size;
                vec![EditOutput::Redraw, EditOutput::Picked(Pick::FontSize(size))]
            }
            EditInput::Move { .. } | EditInput::Release | EditInput::Pick(Pick::Width(_)) => {
                return Err(input);
            }
            EditInput::DoubleClick(_)
            | EditInput::Pick(Pick::Tool(_))
            | EditInput::Delete
            | EditInput::Undo
            | EditInput::Redo
            | EditInput::Leave => {
                let mut outputs = self.finish_typing(annotations);
                outputs.extend(self.input_in(input, annotations, cx));
                outputs
            }
        };
        Ok(outputs)
    }

    /// `at` から文字を打ち始める。打ち直すなら `retyping` に元の書き込みの位置とその書き込みを渡す。
    fn start_typing(
        &mut self,
        at: Point,
        retyping: Option<(usize, &Annotation)>,
    ) -> Vec<EditOutput> {
        self.draft = None;
        self.grab = None;
        self.selected = None;
        self.typing = Some(Box::new(match retyping {
            Some((
                index,
                Annotation {
                    shape: Shape::Text { at, text, font },
                    style,
                },
            )) => Typing {
                at: *at,
                text: text.clone(),
                font: font.clone(),
                style: *style,
                retyping: Some(index),
                composition: None,
                high_surrogate: None,
            },
            _ => Typing {
                at,
                text: String::new(),
                font: self.font.clone(),
                style: self.style,
                retyping: None,
                composition: None,
                high_surrogate: None,
            },
        }));
        vec![EditOutput::TypingStarted, EditOutput::Redraw]
    }

    /// `index` の文字の書き込みを打ち直す。描きかけは先に書き込みにする。
    fn retype(&mut self, index: usize, annotations: &mut Annotations) -> Vec<EditOutput> {
        self.commit(annotations);
        let Some(annotation) = annotations.items().get(index) else {
            return vec![];
        };
        let at = match &annotation.shape {
            Shape::Text { at, .. } => *at,
            _ => return vec![],
        };
        self.start_typing(at, Some((index, annotation)))
    }

    /// `at` にある文字の書き込みのうち、いちばん上のもの。
    fn text_at(&self, at: Point, annotations: &Annotations, cx: &EditContext) -> Option<usize> {
        annotations
            .items()
            .iter()
            .rposition(|a| matches!(a.shape, Shape::Text { .. }) && cx.hits(a, at))
    }

    /// 打っている文字を確定する。空（空白だけ）なら何も残さず、打ち直していたなら元のものを消す。
    fn finish_typing(&mut self, annotations: &mut Annotations) -> Vec<EditOutput> {
        let Some(typing) = self.typing.take() else {
            return vec![];
        };
        let empty = typing.text.trim().is_empty();
        match (typing.retyping, empty) {
            (None, true) => {}
            (None, false) => annotations.push(typing.annotation()),
            (Some(index), true) => {
                annotations.remove(index);
            }
            (Some(index), false) => {
                annotations.replace(index, typing.annotation());
            }
        }
        vec![EditOutput::Redraw, EditOutput::TypingEnded]
    }

    /// 道具・色・太さを選ぶ。道具を替えるときは、描きかけがあれば前の道具のまま書き込みにしてから替える。
    fn pick(&mut self, pick: Pick, annotations: &mut Annotations) -> Vec<EditOutput> {
        let mut outputs = vec![];
        match pick {
            Pick::Tool(tool) => {
                outputs = self.commit(annotations);
                self.erasing = None;
                self.tool = tool;
                if tool != Tool::Select {
                    self.grab = None;
                    if self.selected.take().is_some() {
                        outputs.push(EditOutput::Redraw);
                    }
                }
                outputs.push(EditOutput::ToolChanged);
            }
            Pick::Color(color) => {
                self.style.color = color;
                outputs = self.restyle_selected(annotations, |style| style.color = color);
            }
            Pick::Width(width) => {
                self.style.width = width;
                // 文字は太さを使わないので、選んだ文字の書き込みは変えない（履歴も積まない）
                if !self.selected_is_text(annotations) {
                    outputs = self.restyle_selected(annotations, |style| style.width = width);
                }
            }
            Pick::FontSize(size) => {
                self.font.size = size;
                outputs = self.resize_selected_text(size, annotations);
            }
        }
        outputs.push(EditOutput::Picked(pick));
        outputs
    }

    fn selected_is_text(&self, annotations: &Annotations) -> bool {
        self.selected
            .and_then(|index| annotations.items().get(index))
            .is_some_and(|a| matches!(a.shape, Shape::Text { .. }))
    }

    /// 選んだ文字の書き込みの大きさを `size` にする。選んでいないか、文字でないか、もう同じなら何もしない。
    fn resize_selected_text(
        &mut self,
        size: u32,
        annotations: &mut Annotations,
    ) -> Vec<EditOutput> {
        let Some(index) = self.selected else {
            return vec![];
        };
        let Some(mut after) = annotations.items().get(index).cloned() else {
            return vec![];
        };
        let Shape::Text { font, .. } = &mut after.shape else {
            return vec![];
        };
        font.size = size;
        if !annotations.replace(index, after) {
            return vec![];
        }
        vec![EditOutput::Redraw]
    }

    /// `at` にある書き込みのうち、いちばん上のものを選ぶ。何もなければ選択を外す。
    fn select_at(
        &mut self,
        at: Point,
        annotations: &Annotations,
        cx: &EditContext,
    ) -> Vec<EditOutput> {
        let hit = annotations.items().iter().rposition(|a| cx.hits(a, at));
        self.grab = hit.map(|_| Grab {
            last: at,
            moved: false,
        });
        if hit == self.selected {
            return vec![];
        }
        self.selected = hit;
        vec![EditOutput::Redraw]
    }

    /// 消しゴムで `at` を押した。そこにある書き込みのうち、いちばん上のものを消す。
    fn start_erasing(
        &mut self,
        at: Point,
        annotations: &mut Annotations,
        cx: &EditContext,
    ) -> Vec<EditOutput> {
        let hit = annotations.items().iter().rposition(|a| cx.hits(a, at));
        if let Some(index) = hit {
            annotations.erase(index, false);
        }
        self.erasing = Some(Erasing {
            last: at,
            erased: hit.is_some(),
        });
        match hit {
            Some(_) => vec![EditOutput::Redraw],
            None => vec![],
        }
    }

    /// 消しゴムでなぞってカーソルが `at` に動いた。前の画素から `at` までの 1 画素ごとに、当たった書き込みを
    /// すべて消す（速く動かしても間を飛ばさない）。
    fn erase_to(
        &mut self,
        at: Point,
        annotations: &mut Annotations,
        cx: &EditContext,
    ) -> Vec<EditOutput> {
        let Some(mut erasing) = self.erasing else {
            return vec![];
        };
        if at == erasing.last {
            return vec![];
        }
        let (dx, dy) = (at.x - erasing.last.x, at.y - erasing.last.y);
        let steps = dx.unsigned_abs().max(dy.unsigned_abs());
        let mut erased_now = false;
        for i in 1..=i64::from(steps) {
            let along = |d: i32| (i64::from(d) * i / i64::from(steps)) as i32;
            let point = Point::new(erasing.last.x + along(dx), erasing.last.y + along(dy));
            for index in (0..annotations.items().len()).rev() {
                if cx.hits(&annotations.items()[index], point) {
                    annotations.erase(index, erasing.erased);
                    erasing.erased = true;
                    erased_now = true;
                }
            }
        }
        erasing.last = at;
        self.erasing = Some(erasing);
        if erased_now {
            vec![EditOutput::Redraw]
        } else {
            vec![]
        }
    }

    /// 選んだ書き込みのドラッグでカーソルが `at` に動いた。動いた分だけずらす。
    fn drag_selected(
        &mut self,
        at: Point,
        annotations: &mut Annotations,
        cx: &EditContext,
    ) -> Vec<EditOutput> {
        let (Some(index), Some(grab)) = (self.selected, self.grab) else {
            return vec![];
        };
        if at == grab.last {
            return vec![];
        }
        let (dx, dy) = (at.x - grab.last.x, at.y - grab.last.y);
        let moved = if grab.moved {
            annotations.drag(index, dx, dy);
            true
        } else {
            annotations.translate(index, dx, dy)
        };
        self.grab = Some(Grab { last: at, moved });
        self.drop_hidden_selection(annotations, cx);
        vec![EditOutput::Redraw]
    }

    /// 選んだ書き込みの色か太さを `change` で変える。選んでいないか、もう同じなら何もしない。
    fn restyle_selected(
        &mut self,
        annotations: &mut Annotations,
        change: impl FnOnce(&mut Style),
    ) -> Vec<EditOutput> {
        let Some(index) = self.selected else {
            return vec![];
        };
        let Some(mut style) = annotations.items().get(index).map(|a| a.style) else {
            return vec![];
        };
        change(&mut style);
        if !annotations.restyle(index, style) {
            return vec![];
        }
        vec![EditOutput::Redraw]
    }

    /// 選んだ書き込みを消す。選んでいなければ受けない。
    fn delete_selected(&mut self, annotations: &mut Annotations) -> Vec<EditOutput> {
        let Some(index) = self.selected.take() else {
            return vec![];
        };
        self.grab = None;
        annotations.remove(index);
        vec![EditOutput::Redraw]
    }

    /// 選んだ書き込みを矢印キーで `(dx, dy)` 画素ずらす。選んでいなければ受けない。
    fn nudge(
        &mut self,
        dx: i32,
        dy: i32,
        annotations: &mut Annotations,
        cx: &EditContext,
    ) -> Vec<EditOutput> {
        let Some(index) = self.selected else {
            return vec![];
        };
        if !annotations.translate(index, dx, dy) {
            return vec![];
        }
        self.drop_hidden_selection(annotations, cx);
        vec![EditOutput::Redraw]
    }

    /// 元に戻す（`step` が `Annotations::undo`）かやり直す（`Annotations::redo`）。描きかけは先に書き込みに
    /// して（元に戻すなら、それが最初に戻る）、選択は外す（戻した変更で位置がずれたり消えたりするため）。
    /// 戻す・やり直すものがなければ受けない。
    fn step_history(
        &mut self,
        annotations: &mut Annotations,
        step: fn(&mut Annotations) -> bool,
    ) -> Vec<EditOutput> {
        let committed = self.commit(annotations);
        self.grab = None;
        // なぞりの続きを、戻した（やり直した）変更にまとめないため
        self.erasing = None;
        let deselected = self.selected.take().is_some();
        if step(annotations) || deselected || !committed.is_empty() {
            vec![EditOutput::Redraw]
        } else {
            vec![]
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

/// 矢印キーで書き込みを動かす向きと画素数。`Shift` を押していれば 10 画素、修飾キーがなければ 1 画素。
fn nudge(chord: Chord) -> Option<(i32, i32)> {
    let step = match chord.modifiers {
        m if m == Modifiers::default() => 1,
        Modifiers {
            shift: true,
            win: false,
            ctrl: false,
            alt: false,
        } => NUDGE_SHIFT_STEP,
        _ => return None,
    };
    match chord.key.vk() {
        0x25 => Some((-step, 0)),
        0x26 => Some((0, -step)),
        0x27 => Some((step, 0)),
        0x28 => Some((0, step)),
        _ => None,
    }
}

/// `Shift` を押しながらの矢印キーで動かす画素数。
const NUDGE_SHIFT_STEP: i32 = 10;

/// `Delete` と `Backspace`。選んだ書き込みを消す。
const DELETE: Chord = Chord::plain(Key::known(0x2E));
const BACKSPACE: Chord = Chord::plain(Key::known(0x08));

/// `V`。選択の道具を選ぶ。
const VK_V: u16 = 0x56;

/// `E`。消しゴムを選ぶ（編集モードの外では、スクラップのショートカットの「書き込み」の既定のキー。編集モードの
/// 間はキーを編集モードが先に受けるので、ショートカットには届かない）。
const VK_E: u16 = 0x45;

/// `Ctrl+Z`。元に戻す。
const UNDO: Chord = Chord::plain(Key::known(0x5A)).ctrl();

/// `Ctrl+Y` と `Ctrl+Shift+Z`。やり直す。
const REDO: [Chord; 2] = [
    Chord::plain(Key::known(0x59)).ctrl(),
    Chord::plain(Key::known(0x5A)).ctrl().shift(),
];

/// `Esc`。編集モードを抜ける（文字を打っている間は確定するだけ）。
const ESCAPE: Chord = Chord::plain(Key::known(0x1B));

/// `Enter` と `Shift+Enter`。文字を打っている間は改行する。
const ENTER: [Chord; 2] = [
    Chord::plain(Key::known(0x0D)),
    Chord::plain(Key::known(0x0D)).shift(),
];

/// 書き込みの編集モードへの入力。座標は、`ScrapModel::edit_input` に渡すときはスクリーン座標（物理ピクセル）、
/// `EditState::input` が受けるときは元の画像の画素。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditInput {
    /// 左ボタンを押した。
    Press(Point),
    /// ダブルクリックした（2 回目の押下。1 回目は `Press`）。文字の書き込みの上なら打ち直し、ほかは `Press`
    /// と同じ（消しゴムでは、文字の上でも `Press` と同じ）。
    DoubleClick(Point),
    /// カーソルが動いた。押している間だけ描きかけを変える。`shift` は `Shift` を押しているか（図形の制約）。
    Move { at: Point, shift: bool },
    /// 左ボタンを離した、またはマウスを捕まえていられなくなった。描きかけを書き込みにする。
    Release,
    /// キーを押した。`Esc` で抜け、数字キーで道具を選び、`Ctrl+Z` で元に戻し、`Ctrl+Y` と `Ctrl+Shift+Z` で
    /// やり直す。ほかのキーは受けない（何も返さない）。
    Key(Chord),
    /// ツールバーで道具・色・太さを選んだ（数字キーで道具を選ぶのは `Key`）。
    Pick(Pick),
    /// 文字を打った。打っている間だけ受け、制御文字は捨てる。
    Char(char),
    /// `WM_CHAR` の UTF-16 の 1 単位。サロゲート ペアは 2 つ揃ってから 1 文字として打つ。ほかは `Char` と同じ。
    Utf16(u16),
    /// 日本語入力の変換が進んだ（`WM_IME_COMPOSITION`）か終わった（`WM_IME_ENDCOMPOSITION`、どちらも None）。
    /// 確定した文字 `result` を打ち、変換中の文字を `composing` にする。打っている間だけ受ける。
    Composition {
        result: String,
        composing: Option<Composition>,
    },
    /// ツールバーの削除を押した（`Delete` と `Backspace` は `Key`）。選んだ書き込みを消す。
    Delete,
    /// ツールバーの元に戻すを押した（`Ctrl+Z` は `Key`）。
    Undo,
    /// ツールバーのやり直すを押した（`Ctrl+Y` と `Ctrl+Shift+Z` は `Key`）。
    Redo,
    /// 抜ける（メニューの「書き込み」、ほかのウィンドウへのフォーカスの移動、スクラップを閉じる前）。
    Leave,
}

impl EditInput {
    /// 座標を `f` で直した入力。
    pub fn map_point(self, f: impl FnOnce(Point) -> Point) -> EditInput {
        match self {
            EditInput::Press(at) => EditInput::Press(f(at)),
            EditInput::DoubleClick(at) => EditInput::DoubleClick(f(at)),
            EditInput::Move { at, shift } => EditInput::Move { at: f(at), shift },
            other => other,
        }
    }
}

/// 編集モードへの入力の結果。空なら入力を受けなかった（キーなら Windows の既定の処理に回す）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOutput {
    /// 書き込みか描きかけが変わった。描き直す。
    Redraw,
    /// 道具が替わった（`EditState::tool`）。同じ道具を選び直しても返す。
    ToolChanged,
    /// 編集モードを抜けた。
    Left,
    /// 文字を打ち始めた（日本語入力を使えるようにする）。
    TypingStarted,
    /// 文字を打ち終えた（日本語入力を止め、変換中の文字は捨てる）。
    TypingEnded,
    /// 変換中の文字を確定したことにして、打った文字に入れた（日本語入力の側の変換中の文字を捨てる）。
    CompositionTaken,
    /// 道具・色・太さを（ツールバーか数字キーで）選んだ。ツールバーを描き直し、`Carryover::record` で次の
    /// 編集モードに引き継ぐ。
    Picked(Pick),
}

/// 編集モードで選べる、これから描くものの道具・色・太さ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Tool(Tool),
    Color(Rgb),
    Width(u32),
    /// 文字の大きさ（元の画像の画素）。
    FontSize(u32),
}

/// 前の編集モードで選んだ道具・色・太さ。アプリが動いている間、すべてのスクラップで引き継ぐ（設定ファイルには
/// 書き戻さない）。色と太さは選ぶまでは設定の既定値に従う。選んだあとでも、設定の既定値を変えればそれが効く
/// （`Carryover::settings_changed`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Carryover {
    /// 最後に使った描く道具（選択と消しゴムは覚えない）。最初は矩形。
    tool: Tool,
    color: Option<Rgb>,
    width: Option<u32>,
    font_size: Option<u32>,
}

impl Carryover {
    /// 編集モードの始まりの状態。`defaults` は設定の `[annotation]`（文字は既定の書体）。
    pub fn start(&self, defaults: Style) -> EditState {
        self.start_with(defaults, Font::default())
    }

    /// 編集モードの始まりの状態。`defaults` と `font` は設定の `[annotation]`。書体は引き継がず、いつも設定の
    /// もの（ツールバーで選べるのは大きさだけ）。
    pub fn start_with(&self, defaults: Style, font: Font) -> EditState {
        let mut state = EditState::new(Style {
            color: self.color.unwrap_or(defaults.color),
            width: self.width.unwrap_or(defaults.width),
        });
        state.tool = self.tool;
        state.font = Font {
            size: self.font_size.unwrap_or(font.size),
            ..font
        };
        state
    }

    /// 設定の `[annotation]` が `old` から `new` に変わった。既定値が変わった色・太さ・文字サイズは、前に選んだ
    /// ものを忘れて新しい既定値に従う（変えた設定がすぐ効くように）。
    pub fn settings_changed(&mut self, old: &AnnotationDefaults, new: &AnnotationDefaults) {
        if old.color != new.color {
            self.color = None;
        }
        if old.width != new.width {
            self.width = None;
        }
        if old.font_size != new.font_size {
            self.font_size = None;
        }
    }

    /// 編集モードの出力のうち、選んだもの（`EditOutput::Picked`）を覚える。
    pub fn record(&mut self, outputs: &[EditOutput]) {
        for output in outputs {
            match *output {
                // 選択と消しゴムは描く道具ではないので、次の編集モードは最後に使った描く道具で始める
                EditOutput::Picked(Pick::Tool(Tool::Select | Tool::Eraser)) => {}
                EditOutput::Picked(Pick::Tool(tool)) => self.tool = tool,
                EditOutput::Picked(Pick::Color(color)) => self.color = Some(color),
                EditOutput::Picked(Pick::Width(width)) => self.width = Some(width),
                EditOutput::Picked(Pick::FontSize(size)) => self.font_size = Some(size),
                _ => {}
            }
        }
    }
}

/// 矢印の頭（三角形）。座標は描く先のもの。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrowHead {
    /// 矢印の先（終点）。
    pub tip: (f64, f64),
    /// 三角形の底辺の中点。矢印の軸はここまで引く（太い線の端が頭の先からはみ出さないように）。
    pub base: (f64, f64),
    /// 底辺の両端。矢印の向きに向かって左と右。
    pub left: (f64, f64),
    pub right: (f64, f64),
}

/// 太さ 1 あたりの矢印の頭の長さと、底辺の半分の幅。
const ARROW_HEAD_LENGTH: f64 = 4.0;
const ARROW_HEAD_HALF_WIDTH: f64 = 2.0;

/// `from` から `to` への矢印（描く先の座標と太さ）の頭。大きさは太さに比例し、矢印が頭より短ければ頭を
/// 矢印の長さに縮める。長さ 0 なら None。
pub fn arrow_head(from: (f64, f64), to: (f64, f64), width: f64) -> Option<ArrowHead> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return None;
    }
    let (ux, uy) = (dx / length, dy / length);
    let head = (width * ARROW_HEAD_LENGTH).min(length);
    let half = head * ARROW_HEAD_HALF_WIDTH / ARROW_HEAD_LENGTH;
    let base = (to.0 - ux * head, to.1 - uy * head);
    // 矢印の向きを右に 90° 回した向き（画面の座標は y が下向き）
    let (nx, ny) = (-uy, ux);
    Some(ArrowHead {
        tip: to,
        base,
        left: (base.0 - nx * half, base.1 - ny * half),
        right: (base.0 + nx * half, base.1 + ny * half),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Rgb;

    fn p(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }

    fn digit(n: u16) -> EditInput {
        EditInput::Key(Chord::plain(Key::known(0x30 + n)))
    }

    const BLUE: Style = Style {
        color: Rgb(0x1E, 0x88, 0xE5),
        width: 5,
    };

    /// `from` から `to` へ 1 回でドラッグして離す。
    fn drag(state: &mut EditState, annotations: &mut Annotations, from: Point, to: Point) {
        state.input(EditInput::Press(from), annotations);
        state.input(
            EditInput::Move {
                at: to,
                shift: false,
            },
            annotations,
        );
        state.input(EditInput::Release, annotations);
    }

    /// `Shift` を押しながら `from` から `to` へドラッグして離した書き込みの形。
    fn shift_drag(tool: Tool, from: Point, to: Point) -> Option<Shape> {
        let mut state = EditState::new(BLUE);
        state.tool = tool;
        let mut annotations = Annotations::default();
        state.input(EditInput::Press(from), &mut annotations);
        state.input(
            EditInput::Move {
                at: to,
                shift: true,
            },
            &mut annotations,
        );
        state.input(EditInput::Release, &mut annotations);
        annotations.items().first().map(|a| a.shape.clone())
    }

    #[test]
    fn shift_makes_the_rect_a_square_and_the_ellipse_a_circle_on_the_longer_side() {
        assert_eq!(
            shift_drag(Tool::Rect, p(10, 20), p(40, 30)),
            Some(Shape::Rect {
                from: p(10, 20),
                to: p(40, 50),
            })
        );
        // 左上へのドラッグは左上へ広がる
        assert_eq!(
            shift_drag(Tool::Ellipse, p(50, 50), p(45, 20)),
            Some(Shape::Ellipse {
                from: p(50, 50),
                to: p(20, 20),
            })
        );
        // 縦か横にまっすぐなドラッグも正方形になる
        assert_eq!(
            shift_drag(Tool::Rect, p(10, 10), p(10, 30)),
            Some(Shape::Rect {
                from: p(10, 10),
                to: p(30, 30),
            })
        );
    }

    #[test]
    fn the_second_number_key_picks_the_ellipse_and_a_drag_draws_one_with_the_style() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        assert_ne!(state.input(digit(2), &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Ellipse);
        drag(&mut state, &mut annotations, p(10, 20), p(40, 30));
        assert_eq!(
            annotations.items(),
            &[Annotation {
                shape: Shape::Ellipse {
                    from: p(10, 20),
                    to: p(40, 30),
                },
                style: BLUE,
            }]
        );
    }

    #[test]
    fn the_third_and_fourth_number_keys_draw_an_arrow_and_a_line_from_start_to_end() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(3), &mut annotations);
        assert_eq!(state.tool, Tool::Arrow);
        drag(&mut state, &mut annotations, p(40, 30), p(10, 20));
        state.input(digit(4), &mut annotations);
        assert_eq!(state.tool, Tool::Line);
        drag(&mut state, &mut annotations, p(1, 2), p(3, 4));
        let shapes: Vec<_> = annotations
            .items()
            .iter()
            .map(|a| a.shape.clone())
            .collect();
        assert_eq!(
            shapes,
            vec![
                Shape::Arrow {
                    from: p(40, 30),
                    to: p(10, 20),
                },
                Shape::Line {
                    from: p(1, 2),
                    to: p(3, 4),
                },
            ]
        );
        assert!(annotations.items().iter().all(|a| a.style == BLUE));
    }

    #[test]
    fn shift_snaps_lines_and_arrows_to_the_nearest_45_degrees() {
        let line = |from, to| Shape::Line { from, to };
        // ほぼ横は横、ほぼ縦は縦
        assert_eq!(
            shift_drag(Tool::Line, p(10, 10), p(50, 18)),
            Some(line(p(10, 10), p(50, 10)))
        );
        assert_eq!(
            shift_drag(Tool::Line, p(10, 10), p(4, -30)),
            Some(line(p(10, 10), p(10, -30)))
        );
        // 斜めは縦と横の平均の長さで 45°
        assert_eq!(
            shift_drag(Tool::Line, p(10, 10), p(-20, 34)),
            Some(line(p(10, 10), p(-17, 37)))
        );
        assert_eq!(
            shift_drag(Tool::Arrow, p(0, 0), p(30, 20)),
            Some(Shape::Arrow {
                from: p(0, 0),
                to: p(25, 25),
            })
        );
    }

    #[test]
    fn the_sixth_number_key_draws_freehand_keeping_only_points_a_step_apart_and_the_last() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(6), &mut annotations);
        assert_eq!(state.tool, Tool::Freehand);
        state.input(EditInput::Press(p(0, 0)), &mut annotations);
        for (at, shift) in [
            (p(1, 0), false),
            (p(2, 0), true),
            (p(3, 0), false),
            (p(4, 1), false),
            (p(4, 2), false),
        ] {
            state.input(EditInput::Move { at, shift }, &mut annotations);
        }
        let points = vec![p(0, 0), p(2, 0), p(4, 1), p(4, 2)];
        // 描きかけも同じ点で描く
        assert_eq!(
            state.draft().map(|a| a.shape),
            Some(Shape::Freehand {
                points: points.clone(),
            })
        );
        state.input(EditInput::Release, &mut annotations);
        assert_eq!(
            annotations.items(),
            &[Annotation {
                shape: Shape::Freehand { points },
                style: BLUE,
            }]
        );
    }

    #[test]
    fn a_freehand_stroke_that_ends_where_it_started_without_leaving_is_dropped() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(6), &mut annotations);
        state.input(EditInput::Press(p(5, 5)), &mut annotations);
        state.input(
            EditInput::Move {
                at: p(6, 5),
                shift: false,
            },
            &mut annotations,
        );
        state.input(
            EditInput::Move {
                at: p(5, 5),
                shift: false,
            },
            &mut annotations,
        );
        assert_eq!(state.input(EditInput::Release, &mut annotations), vec![]);
        assert!(annotations.is_empty());
    }

    #[test]
    fn number_keys_past_seven_and_modified_number_keys_are_not_taken() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(3), &mut annotations);
        assert_eq!(state.input(digit(8), &mut annotations), vec![]);
        // 修飾キーつきの数字は道具を選ばない
        let ctrl_1 = EditInput::Key(Chord::plain(Key::known(0x31)).ctrl());
        assert_eq!(state.input(ctrl_1, &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Arrow);
        // テンキーでも選べる
        let numpad_1 = EditInput::Key(Chord::plain(Key::known(0x61)));
        assert_ne!(state.input(numpad_1, &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Rect);
    }

    #[test]
    fn switching_the_tool_mid_drag_keeps_what_was_drawn_with_the_old_tool() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(EditInput::Press(p(0, 0)), &mut annotations);
        state.input(
            EditInput::Move {
                at: p(9, 9),
                shift: false,
            },
            &mut annotations,
        );
        assert_eq!(
            state.input(digit(4), &mut annotations),
            vec![
                EditOutput::Redraw,
                EditOutput::ToolChanged,
                EditOutput::Picked(Pick::Tool(Tool::Line))
            ]
        );
        assert_eq!(
            annotations.items()[0].shape,
            Shape::Rect {
                from: p(0, 0),
                to: p(9, 9),
            }
        );
        assert_eq!(state.draft(), None);
    }

    fn key(vk: u16) -> EditInput {
        EditInput::Key(Chord::plain(Key::known(vk)))
    }

    const V: u16 = 0x56;

    /// 左ボタンを押して離す（動かさない）。
    fn click(state: &mut EditState, annotations: &mut Annotations, at: Point) -> Vec<EditOutput> {
        let mut outputs = state.input(EditInput::Press(at), annotations);
        outputs.extend(state.input(EditInput::Release, annotations));
        outputs
    }

    /// 道具 `tool` で `from` から `to` へ描く。
    fn draw(
        state: &mut EditState,
        annotations: &mut Annotations,
        tool: Tool,
        from: Point,
        to: Point,
    ) {
        state.input(EditInput::Pick(Pick::Tool(tool)), annotations);
        drag(state, annotations, from, to);
    }

    #[test]
    fn v_picks_the_select_tool_and_a_click_on_the_edge_of_a_rect_selects_it() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(
            &mut state,
            &mut annotations,
            Tool::Rect,
            p(10, 10),
            p(50, 40),
        );
        state.input(key(V), &mut annotations);
        assert_eq!(state.tool, Tool::Select);
        assert_eq!(state.selected(), None);
        assert_eq!(
            click(&mut state, &mut annotations, p(30, 11)),
            vec![EditOutput::Redraw]
        );
        assert_eq!(state.selected(), Some(0));
        // 選択の道具のクリックでは描かない
        assert_eq!(annotations.items().len(), 1);
    }

    fn annotation(shape: Shape, width: u32) -> Annotation {
        Annotation {
            shape,
            style: Style {
                color: BLUE.color,
                width,
            },
        }
    }

    #[test]
    fn rects_and_ellipses_are_hit_on_their_outline_but_not_inside() {
        let rect = annotation(
            Shape::Rect {
                from: p(50, 40),
                to: p(10, 10),
            },
            3,
        );
        assert!(rect.hits(p(10, 25), 0));
        assert!(rect.hits(p(51, 25), 0)); // 太さの半分（1.5）の内
        assert!(!rect.hits(p(52, 25), 0));
        assert!(rect.hits(p(53, 25), 2)); // slop の分だけ広がる
        assert!(!rect.hits(p(30, 25), 0));
        let ellipse = annotation(
            Shape::Ellipse {
                from: p(0, 0),
                to: p(100, 50),
            },
            2,
        );
        assert!(ellipse.hits(p(50, 0), 0));
        assert!(ellipse.hits(p(101, 25), 0));
        assert!(ellipse.hits(p(50, 50), 0));
        assert!(!ellipse.hits(p(50, 25), 0));
        // 外接矩形の角は楕円から遠い
        assert!(!ellipse.hits(p(2, 2), 0));
    }

    #[test]
    fn lines_freehand_and_arrows_are_hit_along_their_strokes_and_arrows_on_their_heads() {
        let line = annotation(
            Shape::Line {
                from: p(0, 0),
                to: p(100, 100),
            },
            2,
        );
        assert!(line.hits(p(50, 50), 0));
        assert!(line.hits(p(51, 50), 0));
        assert!(!line.hits(p(54, 50), 0));
        // 端より先は当たらない
        assert!(!line.hits(p(103, 103), 0));
        let freehand = annotation(
            Shape::Freehand {
                points: vec![p(0, 0), p(20, 0), p(20, 20)],
            },
            1,
        );
        assert!(freehand.hits(p(10, 0), 0));
        assert!(freehand.hits(p(20, 10), 0));
        assert!(!freehand.hits(p(10, 10), 0));
        // 太さ 3 の矢印の頭は長さ 12、底辺の幅 12（x = 88 から 100）。軸から 3 離れた頭の中も当たる
        let arrow = annotation(
            Shape::Arrow {
                from: p(0, 0),
                to: p(100, 0),
            },
            3,
        );
        assert!(arrow.hits(p(50, 1), 0));
        assert!(!arrow.hits(p(50, 5), 0));
        assert!(arrow.hits(p(92, 3), 0));
        assert!(!arrow.hits(p(98, 4), 0));
        assert!(!arrow.hits(p(85, 5), 0));
    }

    #[test]
    fn where_annotations_overlap_the_click_selects_the_one_on_top_and_an_empty_spot_clears_it() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 20),
            p(100, 20),
        );
        draw(
            &mut state,
            &mut annotations,
            Tool::Rect,
            p(10, 20),
            p(60, 50),
        );
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 80),
            p(100, 80),
        );
        state.input(key(V), &mut annotations);
        // 線と矩形の上の辺が重なる。後に描いた矩形が上
        click(&mut state, &mut annotations, p(30, 20));
        assert_eq!(state.selected(), Some(1));
        click(&mut state, &mut annotations, p(80, 20));
        assert_eq!(state.selected(), Some(0));
        // 選んでいるものをもう一度押しても何も変わらない
        assert_eq!(click(&mut state, &mut annotations, p(80, 20)), vec![]);
        assert_eq!(
            click(&mut state, &mut annotations, p(30, 65)),
            vec![EditOutput::Redraw]
        );
        assert_eq!(state.selected(), None);
    }

    fn shapes(annotations: &Annotations) -> Vec<Shape> {
        annotations
            .items()
            .iter()
            .map(|a| a.shape.clone())
            .collect()
    }

    #[test]
    fn dragging_an_annotation_with_the_select_tool_selects_it_and_moves_it_by_the_drag() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(
            &mut state,
            &mut annotations,
            Tool::Rect,
            p(10, 10),
            p(50, 40),
        );
        draw(
            &mut state,
            &mut annotations,
            Tool::Freehand,
            p(100, 100),
            p(110, 100),
        );
        state.input(key(V), &mut annotations);
        state.input(EditInput::Press(p(10, 30)), &mut annotations);
        for at in [p(12, 31), p(15, 35)] {
            let outputs = state.input(EditInput::Move { at, shift: false }, &mut annotations);
            assert_eq!(outputs, vec![EditOutput::Redraw]);
        }
        state.input(EditInput::Release, &mut annotations);
        assert_eq!(state.selected(), Some(0));
        state.input(EditInput::Press(p(104, 100)), &mut annotations);
        state.input(
            EditInput::Move {
                at: p(100, 90),
                shift: false,
            },
            &mut annotations,
        );
        state.input(EditInput::Release, &mut annotations);
        assert_eq!(
            shapes(&annotations),
            vec![
                Shape::Rect {
                    from: p(15, 15),
                    to: p(55, 45),
                },
                Shape::Freehand {
                    points: vec![p(96, 90), p(106, 90)],
                },
            ]
        );
        // 離したあとのカーソルの移動では動かない
        let moved = state.input(
            EditInput::Move {
                at: p(0, 0),
                shift: false,
            },
            &mut annotations,
        );
        assert_eq!(moved, vec![]);
    }

    #[test]
    fn dragging_from_an_empty_spot_moves_nothing() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(10, 0));
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(5, 0));
        let before = annotations.clone();
        drag(&mut state, &mut annotations, p(50, 50), p(60, 60));
        assert_eq!(annotations, before);
        assert_eq!(state.selected(), None);
    }

    const LEFT: u16 = 0x25;
    const UP: u16 = 0x26;
    const RIGHT: u16 = 0x27;
    const DOWN: u16 = 0x28;

    #[test]
    fn arrow_keys_move_the_selected_annotation_a_pixel_or_ten_with_shift() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(10, 10),
            p(20, 10),
        );
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(15, 10));
        for vk in [RIGHT, RIGHT, DOWN, LEFT, DOWN, DOWN] {
            assert_eq!(
                state.input(key(vk), &mut annotations),
                vec![EditOutput::Redraw]
            );
        }
        let shift_up = EditInput::Key(Chord::plain(Key::known(UP)).shift());
        state.input(shift_up, &mut annotations);
        assert_eq!(
            shapes(&annotations),
            vec![Shape::Line {
                from: p(11, 3),
                to: p(21, 3),
            }]
        );
        // Ctrl つきの矢印は受けない
        let ctrl_up = EditInput::Key(Chord::plain(Key::known(UP)).ctrl());
        assert_eq!(state.input(ctrl_up, &mut annotations), vec![]);
    }

    #[test]
    fn arrow_keys_do_nothing_when_nothing_is_selected() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(
            &mut state,
            &mut annotations,
            Tool::Rect,
            p(10, 10),
            p(20, 20),
        );
        let before = annotations.clone();
        for vk in [LEFT, UP, RIGHT, DOWN] {
            assert_eq!(state.input(key(vk), &mut annotations), vec![]);
        }
        state.input(key(V), &mut annotations);
        for vk in [LEFT, UP, RIGHT, DOWN] {
            assert_eq!(state.input(key(vk), &mut annotations), vec![]);
        }
        assert_eq!(annotations, before);
    }

    #[test]
    fn picking_a_colour_or_width_while_selected_applies_it_to_the_annotation_and_the_tool() {
        let red = Rgb(0xE5, 0x39, 0x35);
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(
            &mut state,
            &mut annotations,
            Tool::Ellipse,
            p(0, 0),
            p(40, 20),
        );
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 50),
            p(40, 50),
        );
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(20, 0));
        let outputs = state.input(EditInput::Pick(Pick::Color(red)), &mut annotations);
        assert_eq!(
            outputs,
            vec![EditOutput::Redraw, EditOutput::Picked(Pick::Color(red))]
        );
        state.input(EditInput::Pick(Pick::Width(8)), &mut annotations);
        let changed = Style {
            color: red,
            width: 8,
        };
        assert_eq!(annotations.items()[0].style, changed);
        // 選んでいないものは変わらない
        assert_eq!(annotations.items()[1].style, BLUE);
        // 今のツールの色と太さもその値になり、次に描くものに効く
        assert_eq!(state.style, changed);
        draw(
            &mut state,
            &mut annotations,
            Tool::Rect,
            p(60, 60),
            p(70, 70),
        );
        assert_eq!(annotations.items()[2].style, changed);
    }

    #[test]
    fn picking_a_colour_with_nothing_selected_changes_only_the_tool() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(V), &mut annotations);
        let black = Rgb(0x21, 0x21, 0x21);
        let outputs = state.input(EditInput::Pick(Pick::Color(black)), &mut annotations);
        assert_eq!(outputs, vec![EditOutput::Picked(Pick::Color(black))]);
        assert_eq!(annotations.items()[0].style, BLUE);
        assert_eq!(state.style.color, black);
    }

    #[test]
    fn picking_a_colour_for_the_selected_annotation_keeps_its_width_and_the_other_way_round() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        // 太さ 5 の線を描いたあとで、ツールを太さ 2・黒にする
        let black = Rgb(0x21, 0x21, 0x21);
        state.input(EditInput::Pick(Pick::Width(2)), &mut annotations);
        state.input(EditInput::Pick(Pick::Color(black)), &mut annotations);
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(20, 0));
        let white = Rgb(0xFF, 0xFF, 0xFF);
        state.input(EditInput::Pick(Pick::Color(white)), &mut annotations);
        assert_eq!(
            annotations.items()[0].style,
            Style {
                color: white,
                width: 5,
            }
        );
        state.input(EditInput::Pick(Pick::Width(8)), &mut annotations);
        assert_eq!(
            annotations.items()[0].style,
            Style {
                color: white,
                width: 8,
            }
        );
    }

    const DELETE: u16 = 0x2E;
    const BACKSPACE: u16 = 0x08;

    #[test]
    fn delete_backspace_and_the_toolbar_button_remove_the_selected_annotation() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        for y in [0, 10, 20, 30] {
            draw(&mut state, &mut annotations, Tool::Line, p(0, y), p(40, y));
        }
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(20, 10));
        assert_eq!(
            state.input(key(DELETE), &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert_eq!(state.selected(), None);
        click(&mut state, &mut annotations, p(20, 30));
        state.input(key(BACKSPACE), &mut annotations);
        click(&mut state, &mut annotations, p(20, 0));
        assert_eq!(
            state.input(EditInput::Delete, &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert_eq!(
            shapes(&annotations),
            vec![Shape::Line {
                from: p(0, 20),
                to: p(40, 20),
            }]
        );
    }

    #[test]
    fn delete_does_nothing_when_nothing_is_selected() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(V), &mut annotations);
        assert_eq!(state.input(key(DELETE), &mut annotations), vec![]);
        assert_eq!(state.input(key(BACKSPACE), &mut annotations), vec![]);
        assert_eq!(state.input(EditInput::Delete, &mut annotations), vec![]);
        assert_eq!(annotations.items().len(), 1);
    }

    #[test]
    fn picking_a_drawing_tool_drops_the_selection() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(20, 0));
        state.input(digit(1), &mut annotations);
        assert_eq!(state.selected(), None);
        // 矢印キーはもう何も動かさない
        assert_eq!(state.input(key(RIGHT), &mut annotations), vec![]);
    }

    #[test]
    fn the_next_edit_mode_starts_with_the_last_drawing_tool_not_the_select_tool() {
        let mut carryover = Carryover::default();
        let mut state = carryover.start(BLUE);
        let mut annotations = Annotations::default();
        carryover.record(&state.input(digit(4), &mut annotations));
        carryover.record(&state.input(key(V), &mut annotations));
        assert_eq!(state.tool, Tool::Select);
        assert_eq!(carryover.start(BLUE).tool, Tool::Line);
    }

    #[test]
    fn a_changed_default_in_the_settings_wins_over_what_was_picked_before() {
        let old = AnnotationDefaults::default();
        let mut carryover = Carryover::default();
        let mut state = carryover.start_with(Style::from(&old), Font::from(&old));
        let mut annotations = Annotations::default();
        carryover
            .record(&state.input(EditInput::Pick(Pick::Color(Rgb(0, 0, 0))), &mut annotations));
        carryover.record(&state.input(EditInput::Pick(Pick::Width(8)), &mut annotations));
        carryover.record(&state.input(EditInput::Pick(Pick::FontSize(32)), &mut annotations));
        // 色と文字サイズだけ設定で変えた
        let new = AnnotationDefaults {
            color: Rgb(0x00, 0xBC, 0xD4),
            font_size: 24,
            ..AnnotationDefaults::default()
        };
        carryover.settings_changed(&old, &new);
        let state = carryover.start_with(Style::from(&new), Font::from(&new));
        assert_eq!(
            state.style,
            Style {
                color: Rgb(0x00, 0xBC, 0xD4),
                width: 8,
            }
        );
        assert_eq!(state.font.size, 24);
    }

    #[test]
    fn the_bounds_cover_the_stroke_and_the_arrow_head() {
        let line = annotation(
            Shape::Line {
                from: p(50, 30),
                to: p(10, 10),
            },
            3,
        );
        // 太さの半分（1.5）を画素に切り上げて広げる
        assert_eq!(line.bounds(), Rect::new(8, 8, 52, 32));
        let freehand = annotation(
            Shape::Freehand {
                points: vec![p(5, 9), p(1, 4), p(7, 2)],
            },
            2,
        );
        assert_eq!(freehand.bounds(), Rect::new(0, 1, 8, 10));
        // 太さ 3 の矢印の頭は底辺の幅が 12
        let arrow = annotation(
            Shape::Arrow {
                from: p(0, 0),
                to: p(100, 0),
            },
            3,
        );
        assert_eq!(arrow.bounds(), Rect::new(-2, -6, 102, 6));
    }

    #[test]
    fn the_arrow_head_sits_on_the_end_and_grows_with_the_width() {
        assert_eq!(
            arrow_head((0.0, 0.0), (100.0, 0.0), 3.0),
            Some(ArrowHead {
                tip: (100.0, 0.0),
                base: (88.0, 0.0),
                left: (88.0, -6.0),
                right: (88.0, 6.0),
            })
        );
        let head = arrow_head((10.0, 50.0), (10.0, 10.0), 2.0).unwrap();
        assert_eq!((head.tip, head.base), ((10.0, 10.0), (10.0, 18.0)));
        assert_eq!((head.left, head.right), ((6.0, 18.0), (14.0, 18.0)));
        // 頭より短い矢印は、頭を矢印の長さに縮める
        let head = arrow_head((0.0, 0.0), (5.0, 0.0), 3.0).unwrap();
        assert_eq!((head.base, head.left), ((0.0, 0.0), (0.0, -2.5)));
        assert_eq!(arrow_head((1.0, 1.0), (1.0, 1.0), 3.0), None);
    }

    const Z: u16 = 0x5A;
    const Y: u16 = 0x59;

    fn undo() -> EditInput {
        EditInput::Key(Chord::plain(Key::known(Z)).ctrl())
    }

    fn redo() -> EditInput {
        EditInput::Key(Chord::plain(Key::known(Y)).ctrl())
    }

    #[test]
    fn ctrl_z_takes_back_the_last_drawing_and_ctrl_y_puts_it_back() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        draw(&mut state, &mut annotations, Tool::Rect, p(0, 10), p(9, 19));
        assert_eq!(
            state.input(undo(), &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert_eq!(
            shapes(&annotations),
            vec![Shape::Line {
                from: p(0, 0),
                to: p(40, 0),
            }]
        );
        state.input(undo(), &mut annotations);
        assert!(annotations.is_empty());
        // もう戻すものがなければ受けない
        assert_eq!(state.input(undo(), &mut annotations), vec![]);
        assert_eq!(
            state.input(redo(), &mut annotations),
            vec![EditOutput::Redraw]
        );
        state.input(redo(), &mut annotations);
        assert_eq!(
            shapes(&annotations),
            vec![
                Shape::Line {
                    from: p(0, 0),
                    to: p(40, 0),
                },
                Shape::Rect {
                    from: p(0, 10),
                    to: p(9, 19),
                },
            ]
        );
        assert_eq!(state.input(redo(), &mut annotations), vec![]);
    }

    #[test]
    fn a_whole_drag_of_the_selected_annotation_is_taken_back_at_once() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(V), &mut annotations);
        state.input(EditInput::Press(p(20, 0)), &mut annotations);
        for at in [p(21, 1), p(25, 5), p(30, 10)] {
            state.input(EditInput::Move { at, shift: false }, &mut annotations);
        }
        state.input(EditInput::Release, &mut annotations);
        let moved = vec![Shape::Line {
            from: p(10, 10),
            to: p(50, 10),
        }];
        assert_eq!(shapes(&annotations), moved);
        state.input(undo(), &mut annotations);
        assert_eq!(
            shapes(&annotations),
            vec![Shape::Line {
                from: p(0, 0),
                to: p(40, 0),
            }]
        );
        state.input(redo(), &mut annotations);
        assert_eq!(shapes(&annotations), moved);
        // 2 回目の元に戻すは線を描いたことを戻す
        state.input(undo(), &mut annotations);
        state.input(undo(), &mut annotations);
        assert!(annotations.is_empty());
    }

    #[test]
    fn changing_the_colour_or_width_of_the_selected_annotation_is_one_entry_each() {
        let red = Rgb(0xE5, 0x39, 0x35);
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(20, 0));
        state.input(EditInput::Pick(Pick::Color(red)), &mut annotations);
        state.input(EditInput::Pick(Pick::Width(8)), &mut annotations);
        state.input(undo(), &mut annotations);
        assert_eq!(
            annotations.items()[0].style,
            Style {
                color: red,
                width: 5,
            }
        );
        state.input(undo(), &mut annotations);
        assert_eq!(annotations.items()[0].style, BLUE);
        state.input(redo(), &mut annotations);
        assert_eq!(annotations.items()[0].style.color, red);
        // 元に戻しても、これから描くものの色と太さはそのまま
        assert_eq!(state.style.width, 8);
    }

    #[test]
    fn a_deleted_annotation_comes_back_in_its_place_with_ctrl_z() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        for y in [0, 10, 20] {
            draw(&mut state, &mut annotations, Tool::Line, p(0, y), p(40, y));
        }
        let drawn = shapes(&annotations);
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(20, 10));
        state.input(key(DELETE), &mut annotations);
        assert_eq!(annotations.items().len(), 2);
        state.input(undo(), &mut annotations);
        // 重なりの順（真ん中）にも戻る
        assert_eq!(shapes(&annotations), drawn);
        state.input(redo(), &mut annotations);
        assert_eq!(
            shapes(&annotations),
            vec![drawn[0].clone(), drawn[2].clone()]
        );
    }

    #[test]
    fn each_arrow_key_move_is_taken_back_one_at_a_time() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(20, 0));
        state.input(key(RIGHT), &mut annotations);
        state.input(key(DOWN), &mut annotations);
        state.input(undo(), &mut annotations);
        assert_eq!(
            shapes(&annotations),
            vec![Shape::Line {
                from: p(1, 0),
                to: p(41, 0),
            }]
        );
        // 元に戻すと選択が外れる
        assert_eq!(state.selected(), None);
        state.input(undo(), &mut annotations);
        assert_eq!(
            shapes(&annotations),
            vec![Shape::Line {
                from: p(0, 0),
                to: p(40, 0),
            }]
        );
    }

    #[test]
    fn a_new_change_after_ctrl_z_drops_what_could_be_redone() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 10),
            p(40, 10),
        );
        state.input(undo(), &mut annotations);
        assert!(annotations.can_redo());
        draw(&mut state, &mut annotations, Tool::Rect, p(0, 20), p(9, 29));
        assert!(!annotations.can_redo());
        assert_eq!(state.input(redo(), &mut annotations), vec![]);
        assert_eq!(
            shapes(&annotations),
            vec![
                Shape::Line {
                    from: p(0, 0),
                    to: p(40, 0),
                },
                Shape::Rect {
                    from: p(0, 20),
                    to: p(9, 29),
                },
            ]
        );
    }

    #[test]
    fn ctrl_shift_z_also_redoes_and_ctrl_z_mid_drag_takes_back_the_drawing_in_progress() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(EditInput::Press(p(0, 0)), &mut annotations);
        state.input(
            EditInput::Move {
                at: p(30, 30),
                shift: false,
            },
            &mut annotations,
        );
        assert_eq!(
            state.input(undo(), &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert!(annotations.is_empty());
        assert_eq!(state.draft(), None);
        let ctrl_shift_z = EditInput::Key(Chord::plain(Key::known(Z)).ctrl().shift());
        assert_eq!(
            state.input(ctrl_shift_z, &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert_eq!(annotations.items().len(), 1);
        // ツールバーのボタンも同じ
        assert_eq!(
            state.input(EditInput::Undo, &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert!(annotations.is_empty());
        assert_eq!(
            state.input(EditInput::Redo, &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert_eq!(annotations.items().len(), 1);
    }

    fn chars(state: &mut EditState, annotations: &mut Annotations, text: &str) {
        for c in text.chars() {
            state.input(EditInput::Char(c), annotations);
        }
    }

    fn text(at: Point, text: &str, size: u32) -> Shape {
        Shape::Text {
            at,
            text: text.to_owned(),
            font: Font {
                family: "Yu Gothic UI".to_owned(),
                size,
            },
        }
    }

    const ENTER: u16 = 0x0D;
    const ESC: u16 = 0x1B;

    #[test]
    fn the_fifth_number_key_picks_text_and_a_click_types_from_that_pixel_until_a_click_outside() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        assert_eq!(state.tool, Tool::Text);
        assert_eq!(
            click(&mut state, &mut annotations, p(10, 20)),
            vec![EditOutput::TypingStarted, EditOutput::Redraw]
        );
        assert!(state.is_typing());
        assert_eq!(
            state.input(EditInput::Char('a'), &mut annotations),
            vec![EditOutput::Redraw]
        );
        chars(&mut state, &mut annotations, "あい");
        // 打っている間は書き込みにならず、打っているものとして描く
        assert!(annotations.is_empty());
        assert_eq!(
            state.typing().map(|a| a.shape),
            Some(text(p(10, 20), "aあい", 16))
        );
        // 打っている所の中のクリックでは確定しない
        assert_eq!(click(&mut state, &mut annotations, p(12, 25)), vec![]);
        assert_eq!(
            click(&mut state, &mut annotations, p(300, 300)),
            vec![EditOutput::Redraw, EditOutput::TypingEnded]
        );
        assert!(!state.is_typing());
        assert_eq!(
            annotations.items(),
            &[Annotation {
                shape: text(p(10, 20), "aあい", 16),
                style: BLUE,
            }]
        );
        // 足したのは履歴の 1 件
        state.input(undo(), &mut annotations);
        assert!(annotations.is_empty());
        assert!(!annotations.can_undo());
    }

    #[test]
    fn enter_breaks_the_line_backspace_takes_back_a_char_and_control_chars_are_ignored() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(0, 0));
        chars(&mut state, &mut annotations, "ab");
        assert_eq!(
            state.input(key(ENTER), &mut annotations),
            vec![EditOutput::Redraw]
        );
        // Enter と Backspace の WM_CHAR（\r と \x08）は文字にしない
        chars(&mut state, &mut annotations, "\r\x08\tcd");
        assert_eq!(
            state.input(key(0x08), &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert_eq!(
            state.typing().map(|a| a.shape),
            Some(text(p(0, 0), "ab\nc", 16))
        );
        // 打つ間は数字キーや V で道具を替えず、Delete や矢印も受けない
        for vk in [0x31, 0x56, 0x2E, 0x25] {
            assert_eq!(state.input(key(vk), &mut annotations), vec![], "{vk:#x}");
        }
        assert_eq!(state.tool, Tool::Text);
        assert!(state.is_typing());
    }

    #[test]
    fn esc_while_typing_only_finishes_the_text_and_the_next_esc_leaves() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(5, 5));
        chars(&mut state, &mut annotations, "x");
        assert_eq!(
            state.input(key(ESC), &mut annotations),
            vec![EditOutput::Redraw, EditOutput::TypingEnded]
        );
        assert_eq!(annotations.items().len(), 1);
        assert_eq!(
            state.input(key(ESC), &mut annotations),
            vec![EditOutput::Left]
        );
    }

    #[test]
    fn a_text_with_nothing_but_spaces_leaves_nothing() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(5, 5));
        click(&mut state, &mut annotations, p(300, 300));
        click(&mut state, &mut annotations, p(5, 5));
        chars(&mut state, &mut annotations, " ");
        state.input(key(ENTER), &mut annotations);
        state.input(key(ESC), &mut annotations);
        assert!(annotations.is_empty());
        assert!(!annotations.can_undo());
    }

    #[test]
    fn leaving_or_picking_a_tool_while_typing_keeps_the_text() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(5, 5));
        chars(&mut state, &mut annotations, "a");
        assert_eq!(
            state.input(EditInput::Pick(Pick::Tool(Tool::Rect)), &mut annotations),
            vec![
                EditOutput::Redraw,
                EditOutput::TypingEnded,
                EditOutput::ToolChanged,
                EditOutput::Picked(Pick::Tool(Tool::Rect))
            ]
        );
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(50, 50));
        chars(&mut state, &mut annotations, "b");
        assert_eq!(
            state.input(EditInput::Leave, &mut annotations),
            vec![
                EditOutput::Redraw,
                EditOutput::TypingEnded,
                EditOutput::Left
            ]
        );
        assert_eq!(
            shapes(&annotations),
            vec![text(p(5, 5), "a", 16), text(p(50, 50), "b", 16)]
        );
    }

    #[test]
    fn a_double_click_on_a_text_types_it_again_and_the_change_is_one_entry() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(10, 10));
        chars(&mut state, &mut annotations, "abc");
        state.input(key(ESC), &mut annotations);
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 100),
            p(50, 100),
        );
        state.input(key(V), &mut annotations);
        // ダブルクリックの 1 回目で選び、2 回目で打ち直しになる
        click(&mut state, &mut annotations, p(12, 15));
        assert_eq!(state.selected(), Some(0));
        assert_eq!(
            state.input(EditInput::DoubleClick(p(12, 15)), &mut annotations),
            vec![EditOutput::TypingStarted, EditOutput::Redraw]
        );
        assert_eq!(state.selected(), None);
        // 打ち直している書き込みは、打っているものとして描く（元のものは描かない）
        assert_eq!(state.retyping(), Some(0));
        chars(&mut state, &mut annotations, "d");
        assert_eq!(
            state.typing().map(|a| a.shape),
            Some(text(p(10, 10), "abcd", 16))
        );
        state.input(EditInput::Release, &mut annotations);
        state.input(key(ESC), &mut annotations);
        assert_eq!(state.retyping(), None);
        assert_eq!(
            shapes(&annotations),
            vec![
                text(p(10, 10), "abcd", 16),
                Shape::Line {
                    from: p(0, 100),
                    to: p(50, 100),
                },
            ]
        );
        state.input(undo(), &mut annotations);
        assert_eq!(annotations.items()[0].shape, text(p(10, 10), "abc", 16));
        // 線の上のダブルクリックは打ち直しにならない（選択の道具のクリックと同じ）
        state.input(EditInput::DoubleClick(p(25, 100)), &mut annotations);
        assert!(!state.is_typing());
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn a_text_typed_again_down_to_nothing_is_removed_and_comes_back_with_ctrl_z() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(10, 10));
        chars(&mut state, &mut annotations, "ab");
        state.input(key(ESC), &mut annotations);
        // 文字の道具のままでも、ダブルクリックで打ち直せる（1 回目のクリックの空の文字は残らない）
        click(&mut state, &mut annotations, p(11, 12));
        state.input(EditInput::DoubleClick(p(11, 12)), &mut annotations);
        assert_eq!(state.retyping(), Some(0));
        state.input(key(0x08), &mut annotations);
        state.input(key(0x08), &mut annotations);
        click(&mut state, &mut annotations, p(300, 300));
        assert!(annotations.is_empty());
        state.input(undo(), &mut annotations);
        assert_eq!(shapes(&annotations), vec![text(p(10, 10), "ab", 16)]);
        state.input(undo(), &mut annotations);
        assert!(annotations.is_empty());
    }

    /// 文字の道具で `at` をクリックして打ち始める。
    fn start_typing_at(state: &mut EditState, annotations: &mut Annotations, at: Point) {
        state.input(digit(5), annotations);
        click(state, annotations, at);
    }

    fn composing(text: &str, cursor: usize) -> EditInput {
        EditInput::Composition {
            result: String::new(),
            composing: Some(Composition {
                text: text.to_owned(),
                cursor,
            }),
        }
    }

    #[test]
    fn a_surrogate_pair_from_wm_char_is_typed_as_one_character() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        start_typing_at(&mut state, &mut annotations, p(10, 10));
        // 😀 は U+1F600（D83D DE00）。前半だけでは何も打たない
        assert_eq!(
            state.input(EditInput::Utf16(0xD83D), &mut annotations),
            vec![]
        );
        assert_eq!(
            state.input(EditInput::Utf16(0xDE00), &mut annotations),
            vec![EditOutput::Redraw]
        );
        // 前半のない後半と、後半の来なかった前半は捨てる
        state.input(EditInput::Utf16(0xDE00), &mut annotations);
        state.input(EditInput::Utf16(0xD83D), &mut annotations);
        state.input(EditInput::Utf16(u16::from(b'a')), &mut annotations);
        assert_eq!(
            state.typing().map(|a| a.shape),
            Some(text(p(10, 10), "\u{1F600}a", 16))
        );
    }

    #[test]
    fn wm_char_is_not_taken_unless_typing() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        assert_eq!(
            state.input(EditInput::Utf16(u16::from(b'a')), &mut annotations),
            vec![]
        );
        assert_eq!(
            state.input(EditInput::Utf16(0xD83D), &mut annotations),
            vec![]
        );
    }

    #[test]
    fn the_ime_composition_is_drawn_after_the_text_and_its_result_is_typed() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        start_typing_at(&mut state, &mut annotations, p(10, 10));
        chars(&mut state, &mut annotations, "a");
        assert_eq!(
            state.input(composing("にほ", 2), &mut annotations),
            vec![EditOutput::Redraw]
        );
        assert_eq!(
            state.composition(),
            Some(&Composition {
                text: "にほ".to_owned(),
                cursor: 2,
            })
        );
        // 変換中の文字は、打った文字にはまだ入らない
        assert_eq!(
            state.typing().map(|a| a.shape),
            Some(text(p(10, 10), "a", 16))
        );
        // 確定した文字を打ち、続けて変換している文字に替える
        state.input(
            EditInput::Composition {
                result: "日本".to_owned(),
                composing: Some(Composition {
                    text: "ご".to_owned(),
                    cursor: 1,
                }),
            },
            &mut annotations,
        );
        assert_eq!(
            state.typing().map(|a| a.shape),
            Some(text(p(10, 10), "a日本", 16))
        );
        assert_eq!(state.composition().map(|c| c.text.as_str()), Some("ご"));
        // 変換を終えた（WM_IME_ENDCOMPOSITION）
        state.input(
            EditInput::Composition {
                result: String::new(),
                composing: None,
            },
            &mut annotations,
        );
        assert_eq!(state.composition(), None);
        state.input(key(ESC), &mut annotations);
        assert_eq!(shapes(&annotations), vec![text(p(10, 10), "a日本", 16)]);
    }

    #[test]
    fn the_ime_composition_is_ignored_unless_typing() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        assert_eq!(state.input(composing("あ", 1), &mut annotations), vec![]);
        assert_eq!(state.composition(), None);
    }

    #[test]
    fn other_inputs_while_composing_take_the_composition_as_typed_first() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        start_typing_at(&mut state, &mut annotations, p(10, 10));
        chars(&mut state, &mut annotations, "a");
        state.input(composing("あ", 1), &mut annotations);
        // カーソルの移動と文字はそのまま（変換は続く）
        state.input(
            EditInput::Move {
                at: p(300, 300),
                shift: false,
            },
            &mut annotations,
        );
        assert!(state.composition().is_some());
        // 色を選ぶと、変換中の文字を確定したことにしてから色を変える。IME の側の残りは捨てさせる
        assert_eq!(
            state.input(EditInput::Pick(Pick::Color(Rgb(0, 0, 0))), &mut annotations),
            vec![
                EditOutput::CompositionTaken,
                EditOutput::Redraw,
                EditOutput::Picked(Pick::Color(Rgb(0, 0, 0)))
            ]
        );
        assert_eq!(state.composition(), None);
        assert_eq!(
            state.typing().map(|a| a.shape),
            Some(text(p(10, 10), "aあ", 16))
        );
        // 外のクリックで確定すると、変換中の文字も書き込みに入る
        state.input(composing("い", 1), &mut annotations);
        let outputs = click(&mut state, &mut annotations, p(300, 300));
        assert_eq!(outputs.first(), Some(&EditOutput::CompositionTaken));
        assert!(outputs.contains(&EditOutput::TypingEnded));
        assert_eq!(shapes(&annotations), vec![text(p(10, 10), "aあい", 16)]);
    }

    #[test]
    fn leaving_while_composing_keeps_the_composition() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        start_typing_at(&mut state, &mut annotations, p(10, 10));
        state.input(composing("かな", 2), &mut annotations);
        let outputs = state.input(EditInput::Leave, &mut annotations);
        assert!(outputs.contains(&EditOutput::Left));
        assert_eq!(shapes(&annotations), vec![text(p(10, 10), "かな", 16)]);
    }

    #[test]
    fn ctrl_z_while_typing_again_keeps_the_text_and_undoes_what_came_before() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        let line = Shape::Line {
            from: p(0, 100),
            to: p(50, 100),
        };
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 100),
            p(50, 100),
        );
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(10, 10));
        chars(&mut state, &mut annotations, "ab");
        state.input(key(ESC), &mut annotations);
        // 線を消すと、文字が 0 番目に詰まる。その文字を打ち直す
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(25, 100));
        state.input(key(0x2E), &mut annotations);
        state.input(EditInput::DoubleClick(p(11, 12)), &mut annotations);
        assert_eq!(state.retyping(), Some(0));
        chars(&mut state, &mut annotations, "c");
        // Ctrl+Z は打ち直しを確定してから戻す（ツールバーの元に戻すと同じ）。戻るのは打ち直しの 1 件
        assert_eq!(
            state.input(undo(), &mut annotations),
            vec![
                EditOutput::Redraw,
                EditOutput::TypingEnded,
                EditOutput::Redraw
            ]
        );
        assert!(!state.is_typing());
        assert_eq!(shapes(&annotations), vec![text(p(10, 10), "ab", 16)]);
        // もう 1 回で線が戻る。文字が線を上書きしない
        state.input(undo(), &mut annotations);
        assert_eq!(shapes(&annotations), vec![line, text(p(10, 10), "ab", 16)]);
    }

    #[test]
    fn ctrl_z_while_typing_again_without_changes_undoes_what_came_before() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        let line = Shape::Line {
            from: p(0, 100),
            to: p(50, 100),
        };
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 100),
            p(50, 100),
        );
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(10, 10));
        chars(&mut state, &mut annotations, "ab");
        state.input(key(ESC), &mut annotations);
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(25, 100));
        state.input(key(0x2E), &mut annotations);
        state.input(EditInput::DoubleClick(p(11, 12)), &mut annotations);
        assert_eq!(state.retyping(), Some(0));
        // 何も変えずに Ctrl+Z。消した線が 0 番目に戻り、打ち直していた文字はそのまま残る
        state.input(undo(), &mut annotations);
        assert!(!state.is_typing());
        assert_eq!(
            shapes(&annotations),
            vec![line.clone(), text(p(10, 10), "ab", 16)]
        );
        // 確定のあとの Esc などで線が文字に置き換わらない
        state.input(key(ESC), &mut annotations);
        assert_eq!(shapes(&annotations), vec![line, text(p(10, 10), "ab", 16)]);
    }

    #[test]
    fn ctrl_y_and_ctrl_shift_z_while_typing_commit_the_text_first() {
        for redo_key in [
            redo(),
            EditInput::Key(Chord::plain(Key::known(Z)).ctrl().shift()),
        ] {
            let mut state = EditState::new(BLUE);
            let mut annotations = Annotations::default();
            state.input(digit(5), &mut annotations);
            click(&mut state, &mut annotations, p(10, 10));
            chars(&mut state, &mut annotations, "ab");
            state.input(key(ESC), &mut annotations);
            state.input(undo(), &mut annotations);
            assert!(annotations.is_empty());
            click(&mut state, &mut annotations, p(50, 50));
            chars(&mut state, &mut annotations, "c");
            state.input(redo_key, &mut annotations);
            assert!(!state.is_typing());
            // 打った文字が確定して、やり直しの先は捨てられる
            assert_eq!(shapes(&annotations), vec![text(p(50, 50), "c", 16)]);
        }
    }

    #[test]
    fn the_font_size_applies_to_the_text_being_typed_or_selected_and_keeps_the_text_as_drawn() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        click(&mut state, &mut annotations, p(10, 10));
        chars(&mut state, &mut annotations, "a");
        assert_eq!(
            state.input(EditInput::Pick(Pick::FontSize(32)), &mut annotations),
            vec![EditOutput::Redraw, EditOutput::Picked(Pick::FontSize(32))]
        );
        state.input(EditInput::Pick(Pick::Color(Rgb(0, 0, 0))), &mut annotations);
        state.input(key(ESC), &mut annotations);
        assert_eq!(state.font.size, 32);
        assert_eq!(
            annotations.items(),
            &[Annotation {
                shape: text(p(10, 10), "a", 32),
                style: Style {
                    color: Rgb(0, 0, 0),
                    width: BLUE.width,
                },
            }]
        );
        // 選んだ文字の大きさを変えるのは履歴の 1 件。太さは文字には効かない
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(12, 12));
        assert_eq!(state.selected(), Some(0));
        state.input(EditInput::Pick(Pick::FontSize(12)), &mut annotations);
        assert_eq!(annotations.items()[0].shape, text(p(10, 10), "a", 12));
        assert_eq!(
            state.input(EditInput::Pick(Pick::Width(8)), &mut annotations),
            vec![EditOutput::Picked(Pick::Width(8))]
        );
        state.input(undo(), &mut annotations);
        assert_eq!(annotations.items()[0].shape, text(p(10, 10), "a", 32));
    }

    #[test]
    fn a_text_is_hit_anywhere_in_its_box_which_grows_with_lines_and_the_size() {
        let a = Annotation {
            shape: text(p(10, 20), "ab\nあいう", 20),
            style: BLUE,
        };
        // 全角は大きさと同じ幅、半角はその半分ほど。行の高さは大きさより少し高い
        let bounds = a.bounds();
        assert_eq!((bounds.left, bounds.top, bounds.right), (10, 20, 70));
        assert!(
            (20 + 2 * 20..=20 + 2 * 30).contains(&bounds.bottom),
            "{bounds:?}"
        );
        assert!(a.hits(p(11, 21), 0));
        assert!(a.hits(p(65, bounds.bottom - 1), 0));
        assert!(!a.hits(p(71, 25), 0));
        assert!(a.hits(p(71, 25), 2));
        assert!(!a.hits(p(10, 19), 0));
    }

    #[test]
    fn the_next_edit_mode_starts_with_the_last_font_size_and_the_font_from_the_settings() {
        let mut carryover = Carryover::default();
        let font = Font {
            family: "Meiryo".to_owned(),
            size: 16,
        };
        let mut state = carryover.start_with(BLUE, font.clone());
        assert_eq!(state.font, font);
        let mut annotations = Annotations::default();
        carryover.record(&state.input(EditInput::Pick(Pick::FontSize(24)), &mut annotations));
        let next = carryover.start_with(
            BLUE,
            Font {
                family: "Yu Mincho".to_owned(),
                size: 16,
            },
        );
        assert_eq!(
            next.font,
            Font {
                family: "Yu Mincho".to_owned(),
                size: 24,
            }
        );
    }

    const E: u16 = 0x45;

    /// 押して `path` の順にカーソルを動かし、離す。
    fn stroke(
        state: &mut EditState,
        annotations: &mut Annotations,
        path: &[Point],
    ) -> Vec<EditOutput> {
        let mut outputs = state.input(EditInput::Press(path[0]), annotations);
        for &at in &path[1..] {
            outputs.extend(state.input(EditInput::Move { at, shift: false }, annotations));
        }
        outputs.extend(state.input(EditInput::Release, annotations));
        outputs
    }

    /// 横線を 3 本（y = 0, 20, 40）と、矩形（(100, 0)-(160, 60)）を 1 つ描いたもの。
    fn three_lines_and_a_rect(state: &mut EditState, annotations: &mut Annotations) {
        for y in [0, 20, 40] {
            draw(state, annotations, Tool::Line, p(0, y), p(40, y));
        }
        draw(state, annotations, Tool::Rect, p(100, 0), p(160, 60));
    }

    #[test]
    fn e_and_the_seventh_number_key_pick_the_eraser_and_it_drops_the_selection() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(V), &mut annotations);
        click(&mut state, &mut annotations, p(10, 0));
        assert_eq!(state.selected(), Some(0));
        assert_eq!(
            state.input(key(E), &mut annotations),
            vec![
                EditOutput::Redraw,
                EditOutput::ToolChanged,
                EditOutput::Picked(Pick::Tool(Tool::Eraser))
            ]
        );
        assert_eq!(state.tool, Tool::Eraser);
        assert_eq!(state.selected(), None);
        state.input(digit(1), &mut annotations);
        assert_eq!(state.tool, Tool::Rect);
        assert_ne!(state.input(digit(7), &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Eraser);
        // 修飾キーつきの E は選ばない
        let ctrl_e = EditInput::Key(Chord::plain(Key::known(E)).ctrl());
        state.input(digit(1), &mut annotations);
        assert_eq!(state.input(ctrl_e, &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Rect);
    }

    #[test]
    fn e_while_typing_does_not_pick_the_eraser() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        state.input(EditInput::Press(p(0, 0)), &mut annotations);
        assert_eq!(state.input(key(E), &mut annotations), vec![]);
        assert_eq!(state.tool, Tool::Text);
        assert!(state.is_typing());
    }

    #[test]
    fn a_stroke_of_the_eraser_removes_only_what_it_touched_and_not_a_rect_it_crossed_inside() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        three_lines_and_a_rect(&mut state, &mut annotations);
        state.input(key(E), &mut annotations);
        // y = 0 と y = 20 の線を縦に切る
        let outputs = stroke(&mut state, &mut annotations, &[p(10, -5), p(10, 25)]);
        assert!(outputs.contains(&EditOutput::Redraw));
        // 矩形の中をなぞる（縁には触れない）
        assert_eq!(
            stroke(
                &mut state,
                &mut annotations,
                &[p(110, 10), p(150, 50), p(110, 50)]
            ),
            vec![]
        );
        assert_eq!(
            shapes(&annotations),
            vec![
                Shape::Line {
                    from: p(0, 40),
                    to: p(40, 40),
                },
                Shape::Rect {
                    from: p(100, 0),
                    to: p(160, 60),
                },
            ]
        );
        // 描かない
        assert_eq!(state.draft(), None);
    }

    #[test]
    fn a_click_of_the_eraser_removes_only_the_topmost_annotation_there() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 10),
            p(40, 10),
        );
        state.input(key(E), &mut annotations);
        assert_eq!(
            click(&mut state, &mut annotations, p(20, 0)),
            vec![EditOutput::Redraw]
        );
        assert_eq!(annotations.items().len(), 2);
        // 何もない所のクリックは何もしない
        assert_eq!(click(&mut state, &mut annotations, p(20, 100)), vec![]);
        assert_eq!(annotations.items().len(), 2);
    }

    #[test]
    fn everything_erased_in_one_stroke_comes_back_with_one_undo() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        three_lines_and_a_rect(&mut state, &mut annotations);
        let drawn = annotations.items().to_vec();
        state.input(key(E), &mut annotations);
        stroke(
            &mut state,
            &mut annotations,
            &[p(10, -5), p(10, 45), p(100, 30)],
        );
        assert!(annotations.is_empty());
        state.input(undo(), &mut annotations);
        assert_eq!(annotations.items(), drawn.as_slice());
        state.input(redo(), &mut annotations);
        assert!(annotations.is_empty());
        state.input(undo(), &mut annotations);
        // 2 回の消しゴムは 2 件
        stroke(&mut state, &mut annotations, &[p(10, 0)]);
        stroke(&mut state, &mut annotations, &[p(10, 20)]);
        state.input(undo(), &mut annotations);
        assert_eq!(annotations.items().len(), 3);
        state.input(undo(), &mut annotations);
        assert_eq!(annotations.items(), drawn.as_slice());
    }

    #[test]
    fn a_double_click_of_the_eraser_on_a_text_erases_it_and_does_not_type_it_again() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        state.input(digit(5), &mut annotations);
        state.input(EditInput::Press(p(0, 0)), &mut annotations);
        chars(&mut state, &mut annotations, "ab");
        state.input(EditInput::Pick(Pick::Tool(Tool::Eraser)), &mut annotations);
        assert_eq!(annotations.items().len(), 1);
        state.input(EditInput::DoubleClick(p(5, 5)), &mut annotations);
        state.input(EditInput::Release, &mut annotations);
        assert!(!state.is_typing());
        assert!(annotations.is_empty());
    }

    #[test]
    fn a_stroke_that_touches_nothing_leaves_no_history() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        state.input(key(E), &mut annotations);
        assert_eq!(
            stroke(&mut state, &mut annotations, &[p(0, 50), p(40, 50)]),
            vec![]
        );
        state.input(undo(), &mut annotations);
        assert!(annotations.is_empty());
    }

    #[test]
    fn undo_mid_stroke_ends_the_stroke_and_the_rest_of_it_erases_nothing() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        three_lines_and_a_rect(&mut state, &mut annotations);
        state.input(key(E), &mut annotations);
        state.input(EditInput::Press(p(10, 0)), &mut annotations);
        state.input(undo(), &mut annotations);
        assert_eq!(annotations.items().len(), 4);
        state.input(
            EditInput::Move {
                at: p(10, 40),
                shift: false,
            },
            &mut annotations,
        );
        state.input(EditInput::Release, &mut annotations);
        assert_eq!(annotations.items().len(), 4);
        // 戻したものはやり直せる
        state.input(redo(), &mut annotations);
        assert_eq!(annotations.items().len(), 3);
    }

    #[test]
    fn the_eraser_does_not_touch_what_is_outside_the_visible_area() {
        let mut state = EditState::new(BLUE);
        let mut annotations = Annotations::default();
        draw(&mut state, &mut annotations, Tool::Line, p(0, 0), p(40, 0));
        draw(
            &mut state,
            &mut annotations,
            Tool::Line,
            p(0, 100),
            p(40, 100),
        );
        state.input(key(E), &mut annotations);
        let cx = EditContext {
            hit_slop: 0,
            visible: Some(Rect::new(0, 50, 200, 200)),
        };
        for input in [
            EditInput::Press(p(10, -5)),
            EditInput::Move {
                at: p(10, 105),
                shift: false,
            },
            EditInput::Release,
        ] {
            state.input_in(input, &mut annotations, &cx);
        }
        assert_eq!(
            shapes(&annotations),
            vec![Shape::Line {
                from: p(0, 0),
                to: p(40, 0),
            }]
        );
    }

    #[test]
    fn the_next_edit_mode_starts_with_the_last_drawing_tool_not_the_eraser() {
        let mut carryover = Carryover::default();
        let mut state = carryover.start(BLUE);
        let mut annotations = Annotations::default();
        carryover.record(&state.input(digit(4), &mut annotations));
        carryover.record(&state.input(key(E), &mut annotations));
        assert_eq!(state.tool, Tool::Eraser);
        assert_eq!(carryover.start(BLUE).tool, Tool::Line);
    }
}
