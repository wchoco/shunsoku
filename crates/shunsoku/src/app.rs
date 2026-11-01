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
//! - スクラップへのキーとマウスと右クリックメニューは、core の `scrap::ops` の表（ショートカットの対応表
//!   `App::shortcuts`、`ops::mouse_operation`、`ops::menu`）でスクラップ操作に変え、`run_scrap_operation` で
//!   実行する。操作が返す `Effect` を実行するのはここ（`run_scrap_operation`）。操作を足すときは core の表に
//!   行を足し、新しい `Effect` があればそこに足す。
//! - 設定（`%APPDATA%\shunsoku\settings.toml`）は起動時に読み、`settings_watch` の知らせ（`WM_APP_SETTINGS`）で
//!   読み込み直す。ファイルは作らず、書き換えない。読み込み直しでは core の `settings::diff` で変わった部分を調べ、
//!   ホットキーの登録し直し・スクラップの描き直しなど、変わったものだけを反映する（`on_settings_changed`）。
//!   保存先・ファイル名テンプレート・通知のオン/オフは、使うときに `App::settings` から読むので、次から効く。
//!   キーの割り当てと重なりは core の `settings::keymap` で決める。
//!   問題（読めない・不正な項目・重なったキー・登録できないホットキー・未設定のホットキー）は、まとめて 1 回だけ通知する。
//! - 通知は `notify_user` から出す。設定で通知をオフにしても、止めるのは成功の通知だけ。通知のクリックは
//!   中身を持たないので、クリックで開くフォルダ（`App::notice_folder`）を覚えておき、新しい通知を出したら忘れる。
//! - コピーと保存の画像は `render::render_image` で表示と同じ描き方で作る。保存先とファイル名は UI スレッドで
//!   決め（`save::destination`）、PNG の書き出しと書き込みは裏のスレッドで行い、結果は `WM_APP_SAVED` で受ける。
//!   結果を受けるまでの保存先は `App::pending_saves` に持ち、続けて保存しても同じ名前にしない。
//!   保存先とファイル名テンプレートは設定（`[general]`）のものを使う。
//! - 閉じたスクラップ（`Effect::MoveToTrash`、Alt+F4 も）は `App::trash`（`trash_store`）に閉じる直前の
//!   `ScrapModel` のまま入れる。画素は裏のスレッドで一時ファイルに書き、書き終えたら `WM_APP_TRASH` で受ける。
//!   トレイメニューの「ごみ箱」から選ぶと、その状態のまま浮かべ直してフォーカスを移す（`restore_scrap`）。

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Instant;

use shunsoku_core::APP_NAME;
use shunsoku_core::capture::request::{CaptureRequest, Source};
use shunsoku_core::geometry::Point;
use shunsoku_core::i18n::Catalog;
use shunsoku_core::image::{Fragment, Image};
use shunsoku_core::keys::{Chord, Key, Modifiers};
use shunsoku_core::perf::Mark;
use shunsoku_core::scrap::ops::{
    self, ChoiceLabel, Effect, MenuEntry, MouseGesture, OpContext, OperationDescriptor,
    OperationId, Shortcuts,
};
use shunsoku_core::scrap::{Appearance, Drag, ScrapModel, Wheel};
use shunsoku_core::selection::{Button, Direction, Input, Selection, State};
use shunsoku_core::settings::{self, LogLevel, Settings, SettingsChanges, SettingsIssue};
use shunsoku_core::trash::TrashId;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_A, VK_CONTROL, VK_DOWN,
    VK_ESCAPE, VK_LEFT, VK_LWIN, VK_MENU, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessagePos, GetMessageW,
    HWND_MESSAGE, IDI_APPLICATION, KillTimer, LoadIconW, MSG, PostMessageW, PostQuitMessage,
    RegisterClassExW, RegisterWindowMessageW, SetCursorPos, SetTimer, TranslateMessage,
    WINDOW_EX_STYLE, WM_ACTIVATEAPP, WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_CONTEXTMENU,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
    WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

use crate::{
    clipboard, grab, hotkey, logger, menu, notify, overlay, panic_hook, perf, render, save,
    scrap_window, settings_watch, single_instance, trash_store, tray,
};

/// トレイのアイコンへの操作（`tray::event` で読む）。
const WM_APP_TRAY: u32 = WM_APP + 1;
/// 環境変数 `SHUNSOKU_TEST_PANIC=ui` のとき、UI スレッドでパニックを起こす（手で確かめる用。デバッグ ビルドだけ）。
#[cfg(debug_assertions)]
const WM_APP_TEST_PANIC: u32 = WM_APP + 2;
/// 保存の裏のスレッドが書き終えた（結果は `App::saves` の受け口に入っている）。
const WM_APP_SAVED: u32 = WM_APP + 3;
/// 設定ファイルの監視が知らせを入れた（中身は `App::settings_events` の受け口に入っている）。
const WM_APP_SETTINGS: u32 = WM_APP + 4;
/// ごみ箱の画素を一時ファイルに書き終えた（結果は `App::trash` の受け口に入っている）。
const WM_APP_TRASH: u32 = WM_APP + 5;

/// 2 つのウィンドウのクラス名。2 つ目のプロセスはこの名前でメッセージ専用ウィンドウを探す。
const CLASS_NAME: PCWSTR = w!("shunsoku.app");
/// 2 つ目のプロセスが送る登録メッセージの名前。
const SECOND_INSTANCE_MESSAGE: PCWSTR = w!("shunsoku.SecondInstance");

/// トレイのメニューの項目の番号。
const CMD_EXIT: u32 = 1;
const CMD_SELECT_CAPTURE: u32 = 2;
const CMD_CLOSE_ALL: u32 = 3;
const CMD_HIDE_ALL: u32 = 4;
const CMD_SHOW_ALL: u32 = 5;
/// ごみ箱の一覧の項目の番号は、これに一覧の添字を足したもの。
const CMD_TRASH_FIRST: u32 = 100;

/// 依頼の `delay` を待つタイマーの番号（メッセージ専用ウィンドウの WM_TIMER）。
const TIMER_CAPTURE_DELAY: usize = 1;
/// 倍率の表示を消すタイマーの番号（スクラップのウィンドウの WM_TIMER）と、出しておく時間。
const TIMER_ZOOM_LABEL: usize = 1;
const ZOOM_LABEL_MS: u32 = 1000;

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

/// アプリの状態。UI スレッドだけが持つ。drop の順（宣言の順）で設定ファイルの監視を止め、トレイを消し、
/// 範囲選択の画面を閉じ、ホットキーの登録を外してからウィンドウを壊す。
struct App {
    /// 設定ファイルの監視。`APPDATA` がないときや始められなかったときは None。
    watcher: Option<settings_watch::Watcher>,
    tray: tray::Tray,
    /// トレイの登録に 1 度でも成功したか。性能の区切り `TrayRegistered` は最初の成功のときだけ書く。
    tray_registered: bool,
    /// いま使っている設定（読み込み直しで TOML として読めなかったときは、直前の有効な設定のまま）。
    settings: Settings,
    /// 設定ファイルのパス。通知に出す。
    settings_path: Option<PathBuf>,
    /// 設定ファイルの監視が知らせを入れる口。
    settings_events: mpsc::Receiver<settings_watch::Event>,
    /// スクラップ操作のショートカットの対応表（表の既定値に `settings.shortcuts` を重ねたもの）。
    shortcuts: Shortcuts,
    capture: Capture,
    scraps: Vec<Scrap>,
    /// 閉じたスクラップ。drop で一時ファイルを消す。
    trash: trash_store::TrashStore,
    /// 保存の裏のスレッドが結果を入れる口と、UI スレッドが受ける口。
    saves: (mpsc::Sender<Saved>, mpsc::Receiver<Saved>),
    /// 名前を決めて裏のスレッドに渡し、まだ結果（`WM_APP_SAVED`）を受けていない保存先。次の名前を決めるとき、
    /// 使われている名前として数える（`save::destination`）。
    pending_saves: Vec<PathBuf>,
    /// スクラップの中身を描く Direct2D の描き手。描き直しのたびに作らないよう使い回す（`paint_scrap`）。
    renderer: Option<render::Renderer>,
    /// いま出ている通知をクリックしたときに開くフォルダ（保存の通知のときだけ）。
    notice_folder: Option<PathBuf>,
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

/// 保存の裏のスレッドの結果。
struct Saved {
    path: PathBuf,
    /// 失敗の理由（裏のスレッドのパニックも失敗にする）。
    result: std::result::Result<(), String>,
}

/// 浮かべているスクラップ 1 枚。
struct Scrap {
    /// drop でウィンドウを壊す。
    window: scrap_window::ScrapWindow,
    model: ScrapModel,
    image: Image,
    /// 左ドラッグ。
    drag: Drag,
    /// ホイールの回転（1 ノッチに満たない量をためる）。
    wheel: Wheel,
    /// 倍率を出しているか（`Effect::ShowZoom` から `TIMER_ZOOM_LABEL` が鳴るまで）。
    zoom_label: bool,
}

impl Scrap {
    /// `Effect::Redraw`: `model` のとおりに出し直す。表示の大きさとモードが `before` と同じで、`repaint` でも
    /// なければ、中身は変わらないのでウィンドウを動かすだけにする（ドラッグのたびに描き直さない）。
    fn redraw(
        &self,
        before: &ScrapModel,
        appearance: &Appearance,
        renderer: &mut Option<render::Renderer>,
        repaint: bool,
    ) -> Result<()> {
        let same_content = self.model.image_size == before.image_size
            && self.model.display_size() == before.display_size()
            && self.model.mode == before.mode
            && self.model.appearance_overrides == before.appearance_overrides;
        if same_content && !repaint {
            self.window.move_to(self.model.window_rect(appearance))
        } else {
            self.paint(appearance, renderer)
        }
    }

    /// 中身を描いて、`model` のとおりの位置と大きさに出す。
    fn paint(
        &self,
        appearance: &Appearance,
        renderer: &mut Option<render::Renderer>,
    ) -> Result<()> {
        let label = self.zoom_label.then(|| render::ZoomLabel {
            percent: self.model.zoom(),
            scale: self.window.dpi() as f32 / 96.0,
        });
        paint_scrap(
            renderer,
            &self.window,
            &self.image,
            &self.model,
            appearance,
            label,
        )
    }
}

/// スクラップの中身を描いて、ウィンドウを `model` の矩形（`ScrapModel::window_rect`）に出す。`appearance` は全体の
/// 見た目の既定値で、スクラップごとの上書きを重ねて描く。
///
/// `renderer` は使い回す描き手（`App::renderer`）。なければ作って入れる。描けなかったときは
/// （`D2DERR_RECREATE_TARGET` など）次に作り直せるように入れない。
fn paint_scrap(
    renderer: &mut Option<render::Renderer>,
    window: &scrap_window::ScrapWindow,
    image: &Image,
    model: &ScrapModel,
    appearance: &Appearance,
    label: Option<render::ZoomLabel>,
) -> Result<()> {
    let rect = model.window_rect(appearance);
    let appearance = &model.appearance(appearance);
    let canvas = render::Canvas::new(rect.size())?;
    let mut painter = match renderer.take() {
        Some(painter) => painter,
        None => render::Renderer::new()?,
    };
    painter.paint_scrap(&canvas, image, model.display_size(), appearance, label)?;
    *renderer = Some(painter);
    // レイヤード ウィンドウは中身を写して持つので、canvas はここで捨ててよい
    window.present(canvas.dc(), rect, appearance.opacity)
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
    let perf_started = perf::start();
    // 設定ファイルとログを置くフォルダ
    let app_dir = std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join(APP_NAME));
    let log_path = app_dir.as_deref().and_then(|dir| logger::init(dir).ok());
    log::info!("starting {APP_NAME} v{}", env!("CARGO_PKG_VERSION"));
    if let Err(e) = perf_started {
        log::warn!("could not open the perf log: {e}");
    }

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

    // 通知のクリックでフォルダを開く ShellExecuteW のため
    // SAFETY: UI スレッドで 1 回呼び、終わるときに CoUninitialize で対にする
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if com.is_err() {
        log::warn!("could not initialize COM: {com:?}");
    }

    let settings_path = app_dir.map(|dir| dir.join("settings.toml"));
    let (settings, issues) = read_settings(settings_path.as_deref());
    log::set_max_level(level_filter(settings.log_level));

    let unset = settings.hotkeys.select_capture.is_none();
    let (app, key_issues, failed) = match start(instance, settings, settings_path) {
        Ok(started) => started,
        Err(e) => {
            log::error!("failed to start: {e}");
            return 1;
        }
    };
    let issues: Vec<SettingsIssue> = issues.into_iter().chain(key_issues).collect();
    let notice = settings_notice(
        Moment::Startup,
        &issues,
        &failed,
        unset,
        app.settings_path.as_deref(),
    );
    #[cfg(debug_assertions)]
    start_test_panic(app.windows.message);
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    if let Some((level, body)) = notice {
        notify_user(level, &body);
    }
    log::info!("started");

    let code = message_loop();
    // トレイを消してからウィンドウを壊す。APP から出してから drop するので、
    // 壊すときに呼ばれるウィンドウ プロシージャは App を見ない
    let app = APP.with(|cell| cell.borrow_mut().take());
    drop(app);
    if com.is_ok() {
        // SAFETY: 成功した CoInitializeEx と対にする
        unsafe { CoUninitialize() };
    }
    log::info!("exiting (code {code})");
    code
}

/// 手で確かめる用のパニック（デバッグ ビルドだけ）。環境変数 `SHUNSOKU_TEST_PANIC` が `ui` なら UI スレッドで、
/// `worker` なら裏のスレッドでパニックを起こす。UI スレッドの分はメッセージ ループが回ってから起きる。
#[cfg(debug_assertions)]
fn start_test_panic(message_window: HWND) {
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
}

/// 起動する。キーの割り当ての問題（`settings::keymap`）と、登録できなかったホットキーも返す。
fn start(
    instance: Option<single_instance::Guard>,
    settings: Settings,
    settings_path: Option<PathBuf>,
) -> Result<(App, Vec<SettingsIssue>, Vec<hotkey::Failed>)> {
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
    let tray_registered = tray.register();
    if tray_registered {
        perf::mark(Mark::TrayRegistered);
        log::info!("tray icon registered");
    } else {
        log::warn!("could not register the tray icon; waiting for TaskbarCreated");
    }

    let keymap = settings::keymap(&settings, ops::OPERATIONS);
    log_issues(&keymap.issues);
    let (hotkeys, failed) = hotkey::register(windows.message, &keymap.hotkeys.bound);
    log_failed(&failed);

    let (sender, settings_events) = mpsc::channel();
    let watcher = settings_path.as_deref().and_then(|path| {
        settings_watch::start(path, sender, windows.message, WM_APP_SETTINGS)
            .inspect_err(|e| log::error!("could not start watching the settings: {e}"))
            .ok()
    });

    let trash =
        trash_store::TrashStore::new(settings.general.trash_limit, windows.message, WM_APP_TRASH);
    let app = App {
        watcher,
        tray,
        tray_registered,
        settings,
        settings_path,
        settings_events,
        shortcuts: keymap.shortcuts,
        capture: Capture::Idle,
        scraps: Vec::new(),
        trash,
        saves: mpsc::channel(),
        pending_saves: Vec::new(),
        renderer: None,
        notice_folder: None,
        gpu: grab::Gpu::default(),
        hotkeys,
        windows,
        module,
        _instance: instance,
    };
    Ok((app, keymap.issues, failed))
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
        #[cfg(debug_assertions)]
        WM_APP_TEST_PANIC => panic!("test panic on the UI thread (SHUNSOKU_TEST_PANIC)"),
        WM_APP_SAVED => {
            on_saved();
            Some(LRESULT(0))
        }
        WM_APP_SETTINGS => {
            on_settings_changed();
            Some(LRESULT(0))
        }
        WM_APP_TRASH => {
            with_app(|app| app.trash.on_written());
            Some(LRESULT(0))
        }
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
            // ログオフ・シャットダウン。戻るとプロセスは終わらされる（Explorer も終わるのでトレイは消さない）。
            // drop されないので、ごみ箱の一時ファイルはここで消す
            with_app(|app| app.trash.shutdown());
            log::info!("exiting (session end)");
            Some(LRESULT(0))
        }
        // 登録メッセージは定数でないので match の外で比べる
        _ if Some(&msg) == TASKBAR_CREATED.get() => {
            let ok = with_app(|app| {
                let ok = app.tray.register();
                // 起動時に登録できず、ここで初めて登録できたときも、コールドスタートの終わりとして書く
                if ok && !app.tray_registered {
                    app.tray_registered = true;
                    perf::mark(Mark::TrayRegistered);
                }
                ok
            });
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
            // ごみ箱の一覧（新しいものから）。項目の番号は CMD_TRASH_FIRST + 添字
            let trashed: Vec<(TrashId, String)> = with_app(|app| {
                app.trash
                    .list()
                    .iter()
                    .map(|listed| (listed.id, trash_label(listed)))
                    .collect()
            })
            .unwrap_or_default();
            let mut trash_entries: Vec<menu::Entry> = trashed
                .iter()
                .enumerate()
                .map(|(i, (_, label))| menu::Entry::Item {
                    id: CMD_TRASH_FIRST + i as u32,
                    label: label.as_str().into(),
                    enabled: true,
                    checked: false,
                })
                .collect();
            if trash_entries.is_empty() {
                trash_entries.push(menu::Entry::Item {
                    id: CMD_TRASH_FIRST,
                    label: text("tray.menu.trash_empty").into(),
                    enabled: false,
                    checked: false,
                });
            }
            // 1 枚でも隠れていれば「すべて表示」、なければ「すべて隠す」
            let (any_scrap, any_hidden) = with_app(|app| {
                (
                    !app.scraps.is_empty(),
                    app.scraps.iter().any(|s| !s.window.is_visible()),
                )
            })
            .unwrap_or_default();
            let hide_or_show = if any_hidden {
                menu::Entry::Item {
                    id: CMD_SHOW_ALL,
                    label: text("tray.menu.show_all").into(),
                    enabled: true,
                    checked: false,
                }
            } else {
                menu::Entry::Item {
                    id: CMD_HIDE_ALL,
                    label: text("tray.menu.hide_all").into(),
                    enabled: any_scrap,
                    checked: false,
                }
            };
            let items = [
                menu::Entry::Item {
                    id: CMD_SELECT_CAPTURE,
                    label: text("tray.menu.select_capture").into(),
                    enabled: true,
                    checked: false,
                },
                menu::Entry::Submenu {
                    label: text("tray.menu.trash").into(),
                    enabled: true,
                    items: trash_entries,
                },
                menu::Entry::Item {
                    id: CMD_CLOSE_ALL,
                    label: text("tray.menu.close_all").into(),
                    enabled: any_scrap,
                    checked: false,
                },
                hide_or_show,
                menu::Entry::Item {
                    id: CMD_EXIT,
                    label: text("tray.menu.exit").into(),
                    enabled: true,
                    checked: false,
                },
            ];
            // モーダルなので App を借りずに呼ぶ
            let Some(command) = menu::show(owner, &items, x, y) else {
                return;
            };
            match command.checked_sub(CMD_TRASH_FIRST) {
                Some(i) => {
                    if let Some(&(id, _)) = trashed.get(i as usize) {
                        restore_scrap(id);
                    }
                }
                None => on_command(command),
            }
        }
        tray::Event::Select => request_capture(Source::TrayClick),
        tray::Event::NotificationClick => {
            let folder = with_app(|app| app.notice_folder.clone()).flatten();
            // ShellExecuteW はメッセージを回すことがあるので、App を借りずに呼ぶ
            if let Some(folder) = folder
                && !save::open_folder(&folder)
            {
                log::warn!("could not open the folder {}", folder.display());
            }
        }
        tray::Event::Other => {}
    }
}

fn on_command(command: u32) {
    match command {
        CMD_SELECT_CAPTURE => request_capture(Source::TrayMenu),
        CMD_CLOSE_ALL => close_all_scraps(),
        CMD_HIDE_ALL => set_scraps_visible(false),
        CMD_SHOW_ALL => set_scraps_visible(true),
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
        WM_KEYDOWN => {
            if let Some(input) = selection_key(wparam, lparam) {
                on_selection_input(input);
            }
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

/// スクラップのウィンドウへのメッセージ。キーとマウスは対応表でスクラップ操作に変えて実行する。
/// キーはフォーカスのあるスクラップにだけ届くので、ショートカットはそのときだけ効く。
fn on_scrap_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_LBUTTONDOWN => {
            // カーソルがウィンドウの外へ速く動いても追えるように、マウスを捕まえる
            // SAFETY: 自分のウィンドウ
            unsafe { SetCapture(hwnd) };
            let at = message_screen_point();
            with_app(|app| {
                if let Some(scrap) = app.scrap_mut(hwnd) {
                    scrap.drag.start(at);
                }
            });
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            on_scrap_drag(hwnd, wparam, message_screen_point());
            Some(LRESULT(0))
        }
        WM_LBUTTONUP => {
            // ドラッグは WM_CAPTURECHANGED で終える。App を借りる前に放す（そのメッセージが届くため）
            // SAFETY: UI スレッドで呼ぶ
            let _ = unsafe { ReleaseCapture() };
            Some(LRESULT(0))
        }
        WM_CAPTURECHANGED => {
            with_app(|app| {
                if let Some(scrap) = app.scrap_mut(hwnd) {
                    scrap.drag.end();
                }
            });
            Some(LRESULT(0))
        }
        WM_MBUTTONUP => {
            // 押したときに閉じると、離したときのメッセージが下のウィンドウに届くので、離したときに閉じる
            run_mouse_operation(hwnd, MouseGesture::MiddleClick, OpContext::default());
            Some(LRESULT(0))
        }
        WM_MOUSEWHEEL => on_scrap_wheel(hwnd, wparam, lparam).then_some(LRESULT(0)),
        WM_TIMER if wparam.0 == TIMER_ZOOM_LABEL => {
            hide_zoom_label(hwnd);
            Some(LRESULT(0))
        }
        // Alt との組み合わせは WM_SYSKEYDOWN で来る。対応表にないキーは DefWindowProcW に任せる
        WM_KEYDOWN | WM_SYSKEYDOWN => on_scrap_key(hwnd, wparam).then_some(LRESULT(0)),
        // 右クリック（DefWindowProcW が WM_RBUTTONUP から作る）と Shift+F10・アプリケーション キー
        WM_CONTEXTMENU => {
            show_scrap_menu(hwnd, lparam);
            Some(LRESULT(0))
        }
        // Alt+F4 など。ウィンドウだけ壊されて App に残らないように、閉じるのと同じにする
        WM_CLOSE => {
            close_scrap(hwnd);
            Some(LRESULT(0))
        }
        // 画素を等倍のまま出すので、DPI の違うモニタに動いても大きさを変えない
        WM_DPICHANGED => Some(LRESULT(0)),
        _ => None,
    }
}

/// いま処理しているメッセージが送られたときのカーソルの位置（スクリーン座標、物理ピクセル）。
/// ドラッグ中はウィンドウが動くので、クライアント座標ではなくこれを使う。
fn message_screen_point() -> Point {
    // SAFETY: 引数なし
    let pos = unsafe { GetMessagePos() };
    // 左や上のモニタでは負になるので、符号付き 16 ビットとして読む
    Point::new(
        (pos & 0xFFFF) as u16 as i16 as i32,
        (pos >> 16) as u16 as i16 as i32,
    )
}

impl App {
    fn scrap_mut(&mut self, hwnd: HWND) -> Option<&mut Scrap> {
        self.scraps.iter_mut().find(|s| s.window.hwnd() == hwnd)
    }
}

/// 左ドラッグの間のカーソルの移動を、ドラッグの操作に渡す。
///
/// 左ボタンを押していて、このウィンドウがマウスを捕まえているときだけ動かす。そうでなければ
/// （離したことを取りこぼしたなど）ドラッグを終える。ホバーだけでスクラップが動かないようにするため。
fn on_scrap_drag(hwnd: HWND, wparam: WPARAM, at: Point) {
    // WM_MOUSEMOVE の wparam の MK_LBUTTON（Win32_System_SystemServices を足さないよう値で持つ）
    const MK_LBUTTON: usize = 0x0001;
    let left_down = wparam.0 & MK_LBUTTON != 0;
    // SAFETY: 引数なし
    let captured = unsafe { GetCapture() } == hwnd;
    if !(left_down && captured) {
        let was_dragging = with_app(|app| {
            let scrap = app.scrap_mut(hwnd)?;
            let active = scrap.drag.is_active();
            scrap.drag.end();
            Some(active)
        })
        .flatten()
        .unwrap_or(false);
        if was_dragging {
            log::debug!("scrap drag ended without a button release");
        }
        if captured {
            // App を借りずに放す（WM_CAPTURECHANGED が届くため）
            // SAFETY: UI スレッドで呼ぶ
            let _ = unsafe { ReleaseCapture() };
        }
        return;
    }
    let delta = with_app(|app| app.scrap_mut(hwnd)?.drag.move_to(at)).flatten();
    if let Some(drag) = delta {
        let context = OpContext {
            drag,
            ..OpContext::default()
        };
        run_mouse_operation(hwnd, MouseGesture::LeftDrag, context);
    }
}

/// `WM_MOUSEWHEEL`: 修飾キーなしと Ctrl だけのホイールを、ノッチごとにホイールの操作にする（基準点はカーソルの
/// 位置）。ほかの修飾キーつきは対応がないので false（DefWindowProcW に任せる）。
fn on_scrap_wheel(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    // wparam の下位は押されているキーとボタン（MK_*）、上位は回転量（上が正）
    const MK_SHIFT: usize = 0x0004;
    const MK_CONTROL: usize = 0x0008;
    if wparam.0 & MK_SHIFT != 0 || held_modifiers().alt {
        return false;
    }
    let ctrl = wparam.0 & MK_CONTROL != 0;
    let delta = i32::from((wparam.0 >> 16) as u16 as i16);
    // WM_MOUSEWHEEL の座標はスクリーン座標。左や上のモニタでは負になる
    let cursor = Point::new(
        i32::from((lparam.0 & 0xFFFF) as u16 as i16),
        i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16),
    );
    let Some(notches) = with_app(|app| Some(app.scrap_mut(hwnd)?.wheel.turn(delta))).flatten()
    else {
        return true;
    };
    let gesture = match (ctrl, notches > 0) {
        (false, true) => MouseGesture::WheelUp,
        (false, false) => MouseGesture::WheelDown,
        (true, true) => MouseGesture::CtrlWheelUp,
        (true, false) => MouseGesture::CtrlWheelDown,
    };
    let context = OpContext {
        cursor: Some(cursor),
        ..OpContext::default()
    };
    for _ in 0..notches.unsigned_abs() {
        run_mouse_operation(hwnd, gesture, context);
    }
    true
}

/// `TIMER_ZOOM_LABEL`: 倍率の表示を消して描き直す。
fn hide_zoom_label(hwnd: HWND) {
    // SAFETY: 自分のタイマーを止める
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_ZOOM_LABEL);
    }
    with_app(|app| {
        let appearance = Appearance::from(app.settings.scrap);
        let App {
            scraps, renderer, ..
        } = app;
        let scrap = scraps.iter_mut().find(|s| s.window.hwnd() == hwnd)?;
        if !scrap.zoom_label {
            return None;
        }
        scrap.zoom_label = false;
        // 描くだけでメッセージは届かないので、借りたまま描く
        if let Err(e) = scrap.paint(&appearance, renderer) {
            log::warn!("could not hide the zoom label: {e}");
        }
        Some(())
    });
}

fn run_mouse_operation(hwnd: HWND, gesture: MouseGesture, context: OpContext) {
    if let Some(op) = ops::mouse_operation(ops::OPERATIONS, gesture) {
        run_scrap_operation(hwnd, op, context);
    }
}

/// キーをショートカットの対応表で引いて実行する。対応表になければ false。
fn on_scrap_key(hwnd: HWND, wparam: WPARAM) -> bool {
    // 修飾キーだけのときは名前の表になく、None になる
    let Some(key) = Key::from_vk(wparam.0 as u16) else {
        return false;
    };
    let chord = Chord {
        modifiers: held_modifiers(),
        key,
    };
    let Some(op) = with_app(|app| app.shortcuts.operation(chord)).flatten() else {
        return false;
    };
    run_scrap_operation(hwnd, op, OpContext::default());
    true
}

/// いま押されている修飾キー（キーのメッセージを処理している時点の状態）。
fn held_modifiers() -> Modifiers {
    // SAFETY: 引数は定数
    let down = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
    Modifiers {
        win: down(VK_LWIN.0) || down(VK_RWIN.0),
        ctrl: down(VK_CONTROL.0),
        alt: down(VK_MENU.0),
        shift: down(VK_SHIFT.0),
    }
}

/// 右クリックメニューを表から作って出し、選ばれた操作を実行する。
fn show_scrap_menu(hwnd: HWND, lparam: LPARAM) {
    let built = with_app(|app| {
        let scrap = app.scraps.iter().find(|s| s.window.hwnd() == hwnd)?;
        let defaults = Appearance::from(app.settings.scrap);
        let entries = ops::menu(ops::OPERATIONS, &scrap.model, &app.shortcuts, &defaults);
        Some((entries, scrap.model.position))
    })
    .flatten();
    let Some((entries, image_origin)) = built else {
        return;
    };
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    // キーボードから開いたときは (-1, -1)。画像の左上に出す
    let at = if (x, y) == (-1, -1) {
        image_origin
    } else {
        Point::new(x, y)
    };
    // 項目の番号は commands の添字 + 1（0 は「選ばなかった」）。子メニューの選択肢は選択肢ごとに番号を振る
    let mut commands: Vec<(OperationId, Option<i32>)> = Vec::new();
    let items = scrap_menu_entries(&entries, &mut commands);
    // モーダルなので App を借りずに呼ぶ
    let Some(chosen) = menu::show(hwnd, &items, at.x, at.y) else {
        return;
    };
    let Some(&(id, choice)) = commands.get(chosen as usize - 1) else {
        return;
    };
    if let Some(op) = ops::find(id) {
        let context = OpContext {
            choice,
            ..OpContext::default()
        };
        run_scrap_operation(hwnd, op, context);
    }
}

/// 右クリックメニューの行を Win32 のメニューの行にする。選べる項目には `commands` の添字 + 1 の番号を振る。
fn scrap_menu_entries(
    entries: &[MenuEntry],
    commands: &mut Vec<(OperationId, Option<i32>)>,
) -> Vec<menu::Entry<'static>> {
    fn command(
        commands: &mut Vec<(OperationId, Option<i32>)>,
        id: OperationId,
        choice: Option<i32>,
    ) -> u32 {
        commands.push((id, choice));
        commands.len() as u32
    }
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        items.push(match entry {
            MenuEntry::Item(item) => menu::Entry::Item {
                id: command(commands, item.id, None),
                label: match item.shortcut {
                    Some(chord) => format!("{}\t{chord}", text(item.label_key)).into(),
                    None => text(item.label_key).into(),
                },
                enabled: item.enabled,
                checked: false,
            },
            MenuEntry::Submenu(submenu) => menu::Entry::Submenu {
                label: text(submenu.label_key).into(),
                enabled: submenu.enabled,
                items: submenu
                    .choices
                    .iter()
                    .map(|choice| menu::Entry::Item {
                        id: command(commands, submenu.id, Some(choice.value)),
                        label: match choice.label {
                            ChoiceLabel::Key(key) => text(key).into(),
                            ChoiceLabel::Percent(percent) => format!("{percent}%").into(),
                        },
                        enabled: true,
                        checked: choice.checked,
                    })
                    .collect(),
            },
            MenuEntry::Folder(folder) => menu::Entry::Submenu {
                label: text(folder.label_key).into(),
                enabled: true,
                items: scrap_menu_entries(&folder.entries, commands),
            },
            MenuEntry::Separator => menu::Entry::Separator,
        });
    }
    items
}

/// `App` を返したあとに行う `Effect`。
enum Deferred {
    Copy(Image),
    Save(Image),
    SaveAs(Image),
    /// コピーや保存の画像を作れなかった。`failed_key` はその仕事の失敗の文言のキー。
    RenderFailed {
        failed_key: &'static str,
        reason: String,
    },
    Close,
}

/// コピーや保存の画像を表示と同じ描き方で作り、`job` の `Deferred` にする。作れなければ `failed_key` の失敗にする。
fn render_for(image: &Image, job: fn(Image) -> Deferred, failed_key: &'static str) -> Deferred {
    match render::render_image(image) {
        Ok(image) => job(image),
        Err(e) => Deferred::RenderFailed {
            failed_key,
            reason: e.to_string(),
        },
    }
}

/// スクラップ操作を実行し、返った `Effect` を実行する。スクラップのキー・マウス・メニューはすべてここを通る。
fn run_scrap_operation(hwnd: HWND, op: &OperationDescriptor, context: OpContext) {
    let deferred = with_app(|app| {
        let appearance = Appearance::from(app.settings.scrap);
        let App {
            scraps, renderer, ..
        } = app;
        let scrap = scraps.iter_mut().find(|s| s.window.hwnd() == hwnd)?;
        let before = scrap.model.clone();
        let mut deferred = Vec::new();
        let context = OpContext {
            appearance,
            ..context
        };
        let effects = ops::execute(op, &mut scrap.model, &context);
        let show_zoom = effects.contains(&Effect::ShowZoom);
        if show_zoom {
            scrap.zoom_label = true;
            // 出し直すたびに消すまでの時間を延ばす（同じ番号の SetTimer は置き換わる）
            // SAFETY: 自分のウィンドウ
            if unsafe { SetTimer(Some(hwnd), TIMER_ZOOM_LABEL, ZOOM_LABEL_MS, None) } == 0 {
                log::warn!("could not start the zoom label timer");
            }
        }
        let redraws = effects.contains(&Effect::Redraw);
        for effect in effects {
            match effect {
                // ウィンドウを動かす・描き直すときに届くメッセージ（WM_WINDOWPOSCHANGED、WM_DPICHANGED など）では
                // App を借りないので、借りたままでよい
                Effect::Redraw => {
                    if let Err(e) = scrap.redraw(&before, &appearance, renderer, show_zoom) {
                        // 大きすぎて描けないときなど。ウィンドウは前のままなので、状態も前に戻す
                        log::warn!("could not redraw a scrap: {e}");
                        scrap.model = before.clone();
                    }
                }
                // 倍率が変わらなかった（範囲の端）ときも、倍率だけは出す
                Effect::ShowZoom if !redraws => {
                    if let Err(e) = scrap.paint(&appearance, renderer) {
                        log::warn!("could not show the zoom label: {e}");
                    }
                }
                Effect::ShowZoom => {}
                Effect::MoveToTrash => deferred.push(Deferred::Close),
                // 描くだけでメッセージは届かないので、借りたまま描く
                Effect::CopyToClipboard => {
                    deferred.push(render_for(&scrap.image, Deferred::Copy, "copy.failed"));
                }
                Effect::SaveDefault => {
                    deferred.push(render_for(&scrap.image, Deferred::Save, "save.failed"));
                }
                Effect::ShowSaveDialog => {
                    deferred.push(render_for(&scrap.image, Deferred::SaveAs, "save.failed"));
                }
            }
        }
        Some(deferred)
    })
    .flatten()
    .unwrap_or_default();
    // クリップボード・通知・ウィンドウを壊すのはメッセージが届くことがあるので、App を借りずに行う
    for action in deferred {
        match action {
            Deferred::Copy(image) => copy_to_clipboard(&image),
            Deferred::Save(image) => save_default(image),
            Deferred::SaveAs(image) => save_as(hwnd, image),
            Deferred::RenderFailed { failed_key, reason } => {
                log::error!("could not render the scrap image ({failed_key}): {reason}");
                notify_user(
                    notify::Level::Error,
                    &format!("{}\n{reason}", text(failed_key)),
                );
            }
            Deferred::Close => close_scrap(hwnd),
        }
    }
}

/// `Effect::CopyToClipboard`: 画像をクリップボードに置く。失敗したらエラーの通知を出す。
fn copy_to_clipboard(image: &Image) {
    let Some(owner) = with_app(|app| app.windows.message) else {
        return;
    };
    match clipboard::copy(owner, image) {
        Ok(()) => log::info!(
            "copied {}x{} to the clipboard",
            image.width(),
            image.height()
        ),
        Err(e) => {
            log::error!("could not copy to the clipboard: {e}");
            notify_user(
                notify::Level::Error,
                &format!("{}\n{e}", text("copy.failed")),
            );
        }
    }
}

/// `Effect::SaveDefault`: 全体設定の保存先に、全体のファイル名テンプレートの名前で保存する。
/// 保存先とファイル名はここ（UI スレッド）で決め、PNG の書き出しと書き込みは裏のスレッドで行う。
/// 失敗してもスクラップはそのまま残る。
fn save_default(image: Image) {
    if let Some(path) = default_destination() {
        start_save(path, image, save::Overwrite::Deny);
    }
}

/// `Effect::ShowSaveDialog`: 「名前を付けて保存」のダイアログを出し、選ばれた場所に保存する。初期フォルダと
/// ファイル名は既定の保存（`default_destination`）と同じ。キャンセルしたら何もしない。失敗は既定の保存と同じく通知する。
fn save_as(owner: HWND, image: Image) {
    let Some(proposed) = default_destination() else {
        return;
    };
    // モーダルなので App を借りずに呼ぶ
    match save::ask_path(owner, &proposed, text("save.png_filter")) {
        Ok(Some(path)) => start_save(path, image, save::Overwrite::Allow),
        Ok(None) => log::info!("save as cancelled"),
        Err(e) => {
            log::error!("could not show the save dialog: {e}");
            save_failed(None, &e.to_string());
        }
    }
}

/// 既定の保存先: 全体設定の保存先に、全体のファイル名テンプレートで付けた重ならない名前。フォルダがなければ作る。
/// 決められなければ失敗を通知して None を返す。
fn default_destination() -> Option<PathBuf> {
    let (save_dir, template, pending) = with_app(|app| {
        let general = &app.settings.general;
        (
            general.save_dir.clone(),
            general.file_name_template.clone(),
            app.pending_saves.clone(),
        )
    })?;
    let dir = match save_dir {
        Some(dir) => dir,
        None => match save::default_dir() {
            Ok(dir) => dir,
            Err(e) => {
                log::error!("could not find the Pictures folder: {e}");
                save_failed(None, &e.to_string());
                return None;
            }
        },
    };
    match save::destination(&dir, &template, save::now(), &pending) {
        Ok(path) => Some(path),
        Err(e) => {
            log::error!("could not prepare the save folder {}: {e}", dir.display());
            save_failed(Some(&dir), &e.to_string());
            None
        }
    }
}

/// `path` に PNG を書く仕事を裏のスレッドで始める。結果は `WM_APP_SAVED` で受ける。
fn start_save(path: PathBuf, image: Image, overwrite: save::Overwrite) {
    let Some((sender, message_window)) = with_app(|app| (app.saves.0.clone(), app.windows.message))
    else {
        return;
    };
    // 書き終えるまでの間に次の保存が同じ名前を選ばないように、結果を受けるまで覚えておく
    if with_app(|app| app.pending_saves.push(path.clone())).is_none() {
        log::warn!("could not remember the pending save {}", path.display());
    }
    // HWND は Send でないので、番号にして渡す
    let window = message_window.0 as isize;
    let job_path = path.clone();
    let spawned = std::thread::Builder::new()
        .name("save".into())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                save::write_png(&job_path, &image, overwrite)
            }))
            .map_err(|_| text("job.panicked").to_owned())
            .and_then(|written| written.map_err(|e| e.to_string()));
            let _ = sender.send(Saved {
                path: job_path,
                result,
            });
            // SAFETY: UI スレッドのメッセージ専用ウィンドウに送るだけ。もう壊れていれば失敗するだけ
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(window as *mut _)),
                    WM_APP_SAVED,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        });
    if let Err(e) = spawned {
        log::error!("could not start the save thread: {e}");
        forget_pending_save(&path);
        save_failed(Some(&path), &e.to_string());
    }
}

/// 保存先を、書き込み中の保存先から外す。
fn forget_pending_save(path: &Path) {
    with_app(|app| app.pending_saves.retain(|p| p != path));
}

/// `WM_APP_SAVED`: 保存の結果を通知する。成功なら、通知のクリックで保存先のフォルダを開けるようにする。
fn on_saved() {
    let Some(results) = with_app(|app| app.saves.1.try_iter().collect::<Vec<_>>()) else {
        return;
    };
    for saved in results {
        forget_pending_save(&saved.path);
        match saved.result {
            Ok(()) => {
                log::info!("saved {}", saved.path.display());
                let body = format!("{}\n{}", text("save.saved"), saved.path.display());
                if notify_user(notify::Level::Success, &body) {
                    let folder = saved.path.parent().map(Path::to_path_buf);
                    with_app(|app| app.notice_folder = folder);
                }
            }
            Err(e) => {
                log::error!("could not save {}: {e}", saved.path.display());
                save_failed(Some(&saved.path), &e);
            }
        }
    }
}

/// 保存の失敗の通知。スクラップは残っているので、別の方法で残せる。
fn save_failed(path: Option<&Path>, reason: &str) {
    let mut body = format!("{}\n{reason}", text("save.failed"));
    if let Some(path) = path {
        body.push('\n');
        body.push_str(&path.display().to_string());
    }
    notify_user(notify::Level::Error, &body);
}

/// スクラップを閉じて、閉じる直前の状態のままごみ箱に移す（上限が 0 なら破棄する）。
fn close_scrap(hwnd: HWND) {
    let removed = with_app(|app| {
        let index = app.scraps.iter().position(|s| s.window.hwnd() == hwnd)?;
        let Scrap {
            window,
            model,
            image,
            ..
        } = app.scraps.remove(index);
        // 一時ファイルは裏のスレッドで書くので、借りたままでよい
        app.trash.push(model, image, save::now());
        Some(window)
    })
    .flatten();
    // ウィンドウを壊すとメッセージが届くので、App を借りずに壊す
    if let Some(window) = removed {
        drop(window);
        log::info!("scrap closed");
    }
}

/// ごみ箱の一覧の項目の文言: 閉じた時刻と画像の大きさ。
fn trash_label(listed: &trash_store::Listed) -> String {
    let t = listed.closed_at;
    let size = listed.model.image_size;
    format!(
        "{:02}:{:02}:{:02}  {}×{}",
        t.hour, t.minute, t.second, size.width, size.height
    )
}

/// すべてのスクラップを閉じて、ごみ箱に移す。
fn close_all_scraps() {
    let hwnds: Vec<HWND> =
        with_app(|app| app.scraps.iter().map(|s| s.window.hwnd()).collect()).unwrap_or_default();
    for hwnd in hwnds {
        close_scrap(hwnd);
    }
}

/// すべてのスクラップを隠す・出す（出すときはフォーカスを移さない）。
fn set_scraps_visible(visible: bool) {
    with_app(|app| {
        for scrap in &app.scraps {
            if visible {
                scrap.window.show(false);
            } else {
                scrap.window.hide();
            }
        }
    });
}

/// ごみ箱の `id` のスクラップを、閉じる直前の状態のまま浮かべ直し、フォーカスを移す。
fn restore_scrap(id: TrashId) {
    restore_from_trash(|trash| trash.restore(id));
}

/// 最後に閉じたスクラップを復元する（復元のホットキー）。範囲選択の画面を開いている間は、画面より前に出て
/// キー入力を奪うので無視する。
fn restore_last_scrap() {
    let selecting =
        with_app(|app| matches!(app.capture, Capture::Selecting { .. })).unwrap_or(true);
    if selecting {
        log::info!("restore requested while the overlay is open; ignored");
        return;
    }
    restore_from_trash(trash_store::TrashStore::restore_last);
}

/// ごみ箱から `take` で取り出したスクラップを浮かべ直し、フォーカスを移す。何も取り出せなければ何もしない。
fn restore_from_trash(
    take: impl FnOnce(&mut trash_store::TrashStore) -> Option<std::io::Result<trash_store::Trashed>>,
) {
    let taken = with_app(|app| {
        let restored = take(&mut app.trash)?;
        Some((restored, app.module, Appearance::from(app.settings.scrap)))
    })
    .flatten();
    let Some((restored, module, appearance)) = taken else {
        return;
    };
    let trashed = match restored {
        Ok(trashed) => trashed,
        Err(e) => {
            log::error!("could not read the trashed scrap back: {e}");
            notify_user(
                notify::Level::Error,
                &format!("{}\n{e}", text("trash.restore_failed")),
            );
            return;
        }
    };
    // ウィンドウを作るとメッセージが届くので、App を借りずに作る
    match open_scrap(module, trashed.model, trashed.image, &appearance, true) {
        Ok(scrap) => {
            if with_app(|app| app.scraps.push(scrap)).is_none() {
                log::warn!("could not keep the restored scrap");
            }
            log::info!("scrap restored from the trash");
        }
        Err(e) => log::error!("could not open the restored scrap: {e}"),
    }
}

/// 範囲選択の画面での `WM_KEYDOWN` を入力にする。対象でないキーは None。
fn selection_key(wparam: WPARAM, lparam: LPARAM) -> Option<Input> {
    // SAFETY: GetKeyState は引数のキーの状態を読むだけ
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
    // 押しっぱなしの自動リピート（lParam の bit 30 = 直前も押されていた）
    let repeat = lparam.0 & (1 << 30) != 0;
    let shift = down(VK_SHIFT);
    let win = down(VK_LWIN) || down(VK_RWIN);
    let arrow = |direction| Some(Input::Arrow { direction, shift });
    match VIRTUAL_KEY(wparam.0 as u16) {
        VK_ESCAPE => Some(Input::Escape),
        VK_LEFT => arrow(Direction::Left),
        VK_RIGHT => arrow(Direction::Right),
        VK_UP => arrow(Direction::Up),
        VK_DOWN => arrow(Direction::Down),
        // 押しっぱなしで始点と終点が続けて決まらないよう、リピートは無視する
        VK_SPACE if !repeat => Some(Input::Space),
        // Ctrl+A と Ctrl+Shift+A だけ。Ctrl+Alt+A などほかの修飾キーとの組み合わせは対象にしない
        VK_A if down(VK_CONTROL) && !down(VK_MENU) && !win && !repeat => Some(if shift {
            Input::SelectAllMonitors
        } else {
            Input::SelectMonitor
        }),
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
        Some((state, selection.cursor()))
    })
    .flatten();
    let state = state.map(|(state, cursor)| {
        // 矢印で動かしたカーソルの位置に、実際のマウスのカーソルを合わせる。
        // 合わせたあとの WM_MOUSEMOVE は同じ位置なので、状態は変わらない
        if matches!(input, Input::Arrow { .. }) && matches!(state, State::Idle | State::Selecting) {
            // SAFETY: UI スレッドで呼ぶ。座標はスクリーン座標（物理ピクセル、PMv2）
            if let Err(e) = unsafe { SetCursorPos(cursor.x, cursor.y) } {
                log::warn!("could not move the cursor: {e}");
            }
        }
        state
    });
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
        let model = ScrapModel::new(rect.origin(), image.size());
        match open_scrap(module, model, image, &appearance, focus) {
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
        perf::mark(Mark::ScrapShown);
    }
    log::info!("overlay closed");
}

/// 画像を `model` のとおりに浮かべるスクラップを作る。
fn open_scrap(
    module: HINSTANCE,
    model: ScrapModel,
    image: Image,
    appearance: &Appearance,
    focus: bool,
) -> Result<Scrap> {
    let rect = model.window_rect(appearance);
    let window = scrap_window::ScrapWindow::create(module, rect)?;
    // 描いて出すときに届くメッセージ（WM_WINDOWPOSCHANGED など）では App を借りないので、借りたまま描いてよい。
    // 借りられなければ（起こらないはず）、その場かぎりの描き手で描く
    let painted =
        with_app(|app| paint_scrap(&mut app.renderer, &window, &image, &model, appearance, None))
            .unwrap_or_else(|| paint_scrap(&mut None, &window, &image, &model, appearance, None));
    painted?;
    window.show(focus);
    Ok(Scrap {
        window,
        model,
        image,
        drag: Drag::default(),
        wheel: Wheel::default(),
        zoom_label: false,
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
            log::warn!("could not grab the screen: {e}");
            end_capture();
            // 取得のスレッドのパニックはその仕事の失敗として通知する。ほかの撮れないときの通知は M3
            if matches!(e, grab::GrabError::Panicked) {
                notify_user(
                    notify::Level::Error,
                    &format!("{}\n{}", text("capture.failed"), text("job.panicked")),
                );
            }
            return;
        }
    };
    let monitors = shots.len();
    let grab_ms = started.elapsed().as_secs_f64() * 1000.0;

    // ウィンドウを作るとメッセージが届くので、App を借りずに開く
    let overlay = match overlay::Overlay::open(shots, module) {
        Ok(overlay) => {
            // open は全モニタで DwmFlush が返ってから戻る
            perf::mark(Mark::OverlayFlushed);
            overlay
        }
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
    perf::mark(Mark::HotkeyReceived);
    let action = with_app(|app| app.hotkeys.action(wparam)).flatten();
    match action {
        Some(settings::HotkeyAction::SelectCapture) => request_capture(Source::Hotkey),
        Some(settings::HotkeyAction::RestoreLast) => restore_last_scrap(),
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
    log_issues(&issues);
    (settings, issues)
}

fn log_issues(issues: &[SettingsIssue]) {
    for issue in issues {
        log::warn!("settings issue: {issue:?}");
    }
}

fn log_failed(failed: &[hotkey::Failed]) {
    for f in failed {
        log::warn!(
            "could not register the hotkey {} for {:?}: {}",
            f.chord,
            f.action,
            f.error
        );
    }
}

/// `WM_APP_SETTINGS`: 設定ファイルの監視の知らせを受け、読み込み直した設定を反映する。
///
/// - TOML として読めなければ、直前の有効な設定のまま動き、エラーを通知する（`settings::reload`）。
/// - `settings::diff` で変わったものだけを反映する: ホットキーの登録し直し、浮かべているスクラップの描き直し、
///   ログのレベル。ショートカットの対応表は毎回作り直す（右クリックメニューは開くたびにこれから作る）。
/// - 問題はまとめて 1 回だけ通知する。範囲選択キャプチャのホットキーが空になったら、それも通知する。
fn on_settings_changed() {
    let Some(events) = with_app(|app| app.settings_events.try_iter().collect::<Vec<_>>()) else {
        return;
    };
    let mut content = None;
    for event in events {
        match event {
            settings_watch::Event::Changed(c) => content = Some(c),
            settings_watch::Event::Stopped(reason) => {
                with_app(|app| app.watcher = None);
                notify_user(
                    notify::Level::Error,
                    &format!("{}\n{reason}", text("settings.watch_stopped")),
                );
            }
        }
    }
    let Some(content) = content else {
        return;
    };
    let notice = with_app(|app| {
        let current = &app.settings;
        let (new, mut issues) = match content {
            settings_watch::Content::Missing => settings::reload(current, None),
            settings_watch::Content::Text(src) => settings::reload(current, Some(&src)),
            settings_watch::Content::Unreadable(message) => {
                (current.clone(), vec![SettingsIssue::NotToml { message }])
            }
        };
        let changes = settings::diff(current, &new);
        log::info!("settings reloaded: {changes:?}");
        log_issues(&issues);
        let keymap = settings::keymap(&new, ops::OPERATIONS);
        log_issues(&keymap.issues);
        issues.extend(keymap.issues);
        let failed = if changes.hotkeys {
            app.hotkeys.reregister(&keymap.hotkeys.bound)
        } else {
            Vec::new()
        };
        log_failed(&failed);
        app.shortcuts = keymap.shortcuts;
        let unset =
            app.settings.hotkeys.select_capture.is_some() && new.hotkeys.select_capture.is_none();
        app.settings = new;
        apply_changes(app, &changes);
        settings_notice(
            Moment::Reload,
            &issues,
            &failed,
            unset,
            app.settings_path.as_deref(),
        )
    })
    .flatten();
    if let Some((level, body)) = notice {
        notify_user(level, &body);
    }
}

/// 読み込み直した設定（`app.settings`）のうち、使うときに読むのでは足りないものを反映する。
fn apply_changes(app: &mut App, changes: &SettingsChanges) {
    if changes.log_level {
        log::set_max_level(level_filter(app.settings.log_level));
    }
    if changes.trash {
        app.trash.set_limit(app.settings.general.trash_limit);
    }
    if changes.appearance {
        // 枠と影の幅が変わるとウィンドウの大きさも変わるので、描き直して出し直す。描くだけで App を借りる
        // メッセージは届かないので、借りたままでよい（`run_scrap_operation` と同じ）
        let appearance = Appearance::from(app.settings.scrap);
        for scrap in &app.scraps {
            if let Err(e) = scrap.paint(&appearance, &mut app.renderer) {
                log::warn!("could not redraw a scrap with the new appearance: {e}");
            }
        }
    }
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

/// 設定を読んだとき。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Moment {
    Startup,
    /// 実行中の読み込み直し。
    Reload,
}

/// 設定とホットキーの問題を、1 つの通知の本文にまとめる。問題がなければ None。
/// 通知は前の通知を置き換えるので、別々に出すと最後の 1 つしか見えない。`unset` なら、範囲選択キャプチャの
/// ホットキーが未設定であることも載せる。
fn settings_notice(
    moment: Moment,
    issues: &[SettingsIssue],
    failed: &[hotkey::Failed],
    unset: bool,
    path: Option<&Path>,
) -> Option<(notify::Level, String)> {
    let mut level = notify::Level::Warning;
    let mut lines = Vec::new();
    let mut invalid = Vec::new();
    let mut conflicts = Vec::new();
    let mut overlaps = Vec::new();
    for issue in issues {
        match issue {
            SettingsIssue::NotToml { .. } => {
                level = notify::Level::Error;
                lines.push(
                    text(match moment {
                        Moment::Startup => "settings.unreadable",
                        Moment::Reload => "settings.unreadable_kept",
                    })
                    .to_owned(),
                );
            }
            SettingsIssue::Invalid { item } => invalid.push(item.as_str()),
            SettingsIssue::Conflict { chord, items } => {
                conflicts.push(format!("{chord}（{}）", items.join(", ")));
            }
            SettingsIssue::Overlap {
                chord,
                hotkey,
                shortcut,
            } => overlaps.push(format!("{chord}（{hotkey}, {shortcut}）")),
        }
    }
    for (key, items) in [
        ("settings.invalid_items", invalid.join(", ")),
        ("settings.conflicts", conflicts.join(", ")),
        ("settings.overlaps", overlaps.join(", ")),
    ] {
        if !items.is_empty() {
            lines.push(format!("{}{items}", text(key)));
        }
    }
    if !failed.is_empty() {
        let chords: Vec<String> = failed.iter().map(|f| f.chord.to_string()).collect();
        lines.push(format!("{}{}", text("hotkey.failed"), chords.join(", ")));
    }
    if unset {
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

/// トレイの通知を出す。出したら true。設定で通知をオフにしたときは、成功の通知だけを止める（エラーと警告は出す）。
/// 新しい通知は前の通知を置き換えるので、クリックで開くフォルダを忘れる（保存の通知なら呼び出し側で覚え直す）。
fn notify_user(level: notify::Level, body: &str) -> bool {
    let target = with_app(|app| {
        app.notice_folder = None;
        (app.windows.tray_owner, app.settings.general.notifications)
    });
    let Some((owner, enabled)) = target else {
        log::warn!("notification dropped: {body}");
        return false;
    };
    if matches!(level, notify::Level::Success) && !enabled {
        return false;
    }
    // Explorer とやりとりするので、App を借りずに出す
    let shown = notify::show(owner, tray::ICON_ID, level, text("notify.title"), body);
    if !shown {
        log::warn!("could not show a notification: {body}");
    }
    shown
}
