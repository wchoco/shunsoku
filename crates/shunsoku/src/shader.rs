//! 取得した画像を GPU 上で描き直す処理（HDR の変換、回転）に共通の部品。

use windows::Win32::Foundation::E_POINTER;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::core::{PCSTR, Result};

/// HLSL の `source` の `entry` を `target`（`vs_4_0` など）でコンパイルする。`name` はエラーの文言に使う。
pub fn compile(
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
pub unsafe fn readable(
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
