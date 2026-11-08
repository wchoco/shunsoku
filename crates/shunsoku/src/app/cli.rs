//! CLI の依頼（`pipe_server` から届く）を進め、結果を CLI に返す。
//!
//! - 依頼は `App::pipe` の受け口に届き、`WM_APP_PIPE` で起こされたら、進めている依頼がないときに 1 件ずつ
//!   `submit` に渡す（`on_pipe_requests`）。範囲選択の画面を開いている間に届いたものは失敗として返す。
//! - CLI が待つのをやめた（切った・待ちきれなかった）依頼は取りやめる（`abandon_unwaited_capture`）。
//! - `capture rect` は範囲選択の画面を出さずに撮る（`capture_rect`）。出力はどの撮り方とも同じ `output`。
//! - `open` はトレイの「開く…」と同じ `open_scrap_path` で開き、すぐ応答する（`open_for_cli`）。通知は出さない
//!   （わけは CLI の終了コードと標準エラーで伝える）。
//! - 応答は `REPLIER`（`pipe_server::Replier`）から返す。App を借りられないときにも返せるよう、App の外に置く。

use super::*;
use crate::pipe_server::PipeRequest;

/// CLI の依頼に応答する口（`pipe_server`）。App を借りられないときにも応答できるよう、App の外に置く。
pub(super) static REPLIER: OnceLock<pipe_server::Replier> = OnceLock::new();

/// `WM_APP_PIPE`: 届いている CLI の依頼を進める。範囲選択の画面を開いている間に届いたものは失敗として返し、
/// `delay` を待っている間に届いたものは待たせる（終えたら `end_capture` が起こす）。CLI が待つのをやめた
/// 依頼（切った・待ちきれなかった）は取りやめる。App を借りられなければ、少し後に進め直す。
pub(super) fn on_pipe_requests(message_window: HWND) {
    if APP.with(|cell| cell.try_borrow_mut().is_err()) {
        // SAFETY: 自分のウィンドウのタイマー
        if unsafe { SetTimer(Some(message_window), TIMER_PIPE_RETRY, PIPE_RETRY_MS, None) } == 0 {
            log::warn!("could not start the pipe retry timer");
        }
        return;
    }
    abandon_unwaited_capture(message_window);
    loop {
        let next = with_app(|app| {
            if matches!(app.capture, Capture::Waiting(_)) {
                return None;
            }
            let selecting = matches!(app.capture, Capture::Selecting { .. });
            app.pipe
                .as_ref()?
                .next()
                .map(|request| (request, selecting))
        })
        .flatten();
        let Some((request, selecting)) = next else {
            return;
        };
        let request = match request {
            PipeRequest::Capture(request) => request,
            PipeRequest::Open { path, reply } => {
                if !cli_waiting(reply) {
                    log::info!("CLI request {} is no longer waited for; dropped", reply.0);
                } else if selecting {
                    log::info!("CLI open while the overlay is open; rejected");
                    reply_failed(Some(reply), text("cli.selecting_open"));
                } else {
                    open_for_cli(&path, reply);
                }
                continue;
            }
        };
        if let Some(to) = request.reply
            && !cli_waiting(to)
        {
            log::info!("CLI request {} is no longer waited for; dropped", to.0);
            continue;
        }
        if selecting {
            log::info!("CLI request while the overlay is open; rejected");
            reply_failed(request.reply, text("cli.selecting"));
        } else {
            log::info!("capture requested from the CLI: {:?}", request.method);
            submit(request);
        }
    }
}

/// `open`: スクラップファイルを開いて浮かべ、結果を CLI に返す。読めない・正しくない・新しい版は、それぞれの
/// 終了コード（`ErrorKind::from(LoadErrorKind)`）で返す。
fn open_for_cli(path: &Path, to: ReplyTo) {
    log::info!("open requested from the CLI: {}", path.display());
    let response = match open_scrap_path(path) {
        Ok(()) => Response::Opened,
        Err(OpenFailure::Load(e)) => Response::Failed {
            kind: ErrorKind::from(e.kind()),
            message: format!("{} {}", text(e.message_key()), path.display()),
        },
        Err(OpenFailure::Float(reason)) => Response::Failed {
            kind: ErrorKind::CaptureFailed,
            message: format!("{} {reason}", text("cli.float_failed")),
        },
    };
    reply(to, response);
}

/// 進めている依頼の CLI が待つのをやめていたら（切った・待ちきれなかった）、依頼を取りやめる。
/// 範囲選択の画面を開いていれば閉じ、`delay` を待っていればタイマーを止める。
fn abandon_unwaited_capture(message_window: HWND) {
    let abandoned = with_app(|app| {
        let (reply, selecting) = match &app.capture {
            Capture::Idle => return None,
            Capture::Waiting(request) => (request.reply?, false),
            Capture::Selecting { request, .. } => (request.reply?, true),
        };
        (!cli_waiting(reply)).then_some(selecting)
    })
    .flatten();
    match abandoned {
        Some(true) => close_overlay("the CLI stopped waiting"),
        Some(false) => {
            log::info!("the CLI stopped waiting; the capture is abandoned");
            // SAFETY: 自分のウィンドウのタイマーを止める（`delay`・メニューを待つタイマー）
            unsafe {
                let _ = KillTimer(Some(message_window), TIMER_CAPTURE_DELAY);
            }
            end_capture();
        }
        None => {}
    }
}

/// `to` の依頼に、まだ応答を待っている CLI がいるか。
fn cli_waiting(to: ReplyTo) -> bool {
    REPLIER.get().is_none_or(|replier| replier.is_waiting(to))
}

/// `capture rect`: 範囲選択の画面を出さずに、決まった範囲を撮って出力する。
pub(super) fn capture_rect(monitor: Option<u32>, rect: Rect) {
    let started = Instant::now();
    let area = match monitors::locate_numbered(&monitor_list::list(), monitor, rect) {
        Ok(area) => area,
        Err(e) => {
            log::warn!("could not locate {rect:?} on monitor {monitor:?}: {e:?}");
            let reason = match e {
                LocateError::NoSuchMonitor(n) => format!("{}{n}", text("cli.no_such_monitor")),
                LocateError::NoMainMonitor | LocateError::MonitorNotFound => {
                    text("cli.no_main_monitor").to_owned()
                }
                LocateError::OutOfBounds => text("cli.out_of_monitor").to_owned(),
            };
            fail_capture(&reason);
            return;
        }
    };
    // 取得のスレッドは App に触らないので、借りたまま待ってよい
    let Some(grabbed) = with_app(|app| {
        let cursor = app.settings.general.capture_cursor;
        app.gpu.grab(cursor)
    }) else {
        fail_capture(text("job.panicked"));
        return;
    };
    let shots = match grabbed {
        Ok(shots) => shots,
        Err(e) => {
            log::warn!("could not grab the screen: {e}");
            let reason = match e {
                grab::GrabError::Panicked => text("job.panicked").to_owned(),
                e => e.to_string(),
            };
            fail_capture(&reason);
            return;
        }
    };
    let fragment = shots
        .iter()
        .find(|shot| {
            let r = shot.rect;
            Rect::new(r.left, r.top, r.right, r.bottom).intersect(&area) == Some(area)
        })
        .map(|shot| shot.read_back(area));
    drop(shots);
    let image = match fragment {
        Some(Ok(Some(fragment))) if fragment.image.size() == area.size() => fragment.image,
        Some(Err(e)) => {
            log::warn!("could not read back {area:?}: {e}");
            fail_capture(&e.to_string());
            return;
        }
        _ => {
            log::warn!("no readable shot covers {area:?}");
            fail_capture(text("cli.unreadable_monitor"));
            return;
        }
    };
    let request = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Waiting(request) => Some(request),
        other => {
            app.capture = other;
            None
        }
    })
    .flatten();
    end_capture();
    let Some(request) = request else {
        log::warn!("capture request vanished while capturing a rect");
        return;
    };
    log::info!(
        "captured {}x{} at ({}, {}) in {:.1} ms",
        area.width(),
        area.height(),
        area.left,
        area.top,
        started.elapsed().as_secs_f64() * 1000.0
    );
    output(request, area, image);
}

/// 待っている依頼の、結果を返す先（CLI の依頼だけ）。
pub(super) fn waiting_reply() -> Option<ReplyTo> {
    with_app(|app| match &app.capture {
        Capture::Waiting(request) => request.reply,
        _ => None,
    })
    .flatten()
}

/// 待っている依頼を失敗で終える。CLI の依頼なら理由を返す。
fn fail_capture(reason: &str) {
    let (reply_to, notify) = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Waiting(request) => (request.reply, request.notify),
        other => {
            app.capture = other;
            (None, true)
        }
    })
    .unwrap_or((None, true));
    end_capture();
    if notify {
        notify_user(
            notify::Level::Error,
            &format!("{}\n{reason}", text("capture.failed")),
        );
    }
    reply_failed(reply_to, reason);
}

/// CLI の依頼に応答する。
pub(super) fn reply(to: ReplyTo, response: Response) {
    match REPLIER.get() {
        Some(replier) => replier.reply(to, response),
        None => log::warn!("no pipe server to reply to {}", to.0),
    }
}

/// CLI の依頼なら、キャプチャの失敗（終了コード 1）として返す。
pub(super) fn reply_failed(to: Option<ReplyTo>, message: &str) {
    if let Some(to) = to {
        reply(
            to,
            Response::Failed {
                kind: ErrorKind::CaptureFailed,
                message: message.to_owned(),
            },
        );
    }
}

/// CLI の依頼なら、ユーザーが取り消した（終了コード 4）として返す。
pub(super) fn reply_cancelled(to: Option<ReplyTo>) {
    if let Some(to) = to {
        reply(
            to,
            Response::Failed {
                kind: ErrorKind::Cancelled,
                message: text("cli.cancelled").to_owned(),
            },
        );
    }
}
