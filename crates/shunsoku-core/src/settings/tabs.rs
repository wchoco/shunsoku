//! 設定 GUI のタブに並べる項目と、その並び（上から）。Win32 の層は、この順に欄を置くだけにする。

use super::HotkeyAction;

/// 「キャプチャ」タブの項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureItem {
    /// `[hotkeys]` のホットキーの欄。
    Hotkey(HotkeyAction),
    /// `[general] capture_cursor`。
    CaptureCursor,
    /// `[selection] mode`。
    SelectionMode,
    /// `[selection] crosshair`。
    Crosshair,
    /// `[selection] crosshair_color`。
    CrosshairColor,
    /// キーの組み合わせの欄の使い方。
    Hint,
    /// 「白紙スクラップ」の見出し。ここから下は白紙スクラップの欄。
    BlankScrapHeading,
    /// `[annotation] background`（新しく作る白紙スクラップの背景色）。
    BlankBackground,
}

/// 「キャプチャ」タブの並び。
pub const CAPTURE_TAB: [CaptureItem; 11] = [
    CaptureItem::Hotkey(HotkeyAction::SelectCapture),
    CaptureItem::Hotkey(HotkeyAction::ActiveWindowCapture),
    CaptureItem::Hotkey(HotkeyAction::RestoreLast),
    CaptureItem::CaptureCursor,
    CaptureItem::SelectionMode,
    CaptureItem::Crosshair,
    CaptureItem::CrosshairColor,
    CaptureItem::Hint,
    CaptureItem::BlankScrapHeading,
    CaptureItem::Hotkey(HotkeyAction::BlankScrap),
    CaptureItem::BlankBackground,
];

impl CaptureItem {
    /// 欄の見出し（見出しの項目ではその文字）の文言のキー（`settings_gui.capture.*`）。
    pub fn label_key(self) -> &'static str {
        match self {
            CaptureItem::Hotkey(HotkeyAction::SelectCapture) => {
                "settings_gui.capture.select_capture"
            }
            CaptureItem::Hotkey(HotkeyAction::ActiveWindowCapture) => {
                "settings_gui.capture.active_window_capture"
            }
            CaptureItem::Hotkey(HotkeyAction::RestoreLast) => "settings_gui.capture.restore_last",
            CaptureItem::Hotkey(HotkeyAction::BlankScrap) => {
                "settings_gui.capture.blank_scrap_hotkey"
            }
            CaptureItem::Hotkey(HotkeyAction::Region(_)) => "settings_gui.regions.hotkey",
            CaptureItem::CaptureCursor => "settings_gui.capture.capture_cursor",
            CaptureItem::SelectionMode => "settings_gui.capture.mode",
            CaptureItem::Crosshair => "settings_gui.capture.crosshair",
            CaptureItem::CrosshairColor => "settings_gui.capture.crosshair_color",
            CaptureItem::Hint => "settings_gui.capture.hint",
            CaptureItem::BlankScrapHeading => "settings_gui.capture.blank_scrap",
            CaptureItem::BlankBackground => "settings_gui.capture.blank_background",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Catalog;

    #[test]
    fn the_blank_scrap_heading_hotkey_and_background_close_the_capture_tab() {
        assert_eq!(
            CAPTURE_TAB[CAPTURE_TAB.len() - 3..],
            [
                CaptureItem::BlankScrapHeading,
                CaptureItem::Hotkey(HotkeyAction::BlankScrap),
                CaptureItem::BlankBackground,
            ]
        );
    }

    #[test]
    fn every_hotkey_has_a_field_on_the_capture_tab() {
        for action in HotkeyAction::ALL {
            assert!(
                CAPTURE_TAB.contains(&CaptureItem::Hotkey(action)),
                "{action:?}"
            );
        }
    }

    #[test]
    fn every_capture_tab_item_has_a_text_in_the_bundled_locales() {
        for src in [
            include_str!("../../../../locales/ja.toml"),
            include_str!("../../../../locales/en.toml"),
        ] {
            let catalog = Catalog::parse(src).unwrap();
            for item in CAPTURE_TAB {
                let key = item.label_key();
                assert_ne!(catalog.text(key), key, "{item:?}");
            }
        }
    }
}
