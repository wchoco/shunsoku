//! モニタの一覧を集める（core の `monitors::Monitor`）。番号付けと範囲の判定は core で行う。
//! 撮るたびに集め直す（抜き差しや配置の変更を拾うため）。
//!
//! - どのスレッドから呼んでもよい（DPI はプロセスの PMv2 の設定で読む）。
//! - 識別子は、モニタのデバイスインターフェースのパス（`EnumDisplayDevicesW` に `EDD_GET_DEVICE_INTERFACE_NAME`）。
//!   `\\.\DISPLAY1` のような GDI のデバイス名は並びを変えると入れ替わることがあるので使わない。パスが取れなければ
//!   GDI のデバイス名で代える。

use shunsoku_core::geometry::Rect;
use shunsoku_core::monitors::Monitor;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY,
};
use windows::core::{BOOL, PCWSTR};

/// いまのモニタの一覧（並びは `EnumDisplayMonitors` の順）。
pub fn list() -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: handles は呼び出しの間生きていて、コールバックはその中でだけ使う
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        );
    }
    let names = friendly_names();
    handles
        .into_iter()
        .filter_map(|handle| {
            let mut info = MONITORINFOEXW {
                monitorInfo: MONITORINFO {
                    cbSize: size_of::<MONITORINFOEXW>() as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            // SAFETY: cbSize を MONITORINFOEXW の大きさにして渡す
            if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
                return None;
            }
            let device = from_wide(&info.szDevice);
            let id = interface_path(&info.szDevice).unwrap_or_else(|| device.clone());
            let (mut dpi_x, mut dpi_y) = (96, 96);
            // SAFETY: 出力はこの関数の中で生きている。失敗したら 96 のまま
            let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
            let name = names
                .iter()
                .find(|(gdi, _)| *gdi == device)
                .map(|(_, friendly)| friendly.clone())
                .filter(|friendly| !friendly.is_empty())
                .unwrap_or(device);
            let RECT {
                left,
                top,
                right,
                bottom,
            } = info.monitorInfo.rcMonitor;
            Some(Monitor {
                id,
                name,
                rect: Rect::new(left, top, right, bottom),
                dpi: dpi_x,
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            })
        })
        .collect()
}

unsafe extern "system" fn collect(handle: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    // SAFETY: data は list の handles を指す
    let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
    handles.push(handle);
    true.into()
}

/// GDI のデバイス名 `device`（NUL で終わる）のモニタのデバイスインターフェースのパス。取れなければ None。
fn interface_path(device: &[u16]) -> Option<String> {
    let mut dd = DISPLAY_DEVICEW {
        cb: size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    // SAFETY: device は NUL で終わる。dd は呼び出しの間生きている
    let found = unsafe {
        EnumDisplayDevicesW(
            PCWSTR(device.as_ptr()),
            0,
            &mut dd,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    }
    .as_bool();
    found
        .then(|| from_wide(&dd.DeviceID))
        .filter(|path| !path.is_empty())
}

/// GDI のデバイスの名前（`\\.\DISPLAY1`）と、モニタの製品名（EDID の名前）の組。取れなければ空。
fn friendly_names() -> Vec<(String, String)> {
    let (mut path_count, mut mode_count) = (0u32, 0u32);
    // SAFETY: 数を受ける変数はこの関数の中で生きている
    if unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    } != ERROR_SUCCESS
    {
        return Vec::new();
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: 配列は数のとおりの大きさ
    if unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    } != ERROR_SUCCESS
    {
        return Vec::new();
    }
    paths.truncate(path_count as usize);
    paths
        .iter()
        .filter_map(|path| {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            // SAFETY: header の size はそれぞれの構造体の大きさ
            let ok = unsafe {
                DisplayConfigGetDeviceInfo(&mut source.header) == 0
                    && DisplayConfigGetDeviceInfo(&mut target.header) == 0
            };
            ok.then(|| {
                (
                    from_wide(&source.viewGdiDeviceName),
                    from_wide(&target.monitorFriendlyDeviceName),
                )
            })
        })
        .collect()
}

/// NUL で終わる UTF-16 の配列を文字列にする。
fn from_wide(wide: &[u16]) -> String {
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len])
}
