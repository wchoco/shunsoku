//! 設定 GUI（`settings_dialog`）と `App` をつなぐ。
//!
//! - 開くのは `open`（トレイの「設定…」、2 つ目のプロセスの登録メッセージ）と、範囲選択キャプチャのホットキーが
//!   未設定のまま起動したときの `open_for_unset_hotkey`（その欄にフォーカスを置く）。開いていれば前に出すだけ。
//!   未設定のまま閉じたら（キャンセル）、通知で知らせる（`closed`）。ダイアログは
//!   `App` を借りずに作り（作る間にダイアログ プロシージャが呼ばれるため）、できたら `App::settings_gui` に置く。
//! - 写し（core の `Editor`）は `App::settings_gui` が持つ。欄に出すとき（`refresh`）は写しを写してから、借りずに出す。
//! - 欄にキーが入ったら（`EN_CHANGE`）写しの項目を変え、印とボタンを出し直す。
//! - キーの組み合わせの欄にフォーカスがある間は、全体のホットキーの登録を外す（`App::hotkeys_suspended`）。欄の
//!   フォーカスの出入りで `WM_APP_SETTINGS_FOCUS` を自分に送り、届いた時点のフォーカスで決める（`check_focus`）。
//! - 適用（`apply`）: その時点のファイルと写しを 3 者で突き合わせ（`settings::merge`）、両方で変えた項目があれば
//!   借りずに確かめてから、結果を書き（`write_settings`）、読み込み直しと同じ反映をし（`adopt_settings`）、ホットキーは
//!   必ず登録し直して、登録できなかった項目を写しの印にする（`Editor::applied`）。OK は適用して、全部登録できたら閉じる。
//! - 言語の切り替え（#102）で組み直すときは、`settings_dialog::current_page` と位置を覚えて壊し、
//!   `settings_dialog::create` と `refresh` をし直す（`WM_DESTROY` で `App::settings_gui` を消さないようにする）。

use std::cell::Cell;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use shunsoku_core::settings::editor::Editor;
use shunsoku_core::settings::merge::{Field, merge};
use shunsoku_core::settings::{self, Settings, SettingsItem, unset};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{NMHDR, TCN_SELCHANGE};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, DestroyWindow, EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS, IDCANCEL, IDOK,
    IsDialogMessageW, IsIconic, MB_DEFBUTTON2, MB_ICONWARNING, MB_OKCANCEL, MSG, MessageBoxW,
    PostMessageW, PostQuitMessage, SW_RESTORE, SW_SHOW, SendMessageW, SetForegroundWindow,
    ShowWindow, WM_COMMAND, WM_DESTROY, WM_INITDIALOG, WM_NEXTDLGCTL, WM_NOTIFY, WM_SETTINGCHANGE,
};
use windows::core::PCWSTR;

use super::{
    App, EXIT_PANIC, Moment, WM_APP_SETTINGS_FOCUS, adopt_settings, log_failed, notify_user,
    register_hotkeys, settings_notice, text, with_app, write_settings,
};
use crate::settings_dialog::{self, ChordRow, ID_APPLY, ID_TAB, Page, chord_field};
use crate::wide::to_wide;
use crate::{notify, panic_hook};

/// 開いている設定 GUI。
pub(super) struct SettingsGui {
    hwnd: HWND,
    rows: Vec<ChordRow>,
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
    open_with(page, None);
}

/// 範囲選択キャプチャのホットキーが未設定のまま起動した: 「キャプチャ」タブで開き、その欄にフォーカスを置く。
pub(super) fn open_for_unset_hotkey() {
    open_with(Some(Page::Capture), Some(SettingsItem::SelectCaptureHotkey));
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
    let focus = focus
        .and_then(|item| dialog.rows.iter().find(|row| row.item == item))
        .map_or(ID_TAB, |row| row.field);
    let mut editor = Editor::new(settings.clone());
    // 起動時などに登録できなかったホットキーも印にする
    editor.applied(settings, unregistered);
    let gui = SettingsGui {
        hwnd,
        rows: dialog.rows,
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
            .map(|gui| (gui.rows.clone(), gui.editor.clone()))
    })
    .flatten();
    if let Some((rows, editor)) = snapshot {
        // コントロールにメッセージを送るので、借りずに出す
        settings_dialog::fill(hwnd, &rows, &editor, text);
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
        _ => {}
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
///
/// その時点のファイルを読み、GUI で変えた項目だけを当てる（3 者の突き合わせ、`settings::merge`）。外と GUI の両方で
/// 変えた項目があれば、`App` を借りずに確かめ、上書きを選ばれたら読み直してもう一度突き合わせる（確かめている間に
/// 新しい衝突が増えていれば、また確かめる）。
fn apply(hwnd: HWND) -> bool {
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

/// 適用の 1 回分の結果。
enum Step {
    /// 確かめていない衝突がある。
    Confirm(Vec<Field>),
    /// 書いた（か、書くものがなかった）。ホットキーを全部登録できたら Ok(true)。
    Done(Result<bool, String>),
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
        return Some(Step::Done(Ok(true)));
    }
    let old = app.settings.clone();
    if let Err(e) = write_settings(app, &new) {
        return Some(Step::Done(Err(e)));
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
    Some(Step::Done(Ok(failed.is_empty())))
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

/// ダイアログが壊れた。範囲選択キャプチャのホットキーが未設定のまま（キャンセルで）閉じたら、通知で知らせる。
fn closed() {
    DIALOG.set(None);
    let notice = with_app(|app| {
        app.settings_gui = None;
        if app.hotkeys_suspended {
            resume_hotkeys(app);
        }
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
