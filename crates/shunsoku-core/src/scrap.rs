//! スクラップの状態（`ScrapModel`）と見た目（`Appearance`）。
//!
//! 位置はスクリーン座標（物理ピクセル）で持ち、画像の画素に倍率（`ScrapModel::zoom`、100% が画素等倍）を
//! 掛けた大きさで出す。ウィンドウは画像の外側に
//! 枠と影の分だけ大きく作り、画像の位置（`ScrapModel::position`）がちょうど撮った範囲に重なるようにする。
//!
//! トリムは、元の画像（`ScrapModel::image_size`）の中の表示する範囲だけを持つ。画素は持たず、表示する画像は
//! 元の画像から `ScrapModel::shown_image` で切り出す。トリムの範囲の選択（`Mode::Trimming`）は
//! `ScrapModel::trim_input` が受ける。選び方は範囲選択の画面の精密モードと同じ形で、ドラッグで選んだ範囲を
//! つまみ・範囲の中のドラッグ・矢印で直し（ロジックは `adjust` を共有する）、Enter・Space・範囲の中の
//! ダブルクリックで確定する。範囲は表示している画像の画素で持つので、拡大縮小していても端は画素に合う。
//! 描くもの（範囲、つまみ、十字線、拡大鏡、座標とサイズ）は `ScrapModel::trim_view` にまとめる。
//!
//! 書き込み（`crate::annotate`）は元の画像の画素の座標で `ScrapModel::annotations` に持ち、`mode` の外に
//! 置く（モードを抜けても、ごみ箱から戻しても残る）。書き込みの編集モード（`Mode::Editing`）の間の入力は
//! `ScrapModel::edit_input` が受ける。

pub mod ops;

use crate::adjust::{self, CursorShape, Direction, Grab, Handle};
use crate::annotate::{
    Annotation, Annotations, EditContext, EditInput, EditOutput, EditState, Style, View,
};
use crate::geometry::{Point, Rect, Size};
use crate::image::Image;
use crate::settings::{DpiScaling, Rgb, ScrapAppearance};

/// 白紙スクラップの背景色。背景色は見た目ではなく中身なので、「見た目を既定に戻す」では戻らない。
/// 設定ファイルとレシピでは `#RRGGBB` か `transparent` で書く。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    /// 不透明な一色。
    Color(Rgb),
    /// 透明（下地の画素のアルファが 0）。表示では透けて見えるが、透明な部分もつかめる（Win32 層が
    /// 当たり判定のために見えないほど小さいアルファを載せる）。保存画像はアルファ 0 の PNG になる。
    Transparent,
}

impl Background {
    /// 白（設定の既定値）。
    pub const WHITE: Background = Background::Color(Rgb(0xFF, 0xFF, 0xFF));

    /// 設定ファイルとレシピでの透明の書き方。
    const TRANSPARENT: &str = "transparent";

    /// 下地の 1 画素の BGRA（premultiply しない）。
    fn bgra(self) -> [u8; 4] {
        match self {
            Background::Color(Rgb(r, g, b)) => [b, g, r, 0xFF],
            Background::Transparent => [0, 0, 0, 0],
        }
    }
}

impl Default for Background {
    fn default() -> Background {
        Background::WHITE
    }
}

/// `#RRGGBB`（大文字）か `transparent`。
impl std::fmt::Display for Background {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Background::Color(c) => c.fmt(f),
            Background::Transparent => f.write_str(Background::TRANSPARENT),
        }
    }
}

/// `#RRGGBB`（大文字と小文字を問わない）でも `transparent`（小文字だけ）でもない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotBackground;

impl std::str::FromStr for Background {
    type Err = NotBackground;

    fn from_str(s: &str) -> Result<Background, NotBackground> {
        if s == Background::TRANSPARENT {
            return Ok(Background::Transparent);
        }
        s.parse::<Rgb>()
            .map(Background::Color)
            .map_err(|_| NotBackground)
    }
}

/// 書き込みの当たり判定の、線の外の広さ（96 DPI での画面の画素。モニタの DPI で換算する）。
const HIT_SLOP: u32 = 4;

/// スクラップの表示のモード。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    /// 画像をそのまま出す。
    #[default]
    Normal,
    /// トリムの範囲を選んでいる（`ScrapModel::start_trim` から確定か中止まで）。スクラップ操作はどれも効かない。
    Trimming(TrimState),
    /// 書き込みの編集モード（`ScrapModel::start_editing` から抜けるまで）。スクラップ操作は「書き込み」の
    /// ほかは効かない。
    Editing(EditState),
}

/// トリムの範囲の選択の状態。範囲と押した画素は、表示している画像（トリム済みならその部分）の画素。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrimState {
    /// カーソルの位置（スクリーン座標）。まだ動いていなければ None。
    cursor: Option<Point>,
    /// 範囲の外で左ボタンを押した画素。押している間だけ Some。動かすと、そこから選び直す。
    pressed: Option<Point>,
    /// 押してから、押した画素の外へ動いたか。動いたら範囲を選び直す。
    dragged: bool,
    /// つまみか範囲の中をつかんでいる。押している間だけ Some。
    grab: Option<Grab>,
    /// 選んだ範囲。動かさずに押して離しても（ダブルクリックの 1 回目）消さない。
    selection: Option<Rect>,
}

/// トリムの範囲の選択への入力。座標はスクリーン座標（物理ピクセル）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimInput {
    /// 左ボタンを押した。範囲のつまみか中ならつかみ、外なら選び直す。
    Press(Point),
    /// 左ボタンのダブルクリック（2 回目の押下の代わりに来る）。範囲の中なら確定し、ほかは `Press` と同じ。
    DoubleClick(Point),
    /// 左ボタンを押したまま、カーソルが動いた。
    Move(Point),
    /// 左ボタンを押さずに、カーソルが動いた。押していたなら離したことにする。
    Hover(Point),
    /// 左ボタンを離した、またはマウスを捕まえていられなくなった。
    Release,
    /// 矢印（`shift` なら Shift+矢印）。範囲を動かす。
    Arrow { direction: Direction, shift: bool },
    /// Ctrl+矢印（`shift` なら Ctrl+Shift+矢印）。範囲の右下の辺を動かす。
    ResizeArrow { direction: Direction, shift: bool },
    /// Enter か Space。選んだ範囲でトリムする。
    Confirm,
    /// `Esc`。トリムをやめる。
    Cancel,
}

impl TrimInput {
    /// カーソルが `at` に動いたときの入力。押したまま（`Pointer::holding`）でなければ、離したことを取りこぼした
    /// ものとして `Hover` にする。
    pub fn pointer_move(at: Point, pointer: Pointer) -> TrimInput {
        if pointer.holding() {
            TrimInput::Move(at)
        } else {
            TrimInput::Hover(at)
        }
    }
}

/// トリムの範囲を選んでいる間に描くもの（`ScrapModel::trim_view`）。座標はスクリーン座標。Win32 層は入力の前後で
/// これを比べ、変わったときだけ描き直す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimView {
    /// 選んだ範囲。画素の境目に合わせる。まだ選んでいなければ None。
    pub selection: Option<Rect>,
    /// 範囲のつまみ。範囲がないときと、選び直している間は空。
    pub handles: Vec<Rect>,
    /// 十字線を通すカーソルの位置。カーソルが画像の上にないときは None。十字線は画像の矩形の中だけに引く。
    pub crosshair: Option<Point>,
    /// 拡大鏡で見せる画素（表示している画像 `ScrapModel::shown_image` の画素）。カーソルが画像の上にないときは None。
    pub magnifier: Option<Point>,
    /// カーソルの横（拡大鏡の下）に出す文字。カーソルの下の元の画像（トリム前）の画素の座標（`120, 75`）と、
    /// 範囲を選んでいればその大きさ（画素）。カーソルが画像の上にないときは None。
    pub cursor_label: Option<String>,
}

/// カーソルが動いたとき（`WM_MOUSEMOVE`）の左ボタンとマウスの捕まえ方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pointer {
    /// 左ボタンを押している。
    pub left_down: bool,
    /// このウィンドウがマウスを捕まえている。
    pub captured: bool,
}

impl Pointer {
    /// 押したまま動かしている。ドラッグと範囲の選択は、このときだけ続ける。
    pub fn holding(&self) -> bool {
        self.left_down && self.captured
    }

    /// 左ボタンを離したのに、まだマウスを捕まえている（離したことを取りこぼした）。放す。
    pub fn stray_capture(&self) -> bool {
        self.captured && !self.left_down
    }
}

/// 倍率の下限（%）。
pub const ZOOM_MIN: u16 = 10;
/// 倍率の上限（%）。
pub const ZOOM_MAX: u16 = 800;
/// 画素等倍の倍率（%）。
pub const ZOOM_ACTUAL: u16 = 100;

/// コンパクト表示の正方形の一辺（96 DPI での物理ピクセル）。実際の一辺はモニタの DPI で換算する
/// （`ScrapModel::compact_side`）。
pub const COMPACT_SIDE: u32 = 64;

/// 拡大率 100% のモニタの DPI。
const BASE_DPI: u32 = 96;

/// コンパクト表示に畳んだときの位置。広げるときに、畳んだあとの移動の分だけずらすために覚えておく。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactOrigin {
    /// 畳む直前の画像の左上。
    expanded: Point,
    /// 畳んだ直後の正方形の左上。
    square: Point,
    /// 正方形に映す部分の中心（表示する画像の画素）。畳んだときに正方形の中心の下にあった画素。
    focus: Point,
}

/// 拡大縮小の基準点。この点の下にある画像の画素が、倍率を変えても同じ画面上の位置に残る。
/// コンパクト表示に畳むときは、この点が正方形の中心になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomAnchor {
    /// 画像の中心（キーとメニューから）。
    Center,
    /// スクリーン座標の点（ホイールのときのカーソルの位置）。
    At(Point),
}

/// 1 枚のスクラップの状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapModel {
    /// 画像の左上（スクリーン座標、物理ピクセル）。
    pub position: Point,
    /// 元の画像（トリム前）の大きさ（画素）。
    pub image_size: Size,
    pub mode: Mode,
    /// 倍率（%）。画像の画素が基準で、100 が画素等倍。`ZOOM_MIN..=ZOOM_MAX` に収める。
    zoom: u16,
    /// 見た目のスクラップごとの上書き。全体の既定値に重ねる（`ScrapModel::appearance`）。
    pub appearance_overrides: AppearanceOverrides,
    /// ウィンドウのあるモニタの DPI。最初の `set_dpi` までは分からない。
    dpi: Option<u32>,
    /// 「見かけの大きさを保つ」で倍率を計算し直す元: 最後に倍率を決めたときの倍率と DPI。倍率を DPI の比で
    /// 掛けるたびに丸めると、モニタを行き来するうちに倍率がずれていくので、ここから毎回計算する。
    /// 拡大縮小で倍率を変えたら捨てる。
    dpi_origin: Option<(u16, u32)>,
    /// コンパクト表示に畳んでいれば Some。畳んでいる間、`position` は正方形の左上。
    compact: Option<CompactOrigin>,
    /// トリムしていれば、表示する範囲（元の画像の画素）。
    trimmed: Option<Rect>,
    /// 書き込み。元の画像（トリム前）の画素の座標で持つ。
    annotations: Annotations,
    /// 白紙スクラップなら背景色。画像を持つスクラップは None。
    background: Option<Background>,
}

impl ScrapModel {
    pub fn new(position: Point, image_size: Size) -> ScrapModel {
        ScrapModel {
            position,
            image_size,
            mode: Mode::Normal,
            zoom: ZOOM_ACTUAL,
            appearance_overrides: AppearanceOverrides::default(),
            dpi: None,
            dpi_origin: None,
            compact: None,
            trimmed: None,
            annotations: Annotations::default(),
            background: None,
        }
    }

    /// 中身（白紙スクラップなら背景色 `background`、トリムの範囲 `trimmed` と書き込み `annotations`）を戻した
    /// スクラップ（スクラップファイルから開いたとき）。`position` は表示する画像の左上。倍率は 100%、見た目の
    /// 個別値はなし。`trimmed` は `image_size` の中の空でない範囲であること（`scrap_file` が確かめる）。
    pub fn restored(
        position: Point,
        image_size: Size,
        background: Option<Background>,
        trimmed: Option<Rect>,
        annotations: Annotations,
    ) -> ScrapModel {
        ScrapModel {
            background,
            trimmed,
            annotations,
            ..ScrapModel::new(position, image_size)
        }
    }

    /// 白紙スクラップ。`position` に `size` の大きさで、`background` の一色を下地にする。
    pub fn blank(position: Point, size: Size, background: Background) -> ScrapModel {
        ScrapModel {
            background: Some(background),
            ..ScrapModel::new(position, size)
        }
    }

    /// 白紙スクラップの背景色。画像を持つスクラップなら None。
    pub fn background(&self) -> Option<Background> {
        self.background
    }

    /// 白紙スクラップの背景色を `background` にする。下地の画素は持たないので、表示する側は `blank_image` で
    /// 作り直す（ごみ箱から戻すときも、この背景色で塗り直す）。画像を持つスクラップか、同じ背景色なら何もせず
    /// false。
    pub fn set_background(&mut self, background: Background) -> bool {
        match &mut self.background {
            Some(now) if *now != background => {
                *now = background;
                true
            }
            _ => false,
        }
    }

    /// 白紙スクラップか（画像を持たず、背景色の一色を下地にする）。
    pub fn is_blank(&self) -> bool {
        self.background.is_some()
    }

    /// 背景が透明な白紙スクラップか。透明な部分も当たり判定に入れ、コピーではアルファを残す。
    pub fn is_transparent_blank(&self) -> bool {
        self.background == Some(Background::Transparent)
    }

    /// 何も書かれていない透明な白紙スクラップか。画面では見えずにつかめるだけなので、書き込みの編集モードを
    /// 抜けたら（どの抜け方でも）ごみ箱に入れずに捨てる。
    pub fn is_empty_transparent_blank(&self) -> bool {
        self.is_transparent_blank() && self.annotations.is_empty()
    }

    /// 白紙スクラップの元の画像に当たる下地（大きさが `image_size` の、背景色の一色）。画素は持たずに背景色から
    /// 作るので、ごみ箱は白紙スクラップの画素を退避しない。画像を持つスクラップなら None。
    pub fn blank_image(&self) -> Option<Image> {
        let bgra = self.background?.bgra();
        let count = self.image_size.width as usize * self.image_size.height as usize;
        Image::from_bgra(self.image_size, bgra.repeat(count))
    }

    /// コンパクト表示に畳んでいるか。
    pub fn is_compact(&self) -> bool {
        self.compact.is_some()
    }

    /// トリムの範囲を選んでいるか（`Mode::Trimming`）。
    pub const fn is_trimming(&self) -> bool {
        matches!(self.mode, Mode::Trimming(_))
    }

    /// `before` と描く中身が同じか（位置のほかは変わっていない）。同じならウィンドウを動かすだけでよい。
    pub fn same_content(&self, before: &ScrapModel) -> bool {
        self.image_size == before.image_size
            && self.background == before.background
            && self.image_rect().size() == before.image_rect().size()
            && self.display_size() == before.display_size()
            && self.mode == before.mode
            && self.is_compact() == before.is_compact()
            && self.appearance_overrides == before.appearance_overrides
            && self.annotations == before.annotations
    }

    /// コンパクト表示（一辺 `compact_side` の正方形）に畳む。`anchor` が正方形の中心になる（`Center` なら
    /// 画像の中心）。倍率はそのまま残る。もう畳んでいれば false。
    pub fn enter_compact(&mut self, anchor: ZoomAnchor) -> bool {
        if self.is_compact() {
            return false;
        }
        let center = match anchor {
            ZoomAnchor::Center => {
                let size = self.display_size();
                let half = |from: i32, length: u32| from.saturating_add((length / 2) as i32);
                Point::new(
                    half(self.position.x, size.width),
                    half(self.position.y, size.height),
                )
            }
            ZoomAnchor::At(at) => at,
        };
        let half = (self.compact_side() / 2) as i32;
        let square = Point::new(center.x.saturating_sub(half), center.y.saturating_sub(half));
        let unscale = |at: i32, from: i32| {
            let offset = i64::from(at) - i64::from(from);
            (offset * 100)
                .div_euclid(i64::from(self.zoom))
                .clamp(0, i64::from(i32::MAX)) as i32
        };
        let focus = Point::new(
            unscale(center.x, self.position.x),
            unscale(center.y, self.position.y),
        );
        self.compact = Some(CompactOrigin {
            expanded: self.position,
            square,
            focus,
        });
        self.position = square;
        true
    }

    /// コンパクト表示から広げる。畳む前の位置に、畳んだあとに動かした分だけずらして戻す。畳んでいなければ false。
    pub fn leave_compact(&mut self) -> bool {
        let Some(CompactOrigin {
            expanded, square, ..
        }) = self.compact.take()
        else {
            return false;
        };
        let p = self.position;
        self.position = Point::new(
            expanded.x.saturating_add(p.x.saturating_sub(square.x)),
            expanded.y.saturating_add(p.y.saturating_sub(square.y)),
        );
        true
    }

    /// 実際の見た目。全体の既定値 `defaults` に、このスクラップの上書きを重ねる。
    pub fn appearance(&self, defaults: &Appearance) -> Appearance {
        self.appearance_overrides.apply(defaults)
    }

    /// 倍率（%）。
    pub fn zoom(&self) -> u16 {
        self.zoom
    }

    /// 1 段拡大する（100% までは 10%、その上は 25% 刻みの次の段）。上限で変わらなければ false。
    pub fn zoom_in(&mut self, anchor: ZoomAnchor) -> bool {
        self.set_zoom(step_up(self.zoom), anchor)
    }

    /// 1 段縮小する。下限で変わらなければ false。
    pub fn zoom_out(&mut self, anchor: ZoomAnchor) -> bool {
        self.set_zoom(step_down(self.zoom), anchor)
    }

    /// 倍率を `zoom`（%、範囲に収める）にする。変わらなければ false。
    pub fn set_zoom(&mut self, zoom: u16, anchor: ZoomAnchor) -> bool {
        let changed = self.scale_to(zoom, anchor);
        if changed {
            self.dpi_origin = None;
        }
        changed
    }

    /// ウィンドウのあるモニタの DPI が `dpi` になった（開いたときと `WM_DPICHANGED`）。最初の 1 回は覚えるだけ。
    /// `scaling` が「見かけの大きさを保つ」なら、倍率を DPI の比で計算し直す（`anchor` が基準）。
    /// 計算し直した倍率は四捨五入して範囲に収める。表示の大きさが変わったら true。
    pub fn set_dpi(&mut self, dpi: u32, scaling: DpiScaling, anchor: ZoomAnchor) -> bool {
        let dpi = dpi.max(1);
        let Some(from) = self.dpi.replace(dpi) else {
            return false;
        };
        if from == dpi {
            return false;
        }
        let rescaled = match scaling {
            DpiScaling::KeepPixels => {
                self.dpi_origin = None;
                false
            }
            DpiScaling::KeepApparentSize => {
                let (zoom, origin) = *self.dpi_origin.get_or_insert((self.zoom, from));
                let scaled = (u64::from(zoom) * u64::from(dpi) * 2 + u64::from(origin))
                    / (u64::from(origin) * 2);
                let scaled = u16::try_from(scaled).unwrap_or(u16::MAX);
                self.scale_to(scaled, anchor)
            }
        };
        if self.is_compact() {
            // 正方形はどちらの設定でも 64px 相当に保つ
            return self.resize_compact(compact_side_at(from), anchor);
        }
        rescaled
    }

    /// コンパクト表示の正方形の一辺（物理ピクセル）。`COMPACT_SIDE` をウィンドウのあるモニタの DPI で換算する
    /// （DPI が分かるまでは 96 DPI とみなす）。
    pub fn compact_side(&self) -> u32 {
        compact_side_at(self.dpi.unwrap_or(BASE_DPI))
    }

    /// 畳んだ正方形の一辺が `before` から変わったら、`anchor` の下の点が動かないように位置を動かす。
    /// 大きさを変えただけなので、広げるときの移動の量には数えない。変わったら true。
    fn resize_compact(&mut self, before: u32, anchor: ZoomAnchor) -> bool {
        let after = self.compact_side();
        if after == before {
            return false;
        }
        let p = self.position;
        let (anchor_x, anchor_y) = match anchor {
            ZoomAnchor::Center => (
                f64::from(p.x) + f64::from(before) / 2.0,
                f64::from(p.y) + f64::from(before) / 2.0,
            ),
            ZoomAnchor::At(at) => (f64::from(at.x), f64::from(at.y)),
        };
        self.position = Point::new(
            scaled_origin(anchor_x, p.x, before, after),
            scaled_origin(anchor_y, p.y, before, after),
        );
        if let Some(origin) = self.compact.as_mut() {
            let (dx, dy) = (
                self.position.x.saturating_sub(p.x),
                self.position.y.saturating_sub(p.y),
            );
            let square = origin.square;
            origin.square = Point::new(square.x.saturating_add(dx), square.y.saturating_add(dy));
        }
        true
    }

    /// 倍率を `zoom`（%、範囲に収める）にし、`anchor` の下の画素が動かないように位置を動かす。
    fn scale_to(&mut self, zoom: u16, anchor: ZoomAnchor) -> bool {
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        if zoom == self.zoom {
            return false;
        }
        if self.is_compact() {
            // 畳んだ正方形の大きさと位置は倍率によらない。広げたときに新しい倍率で出る
            self.zoom = zoom;
            return false;
        }
        let before = self.display_size();
        self.zoom = zoom;
        let after = self.display_size();
        let p = self.position;
        let (anchor_x, anchor_y) = match anchor {
            ZoomAnchor::Center => (
                f64::from(p.x) + f64::from(before.width) / 2.0,
                f64::from(p.y) + f64::from(before.height) / 2.0,
            ),
            ZoomAnchor::At(at) => (f64::from(at.x), f64::from(at.y)),
        };
        self.position = Point::new(
            scaled_origin(anchor_x, p.x, before.width, after.width),
            scaled_origin(anchor_y, p.y, before.height, after.height),
        );
        true
    }

    /// 倍率を画素等倍に戻す（中心が基準）。等倍のままなら false。
    pub fn reset_zoom(&mut self) -> bool {
        self.set_zoom(ZOOM_ACTUAL, ZoomAnchor::Center)
    }

    /// 画像を出す大きさ（物理ピクセル）。表示する画像の画素に倍率を掛けて四捨五入し、0 にはしない。
    pub fn display_size(&self) -> Size {
        let shown = self.shown_size();
        let scale = |pixels: u32| u32::try_from(self.scale(pixels)).unwrap_or(u32::MAX).max(1);
        Size::new(scale(shown.width), scale(shown.height))
    }

    /// 画素の数に倍率を掛けた表示の長さ（四捨五入）。
    fn scale(&self, pixels: u32) -> u64 {
        (u64::from(pixels) * u64::from(self.zoom) + 50) / 100
    }

    /// 表示する画像の大きさ（画素）。トリムしていればその範囲の大きさ。
    pub fn shown_size(&self) -> Size {
        self.trimmed.map_or(self.image_size, |r| r.size())
    }

    /// トリムしているか。
    pub fn is_trimmed(&self) -> bool {
        self.trimmed.is_some()
    }

    /// トリムしていれば、表示する範囲（元の画像の画素）。
    pub fn trim_rect(&self) -> Option<Rect> {
        self.trimmed
    }

    /// 表示する画像（保存・コピーの対象）。`source` は元の画像（大きさが `image_size` のもの）。
    pub fn shown_image(&self, source: &Image) -> Image {
        match self.trimmed {
            Some(rect) => source.crop(rect),
            None => source.clone(),
        }
    }

    /// トリムの範囲の選択を始める。すでに選んでいれば false。
    pub fn start_trim(&mut self) -> bool {
        if self.is_trimming() {
            return false;
        }
        self.mode = Mode::Trimming(TrimState::default());
        true
    }

    /// トリムの範囲の選択への入力を受ける。描き直しが要る（範囲かカーソルかモードか表示が変わった）なら true。
    /// トリムの最中でなければ何もしない。
    pub fn trim_input(&mut self, input: TrimInput) -> bool {
        let Mode::Trimming(mut state) = self.mode else {
            return false;
        };
        let before = state;
        let bounds = Rect::from_origin_size(Point::default(), self.shown_size());
        match input {
            // 範囲の中のダブルクリックは確定。外なら押したのと同じ
            TrimInput::DoubleClick(at) if self.trim_target(&state, at).is_some() => {
                return self.trim_input(TrimInput::Confirm);
            }
            TrimInput::Press(at) | TrimInput::DoubleClick(at) => {
                state.cursor = Some(at);
                let pixel = self.shown_pixel_at(at);
                let target = self.trim_target(&state, at);
                match (state.selection, target) {
                    (Some(start), Some(handle)) => {
                        state.grab = Some(Grab {
                            handle,
                            from: pixel,
                            start,
                        });
                    }
                    _ => {
                        state.pressed = Some(pixel);
                        state.dragged = false;
                    }
                }
            }
            TrimInput::Move(at) => {
                state.cursor = Some(at);
                let to = self.shown_pixel_at(at);
                if let Some(grab) = state.grab {
                    state.selection = Some(grab.dragged(to, bounds));
                } else if let Some(from) = state.pressed {
                    // 動かさずに押して離しただけなら、前に選んだ範囲を残す
                    state.dragged |= to != from;
                    if state.dragged {
                        state.selection = Some(Rect::new(
                            from.x.min(to.x),
                            from.y.min(to.y),
                            from.x.max(to.x) + 1,
                            from.y.max(to.y) + 1,
                        ));
                    }
                }
            }
            TrimInput::Hover(at) => {
                state.cursor = Some(at);
                state.pressed = None;
                state.grab = None;
            }
            TrimInput::Release => {
                state.pressed = None;
                state.grab = None;
            }
            TrimInput::Arrow { direction, shift } => {
                state.selection = state
                    .selection
                    .map(|r| adjust::moved(r, direction, shift, bounds));
            }
            TrimInput::ResizeArrow { direction, shift } => {
                state.selection = state
                    .selection
                    .map(|r| adjust::resized(r, direction, shift, bounds));
            }
            TrimInput::Cancel => {
                self.mode = Mode::Normal;
                return true;
            }
            TrimInput::Confirm => {
                let Some(selection) = state.selection else {
                    return false;
                };
                self.mode = Mode::Normal;
                self.trim_to(selection);
                return true;
            }
        }
        self.mode = Mode::Trimming(state);
        state != before
    }

    /// トリムの範囲を選んでいる間に描くもの。トリムの最中でなければ None。
    pub fn trim_view(&self) -> Option<TrimView> {
        let Mode::Trimming(state) = self.mode else {
            return None;
        };
        let selection = state.selection.map(|r| self.trim_screen_rect(r));
        let reselecting = state.pressed.is_some() && state.dragged;
        let handles = match selection {
            Some(rect) if !reselecting => {
                adjust::handle_rects(rect, |len, _| self.scaled_by_dpi(len))
            }
            _ => Vec::new(),
        };
        let cursor = state.cursor.filter(|&at| self.image_rect().contains(at));
        let cursor_label = cursor.map(|at| {
            let pixel = self.image_pixel_at(at);
            let at = format!("{}, {}", pixel.x, pixel.y);
            match state.selection {
                Some(r) => format!("{at}   {} × {}", r.width(), r.height()),
                None => at,
            }
        });
        Some(TrimView {
            selection,
            handles,
            crosshair: cursor,
            magnifier: cursor.map(|at| self.shown_pixel_at(at)),
            cursor_label,
        })
    }

    /// トリムの最中のカーソルの形。範囲のつまみの上なら大きさを変える形、範囲の中なら動かす形、ほかは十字。
    /// つかんでいる間は、つかんだものの形のまま。トリムの最中でなければ十字。
    pub fn trim_cursor_shape(&self) -> CursorShape {
        let Mode::Trimming(state) = self.mode else {
            return CursorShape::Cross;
        };
        if let Some(grab) = state.grab {
            return grab.cursor_shape();
        }
        let target = state.cursor.and_then(|at| self.trim_target(&state, at));
        adjust::cursor_shape(target)
    }

    /// トリムの最中に `at`（スクリーン座標）にあるもの。範囲のつまみなら Some(Some)、範囲の中なら
    /// Some(None)、外か範囲がなければ None。
    fn trim_target(&self, state: &TrimState, at: Point) -> Option<Option<Handle>> {
        let rect = self.trim_screen_rect(state.selection?);
        adjust::hit(rect, at, |len, _| self.scaled_by_dpi(len))
    }

    /// 96 DPI での長さを、ウィンドウのあるモニタの物理ピクセルにする。
    fn scaled_by_dpi(&self, len: i32) -> i32 {
        let dpi = self.dpi.unwrap_or(BASE_DPI) as i32;
        (len * dpi + 48) / 96
    }

    /// 選んでいるトリムの範囲（スクリーン座標）。画素の境目に合わせる。選んでいなければ None。
    pub fn trim_selection(&self) -> Option<Rect> {
        let Mode::Trimming(TrimState {
            selection: Some(r), ..
        }) = self.mode
        else {
            return None;
        };
        Some(self.trim_screen_rect(r))
    }

    /// 表示している画像の画素の矩形 `r` を、画面の上の矩形（スクリーン座標）にする。
    fn trim_screen_rect(&self, r: Rect) -> Rect {
        let p = self.position;
        let at = |origin: i32, pixels: i32| {
            let offset = i64::try_from(self.scale(pixels.max(0) as u32)).unwrap_or(i64::MAX);
            (i64::from(origin) + offset).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        Rect::new(
            at(p.x, r.left),
            at(p.y, r.top),
            at(p.x, r.right),
            at(p.y, r.bottom),
        )
    }

    /// スクリーン座標の点の下にある、表示している画像の画素。画像の外なら端の画素に留める。
    fn shown_pixel_at(&self, at: Point) -> Point {
        let shown = self.shown_size();
        let display = self.display_size();
        let pixel = |offset: i32, pixels: u32, length: u32| {
            let px = (i64::from(offset) * i64::from(pixels)).div_euclid(i64::from(length.max(1)));
            px.clamp(0, i64::from(pixels.max(1)) - 1) as i32
        };
        Point::new(
            pixel(at.x - self.position.x, shown.width, display.width),
            pixel(at.y - self.position.y, shown.height, display.height),
        )
    }

    /// スクリーン座標の点の下にある、元の画像（トリム前）の画素。画像の外なら端の画素に留める。
    fn image_pixel_at(&self, at: Point) -> Point {
        let shown = self.shown_pixel_at(at);
        let origin = self.trimmed.map_or(Point::default(), |r| r.origin());
        Point::new(shown.x + origin.x, shown.y + origin.y)
    }

    /// 書き込みの編集モードにいるか（`Mode::Editing`）。
    pub const fn is_editing(&self) -> bool {
        matches!(self.mode, Mode::Editing(_))
    }

    /// 書き込みの編集モードに入る。これから描く書き込みは `style`。ほかのモード（トリムの範囲の選択、
    /// 編集モード）にいるときとコンパクト表示の間は入らずに false。
    pub fn start_editing(&mut self, style: Style) -> bool {
        self.start_editing_with(EditState::new(style))
    }

    /// `start_editing` の、道具・色・太さを決めた状態 `state` から始めるもの（`annotate::Carryover::start`）。
    pub fn start_editing_with(&mut self, state: EditState) -> bool {
        if self.mode != Mode::Normal || self.is_compact() {
            return false;
        }
        self.mode = Mode::Editing(state);
        true
    }

    /// 書き込みの編集モードの状態（いまの道具・色・太さ）。編集モードでなければ None。
    pub fn edit_state(&self) -> Option<&EditState> {
        match &self.mode {
            Mode::Editing(state) => Some(state),
            _ => None,
        }
    }

    /// 書き込みの編集モードへの入力を受ける（座標はスクリーン座標）。抜けたら（`EditOutput::Left`）
    /// `Mode::Normal` に戻す。編集モードでなければ何もせず、空を返す。
    pub fn edit_input(&mut self, input: EditInput) -> Vec<EditOutput> {
        let input = input.map_point(|at| self.image_pixel_at(at));
        let cx = EditContext {
            hit_slop: self.hit_slop(),
            visible: Some(self.visible_area()),
        };
        let Mode::Editing(state) = &mut self.mode else {
            return vec![];
        };
        let outputs = state.input_in(input, &mut self.annotations, &cx);
        if outputs.contains(&EditOutput::Left) {
            self.mode = Mode::Normal;
        }
        outputs
    }

    /// 書き込みの当たり判定を、線の太さの外にどれだけ広げるか（元の画像の画素）。画面で `HIT_SLOP` を
    /// モニタの DPI で換算した物理ピクセルになるようにする（縮小していれば広く、拡大していれば狭い）。
    fn hit_slop(&self) -> u32 {
        let dpi = self.dpi.unwrap_or(BASE_DPI);
        let screen = f64::from(HIT_SLOP) * f64::from(dpi) / f64::from(BASE_DPI);
        let scale = self.annotation_view().scale_x;
        if scale <= 0.0 {
            return screen.ceil() as u32;
        }
        (screen / scale).ceil() as u32
    }

    /// 編集モードで選んでいる書き込み。選択の印を描くのに使う。
    pub fn selected_annotations(&self) -> Vec<&Annotation> {
        let Some(state) = self.edit_state() else {
            return vec![];
        };
        state
            .selected()
            .iter()
            .filter_map(|&i| self.annotations.items().get(i))
            .collect()
    }

    /// 編集モードで範囲で選んでいる間の範囲（元の画像の画素）。範囲を描くのに使う。
    pub fn selection_range(&self) -> Option<Rect> {
        self.edit_state()?.marquee()
    }

    /// 書き込み（描き終えたもの）。
    pub fn annotations(&self) -> &Annotations {
        &self.annotations
    }

    /// 見えている範囲（元の画像の画素）。トリムしていればその範囲、していなければ画像全体。
    fn visible_area(&self) -> Rect {
        self.trimmed
            .unwrap_or_else(|| Rect::from_origin_size(Point::default(), self.image_size))
    }

    /// 描く書き込み。描き終えたもののうち見えている範囲に映るもの（範囲の外のものは、モデルに残したまま
    /// 描かない）の上に、編集モードの描きかけを重ねる。打ち直している文字の元のものは描かない（打っている
    /// 文字は `typing_annotation` で別に描く）。
    pub fn annotations_to_draw(&self) -> Vec<Annotation> {
        let area = self.visible_area();
        let retyping = self.edit_state().and_then(EditState::retyping);
        let mut drawn: Vec<Annotation> = self
            .annotations
            .items()
            .iter()
            .enumerate()
            .filter(|&(i, a)| Some(i) != retyping && a.is_visible_in(area))
            .map(|(_, a)| a.clone())
            .collect();
        if let Mode::Editing(state) = &self.mode {
            drawn.extend(state.draft());
        }
        drawn
    }

    /// 編集モードで打っている文字（打った分だけ。空のこともある）。日本語入力の変換中の文字とカーソルは
    /// `app` がこの後ろに描く。
    pub fn typing_annotation(&self) -> Option<Annotation> {
        self.edit_state()?.typing()
    }

    /// 画面に書き込みを描く変換。描く先の原点は画像を出す矩形（`image_rect`）の左上で、倍率とトリムと
    /// コンパクト表示の正方形に映す部分に合わせる。
    pub fn annotation_view(&self) -> View {
        let origin = self.trimmed.map_or(Point::default(), |r| r.origin());
        let shown = self.shown_size();
        let (from, to, view) = match self.compact_view() {
            Some(view) => {
                let side = self.compact_side();
                (view.size(), Size::new(side, side), view.origin())
            }
            None => (shown, self.display_size(), Point::default()),
        };
        View {
            left: f64::from(origin.x) + f64::from(view.x),
            top: f64::from(origin.y) + f64::from(view.y),
            scale_x: f64::from(to.width) / f64::from(from.width.max(1)),
            scale_y: f64::from(to.height) / f64::from(from.height.max(1)),
        }
    }

    /// 保存とコピーで書き込みを描く変換。描く先は表示する画像（`shown_image`）で、画素の大きさに描く。
    pub fn output_view(&self) -> View {
        let origin = self.trimmed.map_or(Point::default(), |r| r.origin());
        View {
            left: f64::from(origin.x),
            top: f64::from(origin.y),
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }

    /// 閉じる前に、そのときだけのモードを終える。トリムの範囲の選択はやめ、書き込みの編集モードは描きかけを
    /// 書き込みにして抜ける。ごみ箱から戻したときは `Mode::Normal` で、書き込みは残っている。
    pub fn settle(&mut self) {
        self.trim_input(TrimInput::Cancel);
        self.edit_input(EditInput::Leave);
    }

    /// 表示している画像の中の `selection`（画素）だけを表示する。残った部分の画面上の位置は動かさない。
    fn trim_to(&mut self, selection: Rect) {
        let shown = Rect::from_origin_size(Point::default(), self.shown_size());
        if selection == shown {
            return;
        }
        let origin = self.trimmed.map_or(Point::default(), |r| r.origin());
        self.trimmed = Some(selection.offset(origin.x, origin.y));
        self.position = self.shifted(selection.origin(), 1);
    }

    /// トリムを解除して元の画像に戻す。元の画像は、トリムした部分が画面上の同じ位置に来るように出す。
    /// トリムしていなければ false。
    pub fn untrim(&mut self) -> bool {
        let Some(rect) = self.trimmed.take() else {
            return false;
        };
        self.position = self.shifted(rect.origin(), -1);
        true
    }

    /// `position` を、画素の `by` に倍率を掛けた長さだけ `sign` の向きにずらした点。
    fn shifted(&self, by: Point, sign: i64) -> Point {
        let shift = |from: i32, pixels: i32| {
            let offset = i64::try_from(self.scale(pixels.max(0) as u32)).unwrap_or(i64::MAX);
            (i64::from(from) + sign * offset).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        Point::new(shift(self.position.x, by.x), shift(self.position.y, by.y))
    }

    /// コンパクト表示の正方形に映す部分（表示する画像の画素）。畳んだときに正方形の下にあった部分で、
    /// 大きさはいまの倍率で正方形を埋める分。画像からはみ出さないようにずらし、画像より大きければ画像の
    /// 短い辺の正方形にする。畳んでいなければ None。
    pub fn compact_view(&self) -> Option<Rect> {
        let focus = self.compact?.focus;
        let shown = self.shown_size();
        let zoom = u64::from(self.zoom);
        let side = (u64::from(self.compact_side()) * 100 + zoom / 2) / zoom;
        let side = u32::try_from(side)
            .unwrap_or(u32::MAX)
            .clamp(1, shown.width.min(shown.height).max(1));
        let place = |center: i32, length: u32| {
            let from = i64::from(center) - i64::from(side / 2);
            from.clamp(0, i64::from(length.saturating_sub(side))) as i32
        };
        let origin = Point::new(place(focus.x, shown.width), place(focus.y, shown.height));
        Some(Rect::from_origin_size(origin, Size::new(side, side)))
    }

    /// 画像を出す矩形（スクリーン座標）。大きさは倍率を掛けた表示の大きさ。畳んでいる間は正方形。
    pub fn image_rect(&self) -> Rect {
        let size = if self.is_compact() {
            let side = self.compact_side();
            Size::new(side, side)
        } else {
            self.display_size()
        };
        Rect::from_origin_size(self.position, size)
    }

    /// ウィンドウの矩形（スクリーン座標）。画像の外側に、実際の見た目（既定値 `defaults` に上書きを重ねたもの）の
    /// 枠と影の分を足す。
    pub fn window_rect(&self, defaults: &Appearance) -> Rect {
        self.image_rect()
            .inflate(self.appearance(defaults).margin())
    }
}

/// `dpi` のモニタでのコンパクト表示の正方形の一辺（物理ピクセル、四捨五入）。
fn compact_side_at(dpi: u32) -> u32 {
    let side = (u64::from(COMPACT_SIDE) * u64::from(dpi) * 2 + u64::from(BASE_DPI))
        / (u64::from(BASE_DPI) * 2);
    u32::try_from(side).unwrap_or(u32::MAX).max(1)
}

/// 大きさが `before` から `after` に変わるときの、新しい左上の座標（1 軸）。基準点 `anchor` から左上 `from` までの
/// 距離を、大きさの比で伸び縮みさせる。
fn scaled_origin(anchor: f64, from: i32, before: u32, after: u32) -> i32 {
    let ratio = f64::from(after) / f64::from(before.max(1));
    let to = anchor - (anchor - f64::from(from)) * ratio;
    to.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// 1 段上の倍率。100% までは 10% の、その上は 25% の倍数のうち、`zoom` より大きい最初のもの。
fn step_up(zoom: u16) -> u16 {
    let step = if zoom < ZOOM_ACTUAL { 10 } else { 25 };
    (zoom / step + 1) * step
}

/// 1 段下の倍率。100% までは 10% の、その上は 25% の倍数のうち、`zoom` より小さい最初のもの。
fn step_down(zoom: u16) -> u16 {
    let step = if zoom <= ZOOM_ACTUAL { 10 } else { 25 };
    (zoom.saturating_sub(1) / step) * step
}

/// 枠。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border {
    /// 太さ（物理ピクセル）。
    pub width: u32,
    /// 色（`0xRRGGBB`）。
    pub color: u32,
}

/// スクラップの見た目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    /// 透明度（%）。100 で不透明。
    pub opacity: u8,
    /// 枠の太さと色。枠を出さないときも持っておき、スクラップごとに枠を付けたときに使う。
    pub border: Border,
    /// 枠を出すか。
    pub show_border: bool,
    pub shadow: bool,
}

/// 見た目のスクラップごとの上書き。None の項目は全体の既定値（設定ファイルの `[scrap]`）のまま。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppearanceOverrides {
    /// 透明度（%）。
    pub opacity: Option<u8>,
    /// 枠の有無と色。
    pub border: Option<BorderOverride>,
    /// 枠の太さ（物理ピクセル）。枠の有無とは別に持つので、枠を隠したり色を変えたりしても残る。
    /// 物理ピクセルのまま持つのは、全体の既定値 `Border::width`（設定の `[scrap] border_width`）をそのまま
    /// 差し替える値だから。既定値が DPI で換算しない物理ピクセルなので、上書きだけを 96 DPI の値にすると、同じ数を
    /// 書いても `[scrap]` とレシピのパラメータとで太さが違って見える。96 DPI の値にするなら既定値と一緒に変える。
    pub border_width: Option<u32>,
    /// 影の有無。
    pub shadow: Option<bool>,
}

/// 枠のスクラップごとの上書き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderOverride {
    /// 枠を出さない。
    Hidden,
    /// その色（`0xRRGGBB`）の枠を出す。太さは `AppearanceOverrides::border_width`、上書きがなければ既定値の太さ
    /// （既定値で枠を出さなくても設定の太さ）。
    Color(u32),
}

impl AppearanceOverrides {
    /// `defaults` に重ねた見た目。
    fn apply(&self, defaults: &Appearance) -> Appearance {
        let configured = Border {
            width: self.border_width.unwrap_or(defaults.border.width),
            ..defaults.border
        };
        let (border, show_border) = match self.border {
            None => (configured, defaults.show_border),
            Some(BorderOverride::Hidden) => (configured, false),
            Some(BorderOverride::Color(color)) => (
                Border {
                    color,
                    ..configured
                },
                true,
            ),
        };
        Appearance {
            opacity: self.opacity.unwrap_or(defaults.opacity),
            border,
            show_border,
            shadow: self.shadow.unwrap_or(defaults.shadow),
        }
    }
}

/// 影の幅（物理ピクセル）。枠の外側に広がる。
pub const SHADOW_WIDTH: u32 = 8;

impl Default for Appearance {
    /// 見た目の既定値（spec #32）: 透明度 100%、枠あり 1px `#808080`、影あり。
    fn default() -> Appearance {
        Appearance {
            opacity: 100,
            border: Border {
                width: 1,
                color: 0x80_80_80,
            },
            show_border: true,
            shadow: true,
        }
    }
}

impl Appearance {
    /// レイヤード ウィンドウの全体のアルファ（`SourceConstantAlpha`、0〜255）。透明度（%）から換算する。
    pub fn constant_alpha(&self) -> u8 {
        (u32::from(self.opacity.min(100)) * 255 / 100) as u8
    }

    /// 透明な白紙スクラップの透明な部分に載せる、当たり判定のためのアルファ（premultiplied の画素の A）。
    /// レイヤード ウィンドウはアルファが 0 の画素のクリックを下のウィンドウに通すので、全体のアルファ
    /// （`constant_alpha`）を掛けても 0 に落ちない最小の値にする（見た目は透明と変わらない）。
    pub fn hit_alpha(&self) -> u8 {
        let constant = u32::from(self.constant_alpha()).max(1);
        255u32.div_ceil(constant).min(255) as u8
    }

    /// 出す枠。枠を出さないなら None。
    pub fn shown_border(&self) -> Option<Border> {
        self.show_border.then_some(self.border)
    }

    /// 画像の外側に足す幅（枠 + 影）。
    pub fn margin(&self) -> i32 {
        let border = self.shown_border().map_or(0, |b| b.width);
        let shadow = if self.shadow { SHADOW_WIDTH } else { 0 };
        (border + shadow) as i32
    }
}

impl From<ScrapAppearance> for Appearance {
    /// 設定ファイルの `[scrap]` から作る。枠を出さない設定でも、枠の太さと色は持っておく。
    fn from(settings: ScrapAppearance) -> Appearance {
        let border = settings.border;
        Appearance {
            opacity: settings.opacity,
            border: Border {
                width: border.width,
                color: border.color.to_rrggbb(),
            },
            show_border: border.visible,
            shadow: settings.shadow,
        }
    }
}

/// スクラップの左ドラッグ。押したところから離すまでの間だけ、カーソルの移動量を返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Drag {
    /// ドラッグの間、前回のカーソルの位置（スクリーン座標）。
    from: Option<Point>,
}

impl Drag {
    /// 左ボタンを `at`（スクリーン座標）で押した。
    pub fn start(&mut self, at: Point) {
        self.from = Some(at);
    }

    /// 左ボタンを離した、またはマウスを捕まえていられなくなった。
    pub fn end(&mut self) {
        self.from = None;
    }

    pub fn is_active(&self) -> bool {
        self.from.is_some()
    }

    /// 大きさを変えるとき（DPI が変わったときなど）の基準点。ドラッグ中ならつかんだところが離れないようにカーソルの
    /// 位置 `cursor`、そうでなければ中心。
    pub fn zoom_anchor(&self, cursor: Point) -> ZoomAnchor {
        if self.is_active() {
            ZoomAnchor::At(cursor)
        } else {
            ZoomAnchor::Center
        }
    }

    /// カーソルが `at` に動いた。ドラッグの間なら前回からの移動量。ドラッグしていなければ None。
    /// 押したまま（`Pointer::holding`）でなければ、離したことを取りこぼしたものとしてドラッグを終える
    /// （ホバーだけでスクラップが動かないように）。
    pub fn move_to(&mut self, at: Point, pointer: Pointer) -> Option<Point> {
        if !pointer.holding() {
            self.end();
            return None;
        }
        // 押していないときに位置を覚えると、次の移動がドラッグになってしまう
        let from = self.from.as_mut()?;
        let delta = Point::new(at.x - from.x, at.y - from.y);
        *from = at;
        Some(delta)
    }
}

/// 1 ノッチのホイールの量（Win32 の `WHEEL_DELTA`）。
pub const WHEEL_NOTCH: i32 = 120;

/// ホイールの回転。`WM_MOUSEWHEEL` の量をためて、1 ノッチ（`WHEEL_NOTCH`）ごとに 1 段として返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Wheel {
    /// まだ 1 ノッチに満たない量（上が正）。
    rest: i32,
}

impl Wheel {
    /// ホイールが `delta`（上が正）だけ回った。ノッチの数（上が正）を返す。向きが変わったらためた分は捨てる。
    pub fn turn(&mut self, delta: i32) -> i32 {
        if (self.rest > 0 && delta < 0) || (self.rest < 0 && delta > 0) {
            self.rest = 0;
        }
        self.rest = self.rest.saturating_add(delta);
        let notches = self.rest / WHEEL_NOTCH;
        self.rest -= notches * WHEEL_NOTCH;
        notches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotate::{Annotation, Pick, Shape, Tool};
    use crate::image::Image;
    use crate::keys::{Chord, Key};
    use crate::settings::Rgb;

    #[test]
    fn a_new_scrap_is_normal_and_shows_its_image_where_it_was_captured() {
        let scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        assert_eq!(scrap.mode, Mode::Normal);
        assert_eq!(scrap.image_rect(), Rect::new(-100, 50, 200, 250));
    }

    #[test]
    fn a_blank_scrap_has_the_chosen_size_and_is_painted_with_its_background() {
        let white = Background::Color(Rgb(0xFF, 0xFF, 0xFF));
        let scrap = ScrapModel::blank(Point::new(10, 20), Size::new(3, 2), white);
        assert_eq!(scrap.image_rect(), Rect::new(10, 20, 13, 22));
        assert_eq!(scrap.background(), Some(white));
        let image = scrap.blank_image().unwrap();
        assert_eq!(image.size(), Size::new(3, 2));
        assert_eq!(image.pixel(0, 0), [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(image.pixel(2, 1), [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn a_blank_scrap_paints_its_background_in_bgra() {
        let colour = Background::Color(Rgb(0x10, 0x20, 0x30));
        let scrap = ScrapModel::blank(Point::default(), Size::new(1, 1), colour);
        assert_eq!(
            scrap.blank_image().unwrap().pixel(0, 0),
            [0x30, 0x20, 0x10, 0xFF]
        );
    }

    #[test]
    fn a_transparent_blank_scrap_is_painted_with_zero_alpha() {
        let scrap = ScrapModel::blank(Point::default(), Size::new(2, 1), Background::Transparent);
        let image = scrap.blank_image().unwrap();
        assert_eq!(image.pixel(0, 0), [0, 0, 0, 0]);
        assert_eq!(image.pixel(1, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn a_background_is_written_as_rrggbb_or_transparent_and_read_back() {
        assert_eq!(Background::Transparent.to_string(), "transparent");
        assert_eq!(
            "transparent".parse::<Background>(),
            Ok(Background::Transparent)
        );
        assert_eq!(
            "#10a0ff".parse::<Background>(),
            Ok(Background::Color(Rgb(0x10, 0xA0, 0xFF)))
        );
        assert_eq!(
            Background::Color(Rgb(0x10, 0xA0, 0xFF)).to_string(),
            "#10A0FF"
        );
        for wrong in ["Transparent", "clear", "none", ""] {
            assert_eq!(wrong.parse::<Background>(), Err(NotBackground), "{wrong}");
        }
    }

    #[test]
    fn a_blank_scrap_is_transparent_only_with_the_transparent_background() {
        let size = Size::new(4, 4);
        let transparent = ScrapModel::blank(Point::default(), size, Background::Transparent);
        let white = ScrapModel::blank(Point::default(), size, Background::WHITE);
        let captured = ScrapModel::new(Point::default(), size);
        assert!(transparent.is_blank() && transparent.is_transparent_blank());
        assert!(white.is_blank() && !white.is_transparent_blank());
        assert!(!captured.is_blank() && !captured.is_transparent_blank());
    }

    #[test]
    fn only_a_transparent_blank_scrap_with_nothing_written_is_empty() {
        let size = Size::new(4, 4);
        let transparent = ScrapModel::blank(Point::default(), size, Background::Transparent);
        assert!(transparent.is_empty_transparent_blank());
        assert!(
            !ScrapModel::blank(Point::default(), size, Background::WHITE)
                .is_empty_transparent_blank()
        );
        assert!(!ScrapModel::new(Point::default(), size).is_empty_transparent_blank());
        let mut written = transparent.clone();
        written.start_editing(Style::default());
        written.edit_input(EditInput::Press(Point::new(0, 0)));
        written.edit_input(EditInput::Move {
            at: Point::new(3, 3),
            shift: false,
        });
        written.edit_input(EditInput::Leave);
        assert_eq!(written.annotations().items().len(), 1);
        assert!(!written.is_empty_transparent_blank());
    }

    #[test]
    fn a_captured_scrap_has_no_background_of_its_own() {
        let scrap = ScrapModel::new(Point::default(), Size::new(3, 2));
        assert_eq!(scrap.background(), None);
        assert_eq!(scrap.blank_image(), None);
    }

    #[test]
    fn the_hit_alpha_survives_the_window_opacity_and_stays_out_of_sight() {
        let at = |opacity| Appearance {
            opacity,
            ..Appearance::default()
        };
        assert_eq!(at(100).hit_alpha(), 1);
        assert_eq!(at(50).hit_alpha(), 3);
        assert_eq!(at(10).hit_alpha(), 11);
    }

    #[test]
    fn the_window_surrounds_the_image_with_the_border_and_the_shadow() {
        let scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        // 既定値: 枠 1px + 影 8px
        assert_eq!(
            scrap.window_rect(&Appearance::default()),
            Rect::new(-109, 41, 209, 259)
        );
        let plain = Appearance {
            show_border: false,
            shadow: false,
            ..Appearance::default()
        };
        assert_eq!(scrap.window_rect(&plain), Rect::new(-100, 50, 200, 250));
    }

    #[test]
    fn the_appearance_follows_the_settings() {
        use crate::settings::{self, Rgb};
        assert_eq!(
            Appearance::from(ScrapAppearance::default()),
            Appearance::default()
        );
        let custom = ScrapAppearance {
            opacity: 80,
            border: settings::Border {
                visible: true,
                width: 3,
                color: Rgb(0x12, 0x34, 0x56),
            },
            shadow: false,
        };
        assert_eq!(
            Appearance::from(custom),
            Appearance {
                opacity: 80,
                border: Border {
                    width: 3,
                    color: 0x12_34_56,
                },
                show_border: true,
                shadow: false,
            }
        );
        let hidden = ScrapAppearance {
            border: settings::Border {
                visible: false,
                ..settings::Border::default()
            },
            ..ScrapAppearance::default()
        };
        assert_eq!(Appearance::from(hidden).shown_border(), None);
    }

    #[test]
    fn without_overrides_the_scrap_looks_like_the_defaults() {
        let scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        let defaults = Appearance {
            opacity: 80,
            border: Border {
                width: 3,
                color: 0x12_34_56,
            },
            show_border: true,
            shadow: false,
        };
        assert_eq!(scrap.appearance(&defaults), defaults);
    }

    #[test]
    fn overriding_some_of_the_appearance_keeps_the_defaults_for_the_rest() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.appearance_overrides.opacity = Some(50);
        let defaults = Appearance::default();
        assert_eq!(
            scrap.appearance(&defaults),
            Appearance {
                opacity: 50,
                ..defaults
            }
        );
        // 枠の色だけを変えても、太さは既定値のまま
        let thick = Appearance {
            border: Border {
                width: 4,
                color: 0x80_80_80,
            },
            ..defaults
        };
        scrap.appearance_overrides.border = Some(BorderOverride::Color(0xFF_00_00));
        assert_eq!(
            scrap.appearance(&thick).shown_border(),
            Some(Border {
                width: 4,
                color: 0xFF_00_00,
            })
        );
    }

    #[test]
    fn overriding_all_of_the_appearance_ignores_the_defaults() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.appearance_overrides = AppearanceOverrides {
            opacity: Some(30),
            border: Some(BorderOverride::Color(0x00_00_FF)),
            border_width: None,
            shadow: Some(true),
        };
        let plain = Appearance {
            opacity: 100,
            border: Border {
                width: 3,
                color: 0x80_80_80,
            },
            show_border: false,
            shadow: false,
        };
        // 既定値で枠を出さなくても、枠を付けたときの太さは設定の太さ
        assert_eq!(
            scrap.appearance(&plain),
            Appearance {
                opacity: 30,
                border: Border {
                    width: 3,
                    color: 0x00_00_FF,
                },
                show_border: true,
                shadow: true,
            }
        );
        scrap.appearance_overrides.border = Some(BorderOverride::Hidden);
        scrap.appearance_overrides.shadow = Some(false);
        assert_eq!(
            scrap.appearance(&Appearance::default()).shown_border(),
            None
        );
        assert!(!scrap.appearance(&Appearance::default()).shadow);
    }

    #[test]
    fn the_window_follows_the_overridden_border_and_shadow() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        let defaults = Appearance::default();
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-109, 41, 209, 259));
        scrap.appearance_overrides.shadow = Some(false);
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-101, 49, 201, 251));
        scrap.appearance_overrides.border = Some(BorderOverride::Hidden);
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-100, 50, 200, 250));
        // 透明度は大きさに関係しない
        scrap.appearance_overrides.opacity = Some(10);
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-100, 50, 200, 250));
    }

    #[test]
    fn a_thicker_border_grows_the_window_outward_and_keeps_the_image_in_place() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        let defaults = Appearance::default();
        let image = scrap.image_rect();
        scrap.appearance_overrides.border_width = Some(5);
        assert_eq!(
            scrap.appearance(&defaults).shown_border(),
            Some(Border {
                width: 5,
                color: 0x80_80_80
            })
        );
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-113, 37, 213, 263));
        scrap.appearance_overrides.border_width = Some(16);
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-124, 26, 224, 274));
        assert_eq!(scrap.image_rect(), image);
        // 枠を隠しても太さは持っておき、色を付けて出すとその太さ
        scrap.appearance_overrides.border = Some(BorderOverride::Hidden);
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-108, 42, 208, 258));
        scrap.appearance_overrides.border = Some(BorderOverride::Color(0xFF_00_00));
        assert_eq!(
            scrap.appearance(&defaults).shown_border(),
            Some(Border {
                width: 16,
                color: 0xFF_00_00
            })
        );
    }

    #[test]
    fn zooming_steps_by_10_up_to_100_and_by_25_above_and_stops_at_10_and_800() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(100, 100));
        assert_eq!(scrap.zoom(), 100);
        let mut seen = vec![scrap.zoom()];
        while scrap.zoom_out(ZoomAnchor::Center) {
            seen.push(scrap.zoom());
        }
        assert_eq!(seen, [100, 90, 80, 70, 60, 50, 40, 30, 20, 10]);
        assert!(!scrap.zoom_out(ZoomAnchor::Center));
        assert_eq!(scrap.zoom(), 10);

        let mut seen = vec![scrap.zoom()];
        while scrap.zoom_in(ZoomAnchor::Center) {
            seen.push(scrap.zoom());
        }
        let mut expected: Vec<u16> = (1..=10).map(|n| n * 10).collect();
        expected.extend((5..=32).map(|n| n * 25));
        assert_eq!(seen, expected);
        assert_eq!(scrap.zoom(), 800);
        assert!(!scrap.zoom_in(ZoomAnchor::Center));
        assert_eq!(scrap.zoom(), 800);
        // 下げるときも 100% より上は 25% 刻み
        scrap.zoom_out(ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 775);
    }

    #[test]
    fn the_window_is_sized_from_the_zoomed_image() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_zoom(50, ZoomAnchor::At(Point::new(0, 0)));
        assert_eq!(scrap.display_size(), Size::new(150, 100));
        assert_eq!(scrap.image_rect(), Rect::new(0, 0, 150, 100));
        assert_eq!(
            scrap.window_rect(&Appearance::default()),
            Rect::new(-9, -9, 159, 109)
        );
        // 端数は四捨五入し、10% でも 1px は残す
        let mut tiny = ScrapModel::new(Point::new(0, 0), Size::new(3, 15));
        tiny.set_zoom(ZOOM_MIN, ZoomAnchor::At(Point::new(0, 0)));
        assert_eq!(tiny.display_size(), Size::new(1, 2));
    }

    #[test]
    fn keys_zoom_around_the_center_of_the_scrap() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        // 中心は (50, 150)
        assert!(scrap.zoom_out(ZoomAnchor::Center));
        assert_eq!(scrap.image_rect(), Rect::new(-85, 60, 185, 240));
        assert!(scrap.zoom_in(ZoomAnchor::Center));
        assert!(scrap.zoom_in(ZoomAnchor::Center));
        assert_eq!(scrap.zoom(), 125);
        assert_eq!(scrap.image_rect(), Rect::new(-138, 25, 237, 275));
    }

    #[test]
    fn the_wheel_zooms_around_the_cursor_so_the_pixel_under_it_stays_put() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        let cursor = Point::new(40, 20);
        assert!(scrap.zoom_in(ZoomAnchor::At(cursor)));
        assert_eq!(scrap.image_rect(), Rect::new(-10, -5, 365, 245));
        assert!(scrap.zoom_out(ZoomAnchor::At(cursor)));
        assert_eq!(scrap.image_rect(), Rect::new(0, 0, 300, 200));
        // 画像の右下の角で縮めると、右下の角が動かない
        let corner = Point::new(300, 200);
        for _ in 0..5 {
            scrap.zoom_out(ZoomAnchor::At(corner));
        }
        assert_eq!(scrap.zoom(), 50);
        assert_eq!(scrap.image_rect(), Rect::new(150, 100, 300, 200));
    }

    #[test]
    fn resetting_the_zoom_returns_to_actual_pixels_around_the_center() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_zoom(200, ZoomAnchor::At(Point::new(0, 0)));
        assert_eq!(scrap.image_rect(), Rect::new(0, 0, 600, 400));
        assert!(scrap.reset_zoom());
        assert_eq!(scrap.zoom(), ZOOM_ACTUAL);
        assert_eq!(scrap.image_rect(), Rect::new(150, 100, 450, 300));
        assert!(!scrap.reset_zoom());
    }

    #[test]
    fn a_chosen_zoom_is_kept_within_the_range() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        assert!(scrap.set_zoom(5, ZoomAnchor::Center));
        assert_eq!(scrap.zoom(), ZOOM_MIN);
        assert!(scrap.set_zoom(1000, ZoomAnchor::Center));
        assert_eq!(scrap.zoom(), ZOOM_MAX);
        assert!(!scrap.set_zoom(ZOOM_MAX, ZoomAnchor::Center));
    }

    #[test]
    fn keeping_the_apparent_size_rescales_the_zoom_from_150_to_100_percent_dpi() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        assert!(!scrap.set_dpi(144, DpiScaling::KeepApparentSize, ZoomAnchor::Center));
        assert!(scrap.set_dpi(96, DpiScaling::KeepApparentSize, ZoomAnchor::Center));
        // 300px を 144dpi で見た大きさは、96dpi では 200px。倍率は画素が基準なので 67%
        assert_eq!(scrap.zoom(), 67);
        assert_eq!(scrap.display_size(), Size::new(201, 134));
        // 中心 (150, 100) は動かない
        assert_eq!(scrap.image_rect(), Rect::new(50, 33, 251, 167));
    }

    #[test]
    fn keeping_the_apparent_size_rescales_the_zoom_from_100_to_150_percent_dpi() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(96, DpiScaling::KeepApparentSize, ZoomAnchor::Center);
        assert!(scrap.set_dpi(144, DpiScaling::KeepApparentSize, ZoomAnchor::Center));
        assert_eq!(scrap.zoom(), 150);
        assert_eq!(scrap.image_rect(), Rect::new(-75, -50, 375, 250));
    }

    #[test]
    fn keeping_pixels_leaves_the_size_and_the_zoom_across_dpis() {
        for (from, to) in [(144, 96), (96, 144)] {
            let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
            scrap.set_dpi(from, DpiScaling::KeepPixels, ZoomAnchor::Center);
            assert!(!scrap.set_dpi(to, DpiScaling::KeepPixels, ZoomAnchor::Center));
            assert_eq!(scrap.zoom(), 100, "{from} -> {to}");
            assert_eq!(
                scrap.image_rect(),
                Rect::new(0, 0, 300, 200),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn going_back_to_the_first_dpi_restores_the_zoom_even_when_it_was_rounded() {
        let keep = DpiScaling::KeepApparentSize;
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        scrap.set_dpi(96, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 67);
        // 67% から計算し直すと 100.5% になるが、元の倍率から計算するので 100% に戻る
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 100);
        scrap.set_dpi(120, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 83);
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 100);

        // 上限で止めた倍率も、戻れば元の倍率になる
        scrap.set_zoom(800, ZoomAnchor::Center);
        scrap.set_dpi(96, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 533);
        scrap.set_dpi(192, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), ZOOM_MAX);
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 800);
    }

    #[test]
    fn zooming_after_a_dpi_change_becomes_the_new_base() {
        let keep = DpiScaling::KeepApparentSize;
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        scrap.set_dpi(96, keep, ZoomAnchor::Center);
        assert!(scrap.zoom_in(ZoomAnchor::Center));
        assert_eq!(scrap.zoom(), 70);
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        assert_eq!(scrap.zoom(), 105);
    }

    #[test]
    fn a_dpi_change_while_dragging_scales_around_the_cursor() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(96, DpiScaling::KeepApparentSize, ZoomAnchor::Center);
        let cursor = Point::new(30, 20);
        scrap.set_dpi(192, DpiScaling::KeepApparentSize, ZoomAnchor::At(cursor));
        // ドラッグ中はカーソルの下の画素がカーソルについてくる
        assert_eq!(scrap.image_rect(), Rect::new(-30, -20, 570, 380));
    }

    #[test]
    fn the_wheel_counts_whole_notches_and_keeps_the_rest_of_fine_turns() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.turn(120), 1);
        assert_eq!(wheel.turn(-240), -2);
        // タッチパッドなどの細かい量はためて、1 ノッチになったら返す
        assert_eq!(wheel.turn(60), 0);
        assert_eq!(wheel.turn(30), 0);
        assert_eq!(wheel.turn(40), 1);
        // 向きが変わったらためた分を捨てる
        assert_eq!(wheel.turn(100), 0);
        assert_eq!(wheel.turn(-100), 0);
        assert_eq!(wheel.turn(-20), -1);
    }

    /// (x, y) の画素の B に x、G に y を入れた画像（切り出した位置が分かるように）。
    fn numbered(size: Size) -> Image {
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend([x as u8, y as u8, 0, 0xFF]);
            }
        }
        Image::from_bgra(size, pixels).unwrap()
    }

    fn trimming(scrap: &ScrapModel) -> bool {
        scrap.is_trimming()
    }

    #[test]
    fn dragging_while_trimming_selects_the_pixels_under_both_ends() {
        let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
        assert!(scrap.start_trim());
        assert!(trimming(&scrap));
        assert_eq!(scrap.trim_selection(), None);
        // 押す前のホバーでは何も選ばない
        scrap.trim_input(TrimInput::Hover(Point::new(120, 60)));
        assert_eq!(scrap.trim_selection(), None);
        scrap.trim_input(TrimInput::Press(Point::new(130, 70)));
        assert!(scrap.trim_input(TrimInput::Move(Point::new(110, 90))));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(110, 70, 131, 91)));
        scrap.trim_input(TrimInput::Release);
        // 範囲の外を押すと選び直す。押した画素へ戻ると 1 画素の範囲になる
        scrap.trim_input(TrimInput::Press(Point::new(230, 170)));
        scrap.trim_input(TrimInput::Move(Point::new(240, 170)));
        scrap.trim_input(TrimInput::Move(Point::new(230, 170)));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(230, 170, 231, 171)));
        scrap.trim_input(TrimInput::Move(Point::new(210, 190)));
        scrap.trim_input(TrimInput::Release);
        // 離したあとのホバーでは範囲は変わらない
        scrap.trim_input(TrimInput::Hover(Point::new(300, 200)));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(210, 170, 231, 191)));
        // 画像の外へはみ出したドラッグは画像の端で止まる
        scrap.trim_input(TrimInput::Press(Point::new(390, 240)));
        scrap.trim_input(TrimInput::Move(Point::new(1000, 1000)));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(390, 240, 400, 250)));
        // 選んでいる間は大きさも位置も変わらない
        assert_eq!(scrap.image_rect(), Rect::new(100, 50, 400, 250));
    }

    /// 200% の倍率で、画素 (10, 20)〜(29, 39) を選んだトリムの最中のスクラップ。画面では (0, 0) から。
    fn zoomed_trim() -> ScrapModel {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_zoom(200, ZoomAnchor::At(Point::new(0, 0)));
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(20, 40)));
        scrap.trim_input(TrimInput::Move(Point::new(59, 79)));
        scrap.trim_input(TrimInput::Release);
        assert_eq!(scrap.trim_selection(), Some(Rect::new(20, 40, 60, 80)));
        scrap
    }

    /// 選んでいる範囲（表示している画像の画素）。
    fn trim_pixels(scrap: &ScrapModel) -> Option<Rect> {
        match scrap.mode {
            Mode::Trimming(state) => state.selection,
            _ => None,
        }
    }

    #[test]
    fn dragging_a_handle_resizes_the_trim_range_by_whole_image_pixels() {
        let mut scrap = zoomed_trim();
        // 右下のつまみ（画面の (60, 80)）を 21 画面ピクセル右、7 下へ引く
        scrap.trim_input(TrimInput::Press(Point::new(60, 80)));
        scrap.trim_input(TrimInput::Move(Point::new(81, 87)));
        scrap.trim_input(TrimInput::Release);
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(10, 20, 40, 43)));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(20, 40, 80, 86)));
        // 左のつまみ（画面の (20, 63)）は画像の端より外へは広がらない
        scrap.trim_input(TrimInput::Press(Point::new(20, 63)));
        scrap.trim_input(TrimInput::Move(Point::new(-500, 63)));
        scrap.trim_input(TrimInput::Release);
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(0, 20, 40, 43)));
    }

    #[test]
    fn dragging_inside_the_trim_range_moves_it_and_it_stops_at_the_edge_of_the_image() {
        let mut scrap = zoomed_trim();
        scrap.trim_input(TrimInput::Press(Point::new(40, 60)));
        scrap.trim_input(TrimInput::Move(Point::new(51, 56)));
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(15, 18, 35, 38)));
        scrap.trim_input(TrimInput::Move(Point::new(1000, -1000)));
        scrap.trim_input(TrimInput::Release);
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(280, 0, 300, 20)));
    }

    #[test]
    fn arrows_move_the_trim_range_and_ctrl_arrows_resize_it_by_image_pixels() {
        let mut scrap = zoomed_trim();
        let arrow = |direction, shift| TrimInput::Arrow { direction, shift };
        let resize = |direction, shift| TrimInput::ResizeArrow { direction, shift };
        assert!(scrap.trim_input(arrow(Direction::Right, false)));
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(11, 20, 31, 40)));
        scrap.trim_input(arrow(Direction::Up, true));
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(11, 10, 31, 30)));
        // 画像の端で止まる
        scrap.trim_input(arrow(Direction::Up, true));
        scrap.trim_input(arrow(Direction::Up, true));
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(11, 0, 31, 20)));
        scrap.trim_input(resize(Direction::Right, true));
        scrap.trim_input(resize(Direction::Down, false));
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(11, 0, 41, 21)));
        // 1 画素より小さくはしない
        for _ in 0..5 {
            scrap.trim_input(resize(Direction::Left, true));
        }
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(11, 0, 12, 21)));
    }

    #[test]
    fn arrows_do_nothing_before_a_trim_range_is_selected() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.start_trim();
        let input = TrimInput::Arrow {
            direction: Direction::Left,
            shift: false,
        };
        assert!(!scrap.trim_input(input));
        assert_eq!(scrap.trim_selection(), None);
    }

    #[test]
    fn a_double_click_inside_the_trim_range_confirms_it_and_outside_selects_again() {
        let mut scrap = zoomed_trim();
        // 外: そこを始点に選び直す（まだ動かしていないので前の範囲は残る）
        assert!(scrap.trim_input(TrimInput::DoubleClick(Point::new(200, 200))));
        scrap.trim_input(TrimInput::Move(Point::new(219, 219)));
        scrap.trim_input(TrimInput::Release);
        assert_eq!(trim_pixels(&scrap), Some(Rect::new(100, 100, 110, 110)));
        // 中: 確定する
        assert!(scrap.trim_input(TrimInput::DoubleClick(Point::new(205, 205))));
        assert!(!trimming(&scrap));
        assert_eq!(scrap.shown_size(), Size::new(10, 10));
    }

    #[test]
    fn the_trim_view_labels_the_cursor_with_pixels_of_the_original_image() {
        // 元の画像の (100, 50)〜(199, 149) にトリムしてから 200% にしたスクラップ
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(100, 50)));
        scrap.trim_input(TrimInput::Move(Point::new(199, 149)));
        scrap.trim_input(TrimInput::Confirm);
        scrap.set_zoom(200, ZoomAnchor::At(Point::new(100, 50)));
        assert_eq!(scrap.image_rect(), Rect::new(100, 50, 300, 250));

        scrap.start_trim();
        // 選ぶ前でも、カーソルの十字線・拡大鏡・座標を出す。範囲とつまみはない
        scrap.trim_input(TrimInput::Hover(Point::new(141, 101)));
        let view = scrap.trim_view().unwrap();
        assert_eq!(view.selection, None);
        assert!(view.handles.is_empty());
        assert_eq!(view.crosshair, Some(Point::new(141, 101)));
        assert_eq!(view.magnifier, Some(Point::new(20, 25)));
        assert_eq!(view.cursor_label.as_deref(), Some("120, 75"));

        // 表示している画素 (10, 20)〜(29, 39) を選ぶ。選び直している間はつまみを出さない
        scrap.trim_input(TrimInput::Press(Point::new(120, 90)));
        scrap.trim_input(TrimInput::Move(Point::new(159, 129)));
        let view = scrap.trim_view().unwrap();
        assert!(view.handles.is_empty());
        assert_eq!(view.cursor_label.as_deref(), Some("129, 89   20 × 20"));
        scrap.trim_input(TrimInput::Release);
        let view = scrap.trim_view().unwrap();
        assert_eq!(view.selection, Some(Rect::new(120, 90, 160, 130)));
        assert_eq!(view.handles.len(), 8);
        assert!(view.handles.contains(&Rect::new(157, 127, 164, 134)));

        // 画像の外では十字線・拡大鏡・座標を出さない
        scrap.trim_input(TrimInput::Hover(Point::new(99, 120)));
        let view = scrap.trim_view().unwrap();
        assert_eq!(view.selection, Some(Rect::new(120, 90, 160, 130)));
        assert_eq!(view.crosshair, None);
        assert_eq!(view.magnifier, None);
        assert_eq!(view.cursor_label, None);

        scrap.trim_input(TrimInput::Cancel);
        assert_eq!(scrap.trim_view(), None);
    }

    #[test]
    fn the_cursor_shape_while_trimming_follows_what_is_under_the_cursor() {
        let mut scrap = zoomed_trim();
        let shape_at = |scrap: &mut ScrapModel, at| {
            scrap.trim_input(TrimInput::Hover(at));
            scrap.trim_cursor_shape()
        };
        assert_eq!(shape_at(&mut scrap, Point::new(5, 5)), CursorShape::Cross);
        assert_eq!(shape_at(&mut scrap, Point::new(40, 60)), CursorShape::Move);
        assert_eq!(
            shape_at(&mut scrap, Point::new(20, 40)),
            CursorShape::ResizeNwse
        );
        assert_eq!(
            shape_at(&mut scrap, Point::new(60, 60)),
            CursorShape::ResizeWe
        );
        // つかんでいる間は、範囲の外へ出てもつかんだものの形のまま
        scrap.trim_input(TrimInput::Press(Point::new(40, 80)));
        scrap.trim_input(TrimInput::Move(Point::new(200, 150)));
        assert_eq!(scrap.trim_cursor_shape(), CursorShape::ResizeNs);
        scrap.trim_input(TrimInput::Cancel);
        assert_eq!(scrap.trim_cursor_shape(), CursorShape::Cross);
    }

    #[test]
    fn a_click_without_moving_keeps_the_selection_so_a_double_click_can_confirm_it() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(10, 10)));
        scrap.trim_input(TrimInput::Move(Point::new(19, 29)));
        scrap.trim_input(TrimInput::Release);
        // ダブルクリックの 1 回目のクリック（押した画素の中で揺れても範囲は消えない）
        scrap.trim_input(TrimInput::Press(Point::new(15, 15)));
        scrap.trim_input(TrimInput::Move(Point::new(15, 15)));
        scrap.trim_input(TrimInput::Release);
        assert_eq!(scrap.trim_selection(), Some(Rect::new(10, 10, 20, 30)));
        assert!(scrap.trim_input(TrimInput::Confirm));
        assert!(!trimming(&scrap));
        assert_eq!(scrap.image_rect(), Rect::new(10, 10, 20, 30));
    }

    #[test]
    fn confirming_keeps_the_remaining_part_where_it_was_on_the_screen() {
        let source = numbered(Size::new(300, 200));
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(-90, 70)));
        scrap.trim_input(TrimInput::Move(Point::new(-41, 109)));
        scrap.trim_input(TrimInput::Release);
        assert!(scrap.trim_input(TrimInput::Confirm));
        assert_eq!(scrap.mode, Mode::Normal);
        assert!(scrap.is_trimmed());
        assert_eq!(scrap.image_rect(), Rect::new(-90, 70, -40, 110));
        assert_eq!(scrap.shown_size(), Size::new(50, 40));
        // 元の画像の大きさは残る
        assert_eq!(scrap.image_size, Size::new(300, 200));
        let shown = scrap.shown_image(&source);
        assert_eq!(shown.size(), Size::new(50, 40));
        assert_eq!(shown.pixel(0, 0), [10, 20, 0, 0xFF]);
        assert_eq!(shown.pixel(49, 39), [59, 59, 0, 0xFF]);
    }

    #[test]
    fn trimming_a_zoomed_scrap_selects_the_image_pixels_under_the_drag() {
        let source = numbered(Size::new(300, 200));
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_zoom(200, ZoomAnchor::At(Point::new(0, 0)));
        scrap.start_trim();
        // 表示の (21, 41) は画素 (10, 20)、(60, 81) は画素 (30, 40)
        scrap.trim_input(TrimInput::Press(Point::new(21, 41)));
        scrap.trim_input(TrimInput::Move(Point::new(60, 81)));
        scrap.trim_input(TrimInput::Release);
        // 選んだ範囲は画素の境目に合わせて出す
        assert_eq!(scrap.trim_selection(), Some(Rect::new(20, 40, 62, 82)));
        scrap.trim_input(TrimInput::Confirm);
        assert_eq!(scrap.shown_size(), Size::new(21, 21));
        assert_eq!(scrap.zoom(), 200);
        assert_eq!(scrap.image_rect(), Rect::new(20, 40, 62, 82));
        assert_eq!(scrap.shown_image(&source).pixel(0, 0), [10, 20, 0, 0xFF]);
    }

    #[test]
    fn trimming_again_selects_within_the_part_already_shown() {
        let source = numbered(Size::new(300, 200));
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        for (from, to) in [((10, 10), (109, 109)), ((15, 20), (24, 29))] {
            scrap.start_trim();
            scrap.trim_input(TrimInput::Press(Point::new(from.0, from.1)));
            scrap.trim_input(TrimInput::Move(Point::new(to.0, to.1)));
            scrap.trim_input(TrimInput::Release);
            scrap.trim_input(TrimInput::Confirm);
        }
        assert_eq!(scrap.image_rect(), Rect::new(15, 20, 25, 30));
        assert_eq!(scrap.shown_image(&source).pixel(0, 0), [15, 20, 0, 0xFF]);
    }

    #[test]
    fn escape_cancels_the_trim_and_leaves_the_scrap_as_it_was() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        let before = scrap.clone();
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(10, 10)));
        scrap.trim_input(TrimInput::Move(Point::new(50, 50)));
        assert!(scrap.trim_input(TrimInput::Cancel));
        assert_eq!(scrap, before);
        assert!(!scrap.is_trimmed());
    }

    #[test]
    fn confirming_without_a_selection_keeps_trimming() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.start_trim();
        assert!(!scrap.trim_input(TrimInput::Confirm));
        assert!(trimming(&scrap));
        // トリムの最中でなければ入力は無視する
        let mut normal = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        assert!(!normal.trim_input(TrimInput::Press(Point::new(1, 1))));
        assert!(!normal.trim_input(TrimInput::Confirm));
        assert_eq!(normal.mode, Mode::Normal);
    }

    #[test]
    fn untrimming_returns_the_whole_image_to_where_it_was() {
        let source = numbered(Size::new(300, 200));
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        scrap.set_zoom(150, ZoomAnchor::At(Point::new(-100, 50)));
        let before = scrap.clone();
        assert!(!scrap.untrim());
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(-40, 80)));
        scrap.trim_input(TrimInput::Move(Point::new(0, 120)));
        scrap.trim_input(TrimInput::Release);
        scrap.trim_input(TrimInput::Confirm);
        assert!(scrap.is_trimmed());
        assert!(scrap.untrim());
        assert_eq!(scrap, before);
        assert_eq!(scrap.shown_image(&source), source);
        assert!(!scrap.untrim());
    }

    #[test]
    fn folding_from_keys_makes_a_64px_square_on_the_center_of_the_scrap() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        // 中心は (50, 150)
        assert!(scrap.enter_compact(ZoomAnchor::Center));
        assert!(scrap.is_compact());
        assert_eq!(scrap.image_rect(), Rect::new(18, 118, 82, 182));
        assert_eq!(
            scrap.window_rect(&Appearance::default()),
            Rect::new(9, 109, 91, 191)
        );
        // 畳んだまま畳んでも変わらない
        assert!(!scrap.enter_compact(ZoomAnchor::Center));
        assert_eq!(scrap.image_rect(), Rect::new(18, 118, 82, 182));
    }

    #[test]
    fn the_square_shows_the_part_of_the_image_around_where_it_was_folded() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        assert_eq!(scrap.compact_view(), None);
        // カーソルの下は画像の (110, 60)。そのまわりの 64 画素四方
        scrap.enter_compact(ZoomAnchor::At(Point::new(10, 110)));
        assert_eq!(scrap.compact_view(), Some(Rect::new(78, 28, 142, 92)));
        // 畳んだまま動かしても、映す部分は変わらない
        scrap.position = Point::new(500, 500);
        assert_eq!(scrap.compact_view(), Some(Rect::new(78, 28, 142, 92)));
    }

    #[test]
    fn the_part_shown_in_the_square_stays_inside_the_image() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.enter_compact(ZoomAnchor::At(Point::new(2, 199)));
        assert_eq!(scrap.compact_view(), Some(Rect::new(0, 136, 64, 200)));
    }

    #[test]
    fn the_part_shown_in_the_square_follows_the_zoom() {
        // 200% なら 32 画素四方を映す
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_zoom(200, ZoomAnchor::At(Point::new(0, 0)));
        scrap.enter_compact(ZoomAnchor::At(Point::new(200, 200)));
        assert_eq!(scrap.compact_view(), Some(Rect::new(84, 84, 116, 116)));
        // 縮小していて正方形が画像より大きければ、画像の短い辺の正方形にする
        let mut small = ScrapModel::new(Point::new(0, 0), Size::new(100, 40));
        small.set_zoom(50, ZoomAnchor::At(Point::new(0, 0)));
        small.enter_compact(ZoomAnchor::Center);
        assert_eq!(small.compact_view(), Some(Rect::new(30, 0, 70, 40)));
    }

    #[test]
    fn folding_with_the_mouse_centers_the_square_on_the_cursor() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        assert!(scrap.enter_compact(ZoomAnchor::At(Point::new(-90, 60))));
        assert_eq!(scrap.image_rect(), Rect::new(-122, 28, -58, 92));
    }

    #[test]
    fn unfolding_without_moving_returns_to_where_it_was_before_folding() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        assert!(!scrap.leave_compact());
        scrap.enter_compact(ZoomAnchor::At(Point::new(-90, 60)));
        assert!(scrap.leave_compact());
        assert!(!scrap.is_compact());
        assert_eq!(scrap.image_rect(), Rect::new(-100, 50, 200, 250));
        assert!(!scrap.leave_compact());
    }

    #[test]
    fn unfolding_shifts_the_scrap_by_how_far_the_square_was_moved() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        scrap.enter_compact(ZoomAnchor::Center);
        scrap.position = Point::new(scrap.position.x + 30, scrap.position.y - 500);
        scrap.leave_compact();
        assert_eq!(scrap.image_rect(), Rect::new(-70, -450, 230, -250));
    }

    #[test]
    fn a_folded_scrap_rescales_its_zoom_for_when_it_unfolds_on_another_dpi() {
        let keep = DpiScaling::KeepApparentSize;
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        scrap.enter_compact(ZoomAnchor::Center);
        scrap.set_dpi(96, keep, ZoomAnchor::Center);
        // 倍率は計算し直してあり、広げると移動先の DPI での見かけの大きさで出る
        assert_eq!(scrap.zoom(), 67);
        scrap.leave_compact();
        assert_eq!(scrap.image_rect().size(), Size::new(201, 134));
    }

    #[test]
    fn resizing_the_square_for_a_dpi_does_not_count_as_moving_it() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(96, DpiScaling::KeepPixels, ZoomAnchor::Center);
        scrap.enter_compact(ZoomAnchor::Center);
        scrap.set_dpi(144, DpiScaling::KeepPixels, ZoomAnchor::Center);
        scrap.leave_compact();
        assert_eq!(scrap.image_rect(), Rect::new(0, 0, 300, 200));
    }

    #[test]
    fn the_compact_square_is_64px_at_96_dpi_and_scales_with_the_dpi() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(144, DpiScaling::KeepPixels, ZoomAnchor::Center);
        // 中心は (150, 100)。144 DPI では 64px 相当は 96px
        scrap.enter_compact(ZoomAnchor::Center);
        assert_eq!(scrap.image_rect(), Rect::new(102, 52, 198, 148));
        let mut at_96 = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        at_96.set_dpi(96, DpiScaling::KeepPixels, ZoomAnchor::Center);
        at_96.enter_compact(ZoomAnchor::Center);
        assert_eq!(at_96.image_rect(), Rect::new(118, 68, 182, 132));
    }

    #[test]
    fn a_folded_scrap_resizes_its_square_around_the_anchor_on_another_dpi() {
        for scaling in [DpiScaling::KeepPixels, DpiScaling::KeepApparentSize] {
            let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
            scrap.set_dpi(96, scaling, ZoomAnchor::Center);
            scrap.enter_compact(ZoomAnchor::Center);
            assert_eq!(scrap.image_rect(), Rect::new(118, 68, 182, 132));
            // 正方形の中心 (150, 100) を残して 96px にする
            assert!(
                scrap.set_dpi(144, scaling, ZoomAnchor::Center),
                "{scaling:?}"
            );
            assert_eq!(
                scrap.image_rect(),
                Rect::new(102, 52, 198, 148),
                "{scaling:?}"
            );
            // ドラッグ中はカーソルの下が動かない
            assert!(scrap.set_dpi(96, scaling, ZoomAnchor::At(Point::new(102, 52))));
            assert_eq!(
                scrap.image_rect(),
                Rect::new(102, 52, 166, 116),
                "{scaling:?}"
            );
        }
    }

    #[test]
    fn a_zoomed_scrap_keeps_its_zoom_through_folding_and_unfolding() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_zoom(200, ZoomAnchor::At(Point::new(0, 0)));
        // 中心は表示の大きさ（600x400）の中心
        scrap.enter_compact(ZoomAnchor::Center);
        assert_eq!(scrap.image_rect(), Rect::new(268, 168, 332, 232));
        assert_eq!(scrap.zoom(), 200);
        scrap.leave_compact();
        assert_eq!(scrap.zoom(), 200);
        assert_eq!(scrap.image_rect(), Rect::new(0, 0, 600, 400));
    }

    /// 左ボタンを押していて、ウィンドウがマウスを捕まえている。
    const HOLDING: Pointer = Pointer {
        left_down: true,
        captured: true,
    };

    #[test]
    fn hovering_without_pressing_never_moves_the_scrap() {
        let mut drag = Drag::default();
        assert_eq!(drag.move_to(Point::new(10, 10), HOLDING), None);
        assert_eq!(drag.move_to(Point::new(15, 12), HOLDING), None);
        assert!(!drag.is_active());
    }

    #[test]
    fn a_drag_reports_each_move_from_the_previous_position_until_it_ends() {
        let mut drag = Drag::default();
        drag.start(Point::new(10, 10));
        assert_eq!(
            drag.move_to(Point::new(15, 8), HOLDING),
            Some(Point::new(5, -2))
        );
        assert_eq!(
            drag.move_to(Point::new(14, 8), HOLDING),
            Some(Point::new(-1, 0))
        );
        drag.end();
        assert_eq!(drag.move_to(Point::new(20, 20), HOLDING), None);
        assert_eq!(drag.move_to(Point::new(25, 20), HOLDING), None);
    }

    #[test]
    fn a_drag_ends_when_a_move_arrives_without_the_button_or_the_capture() {
        for pointer in [
            Pointer {
                left_down: false,
                captured: true,
            },
            Pointer {
                left_down: true,
                captured: false,
            },
        ] {
            let mut drag = Drag::default();
            drag.start(Point::new(10, 10));
            // 離したことを取りこぼしても、ホバーでスクラップを動かさない
            assert_eq!(drag.move_to(Point::new(15, 8), pointer), None);
            assert!(!drag.is_active());
            assert_eq!(drag.move_to(Point::new(20, 8), HOLDING), None);
        }
    }

    #[test]
    fn the_capture_is_stray_only_when_it_outlives_the_button() {
        let stray = |left_down, captured| {
            Pointer {
                left_down,
                captured,
            }
            .stray_capture()
        };
        assert!(stray(false, true));
        assert!(!stray(true, true));
        assert!(!stray(true, false));
        assert!(!stray(false, false));
    }

    #[test]
    fn a_trim_move_without_the_button_or_the_capture_releases_the_press() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(100, 100));
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(10, 10)));
        let lost = Pointer {
            left_down: false,
            captured: true,
        };
        assert_eq!(
            TrimInput::pointer_move(Point::new(30, 30), lost),
            TrimInput::Hover(Point::new(30, 30))
        );
        assert_eq!(
            TrimInput::pointer_move(Point::new(30, 30), HOLDING),
            TrimInput::Move(Point::new(30, 30))
        );
        scrap.trim_input(TrimInput::pointer_move(Point::new(30, 30), lost));
        // 押していないことになったので、そのあとの移動では範囲を選ばない
        scrap.trim_input(TrimInput::Move(Point::new(40, 40)));
        assert_eq!(scrap.trim_selection(), None);
    }

    #[test]
    fn a_dpi_change_anchors_at_the_cursor_only_while_dragging() {
        let mut drag = Drag::default();
        let cursor = Point::new(30, 40);
        assert_eq!(drag.zoom_anchor(cursor), ZoomAnchor::Center);
        drag.start(Point::new(10, 10));
        assert_eq!(drag.zoom_anchor(cursor), ZoomAnchor::At(cursor));
        drag.end();
        assert_eq!(drag.zoom_anchor(cursor), ZoomAnchor::Center);
    }

    #[test]
    fn moving_a_scrap_keeps_its_content_but_zooming_or_folding_does_not() {
        let before = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        let mut moved = before.clone();
        moved.position = Point::new(50, 60);
        assert!(moved.same_content(&before));
        let mut zoomed = before.clone();
        zoomed.zoom_in(ZoomAnchor::Center);
        assert!(!zoomed.same_content(&before));
        let mut folded = before.clone();
        folded.enter_compact(ZoomAnchor::Center);
        assert!(!folded.same_content(&before));
        let mut restyled = before.clone();
        restyled.appearance_overrides.shadow = Some(false);
        assert!(!restyled.same_content(&before));
        let mut trimming = before.clone();
        trimming.start_trim();
        assert!(!trimming.same_content(&before));
    }

    #[test]
    fn a_folded_scrap_on_another_dpi_needs_a_repaint_for_its_new_square() {
        let mut before = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        before.set_dpi(96, DpiScaling::KeepPixels, ZoomAnchor::Center);
        before.enter_compact(ZoomAnchor::Center);
        let mut after = before.clone();
        after.set_dpi(144, DpiScaling::KeepPixels, ZoomAnchor::Center);
        assert!(!after.same_content(&before));
    }

    #[test]
    fn a_scrap_is_trimming_from_start_until_it_confirms_or_cancels() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(100, 100));
        assert!(!scrap.is_trimming());
        scrap.start_trim();
        assert!(scrap.is_trimming());
        scrap.trim_input(TrimInput::Cancel);
        assert!(!scrap.is_trimming());
    }

    /// (100, 50) の 300x200 を、元の画像の (10, 20) から 50x40 にトリムし、200% にしたスクラップ。
    fn trimmed_and_zoomed() -> ScrapModel {
        let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(Point::new(110, 70)));
        scrap.trim_input(TrimInput::Move(Point::new(159, 109)));
        scrap.trim_input(TrimInput::Release);
        scrap.trim_input(TrimInput::Confirm);
        scrap.set_zoom(200, ZoomAnchor::Center);
        scrap
    }

    fn rect(from: (i32, i32), to: (i32, i32)) -> Shape {
        Shape::Rect {
            from: Point::new(from.0, from.1),
            to: Point::new(to.0, to.1),
        }
    }

    #[test]
    fn dragging_while_editing_draws_a_rect_in_the_original_image_pixels_with_the_style() {
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        assert!(scrap.start_editing(Style::default()));
        assert!(scrap.is_editing());
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 7)));
        assert_eq!(
            scrap.edit_input(EditInput::Move {
                at: Point::new(o.x + 41, o.y + 21),
                shift: false
            }),
            vec![EditOutput::Redraw]
        );
        // 描きかけも描く
        assert_eq!(scrap.annotations_to_draw().len(), 1);
        assert!(scrap.annotations().is_empty());
        assert_eq!(
            scrap.edit_input(EditInput::Release),
            vec![EditOutput::Redraw]
        );
        // 表示している画素 (4, 3)〜(20, 10) は、元の画像では (14, 23)〜(30, 30)
        assert_eq!(
            scrap.annotations().items(),
            &[Annotation {
                shape: rect((14, 23), (30, 30)),
                style: Style {
                    color: Rgb(0xE5, 0x39, 0x35),
                    width: 3,
                },
            }]
        );
        assert_eq!(scrap.annotations_to_draw(), scrap.annotations().items());
        // 画像の外へはみ出したドラッグは画像の端の画素で止まる
        scrap.edit_input(EditInput::Press(Point::new(o.x - 50, o.y - 50)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 1000, o.y + 1000),
            shift: false,
        });
        scrap.edit_input(EditInput::Release);
        assert_eq!(
            scrap.annotations().items()[1].shape,
            rect((10, 20), (59, 59))
        );
        assert!(scrap.is_editing());
    }

    #[test]
    fn a_double_click_on_a_text_types_it_again_in_place_of_the_original() {
        use crate::annotate::Font;
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        let at = Point::new(o.x + 8, o.y + 7);
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Pick(Pick::Tool(Tool::Text)));
        assert_eq!(
            scrap.edit_input(EditInput::Press(at)),
            vec![EditOutput::TypingStarted, EditOutput::Redraw]
        );
        scrap.edit_input(EditInput::Char('a'));
        let typed = |text: &str| Shape::Text {
            at: Point::new(14, 23),
            text: text.to_owned(),
            font: Font::default(),
        };
        // 打っている文字は描きかけと別に描く（日本語入力の変換中の文字と一緒に）
        assert_eq!(scrap.typing_annotation().map(|a| a.shape), Some(typed("a")));
        assert_eq!(scrap.annotations_to_draw(), vec![]);
        scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x1B))));
        assert!(scrap.is_editing());
        assert_eq!(scrap.typing_annotation(), None);
        assert_eq!(scrap.annotations_to_draw().len(), 1);
        // 打ち直す間は、元の書き込みを描かない
        scrap.edit_input(EditInput::DoubleClick(Point::new(o.x + 10, o.y + 9)));
        assert_eq!(scrap.annotations_to_draw(), vec![]);
        scrap.edit_input(EditInput::Char('b'));
        assert_eq!(
            scrap.typing_annotation().map(|a| a.shape),
            Some(typed("ab"))
        );
        scrap.edit_input(EditInput::Leave);
        assert!(!scrap.is_editing());
        assert_eq!(scrap.annotations_to_draw()[0].shape, typed("ab"));
    }

    #[test]
    fn a_drag_that_ends_on_the_pixel_it_started_draws_nothing() {
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 8)));
        // 200% なので 1px 動いても同じ画素
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 9, o.y + 9),
            shift: false,
        });
        assert_eq!(scrap.annotations_to_draw(), vec![]);
        assert_eq!(scrap.edit_input(EditInput::Release), vec![]);
        // 離れてから戻ってきても同じ
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 8)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 30, o.y + 30),
            shift: false,
        });
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 8, o.y + 8),
            shift: false,
        });
        scrap.edit_input(EditInput::Release);
        assert!(scrap.annotations().is_empty());
        // 押していないホバーでは何も描かない
        assert_eq!(
            scrap.edit_input(EditInput::Move {
                at: Point::new(o.x + 30, o.y + 30),
                shift: false
            }),
            vec![]
        );
        assert!(scrap.annotations_to_draw().is_empty());
    }

    #[test]
    fn the_hit_slop_is_4px_at_96_dpi_and_grows_with_the_monitor_dpi() {
        // 太さ 2（半分は 1）の横線から 7 画素離れた所。96 DPI では当たりは 1 + 4 で外れ、144 DPI では 1 + 6 で当たる
        for (dpi, hit) in [(96, false), (144, true)] {
            let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
            scrap.set_dpi(dpi, DpiScaling::KeepApparentSize, ZoomAnchor::Center);
            let o = scrap.image_rect().origin();
            let at = |x: i32, y: i32| Point::new(o.x + x, o.y + y);
            scrap.start_editing(Style {
                color: Rgb(0, 0, 0),
                width: 2,
            });
            scrap.edit_input(EditInput::Pick(Pick::Tool(Tool::Line)));
            scrap.edit_input(EditInput::Press(at(10, 50)));
            scrap.edit_input(EditInput::Move {
                at: at(100, 50),
                shift: false,
            });
            scrap.edit_input(EditInput::Release);
            scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x56))));
            scrap.edit_input(EditInput::Press(at(50, 57)));
            scrap.edit_input(EditInput::Release);
            assert_eq!(!scrap.selected_annotations().is_empty(), hit, "{dpi} dpi");
        }
    }

    #[test]
    fn after_leaving_and_coming_back_the_select_tool_picks_moves_recolours_and_deletes() {
        let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
        scrap.set_zoom(50, ZoomAnchor::Center);
        let o = scrap.image_rect().origin();
        let at = |x: i32, y: i32| Point::new(o.x + x, o.y + y);
        scrap.start_editing(Style::default());
        // 50% なので、元の画像では (20, 100)〜(200, 100) の線
        scrap.edit_input(EditInput::Pick(Pick::Tool(Tool::Line)));
        scrap.edit_input(EditInput::Press(at(10, 50)));
        scrap.edit_input(EditInput::Move {
            at: at(100, 50),
            shift: false,
        });
        scrap.edit_input(EditInput::Release);
        scrap.edit_input(EditInput::Leave);
        assert!(!scrap.is_editing());
        assert!(scrap.selected_annotations().is_empty());

        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x56))));
        // 画面で 6px 離れた所（元の画像では 12 画素）は当たらない
        scrap.edit_input(EditInput::Press(at(50, 56)));
        scrap.edit_input(EditInput::Release);
        assert!(scrap.selected_annotations().is_empty());
        // 画面で 4px 離れた所（元の画像では 8 画素）は、太さの半分を超えても当たる（当たりの広さは画面の画素で決める）
        scrap.edit_input(EditInput::Press(at(50, 54)));
        scrap.edit_input(EditInput::Release);
        let selected = scrap.selected_annotations().first().copied().cloned();
        assert_eq!(selected.as_ref(), scrap.annotations().items().first());
        // 矢印キーは元の画像の画素で動かす
        scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x28))));
        assert_eq!(
            scrap.annotations().items()[0].shape,
            Shape::Line {
                from: Point::new(20, 101),
                to: Point::new(200, 101),
            }
        );
        let black = Rgb(0x21, 0x21, 0x21);
        scrap.edit_input(EditInput::Pick(Pick::Color(black)));
        assert_eq!(scrap.annotations().items()[0].style.color, black);
        assert_eq!(scrap.edit_state().map(|s| s.style.color), Some(black));
        scrap.edit_input(EditInput::Delete);
        assert!(scrap.annotations().is_empty());
        assert!(scrap.selected_annotations().is_empty());
    }

    #[test]
    fn the_history_stays_after_leaving_so_ctrl_z_works_on_coming_back() {
        let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
        let o = scrap.image_rect().origin();
        let at = |x: i32, y: i32| Point::new(o.x + x, o.y + y);
        let ctrl = |vk: u16| EditInput::Key(Chord::plain(Key::known(vk)).ctrl());
        scrap.start_editing(Style::default());
        for y in [10, 20] {
            scrap.edit_input(EditInput::Press(at(10, y)));
            scrap.edit_input(EditInput::Move {
                at: at(50, y + 10),
                shift: false,
            });
            scrap.edit_input(EditInput::Release);
        }
        scrap.edit_input(ctrl(0x5A));
        scrap.edit_input(EditInput::Leave);
        assert_eq!(scrap.annotations().items().len(), 1);

        scrap.start_editing(Style::default());
        assert!(scrap.annotations().can_redo());
        assert_eq!(scrap.edit_input(ctrl(0x59)), vec![EditOutput::Redraw]);
        assert_eq!(scrap.annotations().items().len(), 2);
        scrap.edit_input(ctrl(0x5A));
        scrap.edit_input(ctrl(0x5A));
        assert!(scrap.annotations().is_empty());
        assert!(!scrap.annotations().can_undo());
    }

    #[test]
    fn ctrl_z_outside_the_edit_mode_changes_nothing() {
        let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
        let o = scrap.image_rect().origin();
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Press(o));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 20, o.y + 20),
            shift: false,
        });
        scrap.edit_input(EditInput::Leave);
        let ctrl_z = EditInput::Key(Chord::plain(Key::known(0x5A)).ctrl());
        assert_eq!(scrap.edit_input(ctrl_z), vec![]);
        assert_eq!(scrap.annotations().items().len(), 1);
    }

    #[test]
    fn zooming_and_untrimming_keep_the_annotations_and_move_only_where_they_are_drawn() {
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 7)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 41, o.y + 21),
            shift: false,
        });
        scrap.edit_input(EditInput::Release);
        let drawn = scrap.annotations().clone();
        // 200% でトリムの左上が (10, 20) なので、画素 (14, 23) の中心は画像の左上から (9, 7)
        let view = scrap.annotation_view();
        assert_eq!(view.point(Point::new(14, 23)), (9.0, 7.0));
        assert_eq!(view.width(3), 6.0);
        scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x1B))));
        scrap.set_zoom(50, ZoomAnchor::Center);
        assert_eq!(scrap.annotations(), &drawn);
        assert_eq!(
            scrap.annotation_view().point(Point::new(14, 23)),
            (2.25, 1.75)
        );
        assert_eq!(scrap.annotation_view().width(3), 1.5);
        scrap.untrim();
        assert_eq!(scrap.annotations(), &drawn);
        assert_eq!(
            scrap.annotation_view().point(Point::new(14, 23)),
            (7.25, 11.75)
        );
        // 保存とコピーは、表示している画像の画素の大きさに描く
        scrap.set_zoom(300, ZoomAnchor::Center);
        assert_eq!(scrap.output_view().point(Point::new(14, 23)), (14.5, 23.5));
        assert_eq!(scrap.output_view().width(3), 3.0);
    }

    #[test]
    fn the_output_of_a_trimmed_scrap_starts_at_the_trimmed_corner() {
        let scrap = trimmed_and_zoomed();
        assert_eq!(scrap.output_view().point(Point::new(14, 23)), (4.5, 3.5));
    }

    #[test]
    fn escape_leaves_the_edit_mode_and_keeps_what_was_drawn() {
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        scrap.start_editing(Style::default());
        // 押したまま Esc でも、描きかけを残して抜ける
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 7)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 41, o.y + 21),
            shift: false,
        });
        let outputs = scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x1B))));
        assert!(outputs.contains(&EditOutput::Left));
        assert_eq!(scrap.mode, Mode::Normal);
        assert_eq!(scrap.annotations().items().len(), 1);
        // 抜けたあとの入力は受けない
        assert_eq!(scrap.edit_input(EditInput::Press(o)), vec![]);
    }

    #[test]
    fn keys_other_than_escape_are_not_taken_while_editing() {
        let mut scrap = trimmed_and_zoomed();
        scrap.start_editing(Style::default());
        assert_eq!(
            scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x41)))),
            vec![]
        );
        assert!(scrap.is_editing());
    }

    #[test]
    fn leaving_in_the_middle_of_a_drag_keeps_it_unless_it_has_no_length() {
        // 「完了」とスクラップが非アクティブになったとき（EditInput::Leave）
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 7)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 41, o.y + 21),
            shift: false,
        });
        let outputs = scrap.edit_input(EditInput::Leave);
        assert_eq!(outputs, vec![EditOutput::Redraw, EditOutput::Left]);
        assert!(!scrap.is_editing());
        assert_eq!(
            scrap.annotations().items()[0].shape,
            Shape::Rect {
                from: Point::new(14, 23),
                to: Point::new(30, 30)
            }
        );

        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 7)));
        assert_eq!(scrap.edit_input(EditInput::Leave), vec![EditOutput::Left]);
        assert_eq!(scrap.annotations().items().len(), 1);
    }

    #[test]
    fn settling_leaves_the_edit_mode_keeping_the_drawing_and_cancels_a_trim() {
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 7)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 41, o.y + 21),
            shift: false,
        });
        scrap.settle();
        assert_eq!(scrap.mode, Mode::Normal);
        assert_eq!(scrap.annotations().items().len(), 1);
        scrap.start_trim();
        scrap.settle();
        assert_eq!(scrap.mode, Mode::Normal);
    }

    #[test]
    fn editing_cannot_start_while_trimming_or_folded() {
        let mut scrap = trimmed_and_zoomed();
        scrap.start_trim();
        assert!(!scrap.start_editing(Style::default()));
        scrap.trim_input(TrimInput::Cancel);
        scrap.enter_compact(ZoomAnchor::Center);
        assert!(!scrap.start_editing(Style::default()));
        scrap.leave_compact();
        assert!(scrap.start_editing(Style::default()));
        assert!(!scrap.start_editing(Style::default()));
    }

    #[test]
    fn drawing_changes_the_content() {
        let mut scrap = trimmed_and_zoomed();
        let o = scrap.image_rect().origin();
        scrap.start_editing(Style::default());
        let before = scrap.clone();
        scrap.edit_input(EditInput::Press(Point::new(o.x + 8, o.y + 7)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 41, o.y + 21),
            shift: false,
        });
        assert!(!scrap.same_content(&before));
        scrap.settle();
        let mut drawn_in_normal = before.clone();
        drawn_in_normal.settle();
        assert!(!scrap.same_content(&drawn_in_normal));
    }

    /// (100, 50) の 300x200 に、太さ 3 の矩形を描いてから、元の画像の (0, 0)〜(100, 100) にトリムしたもの。
    /// 矩形は中にあるもの (10, 10)〜(40, 40)、右の縁にまたがるもの (80, 60)〜(150, 90)、縁から 4 画素だけ外に
    /// あるもの (104, 20)〜(150, 50) の順。
    fn drawn_then_trimmed() -> ScrapModel {
        let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
        let at = |x: i32, y: i32| Point::new(100 + x, 50 + y);
        scrap.start_editing(Style::default());
        for (from, to) in [
            ((10, 10), (40, 40)),
            ((80, 60), (150, 90)),
            ((104, 20), (150, 50)),
        ] {
            scrap.edit_input(EditInput::Press(at(from.0, from.1)));
            scrap.edit_input(EditInput::Move {
                at: at(to.0, to.1),
                shift: false,
            });
            scrap.edit_input(EditInput::Release);
        }
        scrap.edit_input(EditInput::Leave);
        scrap.start_trim();
        scrap.trim_input(TrimInput::Press(at(0, 0)));
        scrap.trim_input(TrimInput::Move(at(99, 99)));
        scrap.trim_input(TrimInput::Release);
        scrap.trim_input(TrimInput::Confirm);
        assert_eq!(scrap.trim_rect(), Some(Rect::new(0, 0, 100, 100)));
        scrap
    }

    fn select_tool(scrap: &mut ScrapModel) {
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Pick(Pick::Tool(Tool::Select)));
    }

    fn click_at(scrap: &mut ScrapModel, at: Point) {
        scrap.edit_input(EditInput::Press(at));
        scrap.edit_input(EditInput::Release);
    }

    #[test]
    fn annotations_outside_the_trim_are_not_drawn_nor_selected_and_come_back_on_untrimming() {
        let mut scrap = drawn_then_trimmed();
        let all = scrap.annotations().items().to_vec();
        // 範囲の外のものはモデルには残るが、描かない
        assert_eq!(all.len(), 3);
        assert_eq!(scrap.annotations_to_draw(), all[..2].to_vec());
        select_tool(&mut scrap);
        // 画像の外のクリックは右の縁の画素 (99, 30) になる。外の矩形の左の辺から 5 画素で当たりの内だが、選ばない
        click_at(&mut scrap, Point::new(100 + 300, 50 + 30));
        assert!(scrap.selected_annotations().is_empty());
        // 縁にまたがるものは、見えている部分のクリックで選べる
        click_at(&mut scrap, Point::new(100 + 90, 50 + 60));
        assert_eq!(scrap.selected_annotations(), [&all[1]]);
        scrap.edit_input(EditInput::Leave);
        assert!(scrap.untrim());
        assert_eq!(scrap.annotations_to_draw(), all);
        select_tool(&mut scrap);
        click_at(&mut scrap, Point::new(100 + 104, 50 + 30));
        assert_eq!(scrap.selected_annotations(), [&all[2]]);
    }

    #[test]
    fn pushing_an_annotation_out_of_the_trim_with_arrow_keys_hides_it_and_ctrl_z_brings_it_back() {
        let mut scrap = drawn_then_trimmed();
        let inside = scrap.annotations().items()[0].clone();
        select_tool(&mut scrap);
        click_at(&mut scrap, Point::new(100 + 25, 50 + 10));
        assert_eq!(scrap.selected_annotations(), [&inside]);
        let shift_right = EditInput::Key(Chord::plain(Key::known(0x27)).shift());
        // 左の辺の外側（太さの半分）が 8 なので、10 画素ずつ 10 回で範囲 (0, 0)〜(100, 100) の外に出る
        for _ in 0..9 {
            scrap.edit_input(shift_right.clone());
            assert!(!scrap.selected_annotations().is_empty());
        }
        assert_eq!(
            scrap.edit_input(shift_right.clone()),
            vec![EditOutput::Redraw]
        );
        assert_eq!(scrap.annotations().items().len(), 3);
        assert_eq!(scrap.annotations_to_draw().len(), 1);
        // 隠れたものは選択から外れ、矢印キーも Delete も効かない
        assert!(scrap.selected_annotations().is_empty());
        assert_eq!(scrap.edit_input(shift_right), vec![]);
        assert_eq!(scrap.edit_input(EditInput::Delete), vec![]);
        // 元に戻すで範囲の中に戻る
        scrap.edit_input(EditInput::Key(Chord::plain(Key::known(0x5A)).ctrl()));
        assert_eq!(scrap.annotations_to_draw().len(), 2);
    }

    #[test]
    fn a_scrap_closed_while_editing_comes_back_from_the_trash_with_its_annotations_and_history() {
        let mut scrap = drawn_then_trimmed();
        let o = scrap.image_rect().origin();
        // 描きかけのまま閉じる（トレイの一括操作など）。閉じる前に抜けて、描きかけを書き込みにしてから入れる
        scrap.start_editing(Style::default());
        scrap.edit_input(EditInput::Press(Point::new(o.x + 20, o.y + 50)));
        scrap.edit_input(EditInput::Move {
            at: Point::new(o.x + 60, o.y + 80),
            shift: false,
        });
        scrap.settle();
        assert!(!scrap.is_editing());
        let closed = scrap.annotations().clone();
        assert_eq!(closed.items().len(), 4);
        let mut trash = crate::trash::Trash::new(5);
        trash.push(scrap);

        let mut restored = trash.restore_last().expect("ごみ箱に入っている");
        assert_eq!(restored.mode, Mode::Normal);
        assert_eq!(restored.annotations(), &closed);
        assert_eq!(restored.trim_rect(), Some(Rect::new(0, 0, 100, 100)));
        assert_eq!(restored.annotations_to_draw().len(), 3);
        // 履歴も戻っているので、入り直して元に戻すと描きかけだったものから消える
        restored.start_editing(Style::default());
        restored.edit_input(EditInput::Key(Chord::plain(Key::known(0x5A)).ctrl()));
        assert_eq!(restored.annotations().items(), &closed.items()[..3]);
        assert!(restored.annotations().can_redo());
    }

    #[test]
    fn a_folded_scrap_draws_its_annotations_in_the_square() {
        let mut scrap = drawn_then_trimmed();
        scrap.untrim();
        assert!(scrap.enter_compact(ZoomAnchor::Center));
        // 300x200 の中心 (150, 100) のまわりの 64x64、つまり画素 (118, 68) からが正方形に映る
        assert_eq!(scrap.compact_view(), Some(Rect::new(118, 68, 182, 132)));
        assert_eq!(scrap.annotations_to_draw().len(), 3);
        assert_eq!(
            scrap.annotation_view().point(Point::new(130, 80)),
            (12.5, 12.5)
        );
        assert_eq!(scrap.annotation_view().width(3), 3.0);
    }
}
