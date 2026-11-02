//! 範囲選択の画面でハイライトするウィンドウの一覧（`selection::WindowInfo`）を集める（spec #34）。
//!
//! - 並びは Z オーダーの手前から奥（`EnumWindows`）。最上位のウィンドウのあとに、その子孫を
//!   `EnumChildWindows` の順（兄弟の間は手前から奥）で続ける。
//! - 見えないもの・最小化したもの・クロークされたもの（別の仮想デスクトップ、裏の UWP）・
//!   クリックが素通りするもの（`WS_EX_TRANSPARENT`）・範囲選択の画面そのもの・デスクトップ（全モニタを
//!   覆う `Progman` / `WorkerW`）は入れない。
//! - 最上位のウィンドウの矩形は WGC が撮る範囲（`DWMWA_EXTENDED_FRAME_BOUNDS`）にする。
//! - 名前は `InternalGetWindowText` で読む（`GetWindowTextW` と違い、応答しないウィンドウで止まらない）。
//!   部品で空なら、クラス名にする。

use shunsoku_core::geometry::Rect;
use shunsoku_core::selection::{WindowId, WindowInfo, WindowKind};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GA_PARENT, GWL_EXSTYLE, GetAncestor, GetClassNameW,
    GetWindowLongW, GetWindowRect, InternalGetWindowText, IsIconic, IsWindowVisible,
    WS_EX_TRANSPARENT,
};
use windows::core::BOOL;

use crate::window_grab::window_bounds;

/// 範囲選択の画面のウィンドウのクラス名（`overlay`）。
const OVERLAY_CLASS: &str = "shunsoku.overlay";
/// スクラップのウィンドウのクラス名（`scrap_window`）。
const SCRAP_CLASS: &str = "shunsoku.scrap";
/// タスクバー（メインのモニタと、ほかのモニタ）。
const TASKBAR_CLASSES: [&str; 2] = ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
/// デスクトップ（壁紙とアイコン）。全モニタを覆うので、ハイライトの対象にしない。
const DESKTOP_CLASSES: [&str; 2] = ["Progman", "WorkerW"];

/// いまのウィンドウの一覧を集める。
pub fn collect() -> Vec<WindowInfo> {
    let mut tops: Vec<HWND> = Vec::new();
    // SAFETY: コールバックには、この関数の中で生きている tops を渡す
    let _ = unsafe { EnumWindows(Some(push_hwnd), LPARAM((&raw mut tops) as isize)) };
    let mut list = Vec::with_capacity(tops.len());
    for hwnd in tops {
        let Some(top) = top_level(hwnd) else {
            continue;
        };
        let normal = top.kind == WindowKind::Normal;
        list.push(top);
        if normal {
            children(hwnd, &mut list);
        }
    }
    list
}

/// 最上位のウィンドウ。ハイライトの対象にならないものは None。
fn top_level(hwnd: HWND) -> Option<WindowInfo> {
    // SAFETY: 状態を読むだけ
    let shown = unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() };
    if !shown || is_cloaked(hwnd) || is_click_through(hwnd) {
        return None;
    }
    let class = class_name(hwnd);
    if class == OVERLAY_CLASS || DESKTOP_CLASSES.contains(&class.as_str()) {
        return None;
    }
    let rect = window_bounds(hwnd);
    if rect.is_empty() {
        return None;
    }
    let kind = if class == SCRAP_CLASS {
        WindowKind::Scrap
    } else if TASKBAR_CLASSES.contains(&class.as_str()) {
        WindowKind::Taskbar
    } else {
        WindowKind::Normal
    };
    Some(WindowInfo {
        id: id(hwnd),
        rect,
        name: text(hwnd),
        parent: None,
        kind,
    })
}

/// `top` の子孫（部品）を `list` に足す。
fn children(top: HWND, list: &mut Vec<WindowInfo>) {
    let mut hwnds: Vec<HWND> = Vec::new();
    // SAFETY: コールバックには、この関数の中で生きている hwnds を渡す
    let _ = unsafe {
        EnumChildWindows(
            Some(top),
            Some(push_hwnd),
            LPARAM((&raw mut hwnds) as isize),
        )
    };
    for hwnd in hwnds {
        // SAFETY: 状態を読むだけ。IsWindowVisible は親が見えないときも false
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() || is_click_through(hwnd) {
            continue;
        }
        let mut r = RECT::default();
        // SAFETY: 読むだけ
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
            continue;
        }
        let rect = Rect::new(r.left, r.top, r.right, r.bottom);
        if rect.is_empty() {
            continue;
        }
        // SAFETY: 読むだけ
        let parent = unsafe { GetAncestor(hwnd, GA_PARENT) };
        let mut name = text(hwnd);
        if name.is_empty() {
            name = class_name(hwnd);
        }
        list.push(WindowInfo {
            id: id(hwnd),
            rect,
            name,
            parent: Some(id(parent)),
            kind: WindowKind::Normal,
        });
    }
}

/// `EnumWindows` / `EnumChildWindows` のコールバック。`lparam` は `Vec<HWND>` を指す。
unsafe extern "system" fn push_hwnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: collect / children が渡した、列挙の間だけ生きている Vec
    let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    list.push(hwnd);
    true.into()
}

fn id(hwnd: HWND) -> WindowId {
    WindowId(hwnd.0 as isize)
}

/// core の識別子から `HWND` に戻す。
pub fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as *mut _)
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    // SAFETY: u32 の大きさを渡して読むだけ
    let read = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        )
    };
    read.is_ok() && cloaked != 0
}

fn is_click_through(hwnd: HWND) -> bool {
    // SAFETY: 読むだけ
    let ex = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    ex & WS_EX_TRANSPARENT.0 != 0
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: バッファの大きさを渡して読むだけ
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
}

fn text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    // SAFETY: バッファの大きさを渡して読むだけ。メッセージは送らない
    let len = unsafe { InternalGetWindowText(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
}
