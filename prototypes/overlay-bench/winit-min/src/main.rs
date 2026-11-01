// PROTOTYPE — 捨てるコード。shunsoku #5 の比較用: ウィンドウ管理を winit に任せた場合の土台のコスト。
// 常駐（tray-icon）→ グローバルホットキー（global-hotkey, Ctrl+Shift+F9）→ 仮想スクリーン全体の最前面・枠なしウィンドウを
// softbuffer で塗って表示するまで。キャプチャはしない（raw 版と共通の処理なので、その分は raw 版の数値を足して読む）。
#![windows_subsystem = "windows"]
use std::{fs::File, io::Write, num::NonZeroU32, rc::Rc, sync::Mutex};

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    platform::windows::WindowAttributesExtWindows,
    window::{Window, WindowId, WindowLevel},
};

fn qpc() -> i64 {
    let mut v = 0;
    unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut v).unwrap() };
    v
}

struct Overlay {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    t_hot: i64,
    logged: bool,
}

struct App {
    t_entry: i64,
    log: Option<File>,
    tray: Option<tray_icon::TrayIcon>,
    _hk: GlobalHotKeyManager,
    overlay: Option<Overlay>,
    n: u32,
}

impl App {
    fn log(&mut self, s: String) {
        if let Some(f) = self.log.as_mut() {
            let _ = writeln!(f, "{s}");
            let _ = f.flush();
        }
    }
}

impl ApplicationHandler<i64> for App {
    fn resumed(&mut self, _el: &ActiveEventLoop) {
        if self.tray.is_none() {
            let icon = tray_icon::Icon::from_rgba(vec![200u8; 16 * 16 * 4], 16, 16).unwrap();
            self.tray = Some(
                tray_icon::TrayIconBuilder::new().with_tooltip("shunsoku prototype (winit)").with_icon(icon).build().unwrap(),
            );
            let mut f = 0;
            unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut f).unwrap() };
            let pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
            let s = format!("ready entry={} ready={} freq={f} method=winit pid={pid}", self.t_entry, qpc());
            self.log(s);
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, t_hot: i64) {
        if self.overlay.is_some() {
            return;
        }
        // 仮想スクリーン全体（物理ピクセル）
        let (mut l, mut t, mut r, mut b) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for m in el.available_monitors() {
            let p = m.position();
            let s = m.size();
            l = l.min(p.x);
            t = t.min(p.y);
            r = r.max(p.x + s.width as i32);
            b = b.max(p.y + s.height as i32);
        }
        let attrs = Window::default_attributes()
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_skip_taskbar(true)
            .with_position(PhysicalPosition::new(l, t))
            .with_inner_size(PhysicalSize::new((r - l) as u32, (b - t) as u32));
        let window = Rc::new(el.create_window(attrs).unwrap());
        let ctx = softbuffer::Context::new(window.clone()).unwrap();
        let surface = softbuffer::Surface::new(&ctx, window.clone()).unwrap();
        window.focus_window();
        window.request_redraw();
        self.overlay = Some(Overlay { window, surface, t_hot, logged: false });
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::RedrawRequested => {
                let mut line = None;
                if let Some(ov) = self.overlay.as_mut() {
                    let size = ov.window.inner_size();
                    ov.surface.resize(NonZeroU32::new(size.width).unwrap(), NonZeroU32::new(size.height).unwrap()).unwrap();
                    let mut buf = ov.surface.buffer_mut().unwrap();
                    buf.fill(0x00303040);
                    buf.present().unwrap();
                    if !ov.logged {
                        ov.logged = true;
                        let shown = qpc();
                        unsafe {
                            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                        }
                        self.n += 1;
                        line = Some(format!(
                            "trigger n={} method=winit hotkey={} captured={} shown={shown} flushed={} dpi={}",
                            self.n,
                            ov.t_hot,
                            ov.t_hot,
                            qpc(),
                            (ov.window.scale_factor() * 96.0) as u32
                        ));
                    }
                }
                if let Some(l) = line {
                    self.log(l);
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                self.overlay = None;
            }
            _ => {}
        }
    }
}

fn main() {
    let t_entry = qpc();
    let log = std::env::var("SHUNSOKU_BENCH_LOG")
        .ok()
        .and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    let el = EventLoop::<i64>::with_user_event().build().unwrap();
    let proxy = Mutex::new(el.create_proxy());
    let hk = GlobalHotKeyManager::new().unwrap();
    hk.register(HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::F9)).unwrap();
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            let _ = proxy.lock().unwrap().send_event(qpc());
        }
    }));
    let mut app = App { t_entry, log, tray: None, _hk: hk, overlay: None, n: 0 };
    el.run_app(&mut app).unwrap();
}
