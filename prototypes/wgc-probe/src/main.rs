// PROTOTYPE — 捨てるコード。shunsoku #11 用。main にはマージしない。
//
// wgc-probe border [--no-request] [--border on|off]
//   非パッケージ exe から RequestAccessAsync(Borderless) を呼び、自分のマゼンタのウィンドウを
//   WGC で撮りながら、画面 DC の BitBlt で周囲の黄色い画素を数える（枠が出ているかの機械判定）。
// wgc-probe elevated [--control]
//   自分を runas で昇格起動して緑のウィンドウ（target）を出させ、非昇格のこのプロセスから
//   WGC / PrintWindow / 画面 BitBlt で撮れるかを見る。半分は非昇格の青いウィンドウで隠す。
//   --control は昇格せずに同じことをする（比較用）。
// wgc-probe target  （内部用）

use std::{mem::size_of, time::{Duration, Instant}};

use windows::{
    core::*,
    Foundation::Metadata::ApiInformation,
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind, GraphicsCaptureItem, GraphicsCaptureSession},
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::*,
        Graphics::{Direct3D::D3D_DRIVER_TYPE_HARDWARE, Direct3D11::*, Dxgi::IDXGIDevice, Gdi::*},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS},
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
                RoInitialize, RO_INIT_MULTITHREADED,
            },
        },
        UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
    },
};

const TARGET_TITLE: PCWSTR = w!("wgc-probe target");

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    let args: Vec<String> = std::env::args().collect();
    let has = |s: &str| args.iter().any(|a| a == s);
    let val = |s: &str| args.iter().position(|a| a == s).and_then(|i| args.get(i + 1)).cloned();
    match args.get(1).map(String::as_str) {
        Some("border") => border(!has("--no-request"), val("--border").as_deref() != Some("on")),
        Some("elevated") => elevated(has("--control")),
        Some("target") => target(),
        _ => {
            println!("usage: wgc-probe border [--no-request] [--border on|off] | elevated [--control]");
            Ok(())
        }
    }
}

// ---------- 共通 ----------

fn os_info() {
    let ver = std::process::Command::new("cmd").args(["/c", "ver"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("os: {ver}");
    println!("process elevated: {}", is_elevated(unsafe { GetCurrentProcess() }));
    let has_prop = ApiInformation::IsPropertyPresent(h!("Windows.Graphics.Capture.GraphicsCaptureSession"), h!("IsBorderRequired")).unwrap_or(false);
    let has_access = ApiInformation::IsTypePresent(h!("Windows.Graphics.Capture.GraphicsCaptureAccess")).unwrap_or(false);
    println!("WGC supported: {:?}, IsBorderRequired present: {has_prop}, GraphicsCaptureAccess present: {has_access}", GraphicsCaptureSession::IsSupported());
}

fn is_elevated(proc: HANDLE) -> bool {
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(proc, TOKEN_QUERY, &mut tok).is_err() {
            return false;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0;
        let ok = GetTokenInformation(tok, TokenElevation, Some(&mut e as *mut _ as _), size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok();
        let _ = CloseHandle(tok);
        ok && e.TokenIsElevated != 0
    }
}

extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_DESTROY {
            eprintln!("WM_DESTROY {:?}", h);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(h, m, w, l)
    }
}

/// 単色背景（WM_ERASEBKGND で塗られるので PrintWindow でも描ける）の枠なしウィンドウを作る。
fn solid_window(class: PCWSTR, title: PCWSTR, rgb: (u8, u8, u8), r: RECT, topmost: bool) -> Result<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hbrBackground: CreateSolidBrush(COLORREF(rgb.0 as u32 | (rgb.1 as u32) << 8 | (rgb.2 as u32) << 16)),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let ex = if topmost { WS_EX_TOPMOST } else { WINDOW_EX_STYLE(0) };
        let hwnd = CreateWindowExW(ex, class, title, WS_POPUP | WS_VISIBLE, r.left, r.top, r.right - r.left, r.bottom - r.top, None, None, Some(hinst.into()), None)?;
        let _ = UpdateWindow(hwnd);
        Ok(hwnd)
    }
}

fn pump(d: Duration) {
    let end = Instant::now() + d;
    unsafe {
        let mut msg = MSG::default();
        while Instant::now() < end {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn primary_center_rect(w: i32, h: i32) -> RECT {
    unsafe {
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let x = (sw - w) / 2;
        let y = (sh - h) / 2;
        RECT { left: x, top: y, right: x + w, bottom: y + h }
    }
}

/// BGRA 画素列
struct Img {
    w: i32,
    h: i32,
    px: Vec<u8>,
}

impl Img {
    fn get(&self, x: i32, y: i32) -> (u8, u8, u8) {
        let i = ((y * self.w + x) * 4) as usize;
        (self.px[i + 2], self.px[i + 1], self.px[i])
    }
    /// 色ごとの割合（緑・青・マゼンタ・黒・その他）
    fn summary(&self) -> String {
        let (mut g, mut b, mut m, mut k, mut o) = (0, 0, 0, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                match self.get(x, y) {
                    (r, gg, bb) if r < 40 && gg > 200 && bb < 40 => g += 1,
                    (r, gg, bb) if r < 40 && gg < 40 && bb > 200 => b += 1,
                    (r, gg, bb) if r > 200 && gg < 40 && bb > 200 => m += 1,
                    (r, gg, bb) if r < 10 && gg < 10 && bb < 10 => k += 1,
                    _ => o += 1,
                }
            }
        }
        let n = (self.w * self.h).max(1) as f64 / 100.0;
        format!("{}x{} green {:.1}% blue {:.1}% magenta {:.1}% black {:.1}% other {:.1}%", self.w, self.h, g as f64 / n, b as f64 / n, m as f64 / n, k as f64 / n, o as f64 / n)
    }
}

fn dib_bits(dc: HDC, bmp: HBITMAP, w: i32, h: i32) -> Img {
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
        ..Default::default()
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    unsafe { GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bi, DIB_RGB_COLORS) };
    Img { w, h, px }
}

/// 画面 DC から BitBlt（CAPTUREBLT）
fn screen_grab(r: RECT) -> Img {
    unsafe {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let sdc = GetDC(None);
        let mdc = CreateCompatibleDC(Some(sdc));
        let bmp = CreateCompatibleBitmap(sdc, w, h);
        let old = SelectObject(mdc, bmp.into());
        let _ = BitBlt(mdc, 0, 0, w, h, Some(sdc), r.left, r.top, SRCCOPY | CAPTUREBLT);
        SelectObject(mdc, old);
        let img = dib_bits(mdc, bmp, w, h);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mdc);
        ReleaseDC(None, sdc);
        img
    }
}

fn print_window(hwnd: HWND, flags: u32) -> (bool, Option<Img>) {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let sdc = GetDC(None);
        let mdc = CreateCompatibleDC(Some(sdc));
        let bmp = CreateCompatibleBitmap(sdc, w, h);
        let old = SelectObject(mdc, bmp.into());
        let ok = PrintWindow(hwnd, mdc, PRINT_WINDOW_FLAGS(flags)).as_bool();
        SelectObject(mdc, old);
        let img = dib_bits(mdc, bmp, w, h);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mdc);
        ReleaseDC(None, sdc);
        (ok, Some(img))
    }
}

struct D3d {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    winrt: IDirect3DDevice,
}

fn d3d() -> Result<D3d> {
    unsafe {
        let mut dev = None;
        let mut ctx = None;
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, Some(&mut ctx))?;
        let dev = dev.unwrap();
        let dxgi: IDXGIDevice = dev.cast()?;
        let winrt: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
        Ok(D3d { dev, ctx: ctx.unwrap(), winrt })
    }
}

fn tex_to_img(d: &D3d, tex: &ID3D11Texture2D) -> Result<Img> {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut st = None;
        d.dev.CreateTexture2D(&desc, None, Some(&mut st))?;
        let st = st.unwrap();
        d.ctx.CopyResource(&st, tex);
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        d.ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
        let (w, h) = (desc.Width as i32, desc.Height as i32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            let src = std::slice::from_raw_parts((map.pData as *const u8).add(y * map.RowPitch as usize), (w * 4) as usize);
            px[y * (w * 4) as usize..(y + 1) * (w * 4) as usize].copy_from_slice(src);
        }
        d.ctx.Unmap(&st, 0);
        Ok(Img { w, h, px })
    }
}

/// WGC でウィンドウを撮るセッション。frame を待つ間はメッセージを回す。
struct WgcRun {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
}

fn wgc_start(d: &D3d, hwnd: HWND, border_required: Option<bool>) -> Result<WgcRun> {
    unsafe {
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = interop.CreateForWindow(hwnd)?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d.winrt, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, item.Size()?)?;
        let session = pool.CreateCaptureSession(&item)?;
        session.SetIsCursorCaptureEnabled(false)?;
        if let Some(b) = border_required {
            match session.SetIsBorderRequired(b) {
                Ok(()) => println!("  SetIsBorderRequired({b}): ok, IsBorderRequired now = {:?}", session.IsBorderRequired()),
                Err(e) => println!("  SetIsBorderRequired({b}): ERR {e}"),
            }
        }
        session.StartCapture()?;
        Ok(WgcRun { session, pool })
    }
}

fn wgc_frame(d: &D3d, run: &WgcRun) -> Result<Img> {
    let start = Instant::now();
    loop {
        if let Ok(f) = run.pool.TryGetNextFrame() {
            let access: IDirect3DDxgiInterfaceAccess = f.Surface()?.cast()?;
            let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
            let img = tex_to_img(d, &tex)?;
            f.Close()?;
            println!("  first frame after {} ms", start.elapsed().as_millis());
            return Ok(img);
        }
        if start.elapsed() > Duration::from_secs(3) {
            return Err(Error::new(E_FAIL, "no WGC frame within 3s"));
        }
        pump(Duration::from_millis(10));
    }
}

fn wgc_stop(run: WgcRun) {
    let _ = run.session.Close();
    let _ = run.pool.Close();
}

// ---------- border ----------

/// 窓の外周 margin px の帯にある黄色っぽい画素数
fn yellow_ring(r: RECT, margin: i32) -> usize {
    let outer = RECT { left: r.left - margin, top: r.top - margin, right: r.right + margin, bottom: r.bottom + margin };
    let img = screen_grab(outer);
    let mut n = 0;
    for y in 0..img.h {
        for x in 0..img.w {
            let inner = x >= 2 * margin && x < img.w - 2 * margin && y >= 2 * margin && y < img.h - 2 * margin;
            if inner {
                continue;
            }
            let (rr, g, b) = img.get(x, y);
            if rr > 200 && g > 150 && b < 90 {
                n += 1;
            }
        }
    }
    n
}

fn border(request: bool, want_off: bool) -> Result<()> {
    os_info();
    if request {
        let t = Instant::now();
        println!("RequestAccessAsync(Borderless) ...");
        match GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless).and_then(|op| op.get()) {
            Ok(s) => println!("  -> {:?} after {} ms", s, t.elapsed().as_millis()),
            Err(e) => println!("  -> ERR {e} after {} ms", t.elapsed().as_millis()),
        }
    }
    let r = primary_center_rect(600, 400);
    let hwnd = solid_window(w!("wgc_probe_border"), w!("wgc-probe border"), (255, 0, 255), r, true)?;
    pump(Duration::from_millis(500));
    println!("yellow px around window before capture: {}", yellow_ring(r, 8));
    let d = d3d()?;
    let run = wgc_start(&d, hwnd, Some(!want_off))?;
    let img = wgc_frame(&d, &run)?;
    println!("  WGC frame: {}", img.summary());
    println!("capturing for 5 s — 目視: マゼンタのウィンドウの周りに黄色い枠が出ているか");
    for i in 0..5 {
        pump(Duration::from_millis(1000));
        println!("  t={}s yellow px around window: {}", i + 1, yellow_ring(r, 8));
    }
    wgc_stop(run);
    pump(Duration::from_millis(500));
    println!("yellow px after stop: {}", yellow_ring(r, 8));
    unsafe { DestroyWindow(hwnd)? };
    Ok(())
}

// ---------- elevated ----------

fn target() -> Result<()> {
    let r = primary_center_rect(600, 400);
    let hwnd = solid_window(w!("wgc_probe_target"), TARGET_TITLE, (0, 255, 0), r, false)?;
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        SetTimer(Some(hwnd), 1, 60_000, None);
        let mut msg = MSG::default();
        eprintln!("target: hwnd {:?} visible {}", hwnd, IsWindowVisible(hwnd).as_bool());
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0).0;
            if r == 0 || r == -1 { eprintln!("target: GetMessage {r} msg {:#x}", msg.message); break; }
            if msg.message == WM_TIMER && msg.hwnd == hwnd && msg.wParam.0 == 1 {
                let _ = DestroyWindow(hwnd);
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn elevated(control: bool) -> Result<()> {
    os_info();
    let exe: Vec<u16> = std::env::current_exe().unwrap().as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
    unsafe {
        let mut sei = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: if control { w!("open") } else { w!("runas") },
            lpFile: PCWSTR(exe.as_ptr()),
            lpParameters: w!("target"),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        println!("launching target ({}) ...", if control { "not elevated" } else { "runas: UAC の同意をお願いします" });
        ShellExecuteExW(&mut sei)?;
        let start = Instant::now();
        let hwnd = loop {
            if let Ok(h) = FindWindowW(w!("wgc_probe_target"), TARGET_TITLE) {
                break h;
            }
            if start.elapsed() > Duration::from_secs(60) {
                return Err(Error::new(E_FAIL, "target window not found"));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let tp = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        println!("target hwnd found; target elevated: {}", tp.map(|p| is_elevated(p)).unwrap_or(false));
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r)?;
        // 左半分を非昇格の青いウィンドウで隠す
        let cover = RECT { left: r.left - 20, top: r.top - 20, right: (r.left + r.right) / 2, bottom: r.bottom + 20 };
        let occ = solid_window(w!("wgc_probe_occluder"), w!("wgc-probe occluder"), (0, 0, 255), cover, true)?;
        pump(Duration::from_millis(800));
        println!("window rect {}x{} (左半分を青で隠している)", r.right - r.left, r.bottom - r.top);

        println!("[screen BitBlt of target rect] {}", screen_grab(r).summary());
        for (name, f) in [("PrintWindow(0)", 0u32), ("PrintWindow(PW_RENDERFULLCONTENT)", PW_RENDERFULLCONTENT)] {
            let t = Instant::now();
            let (ok, img) = print_window(hwnd, f);
            println!("[{name}] returned {ok} in {} ms: {}", t.elapsed().as_millis(), img.map(|i| i.summary()).unwrap_or_default());
        }
        println!("[WGC CreateForWindow]");
        let d = d3d()?;
        match wgc_start(&d, hwnd, Some(false)) {
            Ok(run) => {
                match wgc_frame(&d, &run) {
                    Ok(img) => println!("  frame: {}", img.summary()),
                    Err(e) => println!("  frame ERR {e}"),
                }
                wgc_stop(run);
            }
            Err(e) => println!("  start ERR {e}"),
        }
        let _ = DestroyWindow(occ);
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        println!("(昇格したターゲットは UIPI で WM_CLOSE が届かない場合、60 秒後に自分で閉じます)");
    }
    Ok(())
}
