//! コマンドの Win32 の部分（Windows だけ）。判断は Win32 によらないモジュール（`guard`、`wait`、
//! `list_view`、`test_env`）に任せ、ここはウィンドウやプロセスから値を集めて渡すだけにする。

use std::ffi::c_void;
use std::fs;
use std::io::{Read, Write as _};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Child, Command as Process, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use shunsoku_core::keys::Chord;
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, HWND, LPARAM, POINT, RECT,
    SetHandleInformation, WPARAM,
};
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_OPERATION, PROCESS_VM_READ,
    PROCESS_VM_WRITE, TerminateProcess,
};
use windows::Win32::UI::Controls::{
    HDM_GETITEMCOUNT, HKM_GETHOTKEY, LVCF_TEXT, LVCOLUMNW, LVITEMW, LVM_GETCOLUMNW,
    LVM_GETCOLUMNWIDTH, LVM_GETHEADER, LVM_GETITEMCOUNT, LVM_GETITEMTEXTW, LVM_GETSTRINGWIDTHW,
    TCIF_TEXT, TCITEMW, TCM_GETCURSEL, TCM_GETITEMCOUNT, TCM_GETITEMRECT, TCM_GETITEMW,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, IsWindowEnabled, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEINPUT, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::Shell::{NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BringWindowToTop, CB_GETCURSEL, CB_GETLBTEXT, CB_GETLBTEXTLEN, EnumChildWindows,
    EnumWindows, FindWindowExW, GA_ROOT, GUITHREADINFO, GetAncestor, GetClassNameW, GetDlgCtrlID,
    GetForegroundWindow, GetGUIThreadInfo, GetMenuItemCount, GetMenuState, GetMenuStringW,
    GetSubMenu, GetWindowRect, GetWindowThreadProcessId, HMENU, HWND_MESSAGE, IsWindow,
    IsWindowVisible, MF_BYPOSITION, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MN_GETHMENU,
    PostMessageW, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SendMessageTimeoutW,
    SetCursorPos, SetForegroundWindow, SetWindowPos, WM_APP, WM_CONTEXTMENU, WM_GETTEXT,
    WM_GETTEXTLENGTH, WindowFromPoint,
};
use windows::core::{BOOL, PCWSTR};

use crate::cli::{Button, Command, FocusFilter, Invocation, WindowFilter};
use crate::guard::{self, Launched, Seen};
use crate::list_view::{self, Cell};
use crate::test_env::test_env;
use crate::wait::{self, RealClock, TimedOut};

/// コマンドの失敗。終了コードを決める。
pub enum Failure {
    /// 失敗・時間切れ（終了コード 1）。
    Failed(String),
    /// 安全の決まりで止めた（終了コード 3）。
    Refused(String),
}

type Result<T> = std::result::Result<T, Failure>;

fn failed(e: impl std::fmt::Display) -> Failure {
    Failure::Failed(e.to_string())
}

fn timed_out(what: &str) -> impl FnOnce(TimedOut) -> Failure + '_ {
    move |TimedOut| Failure::Failed(format!("時間切れ: {what}"))
}

/// 本体のメッセージ専用ウィンドウのクラス名（`crates/shunsoku/src/app.rs` の `CLASS_NAME`）。
const APP_CLASS: &str = "shunsoku.app";
/// 本体のトレイの通知のメッセージ（`app.rs` の `WM_APP_TRAY`）と、アイコンの ID（`tray.rs` の `ICON_ID`）。
const WM_APP_TRAY: u32 = WM_APP + 1;
const TRAY_ICON_ID: u32 = 1;
/// 他のプロセスのウィンドウに送るメッセージの待ちの上限。
const SEND_TIMEOUT_MS: u32 = 2000;
/// `GetExitCodeProcess` の動いている印。
const STILL_ACTIVE: u32 = 259;

pub fn run(inv: Invocation) -> Result<()> {
    // SAFETY: プロセスの DPI の扱いを決めるだけ。失敗しても（manifest などで決まっていても）続ける
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let root = inv.root.as_str();
    match inv.command {
        Command::Launch {
            portable,
            exe,
            args,
        } => {
            let child = spawn(root, portable, &exe, &args, false)?;
            println!("pid {}", child.id());
            Ok(())
        }
        Command::Run {
            portable,
            timeout,
            exe,
            args,
        } => run_to_end(root, portable, &exe, &args, timeout),
        Command::Kill { pid } => kill(root, pid),
        Command::Pids => {
            for pid in targets(root) {
                println!("{pid}");
            }
            Ok(())
        }
        Command::WaitWindow { filter, timeout } => {
            let h = wait::wait_for(&mut RealClock::new(), timeout, || {
                top_windows().into_iter().find(|&h| matches(h, &filter))
            })
            .map_err(timed_out("ウィンドウが出ない"))?;
            println!("{}", describe(h));
            Ok(())
        }
        Command::WaitGone { filter, timeout } => {
            wait::wait_for(&mut RealClock::new(), timeout, || {
                (!top_windows().into_iter().any(|h| matches(h, &filter))).then_some(())
            })
            .map_err(timed_out("ウィンドウが消えない"))
        }
        Command::WaitExit { pid, timeout } => {
            let code = wait::wait_for(&mut RealClock::new(), timeout, || match exit_code(pid) {
                Some(STILL_ACTIVE) => None,
                code => Some(code),
            })
            .map_err(timed_out("プロセスが終わらない"))?;
            match code {
                Some(c) => println!("exit {c}"),
                None => println!("exit (もうない)"),
            }
            Ok(())
        }
        Command::WaitLog {
            path,
            needle,
            skip,
            timeout,
        } => {
            let line = wait::wait_for(&mut RealClock::new(), timeout, || {
                let content = read_lossy(&path)?;
                wait::find_line(&content, skip, &needle).map(str::to_owned)
            })
            .map_err(timed_out("ログに行が出ない"))?;
            println!("{line}");
            Ok(())
        }
        Command::Lines { path } => {
            let n = read_lossy(&path).map_or(0, |c| wait::complete_lines(&c).count());
            println!("{n}");
            Ok(())
        }
        Command::WaitFile {
            path,
            since,
            timeout,
        } => {
            let (mtime, size) = wait::wait_for(&mut RealClock::new(), timeout, || {
                let meta = fs::metadata(&path).ok()?;
                let mtime = unix_ms(meta.modified().ok()?);
                since
                    .is_none_or(|s| mtime > s)
                    .then_some((mtime, meta.len()))
            })
            .map_err(timed_out("ファイルが書かれない"))?;
            println!("mtime {mtime} size {size}");
            Ok(())
        }
        Command::Now => {
            println!("{}", unix_ms(SystemTime::now()));
            Ok(())
        }
        Command::WaitFocus { filter, timeout } => {
            let h = wait::wait_for(&mut RealClock::new(), timeout, || {
                focused().filter(|&h| focus_matches(h, &filter))
            })
            .map_err(timed_out("フォーカスが移らない"))?;
            println!("focus id={} {}", dlg_id(h), describe(h));
            Ok(())
        }
        Command::Wins { pid } => {
            for h in top_windows() {
                if pid.is_none_or(|p| pid_of(h) == p) {
                    println!("{}", describe(h));
                }
            }
            Ok(())
        }
        Command::Fg => {
            // SAFETY: 引数のない問い合わせ
            let fg = unsafe { GetForegroundWindow() };
            println!("fg {}", describe(fg));
            if let Some(f) = focused() {
                println!("focus id={} {}", dlg_id(f), describe(f));
            }
            Ok(())
        }
        Command::Kids { hwnd } => {
            let parent = window(hwnd)?;
            println!("dpi {}", dpi(parent));
            for c in child_windows(parent) {
                println!(
                    "{:>5} {} \"{}\" vis={} en={}{}",
                    dlg_id(c),
                    describe_short(c),
                    text_of(c).replace("\r\n", "\\n"),
                    visible(c) as u8,
                    // SAFETY: ハンドルを渡すだけの問い合わせ
                    unsafe { IsWindowEnabled(c) }.as_bool() as u8,
                    control_state(c),
                );
            }
            Ok(())
        }
        Command::Text { hwnd } => {
            println!("{}", text_of(window(hwnd)?));
            Ok(())
        }
        Command::Menus => {
            let mut any = false;
            for h in top_windows() {
                if class_of(h) == "#32768" {
                    any = true;
                    println!("menu {}", describe(h));
                    let menu = HMENU(send(h, MN_GETHMENU, 0, 0) as *mut c_void);
                    dump_menu(menu, 1);
                }
            }
            if any {
                Ok(())
            } else {
                Err(failed("開いているメニューがない"))
            }
        }
        Command::Tabs { hwnd } => tabs_report(window(hwnd)?),
        Command::ListView { hwnd, pad } => list_view_report(window(hwnd)?, pad),
        Command::Activate { hwnd } => activate(target_window(root, hwnd)?),
        Command::Move { hwnd, x, y } => {
            let h = target_window(root, hwnd)?;
            // SAFETY: テスト対象のウィンドウを動かすだけ（大きさと重なりは変えない）
            unsafe {
                SetWindowPos(
                    h,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .map_err(failed)?;
            println!("{} dpi={}", describe(h), dpi(h));
            Ok(())
        }
        Command::Keys { chords } => {
            for chord in chords {
                guard_input(root, None)?;
                send_chord(chord);
            }
            Ok(())
        }
        Command::Type { text } => {
            for unit in text.encode_utf16() {
                guard_input(root, None)?;
                send_unicode(unit);
            }
            Ok(())
        }
        Command::Click { x, y, button } => {
            // SAFETY: カーソルを動かすだけ。入力はこの後の確かめが通ってから送る
            unsafe { SetCursorPos(x, y) }.map_err(failed)?;
            guard_input(root, Some(POINT { x, y }))?;
            send_click(button);
            Ok(())
        }
        Command::TrayMenu { pid } => tray_menu(root, pid),
    }
}

// --- 起動と停止 ---

/// テスト用の環境を付けて起動し、PID を控える。起動はここを通す（付けずに起動する道を作らない）。
fn spawn(root: &str, portable: bool, exe: &str, args: &[String], piped: bool) -> Result<Child> {
    let env = test_env(root, portable);
    for (key, value) in &env {
        if matches!(*key, "APPDATA" | "TEMP") {
            fs::create_dir_all(value).map_err(|e| failed(format!("{value} を作れない: {e}")))?;
        }
    }
    let dir = Path::new(exe).parent().unwrap_or(Path::new("."));
    let mut cmd = Process::new(exe);
    cmd.current_dir(dir)
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::null());
    if piped {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    // 子に WSL とつながった標準の入出力を継がせない。継がせると、子が生きている間 WSL 側の呼び出しが終わらない
    for std in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: 自分の標準ハンドルの継承の印を外すだけ
        unsafe {
            if let Ok(h) = GetStdHandle(std) {
                let _ = SetHandleInformation(h, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
            }
        }
    }
    let child = cmd
        .spawn()
        .map_err(|e| failed(format!("{exe} を起動できない: {e}")))?;
    let handle = HANDLE(child.as_raw_handle());
    let launched = Launched {
        pid: child.id(),
        created: created_of(handle).ok_or_else(|| failed("起動した時刻を読めない"))?,
    };
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(launched_path(root))
        .and_then(|mut f| f.write_all(launched.to_line().as_bytes()))
        .map_err(|e| failed(format!("PID を控えられない: {e}")))?;
    Ok(child)
}

fn run_to_end(
    root: &str,
    portable: bool,
    exe: &str,
    args: &[String],
    timeout: Duration,
) -> Result<()> {
    let started = std::time::Instant::now();
    let mut child = spawn(root, portable, exe, args, true)?;
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let out_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out.read_to_end(&mut v);
        v
    });
    let err_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let status = wait::wait_for(&mut RealClock::new(), timeout, || {
        child.try_wait().ok().flatten()
    });
    let status = match status {
        Ok(s) => s,
        Err(TimedOut) => {
            // 自分が起動したばかりの子なので止めてよい
            let _ = child.kill();
            return Err(failed(format!("時間切れ: {exe} が終わらない（止めた）")));
        }
    };
    let out = out_reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    println!(
        "exit {} pid {} ms {}",
        status.code().unwrap_or(-1),
        child.id(),
        started.elapsed().as_millis()
    );
    println!("--stdout--");
    print!("{}", String::from_utf8_lossy(&out));
    println!("--stderr--");
    print!("{}", String::from_utf8_lossy(&err));
    Ok(())
}

fn launched_path(root: &str) -> String {
    format!(r"{}\launched.txt", root.trim_end_matches(['\\', '/']))
}

/// 自分が起動して、まだ動いているプロセス（PID と起動した時刻が控えと合うもの）。
fn targets(root: &str) -> Vec<u32> {
    let content = fs::read_to_string(launched_path(root)).unwrap_or_default();
    guard::parse_launched(&content)
        .into_iter()
        .filter(|l| {
            let h = open(l.pid, PROCESS_QUERY_LIMITED_INFORMATION);
            h.as_ref().is_some_and(|h| {
                created_of(h.0) == Some(l.created) && exit_code_of(h.0) == Some(STILL_ACTIVE)
            })
        })
        .map(|l| l.pid)
        .collect()
}

fn kill(root: &str, pid: u32) -> Result<()> {
    if !targets(root).contains(&pid) {
        return Err(Failure::Refused(format!(
            "PID {pid} は自分が起動して動いているプロセスでないので止めない"
        )));
    }
    let h = open(pid, PROCESS_TERMINATE).ok_or_else(|| failed("プロセスを開けない"))?;
    // SAFETY: 開いたハンドルを渡す
    unsafe { TerminateProcess(h.0, 1) }.map_err(failed)?;
    println!("killed {pid}");
    Ok(())
}

/// 閉じ忘れないプロセスのハンドル。
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: OpenProcess で開いたハンドル
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open(
    pid: u32,
    access: windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS,
) -> Option<Owned> {
    // SAFETY: 問い合わせのみ。返ったハンドルは Owned が閉じる
    unsafe { OpenProcess(access, false, pid) }.ok().map(Owned)
}

fn created_of(h: HANDLE) -> Option<u64> {
    let (mut c, mut e, mut k, mut u) = Default::default();
    // SAFETY: 開いたハンドルと、呼び出しの間生きている構造体
    unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) }.ok()?;
    let c: FILETIME = c;
    Some(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
}

fn exit_code_of(h: HANDLE) -> Option<u32> {
    let mut code = 0;
    // SAFETY: 開いたハンドル
    unsafe { GetExitCodeProcess(h, &mut code) }.ok()?;
    Some(code)
}

/// `pid` の終了コード。プロセスを開けなければ None。
fn exit_code(pid: u32) -> Option<u32> {
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    exit_code_of(h.0)
}

// --- ファイル ---

fn read_lossy(path: &str) -> Option<String> {
    // 本体が書いている途中でも読めるよう、共有で開く（std の既定は読み書きと削除を共有する）
    let bytes = fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

// --- ウィンドウ ---

fn window(raw: isize) -> Result<HWND> {
    let h = HWND(raw as *mut c_void);
    // SAFETY: ハンドルを渡すだけの問い合わせ
    if unsafe { IsWindow(Some(h)) }.as_bool() {
        Ok(h)
    } else {
        Err(failed(format!("ウィンドウがない: 0x{raw:X}")))
    }
}

/// 見えているトップレベルのウィンドウと、本体のメッセージ専用ウィンドウ（EnumWindows に出ない）。
fn top_windows() -> Vec<HWND> {
    let mut v: Vec<HWND> = Vec::new();
    unsafe extern "system" fn push(h: HWND, l: LPARAM) -> BOOL {
        // SAFETY: l は呼び出しの間生きている Vec を指す
        let v = unsafe { &mut *(l.0 as *mut Vec<HWND>) };
        // SAFETY: ハンドルを渡すだけの問い合わせ
        if unsafe { IsWindowVisible(h) }.as_bool() {
            v.push(h);
        }
        true.into()
    }
    // SAFETY: コールバックは列挙の間だけ v を使う
    unsafe {
        let _ = EnumWindows(Some(push), LPARAM(&mut v as *mut Vec<HWND> as isize));
    }
    v.extend(message_windows());
    v
}

fn message_windows() -> Vec<HWND> {
    let class: Vec<u16> = APP_CLASS.encode_utf16().chain([0]).collect();
    let mut v = Vec::new();
    let mut after = None;
    // SAFETY: クラス名は呼び出しの間生きている
    while let Ok(h) =
        unsafe { FindWindowExW(Some(HWND_MESSAGE), after, PCWSTR(class.as_ptr()), None) }
    {
        v.push(h);
        after = Some(h);
    }
    v
}

fn child_windows(parent: HWND) -> Vec<HWND> {
    let mut v: Vec<HWND> = Vec::new();
    unsafe extern "system" fn push(h: HWND, l: LPARAM) -> BOOL {
        // SAFETY: l は呼び出しの間生きている Vec を指す
        unsafe { &mut *(l.0 as *mut Vec<HWND>) }.push(h);
        true.into()
    }
    // SAFETY: コールバックは列挙の間だけ v を使う
    unsafe {
        let _ = EnumChildWindows(
            Some(parent),
            Some(push),
            LPARAM(&mut v as *mut Vec<HWND> as isize),
        );
    }
    v
}

fn matches(h: HWND, f: &WindowFilter) -> bool {
    f.pid.is_none_or(|p| pid_of(h) == p)
        && f.class.as_ref().is_none_or(|c| class_of(h) == *c)
        && f.title
            .as_ref()
            .is_none_or(|t| text_of(h).contains(t.as_str()))
}

fn focus_matches(h: HWND, f: &FocusFilter) -> bool {
    f.pid.is_none_or(|p| pid_of(h) == p)
        && f.class.as_ref().is_none_or(|c| class_of(h) == *c)
        && f.id.is_none_or(|id| dlg_id(h) == id)
        && f.hwnd.is_none_or(|w| h.0 as isize == w)
}

/// 前面のウィンドウのスレッドで、フォーカスのあるコントロール。
fn focused() -> Option<HWND> {
    // SAFETY: 問い合わせのみ。構造体は呼び出しの間生きている
    unsafe {
        let fg = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(fg, None);
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(tid, &mut info).ok()?;
        (!info.hwndFocus.is_invalid()).then_some(info.hwndFocus)
    }
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0;
    // SAFETY: ハンドルを渡すだけの問い合わせ
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: バッファは呼び出しの間生きている
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    let mut result = 0usize;
    // SAFETY: 返事を待つ上限つきで送る。l が指すものは呼び出しの間生きている（呼び出し側の責任）
    unsafe {
        SendMessageTimeoutW(
            h,
            msg,
            WPARAM(w),
            LPARAM(l),
            SMTO_ABORTIFHUNG,
            SEND_TIMEOUT_MS,
            Some(&mut result),
        );
    }
    result as isize
}

/// コントロールやウィンドウの文字（`WM_GETTEXT`。他のプロセスのものもシステムが写す）。
fn text_of(h: HWND) -> String {
    let len = send(h, WM_GETTEXTLENGTH, 0, 0).max(0) as usize;
    let mut buf = vec![0u16; len + 1];
    let n = send(h, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize).max(0) as usize;
    String::from_utf16_lossy(&buf[..n.min(len)])
}

fn dlg_id(h: HWND) -> i32 {
    // SAFETY: ハンドルを渡すだけの問い合わせ
    unsafe { GetDlgCtrlID(h) }
}

fn visible(h: HWND) -> bool {
    // SAFETY: ハンドルを渡すだけの問い合わせ
    unsafe { IsWindowVisible(h) }.as_bool()
}

fn dpi(h: HWND) -> u32 {
    // SAFETY: ハンドルを渡すだけの問い合わせ
    unsafe { GetDpiForWindow(h) }
}

fn rect_of(h: HWND) -> RECT {
    let mut r = RECT::default();
    // SAFETY: 構造体は呼び出しの間生きている
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    r
}

fn describe_short(h: HWND) -> String {
    let r = rect_of(h);
    format!(
        "0x{:X} {} {},{} {}x{}",
        h.0 as isize,
        class_of(h),
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top
    )
}

/// ウィンドウの説明。`0x<hwnd> pid=<pid> class=<クラス> title="<文字>" rect=l,t,r,b (wxh) vis=0|1`。
fn describe(h: HWND) -> String {
    if h.is_invalid() {
        return "(なし)".into();
    }
    let r = rect_of(h);
    format!(
        "0x{:X} pid={} class={} title=\"{}\" rect={},{},{},{} ({}x{}) vis={}",
        h.0 as isize,
        pid_of(h),
        class_of(h),
        text_of(h),
        r.left,
        r.top,
        r.right,
        r.bottom,
        r.right - r.left,
        r.bottom - r.top,
        visible(h) as u8
    )
}

/// 標準のコントロールの状態（チェック、コンボボックスの選択）。
fn control_state(h: HWND) -> String {
    match class_of(h).as_str() {
        "Button" => format!(" check={}", send(h, BM_GETCHECK, 0, 0)),
        "SysTabControl32" => format!(" cur={}", send(h, TCM_GETCURSEL, 0, 0)),
        "msctls_hotkey32" => format!(" hk=0x{:X}", send(h, HKM_GETHOTKEY, 0, 0)),
        "ComboBox" => {
            let sel = send(h, CB_GETCURSEL, 0, 0);
            if sel < 0 {
                return " sel=-1".into();
            }
            let len = send(h, CB_GETLBTEXTLEN, sel as usize, 0).max(0) as usize;
            let mut buf = vec![0u16; len + 1];
            let n = send(h, CB_GETLBTEXT, sel as usize, buf.as_mut_ptr() as isize).max(0);
            format!(
                " sel={sel}:{}",
                String::from_utf16_lossy(&buf[..(n as usize).min(len)])
            )
        }
        _ => String::new(),
    }
}

fn dump_menu(menu: HMENU, depth: usize) {
    // SAFETY: メニューのハンドルを渡すだけの問い合わせ。バッファは呼び出しの間生きている
    unsafe {
        let n = GetMenuItemCount(Some(menu));
        for i in 0..n.max(0) as u32 {
            let mut buf = [0u16; 256];
            let len = GetMenuStringW(menu, i, Some(&mut buf), MF_BYPOSITION);
            let state = GetMenuState(menu, i, MF_BYPOSITION);
            let label = String::from_utf16_lossy(&buf[..len.max(0) as usize]).replace('\t', " | ");
            let mut flags = String::new();
            flags += if state & MF_CHECKED.0 != 0 {
                "[x]"
            } else {
                "[ ]"
            };
            if state & (MF_GRAYED.0 | MF_DISABLED.0) != 0 {
                flags += " GRAY";
            }
            if state & MF_SEPARATOR.0 != 0 {
                flags += " ----";
            }
            println!("{}{i} {flags} {label}", "  ".repeat(depth));
            let sub = GetSubMenu(menu, i as i32);
            if !sub.is_invalid() && depth < 4 {
                dump_menu(sub, depth + 1);
            }
        }
    }
}

// --- リストビュー ---

/// 他のプロセスに置いたバッファ。リストビューのメッセージは、文字や構造体をそのプロセスのメモリで受け渡す。
struct Remote {
    process: Owned,
    ptr: *mut c_void,
}

const REMOTE_SIZE: usize = 8192;

impl Remote {
    fn new(pid: u32) -> Result<Remote> {
        let process = open(
            pid,
            PROCESS_VM_OPERATION | PROCESS_VM_READ | PROCESS_VM_WRITE,
        )
        .ok_or_else(|| failed("一覧のプロセスを開けない"))?;
        // SAFETY: 開いたプロセスにメモリを取る。Drop で返す
        let ptr = unsafe {
            VirtualAllocEx(
                process.0,
                None,
                REMOTE_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if ptr.is_null() {
            return Err(failed("一覧のプロセスにメモリを取れない"));
        }
        Ok(Remote { process, ptr })
    }

    fn at(&self, offset: usize) -> *mut c_void {
        // 取った範囲の中だけを指す
        self.ptr.wrapping_byte_add(offset)
    }

    fn write<T>(&self, offset: usize, value: &T) -> Result<()> {
        assert!(offset + size_of::<T>() <= REMOTE_SIZE);
        // SAFETY: 取った範囲に、生きている値の分だけ写す
        unsafe {
            WriteProcessMemory(
                self.process.0,
                self.at(offset),
                value as *const T as *const c_void,
                size_of::<T>(),
                None,
            )
        }
        .map_err(failed)
    }

    fn read<T: Default>(&self, offset: usize) -> Result<T> {
        assert!(offset + size_of::<T>() <= REMOTE_SIZE);
        let mut value = T::default();
        // SAFETY: 取った範囲から、値の大きさだけ読む
        unsafe {
            ReadProcessMemory(
                self.process.0,
                self.at(offset),
                &mut value as *mut T as *mut c_void,
                size_of::<T>(),
                None,
            )
        }
        .map_err(failed)?;
        Ok(value)
    }

    fn read_wide(&self, offset: usize, units: usize) -> Result<String> {
        let mut buf = vec![0u16; units];
        // SAFETY: 取った範囲から、buf の大きさだけ読む
        unsafe {
            ReadProcessMemory(
                self.process.0,
                self.at(offset),
                buf.as_mut_ptr() as *mut c_void,
                units * 2,
                None,
            )
        }
        .map_err(failed)?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(units);
        Ok(String::from_utf16_lossy(&buf[..end]))
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        // SAFETY: VirtualAllocEx で取ったメモリ
        unsafe {
            let _ = VirtualFreeEx(self.process.0, self.ptr, 0, MEM_RELEASE);
        }
    }
}

/// 構造体の置き場と文字の置き場。
const STRUCT_AT: usize = 0;
const TEXT_AT: usize = 512;
const TEXT_UNITS: usize = (REMOTE_SIZE - TEXT_AT) / 2;

fn list_view_report(list: HWND, pad_96: Option<i32>) -> Result<()> {
    use list_view::Place;
    let remote = Remote::new(pid_of(list))?;
    let header = HWND(send(list, LVM_GETHEADER, 0, 0) as *mut c_void);
    let columns = send(header, HDM_GETITEMCOUNT, 0, 0).max(0) as usize;
    let rows = send(list, LVM_GETITEMCOUNT, 0, 0).max(0) as usize;
    let dpi = dpi(list);
    let pad = |place: Place| match pad_96 {
        Some(p) => list_view::padding(dpi, p),
        None => place.pad(dpi),
    };
    let text_ptr = remote.at(TEXT_AT);

    let measure = |text: String| -> Result<Cell> {
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
        if wide.len() > TEXT_UNITS {
            return Err(failed("文字が長すぎる"));
        }
        for (i, u) in wide.iter().enumerate() {
            remote.write(TEXT_AT + i * 2, u)?;
        }
        let width = send(list, LVM_GETSTRINGWIDTHW, 0, text_ptr as isize) as i32;
        Ok(Cell { text, width })
    };

    println!(
        "dpi {dpi} pad header={} first={} other={} columns {columns} rows {rows}",
        pad(Place::Header),
        pad(Place::FirstColumn),
        pad(Place::OtherColumn)
    );
    let mut clipped = 0;
    let mut report = |what: String, place: Place, column: i32, cell: Cell| {
        let bad = list_view::clipped(column, &cell, pad(place));
        clipped += bad as usize;
        println!(
            "{what} \"{}\" text={} column={column} {}",
            cell.text,
            cell.width,
            if bad { "CLIPPED" } else { "ok" }
        );
    };
    for c in 0..columns {
        let width = send(list, LVM_GETCOLUMNWIDTH, c, 0) as i32;
        let column = LVCOLUMNW {
            mask: LVCF_TEXT,
            pszText: windows::core::PWSTR(text_ptr as *mut u16),
            cchTextMax: TEXT_UNITS as i32,
            ..Default::default()
        };
        remote.write(STRUCT_AT, &column)?;
        send(list, LVM_GETCOLUMNW, c, remote.at(STRUCT_AT) as isize);
        let title = remote.read_wide(TEXT_AT, TEXT_UNITS)?;
        report(format!("header {c}"), Place::Header, width, measure(title)?);
        for r in 0..rows {
            let item = LVITEMW {
                iSubItem: c as i32,
                pszText: windows::core::PWSTR(text_ptr as *mut u16),
                cchTextMax: TEXT_UNITS as i32,
                ..Default::default()
            };
            remote.write(STRUCT_AT, &item)?;
            let n = send(list, LVM_GETITEMTEXTW, r, remote.at(STRUCT_AT) as isize).max(0);
            let text = remote.read_wide(TEXT_AT, (n as usize).min(TEXT_UNITS))?;
            report(
                format!("row {r} col {c}"),
                Place::of_row(c),
                width,
                measure(text)?,
            );
        }
    }
    println!("clipped {clipped}");
    if clipped == 0 {
        Ok(())
    } else {
        Err(failed(format!("{clipped} 個の文字が列に収まらない")))
    }
}

// --- タブ ---

fn tabs_report(tab: HWND) -> Result<()> {
    let remote = Remote::new(pid_of(tab))?;
    let count = send(tab, TCM_GETITEMCOUNT, 0, 0).max(0) as usize;
    let tab_rect = rect_of(tab);
    println!("cur {} count {count}", send(tab, TCM_GETCURSEL, 0, 0));
    for i in 0..count {
        let item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: windows::core::PWSTR(remote.at(TEXT_AT) as *mut u16),
            cchTextMax: TEXT_UNITS as i32,
            ..Default::default()
        };
        remote.write(STRUCT_AT, &item)?;
        send(tab, TCM_GETITEMW, i, remote.at(STRUCT_AT) as isize);
        let text = remote.read_wide(TEXT_AT, TEXT_UNITS)?;
        send(tab, TCM_GETITEMRECT, i, remote.at(STRUCT_AT) as isize);
        let r: RECT = remote.read(STRUCT_AT)?;
        // 項目の矩形はタブのクライアント座標。枠のないコントロールなので、ウィンドウの原点を足せばスクリーン座標
        println!(
            "{i} \"{text}\" center={},{}",
            tab_rect.left + (r.left + r.right) / 2,
            tab_rect.top + (r.top + r.bottom) / 2
        );
    }
    Ok(())
}

// --- 入力 ---

/// テスト対象（自分が起動したプロセス）のウィンドウ。そうでなければ止める。
fn target_window(root: &str, raw: isize) -> Result<HWND> {
    let h = window(raw)?;
    if !targets(root).contains(&pid_of(h)) {
        return Err(Failure::Refused(format!(
            "テスト対象のウィンドウでないので触らない: {}",
            describe(h)
        )));
    }
    Ok(h)
}

/// 入力を送る前の確かめ。前面のウィンドウ（と、クリックならカーソルの下のウィンドウ）がテスト対象でなければ止める。
fn guard_input(root: &str, point: Option<POINT>) -> Result<()> {
    let seen = |h: HWND| Seen {
        pid: pid_of(h),
        describe: describe(h),
    };
    // SAFETY: 問い合わせのみ
    let fg = unsafe { GetForegroundWindow() };
    let under = point.map(|p| {
        // SAFETY: 問い合わせのみ
        let h = unsafe { WindowFromPoint(p) };
        seen(h)
    });
    guard::check_input(&targets(root), &seen(fg), under.as_ref()).map_err(Failure::Refused)
}

fn send_inputs(inputs: &[INPUT]) {
    // SAFETY: 配列は呼び出しの間生きている
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    std::thread::sleep(Duration::from_millis(30));
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

fn send_chord(chord: Chord) {
    let m = chord.modifiers;
    let mut keys: Vec<u16> = [
        (m.win, VK_LWIN),
        (m.ctrl, VK_CONTROL),
        (m.alt, VK_MENU),
        (m.shift, VK_SHIFT),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, vk)| vk.0)
    .collect();
    keys.push(chord.key.vk());
    // 矢印・Home・End・PageUp/Down・Insert・Delete は拡張キー
    let ext = |vk: u16| {
        if (0x21..=0x2E).contains(&vk) {
            KEYEVENTF_EXTENDEDKEY
        } else {
            KEYBD_EVENT_FLAGS(0)
        }
    };
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&vk| key_input(vk, 0, ext(vk)))
        .chain(
            keys.iter()
                .rev()
                .map(|&vk| key_input(vk, 0, ext(vk) | KEYEVENTF_KEYUP)),
        )
        .collect();
    send_inputs(&inputs);
}

fn send_unicode(unit: u16) {
    send_inputs(&[
        key_input(0, unit, KEYEVENTF_UNICODE),
        key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
    ]);
}

fn send_click(button: Button) {
    let (down, up) = match button {
        Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
    };
    let mouse = |flags: MOUSE_EVENT_FLAGS| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    send_inputs(&[mouse(down), mouse(up)]);
}

/// テスト対象のウィンドウを前面にする。前面のスレッドに入力をつないでから `SetForegroundWindow` する。
fn activate(h: HWND) -> Result<()> {
    // SAFETY: 問い合わせと、前面を移す呼び出し。つないだ入力はすぐに外す
    unsafe {
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        let attached = fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
    }
    let root = || {
        // SAFETY: 問い合わせのみ
        unsafe { GetAncestor(GetForegroundWindow(), GA_ROOT) }
    };
    // SAFETY: 問い合わせのみ
    let want = unsafe { GetAncestor(h, GA_ROOT) };
    wait::wait_for(&mut RealClock::new(), Duration::from_secs(1), || {
        (root() == want || pid_of(root()) == pid_of(h)).then_some(())
    })
    .map_err(timed_out("前面にならない"))?;
    // SAFETY: 問い合わせのみ
    println!("fg {}", describe(unsafe { GetForegroundWindow() }));
    Ok(())
}

/// テスト対象の通知領域のアイコンのメニューを開く。本体のメッセージ専用ウィンドウを前面にしてから、
/// アイコンの右クリック（NOTIFYICON_VERSION_4 の `WM_CONTEXTMENU`）を本体に送る。
fn tray_menu(root: &str, pid: u32) -> Result<()> {
    if !targets(root).contains(&pid) {
        return Err(Failure::Refused(format!(
            "PID {pid} は自分が起動したプロセスでないのでメニューを開かない"
        )));
    }
    let owner = message_windows()
        .into_iter()
        .find(|&h| pid_of(h) == pid)
        .ok_or_else(|| failed("本体のウィンドウがない（まだトレイに出ていない？）"))?;
    let id = NOTIFYICONIDENTIFIER {
        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: owner,
        uID: TRAY_ICON_ID,
        ..Default::default()
    };
    // SAFETY: 構造体は呼び出しの間生きている
    let r = unsafe { Shell_NotifyIconGetRect(&id) }
        .map_err(|e| failed(format!("通知領域のアイコンがない: {e}")))?;
    let (x, y) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    let _ = activate(owner);
    let wp = ((y as u32 & 0xFFFF) << 16) | (x as u32 & 0xFFFF);
    let lp = WM_CONTEXTMENU | (TRAY_ICON_ID << 16);
    // SAFETY: テスト対象のウィンドウに送る
    unsafe {
        PostMessageW(
            Some(owner),
            WM_APP_TRAY,
            WPARAM(wp as usize),
            LPARAM(lp as isize),
        )
    }
    .map_err(failed)?;
    let menu = wait::wait_for(&mut RealClock::new(), Duration::from_secs(3), || {
        top_windows()
            .into_iter()
            .find(|&h| class_of(h) == "#32768" && pid_of(h) == pid)
    })
    .map_err(timed_out("メニューが開かない"))?;
    println!("menu {}", describe(menu));
    dump_menu(HMENU(send(menu, MN_GETHMENU, 0, 0) as *mut c_void), 1);
    Ok(())
}
