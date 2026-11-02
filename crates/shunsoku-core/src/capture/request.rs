//! キャプチャの依頼の入口。
//!
//! 発生源（トレイ、ホットキー、M3 の CLI）ごとの違いは `CaptureRequest::from_source` の中だけで決める。
//! Win32 層の `app` は依頼を受けて、待機（`delay`）→ 取得 → 範囲選択の画面 → 切り抜き → 出力 → 結果、
//! の 1 本の流れで進める。発生源を足すときは `Source` に 1 つ足し、ここで対応する依頼を返す。
//! CLI からの依頼は `CaptureRequest::from_cli` で作り、結果を返す先（`reply`）を持たせる（ADR-0003）。

use std::path::PathBuf;
use std::time::Duration;

use crate::cli_args;
use crate::geometry::Rect;

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
    /// 決まった範囲（CLI の `capture rect`）。範囲選択の画面は出さない。
    Rect {
        /// `monitors` の番号（1 から）。None ならメインモニタ。
        monitor: Option<u32>,
        /// モニタ相対の物理ピクセル。モニタからはみ出すかは撮るときに `monitors::locate` で調べる。
        rect: Rect,
    },
}

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
    /// 全体設定の保存先とファイル名テンプレート。同名があれば連番を付ける。
    Default,
    /// 決まったファイル（CLI の `--out`）。あれば上書きする。
    File(PathBuf),
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

    /// CLI の `capture` から依頼を作る。既定は保存だけで、ほかの出力はオプションで足す。`--scrap` のときだけ
    /// スクラップにフォーカスを移す。保存に失敗したら、`--scrap` の有無にかかわらずスクラップに逃がさず失敗として返す。
    /// `--quiet` ならトレイ通知を出さない。
    pub fn from_cli(capture: &cli_args::Capture, reply: ReplyTo) -> CaptureRequest {
        let method = match capture.target {
            cli_args::Target::Rect { monitor, rect } => CaptureMethod::Rect { monitor, rect },
        };
        let output = &capture.output;
        let save = match &output.save {
            cli_args::Save::Default => Some(SaveTarget::Default),
            cli_args::Save::To(path) => Some(SaveTarget::File(PathBuf::from(path))),
            cli_args::Save::No => None,
        };
        CaptureRequest {
            method,
            outputs: Outputs {
                save,
                scrap: output.scrap,
                copy: output.copy,
                focus_scrap: output.scrap,
            },
            on_save_failure: OnSaveFailure::Fail,
            delay: Duration::ZERO,
            notify: !output.quiet,
            reply: Some(reply),
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
            reply: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
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
            output: cli_args::Output::default(),
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

    fn cli_outputs(output: cli_args::Output) -> (Outputs, bool) {
        let capture = cli_args::Capture {
            target: cli_args::Target::Rect {
                monitor: None,
                rect: Rect::new(0, 0, 1, 1),
            },
            output,
        };
        let request = CaptureRequest::from_cli(&capture, ReplyTo(1));
        // 出力を何にしても、保存に失敗したらスクラップに逃がさない
        assert_eq!(request.on_save_failure, OnSaveFailure::Fail);
        (request.outputs, request.notify)
    }

    #[test]
    fn cli_scrap_shows_a_focused_scrap_besides_saving() {
        let (outputs, _) = cli_outputs(cli_args::Output {
            scrap: true,
            ..cli_args::Output::default()
        });
        assert_eq!(
            outputs,
            Outputs {
                save: Some(SaveTarget::Default),
                scrap: true,
                copy: false,
                focus_scrap: true,
            }
        );
    }

    #[test]
    fn cli_copy_and_no_save_copy_without_saving_or_moving_focus() {
        let (outputs, _) = cli_outputs(cli_args::Output {
            save: cli_args::Save::No,
            copy: true,
            ..cli_args::Output::default()
        });
        assert_eq!(
            outputs,
            Outputs {
                save: None,
                scrap: false,
                copy: true,
                focus_scrap: false,
            }
        );
    }

    #[test]
    fn cli_out_saves_to_that_file() {
        let (outputs, _) = cli_outputs(cli_args::Output {
            save: cli_args::Save::To(r"C:\shots\a.png".to_owned()),
            ..cli_args::Output::default()
        });
        assert_eq!(
            outputs.save,
            Some(SaveTarget::File(PathBuf::from(r"C:\shots\a.png")))
        );
    }

    #[test]
    fn cli_quiet_turns_off_the_notifications() {
        let (_, notify) = cli_outputs(cli_args::Output::default());
        assert!(notify);
        let (_, notify) = cli_outputs(cli_args::Output {
            quiet: true,
            ..cli_args::Output::default()
        });
        assert!(!notify);
    }

    #[test]
    fn hotkey_and_tray_requests_expect_no_reply() {
        for source in [Source::Hotkey, Source::TrayClick, Source::TrayMenu] {
            assert_eq!(CaptureRequest::from_source(source).reply, None);
        }
    }
}
