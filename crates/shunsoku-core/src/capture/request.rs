//! キャプチャの依頼の入口。
//!
//! 発生源（トレイ、ホットキー、M3 の CLI）ごとの違いは `CaptureRequest::from_source` の中だけで決める。
//! Win32 層の `app` は依頼を受けて、待機（`delay`）→ 取得 → 範囲選択の画面 → 切り抜き → 出力 → 結果、
//! の 1 本の流れで進める。発生源を足すときは `Source` に 1 つ足し、ここで対応する依頼を返す。
//! M3 で CLI からの依頼を足すときは、`Source` と `CaptureRequest::reply` を足す（ADR-0003）。
//! 定型領域（ホットキー・トレイ）は領域の設定で決まるので、`CaptureRequest::region` で作る。

use std::path::PathBuf;
use std::time::Duration;

use crate::settings::Region;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureMethod {
    /// 範囲選択キャプチャ。範囲選択の画面でユーザーが範囲を選ぶ。
    Select,
    /// 定型領域キャプチャ。撮るときに、その名前の定型領域を設定から引く。
    Region(RegionId),
}

/// 定型領域を指すもの。名前（`settings::Region::name`。重ならない）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionId(pub String);

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
    /// 定型領域の保存先とファイル名テンプレート。`dir` が None なら全体の保存先。`name` は `{name}` の値。
    Region {
        dir: Option<PathBuf>,
        file_name_template: String,
        name: String,
    },
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

    /// 定型領域キャプチャ（ホットキー・トレイ）: 領域の設定に従って保存する・スクラップを出す。スクラップには
    /// フォーカスを移さない（撮ったあとも元のアプリで作業を続けるため）。保存に失敗したらスクラップに逃がす。
    pub fn region(region: &Region) -> CaptureRequest {
        CaptureRequest {
            method: CaptureMethod::Region(RegionId(region.name.clone())),
            outputs: Outputs {
                save: region.save.then(|| SaveTarget::Region {
                    dir: region.save_dir.clone(),
                    file_name_template: region.file_name_template.clone(),
                    name: region.name.clone(),
                }),
                scrap: region.scrap,
                copy: false,
                focus_scrap: false,
            },
            on_save_failure: OnSaveFailure::FallbackToScrap,
            delay: Duration::ZERO,
            notify: true,
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
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::geometry::Rect;

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

    fn region(save: bool, scrap: bool) -> Region {
        Region {
            name: "dash".to_owned(),
            hotkey: None,
            monitor: "M1".to_owned(),
            rect: Rect::new(0, 0, 10, 10),
            save_dir: Some(PathBuf::from(r"D:\dash")),
            file_name_template: "{name}_{seq}".to_owned(),
            save,
            scrap,
        }
    }

    fn region_save_target() -> SaveTarget {
        SaveTarget::Region {
            dir: Some(PathBuf::from(r"D:\dash")),
            file_name_template: "{name}_{seq}".to_owned(),
            name: "dash".to_owned(),
        }
    }

    #[test]
    fn a_region_by_default_is_only_saved_to_its_own_folder_and_notified() {
        let request = CaptureRequest::region(&region(true, false));
        assert_eq!(
            request,
            CaptureRequest {
                method: CaptureMethod::Region(RegionId("dash".to_owned())),
                outputs: Outputs {
                    save: Some(region_save_target()),
                    scrap: false,
                    copy: false,
                    focus_scrap: false,
                },
                on_save_failure: OnSaveFailure::FallbackToScrap,
                delay: Duration::ZERO,
                notify: true,
            }
        );
    }

    #[test]
    fn a_region_shown_as_a_scrap_does_not_take_the_focus() {
        let request = CaptureRequest::region(&region(false, true));
        assert_eq!(
            request.outputs,
            Outputs {
                save: None,
                scrap: true,
                copy: false,
                focus_scrap: false,
            }
        );
        let both = CaptureRequest::region(&region(true, true));
        assert_eq!(both.outputs.save, Some(region_save_target()));
        assert!(both.outputs.scrap && !both.outputs.focus_scrap);
    }

    #[test]
    fn a_region_that_fails_to_save_falls_back_to_a_scrap_without_the_focus() {
        let request = CaptureRequest::region(&region(true, false));
        assert_eq!(request.on_save_failure, OnSaveFailure::FallbackToScrap);
        assert!(!request.outputs.focus_scrap);
    }
}
