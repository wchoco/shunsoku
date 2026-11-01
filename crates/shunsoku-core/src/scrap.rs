//! スクラップの状態（`ScrapModel`）と見た目（`Appearance`）。
//!
//! 位置はスクリーン座標（物理ピクセル）で持ち、画像の画素に倍率（`ScrapModel::zoom`、100% が画素等倍）を
//! 掛けた大きさで出す。ウィンドウは画像の外側に
//! 枠と影の分だけ大きく作り、画像の位置（`ScrapModel::position`）がちょうど撮った範囲に重なるようにする。

pub mod ops;

use crate::geometry::{Point, Rect, Size};
use crate::settings::{DpiScaling, ScrapAppearance};

/// スクラップの表示のモード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 画像をそのまま出す。
    #[default]
    Normal,
}

/// 倍率の下限（%）。
pub const ZOOM_MIN: u16 = 10;
/// 倍率の上限（%）。
pub const ZOOM_MAX: u16 = 800;
/// 画素等倍の倍率（%）。
pub const ZOOM_ACTUAL: u16 = 100;

/// 拡大縮小の基準点。この点の下にある画像の画素が、倍率を変えても同じ画面上の位置に残る。
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
    /// 画像の大きさ（画素）。
    pub image_size: Size,
    pub mode: Mode,
    /// 倍率（%）。画像の画素が基準で、100 が画素等倍。`ZOOM_MIN..=ZOOM_MAX` に収める。
    zoom: u16,
    /// ウィンドウのあるモニタの DPI。最初の `set_dpi` までは分からない。
    dpi: Option<u32>,
    /// 「見かけの大きさを保つ」で倍率を計算し直す元: 最後に倍率を決めたときの倍率と DPI。倍率を DPI の比で
    /// 掛けるたびに丸めると、モニタを行き来するうちに倍率がずれていくので、ここから毎回計算する。
    /// 拡大縮小で倍率を変えたら捨てる。
    dpi_origin: Option<(u16, u32)>,
}

impl ScrapModel {
    pub fn new(position: Point, image_size: Size) -> ScrapModel {
        ScrapModel {
            position,
            image_size,
            mode: Mode::Normal,
            zoom: ZOOM_ACTUAL,
            dpi: None,
            dpi_origin: None,
        }
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
        match scaling {
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
        }
    }

    /// 倍率を `zoom`（%、範囲に収める）にし、`anchor` の下の画素が動かないように位置を動かす。
    fn scale_to(&mut self, zoom: u16, anchor: ZoomAnchor) -> bool {
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        if zoom == self.zoom {
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
        // 基準点から画像の左上までの距離を、表示の大きさの比で伸び縮みさせる
        let scale = |anchor: f64, from: i32, before: u32, after: u32| {
            let ratio = f64::from(after) / f64::from(before.max(1));
            let to = anchor - (anchor - f64::from(from)) * ratio;
            to.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        };
        self.position = Point::new(
            scale(anchor_x, p.x, before.width, after.width),
            scale(anchor_y, p.y, before.height, after.height),
        );
        true
    }

    /// 倍率を画素等倍に戻す（中心が基準）。等倍のままなら false。
    pub fn reset_zoom(&mut self) -> bool {
        self.set_zoom(ZOOM_ACTUAL, ZoomAnchor::Center)
    }

    /// 画像を出す大きさ（物理ピクセル）。画像の画素に倍率を掛けて四捨五入し、0 にはしない。
    pub fn display_size(&self) -> Size {
        let scale = |pixels: u32| {
            let scaled = (u64::from(pixels) * u64::from(self.zoom) + 50) / 100;
            u32::try_from(scaled).unwrap_or(u32::MAX).max(1)
        };
        Size::new(scale(self.image_size.width), scale(self.image_size.height))
    }

    /// 画像を出す矩形（スクリーン座標）。大きさは倍率を掛けた表示の大きさ。
    pub fn image_rect(&self) -> Rect {
        Rect::from_origin_size(self.position, self.display_size())
    }

    /// ウィンドウの矩形（スクリーン座標）。画像の外側に枠と影の分を足す。
    pub fn window_rect(&self, appearance: &Appearance) -> Rect {
        self.image_rect().inflate(appearance.margin())
    }
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
    pub border: Option<Border>,
    pub shadow: bool,
}

/// 影の幅（物理ピクセル）。枠の外側に広がる。
pub const SHADOW_WIDTH: u32 = 8;

impl Default for Appearance {
    /// 見た目の既定値（spec #32）: 透明度 100%、枠あり 1px `#808080`、影あり。
    fn default() -> Appearance {
        Appearance {
            opacity: 100,
            border: Some(Border {
                width: 1,
                color: 0x80_80_80,
            }),
            shadow: true,
        }
    }
}

impl Appearance {
    /// 画像の外側に足す幅（枠 + 影）。
    pub fn margin(&self) -> i32 {
        let border = self.border.map_or(0, |b| b.width);
        let shadow = if self.shadow { SHADOW_WIDTH } else { 0 };
        (border + shadow) as i32
    }
}

impl From<ScrapAppearance> for Appearance {
    /// 設定ファイルの `[scrap]` から作る。枠を出さない設定なら枠なし。
    fn from(settings: ScrapAppearance) -> Appearance {
        let border = settings.border;
        let (r, g, b) = (border.color.0, border.color.1, border.color.2);
        Appearance {
            opacity: settings.opacity,
            border: border.visible.then_some(Border {
                width: border.width,
                color: u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
            }),
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

    /// カーソルが `at` に動いた。ドラッグの間なら前回からの移動量。ドラッグしていなければ None。
    pub fn move_to(&mut self, at: Point) -> Option<Point> {
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
            opacity: 100,
            border: None,
            shadow: false,
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
                border: Some(Border {
                    width: 3,
                    color: 0x12_34_56,
                }),
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
        assert_eq!(Appearance::from(hidden).border, None);
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

    #[test]
    fn hovering_without_pressing_never_moves_the_scrap() {
        let mut drag = Drag::default();
        assert_eq!(drag.move_to(Point::new(10, 10)), None);
        assert_eq!(drag.move_to(Point::new(15, 12)), None);
        assert!(!drag.is_active());
    }

    #[test]
    fn a_drag_reports_each_move_from_the_previous_position_until_it_ends() {
        let mut drag = Drag::default();
        drag.start(Point::new(10, 10));
        assert_eq!(drag.move_to(Point::new(15, 8)), Some(Point::new(5, -2)));
        assert_eq!(drag.move_to(Point::new(14, 8)), Some(Point::new(-1, 0)));
        drag.end();
        assert_eq!(drag.move_to(Point::new(20, 20)), None);
        assert_eq!(drag.move_to(Point::new(25, 20)), None);
    }
}
