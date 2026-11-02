//! モニタの一覧（core の `monitor::MonitorInfo`）を集める。撮るたびに集め直す（抜き差しや配置の変更を拾うため）。
//!
//! 識別子は、モニタのデバイスインターフェースのパス（`EnumDisplayDevicesW` に `EDD_GET_DEVICE_INTERFACE_NAME`）。
//! `\\.\DISPLAY1` のような GDI のデバイス名は並びを変えると入れ替わることがあるので使わない。パスが取れなければ
//! GDI のデバイス名で代える。

use shunsoku_core::geometry::Rect;
use shunsoku_core::monitor::MonitorInfo;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY, USER_DEFAULT_SCREEN_DPI,
};
use windows::core::{BOOL, PCWSTR};

/// いまデスクトップにつながっているモニタ。
pub fn list() -> Vec<MonitorInfo> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: コールバックは列挙の間だけ handles を借りる
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        );
    }
    handles.into_iter().filter_map(info).collect()
}

unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: data は list の handles
    let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    handles.push(monitor);
    true.into()
}

fn info(monitor: HMONITOR) -> Option<MonitorInfo> {
    let mut mi = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: cbSize を MONITORINFOEXW の大きさにして渡す
    if !unsafe { GetMonitorInfoW(monitor, &mut mi.monitorInfo) }.as_bool() {
        return None;
    }
    let device_name = from_wide(&mi.szDevice);
    let mut dd = DISPLAY_DEVICEW {
        cb: size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    // SAFETY: szDevice は NUL で終わる。dd は呼び出しの間生きている
    let found = unsafe {
        EnumDisplayDevicesW(
            PCWSTR(mi.szDevice.as_ptr()),
            0,
            &mut dd,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    }
    .as_bool();
    let interface = found.then(|| from_wide(&dd.DeviceID));
    let id = interface
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| device_name.clone());
    let name = if found {
        from_wide(&dd.DeviceString)
    } else {
        device_name
    };
    let (mut dpi, mut dpi_y) = (USER_DEFAULT_SCREEN_DPI, USER_DEFAULT_SCREEN_DPI);
    // SAFETY: DPI を読むだけ。失敗したら 100% とみなす
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }.is_err() {
        dpi = USER_DEFAULT_SCREEN_DPI;
    }
    let r = mi.monitorInfo.rcMonitor;
    Some(MonitorInfo {
        id,
        name,
        rect: Rect::new(r.left, r.top, r.right, r.bottom),
        dpi,
        primary: mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    })
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
