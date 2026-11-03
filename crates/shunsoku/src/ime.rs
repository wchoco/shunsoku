//! スクラップの文字の書き込みの IME（IMM32）。
//!
//! - スクラップのウィンドウは、文字を打っている間だけ IME を使う（`enable`）。打っていない間は入力コンテキストを
//!   外して（`disable`）、IME がオンでもキー（ショートカットや道具を選ぶ数字）がそのまま届くようにする。
//! - 変換中の文字は IME の既定の窓に出さず（`WM_IME_SETCONTEXT` で `ISC_SHOWUICOMPOSITIONWINDOW` を落とす）、
//!   スクラップの上に打っている文字の続きとして描く（`render::TypingLayer`）。候補の窓は IME が出す。
//!   その位置はキャレットに合わせる（`place`）。
//! - どれも `App` を借りずに呼ぶ（IME がスクラップのウィンドウにメッセージを送り返すことがあるため）。

use shunsoku_core::annotate::Composition;
use shunsoku_core::geometry::Rect;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, CPS_CANCEL, GCS_COMPSTR, GCS_CURSORPOS,
    GCS_RESULTSTR, HIMC, IACE_DEFAULT, IME_COMPOSITION_STRING, ISC_SHOWUICOMPOSITIONWINDOW,
    ImmAssociateContextEx, ImmGetCompositionStringW, ImmGetContext, ImmNotifyIME,
    ImmReleaseContext, ImmSetCandidateWindow, ImmSetCompositionWindow, NI_COMPOSITIONSTR,
};

/// `WM_IME_COMPOSITION` で変わったもの。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompositionUpdate {
    /// 確定した文字（`GCS_RESULTSTR`）。
    pub result: String,
    /// まだ変換中の文字（`GCS_COMPSTR`）。なければ None。
    pub composition: Option<Composition>,
}

/// `hwnd` で IME を使えるようにする（既定の入力コンテキストを結びつける）。
pub fn enable(hwnd: HWND) {
    // SAFETY: 自分のウィンドウ
    if !unsafe { ImmAssociateContextEx(hwnd, HIMC::default(), IACE_DEFAULT) }.as_bool() {
        log::warn!("could not enable the IME on a scrap");
    }
}

/// `hwnd` で変換中の文字を捨て、IME を使えないようにする（入力コンテキストを外す）。
pub fn disable(hwnd: HWND) {
    cancel(hwnd);
    // SAFETY: 自分のウィンドウ。HIMC を空にすると結びつきを外す
    unsafe {
        let _ = ImmAssociateContextEx(hwnd, HIMC::default(), 0);
    }
}

/// 変換中の文字を捨てる（取り込んだあとに IME の側の残りを消す）。
pub fn cancel(hwnd: HWND) {
    with_context(hwnd, |himc| {
        // SAFETY: with_context が渡す、生きている入力コンテキスト
        unsafe {
            let _ = ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
        }
    });
}

/// 変換中の文字と候補の窓を、キャレット（スクリーン座標）のところに出させる。候補の窓はキャレットの行を
/// 隠さないよう、その下に出させる。
pub fn place(hwnd: HWND, caret: Rect) {
    // SAFETY: 引数はこの関数の中で生きている
    let origin = unsafe {
        let mut p = POINT {
            x: caret.left,
            y: caret.top,
        };
        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut p);
        p
    };
    let area = RECT {
        left: origin.x,
        top: origin.y,
        right: origin.x + caret.width() as i32,
        bottom: origin.y + caret.height() as i32,
    };
    with_context(hwnd, |himc| {
        let composition = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: origin,
            rcArea: area,
        };
        let candidate = CANDIDATEFORM {
            dwIndex: 0,
            dwStyle: CFS_EXCLUDE,
            ptCurrentPos: origin,
            rcArea: area,
        };
        // SAFETY: with_context が渡す、生きている入力コンテキスト
        unsafe {
            let _ = ImmSetCompositionWindow(himc, &composition);
            let _ = ImmSetCandidateWindow(himc, &candidate);
        }
    });
}

/// `WM_IME_SETCONTEXT` の lparam から、変換中の文字の既定の窓を出す印を落とす。
pub fn without_composition_window(lparam: LPARAM) -> LPARAM {
    LPARAM(lparam.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize))
}

/// `WM_IME_COMPOSITION`（lparam は変わったものの印）で、確定した文字と変換中の文字を読む。
pub fn read_composition(hwnd: HWND, lparam: LPARAM) -> CompositionUpdate {
    let flags = lparam.0 as u32;
    with_context(hwnd, |himc| {
        let result = if flags & GCS_RESULTSTR.0 != 0 {
            read_string(himc, GCS_RESULTSTR)
        } else {
            String::new()
        };
        let composition = if flags & GCS_COMPSTR.0 != 0 {
            let text = read_string(himc, GCS_COMPSTR);
            // SAFETY: with_context が渡す、生きている入力コンテキスト。GCS_CURSORPOS は値を返す
            let cursor = if flags & GCS_CURSORPOS.0 != 0 {
                unsafe { ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0) }.max(0) as usize
            } else {
                text.encode_utf16().count()
            };
            (!text.is_empty()).then_some(Composition { text, cursor })
        } else {
            None
        };
        CompositionUpdate {
            result,
            composition,
        }
    })
    .unwrap_or_default()
}

/// 入力コンテキストの文字（`GCS_RESULTSTR` か `GCS_COMPSTR`）。
fn read_string(himc: HIMC, which: IME_COMPOSITION_STRING) -> String {
    // SAFETY: 大きさ（バイト）を聞いてから、その大きさの領域に書かせる
    unsafe {
        let bytes = ImmGetCompositionStringW(himc, which, None, 0);
        if bytes <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; bytes as usize / 2];
        let written = ImmGetCompositionStringW(
            himc,
            which,
            Some(buf.as_mut_ptr().cast()),
            (buf.len() * 2) as u32,
        );
        buf.truncate(written.max(0) as usize / 2);
        String::from_utf16_lossy(&buf)
    }
}

/// `hwnd` の入力コンテキストを借りて `f` を呼ぶ。結びついていなければ None。
fn with_context<R>(hwnd: HWND, f: impl FnOnce(HIMC) -> R) -> Option<R> {
    // SAFETY: 自分のウィンドウ。借りたものは返す
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return None;
        }
        let r = f(himc);
        let _ = ImmReleaseContext(hwnd, himc);
        Some(r)
    }
}
