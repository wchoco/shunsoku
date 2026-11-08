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
//!   範囲選択の画面（`overlay`）→ 切り出し → 出力 → 結果。同時に進める依頼は 1 件だけ（`Capture`）。
//!   出力（コピー・スクラップ・保存）は撮り方によらず `output` で出す。CLI の依頼の受け取り・取りやめ・応答と
//!   `capture rect` は子モジュールの `cli` にまとめる。
//! - 定型領域キャプチャ（`CaptureMethod::Region`）は範囲選択の画面を出さず、1 つのメッセージの処理の中で
//!   撮り終える（`capture_region`）: モニタの一覧（`monitor_list`）→ core の `monitors::locate_by_id` → そのモニタだけ取得
//!   （`Gpu::grab_monitor`）→ 切り出し → 出力。依頼は `App::regions_waiting` に届いた順に待たせ、`WM_APP_REGION`
//!   で 1 件ずつ撮る。範囲選択の画面を開いている間の依頼は無視する。shunsoku のメニューは閉じ、消えきるまで
//!   待ってから撮る（`run_waiting_regions`）。保存に失敗したら、スクラップに逃がす（`App::save_fallbacks`）。
//! - 範囲選択の入力（マウス・キー）は core の `selection::Input` にして `Selection::handle` に渡す。
//!   入力を足すときは `Input` を足し、ここではメッセージから作るだけにする。
//! - スクラップ（`Scrap`）は `App::scraps` に持つ。ウィンドウは `scrap_window`、中身の描画は `render`。
//! - スクラップへのキーとマウスと右クリックメニューは、core の `scrap::ops` の表（ショートカットの対応表
//!   `App::shortcuts`、`ops::mouse_operation`、`ops::menu`）でスクラップ操作に変え、`run_scrap_operation` で
//!   実行する。操作が返す `Effect` を実行するのはここ（`perform`）。操作を足すときは core の表に
//!   行を足し、新しい `Effect` があればそこに足す。
//! - ショートカットに付いたレシピ（`[[recipes]]`）は `run_recipe` で実行する。手を進めるのは core の
//!   `recipe::Run`（ADR-0007）で、ここは手ごとに `Effect` を実行し（`perform`）、その結果（`Outcome`）を返す。
//!   保存の手は、裏のスレッドの書き込みの結果（`WM_APP_SAVED`）を受けるまで `App::recipe` に預けて待つ。待って
//!   いる間にそのスクラップを閉じるか、ほかのレシピを始めたら、待っているレシピは捨てる。
//! - 設定（`%APPDATA%\shunsoku\settings.toml`、ポータブルモードでは exe の横のもの。場所は core の
//!   `settings::locate`）は起動時に読み、`settings_watch` の知らせ（`WM_APP_SETTINGS`）で
//!   読み込み直す。アプリから書き戻すときは `write_settings` で変わった項目だけを書き、書いた直後の中身を監視が
//!   拾っても読み込み直さない（`App::own_writes`）。読み込み直しでは core の `settings::diff` で変わった部分を調べ、
//!   ホットキーの登録し直し・スクラップの描き直しなど、変わったものだけを反映する（`on_settings_changed`）。
//!   保存先・ファイル名テンプレート・通知のオン/オフは、使うときに `App::settings` から読むので、次から効く。
//!   キーの割り当てと重なりは core の `settings::keymap` で決める。
//!   問題（読めない・不正な項目・重なったキー・登録できないホットキー・未設定のホットキー）は、まとめて 1 回だけ通知する。
//!   範囲選択キャプチャのホットキーが未設定なら、起動時は通知せずに設定 GUI を開き、読み込み直しで空になったときと
//!   未設定のまま GUI を閉じたときは通知する（core の `settings::unset::prompt`）。
//! - 設定 GUI（トレイの「設定…」・2 つ目の起動）は `settings_gui` が開き、ダイアログの組み立ては `settings_dialog`。
//!   適用は `write_settings` で書いてから `adopt_settings` で読み込み直しと同じ反映をする。キーの入力欄に
//!   フォーカスがある間はホットキーの登録を外す（`App::hotkey_pause`）。
//! - 通知は `notify_user` から出す。設定で通知をオフにしても、止めるのは成功の通知だけ。通知のクリックは
//!   中身を持たないので、クリックで開くフォルダを覚えておき、新しい通知を出したら忘れる（core の `notice::Notices`）。
//! - コピーと保存の画像は `render::render_image` で表示と同じ描き方で作る。保存先とファイル名は UI スレッドで
//!   決め（`save::destination`）、PNG の書き出しと書き込みは裏のスレッドで行い、結果は `WM_APP_SAVED` で受ける。
//!   結果を受けるまでの保存先は `App::pending_saves` に持ち、続けて保存しても同じ名前にしない。
//!   保存先とファイル名テンプレートは設定（`[general]`）のものを使う。
//! - 閉じたスクラップ（`Effect::MoveToTrash`、Alt+F4 も）は `App::trash`（`trash_store`）に閉じる直前の
//!   `ScrapModel` のまま入れる。画素は裏のスレッドで一時ファイルに書き、書き終えたら `WM_APP_TRASH` で受ける。
//!   トレイメニューの「ごみ箱」から選ぶと、その状態のまま浮かべ直してフォーカスを移す（`restore_scrap`）。
//!   ごみ箱に入れる画素は元の画像（トリム前）で、トリムの範囲は `ScrapModel` に残るので、戻すとトリムしたまま戻る。
//! - トリムの範囲を選んでいる間（`Effect::EnterTrimMode` から）は、左ボタン・カーソルの移動・ダブルクリック・
//!   Enter・Space・`Esc`・矢印（Ctrl で大きさ、Shift で 10 画素）を core の `ScrapModel::trim_input` に渡し、範囲を
//!   描き直す（`on_trim_input`）。つまみ・十字線・拡大鏡は core の `ScrapModel::trim_view` のとおりに描き、カーソルの
//!   形は `ScrapModel::trim_cursor_shape` で当てる。十字線の色は入ったときの `[selection] crosshair_color`。
//! - 書き込みの編集モードの間（`Effect::EnterEditMode` から）は、左ボタン・カーソルの移動・キーを core の
//!   `ScrapModel::edit_input` に渡し、描き直す（`on_edit_input`）。ホイールは core の `annotate::toolbar::wheel_pick`
//!   で道具・色・太さを選ぶ（`on_edit_wheel`、割り当ては固定）。ショートカットとマウスの割り当て（ホイール・
//!   中クリック・ダブルクリックも）は効かない。ほかのウィンドウにフォーカスが移ったら（`WM_ACTIVATE`）抜ける。
//! - 編集モードの間は、スクラップの直下にツールバー（`toolbar_window`、`Scrap::toolbar`）を出す。押してもスクラップの
//!   フォーカスは動かない。道具・色・太さを選ぶ（`EditInput::Pick`）、完了（`EditInput::Leave`）、つまみのドラッグで
//!   スクラップごと動かす（`on_toolbar_drag`）。置き場所は core の `annotate::toolbar::place` で、スクラップを描き直す・
//!   動かすたび（`Scrap::paint` / `Scrap::redraw`）についていく。選んだものは `App::carryover` に覚え、次の編集モードに
//!   引き継ぐ。
//!   書き込みは表示にも保存とコピーにも `render` の同じ関数で描く（ADR-0006）。
//! - 文字の道具で打つ間（`EditOutput::TypingStarted` から `TypingEnded` まで）だけ、スクラップで IME を使う
//!   （`ime`、`sync_ime`）。`WM_CHAR` は `EditInput::Utf16`、IME の確定した文字と変換中の文字は
//!   `EditInput::Composition` にして渡す。変換中の文字は core（`EditState::composition`）が持ち、打っている文字の
//!   続きに描く。変換中にクリックやツールバーなどで打ち終えるときに変換中の文字を確定したことにするのも core で、
//!   そのとき（`EditOutput::CompositionTaken`）は IME の側の変換中の文字を捨てる。編集モードのダブルクリックは
//!   `EditInput::DoubleClick`。

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use shunsoku_core::APP_NAME;
use shunsoku_core::annotate::toolbar::{self, Button as ToolbarButton, Layout as ToolbarLayout};
use shunsoku_core::annotate::{Carryover, EditInput, EditOutput, Style};
use shunsoku_core::app_window;
use shunsoku_core::capture::request::{
    self, CaptureMethod, CaptureRequest, OnSaveFailure, ReplyTo, SaveTarget, Source,
};
use shunsoku_core::geometry::{Point, Rect};
use shunsoku_core::i18n::{self, Language};
use shunsoku_core::image::{Fragment, Image};
use shunsoku_core::keys::{Chord, Key, Modifiers};
use shunsoku_core::monitors::{self, LocateError};
use shunsoku_core::notice::Notices;
use shunsoku_core::perf::Mark;
use shunsoku_core::protocol::{ErrorKind, Response};
use shunsoku_core::recipe::{Advance, Outcome, Parked, PlannedStep, Run};
use shunsoku_core::scrap::ops::{
    self, ChoiceLabel, Effect, MenuEntry, MouseBindings, MouseGesture, OpContext,
    OperationDescriptor, OperationId, Shortcuts,
};
use shunsoku_core::scrap::{
    Appearance, Background, Drag, Mode, Pointer, ScrapModel, TrimInput, Wheel, ZoomAnchor,
};
use shunsoku_core::scrap_file::{self, ScrapFile};
use shunsoku_core::selection::{Button, CursorShape, Direction, Input, Selection, State};
use shunsoku_core::settings::form::HotkeyPause;
use shunsoku_core::settings::{
    self, LogLevel, Rgb, Settings, SettingsChanges, SettingsIssue, unset,
};
use shunsoku_core::trash::TrashId;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_A, VK_CONTROL, VK_DOWN,
    VK_ESCAPE, VK_LEFT, VK_LWIN, VK_MENU, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EndMenu, EnumThreadWindows,
    GetClassNameW, GetMessagePos, GetMessageW, HWND_MESSAGE, IDI_APPLICATION, IsWindowVisible,
    KillTimer, LoadIconW, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SPI_GETSELECTIONFADE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetCursorPos, SetTimer, SystemParametersInfoW, TranslateMessage, WA_INACTIVE, WINDOW_EX_STYLE,
    WM_ACTIVATE, WM_ACTIVATEAPP, WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_CONTEXTMENU,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};
// 精密モード（#66）で足したもの
use windows::Win32::UI::Input::KeyboardAndMouse::VK_TAB;
use windows::Win32::UI::WindowsAndMessaging::{HTCLIENT, WM_SETCURSOR};
// 文字の書き込み（#87）で足したもの
use crate::ime;
use shunsoku_core::annotate::{EditState, Font};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_CHAR, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_SETCONTEXT, WM_IME_STARTCOMPOSITION,
};
// 書き込みのツールバー（#84）で足したもの
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IDC_SIZEALL, LoadCursorW, MA_NOACTIVATE, SetCursor, WM_MOUSEACTIVATE,
};
// 消しゴム（#145）で足したもの
use shunsoku_core::annotate::Tool;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, IDC_ARROW, WindowFromPoint};

mod cli;
mod settings_gui;
#[cfg(test)]
mod tests;

use crate::{
    autostart, clipboard, grab, hotkey, logger, menu, monitor_list, notify, overlay, panic_hook,
    perf, pipe_server, render, save, scrap_window, settings_watch, single_instance, toolbar_window,
    trash_store, tray, window_list,
};
use cli::{
    REPLIER, capture_rect, on_pipe_requests, reply, reply_cancelled, reply_failed, waiting_reply,
};

/// トレイのアイコンへの操作（`tray::event` で読む）。道具も送るので、番号は `app_window` に置く。
const WM_APP_TRAY: u32 = WM_APP + app_window::TRAY_MESSAGE_FROM_WM_APP;
/// 環境変数 `SHUNSOKU_TEST_PANIC=ui` のとき、UI スレッドでパニックを起こす（手で確かめる用。デバッグ ビルドだけ）。
#[cfg(debug_assertions)]
const WM_APP_TEST_PANIC: u32 = WM_APP + 2;
/// 保存の裏のスレッドが書き終えた（結果は `App::saves` の受け口に入っている）。
const WM_APP_SAVED: u32 = WM_APP + 3;
/// 設定ファイルの監視が知らせを入れた（中身は `App::settings_events` の受け口に入っている）。
const WM_APP_SETTINGS: u32 = WM_APP + 4;
/// ごみ箱の画素を一時ファイルに書き終えた（結果は `App::trash` の受け口に入っている）。
const WM_APP_TRASH: u32 = WM_APP + 5;
/// CLI の依頼がパイプのサーバから届いた（中身は `App::pipe` の受け口に入っている）。依頼を終えたときにも
/// 自分に送り、待たせている依頼を進める。
const WM_APP_PIPE: u32 = WM_APP + 6;
/// 待っている定型領域キャプチャの依頼（`App::regions_waiting`）を進める。
const WM_APP_REGION: u32 = WM_APP + 7;
/// 設定 GUI のキーの組み合わせの欄のフォーカスが変わった（設定 GUI が自分に送る。`settings_gui::check_focus`）。
const WM_APP_SETTINGS_FOCUS: u32 = WM_APP + 8;
/// 設定 GUI の DPI が変わり、ダイアログが大きさを直し終えた（設定 GUI が自分に送る。定型領域の一覧の列の幅を直す）。
const WM_APP_SETTINGS_DPI: u32 = WM_APP + 9;

/// 2 つのウィンドウのクラス名。2 つ目のプロセスはこの名前でメッセージ専用ウィンドウを探す。
const CLASS_NAME: PCWSTR = PCWSTR(app_window::APP_CLASS_WIDE.as_ptr());
/// 2 つ目のプロセスが送る登録メッセージの名前。
const SECOND_INSTANCE_MESSAGE: PCWSTR = w!("shunsoku.SecondInstance");

/// トレイのメニューの項目の番号。
const CMD_EXIT: u32 = 1;
const CMD_SELECT_CAPTURE: u32 = 2;
const CMD_CLOSE_ALL: u32 = 3;
const CMD_HIDE_ALL: u32 = 4;
const CMD_SHOW_ALL: u32 = 5;
const CMD_ADD_REGION: u32 = 6;
const CMD_SETTINGS: u32 = 7;
const CMD_BLANK_SCRAP: u32 = 8;
const CMD_OPEN: u32 = 9;
/// ごみ箱の一覧の項目の番号は、これに一覧の添字を足したもの。
const CMD_TRASH_FIRST: u32 = 100;
/// 「定型領域」の項目の番号は、これに `Settings::regions` の添字を足したもの。
const CMD_REGION_FIRST: u32 = 1000;

/// 依頼の `delay` を待つタイマーの番号（メッセージ専用ウィンドウの WM_TIMER）。
const TIMER_CAPTURE_DELAY: usize = 1;
/// shunsoku のメニューが消えきるのを待って定型領域キャプチャを進め直すタイマーの番号（メッセージ専用
/// ウィンドウの WM_TIMER）。待っている依頼（`Capture::Waiting`）は `TIMER_CAPTURE_DELAY` で進め直す。
const TIMER_REGION_RETRY: usize = 2;
/// App を借りられずに進められなかった CLI の依頼を進め直すタイマーの番号（メッセージ専用ウィンドウの
/// WM_TIMER）と間隔。
const TIMER_PIPE_RETRY: usize = 3;
const PIPE_RETRY_MS: u32 = 50;
/// メニューが消えきったかを確かめ直す間隔。
const MENU_RETRY_MS: u32 = 30;
/// メニューが消えきるのを待つ回数の上限。超えたらそのまま撮る。
const MAX_MENU_RETRIES: u32 = 30;
/// メニューの項目をマウスで選んだときのフェード（SPI_GETSELECTIONFADE）を待つ時間。フェードのウィンドウは
/// 列挙で見つからないことがあるので、メニューから選ばれた依頼は時間で待つ。
const MENU_FADE_MS: u64 = 400;
/// 倍率の表示を消すタイマーの番号（スクラップのウィンドウの WM_TIMER）と、出しておく時間。
const TIMER_ZOOM_LABEL: usize = 1;
const ZOOM_LABEL_MS: u32 = 1000;
/// スクラップのウィンドウのモニタの DPI が変わった（wparam が新しい DPI）。`WM_DPICHANGED` はウィンドウを
/// 動かしている間（App を借りている間）に届くので、自分に送り直して借りられるときに core へ渡す。
const WM_APP_SCRAP_DPI: u32 = WM_APP + 0x100;
/// スクラップが非アクティブになった（`WM_ACTIVATE` の `WA_INACTIVE`）。隠す・壊すなど App を借りている間にも届くので、
/// 自分に送り直して借りられるときに編集モードを抜ける。
const WM_APP_SCRAP_INACTIVE: u32 = WM_APP + 0x101;

/// UI スレッドのパニックで終わったときの終了コード（Rust の既定と同じ）。
const EXIT_PANIC: i32 = 101;

/// 登録メッセージの番号。`App` を借りずに比べられるように別に持つ。
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
static SECOND_INSTANCE: OnceLock<u32> = OnceLock::new();

/// いまの UI の言語が英語か（`text` で引くカタログ）。裏のスレッドとパニックのフックも読むので atomic。
static ENGLISH: AtomicBool = AtomicBool::new(false);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// アプリの状態。UI スレッドだけが持つ。drop の順（宣言の順）で設定ファイルの監視を止め、トレイを消し、
/// 範囲選択の画面を閉じ、ホットキーの登録を外してからウィンドウを壊す。
struct App {
    /// 設定ファイルの監視。設定ファイルの場所が決まらないときや始められなかったときは None。
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
    /// 自分で書いた設定ファイルの中身（`write_settings`）。監視が拾っても読み込み直さない。
    own_writes: settings::OwnWrites,
    /// スクラップ操作のショートカットの対応表（表の既定値に `settings.shortcuts` を重ねたもの）。
    shortcuts: Shortcuts,
    /// スクラップ操作のマウスの対応表（表の既定値に `settings.mouse` を重ねたもの）。
    mouse: MouseBindings,
    capture: Capture,
    /// 待っている定型領域キャプチャの依頼（届いた順）。`Capture` が `Idle` のときに 1 件ずつ撮る。
    regions_waiting: VecDeque<CaptureRequest>,
    /// shunsoku のメニューが消えきるのを待った回数（`wait_for_menus`）。
    menu_retries: u32,
    /// これより前には撮らない（メニューで選んだ項目のフェードを待つ）。
    menu_not_before: Option<Instant>,
    scraps: Vec<Scrap>,
    /// 閉じたスクラップ。drop で一時ファイルを消す。
    trash: trash_store::TrashStore,
    /// 保存の裏のスレッドが結果を入れる口と、UI スレッドが受ける口。
    saves: (mpsc::Sender<Saved>, mpsc::Receiver<Saved>),
    /// CLI の依頼を受けるパイプのサーバの受け口。始められなかったときは None。
    pipe: Option<pipe_server::PipeServer>,
    /// 名前を決めて裏のスレッドに渡し、まだ結果（`WM_APP_SAVED`）を受けていない保存先。次の名前を決めるとき、
    /// 使われている名前として数える（`save::destination`）。
    pending_saves: Vec<PathBuf>,
    /// 書き込み中の保存のうち、失敗したらスクラップを浮かべるもの（定型領域キャプチャの
    /// `OnSaveFailure::FallbackToScrap`）。結果（`WM_APP_SAVED`）を受けたら外す。
    save_fallbacks: Vec<(PathBuf, Fallback)>,
    /// 保存の結果（`WM_APP_SAVED`）を待っているレシピ。スクラップのウィンドウ（番号）と保存先で見分ける。
    recipe: Parked<isize, PathBuf>,
    /// スクラップの中身を描く Direct2D の描き手。描き直しのたびに作らないよう使い回す（`paint_scrap`）。
    renderer: Option<render::Renderer>,
    /// 書き込みの編集モードで最後に選んだ道具・色・太さ。アプリが動いている間、次の編集モードに引き継ぐ。
    carryover: Carryover,
    /// いま出ている通知と、そのクリックで開くフォルダ（保存の通知のときだけ）。
    notices: Notices,
    gpu: grab::Gpu,
    /// 開いている設定 GUI。
    settings_gui: Option<settings_gui::SettingsGui>,
    /// 設定 GUI のキーの組み合わせの欄にフォーカスがあり、全体のホットキーの登録を外している。
    hotkey_pause: HotkeyPause,
    /// 最後に登録したときに登録できなかったホットキー（設定 GUI の印にする）。
    unregistered: Vec<settings::SettingsItem>,
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
        /// ウィンドウの一覧を持って大きいので Box にする。
        selection: Box<Selection>,
    },
}

/// 保存の裏のスレッドの結果。
struct Saved {
    path: PathBuf,
    /// 失敗の理由（裏のスレッドのパニックも失敗にする）。
    result: std::result::Result<(), String>,
    /// 結果を返す先（CLI の依頼の保存だけ）。
    reply: Option<ReplyTo>,
    /// 結果をトレイ通知で知らせる（CLI の `--quiet` なら false）。
    notify: bool,
}

/// 保存に失敗したときに浮かべるスクラップの位置（スクリーン座標）と画像。フォーカスは移さない。
struct Fallback {
    at: Point,
    image: Image,
}

/// 浮かべているスクラップ 1 枚。
struct Scrap {
    /// 書き込みの編集モードの間だけ出すツールバー。持ち主のウィンドウより先に壊すよう、`window` より前に置く。
    toolbar: Option<Toolbar>,
    /// drop でウィンドウを壊す。
    window: scrap_window::ScrapWindow,
    model: ScrapModel,
    /// 元の画像（トリム前）。
    image: Image,
    /// トリムしていれば、`image` から切り出した表示する画像（`ScrapModel::shown_image`）。
    shown: Option<Image>,
    /// 左ドラッグ。
    drag: Drag,
    /// ホイールの回転（1 ノッチに満たない量をためる）。
    wheel: Wheel,
    /// 倍率を出しているか（`Effect::ShowZoom` から `TIMER_ZOOM_LABEL` が鳴るまで）。
    zoom_label: bool,
    /// 最後に描いたキャレットの矩形（ウィンドウの中の座標）。打っていなければ None。
    caret: std::cell::Cell<Option<Rect>>,
    /// トリムの十字線の色。トリムに入ったときの `[selection] crosshair_color`。
    crosshair: Rgb,
}

/// 書き込みの編集モードのツールバー。
struct Toolbar {
    /// drop でウィンドウを壊す。
    window: toolbar_window::ToolbarWindow,
    /// 並べる色（`toolbar::palette`）。入ったときの設定の既定色で決める。
    colours: Vec<Rgb>,
    /// 最後に出した並びと矩形（スクリーン座標）。まだ出していなければ None。当たり判定に使う。
    placed: std::cell::RefCell<Option<(ToolbarLayout, Rect)>>,
    /// つまみのドラッグ。
    drag: Drag,
}

impl Toolbar {
    /// スクリーン座標の点 `at` にあるボタン。
    fn hit(&self, at: Point) -> Option<ToolbarButton> {
        let placed = self.placed.borrow();
        let (layout, rect) = placed.as_ref()?;
        layout.hit(Point::new(at.x - rect.left, at.y - rect.top))
    }
}

/// ツールバーとスクラップの画像の間（DPI 96 のときの物理ピクセル。枠と影の外に足す）。
const TOOLBAR_GAP: f64 = 4.0;

/// `hwnd` のあるモニタの作業領域（タスクバーを除く。スクリーン座標）。
fn work_area(hwnd: HWND) -> Option<Rect> {
    // SAFETY: 引数は生きているウィンドウと、この関数の中の構造体
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).as_bool().then(|| {
            let r = info.rcWork;
            Rect::new(r.left, r.top, r.right, r.bottom)
        })
    }
}

impl Scrap {
    /// 書き込みのツールバーを、スクラップの画像の直下（`toolbar::place`）に出す。`repaint` か、DPI が変わって並びが
    /// 変わったときだけ描き直し、そうでなければ動かすだけにする。ツールバーがなければ何もしない。
    fn sync_toolbar(
        &self,
        appearance: &Appearance,
        renderer: &mut Option<render::Renderer>,
        repaint: bool,
    ) {
        let (Some(toolbar), Some(state)) = (&self.toolbar, self.model.edit_state()) else {
            return;
        };
        let scale = f64::from(self.window.dpi()) / 96.0;
        let layout = ToolbarLayout::for_state(&toolbar.colours, scale, state);
        let size = layout.size();
        let image = self.model.image_rect();
        let work = work_area(self.window.hwnd()).unwrap_or(image);
        let gap = self.model.appearance(appearance).margin() + (TOOLBAR_GAP * scale).round() as i32;
        let rect = Rect::from_origin_size(toolbar::place(image, size, work, gap), size);
        let mut placed = toolbar.placed.borrow_mut();
        let first = placed.is_none();
        let same = placed.as_ref().is_some_and(|(l, _)| *l == layout);
        let shown = if same && !repaint {
            toolbar.window.move_to(rect)
        } else {
            (|| {
                let canvas = render::Canvas::new(size)?;
                let painter = match renderer.take() {
                    Some(painter) => painter,
                    None => render::Renderer::new()?,
                };
                painter.paint_toolbar(
                    &canvas,
                    &layout,
                    state,
                    self.model.annotations(),
                    scale as f32,
                )?;
                *renderer = Some(painter);
                toolbar.window.present(canvas.dc(), rect)
            })()
        };
        if let Err(e) = shown {
            log::warn!("could not show the annotation toolbar: {e}");
            return;
        }
        *placed = Some((layout, rect));
        if first {
            toolbar.window.show();
        }
    }

    /// 表示する画像（保存・コピーの対象）。トリムしていればトリム後。
    fn shown(&self) -> &Image {
        self.shown.as_ref().unwrap_or(&self.image)
    }

    /// トリムの範囲が `before` から変わっていたら、表示する画像を切り出し直す。白紙スクラップの背景が
    /// 変わっていたら、画像をその背景で塗り直してから切り出し直す。
    fn sync_shown(&mut self, before: &ScrapModel) {
        let repainted = self.model.background() != before.background();
        if repainted && let Some(image) = self.model.blank_image() {
            self.image = image;
        }
        if repainted || self.model.trim_rect() != before.trim_rect() {
            self.shown = shown_image(&self.model, &self.image);
        }
    }

    /// `Effect::Redraw`: `model` のとおりに出し直す。表示の大きさとモードが `before` と同じで、`repaint` でも
    /// なければ、中身は変わらないのでウィンドウを動かすだけにする（ドラッグのたびに描き直さない）。
    fn redraw(
        &self,
        before: &ScrapModel,
        appearance: &Appearance,
        renderer: &mut Option<render::Renderer>,
        repaint: bool,
    ) -> Result<()> {
        if self.model.same_content(before) && !repaint {
            self.window.move_to(self.model.window_rect(appearance))?;
            self.sync_toolbar(appearance, renderer, false);
            Ok(())
        } else {
            self.paint(appearance, renderer)
        }
    }

    /// `before` から変えた `model` を表示に移して（`sync_shown`）出し直す（`redraw`）。描けなければ（大きすぎるときなど）
    /// ウィンドウは前のままなので、状態も `before` に戻して false。`what` はログに出す、何をしていたか。
    fn redraw_or_revert(
        &mut self,
        before: &ScrapModel,
        appearance: &Appearance,
        renderer: &mut Option<render::Renderer>,
        repaint: bool,
        what: &str,
    ) -> bool {
        self.sync_shown(before);
        match self.redraw(before, appearance, renderer, repaint) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("could not redraw a scrap ({what}): {e}");
                let failed = std::mem::replace(&mut self.model, before.clone());
                // 戻した状態と比べて、表示する画像も前のものに切り出し直す
                self.sync_shown(&failed);
                false
            }
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
        let caret = paint_scrap(
            renderer,
            &self.window,
            self.shown(),
            &self.model,
            appearance,
            label,
            self.crosshair,
        )?;
        self.caret.set(caret);
        self.sync_toolbar(appearance, renderer, true);
        Ok(())
    }
}

/// スクラップの中身を描いて、ウィンドウを `model` の矩形（`ScrapModel::window_rect`）に出す。`appearance` は全体の
/// 見た目の既定値で、スクラップごとの上書きを重ねて描く。
/// コンパクト表示のときは、画像のうち `ScrapModel::compact_view` の部分を正方形に引き伸ばして描く。
///
/// `renderer` は使い回す描き手（`App::renderer`）。なければ作って入れる。描けなかったときは
/// （`D2DERR_RECREATE_TARGET` など）次に作り直せるように入れない。
/// 文字を打っていれば、IME の変換中の文字（`EditState::composition`）をつなげて描き、キャレットの矩形（ウィンドウの
/// 中の座標）を返す。トリムの最中は、範囲・つまみ・十字線（色は `crosshair`）・拡大鏡を重ねる。
fn paint_scrap(
    renderer: &mut Option<render::Renderer>,
    window: &scrap_window::ScrapWindow,
    image: &Image,
    model: &ScrapModel,
    appearance: &Appearance,
    label: Option<render::ZoomLabel>,
    crosshair: Rgb,
) -> Result<Option<Rect>> {
    let rect = model.window_rect(appearance);
    let appearance = &model.appearance(appearance);
    let canvas = render::Canvas::new(rect.size())?;
    let mut painter = match renderer.take() {
        Some(painter) => painter,
        None => render::Renderer::new()?,
    };
    let view;
    let image = match model.compact_view() {
        Some(rect) => {
            view = image.crop(rect);
            &view
        }
        None => image,
    };
    let display = model.image_rect().size();
    let scale = window.dpi() as f32 / 96.0;
    let trim = model.trim_view().map(|view| {
        // スクリーン座標から、ウィンドウの中の座標へ
        let margin = appearance.margin();
        let (dx, dy) = (margin - model.position.x, margin - model.position.y);
        let local = |p: Point| Point::new(p.x + dx, p.y + dy);
        render::TrimOverlay {
            selection: view.selection.map(|r| r.offset(dx, dy)),
            handles: view.handles.iter().map(|r| r.offset(dx, dy)).collect(),
            crosshair: view.crosshair.map(|at| (local(at), crosshair)),
            magnifier: view.crosshair.zip(view.magnifier).map(|(at, pixel)| {
                render::TrimMagnifier {
                    cursor: local(at),
                    pixel,
                    label: view.cursor_label.clone(),
                }
            }),
            scale,
        }
    });
    let annotations = model.annotations_to_draw();
    let selected: Vec<Rect> = model
        .selected_annotations()
        .iter()
        .map(|a| a.bounds())
        .collect();
    let typing = model.typing_annotation();
    let composition = model.edit_state().and_then(EditState::composition);
    let overlays = render::Overlays {
        annotations: render::AnnotationLayer {
            annotations: &annotations,
            view: model.annotation_view(),
            selected: &selected,
            selection_range: model.selection_range(),
            typing: typing.as_ref().map(|annotation| render::TypingLayer {
                annotation,
                composition: composition.map_or("", |c| c.text.as_str()),
                cursor: composition.map_or(0, |c| c.cursor),
            }),
            scale,
        },
        label,
        trim: trim.as_ref(),
    };
    let caret = painter.paint_scrap(&canvas, image, display, appearance, overlays)?;
    *renderer = Some(painter);
    // レイヤード ウィンドウは中身を写して持つので、canvas はここで捨ててよい
    window.present(canvas.dc(), rect, appearance.opacity)?;
    Ok(caret)
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

/// いまの言語でのキーの文言。キーがなければキーそのもの。
fn text(key: &'static str) -> &'static str {
    i18n::catalog(language()).text(key)
}

/// いまの UI の言語。
fn language() -> Language {
    if ENGLISH.load(Ordering::Relaxed) {
        Language::English
    } else {
        Language::Japanese
    }
}

/// 設定 `settings` と OS の表示言語から、UI の言語を決める。変わったら true。
fn set_language(settings: &Settings) -> bool {
    // SAFETY: 引数なし
    let langid = unsafe { GetUserDefaultUILanguage() };
    let language = settings.general.ui_language(langid);
    let old = ENGLISH.swap(language == Language::English, Ordering::Relaxed);
    log::info!("UI language: {language:?} (OS LANGID {langid:#06x})");
    old != (language == Language::English)
}

/// 設定ファイルの場所を決める（core の `settings::locate`）。exe の横に `settings.toml` があればそれ
/// （ポータブルモード）、なければ `%APPDATA%\shunsoku\settings.toml`。ポータブルモードで書けなくても
/// `%APPDATA%` には逃がさない（`write_settings` がエラーを返し、呼び出し側が通知する）。
fn locate_settings() -> Option<settings::SettingsLocation> {
    // ロガーはまだ無いので、取れなければ黙って %APPDATA% にする
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    locate_settings_at(exe_dir, appdata.as_deref())
}

/// exe のフォルダ `exe_dir` と `%APPDATA%`（`appdata`）から、設定ファイルの場所を決める。ファイルの有無は
/// 実際のファイルで確かめる。
fn locate_settings_at(
    exe_dir: Option<&Path>,
    appdata: Option<&Path>,
) -> Option<settings::SettingsLocation> {
    settings::locate(exe_dir, appdata, Path::is_file)
}

/// 常駐本体を動かし、終了コードを返す。
pub fn run() -> i32 {
    let perf_started = perf::start();
    // 設定ファイルの場所（ポータブルモードの判定）。ログも同じフォルダに書く
    let location = locate_settings();
    let log_path = location
        .as_ref()
        .and_then(|location| logger::init(location.dir()).ok());
    log::info!("starting {APP_NAME} v{}", env!("CARGO_PKG_VERSION"));
    match &location {
        Some(location) if location.portable => {
            log::info!("portable mode: using {}", location.settings_file.display())
        }
        Some(_) => {}
        None => log::warn!("neither the exe folder nor APPDATA is known"),
    }
    if let Err(e) = perf_started {
        log::warn!("could not open the perf log: {e}");
    }

    // 設定を読むまでは OS の表示言語。文言はパニックした時点の言語で引く
    set_language(&Settings::default());
    panic_hook::install(std::thread::current().id(), move || {
        let body = match &log_path {
            Some(path) => format!("{}\n{}", text("panic.body"), path.display()),
            None => text("panic.body").to_owned(),
        };
        (text("panic.title").to_owned(), body)
    });

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

    // 通知のクリックでフォルダを開くシェルの呼び出し（SHOpenFolderAndSelectItems）のため
    // SAFETY: UI スレッドで 1 回呼び、終わるときに CoUninitialize で対にする
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if com.is_err() {
        log::warn!("could not initialize COM: {com:?}");
    }

    let settings_path = location.map(|location| location.settings_file);
    let (settings, issues) = read_settings(settings_path.as_deref());
    log::set_max_level(level_filter(settings.log_level));
    set_language(&settings);

    // 未設定なら、通知ではなく設定 GUI を開いて促す（初回起動の流れ）
    let open_gui = unset::prompt(unset::When::Startup, &settings) == Some(unset::Prompt::OpenGui);
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
        false,
        app.settings_path.as_deref(),
    );
    #[cfg(debug_assertions)]
    start_test_panic(app.windows.message);
    let message_window = app.windows.message;
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    // 依頼が届くのは App を置いてから
    match pipe_server::start(
        message_window,
        WM_APP_PIPE,
        text("job.panicked").to_owned(),
        text("cli.timed_out").to_owned(),
    ) {
        Ok((server, replier)) => {
            let _ = REPLIER.set(replier);
            with_app(|app| app.pipe = Some(server));
        }
        Err(e) => log::error!("could not start the pipe server: {e}"),
    }
    if let Some((level, body)) = notice {
        notify_user(level, &body);
    }
    sync_autostart();
    if open_gui {
        log::info!("the select-capture hotkey is unset; opening the settings GUI");
        settings_gui::open_for_unset_hotkey();
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
    toolbar_window::register_class(module, Some(toolbar_wndproc))?;

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
        own_writes: settings::OwnWrites::default(),
        shortcuts: keymap.shortcuts,
        mouse: keymap.mouse,
        capture: Capture::Idle,
        regions_waiting: VecDeque::new(),
        menu_retries: 0,
        menu_not_before: None,
        scraps: Vec::new(),
        trash,
        saves: mpsc::channel(),
        pipe: None,
        pending_saves: Vec::new(),
        save_fallbacks: Vec::new(),
        recipe: Parked::default(),
        renderer: None,
        carryover: Carryover::default(),
        notices: Notices::default(),
        gpu: grab::Gpu::default(),
        settings_gui: None,
        hotkey_pause: HotkeyPause::default(),
        unregistered: failed.iter().map(|f| f.action.into()).collect(),
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
            // 設定 GUI へのメッセージ（Tab の移動など）はダイアログに任せる
            _ if settings_gui::is_dialog_message(&msg) => {}
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

/// 書き込みのツールバーのウィンドウのプロシージャ。
unsafe extern "system" fn toolbar_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 受け取った引数をそのまま渡す
    unsafe { dispatch(on_toolbar_message, hwnd, msg, wparam, lparam) }
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
        WM_APP_PIPE => {
            on_pipe_requests(hwnd);
            Some(LRESULT(0))
        }
        WM_TIMER if wparam.0 == TIMER_PIPE_RETRY => {
            // SAFETY: 自分のタイマーを止める
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_PIPE_RETRY);
            }
            on_pipe_requests(hwnd);
            Some(LRESULT(0))
        }
        WM_HOTKEY => {
            on_hotkey(wparam);
            Some(LRESULT(0))
        }
        WM_APP_REGION => {
            run_waiting_regions();
            Some(LRESULT(0))
        }
        WM_TIMER if wparam.0 == TIMER_REGION_RETRY => {
            // SAFETY: 自分のタイマーを止める
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_REGION_RETRY);
            }
            run_waiting_regions();
            Some(LRESULT(0))
        }
        WM_TIMER if wparam.0 == TIMER_CAPTURE_DELAY => {
            // SAFETY: 自分のタイマーを止める
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_CAPTURE_DELAY);
            }
            take_capture();
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
            log::info!("another instance was launched; opening the settings GUI");
            settings_gui::open(None);
            Some(LRESULT(0))
        }
        _ => None,
    }
}

fn on_tray(owner: HWND, event: tray::Event) {
    match event {
        tray::Event::Menu { x, y } => {
            // ごみ箱の一覧（新しいものから）。項目の番号は CMD_TRASH_FIRST + 添字
            // サムネイルは小さいので写して持つ（メニューはモーダルなので App を借りたまま出せない）
            let trashed: Vec<(TrashId, String, Image)> = with_app(|app| {
                app.trash
                    .list()
                    .iter()
                    .map(|listed| (listed.id, trash_label(listed), listed.thumbnail.clone()))
                    .collect()
            })
            .unwrap_or_default();
            let mut trash_entries: Vec<menu::Entry> = trashed
                .iter()
                .enumerate()
                .map(|(i, (_, label, thumbnail))| menu::Entry::Picture {
                    id: CMD_TRASH_FIRST + i as u32,
                    label: label.as_str().into(),
                    image: thumbnail,
                })
                .collect();
            if trash_entries.is_empty() {
                trash_entries.push(menu::Entry::item(
                    CMD_TRASH_FIRST,
                    text("tray.menu.trash_empty"),
                    false,
                ));
            }
            // 定型領域の一覧（設定ファイルの順）。項目の番号は CMD_REGION_FIRST + 添字。名前の & はそのまま出す
            let region_names: Vec<String> = with_app(|app| {
                app.settings
                    .regions
                    .iter()
                    .map(|region| shunsoku_core::menu::literal_label(&region.name))
                    .collect()
            })
            .unwrap_or_default();
            let mut region_entries: Vec<menu::Entry> = region_names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    menu::Entry::item(CMD_REGION_FIRST + i as u32, name.as_str(), true)
                })
                .collect();
            if region_entries.is_empty() {
                region_entries.push(menu::Entry::item(
                    CMD_REGION_FIRST,
                    text("tray.menu.regions_empty"),
                    false,
                ));
            }
            // 1 枚でも隠れていれば「すべて表示」、なければ「すべて隠す」
            let (any_scrap, any_hidden) = with_app(|app| {
                (
                    !app.scraps.is_empty(),
                    app.scraps.iter().any(|s| !s.window.is_visible()),
                )
            })
            .unwrap_or_default();
            use shunsoku_core::tray::MenuItem;
            let (mut region_entries, mut trash_entries) =
                (Some(region_entries), Some(trash_entries));
            let items: Vec<menu::Entry> = shunsoku_core::tray::menu(any_hidden)
                .into_iter()
                .map(|item| match item {
                    MenuItem::SelectCapture => menu::Entry::item(
                        CMD_SELECT_CAPTURE,
                        text("tray.menu.select_capture"),
                        true,
                    ),
                    MenuItem::BlankScrap => {
                        menu::Entry::item(CMD_BLANK_SCRAP, text("tray.menu.blank_scrap"), true)
                    }
                    MenuItem::Regions => menu::Entry::Submenu {
                        label: text("tray.menu.regions").into(),
                        enabled: true,
                        items: region_entries.take().unwrap_or_default(),
                    },
                    MenuItem::AddRegion => {
                        menu::Entry::item(CMD_ADD_REGION, text("tray.menu.add_region"), true)
                    }
                    MenuItem::Open => menu::Entry::item(CMD_OPEN, text("tray.menu.open"), true),
                    MenuItem::Trash => menu::Entry::Submenu {
                        label: text("tray.menu.trash").into(),
                        enabled: true,
                        items: trash_entries.take().unwrap_or_default(),
                    },
                    MenuItem::CloseAll => {
                        menu::Entry::item(CMD_CLOSE_ALL, text("tray.menu.close_all"), any_scrap)
                    }
                    MenuItem::HideAll => {
                        menu::Entry::item(CMD_HIDE_ALL, text("tray.menu.hide_all"), any_scrap)
                    }
                    MenuItem::ShowAll => {
                        menu::Entry::item(CMD_SHOW_ALL, text("tray.menu.show_all"), true)
                    }
                    MenuItem::Separator => menu::Entry::Separator,
                    MenuItem::Settings => {
                        menu::Entry::item(CMD_SETTINGS, text("tray.menu.settings"), true)
                    }
                    MenuItem::Exit => menu::Entry::item(CMD_EXIT, text("tray.menu.exit"), true),
                })
                .collect();
            // モーダルなので App を借りずに呼ぶ
            let Some(command) = menu::show(owner, &items, x, y) else {
                return;
            };
            if let Some(i) = command.checked_sub(CMD_REGION_FIRST) {
                request_region(i as usize, "tray");
                return;
            }
            match command.checked_sub(CMD_TRASH_FIRST) {
                Some(i) => {
                    if let Some(&(id, ..)) = trashed.get(i as usize) {
                        restore_scrap(id);
                    }
                }
                None => on_command(command),
            }
        }
        tray::Event::NotificationClick => {
            let folder = with_app(|app| app.notices.clicked().map(Path::to_path_buf)).flatten();
            // フォルダを開くシェルの呼び出しはメッセージを回すことがあるので、App を借りずに呼ぶ
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
        CMD_SELECT_CAPTURE => {
            chosen_from_menu();
            request_capture(Source::TrayMenu);
        }
        CMD_ADD_REGION => {
            chosen_from_menu();
            request_capture(Source::AddRegion);
        }
        CMD_BLANK_SCRAP => {
            chosen_from_menu();
            request_capture(Source::BlankScrap);
        }
        CMD_CLOSE_ALL => close_all_scraps(),
        CMD_HIDE_ALL => set_scraps_visible(false),
        CMD_SHOW_ALL => set_scraps_visible(true),
        CMD_SETTINGS => settings_gui::open(None),
        CMD_OPEN => open_scrap_file(),
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
        // 2 回目の押下は WM_LBUTTONDOWN の代わりにこれで来る（CS_DBLCLKS）
        WM_LBUTTONDBLCLK => {
            // SAFETY: 自分のウィンドウ
            unsafe { SetCapture(hwnd) };
            on_selection_input(Input::DoubleClick { at: at() });
            Some(LRESULT(0))
        }
        WM_RBUTTONDOWN => {
            on_selection_input(Input::Press {
                button: Button::Right,
                at: at(),
            });
            Some(LRESULT(0))
        }
        // 調整の状態では、つまみや範囲の上でカーソルの形を変える
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let shape = with_app(|app| match &app.capture {
                Capture::Selecting { selection, .. } => Some(selection.cursor_shape()),
                _ => None,
            })
            .flatten();
            overlay::set_cursor(shape.unwrap_or(CursorShape::Cross));
            Some(LRESULT(1))
        }
        WM_KEYDOWN => {
            if let Some(input) = selection_key(wparam, lparam) {
                on_selection_input(input);
            }
            Some(LRESULT(0))
        }
        WM_KEYUP => {
            if let Some(input) = selection_modifiers(wparam) {
                on_selection_input(input);
            }
            Some(LRESULT(0))
        }
        // 他のアプリに前面を取られるとキー入力が来なくなり、閉じられなくなるので閉じる
        // （確定・取り消しで閉じた後にも届くので、そのときは何もしない）
        WM_ACTIVATEAPP if wparam.0 == 0 => {
            close_overlay("the overlay lost the foreground");
            Some(LRESULT(0))
        }
        // モニタの構成が変わると、固定された画面は実際の画面と合わなくなる
        WM_DISPLAYCHANGE => {
            close_overlay("the display changed");
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
            // Shift を押していれば、選択の道具では選択に足したり外したりする
            let press = if wparam.0 & MK_SHIFT != 0 {
                EditInput::ShiftPress(at)
            } else {
                EditInput::Press(at)
            };
            if on_edit_input(hwnd, press).is_some() {
                return Some(LRESULT(0));
            }
            if !on_trim_input(hwnd, TrimInput::Press(at)) {
                with_app(|app| {
                    if let Some(scrap) = app.scrap_mut(hwnd) {
                        scrap.drag.start(at);
                    }
                });
            }
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            if is_editing(hwnd) {
                on_edit_move(hwnd, wparam, message_screen_point());
            } else if is_trimming(hwnd) {
                on_trim_move(hwnd, wparam, message_screen_point());
            } else {
                on_scrap_drag(hwnd, wparam, message_screen_point());
            }
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
            on_trim_input(hwnd, TrimInput::Release);
            on_edit_input(hwnd, EditInput::Release);
            Some(LRESULT(0))
        }
        // 2 回目の押下は WM_LBUTTONDOWN の代わりにこれで来る（CS_DBLCLKS）。トリムの最中は確定にする
        // 書き込みの編集モードでは core に渡す（文字の上なら打ち直し、ほかはふつうの押下と同じに描き始める）
        WM_LBUTTONDBLCLK => {
            let at = message_screen_point();
            if is_editing(hwnd) {
                // SAFETY: 自分のウィンドウ
                unsafe { SetCapture(hwnd) };
                on_edit_input(hwnd, EditInput::DoubleClick(at));
                return Some(LRESULT(0));
            }
            // トリムの最中は、範囲の中なら確定、ほかはふつうの押下と同じ（core が決める）
            if is_trimming(hwnd) {
                // SAFETY: 自分のウィンドウ
                unsafe { SetCapture(hwnd) };
                on_trim_input(hwnd, TrimInput::DoubleClick(at));
                return Some(LRESULT(0));
            }
            let context = OpContext {
                cursor: Some(at),
                ..OpContext::default()
            };
            run_mouse_operation(hwnd, MouseGesture::DoubleClick, context);
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
        WM_KEYDOWN | WM_SYSKEYDOWN => on_scrap_key(hwnd, wparam, lparam).then_some(LRESULT(0)),
        // トリムの最中は、範囲のつまみや中の上でカーソルの形を変える
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT && is_trimming(hwnd) => {
            let shape = with_app(|app| app.scrap(hwnd).map(|s| s.model.trim_cursor_shape()))
                .flatten()
                .unwrap_or(CursorShape::Cross);
            overlay::set_cursor(shape);
            Some(LRESULT(1))
        }
        // 書き込みの編集モードで消しゴムを選んでいる間は、消しゴムのカーソルにする
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT && has_eraser(hwnd) => {
            scrap_window::set_eraser_cursor();
            Some(LRESULT(1))
        }
        // 文字を打っている間の文字（TranslateMessage が作る）
        WM_CHAR => on_scrap_char(hwnd, wparam).then_some(LRESULT(0)),
        // 変換中の文字は IME の窓に出さず、スクラップに描く。App は借りない（IME の呼び出しの中でも届く）
        WM_IME_SETCONTEXT => {
            // SAFETY: 受け取った引数を（印を 1 つ落として）そのまま渡す
            Some(unsafe {
                DefWindowProcW(hwnd, msg, wparam, ime::without_composition_window(lparam))
            })
        }
        WM_IME_STARTCOMPOSITION if is_typing(hwnd) => Some(LRESULT(0)),
        WM_IME_COMPOSITION if is_typing(hwnd) => {
            on_ime_composition(hwnd, lparam);
            Some(LRESULT(0))
        }
        WM_IME_ENDCOMPOSITION if is_typing(hwnd) => {
            on_edit_input(
                hwnd,
                EditInput::Composition {
                    result: String::new(),
                    composing: None,
                },
            );
            Some(LRESULT(0))
        }
        // 右クリック（DefWindowProcW が WM_RBUTTONUP から作る）と Shift+F10・アプリケーション キー
        WM_CONTEXTMENU => {
            show_scrap_menu(hwnd, lparam);
            Some(LRESULT(0))
        }
        // ほかのウィンドウにフォーカスが移ったら、書き込みの編集モードを抜ける。既定の処理にも回す
        // （隠す・壊すときなど App を借りている間にも届くので、送り直してから抜ける）
        WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
            // SAFETY: 自分のウィンドウに送る
            if let Err(e) =
                unsafe { PostMessageW(Some(hwnd), WM_APP_SCRAP_INACTIVE, WPARAM(0), LPARAM(0)) }
            {
                log::warn!("could not post the deactivation of a scrap: {e}");
            }
            None
        }
        // 送り直すまでの間にもう一度アクティブになっていれば（ツールバーは押してもアクティブにならない）抜けない
        WM_APP_SCRAP_INACTIVE => {
            // SAFETY: 引数なし
            if unsafe { GetForegroundWindow() } != hwnd {
                on_edit_input(hwnd, EditInput::Leave);
            }
            Some(LRESULT(0))
        }
        // Alt+F4 など。ウィンドウだけ壊されて App に残らないように、閉じるのと同じにする
        WM_CLOSE => {
            close_scrap(hwnd);
            Some(LRESULT(0))
        }
        // 提案された矩形は当てない（大きさは core が決める。`settings::DpiScaling`）
        WM_DPICHANGED => {
            let dpi = wparam.0 & 0xFFFF;
            // SAFETY: 自分のウィンドウに送る
            if let Err(e) =
                unsafe { PostMessageW(Some(hwnd), WM_APP_SCRAP_DPI, WPARAM(dpi), LPARAM(0)) }
            {
                log::warn!("could not post the dpi change of a scrap: {e}");
            }
            Some(LRESULT(0))
        }
        WM_APP_SCRAP_DPI => {
            on_scrap_dpi_changed(hwnd, wparam.0 as u32);
            Some(LRESULT(0))
        }
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
    fn scrap(&self, hwnd: HWND) -> Option<&Scrap> {
        self.scraps.iter().find(|s| s.window.hwnd() == hwnd)
    }

    fn scrap_mut(&mut self, hwnd: HWND) -> Option<&mut Scrap> {
        self.scraps.iter_mut().find(|s| s.window.hwnd() == hwnd)
    }

    /// `hwnd` のスクラップと、描くのに要る設定の見た目と描画器を、まとめて借りる。
    fn scrap_to_draw(
        &mut self,
        hwnd: HWND,
    ) -> Option<(&mut Scrap, Appearance, &mut Option<render::Renderer>)> {
        let appearance = Appearance::from(self.settings.scrap);
        let App {
            scraps, renderer, ..
        } = self;
        let scrap = scraps.iter_mut().find(|s| s.window.hwnd() == hwnd)?;
        Some((scrap, appearance, renderer))
    }
}

/// `WM_MOUSEMOVE` の `wparam` と、`hwnd` がマウスを捕まえているかから作る、左ボタンの様子。
fn pointer_on_move(hwnd: HWND, wparam: WPARAM) -> Pointer {
    // WM_MOUSEMOVE の wparam の MK_LBUTTON（Win32_System_SystemServices を足さないよう値で持つ）
    const MK_LBUTTON: usize = 0x0001;
    Pointer {
        left_down: wparam.0 & MK_LBUTTON != 0,
        // SAFETY: 引数なし
        captured: unsafe { GetCapture() } == hwnd,
    }
}

/// 離したことを取りこぼしてマウスを捕まえたままなら放す。WM_CAPTURECHANGED が届くので、App を借りずに呼ぶ。
fn release_stray_capture(pointer: Pointer) {
    if pointer.stray_capture() {
        // SAFETY: UI スレッドで呼ぶ
        let _ = unsafe { ReleaseCapture() };
    }
}

/// 左ドラッグの間のカーソルの移動を、ドラッグの操作に渡す。押したままでなければ（離したことを取りこぼした
/// など）ドラッグを終える（`Drag::move_to`）。
fn on_scrap_drag(hwnd: HWND, wparam: WPARAM, at: Point) {
    let pointer = pointer_on_move(hwnd, wparam);
    let moved = with_app(|app| {
        let drag = &mut app.scrap_mut(hwnd)?.drag;
        let was_dragging = drag.is_active();
        Some((was_dragging, drag.move_to(at, pointer)))
    })
    .flatten();
    release_stray_capture(pointer);
    let Some((was_dragging, delta)) = moved else {
        return;
    };
    if was_dragging && !pointer.holding() {
        log::debug!("scrap drag ended without a button release");
    }
    if let Some(drag) = delta {
        let context = OpContext {
            drag,
            ..OpContext::default()
        };
        run_mouse_operation(hwnd, MouseGesture::LeftDrag, context);
    }
}

/// 書き込みのツールバーのウィンドウへのメッセージ。押してもアクティブにならない（スクラップのフォーカスを奪わない）。
fn on_toolbar_message(hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
        WM_LBUTTONDOWN => {
            on_toolbar_press(hwnd, message_screen_point());
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            on_toolbar_drag(hwnd, wparam, message_screen_point());
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
                if let Some(toolbar) = app.toolbar_mut(hwnd) {
                    toolbar.drag.end();
                }
            });
            Some(LRESULT(0))
        }
        // つまみの上では移動のカーソルにする
        WM_SETCURSOR => {
            let on_grip = with_app(|app| {
                app.toolbar_mut(hwnd)
                    .and_then(|t| t.hit(message_screen_point()))
                    == Some(ToolbarButton::Grip)
            })
            .unwrap_or(false);
            if !on_grip {
                return None;
            }
            // SAFETY: 共有のカーソルなので壊さなくてよい
            if let Ok(cursor) = unsafe { LoadCursorW(None, IDC_SIZEALL) } {
                // SAFETY: UI スレッドで呼ぶ
                unsafe { SetCursor(Some(cursor)) };
            }
            Some(LRESULT(1))
        }
        // 大きさはスクラップのモニタの DPI で決める（`Scrap::sync_toolbar`）
        WM_DPICHANGED => Some(LRESULT(0)),
        _ => None,
    }
}

impl App {
    /// ツールバーのウィンドウ `hwnd` を持つスクラップ。
    fn scrap_of_toolbar(&mut self, hwnd: HWND) -> Option<&mut Scrap> {
        self.scraps
            .iter_mut()
            .find(|s| s.toolbar.as_ref().is_some_and(|t| t.window.hwnd() == hwnd))
    }

    fn toolbar_mut(&mut self, hwnd: HWND) -> Option<&mut Toolbar> {
        self.scrap_of_toolbar(hwnd)?.toolbar.as_mut()
    }
}

/// ツールバーのボタンを押した: 道具・色・太さを選ぶ、完了で抜ける、つまみならドラッグを始める。
fn on_toolbar_press(hwnd: HWND, at: Point) {
    let Some((scrap, button)) = with_app(|app| {
        let scrap = app.scrap_of_toolbar(hwnd)?;
        let button = scrap.toolbar.as_ref()?.hit(at)?;
        Some((scrap.window.hwnd(), button))
    })
    .flatten() else {
        return;
    };
    match button {
        ToolbarButton::Grip => {
            // 速く動かしても追えるようにマウスを捕まえる。キー入力とアクティブはスクラップのまま
            // SAFETY: 自分のウィンドウ
            unsafe { SetCapture(hwnd) };
            with_app(|app| {
                if let Some(toolbar) = app.toolbar_mut(hwnd) {
                    toolbar.drag.start(at);
                }
            });
        }
        ToolbarButton::Done => {
            on_edit_input(scrap, EditInput::Leave);
        }
        ToolbarButton::Delete => {
            on_edit_input(scrap, EditInput::Delete);
        }
        ToolbarButton::Undo => {
            on_edit_input(scrap, EditInput::Undo);
        }
        ToolbarButton::Redo => {
            on_edit_input(scrap, EditInput::Redo);
        }
        button => {
            if let Some(pick) = button.pick() {
                on_edit_input(scrap, EditInput::Pick(pick));
            }
        }
    }
}

/// つまみのドラッグの間のカーソルの移動で、スクラップごと動かす（ツールバーはついてくる）。編集モードの間に
/// スクラップを動かせるのはここだけ。
fn on_toolbar_drag(hwnd: HWND, wparam: WPARAM, at: Point) {
    let pointer = pointer_on_move(hwnd, wparam);
    with_app(|app| {
        let scrap = app.scrap_of_toolbar(hwnd)?;
        let delta = scrap.toolbar.as_mut()?.drag.move_to(at, pointer)?;
        let scrap_hwnd = scrap.window.hwnd();
        let (scrap, appearance, renderer) = app.scrap_to_draw(scrap_hwnd)?;
        let before = scrap.model.clone();
        let p = scrap.model.position;
        scrap.model.position = Point::new(p.x.saturating_add(delta.x), p.y.saturating_add(delta.y));
        // 動かすときに届くメッセージでは App を借りないので、借りたまま動かす
        scrap.redraw_or_revert(&before, &appearance, renderer, false, "toolbar grip");
        Some(())
    });
    release_stray_capture(pointer);
}

/// 書き込みの編集モードにいるか。
fn is_editing(hwnd: HWND) -> bool {
    with_app(|app| {
        app.scraps
            .iter()
            .any(|s| s.window.hwnd() == hwnd && s.model.is_editing())
    })
    .unwrap_or(false)
}

/// マウスのメッセージの wparam の、`Shift` を押しているかの印。Win32_System_SystemServices を足さないよう値で持つ。
const MK_SHIFT: usize = 0x0004;

/// 書き込みの編集モードのカーソルの移動。押したままでなければ（離したことを取りこぼしたなど）離したことにする。
fn on_edit_move(hwnd: HWND, wparam: WPARAM, at: Point) {
    let pointer = pointer_on_move(hwnd, wparam);
    let input = if pointer.holding() {
        // Shift を押していれば図形に制約をかける（正方形・円・45° 刻み）
        let shift = wparam.0 & MK_SHIFT != 0;
        EditInput::Move { at, shift }
    } else {
        EditInput::Release
    };
    on_edit_input(hwnd, input);
    release_stray_capture(pointer);
}

/// 書き込みの編集モードに入力を渡し、変わったら描き直す。編集モードでなければ何もせず None。
/// 変換中の文字を確定したことにしたら（`EditOutput::CompositionTaken`）IME の側の変換中の文字を捨て、打ち始めたら・
/// 打ち終えたら IME を使う・使わないを切り替える。
fn on_edit_input(hwnd: HWND, input: EditInput) -> Option<Vec<EditOutput>> {
    let outputs = edit_input(hwnd, input);
    if let Some(outputs) = &outputs {
        if outputs.contains(&EditOutput::CompositionTaken) {
            ime::cancel(hwnd);
        }
        if outputs.contains(&EditOutput::ToolChanged) {
            refresh_edit_cursor(hwnd);
        }
        if !outputs.is_empty() {
            let switch = outputs
                .iter()
                .any(|o| matches!(o, EditOutput::TypingStarted | EditOutput::TypingEnded));
            sync_ime(hwnd, switch);
        }
    }
    outputs
}

/// 書き込みの編集モードで消しゴムを選んでいるか。
fn has_eraser(hwnd: HWND) -> bool {
    with_app(|app| {
        app.scrap(hwnd)
            .is_some_and(|s| s.model.edit_state().is_some_and(|e| e.tool == Tool::Eraser))
    })
    .unwrap_or(false)
}

/// 道具が替わったら、カーソルがスクラップの上にあればその形にし直す（キーで替えたとき、動かすまで前の形の
/// ままにならないように）。ツールバーの上なら、スクラップに戻ったときの `WM_SETCURSOR` に任せる。
fn refresh_edit_cursor(hwnd: HWND) {
    let mut at = POINT::default();
    // SAFETY: 書き込み先は生きている
    if unsafe { GetCursorPos(&mut at) }.is_err() {
        return;
    }
    // SAFETY: 問い合わせだけ
    if unsafe { WindowFromPoint(at) } != hwnd {
        return;
    }
    if has_eraser(hwnd) {
        scrap_window::set_eraser_cursor();
    } else {
        // SAFETY: 共有のカーソルなので壊さなくてよい
        if let Ok(cursor) = unsafe { LoadCursorW(None, IDC_ARROW) } {
            // SAFETY: UI スレッドで呼ぶ
            unsafe { SetCursor(Some(cursor)) };
        }
    }
}

/// 文字を打っているか。
fn is_typing(hwnd: HWND) -> bool {
    with_app(|app| {
        app.scrap(hwnd)
            .is_some_and(|s| s.model.edit_state().is_some_and(EditState::is_typing))
    })
    .unwrap_or(false)
}

/// `WM_CHAR`: 文字を打っている間の文字を渡す（サロゲート ペアは core が組み立てる）。受けなければ false。
fn on_scrap_char(hwnd: HWND, wparam: WPARAM) -> bool {
    on_edit_input(hwnd, EditInput::Utf16(wparam.0 as u16)).is_some_and(|o| !o.is_empty())
}

/// `WM_IME_COMPOSITION`: 確定した文字と変換中の文字を渡す。
fn on_ime_composition(hwnd: HWND, lparam: LPARAM) {
    let update = ime::read_composition(hwnd, lparam);
    on_edit_input(
        hwnd,
        EditInput::Composition {
            result: update.result,
            composing: update.composition,
        },
    );
}

/// 打っている間は、変換中の文字と候補の窓をキャレットに合わせる。`switch`（打ち始めた・打ち終えた）なら、
/// 打っていれば IME を使えるようにし、打っていなければ使えないようにする。IME を呼ぶのは App を返してから。
fn sync_ime(hwnd: HWND, switch: bool) {
    let Some((typing, caret)) = with_app(|app| {
        let appearance = Appearance::from(app.settings.scrap);
        let scrap = app.scrap(hwnd)?;
        let typing = scrap.model.edit_state().is_some_and(EditState::is_typing);
        let origin = scrap.model.window_rect(&appearance).origin();
        let caret = scrap.caret.get().map(|r| r.offset(origin.x, origin.y));
        Some((typing, caret))
    })
    .flatten() else {
        return;
    };
    match (switch, typing) {
        (true, true) => ime::enable(hwnd),
        (true, false) => ime::disable(hwnd),
        (false, _) => {}
    }
    if let (true, Some(caret)) = (typing, caret) {
        ime::place(hwnd, caret);
    }
}

/// 書き込みの編集モードに入力を渡し、変わったら描き直す（`on_edit_input` の中身）。
fn edit_input(hwnd: HWND, input: EditInput) -> Option<Vec<EditOutput>> {
    with_app(|app| {
        let (scrap, appearance, renderer) = app.scrap_to_draw(hwnd)?;
        if !scrap.model.is_editing() {
            return None;
        }
        let outputs = scrap.model.edit_input(input);
        if outputs.contains(&EditOutput::Left) {
            log::info!(
                "edit mode left ({} annotations)",
                scrap.model.annotations().items().len()
            );
            // ツールバーのウィンドウを壊すときに届くメッセージでは App を借りない
            scrap.toolbar = None;
        }
        for output in &outputs {
            if let EditOutput::Picked(pick) = output {
                log::info!("edit pick {pick:?}");
            }
        }
        if !outputs.is_empty() {
            // 描くだけで App を借りるメッセージは届かないので、借りたまま描く
            if let Err(e) = scrap.paint(&appearance, renderer) {
                log::warn!("could not draw the annotations: {e}");
            }
        }
        app.carryover.record(&outputs);
        Some(outputs)
    })
    .flatten()
}

/// トリムの範囲を選んでいるか。
fn is_trimming(hwnd: HWND) -> bool {
    with_app(|app| app.scrap(hwnd).is_some_and(|s| s.model.is_trimming())).unwrap_or(false)
}

/// トリムの最中のカーソルの移動。押したままでなければ（離したことを取りこぼしたなど）押していないことにする
/// （`TrimInput::pointer_move`）。
fn on_trim_move(hwnd: HWND, wparam: WPARAM, at: Point) {
    let pointer = pointer_on_move(hwnd, wparam);
    on_trim_input(hwnd, TrimInput::pointer_move(at, pointer));
    release_stray_capture(pointer);
}

/// トリムの範囲の選択に入力を渡し、変わったら描き直す。トリムの最中でなければ何もせず false。
fn on_trim_input(hwnd: HWND, input: TrimInput) -> bool {
    with_app(|app| {
        let (scrap, appearance, renderer) = app.scrap_to_draw(hwnd)?;
        if !scrap.model.is_trimming() {
            return Some(false);
        }
        let before = scrap.model.clone();
        if scrap.model.trim_input(input) {
            // 動かす・描き直すときに届くメッセージでは App を借りないので、借りたまま描く
            scrap.redraw_or_revert(&before, &appearance, renderer, true, "trimming");
            if scrap.model.mode == Mode::Normal {
                log::info!("trim finished (trimmed: {})", scrap.model.is_trimmed());
            }
        }
        Some(true)
    })
    .flatten()
    .unwrap_or(false)
}

/// `WM_MOUSEWHEEL`: ホイールを、ノッチごとにホイールの操作（`MouseGesture::wheel`）にする（基準点はカーソルの
/// 位置）。書き込みの編集モードの間は、ノッチごとに道具・色・太さを選ぶ（`on_edit_wheel`）。対応のない
/// 修飾キーつきは false（DefWindowProcW に任せる）。
fn on_scrap_wheel(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    // wparam の下位は押されているキーとボタン（MK_*）、上位は回転量（上が正）
    const MK_SHIFT: usize = 0x0004;
    const MK_CONTROL: usize = 0x0008;
    let delta = i32::from((wparam.0 >> 16) as u16 as i16);
    // Alt と Win は wparam にないので、キーの状態から読む
    let held = held_modifiers();
    let modifiers = Modifiers {
        ctrl: wparam.0 & MK_CONTROL != 0,
        shift: wparam.0 & MK_SHIFT != 0,
        ..held
    };
    if is_editing(hwnd) {
        return on_edit_wheel(hwnd, modifiers, delta);
    }
    let Some(gesture) = MouseGesture::wheel(modifiers, delta) else {
        return false;
    };
    // WM_MOUSEWHEEL の座標はスクリーン座標。左や上のモニタでは負になる
    let cursor = Point::new(
        i32::from((lparam.0 & 0xFFFF) as u16 as i16),
        i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16),
    );
    let Some(notches) = wheel_notches(hwnd, delta) else {
        return true;
    };
    // 向きが変わるとためた分は捨てるので、ノッチの向きは回転の向きと同じ
    let context = OpContext {
        cursor: Some(cursor),
        ..OpContext::default()
    };
    for _ in 0..notches.unsigned_abs() {
        run_mouse_operation(hwnd, gesture, context.clone());
    }
    true
}

/// 書き込みの編集モードのホイール: ノッチごとに、core の `toolbar::wheel_pick` が選ぶ道具・色・太さを、
/// ツールバーで選んだのと同じに渡す。対応のない修飾キーつきは false。
fn on_edit_wheel(hwnd: HWND, modifiers: Modifiers, delta: i32) -> bool {
    let pick = |rotation: i32| {
        with_app(|app| {
            let scrap = app.scrap(hwnd)?;
            let state = scrap.model.edit_state()?;
            let colours = match &scrap.toolbar {
                Some(toolbar) => Cow::Borrowed(toolbar.colours.as_slice()),
                // ツールバーを作れなかったときも、ツールバーに並べるはずだった色で選ぶ
                None => Cow::Owned(toolbar::palette(app.settings.annotation.color)),
            };
            toolbar::wheel_pick(state, &colours, modifiers, rotation)
        })
        .flatten()
    };
    if pick(delta).is_none() {
        return false;
    }
    let Some(notches) = wheel_notches(hwnd, delta) else {
        return true;
    };
    // 1 ノッチで 1 段。選ぶたびに状態が変わるので、ノッチごとに選び直す
    for _ in 0..notches.unsigned_abs() {
        match pick(notches.signum()) {
            Some(p) => {
                on_edit_input(hwnd, EditInput::Pick(p));
            }
            None => break,
        }
    }
    true
}

/// ホイールの回転量 `delta` をスクラップ `hwnd` にためて、回り切ったノッチの数（上が正）を返す。スクラップが
/// なければ None。
fn wheel_notches(hwnd: HWND, delta: i32) -> Option<i32> {
    with_app(|app| Some(app.scrap_mut(hwnd)?.wheel.turn(delta))).flatten()
}

/// `WM_APP_SCRAP_DPI`: 新しい DPI を core に渡し、表示の大きさが変わったら描き直す。基準点はドラッグ中なら
/// カーソル（つかんだところが離れないように）、そうでなければ中心。
fn on_scrap_dpi_changed(hwnd: HWND, dpi: u32) {
    let at = message_screen_point();
    with_app(|app| {
        let scaling = app.settings.general.dpi_scaling;
        let (scrap, appearance, renderer) = app.scrap_to_draw(hwnd)?;
        let anchor = scrap.drag.zoom_anchor(at);
        let before = scrap.model.clone();
        if scrap.model.set_dpi(dpi, scaling, anchor) {
            log::debug!(
                "scrap zoom {}% -> {}% at {dpi} dpi",
                before.zoom(),
                scrap.model.zoom()
            );
            // 描くだけで App を借りるメッセージは届かないので、借りたまま描く
            scrap.redraw_or_revert(&before, &appearance, renderer, false, "new dpi");
        }
        Some(())
    });
}

/// `TIMER_ZOOM_LABEL`: 倍率の表示を消して描き直す。
fn hide_zoom_label(hwnd: HWND) {
    // SAFETY: 自分のタイマーを止める
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_ZOOM_LABEL);
    }
    with_app(|app| {
        let (scrap, appearance, renderer) = app.scrap_to_draw(hwnd)?;
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

/// マウスの操作をマウスの対応表で引いて実行する。書き込みの編集モードの間は、どの割り当ても効かない。
fn run_mouse_operation(hwnd: HWND, gesture: MouseGesture, context: OpContext) {
    if is_editing(hwnd) {
        return;
    }
    if let Some(op) = with_app(|app| app.mouse.operation(gesture)).flatten() {
        run_scrap_operation(hwnd, op, context);
    }
}

/// キーをショートカットの対応表で引いて実行する。対応表になければ false。
fn on_scrap_key(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    // トリムの最中は Enter・Space で確定、Esc で中止、矢印で範囲を直す。ほかのキーの操作は効かない（どの操作も
    // 実行できない）
    if is_trimming(hwnd) {
        let Some(input) = trim_key(wparam, lparam) else {
            return false;
        };
        return on_trim_input(hwnd, input);
    }
    // 修飾キーだけのときは名前の表になく、None になる
    let Some(key) = Key::from_vk(wparam.0 as u16) else {
        return false;
    };
    let chord = Chord {
        modifiers: held_modifiers(),
        key,
    };
    // 書き込みの編集モードではショートカットは効かない。受けなかったキー（Alt+F4 など）は DefWindowProcW に任せる
    // 変換中の文字を確定しただけ（`CompositionTaken`）なら受けていない
    if let Some(outputs) = on_edit_input(hwnd, EditInput::Key(chord)) {
        return outputs.iter().any(|o| *o != EditOutput::CompositionTaken);
    }
    let bound = with_app(|app| {
        if let Some(op) = app.shortcuts.operation(chord) {
            Some(Bound::Operation(op))
        } else {
            let plan = app.shortcuts.recipe(chord)?;
            Some(Bound::Recipe(plan.name.clone(), plan.steps.clone()))
        }
    })
    .flatten();
    match bound {
        Some(Bound::Operation(op)) => run_scrap_operation(hwnd, op, OpContext::default()),
        Some(Bound::Recipe(name, steps)) => run_recipe(hwnd, &name, steps),
        None => return false,
    }
    true
}

/// ショートカットに付いたもの（`on_scrap_key` で `App` を返してから実行するため、借りずに持てる形にする）。
enum Bound {
    Operation(&'static OperationDescriptor),
    /// レシピの名前（ログ用）と手。
    Recipe(String, Vec<PlannedStep>),
}

/// レシピを実行する。手を 1 つずつ実行し、その手の `Effect` を実行し終えてから結果を返して次の手へ進む
/// （ADR-0007）。保存の結果を待っているレシピがあれば捨てる。
fn run_recipe(hwnd: HWND, name: &str, steps: Vec<PlannedStep>) {
    if with_app(|app| app.recipe.discard()).flatten().is_some() {
        log::info!("the recipe waiting for a save was discarded");
    }
    log::info!("recipe {name:?} started");
    drive_recipe(hwnd, Run::new(steps));
}

/// レシピを進められるところまで進める。失敗・取り消し・今は実行できない手で止まる。止まったわけはログに書く
/// （失敗の通知はその手の `Effect` を実行した側が出している）。保存を裏のスレッドに渡した手は、結果を受ける
/// まで `App::recipe` に預けて返る（`on_saved` が続きを進める）。
fn drive_recipe(hwnd: HWND, mut run: Run) {
    loop {
        let mut advance = None;
        let performed = perform(hwnd, OpContext::default(), |model, context| {
            let next = run.advance(model, context);
            let step = match &next {
                Advance::Step { op, effects } => Some((op.id, effects.clone())),
                _ => None,
            };
            advance = Some(next);
            step
        });
        match advance {
            Some(Advance::Step { op, .. }) => {
                let performed = performed.unwrap_or_default();
                match performed.saving {
                    Some(path) if performed.outcome == Outcome::Succeeded => {
                        log::info!("recipe step {} is waiting for {}", op.id, path.display());
                        let parked = with_app(|app| app.recipe.park(hwnd.0 as isize, path, run));
                        if parked.is_none() {
                            log::warn!("could not keep the recipe waiting for the save");
                        }
                        return;
                    }
                    _ => {
                        log::info!("recipe step {} -> {:?}", op.id, performed.outcome);
                        run.settle(performed.outcome);
                    }
                }
            }
            // 結果を待っている手はここで預けて返るので、待ったまま来ることはない
            Some(Advance::Waiting(id)) => {
                log::warn!("recipe is still waiting for {id}");
                return;
            }
            Some(Advance::Finished) => {
                log::info!("recipe finished");
                return;
            }
            Some(Advance::Stopped(stop)) => {
                log::info!("recipe stopped: {stop:?}");
                return;
            }
            // スクラップがもうない（最後の手の「閉じる」で閉じたなど）
            None => {
                log::info!("recipe ended with the scrap gone");
                return;
            }
        }
    }
}

/// いま押されている修飾キー（キーのメッセージを処理している時点の状態）。
fn held_modifiers() -> Modifiers {
    Modifiers {
        win: key_held(VK_LWIN) || key_held(VK_RWIN),
        ctrl: key_held(VK_CONTROL),
        alt: key_held(VK_MENU),
        shift: key_held(VK_SHIFT),
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
                checked: item.checked,
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
                enabled: folder.is_enabled(),
                items: scrap_menu_entries(&folder.entries, commands),
            },
            MenuEntry::Separator => menu::Entry::Separator,
        });
    }
    items
}

/// `perform` で実行した 1 手の結果。
#[derive(Default)]
struct Performed {
    /// その場でわかった結果（`Outcome::of`）。
    outcome: Outcome,
    /// 裏のスレッドに渡した保存の保存先。書き込みの結果は `WM_APP_SAVED` で届く。
    saving: Option<PathBuf>,
}

/// `App` を返したあとに行う `Effect`。
enum Deferred {
    Copy(Image),
    Save(Image),
    SaveAs(Image),
    SaveScrapFile(Box<ScrapFile>),
    /// コピーや保存の画像を作れなかった。`failed_key` はその仕事の失敗の文言のキー。
    RenderFailed {
        failed_key: &'static str,
        reason: String,
    },
    Close,
    /// 「その他の色…」: 今の背景の色から色の選択ダイアログを出す。
    PickBackground(Rgb),
}

/// コピーや保存の画像（書き込みを描いたもの）を表示と同じ描き方で作り、`job` の `Deferred` にする。作れなければ
/// `failed_key` の失敗にする。
fn render_for(scrap: &Scrap, job: fn(Image) -> Deferred, failed_key: &'static str) -> Deferred {
    let layer = render::AnnotationLayer {
        annotations: scrap.model.annotations().items(),
        view: scrap.model.output_view(),
        selected: &[],
        selection_range: None,
        typing: None,
        // 選択の印もキャレットも描かない
        scale: 1.0,
    };
    match render::render_image(scrap.shown(), layer) {
        Ok(image) => job(image),
        Err(e) => Deferred::RenderFailed {
            failed_key,
            reason: e.to_string(),
        },
    }
}

/// スクラップ操作を実行し、返った `Effect` を実行する。スクラップのキー・マウス・メニューはすべてここを通る。
fn run_scrap_operation(hwnd: HWND, op: &OperationDescriptor, context: OpContext) {
    perform(hwnd, context, |model, context| {
        Some((op.id, ops::execute(op, model, context)))
    });
}

/// `step` でスクラップの `ScrapModel` を変え、返った `Effect` を実行する。`step` は操作の id（描き直しに
/// 失敗したときのログ用）と `Effect` を返す。スクラップがないか `step` が None なら何もせず None。
///
/// 返すのは `Effect` の結果をまとめたもの（`Outcome::of`）と、裏のスレッドに渡した保存の保存先。コピーに
/// 失敗した・画像を作れなかった・保存先を決められなかったら失敗、名前を付けて保存のダイアログを取り消したら
/// 取り消し、ほかは成功。保存の書き込みの結果は、あとから `WM_APP_SAVED` で届く。
fn perform(
    hwnd: HWND,
    context: OpContext,
    step: impl FnOnce(&mut ScrapModel, &OpContext) -> Option<(OperationId, Vec<Effect>)>,
) -> Option<Performed> {
    let deferred = with_app(|app| {
        let annotation = Style::from(&app.settings.annotation);
        let font = Font::from(&app.settings.annotation);
        let crosshair = app.settings.selection.crosshair_color;
        let carryover = app.carryover;
        let module = app.module;
        let (scrap, appearance, renderer) = app.scrap_to_draw(hwnd)?;
        let before = scrap.model.clone();
        let mut deferred = Vec::new();
        let context = OpContext {
            appearance,
            annotation,
            font,
            carryover,
            ..context
        };
        let (op_id, effects) = step(&mut scrap.model, &context)?;
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
                    scrap.redraw_or_revert(&before, &appearance, renderer, show_zoom, op_id);
                }
                Effect::EnterTrimMode => {
                    log::info!("trim started");
                    scrap.crosshair = crosshair;
                    if let Err(e) = scrap.paint(&appearance, renderer) {
                        log::warn!("could not show the trim mode: {e}");
                    }
                }
                Effect::EnterEditMode | Effect::LeaveEditMode => {
                    let entered = effect == Effect::EnterEditMode;
                    log::info!("edit mode {}", if entered { "entered" } else { "left" });
                    // ツールバーのウィンドウを作る・壊すときに届くメッセージでは App を借りない
                    scrap.toolbar = if entered {
                        match toolbar_window::ToolbarWindow::create(module, hwnd) {
                            Ok(window) => Some(Toolbar {
                                window,
                                colours: toolbar::palette(annotation.color),
                                placed: Default::default(),
                                drag: Drag::default(),
                            }),
                            Err(e) => {
                                log::warn!("could not create the annotation toolbar: {e}");
                                None
                            }
                        }
                    } else {
                        None
                    };
                    if let Err(e) = scrap.paint(&appearance, renderer) {
                        log::warn!("could not show the edit mode: {e}");
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
                    deferred.push(render_for(scrap, Deferred::Copy, "copy.failed"));
                }
                Effect::SaveDefault => {
                    deferred.push(render_for(scrap, Deferred::Save, "save.failed"));
                }
                Effect::ShowSaveDialog => {
                    deferred.push(render_for(scrap, Deferred::SaveAs, "save.failed"));
                }
                Effect::ShowBackgroundDialog => {
                    let current = match scrap.model.background() {
                        Some(Background::Color(color)) => color,
                        _ => Rgb(0xFF, 0xFF, 0xFF),
                    };
                    deferred.push(Deferred::PickBackground(current));
                }
                // 描いた画像ではなく、元の画像と書き込みをそのまま書く
                Effect::ShowScrapFileSaveDialog => {
                    deferred.push(Deferred::SaveScrapFile(Box::new(ScrapFile::of(
                        &scrap.model,
                        &scrap.image,
                    ))));
                }
            }
        }
        Some(deferred)
    })
    .flatten()?;
    // 編集モードを抜けたら IME を使えないようにする（core が変換中の文字も書き込みにしてある）
    sync_ime(hwnd, true);
    // クリップボード・通知・ウィンドウを壊すのはメッセージが届くことがあるので、App を借りずに行う
    let mut saving = None;
    let mut save = |started: Saving| match started {
        Saving::Started(path) => {
            saving = Some(path);
            Outcome::Succeeded
        }
        Saving::Ended(outcome) => outcome,
    };
    let outcomes: Vec<Outcome> = deferred
        .into_iter()
        .map(|action| match action {
            Deferred::Copy(image) => copy_to_clipboard(&image),
            Deferred::Save(image) => save(save_default(image)),
            Deferred::SaveAs(image) => save(save_as(hwnd, image)),
            Deferred::SaveScrapFile(file) => save(save_scrap_file(hwnd, file)),
            Deferred::RenderFailed { failed_key, reason } => {
                log::error!("could not render the scrap image ({failed_key}): {reason}");
                notify_user(
                    notify::Level::Error,
                    &format!("{}\n{reason}", text(failed_key)),
                );
                Outcome::Failed
            }
            Deferred::Close => {
                close_scrap(hwnd);
                Outcome::Succeeded
            }
            Deferred::PickBackground(current) => pick_background(hwnd, current),
        })
        .collect();
    Some(Performed {
        outcome: Outcome::of(outcomes),
        saving,
    })
}

/// `Effect::ShowBackgroundDialog`: 色の選択ダイアログを出し、選ばれた色を `background` の選択肢として
/// 実行する。モーダルなので App を借りずに呼ぶ。取り消したら何もしない。
fn pick_background(hwnd: HWND, current: Rgb) -> Outcome {
    let Some(color) = crate::settings_dialog::fields::pick_color(hwnd, current) else {
        log::info!("background colour cancelled");
        return Outcome::Cancelled;
    };
    let Some(op) = ops::find("background") else {
        return Outcome::Failed;
    };
    let context = OpContext {
        choice: Some(color.to_rrggbb() as i32),
        ..OpContext::default()
    };
    perform(hwnd, context, |model, context| {
        Some((op.id, ops::execute(op, model, context)))
    })
    .map_or(Outcome::Failed, |performed| performed.outcome)
}

/// `Effect::CopyToClipboard`: 画像をクリップボードに置く。失敗したらエラーの通知を出し、失敗を返す。
fn copy_to_clipboard(image: &Image) -> Outcome {
    match copy_image(image) {
        Ok(()) => Outcome::Succeeded,
        Err(reason) => {
            notify_user(notify::Level::Error, &reason);
            Outcome::Failed
        }
    }
}

/// 画像をクリップボードに置く。失敗したら通知に出す理由を返す（通知はしない）。
fn copy_image(image: &Image) -> std::result::Result<(), String> {
    let Some(owner) = with_app(|app| app.windows.message) else {
        return Err(text("job.panicked").to_owned());
    };
    match clipboard::copy(owner, image) {
        Ok(()) => {
            log::info!(
                "copied {}x{} to the clipboard",
                image.width(),
                image.height()
            );
            Ok(())
        }
        Err(e) => {
            log::error!("could not copy to the clipboard: {e}");
            Err(format!("{}\n{e}", text("copy.failed")))
        }
    }
}

/// `Effect::SaveDefault`: 全体設定の保存先に、全体のファイル名テンプレートの名前で保存する。
/// 保存先とファイル名はここ（UI スレッド）で決め、PNG の書き出しと書き込みは裏のスレッドで行う。
/// 失敗してもスクラップはそのまま残る。
fn save_default(image: Image) -> Saving {
    match default_destination() {
        Some(path) => start_save(
            path,
            Payload::Png(image, save::Overwrite::Deny),
            None,
            None,
            true,
        ),
        None => Saving::Ended(Outcome::Failed),
    }
}

/// 保存を始めたか（`start_save`）。
enum Saving {
    /// 裏のスレッドに渡した。結果は `WM_APP_SAVED` で届く。
    Started(PathBuf),
    /// 始める前に終わった（失敗の通知は出してある。取り消しなら出さない）。
    Ended(Outcome),
}

/// `Effect::ShowSaveDialog`: 「名前を付けて保存」のダイアログを出し、選ばれた場所に保存する。初期フォルダと
/// ファイル名は既定の保存（`default_destination`）と同じ。キャンセルしたら何もしない。失敗は既定の保存と同じく通知する。
fn save_as(owner: HWND, image: Image) -> Saving {
    let Some(proposed) = default_destination() else {
        return Saving::Ended(Outcome::Failed);
    };
    // モーダルなので App を借りずに呼ぶ
    match save::ask_path(
        owner,
        &proposed,
        save::FileType::png(text("save.png_filter")),
    ) {
        Ok(Some(path)) => start_save(
            path,
            Payload::Png(image, save::Overwrite::Allow),
            None,
            None,
            true,
        ),
        Ok(None) => {
            log::info!("save as cancelled");
            Saving::Ended(Outcome::Cancelled)
        }
        Err(e) => {
            log::error!("could not show the save dialog: {e}");
            save_failed(None, &e.to_string());
            Saving::Ended(Outcome::Failed)
        }
    }
}

/// `Effect::ShowScrapFileSaveDialog`: スクラップファイルの保存先のダイアログを出し、選ばれた場所に書く。初期フォルダと
/// ファイル名は既定の保存と同じ（拡張子だけ替える）。キャンセルしたら何もしない。失敗は既定の保存と同じく通知する。
fn save_scrap_file(owner: HWND, file: Box<ScrapFile>) -> Saving {
    let Some(proposed) = default_destination() else {
        return Saving::Ended(Outcome::Failed);
    };
    let proposed = proposed.with_extension(scrap_file::EXTENSION);
    let file_type = save::FileType::scrap_file(text("scrap_file.filter"));
    // モーダルなので App を借りずに呼ぶ
    match save::ask_path(owner, &proposed, file_type) {
        Ok(Some(path)) => start_save(path, Payload::ScrapFile(file), None, None, true),
        Ok(None) => {
            log::info!("save as scrap file cancelled");
            Saving::Ended(Outcome::Cancelled)
        }
        Err(e) => {
            log::error!("could not show the save dialog: {e}");
            save_failed(None, &e.to_string());
            Saving::Ended(Outcome::Failed)
        }
    }
}

/// 既定の保存先: 全体設定の保存先に、全体のファイル名テンプレートで付けた重ならない名前。フォルダがなければ作る。
/// 決められなければ失敗を通知して None を返す。
fn default_destination() -> Option<PathBuf> {
    let template = with_app(|app| app.settings.general.file_name_template.clone())?;
    destination_in(None, &template, "")
}

/// `dir`（None なら全体設定の保存先）に、`template` で付けた重ならない名前。`name` はテンプレートの `{name}`。
/// フォルダがなければ作る（`write_png`）。決められなければ失敗を通知して None を返す。
fn destination_in(dir: Option<PathBuf>, template: &str, name: &str) -> Option<PathBuf> {
    match resolve_destination(dir, template, name) {
        Ok(path) => Some(path),
        Err((dir, reason)) => {
            save_failed(dir.as_deref(), &reason);
            None
        }
    }
}

/// 既定の保存先を決める。決められなければ、ログに書いて（あれば）フォルダと理由を返す。通知はしない。
fn resolve_default_destination() -> std::result::Result<PathBuf, (Option<PathBuf>, String)> {
    let Some(template) = with_app(|app| app.settings.general.file_name_template.clone()) else {
        return Err((None, text("job.panicked").to_owned()));
    };
    resolve_destination(None, &template, "")
}

/// `dir`（None なら全体設定の保存先）に、`template` で付けた重ならない名前を決める。決められなければ、ログに
/// 書いて（あれば）フォルダと理由を返す。通知はしない。
fn resolve_destination(
    dir: Option<PathBuf>,
    template: &str,
    name: &str,
) -> std::result::Result<PathBuf, (Option<PathBuf>, String)> {
    let Some((save_dir, pending)) = with_app(|app| {
        (
            dir.or_else(|| app.settings.general.save_dir.clone()),
            app.pending_saves.clone(),
        )
    }) else {
        return Err((None, text("job.panicked").to_owned()));
    };
    let dir = match save_dir {
        Some(dir) => dir,
        None => match save::default_dir() {
            Ok(dir) => dir,
            Err(e) => {
                log::error!("could not find the Pictures folder: {e}");
                return Err((None, e.to_string()));
            }
        },
    };
    match save::destination(&dir, template, name, save::now(), &pending) {
        Ok(path) => Ok(path),
        Err(e) => {
            log::error!("could not prepare the save folder {}: {e}", dir.display());
            Err((Some(dir), e.to_string()))
        }
    }
}

/// 裏のスレッドで書くもの。
enum Payload {
    /// 画像の PNG。
    Png(Image, save::Overwrite),
    /// スクラップファイル（ダイアログで上書きを確かめてあるので上書きする）。
    ScrapFile(Box<ScrapFile>),
}

impl Payload {
    fn write(&self, path: &Path) -> std::io::Result<()> {
        match self {
            Payload::Png(image, overwrite) => save::write_png(path, image, *overwrite),
            Payload::ScrapFile(file) => save::write_scrap_file(path, file),
        }
    }
}

/// `path` に `payload` を書く仕事を裏のスレッドで始める。結果は `WM_APP_SAVED` で受ける。失敗したら `fallback` を
/// スクラップとして浮かべる。`reply` があれば、結果を CLI にも返す。`notify` が false なら結果を通知しない。
fn start_save(
    path: PathBuf,
    payload: Payload,
    fallback: Option<Fallback>,
    reply: Option<ReplyTo>,
    notify: bool,
) -> Saving {
    let Some((sender, message_window)) = with_app(|app| (app.saves.0.clone(), app.windows.message))
    else {
        reply_failed(reply, text("job.panicked"));
        return Saving::Ended(Outcome::Failed);
    };
    // 書き終えるまでの間に次の保存が同じ名前を選ばないように、結果を受けるまで覚えておく
    let remembered = with_app(|app| {
        app.pending_saves.push(path.clone());
        if let Some(fallback) = fallback {
            app.save_fallbacks.push((path.clone(), fallback));
        }
    });
    if remembered.is_none() {
        log::warn!("could not remember the pending save {}", path.display());
    }
    // HWND は Send でないので、番号にして渡す
    let window = message_window.0 as isize;
    let job_path = path.clone();
    let spawned = std::thread::Builder::new()
        .name("save".into())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| payload.write(&job_path)))
                .map_err(|_| text("job.panicked").to_owned())
                .and_then(|written| written.map_err(|e| e.to_string()));
            let _ = sender.send(Saved {
                path: job_path,
                result,
                reply,
                notify,
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
        let fallback = forget_pending_save(&path);
        if notify {
            save_failed(Some(&path), &e.to_string());
        }
        if let Some(fallback) = fallback {
            let _ = float_scrap(fallback.at, fallback.image, false);
        }
        reply_failed(reply, &e.to_string());
        return Saving::Ended(Outcome::Failed);
    }
    Saving::Started(path)
}

/// 保存先を、書き込み中の保存先から外す。失敗したときに浮かべるスクラップがあれば返す。
fn forget_pending_save(path: &Path) -> Option<Fallback> {
    with_app(|app| {
        app.pending_saves.retain(|p| p != path);
        let i = app.save_fallbacks.iter().position(|(p, _)| p == path)?;
        Some(app.save_fallbacks.swap_remove(i).1)
    })
    .flatten()
}

/// `WM_APP_SAVED`: 保存の結果を通知する。成功なら、通知のクリックで保存先のフォルダを開けるようにする。
fn on_saved() {
    let Some(results) = with_app(|app| app.saves.1.try_iter().collect::<Vec<_>>()) else {
        return;
    };
    for saved in results {
        let fallback = forget_pending_save(&saved.path);
        // この保存を待っていたレシピ。通知を出してから続きを進める
        let recipe = with_app(|app| app.recipe.resume(&saved.path)).flatten();
        let outcome = if saved.result.is_ok() {
            Outcome::Succeeded
        } else {
            Outcome::Failed
        };
        match saved.result {
            Ok(()) => {
                log::info!("saved {}", saved.path.display());
                let body = format!("{}\n{}", text("save.saved"), saved.path.display());
                if saved.notify
                    && notify_user(notify::Level::Success, &body)
                    && let Some(folder) = saved.path.parent().map(Path::to_path_buf)
                {
                    with_app(|app| app.notices.opens(folder));
                }
                if let Some(to) = saved.reply {
                    // 保存先の設定が相対パスでも、CLI には絶対パスを返す
                    let path = std::path::absolute(&saved.path).unwrap_or(saved.path);
                    reply(
                        to,
                        Response::Captured {
                            path: Some(path.display().to_string()),
                        },
                    );
                }
            }
            Err(e) => {
                log::error!("could not save {}: {e}", saved.path.display());
                if !saved.notify {
                    // CLI の --quiet: 通知しない
                } else if saved.reply.is_some() {
                    // CLI の依頼はスクラップを出さないので、保存の失敗の通知の「スクラップは残っています」は使わない
                    notify_user(
                        notify::Level::Error,
                        &format!("{}\n{e}\n{}", text("capture.failed"), saved.path.display()),
                    );
                } else {
                    save_failed(Some(&saved.path), &e);
                }
                // CLI にはどこに書けなかったかも返す（--out の誤りに気づけるように）
                let path = std::path::absolute(&saved.path).unwrap_or(saved.path);
                reply_failed(saved.reply, &format!("{}: {e}", path.display()));
                if let Some(fallback) = fallback {
                    let _ = float_scrap(fallback.at, fallback.image, false);
                }
            }
        }
        if let Some((scrap, mut run)) = recipe {
            log::info!("recipe step {:?} -> {outcome:?}", run.waiting_for());
            run.settle(outcome);
            drive_recipe(HWND(scrap as *mut _), run);
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
        // 保存を待っているレシピは、スクラップと一緒に捨てる
        if app.recipe.discard_for(&(hwnd.0 as isize)).is_some() {
            log::info!("the recipe waiting for a save was discarded with its scrap");
        }
        let index = app.scraps.iter().position(|s| s.window.hwnd() == hwnd)?;
        let mut scrap = app.scraps.remove(index);
        // トリムの範囲を選んでいる途中で閉じたら選ぶ前の状態で、書き込みの編集モードなら抜けてから入れる
        scrap.model.settle();
        let thumbnail = trash_thumbnail(&scrap);
        let Scrap {
            toolbar,
            window,
            model,
            image,
            ..
        } = scrap;
        // 一時ファイルは裏のスレッドで書くので、借りたままでよい
        app.trash.push(model, image, thumbnail, save::now());
        Some((toolbar, window))
    })
    .flatten();
    // ウィンドウを壊すとメッセージが届くので、App を借りずに壊す（ツールバーが先）
    if let Some((toolbar, window)) = removed {
        drop(toolbar);
        drop(window);
        log::info!("scrap closed");
    }
}

/// ごみ箱の一覧に付けるサムネイル: 閉じたときの見た目（トリムと書き込みを反映したもの）を、閉じたモニタの
/// DPI で長い辺 64px 相当に縮める。書き込みを描けなければ、画像だけを縮める。
fn trash_thumbnail(scrap: &Scrap) -> Image {
    let side = shunsoku_core::trash::thumbnail_side(scrap.window.dpi());
    let layer = render::AnnotationLayer {
        annotations: scrap.model.annotations().items(),
        view: scrap.model.output_view(),
        selected: &[],
        selection_range: None,
        typing: None,
        scale: 1.0,
    };
    match render::render_image(scrap.shown(), layer) {
        Ok(image) => image.thumbnail(side),
        Err(e) => {
            log::warn!("could not draw the annotations for the trash thumbnail: {e}");
            scrap.shown().thumbnail(side)
        }
    }
}

/// ごみ箱の一覧の項目の文言: 閉じた時刻と画像の大きさ。
fn trash_label(listed: &trash_store::Listed) -> String {
    let t = listed.closed_at;
    let size = listed.model.shown_size();
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
            let toolbar = scrap.toolbar.as_ref().map(|t| &t.window);
            if visible {
                scrap.window.show(false);
                toolbar.inspect(|t| t.show());
            } else {
                toolbar.inspect(|t| t.hide());
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

/// Shift か Ctrl の `WM_KEYDOWN` / `WM_KEYUP` なら、押し下げの状態（ハイライトの切り替え）の入力にする。
fn selection_modifiers(wparam: WPARAM) -> Option<Input> {
    matches!(VIRTUAL_KEY(wparam.0 as u16), VK_SHIFT | VK_CONTROL).then(held_selection_modifiers)
}

/// いま押している Shift と Ctrl。`GetKeyState` は処理中のメッセージまでを反映する。
fn held_selection_modifiers() -> Input {
    Input::Modifiers {
        shift: key_held(VK_SHIFT),
        ctrl: key_held(VK_CONTROL),
    }
}

/// `key` を押しているか。`GetKeyState` は処理中のメッセージまでを反映する。
fn key_held(key: VIRTUAL_KEY) -> bool {
    // SAFETY: GetKeyState は引数のキーの状態を読むだけ
    unsafe { GetKeyState(i32::from(key.0)) < 0 }
}

/// 範囲選択の画面とトリムで共通の、`WM_KEYDOWN` の読み方。
struct KeyDown {
    key: VIRTUAL_KEY,
    /// 押しっぱなしの自動リピート（lParam の bit 30 = 直前も押されていた）。
    repeat: bool,
    shift: bool,
    ctrl: bool,
}

impl KeyDown {
    fn read(wparam: WPARAM, lparam: LPARAM) -> KeyDown {
        KeyDown {
            key: VIRTUAL_KEY(wparam.0 as u16),
            repeat: lparam.0 & (1 << 30) != 0,
            shift: key_held(VK_SHIFT),
            ctrl: key_held(VK_CONTROL),
        }
    }

    /// 矢印キーの向き。矢印キーでなければ None。Ctrl つきは大きさを変え、Shift つきは 10px ずつ動かす。
    fn arrow(&self) -> Option<Direction> {
        match self.key {
            VK_LEFT => Some(Direction::Left),
            VK_RIGHT => Some(Direction::Right),
            VK_UP => Some(Direction::Up),
            VK_DOWN => Some(Direction::Down),
            _ => None,
        }
    }
}

/// トリムの最中のキーを、トリムの入力にする。範囲選択の画面の精密モード（`selection_key`）と同じ割り当て。
fn trim_key(wparam: WPARAM, lparam: LPARAM) -> Option<TrimInput> {
    let key = KeyDown::read(wparam, lparam);
    if let Some(direction) = key.arrow() {
        let shift = key.shift;
        return Some(if key.ctrl {
            TrimInput::ResizeArrow { direction, shift }
        } else {
            TrimInput::Arrow { direction, shift }
        });
    }
    match key.key {
        VK_ESCAPE => Some(TrimInput::Cancel),
        VK_RETURN | VK_SPACE if !key.repeat => Some(TrimInput::Confirm),
        _ => None,
    }
}

/// 範囲選択の画面での `WM_KEYDOWN` を入力にする。対象でないキーは None。
fn selection_key(wparam: WPARAM, lparam: LPARAM) -> Option<Input> {
    if let Some(input) = selection_modifiers(wparam) {
        return Some(input);
    }
    let key = KeyDown::read(wparam, lparam);
    if let Some(direction) = key.arrow() {
        let shift = key.shift;
        return Some(if key.ctrl {
            Input::ResizeArrow { direction, shift }
        } else {
            Input::Arrow { direction, shift }
        });
    }
    let win = key_held(VK_LWIN) || key_held(VK_RWIN);
    match key.key {
        VK_ESCAPE => Some(Input::Escape),
        VK_TAB if !key.repeat => Some(Input::ToggleMode),
        VK_RETURN if !key.repeat => Some(Input::Enter),
        // 押しっぱなしで始点と終点が続けて決まらないよう、リピートは無視する
        VK_SPACE if !key.repeat => Some(Input::Space),
        // Ctrl+A と Ctrl+Shift+A だけ。Ctrl+Alt+A などほかの修飾キーとの組み合わせは対象にしない
        VK_A if key.ctrl && !key_held(VK_MENU) && !win && !key.repeat => Some(if key.shift {
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
        let before = selection.view();
        let state = selection.handle(input);
        if selection.view() != before {
            // 描くだけでウィンドウは作らないので、借りたまま描いてよい
            overlay.redraw(selection);
        }
        Some((state, selection.cursor()))
    })
    .flatten();
    let state = state.map(|(state, cursor)| {
        // 矢印で動かしたカーソルの位置に、実際のマウスのカーソルを合わせる。
        // 合わせたあとの WM_MOUSEMOVE は同じ位置なので、状態は変わらない
        // 調整の状態の矢印は範囲を動かすので、カーソルは動かさない
        if matches!(input, Input::Arrow { .. } | Input::ResizeArrow { .. })
            && matches!(state, State::Idle | State::Selecting)
        {
            // SAFETY: UI スレッドで呼ぶ。座標はスクリーン座標（物理ピクセル、PMv2）
            if let Err(e) = unsafe { SetCursorPos(cursor.x, cursor.y) } {
                log::warn!("could not move the cursor: {e}");
            }
        }
        state
    });
    match state {
        Some(State::Confirmed) => finish_selection(),
        Some(State::Cancelled) => close_overlay("cancelled"),
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
        } => Some((request, overlay, selection)),
        other => {
            app.capture = other;
            None
        }
    })
    .flatten();
    let Some((request, overlay, selection)) = taken else {
        return;
    };
    if selection.is_registering() {
        drop(overlay);
        end_capture();
        log::info!("overlay closed");
        match (request.method, selection.rect()) {
            (CaptureMethod::ReselectRegion(index), Some(rect)) => {
                settings_gui::region_reselected(index, rect)
            }
            (_, Some(rect)) => register_region(rect),
            _ => {}
        }
        return;
    }
    if request.method == CaptureMethod::BlankScrap {
        // 白紙スクラップは選んだ範囲（ウィンドウならその矩形）の位置と大きさだけを使い、画素は使わない
        let rect = selection
            .chosen_window()
            .map(|chosen| chosen.rect)
            .or_else(|| selection.rect());
        // スクラップを前面にしてから範囲選択の画面を閉じる（前面にする権利を失わないように）
        if let Some(rect) = rect {
            create_blank_scrap(rect);
        }
        drop(overlay);
        end_capture();
        log::info!("overlay closed");
        return;
    }
    if let Some(chosen) = selection.chosen_window() {
        // ハイライトから選んだウィンドウは、範囲選択の画面を閉じてから WGC で撮る
        log::info!("window chosen on the overlay");
        let request = request.for_window(chosen);
        drop(overlay);
        log::info!("overlay closed");
        with_app(|app| app.capture = Capture::Waiting(request));
        take_capture();
        return;
    }
    let Some(rect) = selection.rect() else {
        drop(overlay);
        end_capture();
        reply_cancelled(request.reply);
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

    // スクラップを前面にしてから範囲選択の画面を閉じる（前面にする権利を失わないように）
    output(request, rect, image);
    drop(overlay);
    end_capture();
    log::info!("overlay closed");
}

/// トリムしていれば、元の画像から切り出した表示する画像（`Scrap::shown`）。
fn shown_image(model: &ScrapModel, image: &Image) -> Option<Image> {
    model.is_trimmed().then(|| model.shown_image(image))
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
    // 文字を打つ間のほかは IME を使わない（IME がオンでもショートカットのキーが届くように）
    ime::disable(window.hwnd());
    // 開いたモニタの DPI を覚える。ごみ箱から戻したものは、閉じたときと DPI が違えば設定に従って大きさを変える
    let mut model = model;
    let scaling = with_app(|app| app.settings.general.dpi_scaling).unwrap_or_default();
    model.set_dpi(window.dpi(), scaling, ZoomAnchor::Center);
    // 描いて出すときに届くメッセージ（WM_WINDOWPOSCHANGED など）では App を借りないので、借りたまま描いてよい。
    // 借りられなければ（起こらないはず）、その場かぎりの描き手で描く
    let shown = shown_image(&model, &image);
    let visible = shown.as_ref().unwrap_or(&image);
    let painted = with_app(|app| {
        paint_scrap(
            &mut app.renderer,
            &window,
            visible,
            &model,
            appearance,
            None,
            shunsoku_core::settings::DEFAULT_CROSSHAIR_COLOR,
        )
    })
    .unwrap_or_else(|| {
        paint_scrap(
            &mut None,
            &window,
            visible,
            &model,
            appearance,
            None,
            shunsoku_core::settings::DEFAULT_CROSSHAIR_COLOR,
        )
    });
    painted?;
    window.show(focus);
    Ok(Scrap {
        toolbar: None,
        window,
        model,
        image,
        shown,
        drag: Drag::default(),
        wheel: Wheel::default(),
        zoom_label: false,
        caret: Default::default(),
        crosshair: shunsoku_core::settings::DEFAULT_CROSSHAIR_COLOR,
    })
}

/// 発生源（トレイ・ホットキー）からキャプチャの依頼を受ける。
fn request_capture(source: Source) {
    log::info!("capture requested from {source:?}");
    submit(CaptureRequest::from_source(source));
}

/// キャプチャの依頼を受ける。ほかの依頼を進めている間（範囲選択の画面を開いている間など）は、ホットキー・
/// トレイの依頼は無視し、CLI の依頼は失敗として返す。
///
/// CLI の依頼は `App::pipe` の受け口で順に待たせ、進めている依頼がないときに 1 件ずつ取り出す
/// （`on_pipe_requests`）。範囲選択の画面を開くまでと `capture rect` は 1 つのメッセージの処理の中で終わるので、
/// 依頼が重なるのは範囲選択の画面を開いている間と `delay` を待っている間だけ。
fn submit(request: CaptureRequest) {
    let delay = request.delay;
    let reply = request.reply;
    let accepted = with_app(|app| {
        if !matches!(app.capture, Capture::Idle) {
            return None;
        }
        app.capture = Capture::Waiting(request);
        Some(app.windows.message)
    })
    .flatten();
    let Some(message_window) = accepted else {
        log::info!("capture requested while another one is in progress; rejected");
        reply_failed(reply, text("cli.selecting"));
        return;
    };
    if delay.is_zero() {
        take_capture();
        return;
    }
    let ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: 自分のウィンドウのタイマー
    if unsafe { SetTimer(Some(message_window), TIMER_CAPTURE_DELAY, ms, None) } == 0 {
        log::warn!("could not start the capture delay timer; capturing now");
        take_capture();
    }
}

/// 待っている依頼を、撮り方に合わせて進める。shunsoku のメニューが出ていれば閉じ、消えきってから撮る
/// （写り込まないように）。
fn take_capture() {
    // SAFETY: このスレッドのメニューを閉じるだけ（出ていなければ何もしない）
    unsafe {
        let _ = EndMenu();
    }
    let next = with_app(|app| {
        let Capture::Waiting(request) = &app.capture else {
            return None;
        };
        let method = request.method.clone();
        Some((wait_for_menus(app), method, app.windows.message))
    })
    .flatten();
    let method = match next {
        None => None,
        Some((MenuWait::Clear { waited }, method, _)) => {
            if waited {
                flush_composition();
            }
            Some(method)
        }
        Some((MenuWait::Wait, method, message_window)) => {
            // SAFETY: 自分のウィンドウのタイマー
            if unsafe {
                SetTimer(
                    Some(message_window),
                    TIMER_CAPTURE_DELAY,
                    MENU_RETRY_MS,
                    None,
                )
            } != 0
            {
                return;
            }
            log::warn!("could not start the menu retry timer; capturing now");
            Some(method)
        }
    };
    match method {
        Some(
            request::CaptureMethod::Select
            | request::CaptureMethod::RegisterRegion
            | request::CaptureMethod::ReselectRegion(_)
            | request::CaptureMethod::BlankScrap,
        ) => open_overlay(),
        Some(request::CaptureMethod::ActiveWindow) => capture_active_window(),
        Some(request::CaptureMethod::Window(target)) => {
            capture_window(window_list::hwnd(target.window), target.crop);
        }
        Some(request::CaptureMethod::Rect { monitor, rect }) => capture_rect(monitor, rect),
        Some(request::CaptureMethod::Region(_) | request::CaptureMethod::TrialRegion(_)) => {
            // 定型領域はふだん `request_region` の待ち行列から撮る。ここに来たら同じ流れで撮る
            let request = with_app(|app| match std::mem::take(&mut app.capture) {
                Capture::Waiting(request) => Some(request),
                other => {
                    app.capture = other;
                    None
                }
            })
            .flatten();
            end_capture();
            if let Some(request) = request {
                capture_region(request);
            }
        }
        None => {}
    }
}

/// 撮った画像（`area` はスクリーン座標の位置）を、依頼の出力（コピー → スクラップ → 保存の順）に出す。
/// 撮り方（範囲選択・ウィンドウ・定型領域・`capture rect`）にかかわらず、ここで出す。
///
/// - 失敗として返す依頼（`OnSaveFailure::Fail`、CLI）は、どれかに失敗したらその先は出さず、失敗を通知して
///   CLI に返す。保存に失敗してもスクラップには逃がさない。保存の結果は `on_saved` が返す。
/// - そうでない依頼（ホットキー・トレイ）は、失敗を通知して残りを出す。保存に失敗したら、スクラップを
///   出していなければスクラップに逃がす。
fn output(request: CaptureRequest, area: Rect, image: Image) {
    let strict = request.on_save_failure == OnSaveFailure::Fail;
    let CaptureRequest {
        outputs,
        notify,
        reply: reply_to,
        ..
    } = request;
    // 失敗として返す依頼の失敗
    let fail = |reason: &str| {
        if notify {
            notify_user(
                notify::Level::Error,
                &format!("{}\n{reason}", text("capture.failed")),
            );
        }
        reply_failed(reply_to, reason);
    };
    if outputs.copy
        && let Err(reason) = copy_image(&image)
    {
        if strict {
            fail(&reason);
            return;
        }
        if notify {
            notify_user(notify::Level::Error, &reason);
        }
    }
    if outputs.scrap {
        match float_scrap(area.origin(), image.clone(), outputs.focus_scrap) {
            Ok(()) => perf::mark(Mark::ScrapShown),
            Err(reason) if strict => {
                fail(&reason);
                return;
            }
            Err(_) => {}
        }
    }
    let Some(target) = outputs.save else {
        if let Some(to) = reply_to {
            reply(to, Response::Captured { path: None });
        }
        return;
    };
    // 決まったファイル（CLI の --out）だけ上書きする
    let overwrite = match target {
        SaveTarget::File(_) => save::Overwrite::Allow,
        _ => save::Overwrite::Deny,
    };
    let fallback = (!strict && !outputs.scrap).then(|| Fallback {
        at: area.origin(),
        image: image.clone(),
    });
    match resolve_save_target(target) {
        Ok(path) => {
            start_save(
                path,
                Payload::Png(image, overwrite),
                fallback,
                reply_to,
                notify,
            );
        }
        Err((dir, reason)) if strict => {
            let mut reason = reason;
            if let Some(dir) = dir {
                reason.push('\n');
                reason.push_str(&dir.display().to_string());
            }
            fail(&reason);
        }
        Err((dir, reason)) => {
            if notify {
                save_failed(dir.as_deref(), &reason);
            }
            if let Some(fallback) = fallback {
                let _ = float_scrap(fallback.at, fallback.image, false);
            }
        }
    }
}

/// 待っている依頼で、いまアクティブなウィンドウを WGC で撮り、依頼の出力（スクラップ）を出す。
///
/// 前面がデスクトップやタスクバー、または前面のウィンドウがないときは撮らない（#63）。
/// 通知は出さず、CLI の依頼には撮れるアクティブウィンドウがないことを返す。
fn capture_active_window() {
    // 撮る前にアクティブなウィンドウを決める（スクラップを出すとフォーカスが移る）
    // SAFETY: 引数なし
    let hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let class = (!hwnd.is_invalid()).then(|| crate::window_list::class_name(hwnd));
    if shunsoku_core::capture::window::is_active_window_to_take(class.as_deref()) {
        capture_window(hwnd, None);
        return;
    }
    log::info!(
        "no active window to capture (foreground class: {})",
        class.as_deref().unwrap_or("none")
    );
    let waiting = with_app(|app| match &app.capture {
        Capture::Waiting(request) => Some(request.reply),
        _ => None,
    })
    .flatten();
    if let Some(reply_to) = waiting {
        end_capture();
        reply_failed(reply_to, text("cli.no_active_window"));
    }
}

/// 待っている依頼で `hwnd` のウィンドウを WGC で撮り、依頼の出力（スクラップ）を出す。
/// `crop`（スクリーン座標）があれば、撮った画像からその部分（部品）を切り出す。
fn capture_window(hwnd: HWND, crop: Option<Rect>) {
    let started = Instant::now();
    let taken = with_app(|app| match &app.capture {
        Capture::Waiting(request) => Some((
            request.clone(),
            app.gpu.primary_device(),
            app.settings.general.capture_cursor,
        )),
        _ => None,
    })
    .flatten();
    let Some((request, device, cursor)) = taken else {
        return;
    };
    let grabbed = if hwnd.is_invalid() {
        Err(text("cli.no_active_window").to_owned())
    } else {
        device.map_err(|e| e.to_string()).and_then(|device| {
            crate::window_grab::grab_window(&device, hwnd, cursor).map_err(|e| e.to_string())
        })
    };
    let (shot, partial) = match grabbed {
        Ok(shot) => (shot, false),
        Err(e) => {
            log::warn!("could not capture the window: {e}");
            // UAC で昇格したウィンドウなど、WGC で撮れないときは見えている部分だけを撮る（#64）
            match (!hwnd.is_invalid())
                .then(|| grab_visible_part(hwnd, cursor))
                .flatten()
            {
                Some(shot) => (shot, true),
                None => {
                    end_capture();
                    if request.notify {
                        notify_user(notify::Level::Error, text("capture.failed"));
                    }
                    reply_failed(request.reply, &e);
                    return;
                }
            }
        }
    };
    if partial && request.notify {
        notify_user(notify::Level::Warning, text("capture.window_partial"));
    }
    // 部品は、撮ったウィンドウの画像から切り出す（WGC は最上位のウィンドウしか撮れない）
    let shot = match crop.map(|crop| crop.intersect(&shot.rect)) {
        None => shot,
        Some(Some(area)) => crate::window_grab::WindowShot {
            rect: area,
            image: shot
                .image
                .crop(area.offset(-shot.rect.left, -shot.rect.top)),
        },
        Some(None) => {
            log::warn!("the chosen part is outside the captured window");
            shot
        }
    };
    log::info!(
        "captured window {}x{} at ({}, {}) in {:.1} ms",
        shot.rect.width(),
        shot.rect.height(),
        shot.rect.left,
        shot.rect.top,
        started.elapsed().as_secs_f64() * 1000.0
    );
    end_capture();
    output(request, shot.rect, shot.image);
}

/// WGC で撮れなかったウィンドウの、モニタに写っている部分を Desktop Duplication で撮る。撮れなければ None。
fn grab_visible_part(hwnd: HWND, cursor: bool) -> Option<crate::window_grab::WindowShot> {
    // 取得のスレッドは App に触らないので、借りたまま待ってよい
    let grabbed = with_app(|app| crate::window_grab::grab_visible(&mut app.gpu, hwnd, cursor))?;
    let shot = match grabbed {
        Ok(Some(shot)) => Some(shot),
        Ok(None) => {
            log::warn!("the active window is on no monitor");
            None
        }
        Err(e) => {
            log::warn!("could not grab the screen for the active window: {e}");
            None
        }
    };
    // 範囲選択の画面と同じく、取得で使った一時的なメモリを返す
    with_app(|app| app.gpu.trim());
    shot
}

/// 待っている依頼の画面を取得して、範囲選択の画面を出す。
fn open_overlay() {
    let started = Instant::now();
    // 取得のスレッドは App に触らないので、借りたまま待ってよい
    let grabbed = with_app(|app| {
        matches!(app.capture, Capture::Waiting(_)).then(|| {
            let cursor = app.settings.general.capture_cursor;
            (app.gpu.grab(cursor), app.module)
        })
    })
    .flatten();
    let Some((grabbed, module)) = grabbed else {
        return;
    };
    let shots = match grabbed {
        Ok(shots) => shots,
        Err(e) => {
            log::warn!("could not grab the screen: {e}");
            let reply_to = waiting_reply();
            end_capture();
            reply_failed(reply_to, &e.to_string());
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
    let overlay = match overlay::Overlay::open(shots, module, text("selection.size_plus")) {
        Ok(overlay) => {
            // open は全モニタで DwmFlush が返ってから戻る
            perf::mark(Mark::OverlayFlushed);
            overlay
        }
        Err(e) => {
            log::error!("could not open the overlay: {e}");
            let reply_to = waiting_reply();
            end_capture();
            reply_failed(reply_to, &e.to_string());
            return;
        }
    };
    let mut selection = overlay.new_selection();
    // ハイライトの対象。画面を出してから集める（出すまでの時間を延ばさないため）。
    // 画面に覆われたあとも、下のウィンドウの位置は開いた時点のまま
    let windows_started = Instant::now();
    selection.set_windows(window_list::collect());
    let windows_ms = windows_started.elapsed().as_secs_f64() * 1000.0;
    // 開く前から Shift を押していれば、すぐにハイライトする
    selection.handle(held_selection_modifiers());
    let rejected = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Waiting(request) => {
            // 開くたびに設定の既定のモードから始める（Tab の切り替えはその回だけ）
            let mut selection = selection
                .with_mode(app.settings.selection.mode)
                .with_crosshair(app.settings.selection.crosshair);
            let mut overlay = overlay;
            overlay.set_crosshair_color(app.settings.selection.crosshair_color);
            if request.method.registers_region() {
                // 定型領域の登録は常に精密モードで、画面の上端に登録だと出す
                selection = selection.registering();
                overlay.set_banner(Some(text("selection.register_banner")));
            }
            let view = selection.view();
            if view.crosshair.is_some() || view.highlight.is_some() {
                // 描くだけでウィンドウは作らないので、借りたまま描いてよい
                overlay.redraw(&selection);
            }
            app.capture = Capture::Selecting {
                request,
                overlay,
                selection: Box::new(selection),
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
        "overlay opened on {monitors} monitor(s): grab {grab_ms:.1} ms, windows {windows_ms:.1} ms, total {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// 範囲選択の画面を閉じ、依頼を終える。CLI の依頼なら、取り消し（終了コード 4）として返す。
/// 開いていなければ何もしない。`why` は閉じるわけ（ログに出す）。
fn close_overlay(why: &str) {
    let taken = with_app(|app| match std::mem::take(&mut app.capture) {
        Capture::Selecting {
            overlay, request, ..
        } => Some((overlay, request.reply)),
        other => {
            app.capture = other;
            None
        }
    })
    .flatten();
    let Some((overlay, reply_to)) = taken else {
        return;
    };
    // ウィンドウを壊すとメッセージが届くので、App を借りずに閉じる
    drop(overlay);
    end_capture();
    reply_cancelled(reply_to);
    log::info!("overlay closed: {why}");
}

/// 依頼を終えて、取得に使った GPU のメモリを返す。待たせている CLI の依頼・定型領域キャプチャがあれば進める。
fn end_capture() {
    let (message_window, regions_waiting) = with_app(|app| {
        app.capture = Capture::Idle;
        app.gpu.trim();
        (app.windows.message, !app.regions_waiting.is_empty())
    })
    .unzip();
    if let Some(window) = message_window {
        // SAFETY: 自分のウィンドウに送るだけ
        let _ = unsafe { PostMessageW(Some(window), WM_APP_PIPE, WPARAM(0), LPARAM(0)) };
        // 待っている定型領域キャプチャは、この処理を抜けてから進める
        if regions_waiting == Some(true) {
            post_region_message(window);
        }
    }
    // 「範囲を選び直す」で隠した設定 GUI を出し直す
    settings_gui::capture_done();
}

/// `WM_HOTKEY`。範囲選択の画面を開いている間に押されたものは `request_capture`・`request_region` が無視する。
fn on_hotkey(wparam: WPARAM) {
    perf::mark(Mark::HotkeyReceived);
    let action = with_app(|app| app.hotkeys.action(wparam)).flatten();
    match action {
        Some(settings::HotkeyAction::SelectCapture) => request_capture(Source::Hotkey),
        Some(settings::HotkeyAction::RestoreLast) => restore_last_scrap(),
        Some(settings::HotkeyAction::Region(i)) => request_region(i, "hotkey"),
        Some(settings::HotkeyAction::ActiveWindowCapture) => {
            request_capture(Source::ActiveWindowHotkey)
        }
        Some(settings::HotkeyAction::BlankScrap) => request_capture(Source::BlankScrap),
        None => log::warn!("unknown hotkey id {}", wparam.0),
    }
}

/// 定型領域キャプチャの依頼を受ける（`index` は `Settings::regions` の添字）。範囲選択の画面を開いている間は
/// 無視する。ほかの依頼を進めている間や、続けて押されたときは、届いた順に待たせて 1 件ずつ撮る。
///
/// shunsoku のメニュー（トレイ・スクラップの右クリック）が出ていれば閉じ、消えきってから撮る（`run_waiting_regions`）。
fn request_region(index: usize, from: &str) {
    let from_menu = from == "tray";
    let queued = with_app(|app| {
        if matches!(app.capture, Capture::Selecting { .. }) {
            return Err("the overlay is open");
        }
        let region = app
            .settings
            .regions
            .get(index)
            .ok_or("the region is no longer in the settings")?;
        app.regions_waiting
            .push_back(CaptureRequest::region(region));
        Ok(app.windows.message)
    });
    match queued {
        Some(Ok(message_window)) => {
            log::info!("region capture {} requested from {from}", index + 1);
            if from_menu {
                chosen_from_menu();
            }
            // SAFETY: このスレッドのメニューを閉じるだけ（出ていなければ何もしない）
            unsafe {
                let _ = EndMenu();
            }
            post_region_message(message_window);
        }
        Some(Err(reason)) => {
            log::info!(
                "region capture {} requested from {from}: {reason}; ignored",
                index + 1
            )
        }
        None => log::warn!(
            "region capture {} requested from {from} while busy; ignored",
            index + 1
        ),
    }
}

/// 設定 GUI の「範囲を選び直す」: 範囲選択の画面を登録のモードで開き、決めた範囲を写しの定型領域 `index` に
/// 入れる（`settings_gui::region_reselected`）。ほかの依頼を進めていれば受けずに false。
fn request_reselect(index: usize) -> bool {
    let idle = with_app(|app| matches!(app.capture, Capture::Idle)).unwrap_or(false);
    if idle {
        log::info!(
            "reselecting region {} requested from the settings GUI",
            index + 1
        );
        submit(CaptureRequest::reselect_region(index));
    }
    idle
}

/// 設定 GUI の「試しに撮る」: 写しの定型領域 `region` の値で撮る（定型領域キャプチャと同じ待ち行列で、この処理を
/// 抜けてから撮る）。領域に保存先がなければ `general_dir` に保存する。ほかの依頼を進めていれば受けずに false。
fn request_trial(region: &settings::Region, general_dir: Option<PathBuf>) -> bool {
    let queued = with_app(|app| {
        if !matches!(app.capture, Capture::Idle) {
            return None;
        }
        app.regions_waiting
            .push_back(CaptureRequest::trial_region(region, general_dir));
        Some(app.windows.message)
    })
    .flatten();
    let Some(message_window) = queued else {
        return false;
    };
    log::info!("trial capture of region {} requested", region.name);
    post_region_message(message_window);
    true
}

/// 待っている定型領域キャプチャを、いまの処理を抜けてから進めるよう `WM_APP_REGION` を送る。
fn post_region_message(message_window: HWND) {
    // SAFETY: 自分のメッセージ専用ウィンドウに送るだけ
    if let Err(e) =
        unsafe { PostMessageW(Some(message_window), WM_APP_REGION, WPARAM(0), LPARAM(0)) }
    {
        log::error!("could not post the region capture message: {e}");
    }
}

/// `WM_APP_REGION`: 待っている定型領域キャプチャを、届いた順に撮る。ほかの依頼を進めている間は待たせたまま
/// にする（`end_capture` が送り直す）。shunsoku のメニューがまだ見えていれば（閉じる途中のフェードも）、
/// 写り込まないよう少し待って撮り直す。
fn run_waiting_regions() {
    loop {
        let next = with_app(|app| {
            if !matches!(app.capture, Capture::Idle) || app.regions_waiting.is_empty() {
                return Ok(None);
            }
            match wait_for_menus(app) {
                MenuWait::Wait => Err(app.windows.message),
                MenuWait::Clear { waited } => {
                    Ok(app.regions_waiting.pop_front().map(|r| (r, waited)))
                }
            }
        });
        match next {
            Some(Ok(Some((request, waited)))) => {
                if waited {
                    flush_composition();
                }
                capture_region(request);
                // 「試しに撮る」で隠した設定 GUI を出し直す
                settings_gui::capture_done();
            }
            Some(Err(message_window)) => {
                // SAFETY: 自分のウィンドウのタイマー
                if unsafe {
                    SetTimer(
                        Some(message_window),
                        TIMER_REGION_RETRY,
                        MENU_RETRY_MS,
                        None,
                    )
                } == 0
                {
                    log::warn!("could not start the region retry timer; capturing now");
                    with_app(|app| app.menu_retries = MAX_MENU_RETRIES);
                    continue;
                }
                return;
            }
            _ => return,
        }
    }
}

/// shunsoku のメニューを撮る前に待つか。
enum MenuWait {
    /// まだ見えている（閉じる途中のフェードも）。少し待って確かめ直す。
    Wait,
    /// 見えていない。`waited` なら待った後（消えた画面がまだ画面に反映されていないかもしれない）。
    Clear { waited: bool },
}

/// shunsoku のメニューが写り込まないよう、撮るのを待つか決める。待つなら回数を数え、上限を超えたらそのまま撮る。
fn wait_for_menus(app: &mut App) -> MenuWait {
    let fading = app.menu_not_before.take();
    if let Some(not_before) = fading
        && Instant::now() < not_before
    {
        app.menu_not_before = Some(not_before);
        return MenuWait::Wait;
    }
    if app.menu_retries < MAX_MENU_RETRIES && menus_visible() {
        app.menu_retries += 1;
        return MenuWait::Wait;
    }
    let waited = fading.is_some() || app.menu_retries > 0;
    app.menu_retries = 0;
    MenuWait::Clear { waited }
}

/// メニューで項目を選んだ。選んだ項目のフェードが消えるまでは撮らない（フェードのウィンドウは列挙で
/// 見つからないことがあるので、時間で待つ）。
fn chosen_from_menu() {
    if selection_fades() {
        with_app(|app| {
            app.menu_not_before =
                Some(Instant::now() + std::time::Duration::from_millis(MENU_FADE_MS));
        });
    }
}

/// 隠したウィンドウが画面に反映されるのを待つ（DWM の次の合成まで）。
fn flush_composition() {
    // SAFETY: 引数なし
    if let Err(e) = unsafe { DwmFlush() } {
        log::warn!("DwmFlush failed: {e}");
    }
}

/// メニューで選んだ項目を消える前にフェードさせる設定（SPI_GETSELECTIONFADE）が有効か。
fn selection_fades() -> bool {
    let mut fades = windows::core::BOOL(0);
    // SAFETY: BOOL を 1 つ受け取るだけ
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETSELECTIONFADE,
            0,
            Some(&mut fades as *mut windows::core::BOOL as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && fades.as_bool()
}

/// このスレッドのメニュー（`#32768`）か、その影・閉じるときのフェードのウィンドウが見えているか。
fn menus_visible() -> bool {
    unsafe extern "system" fn visit(hwnd: HWND, found: LPARAM) -> windows::core::BOOL {
        let mut class = [0u16; 32];
        // SAFETY: バッファの長さを渡して読む
        let len = unsafe { GetClassNameW(hwnd, &mut class) };
        let class = String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)]);
        // SAFETY: ウィンドウの状態を読むだけ
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        if visible && matches!(class.as_str(), "#32768" | "SysShadow" | "SysFader") {
            // SAFETY: found は menus_visible の bool
            unsafe { *(found.0 as *mut bool) = true };
            return false.into();
        }
        true.into()
    }
    let mut found = false;
    // SAFETY: コールバックは列挙の間だけ found を借りる
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(visit),
            LPARAM(&mut found as *mut bool as isize),
        );
    }
    found
}

/// 定型領域キャプチャを 1 件撮り、依頼の出力（保存・スクラップ）を出す。撮れなければ失敗を通知する。
fn capture_region(request: CaptureRequest) {
    let Some(name) = request.region_name().map(str::to_owned) else {
        return;
    };
    let name = name.as_str();
    // 試しに撮る依頼（設定 GUI）は写しの値で、ほかは適用済みの設定から名前で引く
    let region = with_app(|app| request.region_to_capture(&app.settings.regions)).flatten();
    let Some(region) = region else {
        log::warn!("region {name} is not in the settings");
        let message = format!("{}{name}", text("region.gone"));
        if request.notify {
            notify_user(notify::Level::Error, &message);
        }
        reply_failed(request.reply, &message);
        return;
    };
    // CLI の依頼は、領域の保存先とファイル名テンプレートで保存する（領域の撮影後の動作は見ない）
    let request = request.clone().with_region_destination(&region);
    let monitors = monitor_list::list();
    let located = match monitors::locate_by_id(&monitors, &region.monitor, region.rect) {
        Ok(located) => located,
        Err(e) => {
            log::warn!(
                "could not capture region {name} (monitor {}): {e:?}; monitors: {:?}",
                region.monitor,
                monitors.iter().map(|m| &m.id).collect::<Vec<_>>()
            );
            let key = match e {
                LocateError::OutOfBounds => "region.out_of_bounds",
                LocateError::MonitorNotFound
                | LocateError::NoSuchMonitor(_)
                | LocateError::NoMainMonitor => "region.monitor_not_found",
            };
            let message = format!("{}{name}", text(key));
            if request.notify {
                notify_user(notify::Level::Error, &message);
            }
            reply_failed(request.reply, &message);
            return;
        }
    };
    let monitor_rect = monitors[located.index].rect;
    let screen = located.screen;
    // 直前に閉じたウィンドウ（メニューなど）が消えた画面を撮るよう、合成を 1 回待つ
    // SAFETY: 引数なしの呼び出し
    unsafe {
        let _ = DwmFlush();
    }
    // 取得のスレッドは App に触らないので、借りたまま待ってよい
    let captured = with_app(|app| {
        let cursor = app.settings.general.capture_cursor;
        let result = app
            .gpu
            .grab_monitor(monitor_rect, cursor)
            .map_err(|e| e.to_string())
            .and_then(|shot| match shot.read_back(screen) {
                Ok(Some(fragment)) => Ok(fragment),
                Ok(None) => {
                    log::warn!("the monitor of region {name} cannot be read back");
                    Err(text("cli.unreadable_monitor").to_owned())
                }
                Err(e) => Err(e.to_string()),
            });
        app.gpu.trim();
        result
    });
    let fragment = match captured {
        Some(Ok(fragment)) => fragment,
        Some(Err(e)) => {
            log::error!("could not capture region {name}: {e}");
            if request.notify {
                notify_user(
                    notify::Level::Error,
                    &format!("{}\n{name}\n{e}", text("capture.failed")),
                );
            }
            reply_failed(request.reply, &e);
            return;
        }
        None => {
            reply_failed(request.reply, text("job.panicked"));
            return;
        }
    };
    let (at, image) = (fragment.at, fragment.image);
    log::info!(
        "captured region {name}: {}x{} at ({}, {})",
        image.width(),
        image.height(),
        at.x,
        at.y
    );

    output(request, Rect::from_origin_size(at, image.size()), image);
}

/// 範囲選択の画面で決めた範囲（スクリーン座標）を、定型領域として設定ファイルの `[[regions]]` に足す。
/// 名前は重ならない「領域 N」（設定 GUI を開いていれば、その写しの定型領域とも重ならない）、モニタは範囲の左上を
/// 含むモニタ。足せたら、トレイの「定型領域」サブメニュー（開くたびに `App::settings` から作る）に出て、すぐに
/// 撮れる。結果は通知で知らせ、設定 GUI の「定型領域」タブで足した領域の名前の欄にフォーカスを置く。
fn register_region(rect: Rect) {
    let monitors = monitor_list::list();
    let Some((index, relative)) = monitors::relative_to(&monitors, rect) else {
        log::warn!("no monitor contains the region to register at {rect:?}");
        notify_user(notify::Level::Error, text("region.register_failed"));
        return;
    };
    let monitor = &monitors[index];
    let result = with_app(|app| {
        let region = settings_gui::region_to_register(
            app,
            text("region.default_name"),
            &monitor.id,
            relative,
        );
        let mut new = app.settings.clone();
        new.regions.push(region.clone());
        write_settings(app, &new).map(|()| region)
    });
    match result {
        Some(Ok(region)) => {
            log::info!(
                "registered region {} on monitor {} at {:?}",
                region.name,
                region.monitor,
                region.rect
            );
            // 通知は出さない（設定 GUI でその領域を開くので、登録したことはそこでわかる）
            // 書いた定型領域はファイルの要素に結び付いている（`Settings::as_written`）
            let name = region.name.clone();
            settings_gui::region_registered(settings::Region {
                loaded_name: Some(name),
                ..region
            });
        }
        Some(Err(e)) => {
            log::error!("could not register the region: {e:?}");
            notify_user(
                notify::Level::Error,
                &e.notice(i18n::catalog(language()), "region.register_failed"),
            );
        }
        None => {}
    }
}

/// 保存先を決める: `target` の保存先に、`target` のファイル名テンプレートで付けた重ならない名前（決まった
/// ファイルならそのまま）。決められなければ、（あれば）フォルダと理由を返す。通知はしない。
fn resolve_save_target(
    target: SaveTarget,
) -> std::result::Result<PathBuf, (Option<PathBuf>, String)> {
    match target {
        SaveTarget::Default => resolve_default_destination(),
        SaveTarget::File(path) => Ok(path),
        SaveTarget::Region {
            dir,
            file_name_template,
            name,
        } => resolve_destination(dir, &file_name_template, &name),
    }
}

/// 画像をスクラップとして `at`（スクリーン座標）に等倍で浮かべ、`App::scraps` に入れる。出せなければ理由を返す。
fn float_scrap(at: Point, image: Image, focus: bool) -> std::result::Result<(), String> {
    let model = ScrapModel::new(at, image.size());
    float_model(model, image, focus).map(|_| ())
}

/// `model` のスクラップ（元の画像は `image`）を浮かべ、`App::scraps` に入れる。出したウィンドウを返す。
/// 出せなければ理由を返す。
fn float_model(model: ScrapModel, image: Image, focus: bool) -> std::result::Result<HWND, String> {
    let Some((module, appearance)) =
        with_app(|app| (app.module, Appearance::from(app.settings.scrap)))
    else {
        return Err(text("job.panicked").to_owned());
    };
    // ウィンドウを作るとメッセージが届くので、App を借りずに作る
    match open_scrap(module, model, image, &appearance, focus) {
        Ok(scrap) => {
            let hwnd = scrap.window.hwnd();
            if with_app(|app| app.scraps.push(scrap)).is_none() {
                log::warn!("could not keep the scrap");
            }
            Ok(hwnd)
        }
        Err(e) => {
            log::error!("could not open a scrap: {e}");
            Err(e.to_string())
        }
    }
}

/// `rect`（スクリーン座標）に、設定の既定の背景色の白紙スクラップを浮かべてフォーカスを移し、すぐに書き込みの
/// 編集モードに入る。出せなければ通知する。
fn create_blank_scrap(rect: Rect) {
    let Some(background) = with_app(|app| app.settings.annotation.background) else {
        return;
    };
    let model = ScrapModel::blank(rect.origin(), rect.size(), background);
    let Some(image) = model.blank_image() else {
        log::error!("could not paint the blank scrap of {:?}", rect.size());
        return;
    };
    log::info!(
        "blank scrap {}x{} at ({}, {})",
        rect.width(),
        rect.height(),
        rect.left,
        rect.top
    );
    match float_model(model, image, true) {
        Ok(hwnd) => {
            if let Some(op) = ops::find("annotate") {
                run_scrap_operation(hwnd, op, OpContext::default());
            }
        }
        Err(reason) => {
            notify_user(
                notify::Level::Error,
                &format!("{}\n{reason}", text("blank_scrap.failed")),
            );
        }
    }
}

/// トレイの「開く…」: スクラップファイルを選ばせ、新しいスクラップとしてカーソルのあるモニタの中央に浮かべる。
/// 倍率 100%、見た目は全体の既定値、履歴は空。読めなければ何も開かず、わけを通知する。
fn open_scrap_file() {
    let Some((owner, initial)) =
        with_app(|app| (app.windows.message, app.settings.general.save_dir.clone()))
    else {
        return;
    };
    let initial = initial.or_else(|| save::default_dir().ok());
    let file_type = save::FileType::scrap_file(text("scrap_file.filter"));
    // モーダルなので App を借りずに呼ぶ
    let path = match save::ask_open_path(owner, initial.as_deref(), file_type) {
        Ok(Some(path)) => path,
        Ok(None) => {
            log::info!("open cancelled");
            return;
        }
        Err(e) => {
            log::error!("could not show the open dialog: {e}");
            notify_user(
                notify::Level::Error,
                &format!("{}\n{e}", text("scrap_file.unreadable")),
            );
            return;
        }
    };
    let file = match scrap_file::read(&path) {
        Ok(file) => file,
        Err(e) => {
            log::error!("could not open {}: {e}", path.display());
            notify_user(
                notify::Level::Error,
                &format!("{}\n{}", text(e.message_key()), path.display()),
            );
            return;
        }
    };
    let (model, content) = file.into_scrap(cursor_monitor());
    let scrap_file::Content::Image(image) = content;
    match float_model(model, image, true) {
        Ok(_) => log::info!("opened {}", path.display()),
        Err(reason) => {
            notify_user(
                notify::Level::Error,
                &format!("{}\n{reason}", text("scrap_file.unreadable")),
            );
        }
    }
}

/// カーソルのあるモニタの矩形（スクリーン座標）。わからなければメインモニタ。
fn cursor_monitor() -> Rect {
    let monitors = monitor_list::list();
    let mut cursor = POINT::default();
    // SAFETY: cursor はこの関数の中で生きている
    let at = unsafe { GetCursorPos(&mut cursor) }
        .ok()
        .map(|()| Point::new(cursor.x, cursor.y));
    at.and_then(|at| monitors.iter().find(|m| m.rect.contains(at)))
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
        .map_or(Rect::new(0, 0, 0, 0), |m| m.rect)
}

/// 設定ファイルを読む。ファイルがなければ既定値で、問題にしない。読めなければ既定値で、TOML として読めない
/// ときと同じ問題にする。
fn read_settings(path: Option<&Path>) -> (Settings, Vec<SettingsIssue>) {
    let Some(path) = path else {
        log::warn!("no settings file location; using the default settings");
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

/// 設定ファイルに `new` を書き戻す。変わった項目だけを書き換え、手で書いたコメントと並び順は残す
/// （`settings::rewrite`）。ファイルがなければ作る。
///
/// - 書いた中身を `App::own_writes` に覚え、監視が拾っても読み込み直さない。そのため、書けたら
///   `App::settings` を `new` にする。ホットキーなど、使うときに読むのでは足りないものの反映は呼び出し側で行う。
/// - 今のファイルが TOML として読めなければ、上書きせずにエラーを返す（手で書いた中身を消さないため）。
/// - 書きかけの中身を監視に読まれないよう、隣の一時ファイルに書いてから名前を変える。
fn write_settings(
    app: &mut App,
    new: &Settings,
) -> std::result::Result<(), settings::WriteFailure> {
    let path = app
        .settings_path
        .clone()
        .ok_or(settings::WriteFailure::NoLocation)?;
    write_settings_file(&path, new, &mut app.own_writes)?;
    app.settings = new.as_written();
    Ok(())
}

/// `write_settings` のうち、ファイルを書く部分。書いた中身を `own_writes` に覚える。
fn write_settings_file(
    path: &Path,
    new: &Settings,
    own_writes: &mut settings::OwnWrites,
) -> std::result::Result<(), settings::WriteFailure> {
    use settings::WriteFailure;
    // 通知には断られたファイルと OS の文言だけを出す（`WriteFailure::detail`）。どの段階で断られたかはログに残す
    let refused_by_os = |stage: &str, path: &std::path::Path, e: std::io::Error| {
        log::warn!(
            "writing the settings: could not {stage} {}: {e}",
            path.display()
        );
        WriteFailure::Io {
            path: path.display().to_string(),
            error: e.to_string(),
        }
    };
    let src = match std::fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(refused_by_os("read back", path, e)),
    };
    let text = settings::rewrite(&src, new).ok_or_else(|| WriteFailure::NotToml {
        path: path.display().to_string(),
    })?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| refused_by_os("create", dir, e))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &text).map_err(|e| refused_by_os("write", &tmp, e))?;
    // 名前を変える前に覚える（監視の知らせは UI スレッドのこの関数が返ってから処理されるが、念のため）
    own_writes.wrote(&text);
    if let Err(e) = std::fs::rename(&tmp, path) {
        own_writes.forget();
        let _ = std::fs::remove_file(&tmp);
        return Err(refused_by_os("replace", path, e));
    }
    log::info!("settings written to {}", path.display());
    Ok(())
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
/// - 中身が自分で書いた直後のもの（`write_settings`）なら、読み込み直さず、通知もしない。
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
    let mut autostart_changed = false;
    let notice = with_app(|app| {
        let own = match &content {
            settings_watch::Content::Text(src) => app.own_writes.is_own(src),
            settings_watch::Content::Missing | settings_watch::Content::Unreadable(_) => {
                app.own_writes.forget();
                false
            }
        };
        if own {
            log::info!("settings file changed by shunsoku itself; not reloading");
            return None;
        }
        let current = &app.settings;
        let (new, mut issues) = match content {
            settings_watch::Content::Missing => settings::reload(current, None),
            settings_watch::Content::Text(src) => settings::reload(current, Some(&src)),
            settings_watch::Content::Unreadable(message) => {
                (current.clone(), vec![SettingsIssue::NotToml { message }])
            }
        };
        log::info!("settings reloaded");
        log_issues(&issues);
        // 空になっても GUI は開かず、通知だけ出す
        let unset = unset::prompt(
            unset::When::Reload {
                before: &app.settings,
            },
            &new,
        ) == Some(unset::Prompt::Notify);
        let old = std::mem::replace(&mut app.settings, new);
        let (keymap_issues, failed) = adopt_settings(app, &old, false);
        autostart_changed = old.general.autostart != app.settings.general.autostart;
        issues.extend(keymap_issues);
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
    if autostart_changed {
        sync_autostart();
    }
}

/// 新しくした設定（`app.settings`、前は `old`）を反映する。読み込み直しと設定 GUI の適用で使う。
///
/// - 変わったものだけを反映する（`settings::diff`）。ホットキーは、変わったとき（`reregister` なら必ず）に登録し直す。
///   ただし、設定 GUI の欄にフォーカスがあって外している間は登録しない（フォーカスが外れたら登録し直す）。
/// - ショートカットとマウスの対応表は毎回作り直す。書き込みの既定値の変化を `Carryover` に伝える。
/// - キーの割り当ての問題と、登録できなかったホットキーを返す。
fn adopt_settings(
    app: &mut App,
    old: &Settings,
    reregister: bool,
) -> (Vec<SettingsIssue>, Vec<hotkey::Failed>) {
    let changes = settings::diff(old, &app.settings);
    log::info!("settings changed: {changes:?}");
    let keymap = settings::keymap(&app.settings, ops::OPERATIONS);
    log_issues(&keymap.issues);
    let failed = if (changes.hotkeys || reregister) && !app.hotkey_pause.is_paused() {
        register_bound(app, &keymap.hotkeys.bound)
    } else {
        Vec::new()
    };
    log_failed(&failed);
    app.shortcuts = keymap.shortcuts;
    app.mouse = keymap.mouse;
    app.carryover
        .settings_changed(&old.annotation, &app.settings.annotation);
    apply_changes(app, &changes);
    (keymap.issues, failed)
}

/// いまの設定（`app.settings`）のホットキーを登録し直す。登録できなかったものを `App::unregistered` に覚えて返す。
fn register_hotkeys(app: &mut App) -> Vec<hotkey::Failed> {
    let keymap = settings::keymap(&app.settings, ops::OPERATIONS);
    register_bound(app, &keymap.hotkeys.bound)
}

/// ホットキー `bound`（いまの設定から作った対応表）を登録し直す。登録できなかったものを `App::unregistered` に
/// 覚えて返す。
fn register_bound(
    app: &mut App,
    bound: &std::collections::BTreeMap<Chord, settings::HotkeyAction>,
) -> Vec<hotkey::Failed> {
    let failed = app.hotkeys.reregister(bound);
    app.unregistered = failed.iter().map(|f| f.action.into()).collect();
    failed
}

/// 読み込み直した設定（`app.settings`）のうち、使うときに読むのでは足りないものを反映する。
fn apply_changes(app: &mut App, changes: &SettingsChanges) {
    if changes.log_level {
        log::set_max_level(level_filter(app.settings.log_level));
    }
    // メニューと通知は出すたびに文言を引く。開いている設定 GUI は、適用したときに組み直す（`settings_gui`）
    if changes.language {
        set_language(&app.settings);
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
    let mut mouse_conflicts = Vec::new();
    let (open, close) = (text("settings.items_open"), text("settings.items_close"));
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
                conflicts.push(format!("{chord}{open}{}{close}", join_items(items)));
            }
            SettingsIssue::Overlap {
                chord,
                hotkey,
                shortcut,
            } => overlaps.push(format!("{chord}{open}{hotkey}, {shortcut}{close}")),
            SettingsIssue::MouseConflict { gesture, items } => {
                mouse_conflicts.push(format!("{gesture}{open}{}{close}", join_items(items)));
            }
        }
    }
    for (key, items) in [
        ("settings.invalid_items", invalid.join(", ")),
        ("settings.conflicts", conflicts.join(", ")),
        ("settings.overlaps", overlaps.join(", ")),
        ("settings.mouse_conflicts", mouse_conflicts.join(", ")),
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

/// 項目名を `, ` でつなぐ（`shortcuts.copy, shortcuts.close`）。
fn join_items(items: &[settings::SettingsItem]) -> String {
    let names: Vec<String> = items.iter().map(ToString::to_string).collect();
    names.join(", ")
}

/// 自動起動の登録（Run キー）をいまの設定に合わせる。起動したとき、設定 GUI で適用したとき、読み込み直しで
/// 自動起動が変わったときに呼ぶ。合わせられなければ通知する。通知を出すので `App` を借りずに呼ぶ。
fn sync_autostart() {
    let Some(enabled) = with_app(|app| app.settings.general.autostart) else {
        return;
    };
    match autostart::sync(enabled) {
        Ok(action) => log::info!("autostart (enabled: {enabled}): {action:?}"),
        Err(e) => {
            log::error!("autostart: {e}");
            notify_user(
                notify::Level::Error,
                &format!("{}\n{e}", text("settings.autostart_failed")),
            );
        }
    }
}

/// トレイの通知を出す。出したら true。設定で通知をオフにしたときは、成功の通知だけを止める（エラーと警告は出す）。
/// 新しい通知は前の通知を置き換えるので、出すならクリックで開くフォルダを忘れる（保存の通知なら呼び出し側で覚え
/// 直す）。出さないなら、前の通知とそのフォルダは残す（`Notices::showing`）。
fn notify_user(level: notify::Level, body: &str) -> bool {
    let target = with_app(|app| {
        let success = matches!(level, notify::Level::Success);
        let show = app
            .notices
            .showing(success, app.settings.general.notifications);
        (app.windows.tray_owner, show)
    });
    let Some((owner, show)) = target else {
        log::warn!("notification dropped: {body}");
        return false;
    };
    if !show {
        return false;
    }
    // Explorer とやりとりするので、App を借りずに出す
    let shown = notify::show(owner, tray::ICON_ID, level, text("notify.title"), body);
    if !shown {
        log::warn!("could not show a notification: {body}");
    }
    shown
}
