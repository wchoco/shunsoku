//! キャプチャの依頼の入口。
//!
//! 発生源（トレイ、ホットキー、M3 の CLI）ごとの違いは `CaptureRequest::from_source` の中だけで決める。
//! Win32 層の `app` は依頼を受けて、待機（`delay`）→ 取得 → 範囲選択の画面 → 切り抜き → 出力 → 結果、
//! の 1 本の流れで進める。発生源を足すときは `Source` に 1 つ足し、ここで対応する依頼を返す。
//! M3 で CLI からの依頼を足すときは、`Source` と `CaptureRequest::reply` を足す（ADR-0003）。

use std::time::Duration;

/// 依頼の発生源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 範囲選択キャプチャのホットキー。
    Hotkey,
    /// トレイのアイコンの左クリック（またはキーボードで選んで Enter / Space）。
    TrayClick,
    /// トレイのメニューの「範囲選択キャプチャ」。
    TrayMenu,
}

/// どう撮るか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMethod {
    /// 範囲選択キャプチャ。範囲選択の画面でユーザーが範囲を選ぶ。
    Select,
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
            Source::Hotkey | Source::TrayClick | Source::TrayMenu => CaptureRequest::select(),
        }
    }

    /// 範囲選択キャプチャ: スクラップを出してフォーカスを移す。保存しない。
    fn select() -> CaptureRequest {
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
    fn hotkey_requests_a_select_capture_shown_as_a_focused_scrap() {
        let request = CaptureRequest::from_source(Source::Hotkey);
        assert_eq!(request, select_capture());
    }
}
