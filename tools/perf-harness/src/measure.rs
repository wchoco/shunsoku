//! 本体を起動して、各指標を測る（Windows だけ）。
//!
//! - 本体は、ハーネスが用意した一時フォルダを `APPDATA` にして起動する。そこにハーネスのホットキーだけを書いた
//!   `settings.toml` を置くので、ユーザーの設定にもログにも触れない。本体のログ（`shunsoku.log`）もそこに残る。
//! - 本体は起動時に自動起動の登録を同期するので、`SHUNSOKU_AUTOSTART_VALUE` で Run キーの値の名前を
//!   `shunsoku-perf-harness` に差し替え、ユーザーの登録（`shunsoku`）に触れさせない。
//! - 区切りの時刻は本体が `SHUNSOKU_PERF_LOG` に書き、ハーネスは書かれている途中のログを読む。
//!   QPC はプロセスをまたいで同じ時計なので、ハーネスの QPC の時刻とそのまま比べられる。
//! - Private Working Set は `QueryWorkingSet` で、共有できないページを数える（タスク マネージャーの
//!   「メモリ」、性能カウンターの `Working Set - Private` と同じもの）。`PROCESS_MEMORY_COUNTERS_EX` の
//!   `PrivateUsage` はコミット（Private Bytes）で別物なので使わない。

use std::error::Error;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};

use shunsoku_core::autostart;
use shunsoku_core::keys::{Chord, Key, Modifiers};
use shunsoku_core::perf::{self, Mark, PerfLog, Summary};
use windows::Win32::Foundation::{ERROR_BAD_LENGTH, HANDLE, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DISPLAY_DEVICEW, EnumDisplayDevicesW, GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MONITORINFOEXW, MonitorFromPoint,
};
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::System::ProcessStatus::QueryWorkingSet;
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EDD_GET_DEVICE_INTERFACE_NAME, FindWindowExW, HWND_MESSAGE, PostMessageW, SetCursorPos,
    WM_CLOSE,
};
use windows::core::PCWSTR;

use crate::{Metric, Options};

/// 本体に渡す Run キーの値の名前。本体は起動時に自動起動を同期するので、ユーザーの登録（`shunsoku`）に触れないよう差し替える。
const AUTOSTART_VALUE: &str = "shunsoku-perf-harness";

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// 起動してトレイの登録が終わるまで待つ上限。
const START_TIMEOUT: Duration = Duration::from_secs(10);
/// ホットキーを送ってから範囲選択の画面が出るまで待つ上限。
const OVERLAY_TIMEOUT: Duration = Duration::from_secs(5);
/// 範囲をドラッグしてからスクラップが出るまで待つ上限。
const SCRAP_TIMEOUT: Duration = Duration::from_secs(3);
/// 常駐時のメモリの正式値を読む、トレイの登録からの時間（spec #32）。
const IDLE_AFTER_TRAY: Duration = Duration::from_secs(5);
/// 参考値を読む前に落ち着くのを待つ時間。
const SETTLE: Duration = Duration::from_secs(2);
/// 参考値「撮影 10 回後」の回数。
const CAPTURES: usize = 10;
/// 書き込みの更新で、白紙スクラップにする定型領域に付けるホットキー。`--hotkey` の既定値と重ならないもの。
const ANNOTATE_HOTKEY: &str = "Ctrl+Alt+Shift+F10";
/// 書き込みの更新で送るドラッグの歩数。
const ANNOTATE_STEPS: usize = 200;
/// 書き込みの更新で、1 歩の描き直しが出るまで待つ上限。過ぎたら次の歩に進む（その歩は値に入らない）。
const STEP_TIMEOUT: Duration = Duration::from_secs(1);
/// フリーハンドを選ぶキー（数字の 6、core の `annotate::Tool::from_key`）。
const VK_6: u16 = 0x36;
/// 書き込みの編集モードに入るキー（`E`、スクラップ操作 `annotate` の既定のショートカット）。
const VK_E: u16 = 0x45;

/// 本体のウィンドウのクラス名（`shunsoku_core::app_window`）。
const APP_CLASS: windows::core::PCWSTR =
    windows::core::PCWSTR(shunsoku_core::app_window::APP_CLASS_WIDE.as_ptr());

/// `QueryWorkingSet` の 1 ページの情報のうち「共有できる」のビット。
const PAGE_SHAREABLE: usize = 1 << 8;

const MIB: f64 = 1024.0 * 1024.0;

pub fn run(options: &Options) -> Result<()> {
    // SetCursorPos を物理ピクセルで指定するため
    // SAFETY: 引数は定数
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if running_instance() {
        return Err("shunsoku が常駐しています。終了してから測ってください".into());
    }
    let workspace = Workspace::new(options.hotkey)?;
    println!(
        "{} を {} 回ずつ測ります（ホットキー {}、作業フォルダ {}）",
        options.exe.display(),
        options.runs,
        options.hotkey,
        workspace.dir.display()
    );
    let mut report = Report::default();
    for metric in &options.metrics {
        match metric {
            Metric::Memory => report.memory = Some(measure_memory(options, &workspace)?),
            Metric::Hotkey => report.hotkey = Some(measure_hotkey(options, &workspace)?),
            Metric::ColdStart => report.cold_start = Some(measure_cold_start(options, &workspace)?),
            Metric::Annotate => report.annotate = Some(measure_annotate(options, &workspace)?),
        }
    }
    report.print();
    // 途中で失敗したとき（`?` で戻ったとき）は、本体のログを見られるように作業フォルダを残す
    let _ = std::fs::remove_dir_all(&workspace.dir);
    Ok(())
}

/// 指標ごとの測った値。
#[derive(Default)]
struct Report {
    memory: Option<Memory>,
    hotkey: Option<Hotkey>,
    cold_start: Option<ColdStart>,
    /// 書き込みの更新（ms）。1 回の描き直しごとの値。
    annotate: Option<Vec<f64>>,
}

/// 常駐時のメモリ（MB）。
struct Memory {
    idle: Vec<f64>,
    after_captures: Vec<f64>,
    scrap: Vec<f64>,
}

/// ホットキーから範囲選択の画面まで（ms）。
struct Hotkey {
    total: Vec<f64>,
    to_wm_hotkey: Vec<f64>,
}

/// コールドスタート（ms）。
struct ColdStart {
    total: Vec<f64>,
    to_process_start: Vec<f64>,
}

impl Report {
    fn print(&self) {
        println!();
        if let Some(m) = &self.memory {
            line("常駐時のメモリ（トレイの登録から 5 秒）", &m.idle, "MB");
            line("  参考: 撮影 10 回後", &m.after_captures, "MB");
            line("  参考: スクラップ 1 枚表示中", &m.scrap, "MB");
        }
        if let Some(h) = &self.hotkey {
            line(
                "ホットキーから範囲選択の画面まで（SendInput → DwmFlush）",
                &h.total,
                "ms",
            );
            line("  内訳: SendInput → WM_HOTKEY", &h.to_wm_hotkey, "ms");
        }
        if let Some(c) = &self.cold_start {
            line(
                "コールドスタート（プロセスの作成 → トレイの登録）",
                &c.total,
                "ms",
            );
            line("  内訳: プロセスの作成 → main", &c.to_process_start, "ms");
        }
        if let Some(a) = &self.annotate {
            line(
                "書き込みの更新（描き直しの始め → ウィンドウに出し終え）",
                a,
                "ms",
            );
        }
        // docs/perf.md の累積表に貼る行
        let cell = |values: Option<&Vec<f64>>| {
            values
                .and_then(|v| Summary::of(v))
                .map_or("-".to_owned(), |s| {
                    format!("{:.1} / {:.1}", s.median, s.p90)
                })
        };
        println!();
        if self.annotate.is_some() {
            println!("docs/perf.md の書き込みの更新の行（中央値 / p90）:");
            println!("| <時点> | <日付> | {} | |", cell(self.annotate.as_ref()));
        }
        if self.memory.is_none() && self.hotkey.is_none() && self.cold_start.is_none() {
            return;
        }
        println!("docs/perf.md の行（中央値 / p90）:");
        println!(
            "| <マイルストーン> | <日付> | {} | {} | {} | |",
            cell(self.memory.as_ref().map(|m| &m.idle)),
            cell(self.hotkey.as_ref().map(|h| &h.total)),
            cell(self.cold_start.as_ref().map(|c| &c.total)),
        );
    }
}

fn line(label: &str, values: &[f64], unit: &str) {
    match Summary::of(values) {
        Some(s) => println!(
            "{label}: 中央値 {:.1} {unit}、p90 {:.1} {unit}（{} 回）",
            s.median, s.p90, s.count
        ),
        None => println!("{label}: 測れなかった"),
    }
}

/// 常駐時のメモリ。1 回ごとに起動し直す。
fn measure_memory(options: &Options, workspace: &Workspace) -> Result<Memory> {
    let mut memory = Memory {
        idle: Vec::new(),
        after_captures: Vec::new(),
        scrap: Vec::new(),
    };
    let mut scrap_missing = false;
    for run in 0..options.runs {
        progress("常駐時のメモリ", run, options.runs);
        let (mut app, _) = Instance::spawn(options, workspace, run)?;
        let tray = app.wait(Mark::TrayRegistered, 0, START_TIMEOUT)?;
        // トレイの登録からちょうど 5 秒の時点で読む
        let since = Duration::from_secs_f64(app.log().millis(tray, qpc()).unwrap_or(0.0) / 1000.0);
        sleep(IDLE_AFTER_TRAY.saturating_sub(since));
        memory.idle.push(app.private_working_set()? as f64 / MIB);

        // 参考値: 撮ってスクラップを出し、閉じるのを 10 回。主モニタの左上寄りを 200×150 px ドラッグし、
        // 前面になったスクラップを Esc で閉じる（取得・読み戻し・スクラップの経路を通す）
        let mut captured = 0;
        for i in 0..CAPTURES {
            send_hotkey(options.hotkey);
            app.wait(Mark::OverlayFlushed, i, OVERLAY_TIMEOUT)?;
            sleep(Duration::from_millis(200));
            if scrap_missing {
                send_escape();
            } else {
                drag((100, 100), (300, 250));
                if app.wait(Mark::ScrapShown, captured, SCRAP_TIMEOUT).is_ok() {
                    captured += 1;
                    sleep(Duration::from_millis(200));
                } else {
                    // スクラップを出せない版（区切り scrap_shown を書かない）。範囲選択の画面を閉じるだけにする
                    scrap_missing = true;
                }
                send_escape();
            }
            sleep(Duration::from_millis(300));
        }
        sleep(SETTLE);
        memory
            .after_captures
            .push(app.private_working_set()? as f64 / MIB);

        // 参考値: スクラップ 1 枚表示中。主モニタの左上寄りを 400×300 px ドラッグする
        send_hotkey(options.hotkey);
        app.wait(Mark::OverlayFlushed, CAPTURES, OVERLAY_TIMEOUT)?;
        sleep(Duration::from_millis(200));
        drag((100, 100), (500, 400));
        if !scrap_missing && app.wait(Mark::ScrapShown, captured, SCRAP_TIMEOUT).is_ok() {
            sleep(SETTLE);
            memory.scrap.push(app.private_working_set()? as f64 / MIB);
        } else {
            // スクラップを出せない版（区切り scrap_shown を書かない）。範囲選択の画面を閉じて次へ
            scrap_missing = true;
            send_escape();
        }
        drop(app);
        sleep(Duration::from_millis(500));
    }
    Ok(memory)
}

/// ホットキーから範囲選択の画面まで。1 回起動したまま繰り返す。
fn measure_hotkey(options: &Options, workspace: &Workspace) -> Result<Hotkey> {
    let mut hotkey = Hotkey {
        total: Vec::new(),
        to_wm_hotkey: Vec::new(),
    };
    let (mut app, _) = Instance::spawn(options, workspace, 0)?;
    app.wait(Mark::TrayRegistered, 0, START_TIMEOUT)?;
    sleep(Duration::from_secs(1));
    for run in 0..options.runs {
        progress("ホットキーから範囲選択の画面まで", run, options.runs);
        let sent = qpc();
        send_hotkey(options.hotkey);
        let flushed = app.wait(Mark::OverlayFlushed, run, OVERLAY_TIMEOUT)?;
        let log = app.log();
        hotkey.total.extend(log.millis(sent, flushed));
        if let Some(received) = log.nth(Mark::HotkeyReceived, run) {
            hotkey.to_wm_hotkey.extend(log.millis(sent, received));
        }
        sleep(Duration::from_millis(300));
        send_escape();
        sleep(Duration::from_millis(500));
    }
    Ok(hotkey)
}

/// コールドスタート。1 回ごとに起動し直す。
fn measure_cold_start(options: &Options, workspace: &Workspace) -> Result<ColdStart> {
    let mut cold = ColdStart {
        total: Vec::new(),
        to_process_start: Vec::new(),
    };
    for run in 0..options.runs {
        progress("コールドスタート", run, options.runs);
        let (mut app, created) = Instance::spawn(options, workspace, run)?;
        let tray = app.wait(Mark::TrayRegistered, 0, START_TIMEOUT)?;
        let log = app.log();
        cold.total.extend(log.millis(created, tray));
        if let Some(start) = log.nth(Mark::ProcessStart, 0) {
            cold.to_process_start.extend(log.millis(created, start));
        }
        drop(app);
        sleep(Duration::from_millis(500));
    }
    Ok(cold)
}

/// 書き込みの更新。主モニタと同じ大きさの白紙スクラップ（白・不透明）を、白紙スクラップにする定型領域で 1 枚出し、
/// 書き込みの編集モード（`E`）でフリーハンド（`6`）を選んで、主モニタの中央を回る 200 歩のドラッグを送る。
/// 1 歩ごとに描き直しが出るのを待ってから次の歩を送り、ドラッグの間の描き直しの 1 回ずつを値にする。
fn measure_annotate(options: &Options, workspace: &Workspace) -> Result<Vec<f64>> {
    let hotkey: Chord = ANNOTATE_HOTKEY.parse()?;
    let (monitor, width, height) = primary_monitor()?;
    workspace.write_settings(
        options.hotkey,
        &format!(
            "\n[annotation]\nbackground = \"#FFFFFF\"\n\n\
             [[regions]]\nname = \"perf-annotate\"\nhotkey = \"{hotkey}\"\nmonitor = '{monitor}'\n\
             x = 0\ny = 0\nwidth = {width}\nheight = {height}\nblank_scrap = true\n"
        ),
    )?;
    let result = annotate_once(options, workspace, hotkey, width, height);
    // ほかの指標が同じ作業フォルダを使うので、設定を戻す
    workspace.write_settings(options.hotkey, "")?;
    result
}

fn annotate_once(
    options: &Options,
    workspace: &Workspace,
    hotkey: Chord,
    width: i32,
    height: i32,
) -> Result<Vec<f64>> {
    let (mut app, _) = Instance::spawn(options, workspace, 0)?;
    app.wait(Mark::TrayRegistered, 0, START_TIMEOUT)?;
    sleep(Duration::from_secs(1));
    send_hotkey(hotkey);
    // 白紙スクラップは区切りを書かないので、出て前面になるのを待つ
    sleep(Duration::from_secs(1));
    send_key(VK_E);
    sleep(Duration::from_millis(300));
    send_key(VK_6);
    // 道具を選ぶと描き直すので、その終わりで編集モードに入ってフリーハンドを選べたことを確かめる
    app.wait(Mark::AnnotatePaintEnd, 0, OVERLAY_TIMEOUT)?;
    sleep(Duration::from_millis(300));

    // 主モニタの中央を回る円。1 歩ずつ別の画素に動かす
    let (cx, cy) = (width / 2, height / 2);
    let radius = f64::from(width.min(height) / 4);
    let at = |step: usize| {
        let angle = step as f64 / ANNOTATE_STEPS as f64 * std::f64::consts::TAU;
        (
            cx + (radius * angle.cos()).round() as i32,
            cy + (radius * angle.sin()).round() as i32,
        )
    };
    let ends = |app: &Instance| {
        app.log()
            .marks
            .iter()
            .filter(|(m, _)| *m == Mark::AnnotatePaintEnd)
            .count()
    };
    move_mouse(at(0), width, height);
    sleep(Duration::from_millis(100));
    let started = qpc();
    send(&[mouse_button(MOUSEEVENTF_LEFTDOWN)]);
    let mut painted = ends(&app);
    for step in 1..=ANNOTATE_STEPS {
        progress("書き込みの更新", step - 1, ANNOTATE_STEPS);
        move_mouse(at(step), width, height);
        let sent = Instant::now();
        while ends(&app) <= painted && sent.elapsed() < STEP_TIMEOUT {
            sleep(Duration::from_millis(1));
        }
        painted = ends(&app);
    }
    let finished = qpc();
    send(&[mouse_button(MOUSEEVENTF_LEFTUP)]);
    sleep(Duration::from_millis(300));
    let log = app.log();
    let values = log
        .spans(Mark::AnnotatePaintStart, Mark::AnnotatePaintEnd)
        .into_iter()
        .filter(|&(start, _)| start >= started && start <= finished)
        .filter_map(|(start, end)| log.millis(start, end))
        .collect();
    drop(app);
    Ok(values)
}

/// 主モニタの識別子（本体の `monitor_list` と同じ、デバイスインターフェースのパス。取れなければ GDI のデバイス名）と、
/// 幅と高さ（物理ピクセル）。主モニタの左上はスクリーン座標の (0, 0)。
fn primary_monitor() -> Result<(String, i32, i32)> {
    // SAFETY: 問い合わせだけ
    let handle = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: cbSize を MONITORINFOEXW の大きさにして渡す
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
        return Err("主モニタの情報を読めない".into());
    }
    let mut dd = DISPLAY_DEVICEW {
        cb: size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    // SAFETY: szDevice は NUL で終わる。dd は呼び出しの間生きている
    let found = unsafe {
        EnumDisplayDevicesW(
            PCWSTR(info.szDevice.as_ptr()),
            0,
            &mut dd,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    }
    .as_bool();
    let id = Some(from_wide(&dd.DeviceID))
        .filter(|path| found && !path.is_empty())
        .unwrap_or_else(|| from_wide(&info.szDevice));
    let rect = info.monitorInfo.rcMonitor;
    Ok((id, rect.right - rect.left, rect.bottom - rect.top))
}

/// NUL で終わる UTF-16 の配列を文字列にする。
fn from_wide(wide: &[u16]) -> String {
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len])
}

fn progress(name: &str, run: usize, runs: usize) {
    eprint!("\r{name}: {}/{runs}   ", run + 1);
    if run + 1 == runs {
        eprintln!();
    }
}

/// 本体の `APPDATA` にする一時フォルダ。
struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    fn new(hotkey: Chord) -> Result<Workspace> {
        let dir = std::env::temp_dir().join(format!("shunsoku-perf-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(shunsoku_core::APP_NAME))?;
        let workspace = Workspace { dir };
        workspace.write_settings(hotkey, "")?;
        Ok(workspace)
    }

    /// ハーネスのホットキーだけの `settings.toml` に、`extra`（TOML の続き）を足して書く。
    fn write_settings(&self, hotkey: Chord, extra: &str) -> Result<()> {
        let settings = format!(
            "version = {}\n\n[hotkeys]\nselect_capture = \"{hotkey}\"\n{extra}",
            shunsoku_core::settings::VERSION
        );
        std::fs::write(
            self.dir.join(shunsoku_core::APP_NAME).join("settings.toml"),
            settings,
        )?;
        Ok(())
    }

    fn perf_log(&self, run: usize) -> PathBuf {
        self.dir.join(format!("perf-{run}.log"))
    }
}

/// 計測中の本体。drop で終わらせる。
struct Instance {
    child: Child,
    log: PathBuf,
}

impl Instance {
    /// 本体を起動する。プロセスを作る直前の QPC の時刻も返す。
    fn spawn(options: &Options, workspace: &Workspace, run: usize) -> Result<(Instance, i64)> {
        let log = workspace.perf_log(run);
        let _ = std::fs::remove_file(&log);
        let mut command = Command::new(&options.exe);
        command
            .env("APPDATA", &workspace.dir)
            .env(perf::LOG_VARIABLE, &log)
            .env(autostart::VALUE_NAME_VARIABLE, AUTOSTART_VALUE);
        let created = qpc();
        let child = command
            .spawn()
            .map_err(|e| format!("{} を起動できない: {e}", options.exe.display()))?;
        Ok((Instance { child, log }, created))
    }

    fn log(&self) -> PerfLog {
        read_log(&self.log)
    }

    /// 区切り `mark` の `n` 回目（0 始まり）が書かれるまで待ち、その時刻を返す。
    fn wait(&mut self, mark: Mark, n: usize, timeout: Duration) -> Result<i64> {
        let started = Instant::now();
        loop {
            if let Some(ticks) = self.log().nth(mark, n) {
                return Ok(ticks);
            }
            if let Some(status) = self.child.try_wait()? {
                return Err(format!(
                    "{} を待つ間に本体が終わった（{status}）。ログ: {}",
                    mark.name(),
                    self.log.display()
                )
                .into());
            }
            if started.elapsed() > timeout {
                return Err(format!(
                    "{} の {} 回目が {} 秒以内に書かれなかった。ログ: {}",
                    mark.name(),
                    n + 1,
                    timeout.as_secs(),
                    self.log.display()
                )
                .into());
            }
            sleep(Duration::from_millis(1));
        }
    }

    /// 本体の Private Working Set（バイト）。
    fn private_working_set(&self) -> Result<u64> {
        let process = HANDLE(self.child.as_raw_handle());
        // 1 要素目がページの数、続く要素が 1 ページずつの情報
        let mut buf: Vec<usize> = vec![0; 1 + 64 * 1024];
        loop {
            let bytes = u32::try_from(buf.len() * size_of::<usize>())?;
            // SAFETY: buf はこの関数の中で生きていて、bytes はその大きさ
            match unsafe { QueryWorkingSet(process, buf.as_mut_ptr().cast(), bytes) } {
                Ok(()) => {
                    let pages = buf[0].min(buf.len() - 1);
                    let private = buf[1..=pages]
                        .iter()
                        .filter(|&&page| page & PAGE_SHAREABLE == 0)
                        .count();
                    return Ok(private as u64 * page_size());
                }
                // 足りなければ、1 要素目に入ったページの数より少し大きくして読み直す
                Err(e) if e.code() == ERROR_BAD_LENGTH.to_hresult() => {
                    buf.resize(buf[0] + 1 + 4096, 0);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        // 本体はメッセージ専用ウィンドウへの WM_CLOSE でトレイを消して終わる
        // SAFETY: 見つけたウィンドウに送るだけ
        unsafe {
            if let Ok(hwnd) = FindWindowExW(Some(HWND_MESSAGE), None, APP_CLASS, None) {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_log(path: &Path) -> PerfLog {
    PerfLog::parse(&std::fs::read_to_string(path).unwrap_or_default())
}

/// 本体（このハーネスが起動したものに限らない）が常駐しているか。
fn running_instance() -> bool {
    // SAFETY: クラス名は 'static
    unsafe { FindWindowExW(Some(HWND_MESSAGE), None, APP_CLASS, None) }.is_ok()
}

fn qpc() -> i64 {
    let mut ticks = 0;
    // SAFETY: ticks はこの関数の中で生きている。XP 以降は失敗しない
    let _ = unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}

fn page_size() -> u64 {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: info はこの関数の中で生きている
    unsafe { GetSystemInfo(&mut info) };
    u64::from(info.dwPageSize)
}

/// ホットキーを押して離す。押す順は `Chord::strokes`（修飾キーを押してからキーを押し、逆の順に離す）。
fn send_hotkey(chord: Chord) {
    let inputs: Vec<INPUT> = chord
        .strokes()
        .into_iter()
        .map(|k| {
            let mut flags = KEYBD_EVENT_FLAGS(0);
            if k.up {
                flags |= KEYEVENTF_KEYUP;
            }
            if k.extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(k.vk),
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            }
        })
        .collect();
    send(&inputs);
}

/// Esc を押して離す。
fn send_escape() {
    send_key(VK_ESCAPE.0);
}

/// 修飾キーなしで `vk` を押して離す。`vk` は `Key` の名前の表にあるキー。
fn send_key(vk: u16) {
    send_hotkey(Chord {
        modifiers: Modifiers::default(),
        key: Key::from_vk(vk).expect("名前の表にあるキー"),
    });
}

/// 左ボタンを押すか離す（`flags`）入力。
fn mouse_button(flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                ..Default::default()
            },
        },
    }
}

/// カーソルを主モニタ（幅 `width`、高さ `height`）の `to`（物理ピクセル）へ `SendInput` で動かす。絶対座標は
/// 主モニタを 0〜65535 にした値で、画素 x は ceil(x × 65536 / 幅) で表す。
fn move_mouse(to: (i32, i32), width: i32, height: i32) {
    let normal = |v: i32, size: i32| {
        let size = i64::from(size);
        ((i64::from(v) * 65536 + size - 1) / size) as i32
    };
    send(&[INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: normal(to.0, width),
                dy: normal(to.1, height),
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                ..Default::default()
            },
        },
    }]);
}

/// 左ボタンで `from` から `to` まで（スクリーン座標、物理ピクセル）ドラッグする。
fn drag(from: (i32, i32), to: (i32, i32)) {
    let button = mouse_button;
    // SAFETY: 座標を渡すだけ
    unsafe {
        let _ = SetCursorPos(from.0, from.1);
        sleep(Duration::from_millis(50));
        send(&[button(MOUSEEVENTF_LEFTDOWN)]);
        for i in 1..=10 {
            let _ = SetCursorPos(
                from.0 + (to.0 - from.0) * i / 10,
                from.1 + (to.1 - from.1) * i / 10,
            );
            sleep(Duration::from_millis(20));
        }
        send(&[button(MOUSEEVENTF_LEFTUP)]);
    }
}

fn send(inputs: &[INPUT]) {
    // SAFETY: inputs はこの関数の中で生きている
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
}
