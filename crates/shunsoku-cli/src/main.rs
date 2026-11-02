//! shunsoku の CLI（ADR-0003）。引数を解釈し、名前付きパイプで常駐本体に依頼して、結果を終了コードで返す。
//!
//! - 標準出力には結果（保存した絶対パス、モニタの一覧）だけを出し、エラーは標準エラーに出す。
//! - 終了コード: 0 成功 / 1 キャプチャの失敗 / 2 引数の誤り / 3 常駐本体に依頼できない（起動を待ちきれない、
//!   バージョンが合わない）/ 4 ユーザーが取り消した。
//! - 常駐本体が動いていなければ、この exe と同じフォルダの `shunsoku.exe` を起動し、パイプができるまで待つ。
//!   起動した常駐本体はそのまま常駐を続ける。

use shunsoku_core::cli_args::{self, Command};
use shunsoku_core::monitors;
use shunsoku_core::protocol::{self, Request, Response};

/// 引数の誤り。
const EXIT_ARGS: i32 = 2;
/// 常駐本体に依頼できない。
const EXIT_UNAVAILABLE: i32 = 3;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Option<Vec<String>> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().ok())
        .collect();
    let Some(args) = args else {
        error("引数に文字として読めないものがあります");
        return EXIT_ARGS;
    };
    let request = match cli_args::parse(&args) {
        Ok(Command::Help) => {
            print!("{}", cli_args::USAGE);
            return 0;
        }
        Ok(Command::Version) => {
            println!("shunsoku-cli {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        Ok(Command::Capture(capture)) => Request::Capture(capture),
        Ok(Command::Monitors) => Request::Monitors,
        Err(e) => {
            error(&e.0);
            eprintln!("使い方は shunsoku-cli --help で確かめられます。");
            return EXIT_ARGS;
        }
    };
    let reply = match pipe::exchange(&protocol::encode_request(&request)) {
        Ok(reply) => reply,
        Err(failure) => {
            error(&failure.message);
            return failure.code;
        }
    };
    let response = match protocol::decode_response(&reply) {
        Ok(response) => response,
        Err(e) => {
            error(&format!(
                "常駐本体の応答を読めません（shunsoku と shunsoku-cli のバージョンが合っていない可能性があります）: {e:?}"
            ));
            return EXIT_UNAVAILABLE;
        }
    };
    match &response {
        Response::Captured { path: Some(path) } => println!("{path}"),
        Response::Captured { path: None } => {}
        Response::Monitors(list) => print!("{}", monitors::listing(list)),
        Response::Failed { message, .. } => error(message),
    }
    response.exit_code()
}

fn error(message: &str) {
    eprintln!("shunsoku-cli: {message}");
}

/// 依頼を返せなかった理由と終了コード。
struct Failure {
    code: i32,
    message: String,
}

impl Failure {
    fn unavailable(message: impl Into<String>) -> Failure {
        Failure {
            code: EXIT_UNAVAILABLE,
            message: message.into(),
        }
    }
}

#[cfg(windows)]
#[path = "../../shunsoku/src/pipe_identity.rs"]
mod pipe_identity;

#[cfg(windows)]
mod pipe {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    use shunsoku_core::protocol::{self, MAX_MESSAGE};
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PIPE_BUSY, GENERIC_READ,
        GENERIC_WRITE, HANDLE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_NONE, OPEN_EXISTING, ReadFile, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT, WriteFile,
    };
    use windows::Win32::System::Pipes::{
        GetNamedPipeServerProcessId, GetNamedPipeServerSessionId, PIPE_READMODE_MESSAGE,
        SetNamedPipeHandleState, WaitNamedPipeW,
    };
    use windows::core::HSTRING;

    use super::{Failure, pipe_identity};

    /// 常駐本体を起動してからパイプができるまで待つ時間の上限。
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
    /// パイプがまだないときに開き直す間隔。
    const POLL: Duration = Duration::from_millis(100);
    /// 起動する常駐本体の exe の名前（この exe と同じフォルダに置く）。
    const RESIDENT_EXE: &str = "shunsoku.exe";
    /// 常駐本体をこのコンソールから切り離す（`DETACHED_PROCESS`）。Ctrl+C を受けないよう別のグループにする
    /// （`CREATE_NEW_PROCESS_GROUP`）。
    const DETACH: u32 = 0x0000_0008 | 0x0000_0200;

    struct Pipe(HANDLE);

    impl Drop for Pipe {
        fn drop(&mut self) {
            // SAFETY: 開いたハンドルを 1 回だけ閉じる
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// 依頼を書いて、応答を読む。
    pub(super) fn exchange(request: &str) -> Result<String, Failure> {
        let session = pipe_identity::current_session()
            .map_err(|e| Failure::unavailable(format!("セッションを調べられません: {e}")))?;
        let sid = pipe_identity::current_user_sid()
            .map_err(|e| Failure::unavailable(format!("ユーザーを調べられません: {e}")))?;
        let name = HSTRING::from(protocol::pipe_name(session, &sid));
        let pipe = connect(&name)?;
        verify_server(&pipe, session, &sid)?;

        // SAFETY: 自分で開いたハンドルのモードを変える
        unsafe { SetNamedPipeHandleState(pipe.0, Some(&PIPE_READMODE_MESSAGE), None, None) }
            .map_err(|e| Failure::unavailable(format!("常駐本体に依頼できません: {e}")))?;
        // SAFETY: 書く中身はこの関数の中で生きている
        unsafe { WriteFile(pipe.0, Some(request.as_bytes()), None, None) }
            .map_err(|e| Failure::unavailable(format!("常駐本体に依頼できません: {e}")))?;
        let mut buffer = vec![0u8; MAX_MESSAGE];
        let mut read = 0u32;
        // SAFETY: buffer と read はこの関数の中で生きている
        match unsafe { ReadFile(pipe.0, Some(&mut buffer), Some(&mut read), None) } {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => {
                return Err(Failure::unavailable("常駐本体の応答が大きすぎます"));
            }
            Err(e) => {
                // 依頼は届いたが応答がない（常駐本体が終わったなど）
                return Err(Failure {
                    code: 1,
                    message: format!("常駐本体から応答がありません: {e}"),
                });
            }
        }
        buffer.truncate(read as usize);
        String::from_utf8(buffer).map_err(|_| Failure::unavailable("常駐本体の応答を読めません"))
    }

    /// パイプを開く。なければ常駐本体を 1 度だけ起動して、できるまで待つ。
    fn connect(name: &HSTRING) -> Result<Pipe, Failure> {
        let mut deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut launched = false;
        loop {
            // SAFETY: 名前は呼び出しの間生きている。SECURITY_IDENTIFICATION で、相手にこのユーザーとして
            // 振る舞わせない（なりすましの範囲を識別だけにする）
            let opened = unsafe {
                CreateFileW(
                    name,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    None,
                )
            };
            let e = match opened {
                Ok(handle) => return Ok(Pipe(handle)),
                Err(e) => e,
            };
            let now = Instant::now();
            if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() {
                if !launched {
                    launch()?;
                    launched = true;
                    deadline = Instant::now() + STARTUP_TIMEOUT;
                } else if now >= deadline {
                    return Err(Failure::unavailable(format!(
                        "常駐本体の起動を {} 秒待ちましたが、依頼を受け付けませんでした",
                        STARTUP_TIMEOUT.as_secs()
                    )));
                }
                std::thread::sleep(POLL);
            } else if e.code() == ERROR_PIPE_BUSY.to_hresult() {
                // ほかの CLI の依頼で埋まっている。空くまで待つ
                let left = deadline.saturating_duration_since(now);
                if left.is_zero() {
                    return Err(Failure::unavailable("常駐本体が混み合っています"));
                }
                // SAFETY: 名前は呼び出しの間生きている。空かずに時間が来たら false を返すだけ
                let _ = unsafe { WaitNamedPipeW(name, left.as_millis() as u32) };
            } else {
                return Err(Failure::unavailable(format!(
                    "常駐本体に接続できません: {e}"
                )));
            }
        }
    }

    /// 相手が同じセッションの同じユーザーのプロセスか確かめる（名前を先に取られていたときのため）。
    fn verify_server(pipe: &Pipe, session: u32, sid: &str) -> Result<(), Failure> {
        let refused = || Failure::unavailable("パイプの相手が常駐本体ではありません");
        let mut server_session = u32::MAX;
        let mut pid = 0u32;
        // SAFETY: 出力はこの関数の中で生きている
        unsafe {
            GetNamedPipeServerSessionId(pipe.0, &mut server_session).map_err(|_| refused())?;
            GetNamedPipeServerProcessId(pipe.0, &mut pid).map_err(|_| refused())?;
        }
        let user = pipe_identity::process_user_sid(pid).map_err(|_| refused())?;
        if server_session != session || user != sid {
            return Err(refused());
        }
        Ok(())
    }

    /// この exe と同じフォルダの常駐本体を起動する。待たずに返る。
    fn launch() -> Result<(), Failure> {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(RESIDENT_EXE)))
            .ok_or_else(|| Failure::unavailable("常駐本体の場所が分かりません"))?;
        std::process::Command::new(&exe)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACH)
            .spawn()
            .map(drop)
            .map_err(|e| {
                Failure::unavailable(format!(
                    "常駐本体（{}）を起動できません: {e}",
                    exe.display()
                ))
            })
    }
}

#[cfg(not(windows))]
mod pipe {
    use super::Failure;

    pub(super) fn exchange(_request: &str) -> Result<String, Failure> {
        Err(Failure::unavailable(
            "shunsoku-cli は Windows でだけ動きます",
        ))
    }
}
