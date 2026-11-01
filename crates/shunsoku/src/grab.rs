//! 取得: Desktop Duplication で全モニタの画面を並列に取り、GPU 上の自前のテクスチャに写す（ADR-0002）。
//!
//! - モニタ（DXGI の出力）はアダプタに属する。D3D11 のデバイスはアダプタごとに 1 つ作り、`Gpu` に
//!   持ち続ける（作り直すと範囲選択の画面が出るまでが遅くなるため）。閉じたあとは `Gpu::trim` で返す。
//! - 取得はモニタごとに `std::thread::scope` のスレッドで行い、UI スレッドはその場で全部を待つ。
//!   スレッドのパニックは `join` の `Err` になり、その取得の失敗として返す（常駐は続く）。
//! - Duplication はフレームを自前のテクスチャへ写したらすぐ閉じる。開いたままにすると常駐時の
//!   メモリが増える（ADR-0002）。

use std::ffi::c_void;
use std::fmt;
use std::thread;

use shunsoku_core::geometry::{Point, Rect};
use shunsoku_core::image::{Fragment, Image};

use windows::Win32::Foundation::{E_ACCESSDENIED, HMODULE, LUID, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY,
    DXGI_MODE_ROTATION_UNSPECIFIED,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter1, IDXGIDevice3, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI;
use windows::core::{Interface, Result};

/// 開いた直後の空のフレーム（`LastPresentTime == 0`）を捨てて取り直す回数の上限。
const MAX_ACQUIRE_TRIES: u32 = 10;
/// 1 回の `AcquireNextFrame` で待つ時間（ms）。
const ACQUIRE_TIMEOUT_MS: u32 = 500;
/// 画面の切り替え（`DXGI_ERROR_ACCESS_LOST`）のあとに Duplication を開き直す回数の上限。
const MAX_REOPEN: u32 = 2;

/// アダプタごとの D3D11 のデバイス。常駐中は持ち続ける。
#[derive(Default)]
pub struct Gpu {
    adapters: Vec<AdapterDevice>,
}

struct AdapterDevice {
    luid: LUID,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

/// 1 モニタ分の、固定された画面。
pub struct Shot {
    /// モニタの位置と大きさ（スクリーン座標、物理ピクセル）。
    pub rect: RECT,
    /// モニタの実効 DPI（100% で 96）。
    pub dpi: u32,
    /// このモニタが属するアダプタのデバイス。表示のスワップチェーンもこのデバイスで作る。
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// 画面の画像（`rect` と同じ大きさ、`DXGI_FORMAT_B8G8R8A8_UNORM`、`D3D11_USAGE_DEFAULT`）。
    /// 切り抜くときは、ここから重なる部分だけを staging のテクスチャへ写して CPU に書き戻す。
    /// None は、この画像をそのまま出せないモニタ（回転したモニタ、8bit BGRA でない HDR のモニタ）。
    pub texture: Option<ID3D11Texture2D>,
}

impl Shot {
    /// `area`（スクリーン座標）のうちこのモニタに重なる部分を CPU に書き戻す。重なる部分がないか、
    /// このモニタの画像を出せない（`texture` が None）ときは None。
    ///
    /// UI スレッドで呼ぶ（取得のスレッドはもう終わっている）。
    pub fn read_back(&self, area: Rect) -> Result<Option<Fragment>> {
        let Some(texture) = &self.texture else {
            return Ok(None);
        };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: 説明を読むだけ
        unsafe { texture.GetDesc(&mut desc) };
        // 画像がモニタより小さいこともあり得るので、画像のある部分に収める
        let origin = Point::new(self.rect.left, self.rect.top);
        let shown = Rect::new(
            self.rect.left,
            self.rect.top,
            self.rect.right.min(origin.x + desc.Width as i32),
            self.rect.bottom.min(origin.y + desc.Height as i32),
        );
        let Some(area) = area.intersect(&shown) else {
            return Ok(None);
        };
        let local = area.offset(-origin.x, -origin.y);
        let size = area.size();
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: size.width,
            Height: size.height,
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
            left: local.left as u32,
            top: local.top as u32,
            front: 0,
            right: local.right as u32,
            bottom: local.bottom as u32,
            back: 1,
        };
        // SAFETY: 同じデバイスのテクスチャ同士で写し、写した先を読み終えてから Unmap する
        unsafe {
            let mut staging = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
            let staging = staging.ok_or(windows::Win32::Foundation::E_POINTER)?;
            self.context
                .CopySubresourceRegion(&staging, 0, 0, 0, 0, texture, 0, Some(&source));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let stride = mapped.RowPitch as usize;
            let len = stride * (size.height as usize - 1) + size.width as usize * 4;
            let data = std::slice::from_raw_parts(mapped.pData as *const u8, len);
            let image = Image::from_opaque_rows(size, stride, data);
            self.context.Unmap(&staging, 0);
            let image = image.ok_or(windows::Win32::Foundation::E_UNEXPECTED)?;
            Ok(Some(Fragment {
                at: area.origin(),
                image,
            }))
        }
    }
}

/// 取得の失敗。
#[derive(Debug)]
pub enum GrabError {
    /// 撮れるモニタがない。
    NoOutputs,
    /// いまは撮れない（ロック画面や UAC の安全なデスクトップなど）。
    AccessDenied,
    Windows(windows::core::Error),
    /// 取得のスレッドを作れなかった。
    Spawn(std::io::Error),
    /// 取得のスレッドがパニックした。
    Panicked,
}

impl fmt::Display for GrabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrabError::NoOutputs => f.write_str("no output is attached to the desktop"),
            GrabError::AccessDenied => f.write_str("desktop duplication is not available now"),
            GrabError::Windows(e) => write!(f, "{e}"),
            GrabError::Spawn(e) => write!(f, "could not start a grab thread: {e}"),
            GrabError::Panicked => f.write_str("a grab thread panicked"),
        }
    }
}

impl From<windows::core::Error> for GrabError {
    fn from(e: windows::core::Error) -> GrabError {
        if e.code() == E_ACCESSDENIED {
            GrabError::AccessDenied
        } else {
            GrabError::Windows(e)
        }
    }
}

/// COM のポインタを scope のスレッドへ渡す。windows-rs の COM のインターフェースは `!Send`。
struct Raw(*mut c_void);
// SAFETY: 渡す先は D3D11 のデバイス（SetMultithreadProtected 済み）と DXGI の出力で、どちらも
// 複数のスレッドから呼べる。参照は scope の間だけ借り、持ち主は UI スレッドに残る
unsafe impl Send for Raw {}

impl Gpu {
    /// 全モニタの画面を並列に取得する。
    pub fn grab(&mut self) -> std::result::Result<Vec<Shot>, GrabError> {
        // 抜き差しや配置の変更を拾うため、ファクトリは毎回作り直す（ファクトリは作った時点の一覧を持つ）
        // SAFETY: 引数なしの生成
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
        let mut jobs: Vec<(usize, IDXGIOutput1, RECT, u32)> = Vec::new();
        let mut index = 0;
        // SAFETY: 列挙は Err（DXGI_ERROR_NOT_FOUND）で終わる
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
            index += 1;
            let outputs = attached_outputs(&adapter)?;
            if outputs.is_empty() {
                continue;
            }
            let device = self.device_for(&adapter)?;
            jobs.extend(
                outputs
                    .into_iter()
                    .map(|(o, rect, dpi)| (device, o, rect, dpi)),
            );
        }
        if jobs.is_empty() {
            return Err(GrabError::NoOutputs);
        }

        let results: Vec<std::result::Result<Grabbed, GrabError>> = thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|(device, output, _, _)| {
                    let device = Raw(self.adapters[*device].device.as_raw());
                    let output = Raw(output.as_raw());
                    thread::Builder::new()
                        .name("grab".into())
                        .spawn_scoped(scope, move || {
                            let (device, output) = (device, output);
                            // SAFETY: どちらも scope の外で UI スレッドが持っていて、scope の間は生きている
                            let (device, output) = unsafe {
                                (
                                    ID3D11Device::from_raw_borrowed(&device.0),
                                    IDXGIOutput1::from_raw_borrowed(&output.0),
                                )
                            };
                            let (Some(device), Some(output)) = (device, output) else {
                                return Err(GrabError::NoOutputs);
                            };
                            grab_output(device, output).map(|g| g.into_raw())
                        })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| match h {
                    Ok(h) => h.join().unwrap_or(Err(GrabError::Panicked)),
                    Err(e) => Err(GrabError::Spawn(e)),
                })
                .map(|r| r.map(RawGrabbed::into_grabbed))
                .collect()
        });

        let mut shots = Vec::with_capacity(jobs.len());
        // 失敗が 1 つでもあれば全体を失敗にする（欠けた画面で範囲を選ばせない）。
        // 先に成功したモニタのテクスチャは shots ごと捨てる
        for ((device, _, rect, dpi), result) in jobs.into_iter().zip(results) {
            let grabbed = result?;
            let adapter = &self.adapters[device];
            shots.push(Shot {
                rect,
                dpi,
                device: adapter.device.clone(),
                context: adapter.context.clone(),
                texture: grabbed.texture,
            });
        }
        Ok(shots)
    }

    /// 範囲選択の画面を閉じたあと（テクスチャとスワップチェーンを解放したあと）に呼び、
    /// ドライバが持っている一時的なメモリを返す。
    pub fn trim(&self) {
        for adapter in &self.adapters {
            // SAFETY: UI スレッドのデバイスに対する呼び出し
            unsafe {
                adapter.context.ClearState();
                adapter.context.Flush();
                if let Ok(dxgi) = adapter.device.cast::<IDXGIDevice3>() {
                    dxgi.Trim();
                }
            }
        }
    }

    /// アダプタのデバイスの番号を返す。なければ作る。失われたデバイス（ドライバの更新など）は作り直す。
    fn device_for(&mut self, adapter: &IDXGIAdapter1) -> Result<usize> {
        // SAFETY: アダプタの説明を読むだけ
        let luid = unsafe { adapter.GetDesc1()? }.AdapterLuid;
        self.adapters.retain(|a| {
            // SAFETY: デバイスの状態を読むだけ
            let alive = unsafe { a.device.GetDeviceRemovedReason() }.is_ok();
            if !alive {
                log::warn!("D3D11 device was removed; re-creating it");
            }
            alive
        });
        if let Some(i) = self.adapters.iter().position(|a| same_luid(a.luid, luid)) {
            return Ok(i);
        }
        let mut device = None;
        let mut context = None;
        // SAFETY: 出力が属するアダプタでデバイスを作る（別のアダプタのデバイスでは DuplicateOutput が失敗する）
        unsafe {
            D3D11CreateDevice(
                adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let (Some(device), Some(context)) = (device, context) else {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        };
        // 取得のスレッドと UI スレッドの両方から使う
        if let Ok(mt) = device.cast::<ID3D11Multithread>() {
            // SAFETY: フラグを立てるだけ
            let _ = unsafe { mt.SetMultithreadProtected(true) };
        }
        log::info!(
            "created D3D11 device for adapter {:08x}:{:08x}",
            luid.HighPart,
            luid.LowPart
        );
        self.adapters.push(AdapterDevice {
            luid,
            device,
            context,
        });
        Ok(self.adapters.len() - 1)
    }
}

fn same_luid(a: LUID, b: LUID) -> bool {
    a.HighPart == b.HighPart && a.LowPart == b.LowPart
}

/// デスクトップにつながっている出力と、その位置（スクリーン座標、物理ピクセル）と実効 DPI。
fn attached_outputs(adapter: &IDXGIAdapter1) -> Result<Vec<(IDXGIOutput1, RECT, u32)>> {
    let mut outputs = Vec::new();
    let mut index = 0;
    // SAFETY: 列挙は Err（DXGI_ERROR_NOT_FOUND）で終わる
    while let Ok(output) = unsafe { adapter.EnumOutputs(index) } {
        index += 1;
        // SAFETY: 出力の説明を読むだけ
        let desc = unsafe { output.GetDesc()? };
        if desc.AttachedToDesktop.as_bool() {
            let (mut dpi, mut dpi_y) = (USER_DEFAULT_SCREEN_DPI, USER_DEFAULT_SCREEN_DPI);
            // SAFETY: 出力のモニタのハンドルで DPI を読むだけ。失敗したら 100% とみなす
            if unsafe { GetDpiForMonitor(desc.Monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }
                .is_err()
            {
                dpi = USER_DEFAULT_SCREEN_DPI;
            }
            outputs.push((output.cast::<IDXGIOutput1>()?, desc.DesktopCoordinates, dpi));
        }
    }
    Ok(outputs)
}

/// 1 モニタ分の取得の結果。
struct Grabbed {
    texture: Option<ID3D11Texture2D>,
}

/// スレッドから返すための `Grabbed`。
struct RawGrabbed(Option<Raw>);

impl Grabbed {
    fn into_raw(self) -> RawGrabbed {
        RawGrabbed(self.texture.map(|t| Raw(t.into_raw())))
    }
}

impl RawGrabbed {
    fn into_grabbed(self) -> Grabbed {
        Grabbed {
            // SAFETY: into_raw で手放した参照を 1 回だけ取り戻す
            texture: self
                .0
                .map(|raw| unsafe { ID3D11Texture2D::from_raw(raw.0) }),
        }
    }
}

/// 1 モニタ分を取得する。画面の切り替え（ACCESS_LOST）のあとは開き直す。
fn grab_output(
    device: &ID3D11Device,
    output: &IDXGIOutput1,
) -> std::result::Result<Grabbed, GrabError> {
    let mut reopened = 0;
    loop {
        match grab_once(device, output) {
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST && reopened < MAX_REOPEN => {
                reopened += 1;
                log::info!("desktop duplication access lost; reopening ({reopened})");
            }
            result => return result.map_err(GrabError::from),
        }
    }
}

fn grab_once(device: &ID3D11Device, output: &IDXGIOutput1) -> Result<Grabbed> {
    // SAFETY: device はこの出力が属するアダプタのもの。フレームは ReleaseFrame まで使う
    unsafe {
        let duplication = output.DuplicateOutput(device)?;
        let rotation = duplication.GetDesc().Rotation;
        let frame = acquire_frame(&duplication)?;
        let result = copy_frame(device, &frame);
        drop(frame);
        let _ = duplication.ReleaseFrame();
        let texture = result?;

        // 回転したモニタの画像は回転前の向きで来る。M1 では回して出さない（下で None にする）
        let upright =
            rotation == DXGI_MODE_ROTATION_IDENTITY || rotation == DXGI_MODE_ROTATION_UNSPECIFIED;
        let format = texture_format(&texture);
        if !upright {
            log::warn!("output is rotated ({rotation:?}); it is not shown in M1");
        }
        if format != DXGI_FORMAT_B8G8R8A8_UNORM {
            log::warn!("output format is {format:?}, not 8-bit BGRA; it is not shown in M1");
        }
        Ok(Grabbed {
            texture: (upright && format == DXGI_FORMAT_B8G8R8A8_UNORM).then_some(texture),
        })
    }
}

/// 画面の画像が届いたフレームを取る。返したフレームは呼び出し側が ReleaseFrame する。
fn acquire_frame(duplication: &IDXGIOutputDuplication) -> Result<ID3D11Texture2D> {
    let mut last = None;
    for _ in 0..MAX_ACQUIRE_TRIES {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: 前のフレームは ReleaseFrame 済み
        match unsafe { duplication.AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut info, &mut resource) }
        {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(e) => return Err(e),
        }
        let Some(resource) = resource else {
            // SAFETY: 取れたフレームを返す
            let _ = unsafe { duplication.ReleaseFrame() };
            continue;
        };
        // 開いた直後のフレームは画面の画像がまだ来ていない（空）。次のフレームを待つ
        if info.LastPresentTime != 0 {
            return resource.cast();
        }
        last = Some(info.LastPresentTime);
        drop(resource);
        // SAFETY: 取れたフレームを返す
        unsafe { duplication.ReleaseFrame()? };
    }
    log::warn!("no desktop image arrived (last present time: {last:?})");
    Err(DXGI_ERROR_WAIT_TIMEOUT.into())
}

/// フレームを自前のテクスチャに写す。写したあとはフレームを返してよい。
fn copy_frame(device: &ID3D11Device, frame: &ID3D11Texture2D) -> Result<ID3D11Texture2D> {
    // SAFETY: 同じデバイスのテクスチャ同士で写す。コンテキストは SetMultithreadProtected 済み
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        frame.GetDesc(&mut desc);
        let desc = D3D11_TEXTURE2D_DESC {
            MipLevels: 1,
            ArraySize: 1,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: 0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            ..desc
        };
        let mut texture = None;
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.ok_or(windows::Win32::Foundation::E_POINTER)?;
        let context = device.GetImmediateContext()?;
        context.CopyResource(&texture, frame);
        Ok(texture)
    }
}

fn texture_format(texture: &ID3D11Texture2D) -> DXGI_FORMAT {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: 説明を読むだけ
    unsafe { texture.GetDesc(&mut desc) };
    desc.Format
}
