//! 保存画像をファイルに書き出す。
//!
//! - 保存先とファイル名（`destination`）は UI スレッドで決める: フォルダがなければ作り、既存のファイル名の
//!   一覧と書き込み中の保存先から core の `template` で重ならない名前を付ける。
//! - PNG の書き出しと書き込み（`write_png`）は裏のスレッドで行う。書き込みは新しいファイルとしてだけ作り
//!   （`create_new`）、名前を決めたあとに同じ名前のファイルができていても上書きしない。

use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use shunsoku_core::APP_NAME;
use shunsoku_core::image::Image;
use shunsoku_core::template::{self, LocalTime};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

/// 保存画像の拡張子。形式は PNG だけ。
const EXTENSION: &str = "png";

/// 全体の保存先の既定値: ピクチャ フォルダ（Known Folder）の下の `shunsoku`。
pub fn default_dir() -> windows::core::Result<PathBuf> {
    // SAFETY: 返った文字列は CoTaskMemFree で返す
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None)?;
        let text = path.to_string();
        CoTaskMemFree(Some(path.0 as *const _));
        Ok(PathBuf::from(text?).join(APP_NAME))
    }
}

/// いまのローカル時刻。
pub fn now() -> LocalTime {
    // SAFETY: 引数なし
    let t = unsafe { GetLocalTime() };
    LocalTime {
        year: t.wYear,
        month: t.wMonth as u8,
        day: t.wDay as u8,
        hour: t.wHour as u8,
        minute: t.wMinute as u8,
        second: t.wSecond as u8,
        millisecond: t.wMilliseconds,
    }
}

/// `dir` に保存するファイルのパスを決める。フォルダがなければ作る。
///
/// `pending` は名前を決めたがまだ書き終えていない保存先。裏のスレッドが書き終えるまではフォルダに
/// ないので、続けて保存したときに同じ名前にならないよう、これも使われている名前として数える。
pub fn destination(
    dir: &Path,
    file_name_template: &str,
    now: LocalTime,
    pending: &[PathBuf],
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let in_flight = pending
        .iter()
        .filter(|path| path.parent() == Some(dir))
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned());
    let existing: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .chain(in_flight)
        .collect();
    let context = template::Context { name: "", now };
    let name = template::file_name(file_name_template, &context, &existing, EXTENSION);
    Ok(dir.join(name))
}

/// 画像を PNG にして `path` に新しいファイルとして書く（裏のスレッドで呼ぶ）。失敗したら作りかけのファイルを消す。
pub fn write_png(path: &Path, image: &Image) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut out = BufWriter::new(file);
    let written = image
        .write_png(&mut out)
        .map_err(std::io::Error::other)
        .and_then(|()| out.flush());
    if written.is_err() {
        drop(out);
        let _ = std::fs::remove_file(path);
    }
    written
}

/// フォルダをエクスプローラーで開く。UI スレッドで COM を初期化してから呼ぶ。
pub fn open_folder(dir: &Path) -> bool {
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: path は呼び出しの間生きている
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    result.0 as isize > 32
}
