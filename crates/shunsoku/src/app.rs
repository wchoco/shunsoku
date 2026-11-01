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
//! - 範囲選択の入力（マウス・キー）は core の `selection::Input` にして `Selection::handle` に渡す。
//!   入力を足すときは `Input` を足し、ここではメッセージから作るだけにする。
//! - スクラップ（`Scrap`）は `App::scraps` に持つ。ウィンドウは `scrap_window`、中身の描画は `render`。
//! - 設定（`%APPDATA%\shunsoku\settings.toml`）は起動時に 1 回読む。ファイルは作らず、書き換えない。
//!   問題（読めない・不正な項目・登録できないホットキー・未設定のホットキー）は、まとめて 1 回だけ通知する。
//! - 通知は `notify_user` から出す。設定で通知をオフにしても、止めるのは成功の通知だけ。

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use shunsoku_core::APP_NAME;
use shunsoku_core::capture::request::{CaptureRequest, Source};
use shunsoku_core::geometry::Point;
use shunsoku_core::i18n::Catalog;
use shunsoku_core::image::{Fragment, Image};
use shunsoku_core::scrap::{Appearance, ScrapModel};
use shunsoku_core::selection::{Button, Input, Selection, State};
use shunsoku_core::settings::{self, LogLevel, Settings, SettingsIssue};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    IDI_APPLICATION, KillTimer, LoadIconW, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WM_ACTIVATEAPP, WM_APP,
    WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_TIMER, WNDCLASSEXW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use crate::{
    grab, hotkey, logger, notify, overlay, panic_hook, render, scrap_window, single_instance, tray,
};

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

/// アプリの状態。UI スレッドだけが持つ。drop の順（宣言の順）でトレイを消し、範囲選択の画面を閉じ、
/// ホットキーの登録を外してからウィンドウを壊す。
struct App {
    tray: tray::Tray,
    settings: Settings,
    capture: Capture,
    scraps: Vec<Scrap>,
    gpu: grab::Gpu,
    hotkeys: hotkey::Registered,
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
        request: CaptureRequest,
        overlay: overlay::Overlay,
        selection: Selection,
    },
}

/// 浮かべているスクラップ 1 枚。
struct Scrap {
    /// drop でウィンドウを壊す。
    #[expect(dead_code, reason = "スクラップの操作のチケット（#42）で使う")]
    window: scrap_window::ScrapWindow,
    #[expect(dead_code, reason = "スクラップの操作のチケット（#42）で使う")]
    model: ScrapModel,
    #[expect(dead_code, reason = "コピーと保存のチケット（#44）で使う")]
    image: Image,
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
    // 設定ファイルとログを置くフォルダ
    let app_dir = std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join(APP_NAME));
    let log_path = app_dir.as_deref().and_then(|dir| logger::init(dir).ok());
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

    let settings_path = app_dir.map(|dir| dir.join("settings.toml"));
    let (settings, issues) = read_settings(settings_path.as_deref());
    log::set_max_level(level_filter(settings.log_level));

    let (app, failed) = match start(instance, settings) {
        Ok(started) => started,
        Err(e) => {
            log::error!("failed to start: {e}");
            return 1;
        }
    };
    if let Some((level, body)) =
        startup_notice(&issues, &failed, &app.settings, settings_path.as_deref())
    {
        notify_user(&app, level, &body);
    }
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

/// 起動する。登録できなかったホットキーも返す。
fn start(
    instance: Option<single_instance::Guard>,
    settings: Settings,
) -> Result<(App, Vec<hotkey::Failed>)> {
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
    scrap_window::register_class(module, Some(scrap_wndproc))?;

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

    let (hotkeys, failed) = hotkey::register(windows.message, &settings.hotkeys);
    for f in &failed {
        log::warn!(
            "could not register the hotkey {} for {:?}: {}",
            f.chord,
            f.action,
            f.error
        );
    }

    let app = App {
        tray,
        settings,
        capture: Capture::Idle,
        scraps: Vec::new(),
        gpu: grab::Gpu::default(),
        hotkeys,
        windows,
        module,
        _instance: instance,
    };
    Ok((app, failed))
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

/// スクラップのウィンドウのプロシージャ。
unsafe extern "system" fn scrap_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 受け取った引数をそのまま渡す
    unsafe { dispatch(on_scrap_message, hwnd, msg, wparam, lparam) }
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
        WM_HOTKEY => {
            on_hotkey(wparam);
            Some(LRESULT(0))
        }
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
fn on_overlay_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let at = || overlay::screen_point(hwnd, lparam);
    match msg {
        WM_LBUTTONDOWN => {
            // ウィンドウの外（別のモニタ）まで引いても離したことがわかるように、マウスを捕まえる
            // SAFETY: 自分のウィンドウ
            unsafe { SetCapture(hwnd) };
            on_selection_input(Input::Press {
                button: Button::Left,
                at: at(),
            });
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            on_selection_input(Input::Move { at: at() });
            Some(LRESULT(0))
        }
        WM_LBUTTONUP => {
            let at = at();
            // SAFETY: UI スレッドで呼ぶ。App を借りる前に放す（WM_CAPTURECHANGED が届くため）
            let _ = unsafe { ReleaseCapture() };
            on_selection_input(Input::Release {
                button: Button::Left,
                at,
            });
            Some(LRESULT(0))
        }
        WM_RBUTTONDOWN => {
            on_selection_input(Input::Press {
                button: Button::Right,
                at: at(),
            });
            Some(LRESULT(0))
        }
        WM_KEYDOWN if wparam.0 == usize::from(VK_ESCAPE.0) => {
            on_selection_input(Input::Escape);
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

/// スクラップのウィンドウへのメッセージ。
fn on_scrap_message(_hwnd: HWND, msg: u32, _wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        // 画素を等倍のまま出すので、DPI の違うモニタに動いても大きさを変えない
        WM_DPICHANGED => Some(LRESULT(0)),
        _ => None,
    }
}

/// 範囲選択の入力を状態に渡し、描き直す。確定したら撮って出力し、取り消したら画面を閉じる。
fn on_selection_input(input: Input) {
    let state = with_app(|app| {
        let Capture::Selecting {
            overlay, selection, ..
        } = &mut app.capture
        else {
            return None;
        };
        let before = selection.rect();
        let state = selection.handle(input);
        if selection.rect() != before {
            // 描くだけでウィンドウは作らないので、借りたまま描いてよい
            overlay.redraw(selection);
        }
        Some(state)
    })
    .flatten();
    match state {
        Some(State::Confirmed) => finish_selection(),
        Some(State::Cancelled) => {
            log::info!("overlay cancelled");
            close_overlay();
        }
        _ => {}
    }
}

/// 確定した範囲を撮り、依頼の出力（M1 ではスクラップ）を出して、範囲選択の画面を閉じる。
fn finish_selection() {
    let taken = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Selecting {
            request,
            overlay,
            selection,
        } => Some((
            request,
            overlay,
            selection,
            app.module,
            Appearance::from(app.settings.scrap),
        )),
        other => {
            app.capture = other;
            None
        }
    })
    .flatten();
    let Some((request, overlay, selection, module, appearance)) = taken else {
        return;
    };
    let Some(rect) = selection.rect() else {
        drop(overlay);
        end_capture();
        return;
    };
    let fragments: Vec<Fragment> = selection
        .parts()
        .iter()
        .filter_map(|part| match overlay.read_back(part) {
            Ok(fragment) => fragment,
            Err(e) => {
                // 撮れなかった部分は透明になる
                log::warn!("could not read back monitor {}: {e}", part.monitor);
                None
            }
        })
        .collect();
    let image = Image::compose(rect, &fragments);
    log::info!(
        "captured {}x{} at ({}, {}) from {} part(s)",
        rect.width(),
        rect.height(),
        rect.left,
        rect.top,
        fragments.len()
    );

    // ウィンドウを作るとメッセージが届くので、App を借りずに作る。前面にする権利を失わないよう、
    // スクラップを前面にしてから範囲選択の画面を閉じる
    let scrap = if request.outputs.scrap {
        let focus = request.outputs.focus_scrap;
        match open_scrap(module, rect.origin(), image, &appearance, focus) {
            Ok(scrap) => Some(scrap),
            Err(e) => {
                log::error!("could not open a scrap: {e}");
                None
            }
        }
    } else {
        None
    };
    drop(overlay);
    end_capture();
    if let Some(scrap) = scrap {
        let rejected = with_app(|app| {
            app.scraps.push(scrap);
        });
        if rejected.is_none() {
            log::warn!("could not keep the scrap");
        }
    }
    log::info!("overlay closed");
}

/// 画像をスクリーン座標の `at` に等倍で浮かべるスクラップを作る。
fn open_scrap(
    module: HINSTANCE,
    at: Point,
    image: Image,
    appearance: &Appearance,
    focus: bool,
) -> Result<Scrap> {
    let model = ScrapModel::new(at, image.size());
    let rect = model.window_rect(appearance);
    let canvas = render::Canvas::new(rect.size())?;
    render::Renderer::new()?.paint_scrap(&canvas, &image, appearance)?;
    let window = scrap_window::ScrapWindow::create(module, rect)?;
    // レイヤード ウィンドウは中身を写して持つので、canvas はここで捨ててよい
    window.present(canvas.dc(), rect, appearance.opacity)?;
    window.show(focus);
    Ok(Scrap {
        window,
        model,
        image,
    })
}

/// 発生源からキャプチャの依頼を受ける。ほかの依頼を進めている間（範囲選択の画面を開いている間など）は無視する。
///
/// spec の「残りは順に待たせる」ための待ち行列は M1 では作らない。M1 の依頼は `delay` が 0 で、
/// 待機と取得と範囲選択の画面を開くまでが 1 つのメッセージの処理の中で終わる（その間に次の依頼は届かない）。
/// ほかの依頼が届きうるのは範囲選択の画面を開いている間だけで、spec はそのときのホットキーを無視すると決めている
/// （トレイは画面に覆われて押せない）。待ち行列は、`delay` や CLI の依頼が入る M3 で `Capture` に足す。
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
    let selection = overlay.new_selection();
    let rejected = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Waiting(request) => {
            app.capture = Capture::Selecting {
                request,
                overlay,
                selection,
            };
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

/// `WM_HOTKEY`。範囲選択の画面を開いている間に押されたものは `request_capture` が無視する。
fn on_hotkey(wparam: WPARAM) {
    let action = with_app(|app| app.hotkeys.action(wparam)).flatten();
    match action {
        Some(settings::HotkeyAction::SelectCapture) => request_capture(Source::Hotkey),
        None => log::warn!("unknown hotkey id {}", wparam.0),
    }
}

/// 設定ファイルを読む。ファイルがなければ既定値で、問題にしない。読めなければ既定値で、TOML として読めない
/// ときと同じ問題にする。
fn read_settings(path: Option<&Path>) -> (Settings, Vec<SettingsIssue>) {
    let Some(path) = path else {
        log::warn!("APPDATA is not set; using the default settings");
        return (Settings::default(), Vec::new());
    };
    let (settings, issues) = match std::fs::read_to_string(path) {
        Ok(src) => settings::load(&src),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("no settings file at {}; using the defaults", path.display());
            return (Settings::default(), Vec::new());
        }
        Err(e) => {
            let issue = SettingsIssue::NotToml {
                message: e.to_string(),
            };
            (Settings::default(), vec![issue])
        }
    };
    log::info!("settings loaded from {}", path.display());
    for issue in &issues {
        log::warn!("settings issue: {issue:?}");
    }
    (settings, issues)
}

fn level_filter(level: LogLevel) -> log::LevelFilter {
    match level {
        LogLevel::Off => log::LevelFilter::Off,
        LogLevel::Error => log::LevelFilter::Error,
        LogLevel::Warn => log::LevelFilter::Warn,
        LogLevel::Info => log::LevelFilter::Info,
        LogLevel::Debug => log::LevelFilter::Debug,
        LogLevel::Trace => log::LevelFilter::Trace,
    }
}

/// 起動時の設定とホットキーの問題を、1 つの通知の本文にまとめる。問題がなければ None。
/// 通知は前の通知を置き換えるので、別々に出すと最後の 1 つしか見えない。
fn startup_notice(
    issues: &[SettingsIssue],
    failed: &[hotkey::Failed],
    settings: &Settings,
    path: Option<&Path>,
) -> Option<(notify::Level, String)> {
    let mut level = notify::Level::Warning;
    let mut lines = Vec::new();
    let mut invalid = Vec::new();
    for issue in issues {
        match issue {
            SettingsIssue::NotToml { .. } => {
                level = notify::Level::Error;
                lines.push(text("settings.unreadable").to_owned());
            }
            SettingsIssue::Invalid { item } => invalid.push(item.as_str()),
        }
    }
    if !invalid.is_empty() {
        lines.push(format!(
            "{}{}",
            text("settings.invalid_items"),
            invalid.join(", ")
        ));
    }
    if !failed.is_empty() {
        let chords: Vec<String> = failed.iter().map(|f| f.chord.to_string()).collect();
        lines.push(format!("{}{}", text("hotkey.failed"), chords.join(", ")));
    }
    if settings.hotkeys.select_capture.is_none() {
        lines.push(text("hotkey.unset").to_owned());
    }
    if lines.is_empty() {
        return None;
    }
    if let Some(path) = path {
        lines.push(path.display().to_string());
    }
    Some((level, lines.join("\n")))
}

/// トレイの通知を出す。設定で通知をオフにしたときは、成功の通知だけを止める（エラーと警告は出す）。
fn notify_user(app: &App, level: notify::Level, body: &str) {
    if matches!(level, notify::Level::Success) && !app.settings.general.notifications {
        return;
    }
    if !notify::show(
        app.windows.tray_owner,
        tray::ICON_ID,
        level,
        text("notify.title"),
        body,
    ) {
        log::warn!("could not show a notification: {body}");
    }
}
