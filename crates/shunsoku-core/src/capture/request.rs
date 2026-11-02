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
use crate::selection::{Highlight, WindowId};
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
    /// トレイのメニューの「定型領域を追加…」。
    AddRegion,
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
    /// ウィンドウキャプチャ。範囲選択の画面でハイライトから選んだウィンドウ（部品）を撮る。
    /// 範囲選択の画面で選んだあとに `CaptureRequest::for_window` で作る。
    Window(WindowTarget),
    /// 定型領域の登録。範囲選択の画面を登録のモード（`Selection::registering`）で開き、決めた範囲を
    /// `[[regions]]` に足す。撮らない。
    RegisterRegion,
}

/// 範囲選択の画面で選んだ、撮るウィンドウ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTarget {
    /// 撮る最上位のウィンドウ。
    pub window: WindowId,
    /// 部品を選んだら、撮った画像から切り抜く範囲（スクリーン座標）。ウィンドウ全体なら None。
    pub crop: Option<Rect>,
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
    /// 全体設定の保存先とファイル名テンプレート。同名があれば連番を付ける。
    Default,
    /// 決まったファイル（CLI の `--out`）。あれば上書きする。
    File(PathBuf),
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
            Source::AddRegion => CaptureRequest {
                outputs: Outputs {
                    save: None,
                    scrap: false,
                    copy: false,
                    focus_scrap: false,
                },
                ..CaptureRequest::scrap(CaptureMethod::RegisterRegion)
            },
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
            reply: None,
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
    fn adding_a_region_opens_the_overlay_to_register_it_and_outputs_nothing() {
        let request = CaptureRequest::from_source(Source::AddRegion);
        assert_eq!(
            request,
            CaptureRequest {
                method: CaptureMethod::RegisterRegion,
                outputs: Outputs {
                    save: None,
                    scrap: false,
                    copy: false,
                    focus_scrap: false,
                },
                ..select_capture()
            }
        );
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

    #[test]
    fn a_region_that_fails_to_save_falls_back_to_a_scrap_without_the_focus() {
        let request = CaptureRequest::region(&region(true, false));
        assert_eq!(request.on_save_failure, OnSaveFailure::FallbackToScrap);
        assert!(!request.outputs.focus_scrap);
    }
}
