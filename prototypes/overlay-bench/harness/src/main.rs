// PROTOTYPE — 捨てるコード。shunsoku #5 の計測ドライバ。
// harness startup <exe> <n> [args..]   起動から常駐準備完了（トレイ登録後）までの時間
// harness latency <exe> <n> [args..]   SendInput でホットキーを送り、範囲選択の画面が表示されるまでの時間
// harness memory  <exe> [args..]        常駐時 / 撮影 10 回後 / スクラップ 1 枚表示中 の Private Working Set
use std::{
    path::PathBuf,
    process::{Child, Command},
    thread::sleep,
    time::{Duration, Instant},
};

use windows::{
    core::*,
    Win32::{
        Foundation::*,
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

fn qpc() -> i64 {
    let mut v = 0;
    unsafe { QueryPerformanceCounter(&mut v).unwrap() };
    v
}
fn freq() -> f64 {
    let mut f = 0;
    unsafe { QueryPerformanceFrequency(&mut f).unwrap() };
    f as f64
}
fn ms(d: i64) -> f64 {
    d as f64 * 1000.0 / freq()
}

fn kv(line: &str, key: &str) -> Option<i64> {
    line.split_whitespace()
        .find_map(|t| t.strip_prefix(&format!("{key}=")))
        .and_then(|v| v.parse().ok())
}

struct Run {
    child: Child,
    log: PathBuf,
}

impl Run {
    fn start(exe: &str, args: &[String], tag: &str) -> (Run, i64) {
        let log = std::env::temp_dir().join(format!("shunsoku-bench-{tag}-{}-{}.log", std::process::id(), qpc()));
        let _ = std::fs::remove_file(&log);
        let t0 = qpc();
        let child = Command::new(exe).args(args).env("SHUNSOKU_BENCH_LOG", &log).spawn().unwrap();
        (Run { child, log }, t0)
    }
    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log).unwrap_or_default().lines().map(String::from).collect()
    }
    fn wait_line(&self, prefix: &str, nth: usize, timeout: Duration) -> Option<String> {
        let t = Instant::now();
        while t.elapsed() < timeout {
            if let Some(l) = self.lines().into_iter().filter(|l| l.starts_with(prefix)).nth(nth) {
                return Some(l);
            }
            sleep(Duration::from_millis(1));
        }
        None
    }
    fn stop(mut self) {
        unsafe {
            if let Ok(h) = FindWindowW(w!("proto-main"), None) {
                let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(2) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    fn private_ws(&self) -> (f64, f64) {
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$p=Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter 'IDProcess={}'; \"$($p.WorkingSetPrivate) $($p.WorkingSet)\"",
                    self.child.id()
                ),
            ])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        let v: Vec<f64> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        (v.first().copied().unwrap_or(0.0) / 1048576.0, v.get(1).copied().unwrap_or(0.0) / 1048576.0)
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() },
        },
    }
}
fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
}
fn hotkey() {
    send(&[
        key(VK_CONTROL, false),
        key(VK_SHIFT, false),
        key(VK_F9, false),
        key(VK_F9, true),
        key(VK_SHIFT, true),
        key(VK_CONTROL, true),
    ]);
}
fn esc() {
    send(&[key(VK_ESCAPE, false), key(VK_ESCAPE, true)]);
}
fn mouse(flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } } }
}
fn drag(a: (i32, i32), b: (i32, i32)) {
    unsafe {
        let _ = SetCursorPos(a.0, a.1);
        sleep(Duration::from_millis(50));
        send(&[mouse(MOUSEEVENTF_LEFTDOWN)]);
        for i in 1..=10 {
            let _ = SetCursorPos(a.0 + (b.0 - a.0) * i / 10, a.1 + (b.1 - a.1) * i / 10);
            sleep(Duration::from_millis(20));
        }
        send(&[mouse(MOUSEEVENTF_LEFTUP)]);
    }
}

fn stats(name: &str, mut v: Vec<f64>) {
    if v.is_empty() {
        return;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    println!("{name:<28} min {:7.1}  med {:7.1}  p90 {:7.1}  max {:7.1} ms", v[0], p(0.5), p(0.9), v[v.len() - 1]);
}

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let a: Vec<String> = std::env::args().collect();
    let cmd = a[1].as_str();
    let exe = a[2].as_str();
    match cmd {
        "startup" => {
            let n: usize = a[3].parse().unwrap();
            let rest = &a[4..];
            let (mut total, mut entry) = (vec![], vec![]);
            for i in 0..n {
                let (run, t0) = Run::start(exe, rest, "startup");
                let l = run.wait_line("ready", 0, Duration::from_secs(10)).expect("no ready");
                total.push(ms(kv(&l, "ready").unwrap() - t0));
                entry.push(ms(kv(&l, "entry").unwrap() - t0));
                if i == 0 {
                    println!("{l}");
                }
                run.stop();
                sleep(Duration::from_millis(300));
            }
            stats("spawn -> main entry", entry);
            stats("spawn -> ready (tray)", total);
        }
        "latency" => {
            let n: usize = a[3].parse().unwrap();
            let rest = &a[4..];
            let (run, _) = Run::start(exe, rest, "latency");
            run.wait_line("ready", 0, Duration::from_secs(10)).expect("no ready");
            sleep(Duration::from_millis(1000));
            let mut sent = vec![];
            for i in 0..n {
                sent.push(qpc());
                hotkey();
                if run.wait_line("trigger", i, Duration::from_secs(5)).is_none() {
                    println!("trigger {i} timeout");
                    for l in run.lines() {
                        println!("  {l}");
                    }
                    break;
                }
                sleep(Duration::from_millis(250));
                esc();
                sleep(Duration::from_millis(400));
            }
            let trig: Vec<String> = run.lines().into_iter().filter(|l| l.starts_with("trigger")).collect();
            if let Some(l) = trig.first() {
                println!("{l}");
            }
            for l in run.lines().iter().filter(|l| !l.starts_with("trigger") && !l.starts_with("ready")).skip(2).take(3) {
                println!("{l}");
            }
            let col = |f: &dyn Fn(&str, i64) -> i64| -> Vec<f64> {
                trig.iter().zip(&sent).map(|(l, s)| ms(f(l, *s))).collect()
            };
            stats("input -> WM_HOTKEY", col(&|l, s| kv(l, "hotkey").unwrap() - s));
            stats("WM_HOTKEY -> captured", col(&|l, _| kv(l, "captured").unwrap() - kv(l, "hotkey").unwrap()));
            stats("captured -> shown", col(&|l, _| kv(l, "shown").unwrap() - kv(l, "captured").unwrap()));
            stats("shown -> DwmFlush", col(&|l, _| kv(l, "flushed").unwrap() - kv(l, "shown").unwrap()));
            stats("input -> shown", col(&|l, s| kv(l, "shown").unwrap() - s));
            stats("input -> DwmFlush", col(&|l, s| kv(l, "flushed").unwrap() - s));
            run.stop();
        }
        "memory" => {
            let rest = &a[3..];
            let (run, _) = Run::start(exe, rest, "memory");
            run.wait_line("ready", 0, Duration::from_secs(10)).expect("no ready");
            sleep(Duration::from_secs(3));
            let m0 = run.private_ws();
            for i in 0..10 {
                hotkey();
                run.wait_line("trigger", i, Duration::from_secs(5));
                sleep(Duration::from_millis(200));
                esc();
                sleep(Duration::from_millis(300));
            }
            sleep(Duration::from_secs(3));
            let m1 = run.private_ws();
            hotkey();
            run.wait_line("trigger", 10, Duration::from_secs(5));
            sleep(Duration::from_millis(300));
            drag((200, 200), (900, 700));
            sleep(Duration::from_secs(3));
            let m2 = run.private_ws();
            println!("                          private WS    working set");
            println!("idle after start          {:7.1} MB   {:7.1} MB", m0.0, m0.1);
            println!("idle after 10 captures    {:7.1} MB   {:7.1} MB", m1.0, m1.1);
            println!("1 scrap (700x500) shown   {:7.1} MB   {:7.1} MB", m2.0, m2.1);
            for l in run.lines().iter().filter(|l| !l.starts_with("trigger")) {
                println!("{l}");
            }
            run.stop();
        }
        _ => panic!("unknown command"),
    }
}
