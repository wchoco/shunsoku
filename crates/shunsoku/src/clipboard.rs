//! クリップボードに画像を置く。
//!
//! - 形式は `CF_DIBV5`（32bit、`BI_BITFIELDS`、アルファあり、下の行から並べる）。上の行から並ぶ DIB を
//!   うまく扱えないアプリがあるため、高さは正にする。`CF_DIB` と `CF_BITMAP` は Windows が作るので、
//!   ペイントなどにも貼れる。
//! - アルファは premultiply しない。M1 の画像のアルファは 0（モニタの隙間）か 255 だけなので、どちらで
//!   読まれても同じ色になる。
//! - ほかのプロセスがクリップボードを開いていると開けないので、少し待って何度か試す。

use std::time::Duration;

use shunsoku_core::image::Image;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_DIBV5;
use windows::core::Result;

/// クリップボードを開く試行の回数と間隔。
const OPEN_TRIES: u32 = 5;
const OPEN_INTERVAL: Duration = Duration::from_millis(20);

/// `LCS_sRGB`（'sRGB'）。
const LCS_SRGB: u32 = 0x7352_4742;

/// 画像をクリップボードに置く。`owner` はクリップボードの持ち主になるウィンドウ。
pub fn copy(owner: HWND, image: &Image) -> Result<()> {
    let dib = dibv5(image);
    open(owner)?;
    let result = put(&dib);
    // SAFETY: 開いたクリップボードを閉じる
    let closed = unsafe { CloseClipboard() };
    result.and(closed)
}

fn open(owner: HWND) -> Result<()> {
    let mut tries = 0;
    loop {
        // SAFETY: owner は UI スレッドのウィンドウ
        match unsafe { OpenClipboard(Some(owner)) } {
            Ok(()) => return Ok(()),
            Err(e) if tries + 1 >= OPEN_TRIES => return Err(e),
            Err(_) => {
                tries += 1;
                std::thread::sleep(OPEN_INTERVAL);
            }
        }
    }
}

/// 開いたクリップボードを空にして `CF_DIBV5` を置く。
fn put(dib: &[u8]) -> Result<()> {
    // SAFETY: クリップボードは開いている。メモリは置けたらシステムのものになり、置けなければここで返す
    unsafe {
        EmptyClipboard()?;
        let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, dib.len())?;
        let target = GlobalLock(memory).cast::<u8>();
        if target.is_null() {
            let error = windows::core::Error::from_thread();
            let _ = GlobalFree(Some(memory));
            return Err(error);
        }
        std::ptr::copy_nonoverlapping(dib.as_ptr(), target, dib.len());
        let _ = GlobalUnlock(memory);
        if let Err(e) = SetClipboardData(u32::from(CF_DIBV5.0), Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(e);
        }
    }
    Ok(())
}

/// `BITMAPV5HEADER` と、下の行から並べた BGRA の画素。
fn dibv5(image: &Image) -> Vec<u8> {
    let width = image.width();
    let height = image.height();
    let header = BITMAPV5HEADER {
        bV5Size: size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: width as i32,
        // 正の高さで下の行から並べる
        bV5Height: height as i32,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: width * height * 4,
        bV5RedMask: 0x00FF_0000,
        bV5GreenMask: 0x0000_FF00,
        bV5BlueMask: 0x0000_00FF,
        bV5AlphaMask: 0xFF00_0000,
        bV5CSType: LCS_SRGB,
        ..Default::default()
    };
    let mut out = Vec::with_capacity(size_of::<BITMAPV5HEADER>() + image.pixels().len());
    // SAFETY: BITMAPV5HEADER は詰め物のない C の構造体で、バイト列として読んでよい
    out.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            (&header as *const BITMAPV5HEADER).cast::<u8>(),
            size_of::<BITMAPV5HEADER>(),
        )
    });
    let row = width as usize * 4;
    if row > 0 {
        for line in image.pixels().chunks_exact(row).rev() {
            out.extend_from_slice(line);
        }
    }
    out
}
