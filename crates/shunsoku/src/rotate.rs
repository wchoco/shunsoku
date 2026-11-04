//! 回転したモニタの画像を、GPU 上で画面で見えている向きのテクスチャにする（#132）。
//!
//! 対応は core の `capture::rotation`。シェーダーは `Rotation::source_pixel` を 1 次の式の係数にして
//! 受け取るので、向きごとの式はここには書かない。

use shunsoku_core::capture::rotation::Rotation;
use shunsoku_core::geometry::{Point, Size};

use crate::gpu_pass::{self, PixelShader};

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::core::Result;

/// 回すシェーダー。正しい向きの画素 p の値を、回転前の画素 `origin + p.x * step_x + p.y * step_y` から
/// そのまま写す（形式は入力と同じなので、値は変わらない）。定数は `Params`。
static SHADER: PixelShader = PixelShader::new(
    "rotation",
    r"
cbuffer Params : register(b0) {
    int2 origin;
    int2 step_x;
    int2 step_y;
    int2 pad;
};
Texture2D<float4> source : register(t0);

float4 ps(float4 pos : SV_Position) : SV_Target {
    int2 p = int2(pos.xy);
    return source.Load(int3(origin + p.x * step_x + p.y * step_y, 0));
}
",
);

/// `SHADER` の cbuffer `Params`。`Rotation::source_pixel` を 1 次の式の係数にしたもの。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Params {
    /// 正しい向きの (0, 0) に写る、回転前の画素。
    origin: [i32; 2],
    /// 正しい向きで x が 1 増えたときの、回転前の画素の増え方。
    step_x: [i32; 2],
    /// 正しい向きで y が 1 増えたときの、回転前の画素の増え方。
    step_y: [i32; 2],
    /// cbuffer を 16 バイトの倍数にする詰め物。
    pad: [i32; 2],
}

impl Params {
    fn new(rotation: Rotation, raw: Size) -> Self {
        let at = |x, y| rotation.source_pixel(raw, Point::new(x, y));
        let (o, x, y) = (at(0, 0), at(1, 0), at(0, 1));
        Self {
            origin: [o.x, o.y],
            step_x: [x.x - o.x, x.y - o.y],
            step_y: [y.x - o.x, y.y - o.y],
            pad: [0, 0],
        }
    }
}

/// 回転前の向きの `source`（8bit BGRA か FP16）を、画面で見えている向きの同じ形式のテクスチャ
/// （`D3D11_USAGE_DEFAULT`）にする。`Rotation::Identity` なら `source` をそのまま返す。
/// `source` は回したら手放す（呼び出し側に残さない）。
pub fn to_upright(
    device: &ID3D11Device,
    source: ID3D11Texture2D,
    rotation: Rotation,
) -> Result<ID3D11Texture2D> {
    if rotation == Rotation::Identity {
        return Ok(source);
    }
    log::info!("turning a rotated output ({rotation:?}) upright");
    let desc = gpu_pass::desc(&source);
    let raw = Size::new(desc.Width, desc.Height);
    gpu_pass::draw(
        device,
        source,
        rotation.upright_size(raw),
        desc.Format,
        &SHADER,
        &Params::new(rotation, raw),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
    };

    use crate::hdr::HDR_FORMAT;

    const ROTATIONS: [Rotation; 4] = [
        Rotation::Identity,
        Rotation::Rotate90,
        Rotation::Rotate180,
        Rotation::Rotate270,
    ];

    /// 回転前の画像の大きさ。縦横の違う小さな画像で、90° と 270° の取り違えを捕まえる。
    const RAW: Size = Size::new(3, 2);

    fn warp_device() -> ID3D11Device {
        let mut device = None;
        // SAFETY: テストの中で使うデバイスを作るだけ
        unsafe {
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
        }
        device.unwrap()
    }

    /// `pixels`（行ごとに詰めた画素）で、Desktop Duplication の写しと同じくシェーダーから読めない
    /// テクスチャを作る。
    fn texture(
        device: &ID3D11Device,
        format: DXGI_FORMAT,
        size: Size,
        pixels: &[u8],
    ) -> ID3D11Texture2D {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: size.width,
            Height: size.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: 0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr().cast(),
            SysMemPitch: pixels.len() as u32 / size.height,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        // SAFETY: init は desc の大きさの画素を指す
        unsafe {
            device
                .CreateTexture2D(&desc, Some(&init), Some(&mut texture))
                .unwrap();
        }
        texture.unwrap()
    }

    /// テクスチャの大きさと、行ごとに詰めた画素を CPU に読み戻す。
    fn read_back(
        device: &ID3D11Device,
        texture: &ID3D11Texture2D,
        bytes: usize,
    ) -> (Size, Vec<u8>) {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: テストの中で作ったデバイスとテクスチャだけを使い、読み終えてから Unmap する
        unsafe {
            texture.GetDesc(&mut desc);
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
                ..desc
            };
            let mut staging = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            let context = device.GetImmediateContext().unwrap();
            context.CopyResource(&staging, texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let row = desc.Width as usize * bytes;
            let mut pixels = Vec::with_capacity(row * desc.Height as usize);
            for y in 0..desc.Height as usize {
                let start = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                pixels.extend_from_slice(std::slice::from_raw_parts(start, row));
            }
            context.Unmap(&staging, 0);
            (Size::new(desc.Width, desc.Height), pixels)
        }
    }

    /// 1 画素 `bytes` バイトの形式 `format` で、どの画素も（どの成分も）違う値の画像を 4 つの向きで回し、
    /// 読み戻した画像が core の対応と画素単位で一致することを確かめる。
    fn matches_core(format: DXGI_FORMAT, bytes: usize, raw: &[u8]) {
        let device = warp_device();
        for rotation in ROTATIONS {
            let source = texture(&device, format, RAW, raw);
            let out = to_upright(&device, source, rotation).unwrap();
            let (size, pixels) = read_back(&device, &out, bytes);
            assert_eq!(size, rotation.upright_size(RAW), "{rotation:?}");
            for y in 0..size.height as i32 {
                for x in 0..size.width as i32 {
                    let s = rotation.source_pixel(RAW, Point::new(x, y));
                    let at = |p: Point, width: u32| {
                        let i = (p.y as usize * width as usize + p.x as usize) * bytes;
                        i..i + bytes
                    };
                    assert_eq!(
                        &pixels[at(Point::new(x, y), size.width)],
                        &raw[at(s, RAW.width)],
                        "{rotation:?} ({x}, {y})"
                    );
                }
            }
        }
    }

    /// 8bit BGRA の画像を GPU で回した結果が core の対応と一致する（GPU は WARP）。
    #[test]
    fn bgra_matches_core() {
        let raw: Vec<u8> = (0..RAW.width * RAW.height * 4)
            .map(|i| i as u8 * 9)
            .collect();
        matches_core(DXGI_FORMAT_B8G8R8A8_UNORM, 4, &raw);
    }

    /// FP16（HDR）の画像を GPU で回した結果が core の対応と一致する（GPU は WARP）。
    #[test]
    fn fp16_matches_core() {
        // 1.0 から 2^-10 ずつ増やした値。どれも FP16 の正規化数で、float を通っても変わらない
        let raw: Vec<u8> = (0..RAW.width * RAW.height * 4)
            .flat_map(|i| (0x3c00u16 + i as u16 * 7).to_le_bytes())
            .collect();
        matches_core(HDR_FORMAT, 8, &raw);
    }
}
