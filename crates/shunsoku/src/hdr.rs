//! HDR のモニタの画像を GPU 上で SDR の画像（8bit sRGB の BGRA）にする（spec #34「HDR の変換」）。
//!
//! - 変換の式は core の `capture::hdr`。ここの HLSL は同じ式を書き写したもの。変えるときは両方を変える。
//! - モニタの「SDR コンテンツの明るさ」と HDR かどうかは `DisplayConfigGetDeviceInfo` で読む。

use std::sync::OnceLock;

use shunsoku_core::capture::hdr::{HIGHLIGHT_ROLLOFF, SCRGB_WHITE_NITS};

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{E_FAIL, E_POINTER, ERROR_SUCCESS};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT, ID3D11Device, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::core::{PCSTR, Result, s};

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

/// 変換のシェーダー。式は core の `capture::hdr::to_sdr_bgra` と同じ。
const SHADER: &str = r"
cbuffer Params : register(b0) {
    float scale;   // SCRGB_WHITE_NITS / SDR コンテンツの明るさ（normalize）
    float rolloff; // HIGHLIGHT_ROLLOFF
    float2 pad;
};
Texture2D<float4> source : register(t0);

float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}

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
";

/// コンパイル済みのシェーダー（頂点、ピクセル）。
type Shaders = (Vec<u8>, Vec<u8>);

/// コンパイル済みのシェーダー。デバイスによらないので 1 回だけ作る。
fn shaders() -> Result<&'static Shaders> {
    static SHADERS: OnceLock<std::result::Result<Shaders, String>> = OnceLock::new();
    SHADERS
        .get_or_init(|| {
            let vs = compile(s!("vs"), s!("vs_4_0"))?;
            let ps = compile(s!("ps"), s!("ps_4_0"))?;
            Ok((vs, ps))
        })
        .as_ref()
        .map_err(|e| windows::core::Error::new(E_FAIL, e.as_str()))
}

fn compile(entry: PCSTR, target: PCSTR) -> std::result::Result<Vec<u8>, String> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: ソースの長さを渡す。返ったバッファはその場で写す
    unsafe {
        let result = D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            s!("hdr"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        );
        let blob_bytes = |b: &ID3DBlob| {
            std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize())
                .to_vec()
        };
        match (result, code) {
            (Ok(()), Some(code)) => Ok(blob_bytes(&code)),
            (result, _) => {
                let message = errors
                    .map(|e| String::from_utf8_lossy(&blob_bytes(&e)).into_owned())
                    .unwrap_or_default();
                Err(format!(
                    "HDR shader did not compile ({result:?}): {message}"
                ))
            }
        }
    }
}

/// HDR の画像（`HDR_FORMAT`）を、同じ大きさの 8bit sRGB の BGRA のテクスチャ（`D3D11_USAGE_DEFAULT`）にする。
pub fn to_sdr_texture(
    device: &ID3D11Device,
    source: &ID3D11Texture2D,
    sdr_white_nits: f32,
) -> Result<ID3D11Texture2D> {
    let (vs_code, ps_code) = shaders()?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: 以下はすべて同じデバイスのリソースに対する呼び出し。コンテキストは
    // SetMultithreadProtected 済みで、UI スレッドから呼ぶ
    unsafe {
        source.GetDesc(&mut desc);
        let context = device.GetImmediateContext()?;

        // シェーダーから読めないテクスチャ（Desktop Duplication の写し）は、読める写しを作る
        let readable = if desc.BindFlags & D3D11_BIND_SHADER_RESOURCE.0 as u32 != 0 {
            source.clone()
        } else {
            let copy_desc = D3D11_TEXTURE2D_DESC {
                MipLevels: 1,
                ArraySize: 1,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
                ..desc
            };
            let mut copy = None;
            device.CreateTexture2D(&copy_desc, None, Some(&mut copy))?;
            let copy = copy.ok_or(E_POINTER)?;
            context.CopySubresourceRegion(&copy, 0, 0, 0, 0, source, 0, None);
            copy
        };

        let out_desc = D3D11_TEXTURE2D_DESC {
            Width: desc.Width,
            Height: desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut out = None;
        device.CreateTexture2D(&out_desc, None, Some(&mut out))?;
        let out = out.ok_or(E_POINTER)?;

        let mut srv = None;
        device.CreateShaderResourceView(&readable, None, Some(&mut srv))?;
        let mut rtv = None;
        device.CreateRenderTargetView(&out, None, Some(&mut rtv))?;
        let mut vs = None;
        device.CreateVertexShader(vs_code, None, Some(&mut vs))?;
        let mut ps = None;
        device.CreatePixelShader(ps_code, None, Some(&mut ps))?;
        let params: [f32; 4] = [
            SCRGB_WHITE_NITS / sdr_white_nits,
            HIGHLIGHT_ROLLOFF,
            0.0,
            0.0,
        ];
        let buffer_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of_val(&params) as u32,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: params.as_ptr().cast(),
            ..Default::default()
        };
        let mut buffer = None;
        device.CreateBuffer(&buffer_desc, Some(&data), Some(&mut buffer))?;

        context.ClearState();
        context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context.VSSetShader(vs.as_ref(), None);
        context.PSSetShader(ps.as_ref(), None);
        context.PSSetShaderResources(0, Some(&[srv]));
        context.PSSetConstantBuffers(0, Some(&[buffer]));
        context.OMSetRenderTargets(Some(&[rtv]), None);
        context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: desc.Width as f32,
            Height: desc.Height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        context.Draw(3, 0);
        // 後で同じコンテキストを使う描画（範囲選択の画面）にこの状態を残さない
        context.ClearState();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use shunsoku_core::capture::hdr::to_sdr_bgra;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_USAGE_STAGING, D3D11CreateDevice,
    };

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
