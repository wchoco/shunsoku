//! スクラップの状態（`ScrapModel`）と見た目（`Appearance`）。
//!
//! 位置はスクリーン座標（物理ピクセル）で持ち、画像の画素を等倍で出す。ウィンドウは画像の外側に
//! 枠と影の分だけ大きく作り、画像の位置（`ScrapModel::position`）がちょうど撮った範囲に重なるようにする。

use crate::geometry::{Point, Rect, Size};
use crate::settings::ScrapAppearance;

/// スクラップの表示のモード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 画像をそのまま出す。
    #[default]
    Normal,
}

/// 1 枚のスクラップの状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapModel {
    /// 画像の左上（スクリーン座標、物理ピクセル）。
    pub position: Point,
    /// 画像の大きさ（画素）。
    pub image_size: Size,
    pub mode: Mode,
}

impl ScrapModel {
    pub fn new(position: Point, image_size: Size) -> ScrapModel {
        ScrapModel {
            position,
            image_size,
            mode: Mode::Normal,
        }
    }

    /// 画像を出す矩形（スクリーン座標）。
    pub fn image_rect(&self) -> Rect {
        Rect::from_origin_size(self.position, self.image_size)
    }

    /// ウィンドウの矩形（スクリーン座標）。画像の外側に枠と影の分を足す。
    pub fn window_rect(&self, appearance: &Appearance) -> Rect {
        self.image_rect().inflate(appearance.margin())
    }
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
}
