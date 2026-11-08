//! 設定 GUI（`settings_dialog`）と `App` をつなぐ。
//!
//! - 開くのは `open`（トレイの「設定…」、2 つ目のプロセスの登録メッセージ）と、範囲選択キャプチャのホットキーが
//!   未設定のまま起動したときの `open_for_unset_hotkey`（その欄にフォーカスを置く）。開いていれば前に出すだけ。
//!   未設定のまま閉じたら（キャンセル）、通知で知らせる（`closed`）。ダイアログは
//!   `App` を借りずに作り（作る間にダイアログ プロシージャが呼ばれるため）、できたら `App::settings_gui` に置く。
//! - 写し（core の `Editor`）は `App::settings_gui` が持つ。欄に出すとき（`refresh`）は写しを写してから、借りずに出す。
//! - 欄にキーが入ったら（`EN_CHANGE`）写しの項目を変え、印とボタンを出し直す。ほかの欄（`settings_dialog::fields`）
//!   も同じく、チェック・入力・選択のたびに写しを変えて出し直す（`on_field`）。欄の横のボタン（「参照…」「選ぶ…」
//!   「変数を挿入」）はモーダルなので、`App` を借りずに出す（`on_button`）。
//! - 「スクラップ操作」タブ（`settings_dialog::scrap_ops`）: 一覧で行を選んだら、ショートカットの欄の `ChordRow::item`
//!   をその行の操作に差し替える。マウスのドロップダウンと「既定に戻す」は写しの行を変える。
//! - 「定型領域」タブ（`settings_dialog::regions`）: 一覧で行を選んだら、詳細の欄（`Fields::bind_region`）と
//!   ホットキーの欄の `ChordRow::item` をその領域に結び直す。一覧の下のボタンは写しの領域を足す・写す・消す・
//!   動かし（`Editor::add_region` など）、移った行を選ぶ。出し直す（`refresh`）ときは一覧の行を先に写しに揃える。
//!   定型領域を選んで名前の欄にフォーカスを置いて開くのは `open_region`（トレイの「定型領域を追加…」で登録した
//!   あとの `region_registered`。開いていれば写しと base にも足す）。
//!   「範囲を選び直す」と「試しに撮る」は、ダイアログを隠してから（写り込まないように）依頼し、範囲選択の画面を
//!   閉じたか撮り終えたら出し直す（`capture_done`）。選び直した範囲は写しに入れるだけ（`region_reselected`）。
//! - 「レシピ」タブ（`settings_dialog::recipes`）: 一覧で行を選んだら、ショートカットの欄の `ChordRow::item` を
//!   そのレシピに結び直す。一覧の下のボタンと操作の並びの横のボタン、名前の欄の入力は写しのレシピを変える
//!   （`Editor::add_recipe` など）。出し直すときは、定型領域と同じく一覧の行を先に写しに揃える。
//! - キーの組み合わせの欄にフォーカスがある間は、全体のホットキーの登録を外す（`App::hotkey_pause`）。欄の
//!   フォーカスの出入りで `WM_APP_SETTINGS_FOCUS` を自分に送り、届いた時点のフォーカスで決める（`check_focus`）。
//! - 適用（`apply`）: その時点のファイルと写しを 3 者で突き合わせ（`settings::merge`）、両方で変えた項目があれば
//!   借りずに確かめてから、結果を書き（`write_settings`）、読み込み直しと同じ反映をし（`adopt_settings`）、ホットキーは
//!   必ず登録し直して、登録できなかった項目を写しの印にする（`Editor::applied`）。OK は適用して、書けたら閉じる
//!   （登録できなかったホットキーは通知で知らせる）。
//! - 適用で UI の言語が変わったら（`SettingsGui::language` と違ったら）、同じタブ・同じ位置で組み直す（`rebuild`）。
//!   今のダイアログは `REBUILDING` を立てて壊し（`WM_DESTROY` で `App::settings_gui` を消さない）、写しはそのまま
//!   新しいダイアログに出す。外の変更で言語が変わったときは、次に適用したときに組み直す。

use std::cell::Cell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use shunsoku_core::geometry::Rect;
use shunsoku_core::i18n::{self, Language};
use shunsoku_core::monitors;
use shunsoku_core::settings::editor::{AppearanceInput, AppearanceItem, Editor, Move};
use shunsoku_core::settings::form::{ClickPlace, FieldEntry, PauseChange};
use shunsoku_core::settings::merge::{Field, merge};
use shunsoku_core::settings::tab_step::{StepFocus, TabStep};
use shunsoku_core::settings::{self, Region, Settings, SettingsItem, unset};
use windows::Win32::Foundation::RECT;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_TRANSITIONS_FORCEDISABLED, DwmFlush, DwmSetWindowAttribute,
};
use windows::Win32::UI::Controls::{
    EM_GETSEL, EM_SETSEL, LVIS_SELECTED, LVN_ITEMCHANGED, NM_CUSTOMDRAW, NMHDR, NMLISTVIEW,
    NMLVCUSTOMDRAW, TCN_SELCHANGE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, CBN_SELCHANGE, DWLP_MSGRESULT, DestroyWindow, EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS,
    GetWindowRect, IDCANCEL, IDOK, IsDialogMessageW, IsIconic, MB_DEFBUTTON2, MB_ICONWARNING,
    MB_OKCANCEL, MSG, MessageBoxW, PostMessageW, PostQuitMessage, SW_HIDE, SW_RESTORE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WINDOW_LONG_PTR_INDEX, WM_ACTIVATE, WM_COMMAND, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_INITDIALOG, WM_LBUTTONDOWN, WM_NEXTDLGCTL, WM_NOTIFY,
    WM_SETFOCUS, WM_SETTINGCHANGE,
};
use windows::Win32::UI::WindowsAndMessaging::{GetDlgCtrlID, IsChild, WM_KEYDOWN};
use windows::core::{BOOL, PCWSTR};

use super::{
    App, EXIT_PANIC, Moment, WM_APP_SETTINGS_DPI, WM_APP_SETTINGS_FOCUS, adopt_settings, language,
    log_failed, notify_user, register_hotkeys, settings_notice, sync_autostart, text, with_app,
    write_settings,
};
use crate::settings_dialog::fields::{self, Action, Fields, Input, Kind};
use crate::settings_dialog::recipes;
use crate::settings_dialog::regions::{self, ListButton};
use crate::settings_dialog::{
    self, ChordRow, During, Env, ID_APPLY, ID_TAB, Page, chord_field, scrap_ops,
};
use crate::wide::to_wide;
use crate::{hotkey, menu, monitor_list, notify, panic_hook, save};

/// 開いている設定 GUI。
pub(super) struct SettingsGui {
    hwnd: HWND,
    rows: Vec<ChordRow>,
    fields: Fields,
    editor: Editor,
    /// フォーカスのあるテキストの欄と、入ったときの文字（読めない値で出たら戻す）。
    entry: FieldEntry<fields::Field>,
    /// ダイアログを組んだときの UI の言語。
    language: Language,
    /// ダイアログに渡す、外のこと（モニタは `refresh_monitors` で集め直す）。
    env: Env,
}

impl SettingsGui {
    /// テキストの欄 `field` に入った。`text` はそのときの欄の文字。
    fn field_entered(&mut self, field: fields::Field, text: String) {
        self.entry.entered(field, text);
    }

    /// テキストの欄 `field` から出た。読めなければ、写しと欄に戻す文字を返す（`FieldEntry::left`）。
    fn field_left(&mut self, field: fields::Field, readable: bool) -> Option<String> {
        self.entry.left(field, readable)
    }

    /// 適用の前に、フォーカスのある欄 `field` の今の文字 `text` を確かめる。読めなければ、写しと欄に戻す文字を
    /// 返す（`FieldEntry::applying`）。
    fn field_applying(
        &mut self,
        field: fields::Field,
        text: String,
        readable: bool,
    ) -> Option<String> {
        self.entry.applying(field, text, readable)
    }

    /// 「レシピ」タブのショートカットの欄が結ばれているレシピの位置。
    fn bound_recipe(&self) -> Option<usize> {
        let row = self
            .rows
            .iter()
            .find(|row| row.field == recipes::shortcut_id())?;
        match row.item {
            SettingsItem::RecipeShortcut(i) | SettingsItem::RecipeHotkey(i) => Some(i),
            _ => None,
        }
    }

    /// 「レシピ」タブのキーの欄を、結んでいるレシピのいまの最初の操作に合う項目（キャプチャで始まればホットキー、
    /// ほかはショートカット。ADR-0010）に結び直す。
    fn rebind_recipe_key(&mut self) {
        let Some(item) = self
            .bound_recipe()
            .and_then(|index| self.editor.key_item(index))
        else {
            return;
        };
        self.rebind_chord_row(recipes::shortcut_id(), item);
    }

    /// 一覧の行で変わるキーの欄（ID `field`）を、設定の項目 `item` に結び直す。
    fn rebind_chord_row(&mut self, field: i32, item: SettingsItem) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.field == field) {
            row.item = item;
        }
    }
}

thread_local! {
    /// 開いている設定 GUI のウィンドウ。メッセージ ループの `IsDialogMessageW` に渡す（`App` を借りずに読む）。
    static DIALOG: Cell<Option<HWND>> = const { Cell::new(None) };
    /// 「範囲を選び直す」「試しに撮る」の間、隠しているダイアログ（`capture_done` で出し直す）。
    static HIDDEN: Cell<Option<HWND>> = const { Cell::new(None) };
    /// 組み直すために今のダイアログを壊し、新しく作っている（その間の通知は無視する。`WM_DESTROY` で閉じた
    /// ことにしない）。
    static REBUILDING: Cell<bool> = const { Cell::new(false) };
}

/// 設定 GUI へのメッセージ（Tab の移動など）なら処理して true。Ctrl+Tab / Ctrl+Shift+Tab はタブを移す
/// （フォーカスがどの欄にあっても。キーの組み合わせの欄も Tab は入力にしない）。
pub(super) fn is_dialog_message(msg: &MSG) -> bool {
    let Some(dialog) = DIALOG.get() else {
        return false;
    };
    // SAFETY: ウィンドウの親子を調べるだけ
    let inside = msg.hwnd == dialog || unsafe { IsChild(dialog, msg.hwnd) }.as_bool();
    if inside
        && msg.message == WM_KEYDOWN
        && let Some(step) = TabStep::from_key_down(msg.wParam.0 as u16, chord_field::held())
    {
        // SAFETY: フォーカスのあるウィンドウを読むだけ
        let focus = unsafe { GetFocus() };
        let in_page = settings_dialog::in_page(dialog, focus);
        settings_dialog::step_page(dialog, step);
        // 標準のプロパティシートと同じく、欄にいたら新しいページの最初の欄へ移す（隠したページに残さない）
        if StepFocus::after(in_page) == StepFocus::FirstField {
            match settings_dialog::first_field(dialog) {
                Some(field) => focus_control(dialog, field),
                None => set_focus(dialog, ID_TAB),
            }
        }
        return true;
    }
    // SAFETY: msg はこの呼び出しの間生きている
    unsafe { IsDialogMessageW(dialog, msg) }.as_bool()
}

/// 設定 GUI を開く（`page` が None なら最初のタブ）。開いていれば前に出す。
pub(super) fn open(page: Option<Page>) {
    open_with(page, None);
}

/// 範囲選択キャプチャのホットキーが未設定のまま起動した: 「キャプチャ」タブで開き、その欄にフォーカスを置く。
pub(super) fn open_for_unset_hotkey() {
    open_with(Some(Page::Capture), Some(SettingsItem::SelectCaptureHotkey));
}

/// トレイの「定型領域を追加…」で登録する定型領域。名前は `"{base} N"` で、適用済みの設定とも、開いていれば
/// 写しの定型領域とも重ならない。`monitor` はモニタの識別子、`rect` はモニタ相対の範囲。
pub(super) fn region_to_register(app: &App, base: &str, monitor: &str, rect: Rect) -> Region {
    match &app.settings_gui {
        Some(gui) => gui
            .editor
            .new_registered_region(&app.settings, base, monitor, rect),
        None => settings::new_region(&app.settings, base, monitor, rect),
    }
}

/// トレイの「定型領域を追加…」で定型領域 `region` を設定ファイルに書いた。「定型領域」タブで開いてその領域を選び、
/// 名前の欄にフォーカスを置く（自動の名前「領域 N」は初期値のまま）。開いていれば、写しと base にも足す。
pub(super) fn region_registered(region: Region) {
    let Some((open, index)) = with_app(|app| match app.settings_gui.as_mut() {
        Some(gui) => (Some(gui.hwnd), gui.editor.registered(region)),
        // 開くときの写しは今の設定（書いた定型領域は末尾）
        None => (None, app.settings.regions.len().saturating_sub(1)),
    }) else {
        return;
    };
    if let Some(hwnd) = open {
        refresh(hwnd);
    }
    open_region(index);
}

/// 「定型領域」タブで開き、写しの定型領域 `index` を選んで名前の欄にフォーカスを置く。開いていれば前に出す。
fn open_region(index: usize) {
    open(Some(Page::Regions));
    let Some(hwnd) = DIALOG.get() else {
        return;
    };
    regions::select(hwnd, index);
    set_focus(hwnd, regions::name_id());
}

/// 設定 GUI を `page` のタブで開き、`focus` の欄（None ならタブ）にフォーカスを置く。開いていれば前に出す。
fn open_with(page: Option<Page>, focus: Option<SettingsItem>) {
    let Some((existing, module, settings, unregistered)) = with_app(|app| {
        (
            app.settings_gui.as_ref().map(|gui| gui.hwnd),
            app.module,
            app.settings.clone(),
            app.unregistered.clone(),
        )
    }) else {
        return;
    };
    if let Some(hwnd) = existing {
        if let Some(page) = page {
            settings_dialog::show_page(hwnd, page);
        }
        bring_to_front(hwnd);
        if let Some(item) = focus {
            let field = with_app(|app| {
                app.settings_gui
                    .as_ref()
                    .and_then(|gui| field_of(gui, &item))
            })
            .flatten();
            if let Some(field) = field {
                set_focus(hwnd, field);
            }
        }
        return;
    }
    let env = Env {
        monitors: monitor_list::list(),
        default_dir: save::default_dir().ok(),
        file_name: file_name_in,
    };
    // ダイアログを作る間にプロシージャが呼ばれるので、借りずに作る
    let page = page.unwrap_or_default();
    let dialog = match settings_dialog::create(module, page, text, &env, Some(dialog_proc)) {
        Ok(dialog) => dialog,
        Err(e) => {
            log::error!("could not open the settings GUI: {e}");
            return;
        }
    };
    let hwnd = dialog.hwnd;
    let focus = focus
        .and_then(|item| dialog.rows.iter().find(|row| row.item == item))
        .map_or(ID_TAB, |row| row.field);
    let mut editor = Editor::new(settings.clone());
    // 起動時などに登録できなかったホットキーも印にする
    editor.applied(settings, unregistered);
    let gui = SettingsGui {
        hwnd,
        rows: dialog.rows,
        fields: dialog.fields,
        editor,
        entry: FieldEntry::default(),
        language: language(),
        env,
    };
    if with_app(|app| app.settings_gui = Some(gui)).is_none() {
        // SAFETY: 作ったばかりの自分のダイアログ
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return;
    }
    DIALOG.set(Some(hwnd));
    refresh(hwnd);
    // SAFETY: 自分のダイアログ
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    // 既定ではタブに置く（開いただけでホットキーを外さない）。欄に置けば、欄のフォーカスでホットキーを外す
    set_focus(hwnd, focus);
    log::info!("settings GUI opened");
}

/// 写しの項目 `item` の欄の ID。
fn field_of(gui: &SettingsGui, item: &SettingsItem) -> Option<i32> {
    gui.rows
        .iter()
        .find(|row| &row.item == item)
        .map(|row| row.field)
}

/// ダイアログの中の ID `id` のコントロールにフォーカスを置く。
fn set_focus(hwnd: HWND, id: i32) {
    if let Some(control) = settings_dialog::item(hwnd, id) {
        focus_control(hwnd, control);
    }
}

/// ダイアログ `hwnd` のコントロール `control` にフォーカスを移す（既定のボタンの印もダイアログに直させる）。
fn focus_control(hwnd: HWND, control: HWND) {
    // SAFETY: 自分のダイアログと、その子のコントロール
    unsafe {
        SendMessageW(
            hwnd,
            WM_NEXTDLGCTL,
            Some(WPARAM(control.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

fn bring_to_front(hwnd: HWND) {
    // SAFETY: 自分のダイアログ
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// 写しを欄に出す。定型領域の一覧の行を先に揃える（揃えて行を選び直すと、欄を結び直すため）。
fn refresh(hwnd: HWND) {
    let list = with_app(|app| {
        app.settings_gui
            .as_ref()
            .map(|gui| (gui.editor.clone(), gui.fields.region(), gui.env.clone()))
    })
    .flatten();
    if let Some((editor, bound, env)) = list {
        regions::sync_list(hwnd, &editor, bound, text, &env);
        recipes::sync_list(hwnd, &editor, bound_recipe(), text);
    }
    let snapshot = with_app(|app| {
        app.settings_gui.as_mut().map(|gui| {
            gui.rebind_recipe_key();
            let pass = gui.fields.begin_fill();
            let fields = gui.fields.clone();
            (
                gui.rows.clone(),
                fields,
                pass,
                gui.editor.clone(),
                gui.env.clone(),
            )
        })
    })
    .flatten();
    if let Some((rows, fields, pass, editor, env)) = snapshot {
        // コントロールにメッセージを送るので、借りずに出す
        settings_dialog::fill(hwnd, &rows, &editor, text, &env);
        fields::fill(hwnd, &fields, pass, &editor, text, &env);
    }
}

/// モニタの一覧を集め直す（開いたとき、行を選び直したとき、画面の構成が変わったとき）。
fn refresh_monitors() {
    let monitors = monitor_list::list();
    with_app(|app| {
        if let Some(gui) = app.settings_gui.as_mut() {
            gui.env.monitors = monitors;
        }
    });
}

/// `dir` に `template` でいま撮ったら付く名前（欄のプレビュー。`Env::file_name`）。
fn file_name_in(dir: &Path, template: &str, name: &str) -> Option<String> {
    match save::destination(dir, template, name, save::now(), &[]) {
        Ok(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()),
        Err(e) => {
            log::debug!("could not list {}: {e}", dir.display());
            None
        }
    }
}

/// ダイアログ プロシージャ。`app::dispatch` と同じくパニックから守る。
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if panic_hook::ui_panicked() {
        return 0;
    }
    match catch_unwind(AssertUnwindSafe(|| on_message(hwnd, msg, wparam, lparam))) {
        Ok(handled) => isize::from(handled),
        Err(_) => {
            // SAFETY: UI スレッドで呼ぶ
            unsafe { PostQuitMessage(EXIT_PANIC) };
            0
        }
    }
}

/// ダイアログへのメッセージ。処理したら true（false なら既定のダイアログの処理）。
fn on_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    // 組み直しの間（古いダイアログを壊し、新しいダイアログを作る間）の通知は、写しにも App にも当てない
    if REBUILDING.get() {
        return false;
    }
    match msg {
        // フォーカスは開いたあとで置く（`open`）
        WM_INITDIALOG => false,
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            on_command(hwnd, id, code, HWND(lparam.0 as *mut _));
            true
        }
        WM_NOTIFY => {
            // SAFETY: WM_NOTIFY の lparam は NMHDR を指す
            let header = unsafe { &*(lparam.0 as *const NMHDR) };
            if header.idFrom == regions::list_id() as usize && header.code == NM_CUSTOMDRAW {
                // SAFETY: 一覧の NM_CUSTOMDRAW の lparam は NMLVCUSTOMDRAW を指す
                let draw = unsafe { &*(lparam.0 as *const NMLVCUSTOMDRAW) };
                let result = regions::custom_draw(draw);
                // SAFETY: 自分のダイアログの結果を置く
                unsafe {
                    SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(DWLP_MSGRESULT as i32), result)
                };
                return true;
            }
            if header.idFrom == ID_TAB as usize && header.code == TCN_SELCHANGE {
                settings_dialog::show_page(hwnd, settings_dialog::current_page(hwnd));
            } else if header.idFrom == scrap_ops::list_id() as usize && row_selected(header, lparam)
            {
                on_scrap_op_selected(hwnd);
            } else if header.idFrom == regions::list_id() as usize && row_selected(header, lparam) {
                on_region_selected(hwnd);
            } else if header.idFrom == recipes::list_id() as usize && row_selected(header, lparam) {
                on_recipe_selected(hwnd);
            } else if header.idFrom == recipes::steps_id() as usize
                && row_selected(header, lparam)
                && !recipes::is_syncing()
            {
                // 上へ・下へ・削除を押せるかを出し直す
                refresh(hwnd);
            }
            false
        }
        WM_DISPLAYCHANGE => {
            // 定型領域の警告とモニタの名前を出し直す
            refresh_monitors();
            refresh(hwnd);
            false
        }
        WM_SETTINGCHANGE => {
            let area = PCWSTR(lparam.0 as *const u16);
            // SAFETY: lparam は NUL で終わる文字列か NULL
            if !area.is_null()
                && unsafe { area.to_string() }.is_ok_and(|s| s == "ImmersiveColorSet")
            {
                settings_dialog::apply_dark(hwnd);
            }
            false
        }
        WM_APP_SETTINGS_FOCUS => {
            check_focus();
            true
        }
        // 欄の外のクリックで、欄からフォーカスを外す（#118）。ページの中のクリックはタブが拾う
        WM_LBUTTONDOWN => {
            settings_dialog::blur_on_click(hwnd, ClickPlace::Background);
            false
        }
        // 外したフォーカスを、ダイアログの既定の処理で欄へ戻させない
        WM_SETFOCUS => settings_dialog::blurring(),
        // 欄から外したまま前面から外れたら、戻ったときに欄へフォーカスを戻させない（#118）
        WM_ACTIVATE => settings_dialog::on_activate(hwnd, wparam),
        WM_DPICHANGED => {
            // ダイアログが大きさと書体を直し終えてから、一覧の列の幅を合わせ直す
            // SAFETY: 自分のダイアログに送る
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_SETTINGS_DPI, WPARAM(0), LPARAM(0));
            }
            false
        }
        WM_APP_SETTINGS_DPI => {
            regions::fit_columns(hwnd);
            scrap_ops::fit_columns(hwnd);
            recipes::fit_columns(hwnd);
            true
        }
        WM_DESTROY => {
            closed();
            false
        }
        _ => false,
    }
}

/// 一覧の通知 `header`（`lparam` が指す）が、行が選ばれた（選ばれていなかった行が選ばれた）ことか。
fn row_selected(header: &NMHDR, lparam: LPARAM) -> bool {
    if header.code != LVN_ITEMCHANGED {
        return false;
    }
    // SAFETY: LVN_ITEMCHANGED の lparam は NMLISTVIEW を指す
    let change = unsafe { &*(lparam.0 as *const NMLISTVIEW) };
    change.uNewState & LVIS_SELECTED.0 != 0 && change.uOldState & LVIS_SELECTED.0 == 0
}

fn on_command(hwnd: HWND, id: i32, code: u32, control: HWND) {
    if id == IDOK.0 {
        if code == BN_CLICKED && focused_field_readable(hwnd) {
            // 書けたら閉じる。登録できなかったホットキーは、欄の印が見えなくなるので通知で知らせる
            match apply(hwnd) {
                Some(failed) => {
                    destroy(hwnd);
                    if !failed.is_empty() {
                        let chords: Vec<String> =
                            failed.iter().map(|f| f.chord.to_string()).collect();
                        let body = format!("{}{}", text("hotkey.failed"), chords.join(", "));
                        notify_user(notify::Level::Warning, &body);
                    }
                }
                None => rebuild_if_language_changed(hwnd),
            }
        }
        return;
    }
    match id {
        _ if id == IDCANCEL.0 => destroy(hwnd),
        ID_APPLY if code == BN_CLICKED => {
            if !focused_field_readable(hwnd) {
                return;
            }
            apply(hwnd);
            rebuild_if_language_changed(hwnd);
        }
        _ if id == scrap_ops::mouse_id() && code == CBN_SELCHANGE => {
            if let Some((row, gesture)) = scrap_ops::chosen_mouse(hwnd) {
                edit(hwnd, |editor| editor.set_mouse(&row, gesture));
            }
        }
        _ if id == scrap_ops::reset_id() && code == BN_CLICKED => {
            if let Some(row) = scrap_ops::selected(hwnd) {
                edit(hwnd, |editor| editor.reset_binding(&row));
            }
        }
        _ if id == regions::reselect_id() && code == BN_CLICKED => on_reselect(hwnd),
        _ if id == regions::test_capture_id() && code == BN_CLICKED => on_test_capture(hwnd),
        _ if code == BN_CLICKED && regions::list_button(id).is_some() => {
            if let Some(button) = regions::list_button(id) {
                on_region_list_button(hwnd, button);
            }
        }
        _ if code == BN_CLICKED && recipes::button(id).is_some() => {
            if let Some(button) = recipes::button(id) {
                on_recipe_button(hwnd, button);
            }
        }
        _ if id == recipes::name_id() => match code {
            EN_CHANGE if !recipes::is_syncing() => {
                if let (Some(i), Some(name)) = (recipes::selected(hwnd), recipes::name_text(hwnd)) {
                    edit(hwnd, |editor| editor.set_recipe_name(i, &name));
                }
            }
            // 打っている間は書き換えない欄を、出たら写しの名前（前後の空白を除いたもの）に揃える
            EN_KILLFOCUS => refresh(hwnd),
            _ => {}
        },
        _ if let Some(item) = recipes::appearance_item(id) => match code {
            CBN_SELCHANGE | EN_CHANGE if !recipes::is_syncing() => {
                set_appearance(hwnd, item, recipes::appearance_input(hwnd, item));
            }
            // 打っている間は書き換えない欄を、出たら写しの値に揃える
            EN_KILLFOCUS => refresh(hwnd),
            _ => {}
        },
        _ if chord_field::is_field(control) => match code {
            EN_CHANGE => on_chord_changed(hwnd, id, chord_field::value(control)),
            EN_SETFOCUS | EN_KILLFOCUS => {
                // フォーカスが移り終えてから見る
                // SAFETY: 自分のダイアログに送る
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_APP_SETTINGS_FOCUS, WPARAM(0), LPARAM(0));
                }
            }
            _ => {}
        },
        _ => {
            if !fields::is_filling() {
                on_field(hwnd, id, code, control);
            }
        }
    }
}

/// OK・適用の前に、フォーカスのあるテキストの欄を、欄から出たときと同じに確かめる。Enter（既定の OK）や
/// ボタンのアクセスキーはフォーカスを動かさないので、読めない入力の欄から出る前に適用が来る。読めなければ、
/// 入ったときの値に戻して書き方を吹き出しで知らせ、false を返す（適用しない・閉じない）。
fn focused_field_readable(hwnd: HWND) -> bool {
    // SAFETY: フォーカスのあるウィンドウとその ID を読むだけ
    let control = unsafe { GetFocus() };
    // SAFETY: 同上
    let id = unsafe { GetDlgCtrlID(control) };
    let row = with_gui(|gui| gui.fields.row(id)).flatten();
    let Some(row) = row.filter(|row| matches!(row.kind, Kind::Text { .. } | Kind::Number { .. }))
    else {
        return true;
    };
    let Input::Text(s) = fields::input(row, control) else {
        return true;
    };
    let rejected = fields::rejected(row.field, &s);
    let Some(restore) =
        with_gui(|gui| gui.field_applying(row.field, s, rejected.is_none())).flatten()
    else {
        return true;
    };
    // フォーカスのある欄は `refresh` では書き換わらないので、欄の文字を直に戻す（`EN_CHANGE` で写しにも入る）
    fields::set_window_text(control, &restore);
    set_field(row.field, Input::Text(restore));
    refresh(hwnd);
    if let Some(rejected) = rejected {
        fields::show_rejected(control, rejected, text);
    }
    false
}

/// 欄（`settings_dialog::fields`）か、その横のボタンへの通知。
fn on_field(hwnd: HWND, id: i32, code: u32, control: HWND) {
    let Some(fields) =
        with_app(|app| app.settings_gui.as_ref().map(|gui| gui.fields.clone())).flatten()
    else {
        return;
    };
    if let Some(button) = fields.button(id) {
        if code == BN_CLICKED {
            on_button(hwnd, &fields, button.action);
        }
        return;
    }
    let Some(row) = fields.row(id) else {
        return;
    };
    let mut rejected = None;
    match (row.kind, code) {
        (Kind::Check, BN_CLICKED)
        | (Kind::Text { .. } | Kind::Number { .. }, EN_CHANGE)
        | (Kind::Choice, CBN_SELCHANGE) => set_field(row.field, fields::input(row, control)),
        // 入ったときの文字を覚えておく
        (Kind::Text { .. } | Kind::Number { .. }, EN_SETFOCUS) => {
            if let Input::Text(s) = fields::input(row, control) {
                with_gui(|gui| gui.field_entered(row.field, s));
            }
            return;
        }
        // 読めない入力は、フォーカスが外れたら入ったときの値に戻し（打つ途中に読めた値は写しに入っている）、
        // 書き方を吹き出しで知らせる
        (Kind::Text { .. } | Kind::Number { .. }, EN_KILLFOCUS) => {
            if let Input::Text(s) = fields::input(row, control) {
                rejected = fields::rejected(row.field, &s);
            }
            let restore = with_gui(|gui| gui.field_left(row.field, rejected.is_none())).flatten();
            if let Some(text) = restore {
                set_field(row.field, Input::Text(text));
            }
        }
        _ => return,
    }
    refresh(hwnd);
    // 戻した後に出す（欄の文字を書き換えると吹き出しは消える）
    if let Some(rejected) = rejected {
        fields::show_rejected(control, rejected, text);
    }
}

/// 写しの項目 `field` を `input` にする。
fn set_field(field: fields::Field, input: Input) {
    with_gui(|gui| fields::set(field, gui.editor.draft_mut(), input));
}

/// 開いている設定 GUI を短く借りる（`with_app` と同じく、中でメッセージを送らない）。開いていなければ None。
fn with_gui<R>(f: impl FnOnce(&mut SettingsGui) -> R) -> Option<R> {
    with_app(|app| app.settings_gui.as_mut().map(f)).flatten()
}

/// 欄の横のボタンが押された。ダイアログとメニューはモーダルなので、`App` を借りずに出す。
fn on_button(hwnd: HWND, fields: &Fields, action: Action) {
    let Some((draft, default_dir)) = with_app(|app| {
        app.settings_gui
            .as_ref()
            .map(|gui| (gui.editor.draft().clone(), gui.env.default_dir.clone()))
    })
    .flatten() else {
        return;
    };
    let folder_of = |field| fields::folder_of(&draft, field, default_dir.as_deref());
    match action {
        Action::BrowseFolder(field) => match save::ask_folder(hwnd, &folder_of(field)) {
            Ok(Some(dir)) => set_field(field, Input::Text(dir.to_string_lossy().into_owned())),
            Ok(None) => {}
            Err(e) => log::error!("could not show the folder dialog: {e}"),
        },
        Action::PickColor(field) => {
            let Some(current) = fields::color_of(&draft, field) else {
                return;
            };
            if let Some(color) = fields::pick_color(hwnd, current) {
                set_field(field, Input::Text(color.to_string()));
            }
        }
        Action::InsertVariable(of) => insert_variable(hwnd, fields, of),
        Action::OpenFolder(field) => {
            let dir = folder_of(field);
            if !save::open_folder(&dir) {
                log::warn!("could not open the folder {}", dir.display());
            }
        }
    }
    refresh(hwnd);
}

/// 変数のメニューを「変数を挿入」のボタンの下に出し、選ばれた変数をテンプレートの欄の選択に置き換える。
fn insert_variable(hwnd: HWND, fields: &Fields, of: fields::TemplateOf) {
    let Some(edit) = fields
        .row_of(fields::Field::Template(of))
        .and_then(|row| settings_dialog::item(hwnd, row.control))
    else {
        return;
    };
    let button = fields
        .buttons
        .iter()
        .find(|b| b.action == Action::InsertVariable(of))
        .and_then(|b| settings_dialog::item(hwnd, b.id));
    let mut rect = RECT::default();
    if let Some(button) = button {
        // SAFETY: 自分のボタンの位置を読む
        unsafe {
            let _ = GetWindowRect(button, &mut rect);
        }
    }
    let variables: Vec<_> = fields::variables(of).collect();
    let entries: Vec<menu::Entry> = variables
        .iter()
        .zip(1u32..)
        // 文言に変数の書き方が入っている（`{datetime:…}` は挿入する既定の書式を略して見せる）
        .map(|(v, id)| menu::Entry::item(id, text(v.label_key), true))
        .collect();
    let Some(chosen) = menu::show(hwnd, &entries, rect.left, rect.bottom) else {
        return;
    };
    let Some(variable) = chosen
        .checked_sub(1)
        .and_then(|i| variables.get(i as usize))
    else {
        return;
    };
    let (mut start, mut end) = (0u32, 0u32);
    // SAFETY: 自分のテンプレートの欄。選択の位置を受ける変数は呼び出しの間生きている
    unsafe {
        SendMessageW(
            edit,
            EM_GETSEL,
            Some(WPARAM(&mut start as *mut u32 as usize)),
            Some(LPARAM(&mut end as *mut u32 as isize)),
        );
    }
    let current = fields::window_text(edit);
    let (new, caret) =
        shunsoku_core::template::insert(&current, start as usize..end as usize, variable.token);
    // 書き換えで EN_CHANGE が来て、写しに入る
    fields::set_window_text(edit, &new);
    // SAFETY: 自分のテンプレートの欄
    unsafe {
        let _ = SetFocus(Some(edit));
        SendMessageW(
            edit,
            EM_SETSEL,
            Some(WPARAM(caret)),
            Some(LPARAM(caret as isize)),
        );
    }
}

/// 組んだときから UI の言語が変わっていたら、同じタブ・同じ位置で組み直す。
fn rebuild_if_language_changed(hwnd: HWND) {
    let built = with_app(|app| app.settings_gui.as_ref().map(|gui| gui.language)).flatten();
    if built.is_some_and(|built| built != language()) {
        rebuild(hwnd);
    }
}

/// 今のダイアログ `old` を壊し、いまの言語で組み直して写しを出す。ダイアログを作る間にプロシージャが呼ばれる
/// ので、借りずに作る（`open_with` と同じ）。
fn rebuild(old: HWND) {
    let Some((module, env)) = with_app(|app| {
        let env = app.settings_gui.as_ref()?.env.clone();
        Some((app.module, env))
    })
    .flatten() else {
        return;
    };
    let page = settings_dialog::current_page(old);
    let mut rect = RECT::default();
    // SAFETY: 自分のダイアログの位置を読む
    unsafe {
        let _ = GetWindowRect(old, &mut rect);
    }
    let created = {
        let _rebuilding = During::enter(&REBUILDING);
        destroy(old);
        DIALOG.set(None);
        settings_dialog::create(module, page, text, &env, Some(dialog_proc))
    };
    let dialog = match created {
        Ok(dialog) => dialog,
        Err(e) => {
            log::error!("could not rebuild the settings GUI: {e}");
            closed();
            return;
        }
    };
    let hwnd = dialog.hwnd;
    let kept = with_app(|app| {
        let gui = app.settings_gui.as_mut()?;
        gui.hwnd = hwnd;
        gui.rows = dialog.rows;
        gui.fields = dialog.fields;
        gui.language = language();
        Some(())
    })
    .flatten();
    if kept.is_none() {
        destroy(hwnd);
        return;
    }
    DIALOG.set(Some(hwnd));
    refresh(hwnd);
    // SAFETY: 自分のダイアログ
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    set_focus(hwnd, ID_TAB);
    // 壊したダイアログの欄にフォーカスがあれば、ホットキーは外したまま。いまのフォーカスで決め直す
    // SAFETY: 自分のダイアログに送る
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_SETTINGS_FOCUS, WPARAM(0), LPARAM(0));
    }
    log::info!("settings GUI rebuilt in {:?}", language());
}

fn destroy(hwnd: HWND) {
    // SAFETY: 自分のダイアログ。App を借りずに呼ぶ（WM_DESTROY で借りる）
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// 欄 `field` にキーが入った。
fn on_chord_changed(hwnd: HWND, field: i32, chord: Option<shunsoku_core::keys::Chord>) {
    with_app(|app| {
        if let Some(gui) = app.settings_gui.as_mut()
            && let Some(row) = gui.rows.iter().find(|row| row.field == field)
        {
            gui.editor.set_chord(&row.item, chord);
        }
    });
    refresh(hwnd);
}

/// 写しを `change` で変えて、欄に出し直す。`change` の結果を返す。
fn edit<T>(hwnd: HWND, change: impl FnOnce(&mut Editor) -> T) -> Option<T> {
    let result =
        with_app(|app| app.settings_gui.as_mut().map(|gui| change(&mut gui.editor))).flatten();
    refresh(hwnd);
    result
}

/// 「定型領域」タブの一覧で行を選んだ。詳細の欄とホットキーの欄をその領域に結び直す。
fn on_region_selected(hwnd: HWND) {
    let Some(index) = regions::selected(hwnd) else {
        return;
    };
    let field = regions::hotkey_id();
    with_app(|app| {
        if let Some(gui) = app.settings_gui.as_mut() {
            gui.fields.bind_region(index);
            gui.rebind_chord_row(field, SettingsItem::RegionHotkey(index));
        }
    });
    // 一覧を揃えている途中なら、揃えたあとで出す（`refresh`）
    if !regions::is_syncing() {
        refresh_monitors();
        refresh(hwnd);
    }
}

/// 「レシピ」タブのショートカットの欄が指しているレシピの位置（一覧を揃えて選ぶ行）。
fn bound_recipe() -> usize {
    with_app(|app| app.settings_gui.as_ref()?.bound_recipe())
        .flatten()
        .unwrap_or(0)
}

/// 「レシピ」タブの一覧で行を選んだ。ショートカットの欄をそのレシピに結び直す。
fn on_recipe_selected(hwnd: HWND) {
    let Some(index) = recipes::selected(hwnd) else {
        return;
    };
    let field = recipes::shortcut_id();
    with_app(|app| {
        if let Some(gui) = app.settings_gui.as_mut() {
            let item = gui
                .editor
                .key_item(index)
                .unwrap_or(SettingsItem::RecipeShortcut(index));
            gui.rebind_chord_row(field, item);
        }
    });
    // 一覧を揃えている途中なら、揃えたあとで出す（`refresh`）
    if !recipes::is_syncing() {
        refresh(hwnd);
    }
}

/// 「レシピ」タブのボタン。写しを変えて、移った行（レシピか操作）を選ぶ。足したレシピは名前の欄にフォーカスを置く。
fn on_recipe_button(hwnd: HWND, button: recipes::Button) {
    use recipes::Button;
    let selected = recipes::selected(hwnd);
    let step = recipes::selected_step(hwnd);
    let in_range = |editor: &Editor| selected.filter(|&i| i < editor.draft().recipes.len());
    // 選んでいるレシピの選んでいる操作を `change(editor, レシピ, 操作)` で変え、移った操作を選ぶ
    let change_step = |change: fn(&mut Editor, usize, usize) -> Option<usize>| {
        let to = edit(hwnd, |editor| {
            let i = in_range(editor)?;
            let s = step.filter(|&s| s < editor.draft().recipes[i].steps.len())?;
            change(editor, i, s)
        })
        .flatten();
        if let Some(to) = to {
            recipes::select_step(hwnd, to);
        }
        refresh(hwnd);
    };
    match button {
        Button::Add => {
            let base = text("settings_gui.recipes.default_name");
            if let Some(index) = edit(hwnd, |editor| editor.add_recipe(base)) {
                recipes::select(hwnd, index);
                set_focus(hwnd, recipes::name_id());
            }
        }
        Button::Remove => {
            if let Some(index) =
                edit(hwnd, |editor| editor.remove_recipe(in_range(editor)?)).flatten()
            {
                recipes::select(hwnd, index);
            }
        }
        Button::StepUp => change_step(|editor, i, s| Some(editor.move_step(i, s, Move::Up))),
        Button::StepDown => change_step(|editor, i, s| Some(editor.move_step(i, s, Move::Down))),
        Button::StepRemove => change_step(|editor, i, s| editor.remove_step(i, s)),
        Button::StepAdd => {
            let Some(id) = recipes::chosen_step(hwnd) else {
                return;
            };
            let to = edit(hwnd, |editor| editor.add_step(in_range(editor)?, id)).flatten();
            if let Some(to) = to {
                recipes::select_step(hwnd, to);
            }
            refresh(hwnd);
        }
        Button::PickColor => pick_appearance_color(hwnd),
    }
}

/// 「レシピ」タブで選んでいる見た目を指定する操作の項目 `item` を `input` にする。
fn set_appearance(hwnd: HWND, item: AppearanceItem, input: Option<AppearanceInput>) {
    let (Some(i), Some(s), Some(input)) =
        (recipes::selected(hwnd), recipes::selected_step(hwnd), input)
    else {
        return;
    };
    edit(hwnd, |editor| editor.set_appearance(i, s, item, input));
}

/// 「レシピ」タブの「選ぶ…」: 色の画面を開いて、選んだ色を見た目を指定する操作の枠の色にする。
/// 初めの色は欄の色、欄が空ならスクラップの枠の色。
fn pick_appearance_color(hwnd: HWND) {
    let (Some(i), Some(s)) = (recipes::selected(hwnd), recipes::selected_step(hwnd)) else {
        return;
    };
    let current = with_app(|app| {
        let editor = &app.settings_gui.as_ref()?.editor;
        let typed = match editor.appearance(i, s, AppearanceItem::BorderColor)? {
            AppearanceInput::Text(typed) => typed.parse().ok(),
            AppearanceInput::Switch(_) => None,
        };
        Some(typed.unwrap_or(editor.draft().scrap.border.color))
    })
    .flatten();
    // 色の画面はモーダルで回るので、App を借りたまま開かない
    let Some(current) = current else {
        return;
    };
    if let Some(color) = fields::pick_color(hwnd, current) {
        set_appearance(
            hwnd,
            AppearanceItem::BorderColor,
            Some(AppearanceInput::Text(color.to_string())),
        );
    }
}

/// 「定型領域」タブの一覧の下のボタン。写しを変えて、移った行を選ぶ。足した・写した行は名前の欄にフォーカスを置く。
fn on_region_list_button(hwnd: HWND, button: ListButton) {
    let selected = regions::selected(hwnd);
    let target = match button {
        ListButton::Add => {
            let monitors = with_app(|app| {
                app.settings_gui
                    .as_ref()
                    .map(|gui| gui.env.monitors.clone())
            })
            .flatten()
            .unwrap_or_default();
            let Some((monitor, rect)) = regions::new_region_place(&monitors) else {
                log::warn!("no monitor to add a region on");
                return;
            };
            edit(hwnd, |editor| {
                Some(editor.add_region(text("region.default_name"), &monitor, rect))
            })
        }
        ListButton::Duplicate => edit(hwnd, |editor| {
            let i = selected?;
            let name = format!(
                "{}{}",
                editor.draft().regions.get(i)?.name,
                text("settings_gui.regions.copy_suffix")
            );
            Some(editor.duplicate_region(i, &name))
        }),
        ListButton::Remove => edit(hwnd, |editor| {
            let i = selected.filter(|&i| i < editor.draft().regions.len())?;
            editor.remove_region(i)
        }),
        ListButton::Up | ListButton::Down => edit(hwnd, |editor| {
            let i = selected.filter(|&i| i < editor.draft().regions.len())?;
            let to = if button == ListButton::Up {
                Move::Up
            } else {
                Move::Down
            };
            Some(editor.move_region(i, to))
        }),
    }
    .flatten();
    let Some(index) = target else {
        return;
    };
    regions::select(hwnd, index);
    if matches!(button, ListButton::Add | ListButton::Duplicate) {
        set_focus(hwnd, regions::name_id());
    }
}

/// 「範囲を選び直す」: ダイアログを隠して、範囲選択の画面を登録のモードで開く。
fn on_reselect(hwnd: HWND) {
    let Some(index) = regions::selected(hwnd) else {
        return;
    };
    hide_for_capture(hwnd);
    if !super::request_reselect(index) {
        log::info!("reselecting a region while another capture is in progress; ignored");
        capture_done();
    }
}

/// 範囲選択の画面で、写しの定型領域 `index` の範囲を選び直した（`rect` はスクリーン座標）。写しのモニタと範囲を
/// 置き換えて欄に出す。ファイルに書くのは適用のとき。
pub(super) fn region_reselected(index: usize, rect: Rect) {
    let monitors = monitor_list::list();
    let Some((monitor, relative)) = monitors::relative_to(&monitors, rect) else {
        log::warn!("no monitor contains the reselected range {rect:?}");
        return;
    };
    let id = monitors[monitor].id.as_str();
    let Some(hwnd) = with_app(|app| {
        let gui = app.settings_gui.as_mut()?;
        gui.editor.set_region_range(index, id, relative);
        Some(gui.hwnd)
    })
    .flatten() else {
        return;
    };
    log::info!(
        "region {} reselected on monitor {id} at {relative:?}",
        index + 1
    );
    refresh_monitors();
    refresh(hwnd);
}

/// 「試しに撮る」: 選んでいる写しの定型領域の値で撮る。ダイアログは撮り終えるまで隠す。
fn on_test_capture(hwnd: HWND) {
    let Some(index) = regions::selected(hwnd) else {
        return;
    };
    let Some((region, general_dir)) = with_app(|app| {
        app.settings_gui.as_ref().and_then(|gui| {
            let draft = gui.editor.draft();
            let region = draft.regions.get(index).cloned()?;
            let general_dir = draft.general.save_dir.clone();
            Some((region, general_dir.or_else(|| gui.env.default_dir.clone())))
        })
    })
    .flatten() else {
        return;
    };
    // 領域に保存先がなければ、写しの全体の保存先（それもなければピクチャ）に保存する。欄のプレビューと同じ
    hide_for_capture(hwnd);
    if !super::request_trial(&region, general_dir) {
        log::info!("trial capture while another capture is in progress; ignored");
        capture_done();
    }
}

/// 撮る前にダイアログを隠す。消える動きを止め、隠れた画面が合成されるのを待つ（写り込まないように）。
fn hide_for_capture(hwnd: HWND) {
    let disable = BOOL::from(true);
    // SAFETY: 自分のダイアログ。値は呼び出しの間生きている
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &disable as *const BOOL as *const core::ffi::c_void,
            size_of::<BOOL>() as u32,
        );
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = DwmFlush();
    }
    HIDDEN.set(Some(hwnd));
}

/// 範囲選択の画面を閉じたか、撮り終えた。「範囲を選び直す」「試しに撮る」で隠したダイアログを出し直す。
pub(super) fn capture_done() {
    let Some(hwnd) = HIDDEN.take() else {
        return;
    };
    if DIALOG.get() != Some(hwnd) {
        return;
    }
    // SAFETY: 自分のダイアログ
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// 「スクラップ操作」タブの一覧で行を選んだ。ショートカットの欄をその行の操作に結び直す。
fn on_scrap_op_selected(hwnd: HWND) {
    let Some(row) = scrap_ops::selected(hwnd) else {
        return;
    };
    let field = scrap_ops::field_id();
    with_app(|app| {
        if let Some(gui) = app.settings_gui.as_mut() {
            gui.rebind_chord_row(field, SettingsItem::Shortcut(row.op.id.to_owned()));
        }
    });
    refresh(hwnd);
}

/// 写しを適用する。書けたら（書くものがなかったときも）、登録できなかったホットキーを返す（OK で閉じてよい）。
///
/// その時点のファイルを読み、GUI で変えた項目だけを当てる（3 者の突き合わせ、`settings::merge`）。外と GUI の両方で
/// 変えた項目があれば、`App` を借りずに確かめ、上書きを選ばれたら読み直してもう一度突き合わせる（確かめている間に
/// 新しい衝突が増えていれば、また確かめる）。
fn apply(hwnd: HWND) -> Option<Vec<hotkey::Failed>> {
    let mut confirmed: Vec<Field> = Vec::new();
    let result = loop {
        match with_app(|app| apply_merged(app, &confirmed)).flatten() {
            Some(Step::Confirm(conflicts)) => {
                // MessageBox の間は借りない
                if !confirm_overwrite(hwnd, &conflicts) {
                    log::info!("applying the settings GUI cancelled: {conflicts:?}");
                    break None;
                }
                confirmed = conflicts;
            }
            Some(Step::Done(result)) => break Some(result),
            None => break None,
        }
    };
    // SAFETY: 自分のダイアログに送る
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_SETTINGS_FOCUS, WPARAM(0), LPARAM(0));
    }
    refresh(hwnd);
    if matches!(result, Some(Ok(_))) {
        // 正本は設定ファイル。適用したら、変わっていなくても Run キーを合わせ直す
        sync_autostart();
    }
    match result {
        Some(Ok(failed)) => Some(failed),
        Some(Err(e)) => {
            log::error!("could not write the settings: {e:?}");
            notify_user(
                notify::Level::Error,
                &e.notice(i18n::catalog(language()), "settings_gui.write_failed"),
            );
            None
        }
        None => None,
    }
}

/// 適用の 1 回分の結果。
enum Step {
    /// 確かめていない衝突がある。
    Confirm(Vec<Field>),
    /// 書いた（か、書くものがなかった）。Ok は登録できなかったホットキー。
    Done(Result<Vec<hotkey::Failed>, settings::WriteFailure>),
}

/// ファイルを読んで写しと突き合わせ、衝突が `confirmed` に収まっていれば書いて反映する。
fn apply_merged(app: &mut App, confirmed: &[Field]) -> Option<Step> {
    let gui = app.settings_gui.as_ref()?;
    if !gui.editor.buttons().ok {
        return None;
    }
    let theirs = read_current(app);
    let merged = merge(gui.editor.base(), gui.editor.draft(), &theirs);
    if merged
        .conflicts
        .iter()
        .any(|field| !confirmed.contains(field))
    {
        return Some(Step::Confirm(merged.conflicts));
    }
    let new = merged.settings;
    if new == theirs && new == app.settings && app.unregistered.is_empty() {
        if let Some(gui) = app.settings_gui.as_mut() {
            gui.editor.applied(new, []);
        }
        return Some(Step::Done(Ok(Vec::new())));
    }
    let old = app.settings.clone();
    if let Err(e) = write_settings(app, &new) {
        return Some(Step::Done(Err(e)));
    }
    // 適用では登録し直して結果を見る。欄にフォーカスが残っていれば、`check_focus` で外し直す
    app.hotkey_pause.reregistering();
    let (_, failed) = adopt_settings(app, &old, true);
    let unregistered = app.unregistered.clone();
    let applied = app.settings.clone();
    if let Some(gui) = app.settings_gui.as_mut() {
        gui.editor.applied(applied, unregistered);
    }
    log::info!("settings applied from the GUI");
    Some(Step::Done(Ok(failed)))
}

/// 突き合わせに使う、今のファイルの設定。ファイルがなければ既定値。読めないか TOML でなければ、常駐側の設定
/// （直前の有効な設定）を使う（そのときは `write_settings` が書くのを断る）。
fn read_current(app: &App) -> Settings {
    let Some(path) = app.settings_path.as_deref() else {
        return app.settings.clone();
    };
    match std::fs::read_to_string(path) {
        Ok(src) => settings::reload(&app.settings, Some(&src)).0,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            settings::reload(&app.settings, None).0
        }
        Err(_) => app.settings.clone(),
    }
}

/// 外と GUI の両方で変えた項目 `conflicts` を見せ、「GUI の値で上書き」なら true、「適用をやめる」なら false。
/// モーダルなので `App` を借りずに呼ぶ。
fn confirm_overwrite(hwnd: HWND, conflicts: &[Field]) -> bool {
    let items: Vec<String> = conflicts.iter().map(|f| format!("  {f}")).collect();
    let body = to_wide(&format!(
        "{}\n\n{}\n\n{}",
        text("settings_gui.conflict.body"),
        items.join("\n"),
        text("settings_gui.conflict.choices"),
    ));
    let title = to_wide(text("settings_gui.title"));
    // SAFETY: 文字列はこの呼び出しの間生きている。持ち主は自分のダイアログ
    let answer = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(body.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    };
    answer == IDOK
}

/// キーの組み合わせの欄にフォーカスがあれば全体のホットキーの登録を外し、なければ登録し直す。
fn check_focus() {
    // SAFETY: 引数なし
    let focus = unsafe { GetFocus() };
    let in_field = chord_field::is_field(focus);
    with_app(|app| {
        let change = app.hotkey_pause.focus_changed(in_field);
        pause_hotkeys(app, change);
    });
}

fn pause_hotkeys(app: &mut App, change: Option<PauseChange>) {
    match change {
        Some(PauseChange::Pause) => {
            app.hotkeys.reregister(&BTreeMap::new());
            log::info!("hotkeys suspended while a chord field has focus");
        }
        Some(PauseChange::Resume) => {
            let failed = register_hotkeys(app);
            log_failed(&failed);
            log::info!("hotkeys registered again");
        }
        None => {}
    }
}

/// ダイアログが壊れた。範囲選択キャプチャのホットキーが未設定のまま（キャンセルで）閉じたら、通知で知らせる。
fn closed() {
    DIALOG.set(None);
    let notice = with_app(|app| {
        app.settings_gui = None;
        let change = app.hotkey_pause.closed();
        pause_hotkeys(app, change);
        if unset::prompt(unset::When::GuiClosed, &app.settings) == Some(unset::Prompt::Notify) {
            settings_notice(Moment::Reload, &[], &[], true, app.settings_path.as_deref())
        } else {
            None
        }
    })
    .flatten();
    log::info!("settings GUI closed");
    if let Some((level, body)) = notice {
        notify_user(level, &body);
    }
}
