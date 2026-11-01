//! スクラップの状態（`ScrapModel`）と見た目（`Appearance`）。
//!
//! 位置はスクリーン座標（物理ピクセル）で持ち、画像の画素に倍率（`ScrapModel::zoom`、100% が画素等倍）を
//! 掛けた大きさで出す。ウィンドウは画像の外側に
//! 枠と影の分だけ大きく作り、画像の位置（`ScrapModel::position`）がちょうど撮った範囲に重なるようにする。
//!
//! トリムは、元の画像（`ScrapModel::image_size`）の中の表示する範囲だけを持つ。画素は持たず、表示する画像は
//! 元の画像から `ScrapModel::shown_image` で切り出す。トリムの範囲の選択（`Mode::Trimming`）は
//! `ScrapModel::trim_input` が受ける。

pub mod ops;

use crate::geometry::{Point, Rect, Size};
use crate::image::Image;
use crate::settings::{DpiScaling, ScrapAppearance};

/// スクラップの表示のモード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 画像をそのまま出す。
    #[default]
    Normal,
    /// トリムの範囲を選んでいる（`ScrapModel::start_trim` から確定か中止まで）。スクラップ操作はどれも効かない。
    Trimming(TrimState),
}

/// トリムの範囲の選択の状態。座標は表示している画像（トリム済みならその部分）の画素。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrimState {
    /// 左ボタンを押した画素。押している間だけ Some。
    pressed: Option<Point>,
    /// 押してから、押した画素の外へ動いたか。動いたら範囲を選び直す。
    dragged: bool,
    /// 選んだ範囲。動かさずに押して離しても（ダブルクリックの 1 回目）消さない。
    selection: Option<Rect>,
}

/// トリムの範囲の選択への入力。座標はスクリーン座標（物理ピクセル）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimInput {
    /// 左ボタンを押した。
    Press(Point),
    /// カーソルが動いた。押している間だけ範囲を変える。
    Move(Point),
    /// 左ボタンを離した、またはマウスを捕まえていられなくなった。
    Release,
    /// Enter かダブルクリック。選んだ範囲でトリムする。
    Confirm,
    /// `Esc`。トリムをやめる。
    Cancel,
}

impl TrimInput {
    /// カーソルが `at` に動いたときの入力。押したまま（`Pointer::holding`）でなければ、離したことを取りこぼした
    /// ものとして `Release` にする。
    pub fn pointer_move(at: Point, pointer: Pointer) -> TrimInput {
        if pointer.holding() {
            TrimInput::Move(at)
        } else {
            TrimInput::Release
        }
    }
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
struct Folded {
    /// 畳む直前の画像の左上。
    unfolded: Point,
    /// 畳んだ直後の正方形の左上。
    folded: Point,
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
    folded: Option<Folded>,
    /// トリムしていれば、表示する範囲（元の画像の画素）。
    trimmed: Option<Rect>,
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
            folded: None,
            trimmed: None,
        }
    }

    /// コンパクト表示に畳んでいるか。
    pub fn is_compact(&self) -> bool {
        self.folded.is_some()
    }

    /// コンパクト表示（一辺 `compact_side` の正方形）に畳む。`anchor` が正方形の中心になる（`Center` なら
    /// 画像の中心）。倍率はそのまま残る。もう畳んでいれば false。
    pub fn fold(&mut self, anchor: ZoomAnchor) -> bool {
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
        let folded = Point::new(center.x.saturating_sub(half), center.y.saturating_sub(half));
        self.folded = Some(Folded {
            unfolded: self.position,
            folded,
        });
        self.position = folded;
        true
    }

    /// コンパクト表示から広げる。畳む前の位置に、畳んだあとに動かした分だけずらして戻す。畳んでいなければ false。
    pub fn unfold(&mut self) -> bool {
        let Some(Folded { unfolded, folded }) = self.folded.take() else {
            return false;
        };
        let p = self.position;
        self.position = Point::new(
            unfolded.x.saturating_add(p.x.saturating_sub(folded.x)),
            unfolded.y.saturating_add(p.y.saturating_sub(folded.y)),
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
        if let Some(folded) = self.folded.as_mut() {
            folded.folded = Point::new(
                folded
                    .folded
                    .x
                    .saturating_add(self.position.x.saturating_sub(p.x)),
                folded
                    .folded
                    .y
                    .saturating_add(self.position.y.saturating_sub(p.y)),
            );
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
        if matches!(self.mode, Mode::Trimming(_)) {
            return false;
        }
        self.mode = Mode::Trimming(TrimState::default());
        true
    }

    /// トリムの範囲の選択への入力を受ける。描き直しが要る（範囲かモードか表示が変わった）なら true。
    /// トリムの最中でなければ何もしない。
    pub fn trim_input(&mut self, input: TrimInput) -> bool {
        let Mode::Trimming(mut state) = self.mode else {
            return false;
        };
        let before = state;
        match input {
            TrimInput::Press(at) => {
                state.pressed = Some(self.shown_pixel_at(at));
                state.dragged = false;
            }
            TrimInput::Move(at) => {
                if let Some(from) = state.pressed {
                    let to = self.shown_pixel_at(at);
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
            TrimInput::Release => state.pressed = None,
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
        state.selection != before.selection
    }

    /// 選んでいるトリムの範囲（スクリーン座標）。画素の境目に合わせる。選んでいなければ None。
    pub fn trim_selection(&self) -> Option<Rect> {
        let Mode::Trimming(TrimState {
            selection: Some(r), ..
        }) = self.mode
        else {
            return None;
        };
        let p = self.position;
        let at = |origin: i32, pixels: i32| {
            let offset = i64::try_from(self.scale(pixels.max(0) as u32)).unwrap_or(i64::MAX);
            (i64::from(origin) + offset).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        Some(Rect::new(
            at(p.x, r.left),
            at(p.y, r.top),
            at(p.x, r.right),
            at(p.y, r.bottom),
        ))
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
    /// 枠。
    pub border: Option<BorderOverride>,
    /// 影の有無。
    pub shadow: Option<bool>,
}

/// 枠のスクラップごとの上書き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderOverride {
    /// 枠を出さない。
    Hidden,
    /// その色（`0xRRGGBB`）の枠を出す。太さは既定値の太さ（既定値で枠を出さなくても設定の太さ）。
    Color(u32),
}

impl AppearanceOverrides {
    /// `defaults` に重ねた見た目。
    fn apply(&self, defaults: &Appearance) -> Appearance {
        let (border, show_border) = match self.border {
            None => (defaults.border, defaults.show_border),
            Some(BorderOverride::Hidden) => (defaults.border, false),
            Some(BorderOverride::Color(color)) => (
                Border {
                    color,
                    ..defaults.border
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
        let (r, g, b) = (border.color.0, border.color.1, border.color.2);
        Appearance {
            opacity: settings.opacity,
            border: Border {
                width: border.width,
                color: u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
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
    use crate::image::Image;

    #[test]
    fn a_new_scrap_is_normal_and_shows_its_image_where_it_was_captured() {
        let scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        assert_eq!(scrap.mode, Mode::Normal);
        assert_eq!(scrap.image_rect(), Rect::new(-100, 50, 200, 250));
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

    /// (x, y) の画素の B に x、G に y を入れた画像（切り抜いた位置が分かるように）。
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
        matches!(scrap.mode, Mode::Trimming(_))
    }

    #[test]
    fn dragging_while_trimming_selects_the_pixels_under_both_ends() {
        let mut scrap = ScrapModel::new(Point::new(100, 50), Size::new(300, 200));
        assert!(scrap.start_trim());
        assert!(trimming(&scrap));
        assert_eq!(scrap.trim_selection(), None);
        // 押す前のホバーでは何も選ばない
        assert!(!scrap.trim_input(TrimInput::Move(Point::new(120, 60))));
        assert_eq!(scrap.trim_selection(), None);
        scrap.trim_input(TrimInput::Press(Point::new(130, 70)));
        assert!(scrap.trim_input(TrimInput::Move(Point::new(110, 90))));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(110, 70, 131, 91)));
        scrap.trim_input(TrimInput::Release);
        // 押した画素へ戻ると 1 画素の範囲になる
        scrap.trim_input(TrimInput::Press(Point::new(130, 70)));
        scrap.trim_input(TrimInput::Move(Point::new(140, 70)));
        scrap.trim_input(TrimInput::Move(Point::new(130, 70)));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(130, 70, 131, 71)));
        scrap.trim_input(TrimInput::Move(Point::new(110, 90)));
        scrap.trim_input(TrimInput::Release);
        // 離したあとのホバーでは範囲は変わらない
        assert!(!scrap.trim_input(TrimInput::Move(Point::new(300, 200))));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(110, 70, 131, 91)));
        // 画像の外へはみ出したドラッグは画像の端で止まる
        scrap.trim_input(TrimInput::Press(Point::new(390, 240)));
        scrap.trim_input(TrimInput::Move(Point::new(1000, 1000)));
        assert_eq!(scrap.trim_selection(), Some(Rect::new(390, 240, 400, 250)));
        // 選んでいる間は大きさも位置も変わらない
        assert_eq!(scrap.image_rect(), Rect::new(100, 50, 400, 250));
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
        assert!(scrap.fold(ZoomAnchor::Center));
        assert!(scrap.is_compact());
        assert_eq!(scrap.image_rect(), Rect::new(18, 118, 82, 182));
        assert_eq!(
            scrap.window_rect(&Appearance::default()),
            Rect::new(9, 109, 91, 191)
        );
        // 畳んだまま畳んでも変わらない
        assert!(!scrap.fold(ZoomAnchor::Center));
        assert_eq!(scrap.image_rect(), Rect::new(18, 118, 82, 182));
    }

    #[test]
    fn folding_with_the_mouse_centers_the_square_on_the_cursor() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        assert!(scrap.fold(ZoomAnchor::At(Point::new(-90, 60))));
        assert_eq!(scrap.image_rect(), Rect::new(-122, 28, -58, 92));
    }

    #[test]
    fn unfolding_without_moving_returns_to_where_it_was_before_folding() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        assert!(!scrap.unfold());
        scrap.fold(ZoomAnchor::At(Point::new(-90, 60)));
        assert!(scrap.unfold());
        assert!(!scrap.is_compact());
        assert_eq!(scrap.image_rect(), Rect::new(-100, 50, 200, 250));
        assert!(!scrap.unfold());
    }

    #[test]
    fn unfolding_shifts_the_scrap_by_how_far_the_square_was_moved() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        scrap.fold(ZoomAnchor::Center);
        scrap.position = Point::new(scrap.position.x + 30, scrap.position.y - 500);
        scrap.unfold();
        assert_eq!(scrap.image_rect(), Rect::new(-70, -450, 230, -250));
    }

    #[test]
    fn a_folded_scrap_rescales_its_zoom_for_when_it_unfolds_on_another_dpi() {
        let keep = DpiScaling::KeepApparentSize;
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(144, keep, ZoomAnchor::Center);
        scrap.fold(ZoomAnchor::Center);
        scrap.set_dpi(96, keep, ZoomAnchor::Center);
        // 倍率は計算し直してあり、広げると移動先の DPI での見かけの大きさで出る
        assert_eq!(scrap.zoom(), 67);
        scrap.unfold();
        assert_eq!(scrap.image_rect().size(), Size::new(201, 134));
    }

    #[test]
    fn resizing_the_square_for_a_dpi_does_not_count_as_moving_it() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(96, DpiScaling::KeepPixels, ZoomAnchor::Center);
        scrap.fold(ZoomAnchor::Center);
        scrap.set_dpi(144, DpiScaling::KeepPixels, ZoomAnchor::Center);
        scrap.unfold();
        assert_eq!(scrap.image_rect(), Rect::new(0, 0, 300, 200));
    }

    #[test]
    fn the_compact_square_is_64px_at_96_dpi_and_scales_with_the_dpi() {
        let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        scrap.set_dpi(144, DpiScaling::KeepPixels, ZoomAnchor::Center);
        // 中心は (150, 100)。144 DPI では 64px 相当は 96px
        scrap.fold(ZoomAnchor::Center);
        assert_eq!(scrap.image_rect(), Rect::new(102, 52, 198, 148));
        let mut at_96 = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        at_96.set_dpi(96, DpiScaling::KeepPixels, ZoomAnchor::Center);
        at_96.fold(ZoomAnchor::Center);
        assert_eq!(at_96.image_rect(), Rect::new(118, 68, 182, 132));
    }

    #[test]
    fn a_folded_scrap_resizes_its_square_around_the_anchor_on_another_dpi() {
        for scaling in [DpiScaling::KeepPixels, DpiScaling::KeepApparentSize] {
            let mut scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
            scrap.set_dpi(96, scaling, ZoomAnchor::Center);
            scrap.fold(ZoomAnchor::Center);
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
        scrap.fold(ZoomAnchor::Center);
        assert_eq!(scrap.image_rect(), Rect::new(268, 168, 332, 232));
        assert_eq!(scrap.zoom(), 200);
        scrap.unfold();
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
            TrimInput::Release
        );
        assert_eq!(
            TrimInput::pointer_move(Point::new(30, 30), HOLDING),
            TrimInput::Move(Point::new(30, 30))
        );
        scrap.trim_input(TrimInput::pointer_move(Point::new(30, 30), lost));
        // 押していないことになったので、そのあとの移動では範囲を選ばない
        assert!(!scrap.trim_input(TrimInput::Move(Point::new(40, 40))));
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
}
