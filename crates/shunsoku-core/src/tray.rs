//! タスクトレイのアイコンへの操作を読む。`NOTIFYICON_VERSION_4` の通知の引数（lParam の下位のコードと
//! wParam）から、本体が何をするかを決める。Win32 層の `tray` はメッセージの値をそのまま渡すだけにする。
//!
//! 左クリックも右クリックもキーボードでの選択も、同じメニューを出す（#119）。ダブルクリックには何も割り当てない。
//! メニューに並べる項目と順番は `menu` で決め、Win32 層は項目ごとの文言と中身（定型領域・ごみ箱の一覧）を付けて出す。

/// `NIN_SELECT`（`WM_USER + 0`）。左クリック。
pub const NIN_SELECT: u32 = 0x0400;
/// `NINF_KEY`。`NIN_SELECT` に重ねるとキーボードで選んで Enter / Space（`NIN_KEYSELECT`）。
pub const NINF_KEY: u32 = 0x0001;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
/// `NIN_BALLOONUSERCLICK`（`WM_USER + 5`）。通知がクリックされた。
pub const NIN_BALLOONUSERCLICK: u32 = 0x0405;
/// `WM_CONTEXTMENU`。右クリック、またはキーボードでのメニュー。
pub const WM_CONTEXTMENU: u32 = 0x007B;

/// トレイのアイコンへの操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// メニューを出す。座標はスクリーン座標（物理ピクセル）。
    Menu {
        x: i32,
        y: i32,
    },
    /// 通知（`notify`）がクリックされた。何の通知だったかは持たない。
    NotificationClick,
    Other,
}

/// 通知の引数を読む。`code` は lParam の下位 16 ビット、`wparam` はアイコンの位置（下位が x、上位が y）。
pub fn event(code: u32, wparam: usize) -> Event {
    match code {
        // NIN_SELECT | NINF_KEY は NIN_KEYSELECT
        NIN_SELECT | NIN_KEYSELECT | WM_CONTEXTMENU => Event::Menu {
            x: (wparam & 0xFFFF) as u16 as i16 as i32,
            y: ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32,
        },
        NIN_BALLOONUSERCLICK => Event::NotificationClick,
        _ => Event::Other,
    }
}

/// トレイのメニューの項目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// 範囲選択キャプチャ。
    SelectCapture,
    /// 白紙スクラップ（範囲選択の画面で位置と大きさを選んで作る）。
    BlankScrap,
    /// 定型領域のサブメニュー。
    Regions,
    /// 定型領域を追加…。
    AddRegion,
    /// ごみ箱のサブメニュー。
    Trash,
    /// すべてのスクラップを閉じる。
    CloseAll,
    /// すべて隠す。
    HideAll,
    /// すべて表示。
    ShowAll,
    Separator,
    /// 設定…。
    Settings,
    /// 終了。
    Exit,
}

/// メニューに並べる項目（上から）。1 枚でも隠れたスクラップがあれば（`any_hidden`）「すべて表示」、なければ
/// 「すべて隠す」を出す。
pub fn menu(any_hidden: bool) -> Vec<MenuItem> {
    vec![
        MenuItem::SelectCapture,
        MenuItem::BlankScrap,
        MenuItem::Regions,
        MenuItem::AddRegion,
        MenuItem::Trash,
        MenuItem::CloseAll,
        if any_hidden {
            MenuItem::ShowAll
        } else {
            MenuItem::HideAll
        },
        MenuItem::Separator,
        MenuItem::Settings,
        MenuItem::Exit,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// x = 100, y = 200 を wParam の形にしたもの。
    const AT_100_200: usize = (200 << 16) | 100;

    #[test]
    fn the_menu_offers_a_blank_scrap_right_after_the_select_capture() {
        let items = menu(false);
        let at = items
            .iter()
            .position(|&i| i == MenuItem::SelectCapture)
            .unwrap();
        assert_eq!(items.get(at + 1), Some(&MenuItem::BlankScrap));
    }

    #[test]
    fn the_menu_offers_to_show_all_only_while_some_scrap_is_hidden() {
        assert!(menu(false).contains(&MenuItem::HideAll));
        assert!(!menu(false).contains(&MenuItem::ShowAll));
        assert!(menu(true).contains(&MenuItem::ShowAll));
        assert!(!menu(true).contains(&MenuItem::HideAll));
    }

    #[test]
    fn the_menu_ends_with_the_settings_and_exit() {
        assert!(menu(false).ends_with(&[MenuItem::Separator, MenuItem::Settings, MenuItem::Exit]));
    }

    #[test]
    fn a_left_click_opens_the_menu_at_the_icon() {
        assert_eq!(
            event(NIN_SELECT, AT_100_200),
            Event::Menu { x: 100, y: 200 }
        );
    }

    #[test]
    fn a_right_click_opens_the_same_menu() {
        assert_eq!(
            event(WM_CONTEXTMENU, AT_100_200),
            event(NIN_SELECT, AT_100_200)
        );
    }

    #[test]
    fn choosing_the_icon_with_the_keyboard_opens_the_menu() {
        assert_eq!(
            event(NIN_SELECT | NINF_KEY, AT_100_200),
            Event::Menu { x: 100, y: 200 }
        );
    }

    #[test]
    fn an_icon_left_of_or_above_the_primary_monitor_has_negative_coordinates() {
        // x = -10, y = -20 を 16 ビットずつ詰めたもの
        let wparam = (0xFFECusize << 16) | 0xFFF6;
        assert_eq!(event(NIN_SELECT, wparam), Event::Menu { x: -10, y: -20 });
    }

    #[test]
    fn a_double_click_does_nothing() {
        const WM_LBUTTONDBLCLK: u32 = 0x0203;
        assert_eq!(event(WM_LBUTTONDBLCLK, AT_100_200), Event::Other);
    }

    #[test]
    fn a_click_on_a_notification_is_told_apart_from_the_menu() {
        assert_eq!(
            event(NIN_BALLOONUSERCLICK, AT_100_200),
            Event::NotificationClick
        );
    }
}
