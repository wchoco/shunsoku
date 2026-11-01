// PROTOTYPE — 捨てるコード。shunsoku #5 の計測用（Rust + windows-rs で Win32 を直接呼ぶ案）。
// #14 で追加: --par（Desktop Duplication をモニタごとに並列取得）、
//   --display gdi|gpu-per|gpu-one（範囲選択の画面を GDI で描くか、GPU に置いたまま flip-model の
//   スワップチェーンで出すか。gpu-per はモニタごとに 1 枚、gpu-one は仮想スクリーン全体で 1 枚）。
// 常駐 → グローバルホットキー (Ctrl+Shift+F9) → 画面を固定して範囲選択 → 最前面のスクラップ表示。
// 起動引数: --capture dd|gdi|wgc   環境変数 SHUNSOKU_BENCH_LOG に計測ログを追記する。
// スクラップ: ドラッグで移動 / ホイールで拡大縮小 / 右クリックで半透明切替 / ダブルクリックか Esc で閉じる。
// トレイアイコンを右クリックで終了。
#![windows_subsystem = "windows"]

use std::{cell::RefCell, ffi::c_void, fs::File, io::Write, mem::size_of, ptr::null_mut};

use windows::{
    core::*,
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem},
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::*,
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::*,
            Dwm::DwmFlush,
            Dxgi::{Common::*, *},
            Gdi::*,
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            Threading::GetCurrentProcessId,
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
                RoInitialize, RO_INIT_SINGLETHREADED,
            },
        },
        UI::{
            HiDpi::*,
            Input::KeyboardAndMouse::*,
            Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

const HOTKEY_ID: i32 = 1;
const WM_TRAY: u32 = WM_APP + 1;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Method {
    Dd,
    DdKeep,
    Gdi,
    Wgc,
}

struct Dib {
    bmp: HBITMAP,
    bits: *mut u8,
    w: i32,
    h: i32,
}

struct Adapter {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    outputs: Vec<IDXGIOutput1>,
}

struct Overlay {
    hwnd: HWND,
    x: i32,
    y: i32,
    dib: Dib,
    memdc: HDC,
    old: HGDIOBJ,
    drag: Option<(POINT, POINT)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Display {
    Gdi,
    GpuPer,
    GpuOne,
}

/// GPU 表示用: モニタ 1 枚分の固定画像（GPU 上）。
struct Shot {
    a: usize,
    rect: RECT,
    tex: ID3D11Texture2D,
}

/// GPU 表示用: 範囲選択のウィンドウ 1 枚とそのスワップチェーン。
struct Surface {
    hwnd: HWND,
    rect: RECT,
    a: usize,
    swap: IDXGISwapChain1,
    bb: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
}

struct GpuOverlay {
    shots: Vec<Shot>,
    surfaces: Vec<Surface>,
    drag: Option<(POINT, POINT)>,
}

struct App {
    method: Method,
    display: Display,
    par: bool,
    gpu: Option<GpuOverlay>,
    adapters: Vec<Adapter>,
    wgc_dev: Option<IDirect3DDevice>,
    overlay: Option<Overlay>,
    n: u32,
    keep: Vec<Kept>,
    prewarm: Option<HWND>,
    trim: bool,
    lazy: bool,
}

/// dd-keep 用: 常駐中も開いたままの Duplication と、直前の画像を持つ staging テクスチャ。
struct Kept {
    adapter: usize,
    dup: IDXGIOutputDuplication,
    staging: ID3D11Texture2D,
    rect: RECT,
}

struct Scrap {
    dib: Dib,
    scale: f32,
    alpha: u8,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static LOG: RefCell<Option<File>> = const { RefCell::new(None) };
}

fn qpc() -> i64 {
    let mut v = 0;
    unsafe { QueryPerformanceCounter(&mut v).unwrap() };
    v
}

fn log(line: String) {
    LOG.with(|l| {
        if let Some(f) = l.borrow_mut().as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    });
}

fn hinst() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).unwrap().into() }
}

fn new_dib(w: i32, h: i32) -> Dib {
    let bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = null_mut();
    let bmp = unsafe { CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap() };
    Dib { bmp, bits: bits as *mut u8, w, h }
}

fn virtual_screen() -> RECT {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        RECT {
            left: x,
            top: y,
            right: x + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            bottom: y + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

// ---------- キャプチャ ----------

fn init_d3d() -> Result<Vec<Adapter>> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut out = vec![];
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let mut outputs = vec![];
            let mut j = 0;
            while let Ok(o) = adapter.EnumOutputs(j) {
                j += 1;
                if o.GetDesc()?.AttachedToDesktop.as_bool() {
                    outputs.push(o.cast::<IDXGIOutput1>()?);
                }
            }
            if outputs.is_empty() {
                continue;
            }
            let mut dev = None;
            let mut ctx = None;
            D3D11CreateDevice(
                &adapter.cast::<IDXGIAdapter>()?,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            )?;
            let dev = dev.unwrap();
            if let Ok(mt) = dev.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            out.push(Adapter { dev, ctx: ctx.unwrap(), outputs });
        }
        Ok(out)
    }
}

/// GPU テクスチャを staging 経由で DIB の (left, top) に書き写す。
unsafe fn copy_tex(a: &Adapter, tex: &ID3D11Texture2D, left: i32, top: i32, dst: &Dib) -> Result<()> {
    let mut td = D3D11_TEXTURE2D_DESC::default();
    tex.GetDesc(&mut td);
    td.Usage = D3D11_USAGE_STAGING;
    td.BindFlags = 0;
    td.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    td.MiscFlags = 0;
    td.MipLevels = 1;
    td.ArraySize = 1;
    let mut staging = None;
    a.dev.CreateTexture2D(&td, None, Some(&mut staging))?;
    let staging = staging.unwrap();
    a.ctx.CopyResource(&staging, tex);
    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
    a.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
    let w = (td.Width as i32).min(dst.w - left);
    let h = (td.Height as i32).min(dst.h - top);
    for y in 0..h {
        let src = (m.pData as *const u8).add(y as usize * m.RowPitch as usize);
        let d = dst.bits.add((((top + y) * dst.w + left) * 4) as usize);
        std::ptr::copy_nonoverlapping(src, d, (w * 4) as usize);
    }
    a.ctx.Unmap(&staging, 0);
    Ok(())
}

fn capture_gdi(vs: RECT, dib: &Dib) {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let old = SelectObject(mem, dib.bmp.into());
        let _ = BitBlt(mem, 0, 0, dib.w, dib.h, Some(screen), vs.left, vs.top, SRCCOPY | CAPTUREBLT);
        SelectObject(mem, old);
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }
}

fn capture_dd(app: &App, vs: RECT, dib: &Dib) -> Result<()> {
    unsafe {
        for a in &app.adapters {
            for o in &a.outputs {
                let t0 = qpc();
                let r = o.GetDesc()?.DesktopCoordinates;
                let dup = o.DuplicateOutput(&a.dev)?;
                let t1 = qpc();
                // 開いた直後のフレームは LastPresentTime=0（画面の画像が未着で、テクスチャは空）。
                // 画像が届いたフレームが来るまで取り直す。
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut res = None;
                let mut tries = 0;
                loop {
                    tries += 1;
                    dup.AcquireNextFrame(500, &mut info, &mut res)?;
                    if info.LastPresentTime != 0 || tries >= 10 {
                        break;
                    }
                    res = None;
                    dup.ReleaseFrame()?;
                }
                let t2 = qpc();
                let tex: ID3D11Texture2D = res.unwrap().cast()?;
                copy_tex(a, &tex, r.left - vs.left, r.top - vs.top, dib)?;
                dup.ReleaseFrame()?;
                let t3 = qpc();
                let f = freq() as f64 / 1000.0;
                log(format!(
                    "dd-detail dup={:.1} acquire={:.1} copy={:.1} tries={tries} last_present={} accum={} nonblack={:.3}",
                    (t1 - t0) as f64 / f,
                    (t2 - t1) as f64 / f,
                    (t3 - t2) as f64 / f,
                    info.LastPresentTime,
                    info.AccumulatedFrames,
                    nonblack(dib, r.left - vs.left, r.top - vs.top, r.right - r.left, r.bottom - r.top)
                ));
            }
        }
        Ok(())
    }
}

struct Acquired {
    a: usize,
    rect: RECT,
    dup: IDXGIOutputDuplication,
    tex: ID3D11Texture2D,
}

struct Raw(*mut c_void);
unsafe impl Send for Raw {}

/// 1 モニタ分: Duplication を開き、画像が届いたフレームを取る。(dup, tex, tries, dup_ms, acquire_ms)
unsafe fn acquire_one(dev: &ID3D11Device, o: &IDXGIOutput1) -> Result<(IDXGIOutputDuplication, ID3D11Texture2D, u32, f64, f64)> {
    let f = freq() as f64 / 1000.0;
    let t0 = qpc();
    let dup = o.DuplicateOutput(dev)?;
    let t1 = qpc();
    let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut res = None;
    let mut tries = 0;
    loop {
        tries += 1;
        dup.AcquireNextFrame(500, &mut info, &mut res)?;
        if info.LastPresentTime != 0 || tries >= 10 {
            break;
        }
        res = None;
        dup.ReleaseFrame()?;
    }
    let t2 = qpc();
    let tex: ID3D11Texture2D = res.unwrap().cast()?;
    Ok((dup, tex, tries, (t1 - t0) as f64 / f, (t2 - t1) as f64 / f))
}

/// 全モニタの Duplication フレームを取る（par なら並列）。呼び出し側で ReleaseFrame する。
fn acquire_all(app: &App, par: bool) -> Result<Vec<Acquired>> {
    let mut jobs = vec![];
    unsafe {
        for (i, a) in app.adapters.iter().enumerate() {
            for o in &a.outputs {
                jobs.push((i, o.GetDesc()?.DesktopCoordinates, o.clone()));
            }
        }
    }
    let t0 = qpc();
    let results: Vec<std::result::Result<(Raw, Raw, u32, f64, f64), HRESULT>> = if par {
        std::thread::scope(|sc| {
            let hs: Vec<_> = jobs
                .iter()
                .map(|(i, _, o)| {
                    let dev = Raw(app.adapters[*i].dev.as_raw());
                    let out = Raw(o.as_raw());
                    sc.spawn(move || unsafe {
                        let (dev, out) = (dev, out);
                        let d = ID3D11Device::from_raw_borrowed(&dev.0).unwrap();
                        let o = IDXGIOutput1::from_raw_borrowed(&out.0).unwrap();
                        acquire_one(d, o)
                            .map(|(a, b, n, x, y)| (Raw(a.into_raw()), Raw(b.into_raw()), n, x, y))
                            .map_err(|e| e.code())
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        })
    } else {
        jobs.iter()
            .map(|(i, _, o)| unsafe {
                acquire_one(&app.adapters[*i].dev, o)
                    .map(|(a, b, n, x, y)| (Raw(a.into_raw()), Raw(b.into_raw()), n, x, y))
                    .map_err(|e| e.code())
            })
            .collect()
    };
    let total = (qpc() - t0) as f64 * 1000.0 / freq() as f64;
    let mut v = vec![];
    let mut detail = String::new();
    for ((i, rect, _), r) in jobs.into_iter().zip(results) {
        let (dup, tex, n, x, y) = r.map_err(Error::from)?;
        detail += &format!(" [dup={x:.1} acquire={y:.1} tries={n}]");
        unsafe {
            v.push(Acquired {
                a: i,
                rect,
                dup: IDXGIOutputDuplication::from_raw(dup.0),
                tex: ID3D11Texture2D::from_raw(tex.0),
            });
        }
    }
    log(format!("acquire par={par} total={total:.1}{detail}"));
    Ok(v)
}

/// --par で GDI 表示: 並列に取ってから CPU に書き戻す。
fn capture_dd_par(app: &App, vs: RECT, dib: &Dib) -> Result<()> {
    let t0 = qpc();
    let acq = acquire_all(app, true)?;
    let t1 = qpc();
    unsafe {
        for q in &acq {
            copy_tex(&app.adapters[q.a], &q.tex, q.rect.left - vs.left, q.rect.top - vs.top, dib)?;
            q.dup.ReleaseFrame()?;
        }
    }
    let f = freq() as f64 / 1000.0;
    log(format!("ddpar-detail acquire={:.1} readback={:.1}", (t1 - t0) as f64 / f, (qpc() - t1) as f64 / f));
    Ok(())
}

unsafe fn staging_like(a: &Adapter, tex: &ID3D11Texture2D) -> Result<ID3D11Texture2D> {
    let mut td = D3D11_TEXTURE2D_DESC::default();
    tex.GetDesc(&mut td);
    td.Usage = D3D11_USAGE_STAGING;
    td.BindFlags = 0;
    td.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    td.MiscFlags = 0;
    td.MipLevels = 1;
    td.ArraySize = 1;
    let mut st = None;
    a.dev.CreateTexture2D(&td, None, Some(&mut st))?;
    Ok(st.unwrap())
}

fn init_keep(adapters: &[Adapter]) -> Result<Vec<Kept>> {
    let mut v = vec![];
    unsafe {
        for (i, a) in adapters.iter().enumerate() {
            for o in &a.outputs {
                let rect = o.GetDesc()?.DesktopCoordinates;
                let dup = o.DuplicateOutput(&a.dev)?;
                let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
                let mut res = None;
                for _ in 0..10 {
                    dup.AcquireNextFrame(1000, &mut info, &mut res)?;
                    if info.LastPresentTime != 0 {
                        break;
                    }
                    res = None;
                    dup.ReleaseFrame()?;
                }
                let tex: ID3D11Texture2D = res.unwrap().cast()?;
                let staging = staging_like(a, &tex)?;
                a.ctx.CopyResource(&staging, &tex);
                dup.ReleaseFrame()?;
                v.push(Kept { adapter: i, dup, staging, rect });
            }
        }
    }
    Ok(v)
}

/// 開いたままの Duplication から、更新があればそれを、なければ手元の直前の画像を使う。
fn capture_dd_keep(app: &App, vs: RECT, dib: &Dib) -> Result<()> {
    unsafe {
        for k in &app.keep {
            let a = &app.adapters[k.adapter];
            let t0 = qpc();
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res = None;
            let fresh = match k.dup.AcquireNextFrame(0, &mut info, &mut res) {
                Ok(()) => {
                    let has_image = info.LastPresentTime != 0;
                    if has_image {
                        let tex: ID3D11Texture2D = res.unwrap().cast()?;
                        a.ctx.CopyResource(&k.staging, &tex);
                    }
                    k.dup.ReleaseFrame()?;
                    has_image
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => false,
                Err(e) => return Err(e),
            };
            let t1 = qpc();
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            a.ctx.Map(&k.staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
            let (left, top) = (k.rect.left - vs.left, k.rect.top - vs.top);
            let w = (k.rect.right - k.rect.left).min(dib.w - left);
            let h = (k.rect.bottom - k.rect.top).min(dib.h - top);
            for y in 0..h {
                let src = (m.pData as *const u8).add(y as usize * m.RowPitch as usize);
                let d = dib.bits.add((((top + y) * dib.w + left) * 4) as usize);
                std::ptr::copy_nonoverlapping(src, d, (w * 4) as usize);
            }
            a.ctx.Unmap(&k.staging, 0);
            let t2 = qpc();
            let f = freq() as f64 / 1000.0;
            log(format!(
                "ddkeep-detail fresh={fresh} acquire+gpucopy={:.1} map+cpucopy={:.1}",
                (t1 - t0) as f64 / f,
                (t2 - t1) as f64 / f
            ));
        }
        Ok(())
    }
}

/// 診断用: 指定矩形の中で黒でない画素の割合（間引いて数える）。
fn nonblack(d: &Dib, l: i32, t: i32, w: i32, h: i32) -> f64 {
    let (mut n, mut nb) = (0u64, 0u64);
    unsafe {
        for y in (t..t + h).step_by(17) {
            for x in (l..l + w).step_by(17) {
                let p = d.bits.add(((y * d.w + x) * 4) as usize);
                n += 1;
                if *p != 0 || *p.add(1) != 0 || *p.add(2) != 0 {
                    nb += 1;
                }
            }
        }
    }
    nb as f64 / n.max(1) as f64
}

fn monitors() -> Vec<(HMONITOR, RECT)> {
    unsafe extern "system" fn cb(m: HMONITOR, _: HDC, r: *mut RECT, lp: LPARAM) -> BOOL {
        let v = &mut *(lp.0 as *mut Vec<(HMONITOR, RECT)>);
        v.push((m, *r));
        TRUE
    }
    let mut v: Vec<(HMONITOR, RECT)> = vec![];
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

fn capture_wgc(app: &App, vs: RECT, dib: &Dib) -> Result<()> {
    let a = &app.adapters[0];
    let d3d = app.wgc_dev.as_ref().unwrap();
    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    for (hmon, r) in monitors() {
        unsafe {
            let item: GraphicsCaptureItem = interop.CreateForMonitor(hmon)?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                d3d,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                1,
                item.Size()?,
            )?;
            let session = pool.CreateCaptureSession(&item)?;
            session.SetIsCursorCaptureEnabled(false)?;
            if let Err(e) = session.SetIsBorderRequired(false) {
                log(format!("wgc SetIsBorderRequired(false) failed: {e}"));
            }
            session.StartCapture()?;
            let start = qpc();
            let frame = loop {
                if let Ok(f) = pool.TryGetNextFrame() {
                    break f;
                }
                if qpc() - start > 2 * freq() {
                    return Err(Error::new(E_FAIL, "wgc timeout"));
                }
                std::thread::sleep(std::time::Duration::from_micros(200));
            };
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let tex: ID3D11Texture2D = access.GetInterface()?;
            copy_tex(a, &tex, r.left - vs.left, r.top - vs.top, dib)?;
            frame.Close()?;
            session.Close()?;
            pool.Close()?;
        }
    }
    Ok(())
}

fn freq() -> i64 {
    let mut f = 0;
    unsafe { QueryPerformanceFrequency(&mut f).unwrap() };
    f
}

// ---------- 範囲選択の画面 ----------

fn on_hotkey() {
    let (busy, display) = APP.with(|a| {
        let b = a.borrow();
        let app = b.as_ref().unwrap();
        (app.overlay.is_some() || app.gpu.is_some(), app.display)
    });
    if busy {
        return;
    }
    if display != Display::Gdi {
        on_hotkey_gpu(display);
        return;
    }
    let t_hot = qpc();
    let vs = virtual_screen();
    let dib = new_dib(vs.right - vs.left, vs.bottom - vs.top);
    let (method, n) = APP.with(|a| {
        let mut b = a.borrow_mut();
        let app = b.as_mut().unwrap();
        app.n += 1;
        if app.lazy && app.adapters.is_empty() {
            let t = qpc();
            app.adapters = init_d3d().unwrap();
            log(format!("lazy-device create={:.1}", (qpc() - t) as f64 * 1000.0 / freq() as f64));
        }
        let r = match app.method {
            Method::Gdi => {
                capture_gdi(vs, &dib);
                Ok(())
            }
            Method::Dd if app.par => capture_dd_par(app, vs, &dib),
            Method::Dd => capture_dd(app, vs, &dib),
            Method::DdKeep => capture_dd_keep(app, vs, &dib),
            Method::Wgc => capture_wgc(app, vs, &dib),
        };
        if let Err(e) = r {
            log(format!("capture error: {e}"));
        }
        log(format!("captured nonblack={:.3}", nonblack(&dib, 0, 0, dib.w, dib.h)));
        if app.trim {
            unsafe {
                for a in &app.adapters {
                    a.ctx.ClearState();
                    a.ctx.Flush();
                    if let Ok(d3) = a.dev.cast::<IDXGIDevice3>() {
                        d3.Trim();
                    }
                }
            }
        }
        if app.lazy {
            app.adapters.clear();
        }
        (app.method, app.n)
    });
    let t_cap = qpc();
    unsafe {
        let memdc = CreateCompatibleDC(None);
        let old = SelectObject(memdc, dib.bmp.into());
        APP.with(|a| {
            a.borrow_mut().as_mut().unwrap().overlay = Some(Overlay {
                hwnd: HWND::default(),
                x: vs.left,
                y: vs.top,
                dib,
                memdc,
                old,
                drag: None,
            })
        });
        let pre = APP.with(|a| a.borrow().as_ref().unwrap().prewarm);
        let hwnd = match pre {
            Some(h) => h,
            None => create_overlay_window(vs),
        };
        APP.with(|a| a.borrow_mut().as_mut().unwrap().overlay.as_mut().unwrap().hwnd = hwnd);
        if pre.is_some() {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                vs.left,
                vs.top,
                vs.right - vs.left,
                vs.bottom - vs.top,
                SWP_NOREDRAW,
            );
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = UpdateWindow(hwnd);
        let t_shown = qpc();
        let _ = DwmFlush();
        let t_flush = qpc();
        log(format!(
            "trigger n={n} method={method:?} hotkey={t_hot} captured={t_cap} shown={t_shown} flushed={t_flush} dpi={}",
            GetDpiForWindow(hwnd)
        ));
    }
}

fn create_overlay_window(vs: RECT) -> HWND {
    unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("proto-overlay"),
            w!("shunsoku overlay"),
            WS_POPUP,
            vs.left,
            vs.top,
            vs.right - vs.left,
            vs.bottom - vs.top,
            None,
            None,
            Some(hinst()),
            None,
        )
        .unwrap()
    }
}

fn close_overlay() -> Option<Overlay> {
    let (ov, pre) = APP.with(|a| {
        let mut b = a.borrow_mut();
        let app = b.as_mut().unwrap();
        (app.overlay.take(), app.prewarm)
    });
    let ov = ov?;
    unsafe {
        if pre.is_some() {
            let _ = ShowWindow(ov.hwnd, SW_HIDE);
        } else {
            let _ = DestroyWindow(ov.hwnd);
        }
    }
    Some(ov)
}

fn free_overlay(ov: Overlay) {
    unsafe {
        SelectObject(ov.memdc, ov.old);
        let _ = DeleteDC(ov.memdc);
        let _ = DeleteObject(ov.dib.bmp.into());
    }
}

fn lp_pt(lp: LPARAM) -> POINT {
    POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 }
}

fn norm(a: POINT, b: POINT) -> RECT {
    RECT { left: a.x.min(b.x), top: a.y.min(b.y), right: a.x.max(b.x), bottom: a.y.max(b.y) }
}

extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                APP.with(|a| {
                    let b = a.borrow();
                    if let Some(ov) = b.as_ref().and_then(|x| x.overlay.as_ref()) {
                        let _ = BitBlt(hdc, 0, 0, ov.dib.w, ov.dib.h, Some(ov.memdc), 0, 0, SRCCOPY);
                        if let Some((p, q)) = ov.drag {
                            let r = norm(p, q);
                            SelectObject(hdc, GetStockObject(NULL_BRUSH));
                            SelectObject(hdc, GetStockObject(DC_PEN));
                            SetDCPenColor(hdc, COLORREF(0x0000ff));
                            let _ = Rectangle(hdc, r.left, r.top, r.right + 1, r.bottom + 1);
                        }
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let p = lp_pt(lp);
                APP.with(|a| {
                    if let Some(ov) = a.borrow_mut().as_mut().unwrap().overlay.as_mut() {
                        ov.drag = Some((p, p));
                    }
                });
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let p = lp_pt(lp);
                APP.with(|a| {
                    if let Some(ov) = a.borrow_mut().as_mut().unwrap().overlay.as_mut() {
                        if let Some(d) = ov.drag.as_mut() {
                            d.1 = p;
                        }
                    }
                });
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                if let Some(ov) = close_overlay() {
                    if let Some((p, q)) = ov.drag {
                        let r = norm(p, q);
                        if r.right - r.left > 2 && r.bottom - r.top > 2 {
                            let crop = crop_dib(&ov.dib, r);
                            create_scrap(ov.x + r.left, ov.y + r.top, crop);
                        }
                    }
                    free_overlay(ov);
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                if let Some(ov) = close_overlay() {
                    free_overlay(ov);
                }
                LRESULT(0)
            }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                if let Some(ov) = close_overlay() {
                    free_overlay(ov);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// ---------- 範囲選択の画面（GPU に置いたまま表示） ----------

fn intersect(a: RECT, b: RECT) -> Option<RECT> {
    let r = RECT { left: a.left.max(b.left), top: a.top.max(b.top), right: a.right.min(b.right), bottom: a.bottom.min(b.bottom) };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

fn on_hotkey_gpu(display: Display) {
    let t_hot = qpc();
    let f = freq() as f64 / 1000.0;
    // 1) 撮影: Duplication のフレームを自前の GPU テクスチャへ写してすぐ解放する（CPU には書き戻さない）。
    let (shots, n) = APP.with(|a| {
        let mut b = a.borrow_mut();
        let app = b.as_mut().unwrap();
        app.n += 1;
        let acq = acquire_all(app, app.par).unwrap();
        let mut shots = vec![];
        unsafe {
            for q in acq {
                let ad = &app.adapters[q.a];
                let mut td = D3D11_TEXTURE2D_DESC::default();
                q.tex.GetDesc(&mut td);
                td.Usage = D3D11_USAGE_DEFAULT;
                td.BindFlags = 0;
                td.CPUAccessFlags = 0;
                td.MiscFlags = 0;
                td.MipLevels = 1;
                td.ArraySize = 1;
                let mut own = None;
                ad.dev.CreateTexture2D(&td, None, Some(&mut own)).unwrap();
                let own = own.unwrap();
                ad.ctx.CopyResource(&own, &q.tex);
                q.dup.ReleaseFrame().unwrap();
                if app.n == 1 {
                    log(format!("shot format={:?} {}x{}", td.Format, td.Width, td.Height));
                }
                shots.push(Shot { a: q.a, rect: q.rect, tex: own });
            }
        }
        (shots, app.n)
    });
    let t_cap = qpc();
    // 2) ウィンドウ（モニタごと or 仮想スクリーン 1 枚）を作る。APP を借りたまま CreateWindow しない。
    let areas: Vec<(RECT, usize)> = match display {
        Display::GpuPer => shots.iter().map(|s| (s.rect, s.a)).collect(),
        _ => vec![(virtual_screen(), 0)],
    };
    let hwnds: Vec<HWND> = areas
        .iter()
        .map(|(r, _)| unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("proto-gpu-overlay"),
                w!("shunsoku overlay"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                Some(hinst()),
                None,
            )
            .unwrap()
        })
        .collect();
    let t_win = qpc();
    // 3) スワップチェーンを作る。
    let surfaces = APP.with(|a| {
        let b = a.borrow();
        let app = b.as_ref().unwrap();
        areas
            .iter()
            .zip(&hwnds)
            .map(|((r, ai), h)| unsafe { make_surface(&app.adapters[*ai], *ai, *h, *r).unwrap() })
            .collect::<Vec<_>>()
    });
    let t_swap = qpc();
    APP.with(|a| {
        a.borrow_mut().as_mut().unwrap().gpu = Some(GpuOverlay { shots, surfaces, drag: None });
    });
    // 4) 表示して描く。
    unsafe {
        for h in &hwnds {
            let _ = ShowWindow(*h, SW_SHOW);
        }
        let _ = SetForegroundWindow(hwnds[0]);
    }
    let t_show = qpc();
    redraw_gpu();
    let t_shown = qpc();
    unsafe {
        let _ = DwmFlush();
    }
    let t_flush = qpc();
    log(format!(
        "gpu-detail windows={:.1} swapchain={:.1} show={:.1} draw+present={:.1}",
        (t_win - t_cap) as f64 / f,
        (t_swap - t_win) as f64 / f,
        (t_show - t_swap) as f64 / f,
        (t_shown - t_show) as f64 / f
    ));
    log(format!(
        "trigger n={n} method=Dd display={display:?} hotkey={t_hot} captured={t_cap} shown={t_shown} flushed={t_flush} dpi={}",
        unsafe { GetDpiForWindow(hwnds[0]) }
    ));
}

unsafe fn make_surface(ad: &Adapter, ai: usize, hwnd: HWND, rect: RECT) -> Result<Surface> {
    let dxgi: IDXGIDevice = ad.dev.cast()?;
    let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: (rect.right - rect.left) as u32,
        Height: (rect.bottom - rect.top) as u32,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        ..Default::default()
    };
    let swap = factory.CreateSwapChainForHwnd(&ad.dev, hwnd, &desc, None, None)?;
    let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER | DXGI_MWA_NO_WINDOW_CHANGES);
    let bb: ID3D11Texture2D = swap.GetBuffer(0)?;
    let mut rtv = None;
    ad.dev.CreateRenderTargetView(&bb, None, Some(&mut rtv))?;
    Ok(Surface { hwnd, rect, a: ai, swap, bb, rtv: rtv.unwrap() })
}

/// 全ウィンドウを描き直す: 固定画像を GPU 上でバックバッファへ写し、選択枠は ClearView の矩形で描く。
fn redraw_gpu() {
    APP.with(|a| {
        let Ok(b) = a.try_borrow() else { return };
        let app = b.as_ref().unwrap();
        let Some(g) = app.gpu.as_ref() else { return };
        unsafe {
            for s in &g.surfaces {
                let ctx = &app.adapters[s.a].ctx;
                for shot in g.shots.iter().filter(|x| x.a == s.a) {
                    if let Some(ix) = intersect(shot.rect, s.rect) {
                        let bx = D3D11_BOX {
                            left: (ix.left - shot.rect.left) as u32,
                            top: (ix.top - shot.rect.top) as u32,
                            front: 0,
                            right: (ix.right - shot.rect.left) as u32,
                            bottom: (ix.bottom - shot.rect.top) as u32,
                            back: 1,
                        };
                        ctx.CopySubresourceRegion(&s.bb, 0, (ix.left - s.rect.left) as u32, (ix.top - s.rect.top) as u32, 0, &shot.tex, 0, Some(&bx));
                    }
                }
                if let Some((p, q)) = g.drag {
                    let r = norm(p, q);
                    let edges = [
                        RECT { left: r.left, top: r.top, right: r.right + 1, bottom: r.top + 1 },
                        RECT { left: r.left, top: r.bottom, right: r.right + 1, bottom: r.bottom + 1 },
                        RECT { left: r.left, top: r.top, right: r.left + 1, bottom: r.bottom + 1 },
                        RECT { left: r.right, top: r.top, right: r.right + 1, bottom: r.bottom + 1 },
                    ];
                    let local: Vec<RECT> = edges
                        .iter()
                        .filter_map(|e| intersect(*e, s.rect))
                        .map(|e| RECT { left: e.left - s.rect.left, top: e.top - s.rect.top, right: e.right - s.rect.left, bottom: e.bottom - s.rect.top })
                        .collect();
                    if !local.is_empty() {
                        let ctx1: ID3D11DeviceContext1 = ctx.cast().unwrap();
                        ctx1.ClearView(&s.rtv, &[1.0, 0.0, 0.0, 1.0], Some(&local));
                    }
                }
                let _ = s.swap.Present(0, DXGI_PRESENT(0));
            }
        }
    });
}

/// 選択範囲だけを CPU に書き戻して DIB にする。
fn crop_gpu(app: &App, g: &GpuOverlay, r: RECT) -> Result<Dib> {
    let dib = new_dib(r.right - r.left, r.bottom - r.top);
    unsafe {
        for shot in &g.shots {
            let Some(ix) = intersect(shot.rect, r) else { continue };
            let ad = &app.adapters[shot.a];
            let td = D3D11_TEXTURE2D_DESC {
                Width: (ix.right - ix.left) as u32,
                Height: (ix.bottom - ix.top) as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut st = None;
            ad.dev.CreateTexture2D(&td, None, Some(&mut st))?;
            let st = st.unwrap();
            let bx = D3D11_BOX {
                left: (ix.left - shot.rect.left) as u32,
                top: (ix.top - shot.rect.top) as u32,
                front: 0,
                right: (ix.right - shot.rect.left) as u32,
                bottom: (ix.bottom - shot.rect.top) as u32,
                back: 1,
            };
            ad.ctx.CopySubresourceRegion(&st, 0, 0, 0, 0, &shot.tex, 0, Some(&bx));
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            ad.ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
            let (dx, dy, w) = (ix.left - r.left, ix.top - r.top, ix.right - ix.left);
            for y in 0..(ix.bottom - ix.top) {
                let src = (m.pData as *const u8).add(y as usize * m.RowPitch as usize);
                let d = dib.bits.add((((dy + y) * dib.w + dx) * 4) as usize);
                std::ptr::copy_nonoverlapping(src, d, (w * 4) as usize);
            }
            ad.ctx.Unmap(&st, 0);
        }
        set_opaque(&dib);
    }
    Ok(dib)
}

/// 範囲選択を閉じる。sel があればその範囲をスクラップにする。
fn close_gpu(sel: bool) {
    let (g, crop) = APP.with(|a| {
        let mut b = a.borrow_mut();
        let app = b.as_mut().unwrap();
        let Some(g) = app.gpu.take() else { return (None, None) };
        let crop = if sel {
            g.drag.map(|(p, q)| norm(p, q)).filter(|r| r.right - r.left > 2 && r.bottom - r.top > 2).map(|r| {
                let t = qpc();
                let d = crop_gpu(app, &g, r).unwrap();
                log(format!("crop {}x{} readback={:.2}", r.right - r.left, r.bottom - r.top, (qpc() - t) as f64 * 1000.0 / freq() as f64));
                (r, d)
            })
        } else {
            None
        };
        (Some(g), crop)
    });
    let Some(g) = g else { return };
    let hwnds: Vec<HWND> = g.surfaces.iter().map(|s| s.hwnd).collect();
    drop(g);
    unsafe {
        for h in hwnds {
            let _ = DestroyWindow(h);
        }
    }
    APP.with(|a| {
        let b = a.borrow();
        let app = b.as_ref().unwrap();
        if app.trim {
            unsafe {
                for ad in &app.adapters {
                    ad.ctx.ClearState();
                    ad.ctx.Flush();
                    if let Ok(d3) = ad.dev.cast::<IDXGIDevice3>() {
                        d3.Trim();
                    }
                }
            }
        }
    });
    if let Some((r, d)) = crop {
        create_scrap(r.left, r.top, d);
    }
}

fn screen_pt(hwnd: HWND, lp: LPARAM) -> POINT {
    let mut p = lp_pt(lp);
    unsafe {
        let _ = ClientToScreen(hwnd, &mut p);
    }
    p
}

extern "system" fn gpu_overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let p = screen_pt(hwnd, lp);
                APP.with(|a| {
                    if let Ok(mut b) = a.try_borrow_mut() {
                        if let Some(g) = b.as_mut().unwrap().gpu.as_mut() {
                            g.drag = Some((p, p));
                        }
                    }
                });
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let p = screen_pt(hwnd, lp);
                let moved = APP.with(|a| {
                    if let Ok(mut b) = a.try_borrow_mut() {
                        if let Some(d) = b.as_mut().unwrap().gpu.as_mut().and_then(|g| g.drag.as_mut()) {
                            d.1 = p;
                            return true;
                        }
                    }
                    false
                });
                if moved {
                    redraw_gpu();
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                close_gpu(true);
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                close_gpu(false);
                LRESULT(0)
            }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                close_gpu(false);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// ---------- スクラップ ----------

fn crop_dib(src: &Dib, r: RECT) -> Dib {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let d = new_dib(w, h);
    unsafe {
        for y in 0..h {
            let s = src.bits.add((((r.top + y) * src.w + r.left) * 4) as usize);
            let t = d.bits.add((y * w * 4) as usize);
            std::ptr::copy_nonoverlapping(s, t, (w * 4) as usize);
        }
        set_opaque(&d);
    }
    d
}

unsafe fn set_opaque(d: &Dib) {
    for i in 0..(d.w * d.h) as usize {
        *d.bits.add(i * 4 + 3) = 255;
    }
}

fn create_scrap(x: i32, y: i32, dib: Dib) {
    let (w, h) = (dib.w, dib.h);
    let s = Box::into_raw(Box::new(Scrap { dib, scale: 1.0, alpha: 255 }));
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            w!("proto-scrap"),
            w!("shunsoku scrap"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hinst()),
            Some(s as *const c_void),
        )
        .unwrap();
        update_layered(hwnd, &*s);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        log(format!("scrap created {w}x{h} at {x},{y} dpi={}", GetDpiForWindow(hwnd)));
    }
}

unsafe fn update_layered(hwnd: HWND, s: &Scrap) {
    let (sw, sh) = ((s.dib.w as f32 * s.scale) as i32, (s.dib.h as f32 * s.scale) as i32);
    let src = CreateCompatibleDC(None);
    let old_src = SelectObject(src, s.dib.bmp.into());
    let mut tmp = None;
    let (dc, old) = if s.scale == 1.0 {
        (src, None)
    } else {
        let t = new_dib(sw.max(1), sh.max(1));
        let dc = CreateCompatibleDC(None);
        let old = SelectObject(dc, t.bmp.into());
        SetStretchBltMode(dc, HALFTONE);
        let _ = StretchBlt(dc, 0, 0, t.w, t.h, Some(src), 0, 0, s.dib.w, s.dib.h, SRCCOPY);
        set_opaque(&t);
        tmp = Some(t);
        (dc, Some(old))
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: s.alpha,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = UpdateLayeredWindow(
        hwnd,
        None,
        None,
        Some(&SIZE { cx: sw.max(1), cy: sh.max(1) }),
        Some(dc),
        Some(&POINT::default()),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
    if let (Some(old), Some(t)) = (old, tmp) {
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(t.bmp.into());
    }
    SelectObject(src, old_src);
    let _ = DeleteDC(src);
}

unsafe fn scrap_of(hwnd: HWND) -> Option<&'static mut Scrap> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Scrap;
    p.as_mut()
}

extern "system" fn scrap_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCREATE => {
                let cs = &*(lp.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCHITTEST => LRESULT(HTCAPTION as isize),
            WM_NCLBUTTONDBLCLK => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_NCRBUTTONUP => {
                if let Some(s) = scrap_of(hwnd) {
                    s.alpha = if s.alpha == 255 { 128 } else { 255 };
                    update_layered(hwnd, s);
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16;
                if let Some(s) = scrap_of(hwnd) {
                    s.scale = (s.scale * if delta > 0 { 1.1 } else { 1.0 / 1.1 }).clamp(0.1, 8.0);
                    update_layered(hwnd, s);
                }
                LRESULT(0)
            }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                // 画素等倍を保つため、提案された矩形には従わない（観察用にログだけ残す）。
                let r = &*(lp.0 as *const RECT);
                log(format!(
                    "scrap dpichanged dpi={} suggested={}x{}",
                    wp.0 & 0xffff,
                    r.right - r.left,
                    r.bottom - r.top
                ));
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Scrap;
                if !p.is_null() {
                    let s = Box::from_raw(p);
                    let _ = DeleteObject(s.dib.bmp.into());
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// ---------- 常駐 ----------

fn tray(hwnd: HWND, add: bool) {
    unsafe {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: LoadIconW(None, IDI_APPLICATION).unwrap(),
            ..Default::default()
        };
        let tip: Vec<u16> = "shunsoku prototype (raw)".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(if add { NIM_ADD } else { NIM_DELETE }, &nid);
    }
}

extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_HOTKEY => {
                on_hotkey();
                LRESULT(0)
            }
            WM_TRAY => {
                if (lp.0 & 0xffff) as u32 == WM_RBUTTONUP {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                tray(hwnd, false);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn register(name: PCWSTR, proc_: WNDPROC, cursor: PCWSTR, dbl: bool) {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: if dbl { CS_DBLCLKS } else { WNDCLASS_STYLES(0) },
            lpfnWndProc: proc_,
            hInstance: hinst(),
            hCursor: LoadCursorW(None, cursor).unwrap(),
            lpszClassName: name,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

fn main() {
    let t_entry = qpc();
    if let Ok(p) = std::env::var("SHUNSOKU_BENCH_LOG") {
        let f = std::fs::OpenOptions::new().create(true).append(true).open(p).ok();
        LOG.with(|l| *l.borrow_mut() = f);
    }
    let args: Vec<String> = std::env::args().collect();
    let method = match args.iter().position(|a| a == "--capture").and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
        Some("gdi") => Method::Gdi,
        Some("wgc") => Method::Wgc,
        Some("dd-keep") => Method::DdKeep,
        _ => Method::Dd,
    };
    let display = match args.iter().position(|a| a == "--display").and_then(|i| args.get(i + 1)).map(|s| s.as_str()) {
        Some("gpu-per") => Display::GpuPer,
        Some("gpu-one") => Display::GpuOne,
        _ => Display::Gdi,
    };
    let par = args.iter().any(|a| a == "--par");
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut adapters = vec![];
        let mut wgc_dev = None;
        let lazy = args.iter().any(|a| a == "--lazy");
        let trim = args.iter().any(|a| a == "--trim");
        if method != Method::Gdi && !lazy {
            adapters = init_d3d().unwrap();
        }
        if method == Method::Wgc {
            RoInitialize(RO_INIT_SINGLETHREADED).unwrap();
            let dxgi: IDXGIDevice = adapters[0].dev.cast().unwrap();
            wgc_dev = Some(CreateDirect3D11DeviceFromDXGIDevice(&dxgi).unwrap().cast().unwrap());
        }
        let keep = if method == Method::DdKeep { init_keep(&adapters).unwrap() } else { vec![] };
        APP.with(|a| *a.borrow_mut() = Some(App { method, display, par, gpu: None, adapters, wgc_dev, overlay: None, n: 0, keep, prewarm: None, trim, lazy }));

        register(w!("proto-main"), Some(main_proc), IDC_ARROW, false);
        register(w!("proto-overlay"), Some(overlay_proc), IDC_CROSS, false);
        register(w!("proto-gpu-overlay"), Some(gpu_overlay_proc), IDC_CROSS, false);
        register(w!("proto-scrap"), Some(scrap_proc), IDC_SIZEALL, true);
        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("proto-main"),
            w!("shunsoku proto main"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst()),
            None,
        )
        .unwrap();
        if args.iter().any(|a| a == "--prewarm") {
            let h = create_overlay_window(virtual_screen());
            APP.with(|a| a.borrow_mut().as_mut().unwrap().prewarm = Some(h));
        }
        if let Err(e) = RegisterHotKey(Some(main), HOTKEY_ID, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, VK_F9.0 as u32) {
            log(format!("RegisterHotKey failed: {e}"));
        }
        tray(main, true);
        log(format!(
            "ready entry={t_entry} ready={} freq={} method={method:?} display={display:?} par={par} pid={}",
            qpc(),
            freq(),
            GetCurrentProcessId()
        ));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
