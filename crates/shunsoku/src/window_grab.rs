//! ウィンドウキャプチャの取得: `Windows.Graphics.Capture`（WGC）で 1 枚のウィンドウを 1 フレームだけ撮る（spec #34）。
//!
//! - 他のウィンドウに隠れた部分も撮れる（DWM が持っているウィンドウの中身を受け取るため）。
//! - 枠: `IsBorderRequired` があれば `SetIsBorderRequired(false)` を呼ぶ。`RequestAccessAsync(Borderless)` は
//!   呼ばない（非パッケージアプリでは可否がシステムのスイッチで決まり、プロンプトも出ない。#11 のプローブ）。
//!   枠が消せない環境（Windows 10 など）でも `PrintWindow` には切り替えない（spec #34）。
//! - フレームプールは free-threaded で作り、UI スレッドで `TryGetNextFrame` を短く待つ。メッセージは回さない。
//! - HDR のモニタ（ウィンドウとの重なりが最も大きいモニタ）では FP16（scRGB）で撮り、書き戻す前に
//!   `hdr::to_sdr_texture` でそのモニタの「SDR コンテンツの明るさ」を白にした 8bit sRGB にする。
//!   HDR と SDR のモニタ（や明るさの違う HDR のモニタ）にまたがるウィンドウも、その 1 枚のモニタで決める。
//!   WGC のウィンドウの画像は 1 枚で、どのモニタの明るさで写るかを HDR のモニタで確かめていないため、
//!   モニタごとに分けて変換はしない（確かめられたら、またがる部分ごとに変換する）。
//! - 撮ったフレームは staging のテクスチャに写して CPU に書き戻し、すぐにセッションとプールを閉じる。

use std::time::{Duration, Instant};

use shunsoku_core::capture::window::visible_bounds;
use shunsoku_core::geometry::{Rect, Size};
use shunsoku_core::image::{Fragment, Image};

use windows::Foundation::Metadata::ApiInformation;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED, HWND, RECT};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11Device, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows::core::{Error, HSTRING, Interface, Result, factory, h};

use crate::grab::{Gpu, GrabError};
use crate::hdr;

/// 最初のフレームを待つ時間の上限。
const FRAME_TIMEOUT: Duration = Duration::from_secs(1);
/// `TryGetNextFrame` を試す間隔。
const FRAME_POLL: Duration = Duration::from_millis(2);

/// 撮ったウィンドウ。
pub struct WindowShot {
    /// 撮ったときのウィンドウの見えている枠（スクリーン座標、物理ピクセル）。大きさは `image` と同じ。
    pub rect: Rect,
    pub image: Image,
}

/// `hwnd` のウィンドウを 1 フレームだけ撮る。`cursor` はカーソルを写すか（カーソルの設定が入るまでは false）。
///
/// `device` はどのアダプタのものでもよい（WGC がアダプタをまたいで写す）。
pub fn grab_window(device: &ID3D11Device, hwnd: HWND, cursor: bool) -> Result<WindowShot> {
    let origin = window_bounds(hwnd).origin();
    let dxgi: IDXGIDevice = device.cast()?;
    // SAFETY: このプロセスの D3D11 デバイスを WinRT のデバイスで包むだけ
    let winrt: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    // SAFETY: hwnd は呼び出し側が GetForegroundWindow などで得たトップレベルのウィンドウ
    let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
    // SAFETY: ウィンドウからモニタを引くだけ
    let color = hdr::monitor_color(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) });
    let (format, sdr_white) = if color.hdr {
        log::info!(
            "capturing the window in FP16 (SDR white {} nits)",
            color.sdr_white_nits
        );
        (
            DirectXPixelFormat::R16G16B16A16Float,
            Some(color.sdr_white_nits),
        )
    } else {
        (DirectXPixelFormat::B8G8R8A8UIntNormalized, None)
    };
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt, format, 1, item.Size()?)?;
    let session = pool.CreateCaptureSession(&item)?;
    let result = configure(&session, cursor)
        .and_then(|()| session.StartCapture())
        .and_then(|()| next_frame(&pool))
        .and_then(|frame| {
            let image = read_frame(device, &frame, sdr_white);
            let _ = frame.Close();
            image
        });
    let _ = session.Close();
    let _ = pool.Close();
    let image = result?;
    Ok(WindowShot {
        rect: Rect::from_origin_size(origin, image.size()),
        image,
    })
}

/// 枠とカーソルを設定する。古い Windows で項目がなければ触らない。
fn configure(session: &GraphicsCaptureSession, cursor: bool) -> Result<()> {
    const SESSION: &HSTRING = h!("Windows.Graphics.Capture.GraphicsCaptureSession");
    if ApiInformation::IsPropertyPresent(SESSION, h!("IsBorderRequired")).unwrap_or(false) {
        // 消せない環境（スイッチがオフなど）では失敗するが、そのまま枠つきで撮る
        if let Err(e) = session.SetIsBorderRequired(false) {
            log::info!("could not hide the capture border: {e}");
        }
    }
    if ApiInformation::IsPropertyPresent(SESSION, h!("IsCursorCaptureEnabled")).unwrap_or(false) {
        session.SetIsCursorCaptureEnabled(cursor)?;
    }
    Ok(())
}

/// 最初のフレームを待って取る。
fn next_frame(pool: &Direct3D11CaptureFramePool) -> Result<Direct3D11CaptureFrame> {
    let started = Instant::now();
    loop {
        // フレームがまだなければ null（windows-rs では Err）が返る
        if let Ok(frame) = pool.TryGetNextFrame() {
            log::debug!(
                "first window frame after {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
            return Ok(frame);
        }
        if started.elapsed() > FRAME_TIMEOUT {
            return Err(Error::new(E_UNEXPECTED, "no window frame arrived"));
        }
        std::thread::sleep(FRAME_POLL);
    }
}

/// フレームのうち中身のある部分（`ContentSize`）を CPU に書き戻す。`sdr_white` があれば
/// フレームは FP16 で、その明るさを白にして SDR に変換してから書き戻す。
fn read_frame(
    device: &ID3D11Device,
    frame: &Direct3D11CaptureFrame,
    sdr_white: Option<f32>,
) -> Result<Image> {
    let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
    // SAFETY: フレームのサーフェスは D3D11 のテクスチャ。フレームを閉じるまで使う
    let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
    let texture = match sdr_white {
        Some(white) => hdr::to_sdr_texture(device, &texture, white)?,
        None => texture,
    };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: 説明を読むだけ
    unsafe { texture.GetDesc(&mut desc) };
    // ウィンドウが縮んだ直後などは、プールのテクスチャのほうが中身より大きい
    let content = frame.ContentSize()?;
    let width = desc.Width.min(u32::try_from(content.Width).unwrap_or(0));
    let height = desc.Height.min(u32::try_from(content.Height).unwrap_or(0));
    if width == 0 || height == 0 {
        return Err(Error::new(E_UNEXPECTED, "the window frame is empty"));
    }
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: desc.Format,
        SampleDesc: desc.SampleDesc,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let source = D3D11_BOX {
        left: 0,
        top: 0,
        front: 0,
        right: width,
        bottom: height,
        back: 1,
    };
    // SAFETY: 同じデバイスのテクスチャ同士で写し、写した先を読み終えてから Unmap する
    unsafe {
        let context = device.GetImmediateContext()?;
        let mut staging = None;
        device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
        let staging = staging.ok_or(E_POINTER)?;
        context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, 0, Some(&source));
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let stride = mapped.RowPitch as usize;
        let len = stride * (height as usize - 1) + width as usize * 4;
        let data = std::slice::from_raw_parts(mapped.pData as *const u8, len);
        let image = Image::from_opaque_rows(Size::new(width, height), stride, data);
        context.Unmap(&staging, 0);
        image.ok_or_else(|| E_UNEXPECTED.into())
    }
}

/// ウィンドウの見えている枠（見えないリサイズの縁を除く）。WGC が撮る範囲と同じ。
pub fn window_bounds(hwnd: HWND) -> Rect {
    let mut r = RECT::default();
    // SAFETY: RECT の大きさを渡して読むだけ
    let dwm = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut r).cast(),
            size_of::<RECT>() as u32,
        )
    };
    if dwm.is_err() {
        // SAFETY: 読むだけ。失敗したら原点に置く
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
            r = RECT::default();
        }
    }
    Rect::new(r.left, r.top, r.right, r.bottom)
}

/// WGC で撮れなかったウィンドウ（UAC で昇格したウィンドウなど）の後退: ウィンドウの矩形のうちモニタに
/// 写っている部分を Desktop Duplication で撮る（spec #34、#64）。他のウィンドウに隠れた部分は、
/// 隠しているウィンドウが写る。どのモニタにも重ならなければ Ok(None)。`cursor` は `grab_window` と同じ。
pub fn grab_visible(
    gpu: &mut Gpu,
    hwnd: HWND,
    cursor: bool,
) -> std::result::Result<Option<WindowShot>, GrabError> {
    let shots = gpu.grab(cursor)?;
    let monitors: Vec<Rect> = shots
        .iter()
        .map(|s| Rect::new(s.rect.left, s.rect.top, s.rect.right, s.rect.bottom))
        .collect();
    let Some(rect) = visible_bounds(window_bounds(hwnd), &monitors) else {
        return Ok(None);
    };
    let fragments: Vec<Fragment> = shots
        .iter()
        .filter_map(|shot| match shot.read_back(rect) {
            Ok(fragment) => fragment,
            Err(e) => {
                // 撮れなかったモニタの部分は透明になる
                log::warn!("could not read back a monitor for the window: {e}");
                None
            }
        })
        .collect();
    let image = Image::compose(rect, &fragments);
    Ok(Some(WindowShot { rect, image }))
}
