//! Throwaway prototype for ADR-0004 / issue #38.
//!
//! A modeless dialog from a `DIALOGEX` resource (`settings.rc`) created with
//! `CreateDialogParamW`, Per-Monitor V2 (via `app.manifest`), dark title bar via
//! `DWMWA_USE_IMMERSIVE_DARK_MODE`, and a subclassed hot key control
//! (`msctls_hotkey32`) that asks the dialog manager for every key.
//!
//! Two switches exist so that the manual check can compare approaches:
//! - DPI handling: "auto" lets the dialog manager rescale on `WM_DPICHANGED`
//!   (PMv2 default for dialogs); "manual" disables it with
//!   `SetDialogDpiChangeBehavior` and resizes to the suggested rect, re-creates the
//!   font and re-lays out the children itself. Toggled by re-opening the dialog.
//! - Hot key input: by default the subclass only answers `WM_GETDLGCODE` with
//!   `DLGC_WANTALLKEYS` (the ADR-0004 plan) and lets the control handle keys.
//!   With the checkbox on, it also intercepts `WM_KEYDOWN`/`WM_SYSKEYDOWN` and
//!   sets the value with `HKM_SETHOTKEY` itself.
#![windows_subsystem = "windows"]

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DeleteObject, FW_NORMAL,
    HFONT, InvalidateRect, MapWindowPoints, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WindowsProgramming::MulDiv;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, CheckDlgButton, HKM_GETHOTKEY, HKM_SETHOTKEY, HOTKEYF_ALT,
    HOTKEYF_CONTROL, HOTKEYF_EXT, HOTKEYF_SHIFT, ICC_HOTKEY_CLASS, ICC_STANDARD_CLASSES,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, IsDlgButtonChecked,
};
use windows::Win32::UI::HiDpi::{
    DCDC_DISABLE_FONT_UPDATE, DCDC_DISABLE_RELAYOUT, DDC_DISABLE_ALL,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetDialogControlDpiChangeBehavior,
    SetDialogDpiChangeBehavior, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, GetKeyState, MAPVK_VK_TO_VSC, MapVirtualKeyW, SetFocus, VK_CONTROL,
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CB_ADDSTRING, CB_GETDROPPEDCONTROLRECT, CB_SETCURSEL, CreateDialogParamW, DLGC_WANTALLKEYS,
    DestroyWindow, DispatchMessageW, EN_CHANGE, GW_CHILD, GW_HWNDNEXT, GetClientRect, GetDlgItem,
    GetMessageW, GetParent, GetWindow, GetWindowRect, IDCANCEL, IDOK, IsDialogMessageW,
    LB_DELETESTRING, LB_GETCOUNT, LB_INSERTSTRING, MSG, MoveWindow, PostMessageW, PostQuitMessage,
    SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetDlgItemTextW, SetWindowPos, ShowWindow, TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_DESTROY, WM_DPICHANGED, WM_GETDLGCODE, WM_INITDIALOG, WM_KEYDOWN, WM_KEYUP, WM_NCDESTROY,
    WM_SETFONT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::core::{BOOL, HSTRING, PCWSTR, w};

// Keep in sync with settings.rc.
const IDD_SETTINGS: usize = 101;
const IDC_HOTKEY: i32 = 1001;
const IDC_VALUE: i32 = 1002;
const IDC_DPI: i32 = 1003;
const IDC_STATUS: i32 = 1004;
const IDC_LOG: i32 = 1005;
const IDC_INTERCEPT: i32 = 1006;
const IDC_DARK: i32 = 1007;
const IDC_CLEAR: i32 = 1008;
const IDC_REOPEN: i32 = 1009;
const IDC_CLOSE: i32 = 1010;
const IDC_COMBO: i32 = 1011;

const WM_APP_REFRESH: u32 = WM_APP + 1;
const SUBCLASS_ID: usize = 1;
const FONT_PT: i32 = 9; // same as FONT in settings.rc
const LOG_MAX: usize = 200;

thread_local! {
    /// true: handle WM_DPICHANGED ourselves; false: let the dialog manager do it.
    static MANUAL_DPI: Cell<bool> = const { Cell::new(false) };
    static DARK: Cell<bool> = const { Cell::new(true) };
    static INTERCEPT: Cell<bool> = const { Cell::new(false) };
    /// Set before DestroyWindow when the dialog should be re-created (mode switch).
    static REOPEN: Cell<bool> = const { Cell::new(false) };
    /// Top-left to restore after a re-open.
    static POS: Cell<Option<POINT>> = const { Cell::new(None) };
    /// Font we created in manual mode (the template font is owned by the dialog).
    static FONT: Cell<Option<HFONT>> = const { Cell::new(None) };
    /// Manual mode: DPI at creation and each child's rect (client coords) at that DPI.
    static BASE: RefCell<(u32, Vec<(HWND, RECT)>)> = const { RefCell::new((96, Vec::new())) };
}

fn main() -> windows::core::Result<()> {
    unsafe {
        // The manifest already sets PMv2; this fails with ACCESS_DENIED then. Belt and braces.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_HOTKEY_CLASS | ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();

        loop {
            REOPEN.set(false);
            let dlg = CreateDialogParamW(
                Some(hinst),
                PCWSTR(IDD_SETTINGS as *const u16),
                None,
                Some(dlg_proc),
                LPARAM(0),
            )?;
            let _ = ShowWindow(dlg, SW_SHOW);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                if !IsDialogMessageW(dlg, &msg).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if !REOPEN.get() {
                break;
            }
        }
    }
    Ok(())
}

unsafe extern "system" fn dlg_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                on_init(hwnd);
                0 // focus was set explicitly
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                on_command(hwnd, id, code);
                1
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xFFFF) as u32;
                let rc = &*(lparam.0 as *const RECT);
                log(
                    hwnd,
                    &format!(
                        "WM_DPICHANGED dpi={dpi} 提案 {}x{} ({})",
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        if MANUAL_DPI.get() {
                            "手動で処理"
                        } else {
                            "ダイアログマネージャに任せる"
                        }
                    ),
                );
                if MANUAL_DPI.get() {
                    on_dpi_manual(hwnd, dpi, rc);
                    1
                } else {
                    // Default processing runs after we return; refresh the label afterwards.
                    let _ = PostMessageW(Some(hwnd), WM_APP_REFRESH, WPARAM(0), LPARAM(0));
                    0
                }
            }
            WM_APP_REFRESH => {
                update_dpi_label(hwnd);
                1
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                1
            }
            WM_DESTROY => {
                if let Some(f) = FONT.take() {
                    let _ = DeleteObject(f.into());
                }
                PostQuitMessage(0);
                1
            }
            _ => 0,
        }
    }
}

unsafe fn on_init(hwnd: HWND) {
    unsafe {
        let manual = MANUAL_DPI.get();
        if manual {
            // Turn off the dialog manager's automatic PMv2 rescaling for the dialog and children.
            let _ = SetDialogDpiChangeBehavior(hwnd, DDC_DISABLE_ALL, DDC_DISABLE_ALL);
            let both = DCDC_DISABLE_FONT_UPDATE | DCDC_DISABLE_RELAYOUT;
            for child in children(hwnd) {
                let _ = SetDialogControlDpiChangeBehavior(child, both, both);
            }
            record_base(hwnd);
        }
        if let Some(p) = POS.get() {
            let _ = SetWindowPos(
                hwnd,
                None,
                p.x,
                p.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        apply_dark(hwnd);

        let combo = item(hwnd, IDC_COMBO);
        for s in [
            "（なし）",
            "中ボタン",
            "X1 ボタン（戻る）",
            "X2 ボタン（進む）",
        ] {
            let h = HSTRING::from(s);
            SendMessageW(combo, CB_ADDSTRING, None, Some(LPARAM(h.as_ptr() as isize)));
        }
        SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(0)), None);

        let _ = CheckDlgButton(
            hwnd,
            IDC_INTERCEPT,
            if INTERCEPT.get() {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            },
        );
        let hk = item(hwnd, IDC_HOTKEY);
        let _ = SetWindowSubclass(hk, Some(hotkey_subclass), SUBCLASS_ID, 0);
        let _ = SetFocus(Some(hk));

        refresh_value(hwnd);
        update_dpi_label(hwnd);
        status(
            hwnd,
            &format!(
                "起動しました。DPI 処理: {}",
                if manual { "手動" } else { "自動" }
            ),
        );
    }
}

unsafe fn on_command(hwnd: HWND, id: i32, code: u32) {
    unsafe {
        match id {
            IDC_HOTKEY if code == EN_CHANGE => refresh_value(hwnd),
            IDC_INTERCEPT => {
                let on = IsDlgButtonChecked(hwnd, IDC_INTERCEPT) == BST_CHECKED.0;
                INTERCEPT.set(on);
                status(
                    hwnd,
                    if on {
                        "キー入力: サブクラスで横取り（HKM_SETHOTKEY で値を設定）"
                    } else {
                        "キー入力: WM_GETDLGCODE だけ上書き、キーは標準の処理"
                    },
                );
                let _ = SetFocus(Some(item(hwnd, IDC_HOTKEY)));
            }
            IDC_CLEAR => {
                SendMessageW(item(hwnd, IDC_HOTKEY), HKM_SETHOTKEY, Some(WPARAM(0)), None);
                refresh_value(hwnd);
                let _ = SetFocus(Some(item(hwnd, IDC_HOTKEY)));
            }
            IDC_DARK => {
                DARK.set(!DARK.get());
                apply_dark(hwnd);
                update_dpi_label(hwnd);
            }
            IDC_REOPEN => {
                let mut r = RECT::default();
                let _ = GetWindowRect(hwnd, &mut r);
                POS.set(Some(POINT {
                    x: r.left,
                    y: r.top,
                }));
                MANUAL_DPI.set(!MANUAL_DPI.get());
                REOPEN.set(true);
                let _ = DestroyWindow(hwnd);
            }
            IDC_CLOSE => {
                let _ = DestroyWindow(hwnd);
            }
            // Reaching here means the dialog manager, not the hot key control, got the key.
            x if x == IDOK.0 => {
                log(
                    hwnd,
                    "ダイアログが IDOK を受けた（Enter が入力欄に届いていない）",
                );
                status(hwnd, "IDOK を受けた（Enter が入力欄に届いていない）");
            }
            x if x == IDCANCEL.0 => {
                log(
                    hwnd,
                    "ダイアログが IDCANCEL を受けた（Esc が入力欄に届いていない）",
                );
                status(hwnd, "IDCANCEL を受けた（Esc が入力欄に届いていない）");
            }
            _ => {}
        }
    }
}

/// Subclass of the hot key control.
unsafe extern "system" fn hotkey_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        let dlg = GetParent(hwnd).unwrap_or_default();
        match msg {
            WM_GETDLGCODE => {
                let base = DefSubclassProc(hwnd, msg, wparam, lparam);
                let m = lparam.0 as *const MSG;
                if !m.is_null() {
                    let m = &*m;
                    if (m.message == WM_KEYDOWN || m.message == WM_SYSKEYDOWN)
                        && m.wParam.0 == VK_TAB.0 as usize
                    {
                        // Keep Tab / Shift+Tab for focus navigation.
                        return base;
                    }
                }
                LRESULT(base.0 | DLGC_WANTALLKEYS as isize)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = wparam.0 as u16;
                let name = msg_name(msg);
                let key = key_name_from_lparam(lparam, vk);
                if INTERCEPT.get() {
                    if is_modifier(vk) {
                        log(dlg, &format!("{name} {key} → 修飾キーなので保留"));
                    } else {
                        let mods = current_mods()
                            | if (lparam.0 >> 24) & 1 == 1 {
                                HOTKEYF_EXT
                            } else {
                                0
                            };
                        let v = (vk as usize & 0xFF) | ((mods as usize) << 8);
                        SendMessageW(hwnd, HKM_SETHOTKEY, Some(WPARAM(v)), None);
                        refresh_value(dlg);
                        log(dlg, &format!("{name} {key} → 横取り: {}", value_text(hwnd)));
                    }
                    LRESULT(0)
                } else {
                    let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                    refresh_value(dlg);
                    log(
                        dlg,
                        &format!("{name} {key} → 標準処理後: {}", value_text(hwnd)),
                    );
                    r
                }
            }
            WM_KEYUP | WM_SYSKEYUP => {
                if INTERCEPT.get() {
                    LRESULT(0)
                } else {
                    let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                    refresh_value(dlg);
                    r
                }
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(hotkey_subclass), SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn on_dpi_manual(hwnd: HWND, dpi: u32, suggested: &RECT) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let font = CreateFontW(
            -MulDiv(FONT_PT, dpi as i32, 72),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let (base_dpi, base) = BASE.with_borrow(|b| (b.0, b.1.clone()));
        let s = |v: i32| MulDiv(v, dpi as i32, base_dpi as i32);
        for (child, r) in base {
            let _ = MoveWindow(
                child,
                s(r.left),
                s(r.top),
                s(r.right) - s(r.left),
                s(r.bottom) - s(r.top),
                false,
            );
            SendMessageW(
                child,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
        }
        if let Some(old) = FONT.replace(Some(font)) {
            let _ = DeleteObject(old.into());
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
        update_dpi_label(hwnd);
    }
}

unsafe fn record_base(hwnd: HWND) {
    unsafe {
        let mut v = Vec::new();
        for child in children(hwnd) {
            let mut r = RECT::default();
            let _ = GetWindowRect(child, &mut r);
            if GetDlgItem(Some(hwnd), IDC_COMBO).ok() == Some(child) {
                // GetWindowRect of a combo box is the closed height; keep the drop-down height.
                let mut d = RECT::default();
                SendMessageW(
                    child,
                    CB_GETDROPPEDCONTROLRECT,
                    None,
                    Some(LPARAM(&mut d as *mut RECT as isize)),
                );
                r.bottom = r.top + (d.bottom - d.top);
            }
            let mut pts = [
                POINT {
                    x: r.left,
                    y: r.top,
                },
                POINT {
                    x: r.right,
                    y: r.bottom,
                },
            ];
            MapWindowPoints(None, Some(hwnd), &mut pts);
            v.push((
                child,
                RECT {
                    left: pts[0].x,
                    top: pts[0].y,
                    right: pts[1].x,
                    bottom: pts[1].y,
                },
            ));
        }
        let dpi = GetDpiForWindow(hwnd);
        BASE.set((dpi, v));
    }
}

unsafe fn children(hwnd: HWND) -> Vec<HWND> {
    unsafe {
        let mut v = Vec::new();
        let mut c = GetWindow(hwnd, GW_CHILD);
        while let Ok(h) = c {
            if h.is_invalid() {
                break;
            }
            v.push(h);
            c = GetWindow(h, GW_HWNDNEXT);
        }
        v
    }
}

unsafe fn apply_dark(hwnd: HWND) {
    unsafe {
        let v = BOOL::from(DARK.get());
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &v as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );
        // Ask for a non-client repaint so a toggle shows without re-activation.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

unsafe fn update_dpi_label(hwnd: HWND) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let mut c = RECT::default();
        let _ = GetClientRect(hwnd, &mut c);
        let text = format!(
            "DPI {dpi}（{}%）／ クライアント {}x{}px ／ DPI 処理: {} ／ ダーク: {}",
            dpi * 100 / 96,
            c.right - c.left,
            c.bottom - c.top,
            if MANUAL_DPI.get() { "手動" } else { "自動" },
            if DARK.get() { "オン" } else { "オフ" }
        );
        let _ = SetDlgItemTextW(hwnd, IDC_DPI, &HSTRING::from(text));
    }
}

unsafe fn refresh_value(dlg: HWND) {
    unsafe {
        let text = value_text(item(dlg, IDC_HOTKEY));
        let _ = SetDlgItemTextW(dlg, IDC_VALUE, &HSTRING::from(text));
    }
}

/// Human-readable value of the hot key control (HKM_GETHOTKEY).
unsafe fn value_text(hk: HWND) -> String {
    unsafe {
        let v = SendMessageW(hk, HKM_GETHOTKEY, None, None).0 as u32 & 0xFFFF;
        let vk = (v & 0xFF) as u16;
        let mods = v >> 8;
        if v == 0 {
            return "（なし） [0x0000]".to_string();
        }
        let mut parts = Vec::new();
        if mods & HOTKEYF_CONTROL != 0 {
            parts.push("Ctrl".to_string());
        }
        if mods & HOTKEYF_SHIFT != 0 {
            parts.push("Shift".to_string());
        }
        if mods & HOTKEYF_ALT != 0 {
            parts.push("Alt".to_string());
        }
        parts.push(key_name(vk, mods & HOTKEYF_EXT != 0));
        format!("{} [0x{v:04X}]", parts.join(" + "))
    }
}

unsafe fn key_name(vk: u16, ext: bool) -> String {
    unsafe {
        let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC);
        let lp = (scan << 16) | if ext { 1 << 24 } else { 0 };
        let mut buf = [0u16; 64];
        let n = GetKeyNameTextW(lp as i32, &mut buf);
        let name = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        format!("{name}(vk=0x{vk:02X}{})", if ext { ",ext" } else { "" })
    }
}

unsafe fn key_name_from_lparam(lparam: LPARAM, vk: u16) -> String {
    unsafe {
        let mut buf = [0u16; 64];
        let n = GetKeyNameTextW(lparam.0 as i32, &mut buf);
        let name = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let ext = (lparam.0 >> 24) & 1 == 1;
        format!("{name}(vk=0x{vk:02X}{})", if ext { ",ext" } else { "" })
    }
}

unsafe fn current_mods() -> u32 {
    unsafe {
        let down = |vk: u16| GetKeyState(vk as i32) < 0;
        let mut m = 0;
        if down(VK_CONTROL.0) {
            m |= HOTKEYF_CONTROL;
        }
        if down(VK_SHIFT.0) {
            m |= HOTKEYF_SHIFT;
        }
        if down(VK_MENU.0) {
            m |= HOTKEYF_ALT;
        }
        m
    }
}

fn is_modifier(vk: u16) -> bool {
    [
        VK_SHIFT,
        VK_CONTROL,
        VK_MENU,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
    ]
    .iter()
    .any(|k| k.0 == vk)
}

fn msg_name(msg: u32) -> &'static str {
    match msg {
        WM_KEYDOWN => "WM_KEYDOWN",
        WM_SYSKEYDOWN => "WM_SYSKEYDOWN",
        WM_KEYUP => "WM_KEYUP",
        WM_SYSKEYUP => "WM_SYSKEYUP",
        _ => "?",
    }
}

unsafe fn item(dlg: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(dlg), id).unwrap_or_default() }
}

unsafe fn status(dlg: HWND, text: &str) {
    unsafe {
        let _ = SetDlgItemTextW(dlg, IDC_STATUS, &HSTRING::from(text));
    }
}

/// Prepend a line to the key log list box.
unsafe fn log(dlg: HWND, text: &str) {
    unsafe {
        let lb = item(dlg, IDC_LOG);
        let h = HSTRING::from(text);
        SendMessageW(
            lb,
            LB_INSERTSTRING,
            Some(WPARAM(0)),
            Some(LPARAM(h.as_ptr() as isize)),
        );
        let n = SendMessageW(lb, LB_GETCOUNT, None, None).0 as usize;
        if n > LOG_MAX {
            SendMessageW(lb, LB_DELETESTRING, Some(WPARAM(n - 1)), None);
        }
    }
}
