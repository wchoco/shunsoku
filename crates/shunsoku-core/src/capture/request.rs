//! キャプチャの依頼の入口。
//!
//! 発生源（トレイ、ホットキー、M3 の CLI）ごとの違いは `CaptureRequest::from_source` の中だけで決める。
//! Win32 層の `app` は依頼を受けて、待機（`delay`）→ 取得 → 範囲選択の画面 → 切り出し → 出力 → 結果、
//! の 1 本の流れで進める。発生源を足すときは `Source` に 1 つ足し、ここで対応する依頼を返す。
//! CLI からの依頼は `CaptureRequest::from_cli` で作り、結果を返す先（`reply`）を持たせる（ADR-0003）。
//! 定型領域（ホットキー・トレイ）は領域の設定で決まるので、`CaptureRequest::region` で作る。設定 GUI の
//! 「範囲を選び直す」「試しに撮る」は `reselect_region` / `trial_region`。

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
    /// トレイのメニューの「範囲選択キャプチャ」。
    TrayMenu,
    /// アクティブウィンドウのキャプチャのホットキー。
    ActiveWindowHotkey,
    /// トレイのメニューの「定型領域を追加…」。
    AddRegion,
    /// 白紙スクラップのホットキーと、トレイのメニューの「白紙スクラップ」。
    BlankScrap,
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
        /// モニタ相対の物理ピクセル。モニタからはみ出すかは撮るときに `monitors::locate_numbered` で調べる。
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
    /// 設定 GUI の「範囲を選び直す」。範囲選択の画面を登録のモードで開き、決めた範囲を設定 GUI の写しの定型領域
    /// （`Settings::regions` の添字）に入れる。撮らない。ファイルに書くのは GUI の適用のとき。
    ReselectRegion(usize),
    /// 設定 GUI の「試しに撮る」。写しの定型領域の値（未適用でもよい）で撮る。
    TrialRegion(Box<Region>),
    /// 白紙スクラップを作る。範囲選択の画面で範囲かウィンドウを選び、その位置と大きさだけを使う（画素は使わない）。
    BlankScrap,
}

impl CaptureMethod {
    /// 範囲選択の画面を定型領域の登録のモード（`Selection::registering`）で開くか。
    pub fn registers_region(&self) -> bool {
        matches!(
            self,
            CaptureMethod::RegisterRegion | CaptureMethod::ReselectRegion(_)
        )
    }
}

/// 範囲選択の画面で選んだ、撮るウィンドウ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTarget {
    /// 撮る最上位のウィンドウ。
    pub window: WindowId,
    /// 部品を選んだら、撮った画像から切り出す範囲（スクリーン座標）。ウィンドウ全体なら None。
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
    /// 撮ったスクラップに続けて実行するレシピ（`Settings::recipes` の中の位置。ADR-0010）。キャプチャで始まる
    /// レシピのホットキーからの依頼だけが持つ。
    pub recipe: Option<usize>,
}

impl CaptureRequest {
    /// 発生源から依頼を作る。
    pub fn from_source(source: Source) -> CaptureRequest {
        match source {
            Source::Hotkey | Source::TrayMenu => CaptureRequest::scrap(CaptureMethod::Select),
            Source::ActiveWindowHotkey => CaptureRequest::scrap(CaptureMethod::ActiveWindow),
            Source::BlankScrap => CaptureRequest::scrap(CaptureMethod::BlankScrap),
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

    /// キャプチャで始まるレシピ（`Settings::recipes` の `index`）のホットキー: レシピのキャプチャ（`launch`）で撮り、
    /// スクラップを出す。出したスクラップに続きの操作を実行する。
    ///
    /// - 範囲選択キャプチャ: 範囲選択キャプチャのホットキーと同じく、フォーカスを移す。
    /// - 定型領域キャプチャ: `regions` の中の、選んだ定型領域のホットキーと同じく撮る（保存は定型領域の「保存する」の
    ///   とおり。フォーカスは元のアプリに残す）。ただし「スクラップにする」は見ず、いつもスクラップを出す。定型領域が
    ///   もうなくても名前のまま依頼にする（撮るときに、消えた定型領域として知らせる）。
    pub fn recipe(
        index: usize,
        launch: &crate::recipe::Launch,
        regions: &[Region],
    ) -> CaptureRequest {
        use crate::recipe::Capture;
        let request = match launch.capture() {
            Capture::Select => CaptureRequest::scrap(CaptureMethod::Select),
            Capture::Region => {
                let name = launch.region().unwrap_or_default();
                match regions.iter().find(|r| r.name == name) {
                    Some(region) => CaptureRequest::region(region),
                    None => CaptureRequest::scrap(CaptureMethod::Region(RegionId(name.to_owned()))),
                }
                .with_scrap()
            }
        };
        CaptureRequest {
            recipe: Some(index),
            ..request
        }
    }

    /// いつもスクラップを出す（フォーカスは変えない）。
    fn with_scrap(mut self) -> CaptureRequest {
        self.outputs.scrap = true;
        self.outputs.focus_scrap = false;
        self
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
            delay,
            notify: !output.quiet,
            reply: Some(reply),
            recipe: None,
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
            recipe: None,
        }
    }

    /// 設定 GUI の「範囲を選び直す」: 写しの定型領域 `index` の範囲を、範囲選択の画面で選び直す。
    pub fn reselect_region(index: usize) -> CaptureRequest {
        CaptureRequest {
            method: CaptureMethod::ReselectRegion(index),
            ..CaptureRequest::from_source(Source::AddRegion)
        }
    }

    /// 設定 GUI の「試しに撮る」: 写しの定型領域 `region` の値（範囲・保存先・テンプレート・撮影後の動作）で、
    /// 定型領域キャプチャ（`region`）と同じく撮る。領域に保存先がなければ `general_dir`（写しの全体の保存先。
    /// 欄のプレビューと同じ場所）に保存する。
    pub fn trial_region(region: &Region, general_dir: Option<PathBuf>) -> CaptureRequest {
        let mut request = CaptureRequest {
            method: CaptureMethod::TrialRegion(Box::new(region.clone())),
            ..CaptureRequest::region(region)
        };
        if let Some(SaveTarget::Region { dir, .. }) = &mut request.outputs.save
            && dir.is_none()
        {
            *dir = general_dir;
        }
        request
    }

    /// 定型領域キャプチャ（`Region` / `TrialRegion`）の領域の名前。ほかの依頼は None。
    pub fn region_name(&self) -> Option<&str> {
        match &self.method {
            CaptureMethod::Region(id) => Some(&id.0),
            CaptureMethod::TrialRegion(region) => Some(&region.name),
            _ => None,
        }
    }

    /// 撮る定型領域。名前で指す依頼（`Region`）は適用済みの定型領域 `regions` から引き、試しに撮る依頼
    /// （`TrialRegion`）は持っている値を使う。定型領域の依頼でないか、その名前がなければ None。
    pub fn region_to_capture(&self, regions: &[Region]) -> Option<Region> {
        match &self.method {
            CaptureMethod::Region(id) => regions.iter().find(|r| r.name == id.0).cloned(),
            CaptureMethod::TrialRegion(region) => Some((**region).clone()),
            _ => None,
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
            recipe: None,
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
            recipe: None,
        }
    }

    /// 定型領域 `regions` と、`steps = [...]` のキャプチャで始まるレシピ 1 つの `Launch`。
    fn launch_of(steps: &str, regions: &[Region]) -> crate::recipe::Launch {
        let (settings, issues) = crate::settings::load(&format!(
            "version = 1\n[[recipes]]\nname = \"r\"\nsteps = {steps}\nhotkey = \"Ctrl+Alt+C\"\n"
        ));
        assert_eq!(issues, vec![], "{steps}");
        let checked =
            crate::recipe::plan(&settings.recipes, crate::scrap::ops::OPERATIONS, regions);
        assert_eq!(checked.problems, vec![], "{steps}");
        crate::recipe::Launch::of(&checked.plans[0]).expect("starts with a capture")
    }

    #[test]
    fn a_capture_recipe_hotkey_selects_like_the_select_capture_hotkey_and_names_the_recipe() {
        assert_eq!(
            CaptureRequest::recipe(2, &launch_of(r#"["select_capture"]"#, &[]), &[]),
            CaptureRequest {
                recipe: Some(2),
                ..select_capture()
            }
        );
    }

    #[test]
    fn a_fixed_region_recipe_captures_the_region_saving_as_it_says_and_always_shows_a_scrap() {
        let regions = [region(true, false)];
        let launch = launch_of(r#"[{ op = "capture_region", region = "dash" }]"#, &regions);
        assert_eq!(
            CaptureRequest::recipe(3, &launch, &regions),
            CaptureRequest {
                outputs: Outputs {
                    save: Some(region_save_target()),
                    scrap: true,
                    copy: false,
                    focus_scrap: false,
                },
                recipe: Some(3),
                ..CaptureRequest::region(&regions[0])
            }
        );
        // 保存しない定型領域は保存しない
        let regions = [region(false, true)];
        assert_eq!(
            CaptureRequest::recipe(3, &launch, &regions).outputs.save,
            None
        );
    }

    #[test]
    fn a_fixed_region_recipe_of_a_region_gone_since_still_names_it_to_be_reported() {
        let regions = [region(true, false)];
        let launch = launch_of(r#"[{ op = "capture_region", region = "dash" }]"#, &regions);
        let request = CaptureRequest::recipe(3, &launch, &[]);
        assert_eq!(
            request.method,
            CaptureMethod::Region(RegionId("dash".to_owned()))
        );
        assert_eq!(request.region_to_capture(&[]), None);
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
    fn a_blank_scrap_opens_the_overlay_to_choose_where_and_shows_a_focused_scrap() {
        let request = CaptureRequest::from_source(Source::BlankScrap);
        assert_eq!(
            request,
            CaptureRequest {
                method: CaptureMethod::BlankScrap,
                ..select_capture()
            }
        );
        assert!(!request.method.registers_region());
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
                recipe: None,
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
        for source in [Source::Hotkey, Source::TrayMenu] {
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
            blank_scrap: false,
            loaded_name: None,
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
                recipe: None,
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

    /// spec #36: 「範囲を選び直す」は、定型領域の登録と同じく範囲選択の画面を登録のモードで開く。撮らない。
    #[test]
    fn reselecting_a_region_opens_the_register_mode_without_outputs() {
        let request = CaptureRequest::reselect_region(2);
        assert_eq!(request.method, CaptureMethod::ReselectRegion(2));
        assert!(request.method.registers_region());
        assert!(
            CaptureRequest::from_source(Source::AddRegion)
                .method
                .registers_region()
        );
        assert!(
            !CaptureRequest::from_source(Source::TrayMenu)
                .method
                .registers_region()
        );
        assert_eq!(
            request.outputs,
            Outputs {
                save: None,
                scrap: false,
                copy: false,
                focus_scrap: false,
            }
        );
        assert_eq!(request.reply, None);
    }

    /// spec #36: 「試しに撮る」は、写しの領域の値（未適用でも）で、定型領域キャプチャと同じ出力にする。
    #[test]
    fn a_trial_capture_uses_the_regions_values_not_the_applied_settings() {
        let mut draft = region(false, true);
        draft.name = "まだ適用していない".to_owned();
        draft.rect = Rect::new(5, 5, 50, 50);
        let request = CaptureRequest::trial_region(&draft, Some(PathBuf::from(r"E:\draft")));

        assert_eq!(request.outputs, CaptureRequest::region(&draft).outputs);
        assert_eq!(request.region_name(), Some("まだ適用していない"));
        // 適用済みの設定に同じ名前の領域がなくても、別の値の同じ名前の領域があっても、写しの値で撮る
        let mut applied = region(true, false);
        applied.name = draft.name.clone();
        assert_eq!(request.region_to_capture(&[]), Some(draft.clone()));
        assert_eq!(request.region_to_capture(&[applied]), Some(draft));
    }

    #[test]
    fn a_trial_capture_of_a_region_without_a_folder_saves_to_the_drafts_general_folder() {
        let mut draft = region(true, false);
        draft.save_dir = None;
        let request = CaptureRequest::trial_region(&draft, Some(PathBuf::from(r"E:\draft")));
        assert_eq!(
            request.outputs.save,
            Some(SaveTarget::Region {
                dir: Some(PathBuf::from(r"E:\draft")),
                file_name_template: "{name}_{seq}".to_owned(),
                name: "dash".to_owned(),
            })
        );
    }

    #[test]
    fn region_requests_name_their_region_and_others_do_not() {
        assert_eq!(
            CaptureRequest::region(&region(true, false)).region_name(),
            Some("dash")
        );
        assert_eq!(
            CaptureRequest::from_source(Source::TrayMenu).region_name(),
            None
        );
        assert_eq!(CaptureRequest::reselect_region(0).region_name(), None);
    }

    #[test]
    fn a_region_capture_looks_its_region_up_by_name_in_the_applied_settings() {
        let request = CaptureRequest::region(&region(true, false));
        let mut other = region(true, false);
        other.name = "other".to_owned();
        let mut applied = region(false, true);
        applied.rect = Rect::new(1, 1, 2, 2);

        assert_eq!(
            request.region_to_capture(&[other.clone(), applied.clone()]),
            Some(applied)
        );
        assert_eq!(request.region_to_capture(&[other]), None);
        assert_eq!(
            CaptureRequest::from_source(Source::TrayMenu).region_to_capture(&[region(true, true)]),
            None
        );
    }

    fn cli(target: cli_args::Target) -> CaptureRequest {
        CaptureRequest::from_cli(
            &cli_args::Capture {
                target,
                output: cli_args::Output::default(),
            },
            ReplyTo(3),
        )
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
            recipe: None,
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
    fn a_cli_region_capture_keeps_an_explicit_out_or_no_save() {
        for save in [
            cli_args::Save::To("C:\\a.png".to_owned()),
            cli_args::Save::No,
        ] {
            let request = CaptureRequest::from_cli(
                &cli_args::Capture {
                    target: cli_args::Target::Region {
                        name: "dash".to_owned(),
                    },
                    output: cli_args::Output {
                        save,
                        scrap: true,
                        ..cli_args::Output::default()
                    },
                },
                ReplyTo(3),
            );
            assert_eq!(
                request
                    .clone()
                    .with_region_destination(&region(true, false)),
                request
            );
        }
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
