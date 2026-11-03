//! キーの組み合わせの入力欄（ADR-0004）。標準の hot key control（`msctls_hotkey32`）をサブクラス化する。
//! 全体のホットキーとスクラップ操作のショートカットで共通に使う。
//!
//! - `WM_GETDLGCODE` では、標準の処理の戻り値に `DLGC_WANTALLKEYS` を足す（Esc と Enter をダイアログに取られない）。
//!   Tab / Shift+Tab だけは標準の戻り値のまま返し、ダイアログにフォーカスを移させる。
//! - キー押下（`WM_KEYDOWN` / `WM_SYSKEYDOWN`）はここで受け、core の `Chord::from_key_down` で組み合わせにして
//!   `HKM_SETHOTKEY` で入れる。標準の処理には渡さない（標準の処理は Esc と Enter を「なし」にする）。キーを離す・
//!   文字のメッセージも握りつぶす。入ったら親に `WM_COMMAND`（`EN_CHANGE`）を送る。値は `value` で読む。
//! - フォーカスを得た・失ったら、親に `WM_COMMAND`（`EN_SETFOCUS` / `EN_KILLFOCUS`）を送る。
//! - コントロールで表せない組み合わせ（設定ファイルに書いた Win キー・PrintScreen）は、`set` で入れると文字で
//!   描くだけにする（`WM_PAINT`）。キーを入れたら上書きする。
//! - IME は切り離す（変換中のキーが欄に届かなくなるため）。

use std::cell::Cell;

use shunsoku_core::keys::{Chord, Modifiers};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, COLOR_WINDOWTEXT, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DrawTextW,
    EndPaint, FillRect, GetSysColor, GetSysColorBrush, HFONT, InvalidateRect, PAINTSTRUCT,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::HKM_SETHOTKEY;
use windows::Win32::UI::Input::Ime::{HIMC, ImmAssociateContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DLGC_WANTALLKEYS, EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS, GWLP_ID, GetClientRect, GetParent,
    GetWindowLongPtrW, MSG, SendMessageW, WM_CHAR, WM_COMMAND, WM_DEADCHAR, WM_GETDLGCODE,
    WM_GETFONT, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_NCDESTROY, WM_PAINT, WM_SETFOCUS,
    WM_SYSCHAR, WM_SYSDEADCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const SUBCLASS_ID: usize = 1;

/// 欄の値。サブクラスの参照データとして持つ。
type State = Cell<Option<Chord>>;

/// hot key control `hwnd` を入力欄にする。ダイアログを作った直後に 1 回呼ぶ。
pub fn attach(hwnd: HWND) {
    let state = Box::into_raw(Box::new(State::new(None)));
    // SAFETY: state は WM_NCDESTROY で 1 回だけ戻す。IME の切り離しは自分のコントロールに対して
    unsafe {
        if !SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, state as usize).as_bool() {
            drop(Box::from_raw(state));
            log::warn!("could not subclass a chord field");
            return;
        }
        let _ = ImmAssociateContext(hwnd, HIMC::default());
    }
}

/// `hwnd` が入力欄か。
pub fn is_field(hwnd: HWND) -> bool {
    state(hwnd).is_some()
}

/// 欄の値。入力欄でなければ None。
pub fn value(hwnd: HWND) -> Option<Chord> {
    state(hwnd).and_then(Cell::get)
}

/// 欄に `chord` を入れる。None は空（コントロールの「なし」）。コントロールで表せない組み合わせは文字で描く。
pub fn set(hwnd: HWND, chord: Option<Chord>) {
    let Some(state) = state(hwnd) else {
        return;
    };
    state.set(chord);
    let shown = chord.and_then(Chord::to_hotkey_control).unwrap_or(0);
    // SAFETY: 自分のコントロールへのメッセージ
    unsafe {
        SendMessageW(hwnd, HKM_SETHOTKEY, Some(WPARAM(usize::from(shown))), None);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn state(hwnd: HWND) -> Option<&'static State> {
    let mut data = 0usize;
    // SAFETY: data は attach で入れた Box のポインタ。コントロールが生きている間だけ有効で、UI スレッドだけが触る
    unsafe {
        if GetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, Some(&mut data)).as_bool()
            && data != 0
        {
            Some(&*(data as *const State))
        } else {
            None
        }
    }
}

/// いま押されている修飾キー。Win キーは入力に使わないので見ない。
fn held() -> Modifiers {
    // SAFETY: 引数は定数
    let down = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
    Modifiers {
        win: false,
        ctrl: down(VK_CONTROL.0),
        alt: down(VK_MENU.0),
        shift: down(VK_SHIFT.0),
    }
}

/// 親に `WM_COMMAND`（`code`）を送る。
fn notify_parent(hwnd: HWND, code: u32) {
    // SAFETY: 自分の ID と親への送信
    unsafe {
        let id = GetWindowLongPtrW(hwnd, GWLP_ID) as usize & 0xFFFF;
        if let Ok(parent) = GetParent(hwnd) {
            SendMessageW(
                parent,
                WM_COMMAND,
                Some(WPARAM(id | ((code as usize) << 16))),
                Some(LPARAM(hwnd.0 as isize)),
            );
        }
    }
}

/// コントロールで表せない値を文字で描く。
fn paint_text(hwnd: HWND, chord: Chord) {
    let mut text: Vec<u16> = chord.to_string().encode_utf16().collect();
    // SAFETY: BeginPaint と EndPaint を対にする。フォントはコントロールのもの（ダイアログが持つ）
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        FillRect(dc, &rect, GetSysColorBrush(COLOR_WINDOW));
        let font = HFONT(SendMessageW(hwnd, WM_GETFONT, None, None).0 as *mut _);
        let old = (!font.is_invalid()).then(|| SelectObject(dc, font.into()));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(
            dc,
            windows::Win32::Foundation::COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
        );
        rect.left += 2;
        DrawTextW(
            dc,
            &mut text,
            &mut rect,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        if let Some(old) = old {
            SelectObject(dc, old);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // SAFETY: 受け取った引数をそのまま渡す。data は attach で入れた State
    unsafe {
        match msg {
            WM_GETDLGCODE => {
                let base = DefSubclassProc(hwnd, msg, wparam, lparam);
                let m = lparam.0 as *const MSG;
                if !m.is_null()
                    && matches!((*m).message, WM_KEYDOWN | WM_SYSKEYDOWN)
                    && (*m).wParam.0 == usize::from(VK_TAB.0)
                {
                    return base;
                }
                LRESULT(base.0 | DLGC_WANTALLKEYS as isize)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if let Some(chord) = Chord::from_key_down(wparam.0 as u16, held()) {
                    set(hwnd, Some(chord));
                    notify_parent(hwnd, EN_CHANGE);
                }
                LRESULT(0)
            }
            WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => {
                LRESULT(0)
            }
            WM_PAINT => {
                let state = &*(data as *const State);
                match state.get() {
                    Some(chord) if chord.to_hotkey_control().is_none() => {
                        paint_text(hwnd, chord);
                        LRESULT(0)
                    }
                    _ => DefSubclassProc(hwnd, msg, wparam, lparam),
                }
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                notify_parent(
                    hwnd,
                    if msg == WM_SETFOCUS {
                        EN_SETFOCUS
                    } else {
                        EN_KILLFOCUS
                    },
                );
                result
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
                drop(Box::from_raw(data as *mut State));
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}
