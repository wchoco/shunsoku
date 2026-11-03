//! 設定 GUI（`settings_dialog`）と `App` をつなぐ。
//!
//! - 開くのは `open`（トレイの「設定…」、2 つ目のプロセスの登録メッセージ）。開いていれば前に出すだけ。ダイアログは
//!   `App` を借りずに作り（作る間にダイアログ プロシージャが呼ばれるため）、できたら `App::settings_gui` に置く。
//! - 写し（core の `Editor`）は `App::settings_gui` が持つ。欄に出すとき（`refresh`）は写しを写してから、借りずに出す。
//! - 欄にキーが入ったら（`EN_CHANGE`）写しの項目を変え、印とボタンを出し直す。ほかの欄（`settings_dialog::fields`）
//!   も同じく、チェック・入力・選択のたびに写しを変えて出し直す（`on_field`）。欄の横のボタン（「参照…」「選ぶ…」
//!   「変数を挿入」）はモーダルなので、`App` を借りずに出す（`on_button`）。
//! - キーの組み合わせの欄にフォーカスがある間は、全体のホットキーの登録を外す（`App::hotkeys_suspended`）。欄の
//!   フォーカスの出入りで `WM_APP_SETTINGS_FOCUS` を自分に送り、届いた時点のフォーカスで決める（`check_focus`）。
//! - 適用（`apply`）: 写しを書き（`write_settings`）、読み込み直しと同じ反映をし（`adopt_settings`）、ホットキーは
//!   必ず登録し直して、登録できなかった項目を写しの印にする（`Editor::applied`）。OK は適用して、全部登録できたら閉じる。
//! - 言語の切り替え（#102）で組み直すときは、`settings_dialog::current_page` と位置を覚えて壊し、
//!   `settings_dialog::create` と `refresh` をし直す（`WM_DESTROY` で `App::settings_gui` を消さないようにする）。

use std::cell::Cell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use shunsoku_core::settings::editor::Editor;
use windows::Win32::Foundation::RECT;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{EM_GETSEL, EM_SETSEL};
use windows::Win32::UI::Controls::{NMHDR, TCN_SELCHANGE};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, CBN_SELCHANGE, DestroyWindow, EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS, GetWindowRect,
    IDCANCEL, IDOK, IsDialogMessageW, IsIconic, MSG, PostMessageW, PostQuitMessage, SW_RESTORE,
    SW_SHOW, SendMessageW, SetForegroundWindow, ShowWindow, WM_COMMAND, WM_DESTROY, WM_INITDIALOG,
    WM_NEXTDLGCTL, WM_NOTIFY, WM_SETTINGCHANGE,
};
use windows::core::PCWSTR;

use super::{
    App, EXIT_PANIC, WM_APP_SETTINGS_FOCUS, adopt_settings, log_failed, notify_user,
    register_hotkeys, text, with_app, write_settings,
};
use crate::settings_dialog::fields::{self, Action, Field, Fields, Input, Kind};
use crate::settings_dialog::{self, ChordRow, ID_APPLY, ID_TAB, Page, chord_field};
use crate::{menu, notify, panic_hook, save};

/// 開いている設定 GUI。
pub(super) struct SettingsGui {
    hwnd: HWND,
    rows: Vec<ChordRow>,
    fields: Fields,
    editor: Editor,
}

thread_local! {
    /// 開いている設定 GUI のウィンドウ。メッセージ ループの `IsDialogMessageW` に渡す（`App` を借りずに読む）。
    static DIALOG: Cell<Option<HWND>> = const { Cell::new(None) };
}

/// 設定 GUI へのメッセージ（Tab の移動など）なら処理して true。
pub(super) fn is_dialog_message(msg: &MSG) -> bool {
    let Some(dialog) = DIALOG.get() else {
        return false;
    };
    // SAFETY: msg はこの呼び出しの間生きている
    unsafe { IsDialogMessageW(dialog, msg) }.as_bool()
}

/// 設定 GUI を開く（`page` が None なら最初のタブ）。開いていれば前に出す。
pub(super) fn open(page: Option<Page>) {
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
        return;
    }
    // ダイアログを作る間にプロシージャが呼ばれるので、借りずに作る
    let dialog =
        match settings_dialog::create(module, page.unwrap_or_default(), text, Some(dialog_proc)) {
            Ok(dialog) => dialog,
            Err(e) => {
                log::error!("could not open the settings GUI: {e}");
                return;
            }
        };
    let hwnd = dialog.hwnd;
    let mut editor = Editor::new(settings.clone());
    // 起動時などに登録できなかったホットキーも印にする
    editor.applied(settings, unregistered);
    let gui = SettingsGui {
        hwnd,
        rows: dialog.rows,
        fields: dialog.fields,
        editor,
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
    // SAFETY: 自分のダイアログ。フォーカスはタブに置く（開いただけでホットキーを外さない）
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        if let Some(tab) = settings_dialog::item(hwnd, ID_TAB) {
            SendMessageW(
                hwnd,
                WM_NEXTDLGCTL,
                Some(WPARAM(tab.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }
    log::info!("settings GUI opened");
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

/// 写しを欄に出す。
fn refresh(hwnd: HWND) {
    let snapshot = with_app(|app| {
        app.settings_gui
            .as_ref()
            .map(|gui| (gui.rows.clone(), gui.fields.clone(), gui.editor.clone()))
    })
    .flatten();
    if let Some((rows, fields, editor)) = snapshot {
        // コントロールにメッセージを送るので、借りずに出す
        settings_dialog::fill(hwnd, &rows, &editor, text);
        fields::fill(hwnd, &fields, &editor, text);
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
            if header.idFrom == ID_TAB as usize && header.code == TCN_SELCHANGE {
                settings_dialog::show_page(hwnd, settings_dialog::current_page(hwnd));
            }
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
        WM_DESTROY => {
            closed();
            false
        }
        _ => false,
    }
}

fn on_command(hwnd: HWND, id: i32, code: u32, control: HWND) {
    if id == IDOK.0 {
        if code == BN_CLICKED && apply(hwnd) {
            destroy(hwnd);
        }
        return;
    }
    match id {
        _ if id == IDCANCEL.0 => destroy(hwnd),
        ID_APPLY if code == BN_CLICKED => {
            apply(hwnd);
        }
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
    let changed = match (row.kind, code) {
        (Kind::Check, BN_CLICKED)
        | (Kind::Text { .. } | Kind::Number { .. }, EN_CHANGE)
        | (Kind::Choice, CBN_SELCHANGE) => true,
        // 読めない入力は、フォーカスが外れたら写しの値に戻す
        (Kind::Text { .. } | Kind::Number { .. }, EN_KILLFOCUS) => false,
        _ => return,
    };
    if changed {
        set_field(row.field, fields::input(row, control));
    }
    refresh(hwnd);
}

/// 写しの項目 `field` を `input` にする。
fn set_field(field: Field, input: Input) {
    with_app(|app| {
        if let Some(gui) = app.settings_gui.as_mut() {
            fields::set(field, gui.editor.draft_mut(), input);
        }
    });
}

/// 欄の横のボタンが押された。ダイアログとメニューはモーダルなので、`App` を借りずに出す。
fn on_button(hwnd: HWND, fields: &Fields, action: Action) {
    let Some(draft) = with_app(|app| {
        app.settings_gui
            .as_ref()
            .map(|gui| gui.editor.draft().clone())
    })
    .flatten() else {
        return;
    };
    match action {
        Action::BrowseFolder(field) => {
            match save::ask_folder(hwnd, &fields::folder_of(&draft, field)) {
                Ok(Some(dir)) => set_field(field, Input::Text(dir.to_string_lossy().into_owned())),
                Ok(None) => {}
                Err(e) => log::error!("could not show the folder dialog: {e}"),
            }
        }
        Action::PickColor(field) => {
            let Some(current) = fields::color_of(&draft, field) else {
                return;
            };
            if let Some(color) = fields::pick_color(hwnd, current) {
                set_field(field, Input::Text(color.to_string()));
            }
        }
        Action::InsertVariable(of) => insert_variable(hwnd, fields, of),
    }
    refresh(hwnd);
}

/// 変数のメニューを「変数を挿入」のボタンの下に出し、選ばれた変数をテンプレートの欄の選択に置き換える。
fn insert_variable(hwnd: HWND, fields: &Fields, of: fields::TemplateOf) {
    let Some(edit) = fields
        .row_of(Field::Template(of))
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
        .map(|(v, id)| menu::Entry::item(id, format!("{}\t{}", text(v.label_key), v.token), true))
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

/// 写しを適用する。適用でき、ホットキーを全部登録できたら true（OK で閉じてよい）。
fn apply(hwnd: HWND) -> bool {
    let result = with_app(|app| {
        let gui = app.settings_gui.as_ref()?;
        if !gui.editor.buttons().ok {
            return None;
        }
        let new = gui.editor.draft().clone();
        if new == app.settings && app.unregistered.is_empty() {
            return Some(Ok(true));
        }
        let old = app.settings.clone();
        if let Err(e) = write_settings(app, &new) {
            return Some(Err(e));
        }
        // 適用では登録し直して結果を見る。欄にフォーカスが残っていれば、`check_focus` で外し直す
        app.hotkeys_suspended = false;
        let (_, failed) = adopt_settings(app, &old, true);
        let unregistered = app.unregistered.clone();
        let applied = app.settings.clone();
        if let Some(gui) = app.settings_gui.as_mut() {
            gui.editor.applied(applied, unregistered);
        }
        log::info!("settings applied from the GUI");
        Some(Ok(failed.is_empty()))
    })
    .flatten();
    // SAFETY: 自分のダイアログに送る
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_SETTINGS_FOCUS, WPARAM(0), LPARAM(0));
    }
    refresh(hwnd);
    match result {
        Some(Ok(all_registered)) => all_registered,
        Some(Err(e)) => {
            log::error!("could not write the settings: {e}");
            notify_user(
                notify::Level::Error,
                &format!("{}\n{e}", text("settings_gui.write_failed")),
            );
            false
        }
        None => false,
    }
}

/// キーの組み合わせの欄にフォーカスがあれば全体のホットキーの登録を外し、なければ登録し直す。
fn check_focus() {
    // SAFETY: 引数なし
    let focus = unsafe { GetFocus() };
    let in_field = chord_field::is_field(focus);
    with_app(|app| {
        if in_field && !app.hotkeys_suspended {
            app.hotkeys.reregister(&BTreeMap::new());
            app.hotkeys_suspended = true;
            log::info!("hotkeys suspended while a chord field has focus");
        } else if !in_field && app.hotkeys_suspended {
            resume_hotkeys(app);
        }
    });
}

fn resume_hotkeys(app: &mut App) {
    app.hotkeys_suspended = false;
    let failed = register_hotkeys(app);
    log_failed(&failed);
    log::info!("hotkeys registered again");
}

/// ダイアログが壊れた。
fn closed() {
    DIALOG.set(None);
    with_app(|app| {
        app.settings_gui = None;
        if app.hotkeys_suspended {
            resume_hotkeys(app);
        }
    });
    log::info!("settings GUI closed");
}
