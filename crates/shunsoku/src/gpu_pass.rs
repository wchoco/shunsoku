//! 取得した画像を GPU 上で描き直す処理（HDR の変換、回転）に共通の部品。
//!
//! どちらも、入力のテクスチャを全画面の三角形 1 枚で新しいテクスチャに描き直す（`draw`）。
//! 処理ごとに違うのは、ピクセルシェーダー（`PixelShader`）と、それに渡す定数だけ。

use std::sync::OnceLock;

use shunsoku_core::geometry::Size;

use windows::Win32::Foundation::{E_FAIL, E_POINTER};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::core::{PCSTR, Result, s};

/// 全画面を覆う三角形 1 枚を出す頂点シェーダー。頂点は `Draw(3, 0)` の `SV_VertexID` から作る。
const VERTEX_SHADER: &str = r"
float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}
";

/// コンパイル済みの頂点シェーダー。デバイスによらないので 1 回だけ作る。
fn vertex_shader() -> Result<&'static [u8]> {
    static CODE: OnceLock<std::result::Result<Vec<u8>, String>> = OnceLock::new();
    CODE.get_or_init(|| compile(VERTEX_SHADER, "full-screen", s!("vs"), s!("vs_4_0")))
        .as_ref()
        .map(Vec::as_slice)
        .map_err(|e| windows::core::Error::new(E_FAIL, e.as_str()))
}

/// `draw` で使うピクセルシェーダー。HLSL の `source` は、入力を `Texture2D<float4> source : register(t0)`、
/// 定数を `cbuffer Params : register(b0)` で受け、`float4 ps(float4 pos : SV_Position) : SV_Target` を持つ。
/// コンパイルはデバイスによらないので、初めて使うときに 1 回だけ行う。
pub struct PixelShader {
    /// エラーの文言に使う名前。
    name: &'static str,
    source: &'static str,
    code: OnceLock<std::result::Result<Vec<u8>, String>>,
}

impl PixelShader {
    pub const fn new(name: &'static str, source: &'static str) -> Self {
        Self {
            name,
            source,
            code: OnceLock::new(),
        }
    }

    fn code(&self) -> Result<&[u8]> {
        self.code
            .get_or_init(|| compile(self.source, self.name, s!("ps"), s!("ps_4_0")))
            .as_ref()
            .map(Vec::as_slice)
            .map_err(|e| windows::core::Error::new(E_FAIL, e.as_str()))
    }
}

/// `source` を `shader` で描き直した、`size` の大きさ・`format` の形式の新しいテクスチャ
/// （`D3D11_USAGE_DEFAULT`、描画先にもシェーダーの入力にもできる）を返す。出力の画素 p の値は、
/// `shader` が `SV_Position` の p に返した値。
///
/// `constants` は `shader` の cbuffer と同じ並びの `#[repr(C)]` の構造体（パディングを持たず、
/// 大きさは 16 バイトの倍数）。`source` は読める写しを作ったら手放す（呼び出し側に残さない）。
pub fn draw<C: Copy>(
    device: &ID3D11Device,
    source: ID3D11Texture2D,
    size: Size,
    format: DXGI_FORMAT,
    shader: &PixelShader,
    constants: &C,
) -> Result<ID3D11Texture2D> {
    let vs_code = vertex_shader()?;
    let ps_code = shader.code()?;
    let source_desc = desc(&source);
    // SAFETY: 以下はすべて同じデバイスのリソースに対する呼び出し。コンテキストは
    // SetMultithreadProtected 済みで、UI スレッドから呼ぶ。定数は C の大きさだけ読ませる
    unsafe {
        let context = device.GetImmediateContext()?;
        let readable = readable(device, &context, &source, &source_desc)?;
        // 読める写しを作ったら、元のテクスチャはもう要らない
        drop(source);

        let out_desc = D3D11_TEXTURE2D_DESC {
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
            // 回したテクスチャを HDR の変換がそのまま読めるように、シェーダーからも読めるようにする
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
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
        let buffer_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<C>() as u32,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: (constants as *const C).cast(),
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
            Width: size.width as f32,
            Height: size.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        context.Draw(3, 0);
        // 後で同じコンテキストを使う描画（範囲選択の画面）にこの状態を残さない
        context.ClearState();
        Ok(out)
    }
}

/// テクスチャの説明（大きさ、形式など）。
pub fn desc(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: 説明を受け取るだけ
    unsafe { texture.GetDesc(&mut desc) };
    desc
}

/// HLSL の `source` の `entry` を `target`（`vs_4_0` など）でコンパイルする。`name` はエラーの文言に使う。
fn compile(
    source: &str,
    name: &str,
    entry: PCSTR,
    target: PCSTR,
) -> std::result::Result<Vec<u8>, String> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: ソースの長さを渡す。返ったバッファはその場で写す
    unsafe {
        let result = D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
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
                    "{name} shader did not compile ({result:?}): {message}"
                ))
            }
        }
    }
}

/// シェーダーから読める `source`。読めないテクスチャ（Desktop Duplication の写し）は、読める写しを作る。
/// `desc` は `source` の説明。
///
/// # Safety
///
/// `context` は `device` のもの。
unsafe fn readable(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    source: &ID3D11Texture2D,
    desc: &D3D11_TEXTURE2D_DESC,
) -> Result<ID3D11Texture2D> {
    if desc.BindFlags & D3D11_BIND_SHADER_RESOURCE.0 as u32 != 0 {
        return Ok(source.clone());
    }
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
        ..*desc
    };
    // SAFETY: 同じデバイスのテクスチャ同士で写す（呼び出し側の約束）
    unsafe {
        let mut copy = None;
        device.CreateTexture2D(&copy_desc, None, Some(&mut copy))?;
        let copy = copy.ok_or(E_POINTER)?;
        context.CopySubresourceRegion(&copy, 0, 0, 0, 0, source, 0, None);
        Ok(copy)
    }
}
