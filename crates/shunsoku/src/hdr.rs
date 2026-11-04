//! HDR のモニタの画像を GPU 上で SDR の画像（8bit sRGB の BGRA）にする（spec #34「HDR の変換」）。
//!
//! - 変換の式は core の `capture::hdr`。ここの HLSL は同じ式を書き写したもの。変えるときは両方を変える。
//! - モニタの「SDR コンテンツの明るさ」と HDR かどうかは `DisplayConfigGetDeviceInfo` で読む。

use shunsoku_core::capture::hdr::{HIGHLIGHT_ROLLOFF, SCRGB_WHITE_NITS};
use shunsoku_core::geometry::Size;

use crate::gpu_pass::{self, PixelShader};

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::core::Result;

/// HDR の画像のテクスチャの形式（scRGB、線形、1.0 = 80 nit）。
pub const HDR_FORMAT: DXGI_FORMAT = DXGI_FORMAT_R16G16B16A16_FLOAT;

/// モニタの色の設定。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorColor {
    /// HDR（Windows の「HDR を使用する」）が有効か。
    pub hdr: bool,
    /// 「SDR コンテンツの明るさ」（nit）。読めなければ scRGB の 1.0（80 nit）。
    pub sdr_white_nits: f32,
}

impl MonitorColor {
    const SDR: MonitorColor = MonitorColor {
        hdr: false,
        sdr_white_nits: SCRGB_WHITE_NITS,
    };
}

/// モニタの色の設定を読む。読めなければ SDR とみなす。
pub fn monitor_color(monitor: HMONITOR) -> MonitorColor {
    match read_monitor_color(monitor) {
        Ok(Some(color)) => color,
        Ok(None) => {
            log::warn!("no display path for the monitor; treating it as SDR");
            MonitorColor::SDR
        }
        Err(e) => {
            log::warn!("could not read the monitor's color settings ({e}); treating it as SDR");
            MonitorColor::SDR
        }
    }
}

fn read_monitor_color(monitor: HMONITOR) -> Result<Option<MonitorColor>> {
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: cbSize に MONITORINFOEXW の大きさを入れて渡す
    unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast()) }.ok()?;
    let name = info.szDevice;

    let (mut path_count, mut mode_count) = (0, 0);
    // SAFETY: 数を受け取るだけ
    unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) }
        .ok()?;
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: 配列の長さを数として渡す。返った数に縮める
    unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    }
    .ok()?;
    paths.truncate(path_count as usize);

    for path in &paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: header::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(
                DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                path.sourceInfo.adapterId,
                path.sourceInfo.id,
            ),
            ..Default::default()
        };
        // SAFETY: header の size は構造体の大きさ
        if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != ERROR_SUCCESS.0 as i32 {
            continue;
        }
        if !same_name(&source.viewGdiDeviceName, &name) {
            continue;
        }
        let target = (path.targetInfo.adapterId, path.targetInfo.id);
        let mut color = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
            header: header::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(
                DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                target.0,
                target.1,
            ),
            ..Default::default()
        };
        // SAFETY: 同上
        let hdr = unsafe { DisplayConfigGetDeviceInfo(&mut color.header) }
            == ERROR_SUCCESS.0 as i32
            // advancedColorEnabled（ビット 1）。SDR のモニタの自動カラー管理でも立つことがあるが、
            // そのときは SDR の明るさが 80 nit で、変換しても値は変わらない
            && unsafe { color.Anonymous.value } & 0b10 != 0;
        let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
            header: header::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(
                DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                target.0,
                target.1,
            ),
            SDRWhiteLevel: 0,
        };
        // SAFETY: 同上
        let level =
            if unsafe { DisplayConfigGetDeviceInfo(&mut white.header) } == ERROR_SUCCESS.0 as i32 {
                white.SDRWhiteLevel
            } else {
                0
            };
        // SDRWhiteLevel は 1000 が 80 nit
        let sdr_white_nits = if level > 0 {
            level as f32 / 1000.0 * SCRGB_WHITE_NITS
        } else {
            SCRGB_WHITE_NITS
        };
        return Ok(Some(MonitorColor {
            hdr,
            sdr_white_nits,
        }));
    }
    Ok(None)
}

fn header<T>(
    kind: DISPLAYCONFIG_DEVICE_INFO_TYPE,
    adapter: windows::Win32::Foundation::LUID,
    id: u32,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: kind,
        size: size_of::<T>() as u32,
        adapterId: adapter,
        id,
    }
}

/// NUL で終わる UTF-16 の名前を比べる（`\\.\DISPLAY1` など）。
fn same_name(a: &[u16], b: &[u16]) -> bool {
    let trim = |s: &[u16]| s.iter().position(|&c| c == 0).map_or(s.len(), |n| n);
    a[..trim(a)] == b[..trim(b)]
}

/// 変換のシェーダー。式は core の `capture::hdr::to_sdr_bgra` と同じ。定数は `Params`。
static SHADER: PixelShader = PixelShader::new(
    "HDR",
    r"
cbuffer Params : register(b0) {
    float scale;   // SCRGB_WHITE_NITS / SDR コンテンツの明るさ（normalize）
    float rolloff; // HIGHLIGHT_ROLLOFF
    float2 pad;
};
Texture2D<float4> source : register(t0);

float encode_srgb(float v) {
    return v <= 0.0031308 ? v * 12.92 : 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

float4 ps(float4 pos : SV_Position) : SV_Target {
    // normalize
    float3 rgb = max(source.Load(int3(pos.xy, 0)).rgb * scale, 0);
    // tone_map
    float m = max(rgb.r, max(rgb.g, rgb.b));
    if (m > 1) {
        float w = (m - 1) / (m - 1 + rolloff);
        float3 base = rgb / m;
        rgb = base + (1 - base) * w;
    }
    // encode_srgb（8bit への丸めは UNORM のテクスチャへの書き込みで行われる）
    float3 e = float3(encode_srgb(rgb.r), encode_srgb(rgb.g), encode_srgb(rgb.b));
    return float4(saturate(e), 1);
}
",
);

/// `SHADER` の cbuffer `Params`。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Params {
    /// `SCRGB_WHITE_NITS` / SDR コンテンツの明るさ（normalize）。
    scale: f32,
    /// `HIGHLIGHT_ROLLOFF`。
    rolloff: f32,
    /// cbuffer を 16 バイトの倍数にする詰め物。
    pad: [f32; 2],
}

/// HDR の画像（`HDR_FORMAT`）を、同じ大きさの 8bit sRGB の BGRA のテクスチャ（`D3D11_USAGE_DEFAULT`）にする。
pub fn to_sdr_texture(
    device: &ID3D11Device,
    source: &ID3D11Texture2D,
    sdr_white_nits: f32,
) -> Result<ID3D11Texture2D> {
    let desc = gpu_pass::desc(source);
    gpu_pass::draw(
        device,
        source.clone(),
        Size::new(desc.Width, desc.Height),
        DXGI_FORMAT_B8G8R8A8_UNORM,
        &SHADER,
        &Params {
            scale: SCRGB_WHITE_NITS / sdr_white_nits,
            rolloff: HIGHLIGHT_ROLLOFF,
            pad: [0.0, 0.0],
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use shunsoku_core::capture::hdr::to_sdr_bgra;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

    /// f32 を FP16 のビットにする（FP16 でちょうど表せる正規化数と 0 だけを渡す）。
    fn half(v: f32) -> u16 {
        if v == 0.0 {
            return 0;
        }
        let bits = v.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let mantissa = ((bits >> 13) & 0x3ff) as u16;
        sign | ((exp as u16) << 10) | mantissa
    }

    /// シェーダーが core の式と同じ値を出す（GPU は WARP）。
    #[test]
    fn shader_matches_core() {
        // SDR の範囲、白、白より明るい色、負の値
        let pixels: [[f32; 3]; 8] = [
            [0.0, 0.0, 0.0],
            [0.25, 0.5, 0.75],
            [3.0, 3.0, 3.0],
            [1.5, 0.75, 0.125],
            [12.0, 3.0, 0.5],
            [100.0, 0.0, 0.0],
            [-0.25, 1.0, 2.0],
            [0.0078125, 0.03125, 6.0],
        ];
        let sdr_white = 240.0;
        let data: Vec<u16> = pixels
            .iter()
            .flat_map(|p| [half(p[0]), half(p[1]), half(p[2]), half(1.0)])
            .collect();
        // SAFETY: テストの中で作ったデバイスとテクスチャだけを使う
        unsafe {
            let mut device = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
            .unwrap();
            let device: ID3D11Device = device.unwrap();
            let desc = D3D11_TEXTURE2D_DESC {
                Width: pixels.len() as u32,
                Height: 1,
                MipLevels: 1,
                ArraySize: 1,
                Format: HDR_FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                // Desktop Duplication の写しと同じく、シェーダーから読めないテクスチャで渡す
                BindFlags: 0,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let init = D3D11_SUBRESOURCE_DATA {
                pSysMem: data.as_ptr().cast(),
                SysMemPitch: (data.len() * 2) as u32,
                SysMemSlicePitch: 0,
            };
            let mut source = None;
            device
                .CreateTexture2D(&desc, Some(&init), Some(&mut source))
                .unwrap();
            let out = to_sdr_texture(&device, &source.unwrap(), sdr_white).unwrap();

            let staging_desc = D3D11_TEXTURE2D_DESC {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..desc
            };
            let mut staging = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            let context = device.GetImmediateContext().unwrap();
            context.CopyResource(&staging, &out);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let gpu =
                std::slice::from_raw_parts(mapped.pData as *const u8, pixels.len() * 4).to_vec();
            context.Unmap(&staging, 0);

            for (i, p) in pixels.iter().enumerate() {
                let expected = to_sdr_bgra(*p, sdr_white);
                let actual = &gpu[i * 4..i * 4 + 4];
                // GPU の pow と丸めの違いで 1 ずれることはある
                assert!(
                    expected
                        .iter()
                        .zip(actual)
                        .all(|(e, a)| e.abs_diff(*a) <= 1),
                    "{p:?}: core {expected:?}, shader {actual:?}"
                );
            }
        }
    }
}
