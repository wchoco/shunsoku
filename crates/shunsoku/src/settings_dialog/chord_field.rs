//! キーの組み合わせの入力欄（ADR-0004）。標準の hot key control（`msctls_hotkey32`）をサブクラス化する。
//! 全体のホットキーとスクラップ操作のショートカットで共通に使う。
//!
//! - `WM_GETDLGCODE` では、標準の処理の戻り値に `DLGC_WANTALLKEYS` を足す（Esc と Enter をダイアログに取られない）。
//!   Tab / Shift+Tab だけは標準の戻り値のまま返し、ダイアログにフォーカスを移させる。
//! - キー押下（`WM_KEYDOWN` / `WM_SYSKEYDOWN`）はここで受け、core の `FieldInput::from_key_down` で入力にする。
//!   組み合わせは `HKM_SETHOTKEY` でも入れる。修飾キーなしの Backspace・Delete は割り当てを外す（「なし」）。
//!   標準の処理には渡さない（標準の処理は Esc と Enter を「なし」にする）。キーを離す・文字のメッセージも
//!   握りつぶす。入ったら親に `WM_COMMAND`（`EN_CHANGE`）を送る。値は `value` で読む。
//! - フォーカスを得た・失ったら、親に `WM_COMMAND`（`EN_SETFOCUS` / `EN_KILLFOCUS`）を送る。
//! - 描画（`WM_PAINT`）は自分でする。組み合わせは core の表示（`Ctrl+S`）で描き、一覧や設定ファイルと書き方を
//!   揃える（標準の描画は `Ctrl + S`）。Win キー・PrintScreen を含む、コントロールで表せない組み合わせも同じに描く。
//! - 値が空の間は、`set_placeholder` で入れた文字を灰色で描く（スクラップ操作の、上書きのない既定の一覧）。
//!   それもなければ `attach` で渡した「なし」の文字を描く（標準の描画は OS の表示言語の「なし」を出し、UI の
//!   言語に合わないため。#102）。
//! - IME は切り離す（変換中のキーが欄に届かなくなるため）。

use std::cell::{Cell, RefCell};

use shunsoku_core::keys::{Chord, FieldInput, Modifiers};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, DT_LEFT,
    DT_SINGLELINE, DT_VCENTER, DrawTextW, EndPaint, FillRect, GetSysColor, GetSysColorBrush,
    GetTextExtentPoint32W, HFONT, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::HKM_SETHOTKEY;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::Ime::{HIMC, ImmAssociateContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, IsWindowEnabled, VK_CONTROL, VK_MENU, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DLGC_WANTALLKEYS, EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS, GWLP_ID, GetCaretPos, GetClientRect,
    GetParent, GetWindowLongPtrW, MSG, SendMessageW, SetCaretPos, WM_CHAR, WM_COMMAND, WM_DEADCHAR,
    WM_GETDLGCODE, WM_GETFONT, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_NCDESTROY, WM_PAINT,
    WM_SETFOCUS, WM_SYSCHAR, WM_SYSDEADCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const SUBCLASS_ID: usize = 1;
/// 文字の左の余白（96 DPI の px）。
const MARGIN: i32 = 2;

/// 欄の値と、空の間に描く文字。サブクラスの参照データとして持つ。
struct State {
    chord: Cell<Option<Chord>>,
    placeholder: RefCell<String>,
    /// 値が空で `placeholder` もない間に描く「なし」（UI の言語の文言）。
    none: String,
}

/// hot key control `hwnd` を入力欄にする。ダイアログを作った直後に 1 回呼ぶ。`none` は空の間に描く「なし」。
pub fn attach(hwnd: HWND, none: &str) {
    let state = Box::into_raw(Box::new(State {
        chord: Cell::new(None),
        placeholder: RefCell::new(String::new()),
        none: none.to_owned(),
    }));
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
    state(hwnd).and_then(|s| s.chord.get())
}

/// 欄に `chord` を入れる。None は空（「なし」）。
pub fn set(hwnd: HWND, chord: Option<Chord>) {
    let Some(state) = state(hwnd) else {
        return;
    };
    state.chord.set(chord);
    let shown = chord.and_then(Chord::to_hotkey_control).unwrap_or(0);
    // SAFETY: 自分のコントロールへのメッセージ
    unsafe {
        SendMessageW(hwnd, HKM_SETHOTKEY, Some(WPARAM(usize::from(shown))), None);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// 値が空の間に描く文字を `text` にする（空なら描かない）。
pub fn set_placeholder(hwnd: HWND, text: &str) {
    let Some(state) = state(hwnd) else {
        return;
    };
    if *state.placeholder.borrow() != text {
        *state.placeholder.borrow_mut() = text.to_owned();
        // SAFETY: 自分のコントロールの描き直しを頼むだけ
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
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

/// 文字 `text` を描く。`dim` なら灰色で描く。フォーカスがあれば、キャレットを文字の後ろに置く（標準の
/// コントロールは自分の書き方の幅で置くため）。
fn paint_text(hwnd: HWND, text: &str, dim: bool) {
    let mut text: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: BeginPaint と EndPaint を対にする。フォントはコントロールのもの（ダイアログが持つ）
    unsafe {
        let enabled = IsWindowEnabled(hwnd).as_bool();
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let back = if enabled { COLOR_WINDOW } else { COLOR_BTNFACE };
        FillRect(dc, &rect, GetSysColorBrush(back));
        let font = HFONT(SendMessageW(hwnd, WM_GETFONT, None, None).0 as *mut _);
        let old = (!font.is_invalid()).then(|| SelectObject(dc, font.into()));
        SetBkMode(dc, TRANSPARENT);
        let color = if dim || !enabled {
            COLOR_GRAYTEXT
        } else {
            COLOR_WINDOWTEXT
        };
        SetTextColor(dc, COLORREF(GetSysColor(color)));
        // 余白は 96 DPI で 2 px
        rect.left += MARGIN * GetDpiForWindow(hwnd) as i32 / 96;
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &text, &mut size);
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
        if GetFocus() == hwnd {
            let mut caret = POINT::default();
            let _ = GetCaretPos(&mut caret);
            // 文字が空（灰色の既定の一覧・「なし」）なら先頭に置く
            let x = if dim || state(hwnd).is_some_and(|s| s.chord.get().is_none()) {
                rect.left
            } else {
                rect.left + size.cx
            };
            let _ = SetCaretPos(x, caret.y);
        }
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
                let chord = match FieldInput::from_key_down(wparam.0 as u16, held()) {
                    Some(FieldInput::Set(chord)) => Some(chord),
                    Some(FieldInput::Clear) => None,
                    None => return LRESULT(0),
                };
                set(hwnd, chord);
                notify_parent(hwnd, EN_CHANGE);
                LRESULT(0)
            }
            WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => {
                LRESULT(0)
            }
            WM_PAINT => {
                let state = &*(data as *const State);
                match state.chord.get() {
                    Some(chord) => paint_text(hwnd, &chord.to_string(), false),
                    None if !state.placeholder.borrow().is_empty() => {
                        paint_text(hwnd, &state.placeholder.borrow(), true)
                    }
                    None => paint_text(hwnd, &state.none, false),
                }
                LRESULT(0)
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
