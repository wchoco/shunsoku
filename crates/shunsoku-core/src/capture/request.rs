//! キャプチャの依頼の入口。
//!
//! 発生源（トレイ、ホットキー、M3 の CLI）ごとの違いは `CaptureRequest::from_source` の中だけで決める。
//! Win32 層の `app` は依頼を受けて、待機（`delay`）→ 取得 → 範囲選択の画面 → 切り抜き → 出力 → 結果、
//! の 1 本の流れで進める。発生源を足すときは `Source` に 1 つ足し、ここで対応する依頼を返す。
//! M3 で CLI からの依頼を足すときは、`Source` と `CaptureRequest::reply` を足す（ADR-0003）。

use std::time::Duration;

use crate::geometry::Rect;
use crate::selection::{Highlight, WindowId};

/// 依頼の発生源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 範囲選択キャプチャのホットキー。
    Hotkey,
    /// トレイのアイコンの左クリック（またはキーボードで選んで Enter / Space）。
    TrayClick,
    /// トレイのメニューの「範囲選択キャプチャ」。
    TrayMenu,
    /// アクティブウィンドウのキャプチャのホットキー。
    ActiveWindowHotkey,
}

/// どう撮るか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMethod {
    /// 範囲選択キャプチャ。範囲選択の画面でユーザーが範囲を選ぶ。
    Select,
    /// ウィンドウキャプチャ。依頼を受けた時点でアクティブなウィンドウを撮る。
    ActiveWindow,
    /// ウィンドウキャプチャ。範囲選択の画面でハイライトから選んだウィンドウ（部品）を撮る。
    /// 範囲選択の画面で選んだあとに `CaptureRequest::for_window` で作る。
    Window(WindowTarget),
}

/// 範囲選択の画面で選んだ、撮るウィンドウ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTarget {
    /// 撮る最上位のウィンドウ。
    pub window: WindowId,
    /// 部品を選んだら、撮った画像から切り抜く範囲（スクリーン座標）。ウィンドウ全体なら None。
    pub crop: Option<Rect>,
}

/// 撮ったものをどこへ出すか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outputs {
    /// 保存画像として保存する先。None なら保存しない。
    pub save: Option<SaveTarget>,
    /// スクラップとして浮かべる。
    pub scrap: bool,
    /// クリップボードにコピーする。
    pub copy: bool,
    /// 出したスクラップにフォーカスを移す。
    pub focus_scrap: bool,
}

/// 保存先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTarget {
    /// 全体設定の保存先とファイル名テンプレート。
    Default,
}

/// 保存に失敗したときにどうするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnSaveFailure {
    /// スクラップを出して、ほかの方法で残せるようにする（ホットキー・トレイ）。
    FallbackToScrap,
    /// 失敗として返す（M3 の CLI）。
    Fail,
}

/// キャプチャの依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    pub method: CaptureMethod,
    pub outputs: Outputs,
    pub on_save_failure: OnSaveFailure,
    /// 撮る前に待つ時間。
    pub delay: Duration,
    /// 結果をトレイ通知で知らせる。
    pub notify: bool,
}

impl CaptureRequest {
    /// 発生源から依頼を作る。
    pub fn from_source(source: Source) -> CaptureRequest {
        match source {
            Source::Hotkey | Source::TrayClick | Source::TrayMenu => {
                CaptureRequest::scrap(CaptureMethod::Select)
            }
            Source::ActiveWindowHotkey => CaptureRequest::scrap(CaptureMethod::ActiveWindow),
        }
    }

    /// 範囲選択の画面でハイライトから選んだウィンドウを撮る依頼。出力・通知・保存の失敗の扱いは
    /// 範囲選択の画面を開いた依頼（`self`）のものを引き継ぐ。待つ時間はもう過ぎたので 0 にする。
    pub fn for_window(&self, chosen: &Highlight) -> CaptureRequest {
        CaptureRequest {
            method: CaptureMethod::Window(WindowTarget {
                window: chosen.window,
                crop: chosen.part.map(|_| chosen.rect),
            }),
            delay: Duration::ZERO,
            ..self.clone()
        }
    }

    /// 範囲選択・ウィンドウのキャプチャ: スクラップを出してフォーカスを移す。保存しない。
    fn scrap(method: CaptureMethod) -> CaptureRequest {
        CaptureRequest {
            method,
            outputs: Outputs {
                save: None,
                scrap: true,
                copy: false,
                focus_scrap: true,
            },
            on_save_failure: OnSaveFailure::FallbackToScrap,
            delay: Duration::ZERO,
            notify: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// 範囲選択キャプチャの依頼（spec #32）: スクラップを出す・フォーカスを移す・保存しない・通知する。
    fn select_capture() -> CaptureRequest {
        CaptureRequest {
            method: CaptureMethod::Select,
            outputs: Outputs {
                save: None,
                scrap: true,
                copy: false,
                focus_scrap: true,
            },
            on_save_failure: OnSaveFailure::FallbackToScrap,
            delay: Duration::ZERO,
            notify: true,
        }
    }

    #[test]
    fn tray_click_requests_a_select_capture_shown_as_a_focused_scrap() {
        let request = CaptureRequest::from_source(Source::TrayClick);
        assert_eq!(request, select_capture());
    }

    #[test]
    fn tray_menu_requests_a_select_capture_shown_as_a_focused_scrap() {
        let request = CaptureRequest::from_source(Source::TrayMenu);
        assert_eq!(request, select_capture());
    }

    #[test]
    fn active_window_hotkey_requests_a_window_capture_shown_as_a_focused_scrap() {
        let request = CaptureRequest::from_source(Source::ActiveWindowHotkey);
        assert_eq!(
            request,
            CaptureRequest {
                method: CaptureMethod::ActiveWindow,
                ..select_capture()
            }
        );
    }

    fn chosen_part() -> Highlight {
        Highlight {
            window: WindowId(7),
            rect: Rect::new(300, 130, 1100, 800),
            part: Some(WindowId(9)),
            name: "エディタ › 本文".to_owned(),
        }
    }

    #[test]
    fn a_window_chosen_on_the_overlay_becomes_a_window_capture_with_the_same_outputs() {
        let request = CaptureRequest::from_source(Source::Hotkey).for_window(&chosen_part());
        assert_eq!(
            request,
            CaptureRequest {
                method: CaptureMethod::Window(WindowTarget {
                    window: WindowId(7),
                    crop: Some(Rect::new(300, 130, 1100, 800)),
                }),
                ..select_capture()
            }
        );
    }

    #[test]
    fn a_whole_window_chosen_on_the_overlay_is_captured_without_cropping() {
        let whole = Highlight {
            part: None,
            rect: Rect::new(100, 100, 1100, 800),
            ..chosen_part()
        };
        let request = CaptureRequest::from_source(Source::TrayMenu).for_window(&whole);
        assert_eq!(
            request.method,
            CaptureMethod::Window(WindowTarget {
                window: WindowId(7),
                crop: None,
            })
        );
    }

    #[test]
    fn a_window_capture_keeps_the_outputs_of_the_request_that_opened_the_overlay() {
        // 範囲選択の画面を開いた依頼の出力（M3 の CLI の `--no-save` なしの保存など）をそのまま使う。
        // 待つ時間はもう過ぎているので 0 にする
        let opened = CaptureRequest {
            outputs: Outputs {
                save: Some(SaveTarget::Default),
                scrap: false,
                copy: true,
                focus_scrap: false,
            },
            on_save_failure: OnSaveFailure::Fail,
            delay: Duration::from_secs(3),
            notify: false,
            ..select_capture()
        };
        let request = opened.for_window(&chosen_part());
        assert_eq!(
            request,
            CaptureRequest {
                method: CaptureMethod::Window(WindowTarget {
                    window: WindowId(7),
                    crop: Some(Rect::new(300, 130, 1100, 800)),
                }),
                delay: Duration::ZERO,
                ..opened
            }
        );
    }

    #[test]
    fn hotkey_requests_a_select_capture_shown_as_a_focused_scrap() {
        let request = CaptureRequest::from_source(Source::Hotkey);
        assert_eq!(request, select_capture());
    }
}
