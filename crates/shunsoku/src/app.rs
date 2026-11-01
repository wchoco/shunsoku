//! 常駐本体の UI スレッドとアプリの状態。Win32 層のモジュールは互いを呼ばず、ここがつなぐ（ADR-0005）。
//!
//! 決まりごと（後続のチケットもこれに従う）:
//!
//! - アプリの状態（`App`）を持つのは UI スレッドだけ。`APP`（thread_local）に置き、`with_app` で短く借りる。
//! - モーダルなループ（`TrackPopupMenu`、`MessageBoxW` など）の間は `App` を借りない。ループの中で
//!   ウィンドウ プロシージャが呼び直されるため。借りられないときのメッセージはログに書いて捨てる。
//! - UI の文言はすべて `text`（core の `i18n`）から引く。
//! - アプリのメッセージは `WM_APP_*` としてここにまとめる。裏のスレッドは結果を `std::sync::mpsc` に入れ、
//!   `PostMessageW(メッセージ専用ウィンドウ, WM_APP_*)` で UI スレッドを起こす。裏のスレッドに渡す値は
//!   UI スレッドで解決しておき、本体は `catch_unwind` で包んでパニックをその仕事の失敗として返す。
//! - ウィンドウ プロシージャは `catch_unwind` で包む。UI スレッドのパニックは `panic_hook` がログと
//!   メッセージ ボックスを出し、ここでメッセージ ループを終えて終了する。
//! - ウィンドウは 2 つ。ホットキーと登録メッセージを受けるメッセージ専用ウィンドウと、トレイの持ち主の
//!   隠しウィンドウ。メッセージ専用ウィンドウにはブロードキャストの `TaskbarCreated` が届かないため、
//!   Explorer の再起動のあとにトレイを登録し直せるように、トレイだけ通常の（見えない）ウィンドウに持たせる。
//!   ほかに、範囲選択の画面を開いている間だけ、モニタごとのウィンドウ（`overlay`）がある。
//! - キャプチャは `capture::request` の依頼 1 本の流れで進める: 依頼 → 待機（`delay`）→ 取得（`grab`）→
//!   範囲選択の画面（`overlay`）→ 切り抜き → 出力 → 結果。同時に進める依頼は 1 件だけ（`Capture`）。

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use shunsoku_core::APP_NAME;
use shunsoku_core::capture::request::{CaptureRequest, Source};
use shunsoku_core::i18n::Catalog;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    IDI_APPLICATION, KillTimer, LoadIconW, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WM_ACTIVATEAPP, WM_APP,
    WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND, WM_KEYDOWN, WM_TIMER,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use crate::{grab, logger, overlay, panic_hook, single_instance, tray};

/// トレイのアイコンへの操作（`tray::event` で読む）。
const WM_APP_TRAY: u32 = WM_APP + 1;
/// 環境変数 `SHUNSOKU_TEST_PANIC=ui` のとき、UI スレッドでパニックを起こす（手で確かめる用）。
const WM_APP_TEST_PANIC: u32 = WM_APP + 2;

/// 2 つのウィンドウのクラス名。2 つ目のプロセスはこの名前でメッセージ専用ウィンドウを探す。
const CLASS_NAME: PCWSTR = w!("shunsoku.app");
/// 2 つ目のプロセスが送る登録メッセージの名前。
const SECOND_INSTANCE_MESSAGE: PCWSTR = w!("shunsoku.SecondInstance");

/// トレイのメニューの項目の番号。
const CMD_EXIT: u32 = 1;
const CMD_SELECT_CAPTURE: u32 = 2;

/// 依頼の `delay` を待つタイマーの番号（メッセージ専用ウィンドウの WM_TIMER）。
const TIMER_CAPTURE_DELAY: usize = 1;

/// UI スレッドのパニックで終わったときの終了コード（Rust の既定と同じ）。
const EXIT_PANIC: i32 = 101;

/// 登録メッセージの番号。`App` を借りずに比べられるように別に持つ。
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
static SECOND_INSTANCE: OnceLock<u32> = OnceLock::new();

/// 埋め込みの文言（`locales/ja.toml`）。
static CATALOG: OnceLock<Catalog> = OnceLock::new();

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// アプリの状態。UI スレッドだけが持つ。drop の順（宣言の順）でトレイを消し、範囲選択の画面を閉じてから
/// ウィンドウを壊す。
struct App {
    tray: tray::Tray,
    capture: Capture,
    gpu: grab::Gpu,
    windows: Windows,
    module: HINSTANCE,
    _instance: Option<single_instance::Guard>,
}

/// 進めているキャプチャの依頼。同時に 1 件だけ。
#[derive(Default)]
enum Capture {
    #[default]
    Idle,
    /// `delay` を待っている。
    Waiting(CaptureRequest),
    /// 範囲選択の画面を開いている。
    Selecting {
        #[expect(dead_code, reason = "範囲を選んで撮るチケット（#41）で出力に使う")]
        request: CaptureRequest,
        overlay: overlay::Overlay,
    },
}

struct Windows {
    message: HWND,
    tray_owner: HWND,
}

impl Drop for Windows {
    fn drop(&mut self) {
        // SAFETY: 自分で作ったウィンドウを、作ったスレッドで壊す
        unsafe {
            let _ = DestroyWindow(self.tray_owner);
            let _ = DestroyWindow(self.message);
        }
    }
}

/// キーの文言。キーがなければキーそのもの。
fn text(key: &'static str) -> &'static str {
    CATALOG.get().map_or(key, |c| c.text(key))
}

/// 常駐本体を動かし、終了コードを返す。
pub fn run() -> i32 {
    let log_path = std::env::var_os("APPDATA")
        .map(|dir| PathBuf::from(dir).join(APP_NAME))
        .and_then(|dir| logger::init(&dir).ok());
    log::info!("starting {APP_NAME} v{}", env!("CARGO_PKG_VERSION"));

    let catalog = Catalog::parse(include_str!("../../../locales/ja.toml")).unwrap_or_else(|e| {
        log::error!("bundled locale is not valid TOML: {e}");
        Catalog::default()
    });
    let _ = CATALOG.set(catalog);

    let body = match &log_path {
        Some(path) => format!("{}\n{}", text("panic.body"), path.display()),
        None => text("panic.body").to_owned(),
    };
    panic_hook::install(
        std::thread::current().id(),
        text("panic.title").to_owned(),
        body,
    );

    // manifest で Per-Monitor V2 になっている。念のための呼び出しで、既に設定済みならエラーになる
    // SAFETY: 引数は定数
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // SAFETY: 名前は 'static
    let second_instance = unsafe { RegisterWindowMessageW(SECOND_INSTANCE_MESSAGE) };
    let _ = SECOND_INSTANCE.set(second_instance);
    let instance = match single_instance::acquire() {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            let notified = single_instance::notify_first(CLASS_NAME, second_instance);
            log::info!("another instance is running (notified: {notified}); exiting");
            return 0;
        }
        Err(e) => {
            // 二重起動を防げないだけなので、常駐は続ける
            log::warn!("could not create the single-instance mutex: {e}");
            None
        }
    };

    let app = match start(instance) {
        Ok(app) => app,
        Err(e) => {
            log::error!("failed to start: {e}");
            return 1;
        }
    };
    let message_window = app.windows.message;
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    log::info!("started");

    match std::env::var("SHUNSOKU_TEST_PANIC").as_deref() {
        Ok("ui") => {
            // SAFETY: 自分のウィンドウに送る
            let _ = unsafe {
                PostMessageW(
                    Some(message_window),
                    WM_APP_TEST_PANIC,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
        Ok("worker") => {
            let _ = std::thread::Builder::new()
                .name("test-panic".into())
                .spawn(|| panic!("test panic on a background thread (SHUNSOKU_TEST_PANIC)"));
        }
        _ => {}
    }

    let code = message_loop();
    // トレイを消してからウィンドウを壊す。APP から出してから drop するので、
    // 壊すときに呼ばれるウィンドウ プロシージャは App を見ない
    let app = APP.with(|cell| cell.borrow_mut().take());
    drop(app);
    log::info!("exiting (code {code})");
    code
}

fn start(instance: Option<single_instance::Guard>) -> Result<App> {
    // SAFETY: 自分のモジュールのハンドル
    let module: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: module,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: class の文字列は 'static
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_thread());
    }

    overlay::register_class(module, Some(overlay_wndproc))?;

    // SAFETY: 名前は 'static
    let _ = TASKBAR_CREATED.set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });

    // SAFETY: 登録したクラスでウィンドウを作る。どちらも表示しない
    let windows = unsafe {
        let message = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            w!("shunsoku"),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(module),
            None,
        )?;
        let tray_owner = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("shunsoku"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module),
            None,
        );
        match tray_owner {
            Ok(tray_owner) => Windows {
                message,
                tray_owner,
            },
            Err(e) => {
                let _ = DestroyWindow(message);
                return Err(e);
            }
        }
    };

    // アイコンは M1 では Windows の既定のものを使う
    // SAFETY: 共有のアイコンなので壊さなくてよい
    let icon = unsafe { LoadIconW(None, IDI_APPLICATION)? };
    let tray = tray::Tray::new(
        windows.tray_owner,
        WM_APP_TRAY,
        icon,
        text("tray.tooltip").to_owned(),
    );
    if tray.register() {
        log::info!("tray icon registered");
    } else {
        log::warn!("could not register the tray icon; waiting for TaskbarCreated");
    }

    Ok(App {
        tray,
        capture: Capture::Idle,
        gpu: grab::Gpu::default(),
        windows,
        module,
        _instance: instance,
    })
}

fn message_loop() -> i32 {
    let mut msg = MSG::default();
    loop {
        // SAFETY: msg はこの関数の中で生きている
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        match ret.0 {
            0 => return msg.wParam.0 as i32,
            -1 => {
                log::error!(
                    "GetMessageW failed: {}",
                    windows::core::Error::from_thread()
                );
                return 1;
            }
            _ => unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            },
        }
    }
}

/// `App` を借りて `f` を呼ぶ。まだない・もうない・借りられている（モーダルなループの中など）ときは None。
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => app.as_mut().map(f),
        Err(_) => {
            log::warn!("app state is busy; message dropped");
            None
        }
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: 受け取った引数をそのまま渡す
    unsafe { dispatch(on_message, hwnd, msg, wparam, lparam) }
}

/// 範囲選択の画面のウィンドウのプロシージャ。
unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 受け取った引数をそのまま渡す
    unsafe { dispatch(on_overlay_message, hwnd, msg, wparam, lparam) }
}

/// `handler` をパニックから守って呼ぶ。`handler` が None を返したら DefWindowProcW に任せる。
unsafe fn dispatch(
    handler: fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>,
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if !panic_hook::ui_panicked() {
        match catch_unwind(AssertUnwindSafe(|| handler(hwnd, msg, wparam, lparam))) {
            Ok(Some(result)) => return result,
            Ok(None) => {}
            Err(_) => {
                // フックがログとメッセージ ボックスを出し終えている
                // SAFETY: UI スレッドで呼ぶ
                unsafe { PostQuitMessage(EXIT_PANIC) };
                return LRESULT(0);
            }
        }
    }
    // SAFETY: 受け取った引数をそのまま渡す
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// メッセージを処理する。None なら DefWindowProcW に任せる。
fn on_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_APP_TRAY => {
            on_tray(hwnd, tray::event(wparam, lparam));
            Some(LRESULT(0))
        }
        WM_APP_TEST_PANIC => panic!("test panic on the UI thread (SHUNSOKU_TEST_PANIC)"),
        WM_TIMER if wparam.0 == TIMER_CAPTURE_DELAY => {
            // SAFETY: 自分のタイマーを止める
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_CAPTURE_DELAY);
            }
            open_overlay();
            Some(LRESULT(0))
        }
        WM_CLOSE => {
            // 外から閉じるよう頼まれた（`taskkill` の /F なしなど）。トレイの「終了」と同じく終わる
            log::info!("close requested");
            // SAFETY: UI スレッドで呼ぶ
            unsafe { PostQuitMessage(0) };
            Some(LRESULT(0))
        }
        WM_ENDSESSION if wparam.0 != 0 => {
            // ログオフ・シャットダウン。戻るとプロセスは終わらされる（Explorer も終わるのでトレイは消さない）
            log::info!("exiting (session end)");
            Some(LRESULT(0))
        }
        // 登録メッセージは定数でないので match の外で比べる
        _ if Some(&msg) == TASKBAR_CREATED.get() => {
            let ok = with_app(|app| app.tray.register());
            log::info!("taskbar re-created; tray icon re-registered: {ok:?}");
            Some(LRESULT(0))
        }
        _ if Some(&msg) == SECOND_INSTANCE.get() => {
            // M1 では何もしない（M5 で設定 GUI を開く）
            log::info!("another instance was launched");
            Some(LRESULT(0))
        }
        _ => None,
    }
}

fn on_tray(owner: HWND, event: tray::Event) {
    match event {
        tray::Event::ContextMenu { x, y } => {
            let items = [
                tray::MenuItem {
                    id: CMD_SELECT_CAPTURE,
                    label: text("tray.menu.select_capture"),
                    enabled: true,
                },
                tray::MenuItem {
                    id: CMD_EXIT,
                    label: text("tray.menu.exit"),
                    enabled: true,
                },
            ];
            // モーダルなので App を借りずに呼ぶ
            if let Some(command) = tray::show_menu(owner, &items, x, y) {
                on_command(command);
            }
        }
        tray::Event::Select => request_capture(Source::TrayClick),
        tray::Event::Other => {}
    }
}

fn on_command(command: u32) {
    match command {
        CMD_SELECT_CAPTURE => request_capture(Source::TrayMenu),
        CMD_EXIT => {
            log::info!("exit requested from the tray menu");
            // SAFETY: UI スレッドで呼ぶ
            unsafe { PostQuitMessage(0) };
        }
        _ => {}
    }
}

/// 範囲選択の画面のウィンドウへのメッセージ。キー入力は前面の 1 枚にしか来ないので、
/// どのウィンドウに来たかで分けない。
fn on_overlay_message(_hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_KEYDOWN if wparam.0 == usize::from(VK_ESCAPE.0) => {
            log::info!("overlay cancelled (Esc)");
            close_overlay();
            Some(LRESULT(0))
        }
        // 他のアプリに前面を取られるとキー入力が来なくなり、閉じられなくなるので閉じる
        WM_ACTIVATEAPP if wparam.0 == 0 => {
            log::info!("overlay lost the foreground; closing");
            close_overlay();
            Some(LRESULT(0))
        }
        // モニタの構成が変わると、固定された画面は実際の画面と合わなくなる
        WM_DISPLAYCHANGE => {
            log::info!("display changed while the overlay is open; closing");
            close_overlay();
            Some(LRESULT(0))
        }
        // 背景は塗らない（描くのはスワップチェーンだけ）
        WM_ERASEBKGND => Some(LRESULT(1)),
        // 画素を等倍のまま出すので、DPI が変わっても大きさを変えない
        WM_DPICHANGED => Some(LRESULT(0)),
        _ => None,
    }
}

/// 発生源からキャプチャの依頼を受ける。ほかの依頼を進めている間（範囲選択の画面を開いている間など）は無視する。
fn request_capture(source: Source) {
    let request = CaptureRequest::from_source(source);
    let delay = request.delay;
    let accepted = with_app(|app| {
        if !matches!(app.capture, Capture::Idle) {
            return None;
        }
        app.capture = Capture::Waiting(request);
        Some(app.windows.message)
    })
    .flatten();
    let Some(message_window) = accepted else {
        log::info!("capture requested from {source:?} while another one is in progress; ignored");
        return;
    };
    log::info!("capture requested from {source:?}");
    if delay.is_zero() {
        open_overlay();
        return;
    }
    let ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: 自分のウィンドウのタイマー
    if unsafe { SetTimer(Some(message_window), TIMER_CAPTURE_DELAY, ms, None) } == 0 {
        log::warn!("could not start the capture delay timer; capturing now");
        open_overlay();
    }
}

/// 待っている依頼の画面を取得して、範囲選択の画面を出す。
fn open_overlay() {
    let started = Instant::now();
    // 取得のスレッドは App に触らないので、借りたまま待ってよい
    let grabbed = with_app(|app| {
        matches!(app.capture, Capture::Waiting(_)).then(|| (app.gpu.grab(), app.module))
    })
    .flatten();
    let Some((grabbed, module)) = grabbed else {
        return;
    };
    let shots = match grabbed {
        Ok(shots) => shots,
        Err(e) => {
            // 撮れないときの通知は M3
            log::warn!("could not grab the screen: {e}");
            end_capture();
            return;
        }
    };
    let monitors = shots.len();
    let grab_ms = started.elapsed().as_secs_f64() * 1000.0;

    // ウィンドウを作るとメッセージが届くので、App を借りずに開く
    let overlay = match overlay::Overlay::open(shots, module) {
        Ok(overlay) => overlay,
        Err(e) => {
            log::error!("could not open the overlay: {e}");
            end_capture();
            return;
        }
    };
    let rejected = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Waiting(request) => {
            app.capture = Capture::Selecting { request, overlay };
            None
        }
        other => {
            app.capture = other;
            Some(overlay)
        }
    });
    if !matches!(rejected, Some(None)) {
        // 依頼がもうない（起こらないはず）。開いた画面を App を借りずに閉じる
        log::warn!("capture request vanished while opening the overlay");
        drop(rejected);
        end_capture();
        return;
    }
    log::info!(
        "overlay opened on {monitors} monitor(s): grab {grab_ms:.1} ms, total {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// 範囲選択の画面を閉じ、依頼を終える。
fn close_overlay() {
    let overlay = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Selecting { overlay, .. } => Some(overlay),
        other => {
            app.capture = other;
            None
        }
    })
    .flatten();
    let Some(overlay) = overlay else {
        return;
    };
    // ウィンドウを壊すとメッセージが届くので、App を借りずに閉じる
    drop(overlay);
    end_capture();
    log::info!("overlay closed");
}

/// 依頼を終えて、取得に使った GPU のメモリを返す。
fn end_capture() {
    with_app(|app| {
        app.capture = Capture::Idle;
        app.gpu.trim();
    });
}
