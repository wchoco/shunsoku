//! スクラップの状態（`ScrapModel`）と見た目（`Appearance`）。
//!
//! 位置はスクリーン座標（物理ピクセル）で持ち、画像の画素に倍率（`ScrapModel::zoom`、100% が画素等倍）を
//! 掛けた大きさで出す。ウィンドウは画像の外側に
//! 枠と影の分だけ大きく作り、画像の位置（`ScrapModel::position`）がちょうど撮った範囲に重なるようにする。

pub mod ops;

use crate::geometry::{Point, Rect, Size};
use crate::settings::ScrapAppearance;

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
    /// 見た目のスクラップごとの上書き。全体の既定値に重ねる（`ScrapModel::appearance`）。
    pub appearance_overrides: AppearanceOverrides,
}

impl ScrapModel {
    pub fn new(position: Point, image_size: Size) -> ScrapModel {
        ScrapModel {
            position,
            image_size,
            mode: Mode::Normal,
            zoom: ZOOM_ACTUAL,
            appearance_overrides: AppearanceOverrides::default(),
        }
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

    /// ウィンドウの矩形（スクリーン座標）。画像の外側に、実際の見た目（既定値 `defaults` に上書きを重ねたもの）の
    /// 枠と影の分を足す。
    pub fn window_rect(&self, defaults: &Appearance) -> Rect {
        self.image_rect()
            .inflate(self.appearance(defaults).margin())
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

/// 見た目のスクラップごとの上書き。None の項目は全体の既定値（設定ファイルの `[scrap]`）のまま。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppearanceOverrides {
    /// 透明度（%）。
    pub opacity: Option<u8>,
    /// 枠。`Some(None)` で枠なし、`Some(Some(color))` でその色（`0xRRGGBB`）の枠。太さは既定値の太さ（既定値が
    /// 枠なしなら設定の既定の太さ）。
    pub border: Option<Option<u32>>,
    /// 影の有無。
    pub shadow: Option<bool>,
}

impl AppearanceOverrides {
    /// `defaults` に重ねた見た目。
    fn apply(&self, defaults: &Appearance) -> Appearance {
        let border = match self.border {
            None => defaults.border,
            Some(None) => None,
            Some(Some(color)) => Some(Border {
                width: defaults
                    .border
                    .map_or(crate::settings::Border::default().width, |b| b.width),
                color,
            }),
        };
        Appearance {
            opacity: self.opacity.unwrap_or(defaults.opacity),
            border,
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
    fn without_overrides_the_scrap_looks_like_the_defaults() {
        let scrap = ScrapModel::new(Point::new(0, 0), Size::new(300, 200));
        let defaults = Appearance {
            opacity: 80,
            border: Some(Border {
                width: 3,
                color: 0x12_34_56,
            }),
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
            border: Some(Border {
                width: 4,
                color: 0x80_80_80,
            }),
            ..defaults
        };
        scrap.appearance_overrides.border = Some(Some(0xFF_00_00));
        assert_eq!(
            scrap.appearance(&thick).border,
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
            border: Some(Some(0x00_00_FF)),
            shadow: Some(true),
        };
        let plain = Appearance {
            opacity: 100,
            border: None,
            shadow: false,
        };
        // 既定値で枠がなければ、枠を付けたときの太さは設定の既定の太さ
        assert_eq!(
            scrap.appearance(&plain),
            Appearance {
                opacity: 30,
                border: Some(Border {
                    width: 1,
                    color: 0x00_00_FF,
                }),
                shadow: true,
            }
        );
        scrap.appearance_overrides.border = Some(None);
        scrap.appearance_overrides.shadow = Some(false);
        assert_eq!(scrap.appearance(&Appearance::default()).border, None);
        assert!(!scrap.appearance(&Appearance::default()).shadow);
    }

    #[test]
    fn the_window_follows_the_overridden_border_and_shadow() {
        let mut scrap = ScrapModel::new(Point::new(-100, 50), Size::new(300, 200));
        let defaults = Appearance::default();
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-109, 41, 209, 259));
        scrap.appearance_overrides.shadow = Some(false);
        assert_eq!(scrap.window_rect(&defaults), Rect::new(-101, 49, 201, 251));
        scrap.appearance_overrides.border = Some(None);
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
