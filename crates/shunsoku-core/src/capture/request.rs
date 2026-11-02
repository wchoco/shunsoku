//! キャプチャの依頼の入口。
//!
//! 発生源（トレイ、ホットキー、M3 の CLI）ごとの違いは `CaptureRequest::from_source` の中だけで決める。
//! Win32 層の `app` は依頼を受けて、待機（`delay`）→ 取得 → 範囲選択の画面 → 切り抜き → 出力 → 結果、
//! の 1 本の流れで進める。発生源を足すときは `Source` に 1 つ足し、ここで対応する依頼を返す。
//! CLI からの依頼は `CaptureRequest::from_cli` で作り、結果を返す先（`reply`）を持たせる（ADR-0003）。
//! 定型領域（ホットキー・トレイ）は領域の設定で決まるので、`CaptureRequest::region` で作る。

use std::path::PathBuf;
use std::time::Duration;

use crate::cli_args;
use crate::geometry::Rect;
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
    /// アクティブウィンドウのキャプチャのホットキー。
    ActiveWindowHotkey,
}

/// どう撮るか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureMethod {
    /// 範囲選択キャプチャ。範囲選択の画面でユーザーが範囲を選ぶ。
    Select,
    /// ウィンドウキャプチャ。依頼を受けた時点でアクティブなウィンドウを撮る。
    ActiveWindow,
    /// 決まった範囲（CLI の `capture rect`）。範囲選択の画面は出さない。
    Rect {
        /// `monitors` の番号（1 から）。None ならメインモニタ。
        monitor: Option<u32>,
        /// モニタ相対の物理ピクセル。モニタからはみ出すかは撮るときに `monitors::locate` で調べる。
        rect: Rect,
    },
    /// 定型領域キャプチャ。撮るときに、その名前の定型領域を設定から引く。
    Region(RegionId),
}

/// 定型領域を指すもの。名前（`settings::Region::name`。重ならない）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionId(pub String);

/// 結果を返す先。Win32 層のパイプのサーバ（`pipe_server`）が依頼ごとに振る番号で、応答をその依頼の接続に返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyTo(pub u64);

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
    /// 結果（保存したパス、失敗）を返す先。CLI からの依頼だけが持つ。
    pub reply: Option<ReplyTo>,
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

    /// CLI の `capture` から依頼を作る。既定は保存だけで、保存に失敗したらスクラップに逃がさず失敗として返す。
    pub fn from_cli(capture: &cli_args::Capture, reply: ReplyTo) -> CaptureRequest {
        let method = match &capture.target {
            cli_args::Target::Rect { monitor, rect } => CaptureMethod::Rect {
                monitor: *monitor,
                rect: *rect,
            },
            cli_args::Target::Region { name } => CaptureMethod::Region(RegionId(name.clone())),
            cli_args::Target::Select => CaptureMethod::Select,
            cli_args::Target::Active { .. } => CaptureMethod::ActiveWindow,
        };
        let delay = match capture.target {
            cli_args::Target::Active { delay } => Duration::from_secs(u64::from(delay)),
            _ => Duration::ZERO,
        };
        CaptureRequest {
            method,
            outputs: Outputs {
                save: Some(SaveTarget::Default),
                scrap: false,
                copy: false,
                focus_scrap: false,
            },
            on_save_failure: OnSaveFailure::Fail,
            delay,
            notify: true,
            reply: Some(reply),
        }
    }

    /// 定型領域キャプチャ（ホットキー・トレイ）: 領域の設定に従って保存する・スクラップを出す。スクラップには
    /// フォーカスを移さない（撮ったあとも元のアプリで作業を続けるため）。保存に失敗したらスクラップに逃がす。
    pub fn region(region: &Region) -> CaptureRequest {
        CaptureRequest {
            method: CaptureMethod::Region(RegionId(region.name.clone())),
            outputs: Outputs {
                save: region.save.then(|| region_save_target(region)),
                scrap: region.scrap,
                copy: false,
                focus_scrap: false,
            },
            on_save_failure: OnSaveFailure::FallbackToScrap,
            delay: Duration::ZERO,
            notify: true,
            reply: None,
        }
    }

    /// 撮る定型領域が決まったときに、全体の保存先（`SaveTarget::Default`）への保存を、その領域の保存先と
    /// ファイル名テンプレートへの保存に変える。CLI の `capture region` の依頼に使う。出力（保存する・スクラップを
    /// 出す）は変えない（領域の撮影後の動作は見ない）。定型領域の依頼でなければ、そのまま返す。
    pub fn with_region_destination(mut self, region: &Region) -> CaptureRequest {
        if matches!(self.method, CaptureMethod::Region(_))
            && self.outputs.save == Some(SaveTarget::Default)
        {
            self.outputs.save = Some(region_save_target(region));
        }
        self
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
            reply: None,
        }
    }
}

/// 定型領域の保存先とファイル名テンプレートへの保存。
fn region_save_target(region: &Region) -> SaveTarget {
    SaveTarget::Region {
        dir: region.save_dir.clone(),
        file_name_template: region.file_name_template.clone(),
        name: region.name.clone(),
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
            reply: None,
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

    #[test]
    fn hotkey_requests_a_select_capture_shown_as_a_focused_scrap() {
        let request = CaptureRequest::from_source(Source::Hotkey);
        assert_eq!(request, select_capture());
    }

    /// CLI の既定（spec #34）: 保存だけ・スクラップは出さない・保存に失敗したら失敗として返す・通知は設定に従う。
    #[test]
    fn a_cli_rect_capture_only_saves_and_fails_when_saving_fails() {
        let rect = Rect::new(0, 0, 100, 100);
        let capture = cli_args::Capture {
            target: cli_args::Target::Rect {
                monitor: Some(2),
                rect,
            },
        };
        let request = CaptureRequest::from_cli(&capture, ReplyTo(7));
        assert_eq!(
            request,
            CaptureRequest {
                method: CaptureMethod::Rect {
                    monitor: Some(2),
                    rect,
                },
                outputs: Outputs {
                    save: Some(SaveTarget::Default),
                    scrap: false,
                    copy: false,
                    focus_scrap: false,
                },
                on_save_failure: OnSaveFailure::Fail,
                delay: Duration::ZERO,
                notify: true,
                reply: Some(ReplyTo(7)),
            }
        );
    }

    #[test]
    fn hotkey_and_tray_requests_expect_no_reply() {
        for source in [Source::Hotkey, Source::TrayClick, Source::TrayMenu] {
            assert_eq!(CaptureRequest::from_source(source).reply, None);
        }
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
                reply: None,
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

    fn cli(target: cli_args::Target) -> CaptureRequest {
        CaptureRequest::from_cli(&cli_args::Capture { target }, ReplyTo(3))
    }

    /// CLI の既定の出力: 保存だけ・保存に失敗したら失敗として返す。
    fn cli_saving(method: CaptureMethod, save: SaveTarget, delay: Duration) -> CaptureRequest {
        CaptureRequest {
            method,
            outputs: Outputs {
                save: Some(save),
                scrap: false,
                copy: false,
                focus_scrap: false,
            },
            on_save_failure: OnSaveFailure::Fail,
            delay,
            notify: true,
            reply: Some(ReplyTo(3)),
        }
    }

    #[test]
    fn a_cli_active_capture_waits_the_delay_then_only_saves() {
        assert_eq!(
            cli(cli_args::Target::Active { delay: 3 }),
            cli_saving(
                CaptureMethod::ActiveWindow,
                SaveTarget::Default,
                Duration::from_secs(3)
            )
        );
    }

    #[test]
    fn a_cli_select_capture_only_saves() {
        assert_eq!(
            cli(cli_args::Target::Select),
            cli_saving(CaptureMethod::Select, SaveTarget::Default, Duration::ZERO)
        );
    }

    /// spec #34: CLI で定型領域を撮るときは、領域の撮影後の動作は見ず、保存先とファイル名テンプレートは領域のもの。
    #[test]
    fn a_cli_region_capture_saves_with_the_regions_destination_ignoring_its_outputs() {
        let request = cli(cli_args::Target::Region {
            name: "dash".to_owned(),
        });
        // 領域の撮影後の動作は「スクラップだけ」だが、CLI の既定（保存だけ）に従う
        let request = request.with_region_destination(&region(false, true));
        assert_eq!(
            request,
            cli_saving(
                CaptureMethod::Region(RegionId("dash".to_owned())),
                region_save_target(),
                Duration::ZERO
            )
        );
    }

    #[test]
    fn the_region_destination_leaves_a_hotkey_region_request_as_it_is() {
        let request = CaptureRequest::region(&region(true, true));
        assert_eq!(
            request
                .clone()
                .with_region_destination(&region(false, false)),
            request
        );
    }

    #[test]
    fn a_region_that_fails_to_save_falls_back_to_a_scrap_without_the_focus() {
        let request = CaptureRequest::region(&region(true, false));
        assert_eq!(request.on_save_failure, OnSaveFailure::FallbackToScrap);
        assert!(!request.outputs.focus_scrap);
    }
}
