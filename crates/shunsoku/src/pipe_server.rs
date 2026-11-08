//! `shunsoku-cli` からの依頼を受ける名前付きパイプのサーバ（ADR-0003）。
//!
//! - 受付のスレッドが 1 本、接続ごとに仕事のスレッドが 1 本。仕事のスレッドは依頼を読み、キャプチャなら
//!   `CaptureRequest`（`reply` 付き）にして UI スレッドに渡し（`WM_APP_*` で起こす）、`Replier::reply` で
//!   返ってきた応答を書いて切る。`open` も同じく `PipeRequest::Open` にして UI スレッドに渡す。`monitors` は
//!   UI スレッドを通さずにその場で答える。
//! - 仕事のスレッドのパニックは、その依頼の失敗（終了コード 1）として返す。
//! - 応答を待つ間、CLI が切った（Ctrl+C など）か、待つ時間の上限（`protocol::reply_timeout`）を過ぎたら、
//!   応答の行き先を消して UI スレッドを起こす。UI スレッドは行き先のない依頼を取りやめる
//!   （`Replier::is_waiting`）。上限を過ぎたときは失敗として返す。
//! - パイプを開けるのは現在のユーザーとセッションだけにする:
//!   - 名前にセッションの番号とユーザーの SID を入れる（`protocol::pipe_name`）。
//!   - セキュリティ記述子の DACL は現在のユーザーだけに許し、ネットワークのログオンを拒む。継承はしない。
//!     SACL に本体の整合性レベルの mandatory label（no-read-up / no-write-up）を付け、本体より整合性レベルの
//!     低いプロセスには開けさせない（昇格した本体を、同じユーザーの昇格していないプロセスから使わせない）。
//!   - `PIPE_REJECT_REMOTE_CLIENTS` で別のマシンからの接続を断る。
//!   - 1 つ目のインスタンスを `FILE_FLAG_FIRST_PIPE_INSTANCE` で作り、先に同じ名前を作られていたら始めない。
//!     インスタンスは常に 1 つ以上開いておき、名前が空く隙を作らない。
//!   - 接続してきたクライアントのセッションを `GetNamedPipeClientSessionId` で確かめ、違えば何も読まずに切る
//!     （同じユーザーの別のセッションは DACL では区別できないため）。念のためクライアントのプロセスのユーザーも
//!     確かめる。クライアントのプロセスの整合性レベルも確かめ、本体より低ければ何も読まずに切る。
//! - 届いた `capture` の依頼は、CLI の引数と同じ規則で確かめ直す（`protocol::decode_request`）。`open` のパスも
//!   確かめ直し、絶対パスの `.shunsoku` で、デバイスの名前空間・UNC のパイプでないものだけを UI スレッドに渡す
//!   （`cli_args::check_open`。読むときは `scrap_file::read` が、ふつうのファイルでなければ読まない）。

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use shunsoku_core::capture::request::{CaptureRequest, ReplyTo};
use shunsoku_core::protocol::{self, DecodeError, ErrorKind, MAX_MESSAGE, Request, Response};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM,
    WPARAM,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, FlushFileBuffers, PIPE_ACCESS_DUPLEX,
    ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeClientSessionId, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_MESSAGE, PIPE_WAIT, PeekNamedPipe,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, Result};

use shunsoku_pipe_identity as pipe_identity;

/// 同時に開いておく接続の上限。超えた CLI は空くまで待つ（`WaitNamedPipeW`）。
const MAX_INSTANCES: u32 = 8;
/// インスタンスを作れなかったときに作り直すまでの間隔。
const RETRY: Duration = Duration::from_millis(200);
/// 応答を待つ間に、CLI がまだつながっているか確かめる間隔。
const CLIENT_CHECK: Duration = Duration::from_millis(500);

/// UI スレッドに渡す依頼。
pub enum PipeRequest {
    /// `capture`。
    Capture(CaptureRequest),
    /// `open`: スクラップファイルを開いて浮かべ、`reply` に応答する。
    Open { path: PathBuf, reply: ReplyTo },
}

/// UI スレッドが持つ受け口。
pub struct PipeServer {
    requests: mpsc::Receiver<PipeRequest>,
}

impl PipeServer {
    /// 届いている依頼を 1 件取り出す。
    pub fn next(&self) -> Option<PipeRequest> {
        self.requests.try_recv().ok()
    }
}

/// 応答を依頼の接続に返す口。UI スレッドのどこからでも呼べるように、`App` の外に置く。
#[derive(Clone, Default)]
pub struct Replier {
    routes: Arc<Mutex<HashMap<u64, mpsc::Sender<Response>>>>,
}

impl Replier {
    /// `to` の依頼に応答する。もう待っていなければ（CLI が切ったなど）捨てる。
    pub fn reply(&self, to: ReplyTo, response: Response) {
        let sender = self
            .routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&to.0);
        match sender {
            Some(sender) => {
                let _ = sender.send(response);
            }
            // 切った・待ちきれなかった依頼は行き先を先に消している（`forget`）。そのあとの応答なので普通に起きる
            None => log::debug!("no pipe client is waiting for reply {}; dropped", to.0),
        }
    }

    /// `to` の依頼に、まだ応答を待っている CLI がいるか（切った・待ちきれなかった依頼は取りやめてよい）。
    pub fn is_waiting(&self, to: ReplyTo) -> bool {
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&to.0)
    }

    /// `id` の依頼の応答の行き先を消す。
    fn forget(&self, id: u64) {
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
    }

    fn register(&self, id: u64) -> mpsc::Receiver<Response> {
        let (sender, receiver) = mpsc::channel();
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, sender);
        receiver
    }
}

/// 仕事のスレッドで使う、変わらない値。
struct Shared {
    session: u32,
    /// 現在のユーザーの SID。
    sid: String,
    /// このプロセスの整合性レベル。これより低いクライアントは断る。
    integrity: pipe_identity::Integrity,
    replier: Replier,
    requests: Mutex<mpsc::Sender<PipeRequest>>,
    next_id: AtomicU64,
    /// UI スレッドのメッセージ専用ウィンドウ（HWND は Send でないので番号で持つ）と、起こすメッセージ。
    window: isize,
    message: u32,
    /// 仕事のスレッドがパニックしたときの文言。
    panicked: String,
    /// 応答を待ちきれなかったときの文言。
    timed_out: String,
}

/// セキュリティ記述子。受付のスレッドが終わるまで（プロセスが終わるまで）持つ。
struct Security(PSECURITY_DESCRIPTOR);
// SAFETY: LocalAlloc の領域を指すだけで、受付のスレッドだけが読む
unsafe impl Send for Security {}

/// パイプを作り、受付のスレッドを始める。依頼が届いたとき・依頼を取りやめるときに `window` に `message` を送る。
/// `panicked` と `timed_out` は、仕事のスレッドがパニックしたとき・応答を待ちきれなかったときに返す文言。
pub fn start(
    window: HWND,
    message: u32,
    panicked: String,
    timed_out: String,
) -> Result<(PipeServer, Replier)> {
    let session = pipe_identity::current_session()?;
    let sid = pipe_identity::current_user_sid()?;
    let integrity = pipe_identity::current_integrity()?;
    let name = HSTRING::from(protocol::pipe_name(session, &sid));
    // 保護された DACL: ネットワークのログオンを拒み、現在のユーザーだけに許す（継承しない）。
    // SACL: 本体より整合性レベルの低いプロセスには読み書きさせない
    let sddl = HSTRING::from(format!(
        "D:P(D;;GA;;;NU)(A;;GA;;;{sid}){}",
        integrity.mandatory_label()
    ));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: 文字列は呼び出しの間生きている。記述子はプロセスが終わるまで返さない
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &sddl,
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?;
    }
    let security = Security(descriptor);
    let first = create_instance(&name, &security, true)?;

    let (sender, requests) = mpsc::channel();
    let replier = Replier::default();
    let shared = Arc::new(Shared {
        session,
        sid,
        integrity,
        replier: replier.clone(),
        requests: Mutex::new(sender),
        next_id: AtomicU64::new(1),
        window: window.0 as isize,
        message,
        panicked,
        timed_out,
    });
    let pipe = first.0 as isize;
    let spawned = std::thread::Builder::new()
        .name("pipe-listen".into())
        .spawn(move || listen(HANDLE(pipe as *mut _), name, security, shared));
    if let Err(e) = spawned {
        // SAFETY: 作ったインスタンスを閉じる（スレッドは始まっていない）
        unsafe {
            let _ = CloseHandle(first);
        }
        return Err(windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            e.to_string(),
        ));
    }
    log::info!("pipe server listening (integrity {:#x})", integrity.0);
    Ok((PipeServer { requests }, replier))
}

fn create_instance(name: &HSTRING, security: &Security, first: bool) -> Result<HANDLE> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security.0.0,
        bInheritHandle: false.into(),
    };
    let mut open_mode = FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0);
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let buffer = MAX_MESSAGE as u32;
    // SAFETY: 名前と属性は呼び出しの間生きている
    let pipe = unsafe {
        CreateNamedPipeW(
            name,
            open_mode,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            MAX_INSTANCES,
            buffer,
            buffer,
            0,
            Some(&attributes),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(windows::core::Error::from_thread());
    }
    Ok(pipe)
}

/// 受付のスレッド。接続を待ち、つながったら次のインスタンスを作ってから仕事のスレッドに渡す。
fn listen(mut pipe: HANDLE, name: HSTRING, security: Security, shared: Arc<Shared>) {
    loop {
        // SAFETY: pipe はこのスレッドが持つインスタンス
        let connected = match unsafe { ConnectNamedPipe(pipe, None) } {
            Ok(()) => true,
            Err(e) => e.code() == ERROR_PIPE_CONNECTED.to_hresult(),
        };
        if !connected {
            log::warn!(
                "ConnectNamedPipe failed: {}",
                windows::core::Error::from_thread()
            );
            // SAFETY: 同じインスタンスを切って待ち直す
            unsafe {
                let _ = DisconnectNamedPipe(pipe);
            }
            std::thread::sleep(RETRY);
            continue;
        }
        // 名前が空かないよう、渡す前に次のインスタンスを作っておく
        let next = loop {
            match create_instance(&name, &security, false) {
                Ok(next) => break next,
                Err(e) => {
                    log::warn!("could not create the next pipe instance: {e}");
                    std::thread::sleep(RETRY);
                }
            }
        };
        let handle = pipe.0 as isize;
        let worker_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("pipe-serve".into())
            .spawn(move || serve(HANDLE(handle as *mut _), &worker_shared));
        if let Err(e) = spawned {
            log::error!("could not start a pipe worker: {e}");
            close(pipe);
        }
        pipe = next;
    }
}

/// 1 つの接続に答えて切る。
fn serve(pipe: HANDLE, shared: &Shared) {
    let response = catch_unwind(AssertUnwindSafe(|| answer(pipe, shared))).unwrap_or_else(|_| {
        log::error!("a pipe worker panicked");
        Some(Response::Failed {
            kind: ErrorKind::CaptureFailed,
            message: shared.panicked.clone(),
        })
    });
    if let Some(response) = response {
        let wire = protocol::encode_response(&response);
        // SAFETY: 書き終えてから切る
        unsafe {
            if let Err(e) = WriteFile(pipe, Some(wire.as_bytes()), None, None) {
                log::warn!("could not write the pipe reply: {e}");
            }
            let _ = FlushFileBuffers(pipe);
        }
    }
    close(pipe);
}

fn close(pipe: HANDLE) {
    // SAFETY: このインスタンスを持つのは呼び出し元だけで、ここで 1 回だけ閉じる
    unsafe {
        let _ = DisconnectNamedPipe(pipe);
        let _ = CloseHandle(pipe);
    }
}

/// 依頼を読んで応答を作る。答えずに切るなら None。
fn answer(pipe: HANDLE, shared: &Shared) -> Option<Response> {
    let mut session = u32::MAX;
    // SAFETY: session はこの関数の中で生きている
    if let Err(e) = unsafe { GetNamedPipeClientSessionId(pipe, &mut session) } {
        log::warn!("could not read the pipe client's session: {e}");
        return None;
    }
    if session != shared.session {
        log::warn!("refused a pipe client from session {session}");
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: pid はこの関数の中で生きている
    let user = unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) }
        .and_then(|()| pipe_identity::process_user_sid(pid));
    match user {
        Ok(user) if user == shared.sid => {}
        Ok(_) => {
            log::warn!("refused a pipe client of another user (pid {pid})");
            return None;
        }
        Err(e) => {
            log::warn!("could not read the pipe client's user (pid {pid}): {e}");
            return None;
        }
    }
    match pipe_identity::process_integrity(pid) {
        Ok(client) if shared.integrity.admits(client) => {}
        Ok(client) => {
            log::warn!(
                "refused a pipe client below this process's integrity (pid {pid}, {:#x} < {:#x})",
                client.0,
                shared.integrity.0
            );
            return None;
        }
        Err(e) => {
            log::warn!("could not read the pipe client's integrity (pid {pid}): {e}");
            return None;
        }
    }

    let mut buffer = vec![0u8; MAX_MESSAGE];
    let mut read = 0u32;
    // SAFETY: buffer と read はこの関数の中で生きている
    let text = match unsafe { ReadFile(pipe, Some(&mut buffer), Some(&mut read), None) } {
        Ok(()) => String::from_utf8(buffer[..read as usize].to_vec())
            .map_err(|_| DecodeError::Malformed("not UTF-8".to_owned())),
        Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => {
            Err(DecodeError::Malformed("too large".to_owned()))
        }
        Err(e) => {
            log::warn!("could not read a pipe request: {e}");
            return None;
        }
    };
    let request = match text.and_then(|text| protocol::decode_request(&text)) {
        Ok(request) => request,
        Err(e) => {
            log::warn!("refused a pipe request: {e:?}");
            return Some(Response::refused(&e));
        }
    };
    log::info!("pipe request: {request:?}");
    match request {
        Request::Monitors => Some(Response::Monitors(crate::monitor_list::list())),
        Request::Capture(capture) => relay(pipe, shared, protocol::reply_timeout(&capture), |to| {
            PipeRequest::Capture(CaptureRequest::from_cli(&capture, to))
        }),
        Request::Open(open) => relay(pipe, shared, protocol::OPEN_REPLY_TIMEOUT, |reply| {
            PipeRequest::Open {
                path: PathBuf::from(open.path),
                reply,
            }
        }),
    }
}

/// 依頼を UI スレッドに渡し、`timeout` まで応答を待つ。CLI が切ったら None（答えずに切る）。
fn relay(
    pipe: HANDLE,
    shared: &Shared,
    timeout: Duration,
    request: impl FnOnce(ReplyTo) -> PipeRequest,
) -> Option<Response> {
    let deadline = Instant::now() + timeout;
    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    let replies = shared.replier.register(id);
    let sent = shared
        .requests
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .send(request(ReplyTo(id)))
        .is_ok();
    let response = if sent && wake(shared) {
        match wait_reply(pipe, shared, id, &replies, deadline) {
            Waited::Reply(response) => Some(response),
            Waited::ClientGone => return None,
            Waited::TimedOut => {
                return Some(Response::Failed {
                    kind: ErrorKind::CaptureFailed,
                    message: shared.timed_out.clone(),
                });
            }
        }
    } else {
        shared.replier.forget(id);
        None
    };
    Some(response.unwrap_or_else(|| Response::Failed {
        kind: ErrorKind::CaptureFailed,
        message: shared.panicked.clone(),
    }))
}

/// UI スレッドを起こす（`WM_APP_PIPE`）。
fn wake(shared: &Shared) -> bool {
    // SAFETY: UI スレッドのメッセージ専用ウィンドウに送るだけ。もう壊れていれば失敗するだけ
    unsafe {
        PostMessageW(
            Some(HWND(shared.window as *mut _)),
            shared.message,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .is_ok()
}

/// 応答を待った結果。
enum Waited {
    Reply(Response),
    /// CLI が切った。
    ClientGone,
    /// 待つ時間の上限を過ぎた。
    TimedOut,
}

/// `id` の依頼の応答を待つ。CLI が切ったか、`deadline` を過ぎたら、応答の行き先を消して UI スレッドに
/// 取りやめさせる。UI スレッドがなくなったら（起こらないはず）パニックの文言で返す。
fn wait_reply(
    pipe: HANDLE,
    shared: &Shared,
    id: u64,
    replies: &mpsc::Receiver<Response>,
    deadline: Instant,
) -> Waited {
    loop {
        match replies.recv_timeout(CLIENT_CHECK) {
            Ok(response) => return Waited::Reply(response),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Waited::Reply(Response::Failed {
                    kind: ErrorKind::CaptureFailed,
                    message: shared.panicked.clone(),
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let gone = !client_connected(pipe);
        if !gone && Instant::now() < deadline {
            continue;
        }
        shared.replier.forget(id);
        // 行き先を消す前に応答が届いていれば、それを返す
        if let Ok(response) = replies.try_recv() {
            return Waited::Reply(response);
        }
        wake(shared);
        return if gone {
            log::info!("the pipe client of request {id} has gone; abandoning it");
            Waited::ClientGone
        } else {
            log::warn!("request {id} timed out; abandoning it");
            Waited::TimedOut
        };
    }
}

/// CLI がまだつながっているか（切っていれば、のぞくと ERROR_BROKEN_PIPE などで失敗する）。
fn client_connected(pipe: HANDLE) -> bool {
    // SAFETY: 読まずに、つながりを確かめるだけ
    unsafe { PeekNamedPipe(pipe, None, 0, None, None, None) }.is_ok()
}
