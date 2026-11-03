//! コマンドの Win32 の部分（Windows だけ）。判断は Win32 によらないモジュール（`guard`、`wait`、
//! `list_view`、`test_env`）に任せ、ここはウィンドウやプロセスから値を集めて渡すだけにする。

use std::ffi::c_void;
use std::fs;
use std::io::{Read, Write as _};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Child, Command as Process, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use shunsoku_core::app_window;
use shunsoku_core::keys::{Chord, KeyStroke, Modifiers};
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, GlobalFree, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, HGLOBAL, HWND,
    LPARAM, POINT, RECT, SetHandleInformation, WPARAM,
};
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, MEM_COMMIT, MEM_RELEASE,
    MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
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
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::{NIN_SELECT, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BringWindowToTop, CB_GETCURSEL, CB_GETLBTEXT, CB_GETLBTEXTLEN, CreateWindowExW,
    DestroyWindow, EnumChildWindows, EnumWindows, FindWindowExW, FindWindowW, GA_ROOT,
    GUITHREADINFO, GetAncestor, GetClassNameW, GetCursorPos, GetDlgCtrlID, GetForegroundWindow,
    GetGUIThreadInfo, GetMenuItemCount, GetMenuState, GetMenuStringW, GetSubMenu, GetWindowRect,
    GetWindowThreadProcessId, HMENU, HWND_MESSAGE, IsWindow, IsWindowVisible, MF_BYPOSITION,
    MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MN_GETHMENU, PostMessageW, SMTO_ABORTIFHUNG,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SendMessageTimeoutW, SetCursorPos,
    SetForegroundWindow, SetWindowPos, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU,
    WM_GETTEXT, WM_GETTEXTLENGTH, WindowFromPoint,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::cli::{Button, Command, FocusFilter, Invocation, WindowFilter};
use crate::guard::{self, IconRect, Kind, Launched, Seen};
use crate::input;
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

/// 本体のトレイの通知のメッセージ（`shunsoku_core::app_window`）。
const WM_APP_TRAY: u32 = WM_APP + app_window::TRAY_MESSAGE_FROM_WM_APP;
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
            for l in alive(root) {
                println!("{}", l.pid);
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
                guard_input(root, &[])?;
                send_strokes(&chord.strokes());
            }
            Ok(())
        }
        Command::Type { text } => {
            for unit in text.encode_utf16() {
                guard_input(root, &[])?;
                send_unicode(unit);
            }
            Ok(())
        }
        Command::HoldKey { chord, duration } => {
            guard_input(root, &[])?;
            let strokes = chord.strokes();
            let (down, up) = strokes.split_at(strokes.len() / 2);
            send_strokes(down);
            std::thread::sleep(duration);
            // 前面が変わっていても離す（押したままにしない）
            send_strokes(up);
            Ok(())
        }
        Command::Click {
            x,
            y,
            button,
            count,
        } => {
            move_cursor(root, POINT { x, y })?;
            let (down, up) = button_flags(button);
            for _ in 0..count {
                send_mouse(&[down, up]);
            }
            Ok(())
        }
        Command::Drag {
            from,
            to,
            button,
            with,
        } => drag(root, from, to, button, with),
        Command::Wheel { x, y, notches } => {
            move_cursor(root, POINT { x, y })?;
            for delta in input::wheel_steps(notches) {
                send_inputs(&[mouse_input(MOUSEEVENTF_WHEEL, delta)]);
            }
            Ok(())
        }
        Command::HoldButton {
            x,
            y,
            duration,
            button,
        } => {
            move_cursor(root, POINT { x, y })?;
            let (down, up) = button_flags(button);
            send_mouse(&[down]);
            std::thread::sleep(duration);
            send_mouse(&[up]);
            Ok(())
        }
        Command::ClipGet => {
            guard_input(root, &[])?;
            clip_get()
        }
        Command::ClipSet { text } => {
            guard_input(root, &[])?;
            clip_set(&text)
        }
        Command::Hotkey { chords } => {
            for chord in chords {
                guard_hotkey(root)?;
                send_strokes(&chord.strokes());
            }
            Ok(())
        }
        Command::TrayClick { pid } => tray_click(root, pid),
        Command::TrayMenu { pid } => tray_menu(root, pid),
    }
}

// --- 起動と停止 ---

/// テスト用の環境を付けて起動し、PID を控える。起動はここを通す（付けずに起動する道を作らない）。
fn spawn(root: &str, portable: bool, exe: &str, args: &[String], piped: bool) -> Result<Child> {
    let kind = guard::exe_kind(exe).map_err(Failure::Refused)?;
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
        kind,
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

/// 自分が起動して、まだ動いているプロセス（PID と起動した時刻が控えと合うもの）。止めてよいもの。
fn alive(root: &str) -> Vec<Launched> {
    let content = fs::read_to_string(launched_path(root)).unwrap_or_default();
    guard::parse_launched(&content)
        .into_iter()
        .filter(|l| {
            let h = open(l.pid, PROCESS_QUERY_LIMITED_INFORMATION);
            h.as_ref().is_some_and(|h| {
                created_of(h.0) == Some(l.created) && exit_code_of(h.0) == Some(STILL_ACTIVE)
            })
        })
        .collect()
}

/// 入力の宛先にしてよいプロセス（自分が起動して動いている本体）。
fn targets(root: &str) -> Vec<u32> {
    alive(root)
        .into_iter()
        .filter(|l| l.kind == Kind::App)
        .map(|l| l.pid)
        .collect()
}

/// `pid` が `allowed` になければ、安全の決まりで止める。`action` は「止め」「メニューを開か」など。
fn own(allowed: &[u32], pid: u32, action: &str) -> Result<()> {
    guard::check_own(allowed, pid, action).map_err(Failure::Refused)
}

fn kill(root: &str, pid: u32) -> Result<()> {
    let alive: Vec<u32> = alive(root).into_iter().map(|l| l.pid).collect();
    own(&alive, pid, "止め")?;
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
    let class = PCWSTR(app_window::APP_CLASS_WIDE.as_ptr());
    let mut v = Vec::new();
    let mut after = None;
    // SAFETY: クラス名は 'static
    while let Ok(h) = unsafe { FindWindowExW(Some(HWND_MESSAGE), after, class, None) } {
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

/// テスト対象（自分が起動した本体）のウィンドウ。そうでなければ止める。
fn target_window(root: &str, raw: isize) -> Result<HWND> {
    let h = window(raw)?;
    own(&targets(root), pid_of(h), "触ら").map_err(|e| match e {
        Failure::Refused(m) => Failure::Refused(format!("{m}: {}", describe(h))),
        other => other,
    })?;
    Ok(h)
}

fn seen(h: HWND) -> Seen {
    Seen {
        pid: pid_of(h),
        class: class_of(h),
        describe: describe(h),
    }
}

fn foreground() -> HWND {
    // SAFETY: 問い合わせのみ
    unsafe { GetForegroundWindow() }
}

/// 入力を送る前の確かめ。前面のウィンドウと、マウスなら `points` それぞれの下のウィンドウがテスト対象でなければ止める。
/// 下のウィンドウは `WindowFromPoint` で座標から見るので、カーソルを動かす前に確かめられる。
fn guard_input(root: &str, points: &[POINT]) -> Result<()> {
    let under: Vec<Seen> = points
        .iter()
        // SAFETY: 問い合わせのみ
        .map(|&p| seen(unsafe { WindowFromPoint(p) }))
        .collect();
    guard::check_input(&targets(root), &seen(foreground()), &under).map_err(Failure::Refused)
}

fn cursor_pos() -> Result<POINT> {
    let mut p = POINT::default();
    // SAFETY: 構造体は呼び出しの間生きている
    unsafe { GetCursorPos(&mut p) }.map_err(failed)?;
    Ok(p)
}

fn set_cursor(p: POINT) -> Result<()> {
    // SAFETY: 座標を渡すだけ
    unsafe { SetCursorPos(p.x, p.y) }.map_err(failed)
}

/// カーソルを `p` へ動かす。動かす前に確かめる。動かした後にもう一度確かめ（ホバーで出るものが覆うことがある）、
/// 断るときはカーソルを元の場所へ戻す。
fn move_cursor(root: &str, p: POINT) -> Result<()> {
    guard_input(root, &[p])?;
    let back = cursor_pos()?;
    set_cursor(p)?;
    std::thread::sleep(Duration::from_millis(50));
    guard_input(root, &[p]).inspect_err(|_| {
        let _ = set_cursor(back);
    })
}

fn drag(
    root: &str,
    from: (i32, i32),
    to: (i32, i32),
    button: Button,
    with: Modifiers,
) -> Result<()> {
    let path: Vec<POINT> = input::drag_path(from, to, input::DRAG_STEPS)
        .into_iter()
        .map(|(x, y)| POINT { x, y })
        .collect();
    // 通る点すべての下を、カーソルを動かす前に確かめる
    guard_input(root, &path)?;
    move_cursor(root, path[0])?;
    let held = modifier_strokes(with);
    let (press, release) = held.split_at(held.len() / 2);
    send_strokes(press);
    let (down, up) = button_flags(button);
    send_mouse(&[down]);
    std::thread::sleep(Duration::from_millis(100));
    for &p in &path[1..] {
        let _ = set_cursor(p);
        // 動いたことを伝える（SetCursorPos だけでは WM_MOUSEMOVE が遅れることがある）
        send_inputs(&[mouse_input(MOUSEEVENTF_MOVE, 0)]);
    }
    std::thread::sleep(Duration::from_millis(150));
    send_mouse(&[up]);
    send_strokes(release);
    Ok(())
}

/// 修飾キーだけを押して離す順（`Chord::strokes` から、修飾キーでないキーを除いたもの）。
fn modifier_strokes(m: Modifiers) -> Vec<KeyStroke> {
    // 組み合わせにはキーが 1 つ要るので F24（0x87）を置き、押す前に取り除く
    const F24: u16 = 0x87;
    let chord = Chord {
        modifiers: m,
        key: shunsoku_core::keys::Key::from_vk(F24).expect("F24 は名前の表にある"),
    };
    chord
        .strokes()
        .into_iter()
        .filter(|k| k.vk != F24)
        .collect()
}

/// ホットキーを送る前の確かめ（`guard::check_hotkey`）。前面がテスト対象でもデスクトップ・タスクバーでもなければ、
/// タスクバーを前面にしてからもう一度確かめる。
fn guard_hotkey(root: &str) -> Result<()> {
    let targets = targets(root);
    if guard::check_hotkey(&targets, &seen(foreground())).is_ok() {
        return Ok(());
    }
    if targets.is_empty() {
        return guard::check_hotkey(&targets, &seen(foreground())).map_err(Failure::Refused);
    }
    // SAFETY: クラス名は 'static
    if let Ok(taskbar) = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) } {
        let _ = activate(taskbar);
    }
    guard::check_hotkey(&targets, &seen(foreground())).map_err(Failure::Refused)
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

/// キーを押す・離す（順は `Chord::strokes`）。
fn send_strokes(strokes: &[KeyStroke]) {
    if strokes.is_empty() {
        return;
    }
    let inputs: Vec<INPUT> = strokes
        .iter()
        .map(|k| {
            let mut flags = KEYBD_EVENT_FLAGS(0);
            if k.up {
                flags |= KEYEVENTF_KEYUP;
            }
            if k.extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            key_input(k.vk, 0, flags)
        })
        .collect();
    send_inputs(&inputs);
}

fn send_unicode(unit: u16) {
    send_inputs(&[
        key_input(0, unit, KEYEVENTF_UNICODE),
        key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
    ]);
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                mouseData: data as u32,
                ..Default::default()
            },
        },
    }
}

fn button_flags(button: Button) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
    match button {
        Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    }
}

fn send_mouse(flags: &[MOUSE_EVENT_FLAGS]) {
    let inputs: Vec<INPUT> = flags.iter().map(|&f| mouse_input(f, 0)).collect();
    send_inputs(&inputs);
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
    own(&targets(root), pid, "メニューを開か")?;
    let owner = tray_owner(pid)?;
    // メニューはアイコンの上に出す。アイコンがあふれた側に隠れていれば、カーソルの位置に出す
    let (x, y) = match icon_rect(owner) {
        Ok(r) => ((r.left + r.right) / 2, (r.top + r.bottom) / 2),
        Err(_) => {
            let p = cursor_pos()?;
            (p.x, p.y)
        }
    };
    let _ = activate(owner);
    post_tray(owner, WM_CONTEXTMENU, x, y)?;
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

/// テスト対象のアイコンの矩形。アイコンがあふれた側（`^` の中）に隠れていると、シェルは E_FAIL を返す。
fn icon_rect(owner: HWND) -> windows::core::Result<RECT> {
    let id = NOTIFYICONIDENTIFIER {
        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: owner,
        uID: app_window::TRAY_ICON_ID,
        ..Default::default()
    };
    // SAFETY: 構造体は呼び出しの間生きている
    unsafe { Shell_NotifyIconGetRect(&id) }
}

/// シェルの代わりに、テスト対象の本体にトレイの通知（NOTIFYICON_VERSION_4 の形）を送る。
fn post_tray(owner: HWND, code: u32, x: i32, y: i32) -> Result<()> {
    let wp = ((y as u32 & 0xFFFF) << 16) | (x as u32 & 0xFFFF);
    let lp = code | (app_window::TRAY_ICON_ID << 16);
    // SAFETY: テスト対象のウィンドウに送る
    unsafe {
        PostMessageW(
            Some(owner),
            WM_APP_TRAY,
            WPARAM(wp as usize),
            LPARAM(lp as isize),
        )
    }
    .map_err(failed)
}

/// テスト対象の本体のメッセージ専用ウィンドウ（トレイのアイコンの持ち主）。
fn tray_owner(pid: u32) -> Result<HWND> {
    message_windows()
        .into_iter()
        .find(|&h| pid_of(h) == pid)
        .ok_or_else(|| failed("本体のウィンドウがない（まだトレイに出ていない？）"))
}

/// テスト対象の通知領域のアイコンを、本当のクリックで左クリックする（シェルが本体に `NIN_SELECT` を送る）。
/// クリックするのは、テスト対象のアイコンの矩形（`Shell_NotifyIconGetRect`）の中心で、その下が通知領域のときだけ。
/// アイコンがあふれた側に隠れていれば、クリックせずに `NIN_SELECT` を本体に送る（出力の `posted` で分かる）。
fn tray_click(root: &str, pid: u32) -> Result<()> {
    own(&targets(root), pid, "クリックし")?;
    let owner = tray_owner(pid)?;
    let r = match icon_rect(owner) {
        Ok(r) => r,
        Err(e) => {
            // あふれた側に隠れていてクリックできない。クリックせず、シェルが送るのと同じ通知を本体に送る
            let p = cursor_pos()?;
            post_tray(owner, NIN_SELECT, p.x, p.y)?;
            println!(
                "posted NIN_SELECT（アイコンが通知領域に出ていないのでクリックしていない: {e}）"
            );
            return Ok(());
        }
    };
    let icon = IconRect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    };
    let under_root = |x: i32, y: i32| {
        // SAFETY: 問い合わせのみ
        seen(unsafe { GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT) })
    };
    let center = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    let (x, y) = guard::tray_click_point(&icon, &under_root(center.0, center.1))
        .map_err(Failure::Refused)?;
    let back = cursor_pos()?;
    set_cursor(POINT { x, y })?;
    std::thread::sleep(Duration::from_millis(50));
    // 動かした後に、アイコンの上がまだ通知領域か確かめる。違えば戻す
    if let Err(m) = guard::tray_click_point(&icon, &under_root(x, y)) {
        let _ = set_cursor(back);
        return Err(Failure::Refused(m));
    }
    send_mouse(&[MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP]);
    println!(
        "clicked {x},{y} icon={},{},{},{}",
        r.left, r.top, r.right, r.bottom
    );
    Ok(())
}

// --- クリップボード ---

const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;

/// クリップボードの形式の名前。標準の形式は表から、登録された形式は `GetClipboardFormatNameW` で読む。
fn format_name(format: u32) -> String {
    let known = match format {
        1 => Some("CF_TEXT"),
        2 => Some("CF_BITMAP"),
        7 => Some("CF_OEMTEXT"),
        CF_DIB => Some("CF_DIB"),
        CF_UNICODETEXT => Some("CF_UNICODETEXT"),
        15 => Some("CF_HDROP"),
        16 => Some("CF_LOCALE"),
        17 => Some("CF_DIBV5"),
        _ => None,
    };
    if let Some(name) = known {
        return name.into();
    }
    let mut buf = [0u16; 256];
    // SAFETY: バッファは呼び出しの間生きている
    let n = unsafe { GetClipboardFormatNameW(format, &mut buf) };
    if n > 0 {
        String::from_utf16_lossy(&buf[..n as usize])
    } else {
        format!("#{format}")
    }
}

/// クリップボードを開いている間。ほかのプロセスが開いていれば、少し待つ。
struct Clipboard;

impl Clipboard {
    fn open(owner: Option<HWND>) -> Result<Clipboard> {
        wait::wait_for(&mut RealClock::new(), Duration::from_secs(1), || {
            // SAFETY: 開いたら Drop で閉じる
            unsafe { OpenClipboard(owner) }.ok()
        })
        .map_err(timed_out("クリップボードを開けない"))?;
        Ok(Clipboard)
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        // SAFETY: open で開いた
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// 形式の一覧と、文字・画像の大きさを出す。
fn clip_get() -> Result<()> {
    let _open = Clipboard::open(None)?;
    let mut formats = Vec::new();
    let mut f = 0;
    loop {
        // SAFETY: 開いている間の問い合わせ
        f = unsafe { EnumClipboardFormats(f) };
        if f == 0 {
            break;
        }
        formats.push(f);
    }
    let names: Vec<String> = formats.iter().map(|&f| format_name(f)).collect();
    println!("formats {}", names.join(","));
    if formats.contains(&CF_UNICODETEXT) {
        let text = with_global(CF_UNICODETEXT, |bytes| {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        })?;
        println!("text \"{}\"", text.replace("\r\n", "\\n"));
    }
    if formats.contains(&CF_DIB) {
        // BITMAPINFOHEADER の biWidth（4 バイト目から）と biHeight（8 バイト目から。負なら上から下）
        let (w, h) = with_global(CF_DIB, |b| {
            let at = |i: usize| {
                b.get(i..i + 4)
                    .map_or(0, |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
            };
            (at(4), at(8).abs())
        })?;
        println!("image {w}x{h}");
    }
    Ok(())
}

/// 開いているクリップボードの `format` のデータを読む。
fn with_global<T>(format: u32, read: impl FnOnce(&[u8]) -> T) -> Result<T> {
    // SAFETY: 開いている間に取り出し、ロックしている間だけ、ロックした大きさの分だけ読む
    unsafe {
        let h = GetClipboardData(format).map_err(failed)?;
        let g = HGLOBAL(h.0);
        let ptr = GlobalLock(g);
        if ptr.is_null() {
            return Err(failed("クリップボードのデータを読めない"));
        }
        let bytes = std::slice::from_raw_parts(ptr as *const u8, GlobalSize(g));
        let value = read(bytes);
        let _ = GlobalUnlock(g);
        Ok(value)
    }
}

/// クリップボードに文字を入れる。持ち主のウィンドウがないと `SetClipboardData` が失敗するので、
/// メッセージ専用のウィンドウを作って持ち主にする（データは道具が終わっても残る）。
fn clip_set(text: &str) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    // SAFETY: 自分のメッセージ専用のウィンドウを作る。終わったら壊す
    let owner = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            None,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
    }
    .map_err(failed)?;
    let result = put_text(owner, &wide);
    // SAFETY: 作ったウィンドウ
    unsafe {
        let _ = DestroyWindow(owner);
    }
    result?;
    println!("set {} 文字", text.chars().count());
    Ok(())
}

fn put_text(owner: HWND, wide: &[u16]) -> Result<()> {
    let _open = Clipboard::open(Some(owner))?;
    // SAFETY: 開いている間に空にし、取ったメモリに文字を写して渡す。渡したメモリはシステムのものになる
    unsafe {
        EmptyClipboard().map_err(failed)?;
        let g = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(failed)?;
        let ptr = GlobalLock(g);
        if ptr.is_null() {
            let _ = GlobalFree(Some(g));
            return Err(failed("メモリをロックできない"));
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, wide.len());
        let _ = GlobalUnlock(g);
        if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(g.0))) {
            let _ = GlobalFree(Some(g));
            return Err(failed(e));
        }
    }
    Ok(())
}
