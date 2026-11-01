//! 保存画像をファイルに書き出す。
//!
//! - 保存先とファイル名（`destination`）は UI スレッドで決める: 既存のファイル名の一覧と書き込み中の保存先から
//!   core の `template` で重ならない名前を付ける。フォルダはここでは作らず、書き込むときに作る（「名前を付けて保存」を
//!   キャンセルしたときに空のフォルダを残さない）。
//! - PNG の書き出しと書き込み（`write_png`）は裏のスレッドで行う。書き込みは新しいファイルとしてだけ作り
//!   （`create_new`）、名前を決めたあとに同じ名前のファイルができていても上書きしない。ただし「名前を付けて保存」
//!   （`ask_path`）で選ばれた場所は、ダイアログで上書きを確かめてあるので上書きする（`Overwrite::Allow`）。

use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use shunsoku_core::APP_NAME;
use shunsoku_core::image::Image;
use shunsoku_core::template::{self, LocalTime};
use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_STRICTFILETYPES,
    FileSaveDialog, IFileSaveDialog, IShellItem, KF_FLAG_DEFAULT, SHCreateItemFromParsingName,
    SHGetKnownFolderPath, SIGDN_FILESYSPATH, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

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

/// `dir` に保存するファイルのパスを決める。フォルダがまだなければ、中は空とみなす（作るのは `write_png`）。
///
/// `pending` は名前を決めたがまだ書き終えていない保存先。裏のスレッドが書き終えるまではフォルダに
/// ないので、続けて保存したときに同じ名前にならないよう、これも使われている名前として数える。
pub fn destination(
    dir: &Path,
    file_name_template: &str,
    now: LocalTime,
    pending: &[PathBuf],
) -> std::io::Result<PathBuf> {
    let in_flight = pending
        .iter()
        .filter(|path| path.parent() == Some(dir))
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned());
    let on_disk: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let existing: Vec<String> = on_disk.into_iter().chain(in_flight).collect();
    let context = template::Context { name: "", now };
    let name = template::file_name(file_name_template, &context, &existing, EXTENSION);
    Ok(dir.join(name))
}

/// 「名前を付けて保存」のダイアログを出し、選ばれたパスを返す。キャンセルされたら None。
///
/// 初期フォルダは `path` のフォルダ（まだなければ、あるうちで最も近い親）、ファイル名は `path` のファイル名にする
/// （どちらも既定の保存と同じものを渡す）。
/// 形式は PNG だけ。既にあるファイルを選んだら、ダイアログが上書きを確かめる。モーダルなので `App` を借りずに
/// 呼ぶ。UI スレッドで COM を初期化してから呼ぶ。
pub fn ask_path(
    owner: HWND,
    path: &Path,
    filter_name: &str,
) -> windows::core::Result<Option<PathBuf>> {
    let folder = path
        .ancestors()
        .skip(1)
        .find(|dir| dir.is_dir())
        .map(HSTRING::from);
    let file_name = path.file_name().map(HSTRING::from).unwrap_or_default();
    let filter_name = HSTRING::from(filter_name);
    let filters = [COMDLG_FILTERSPEC {
        pszName: PCWSTR(filter_name.as_ptr()),
        pszSpec: w!("*.png"),
    }];
    // SAFETY: 渡す文字列はどれも呼び出しの間生きている。返った文字列は CoTaskMemFree で返す
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetFileTypes(&filters)?;
        dialog.SetDefaultExtension(w!("png"))?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(
            options | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT | FOS_STRICTFILETYPES,
        )?;
        if let Some(folder) = &folder {
            let item: IShellItem = SHCreateItemFromParsingName(folder, None)?;
            dialog.SetFolder(&item)?;
        }
        dialog.SetFileName(&file_name)?;
        match dialog.Show(Some(owner)) {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => return Ok(None),
            Err(e) => return Err(e),
        }
        let chosen = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let text = chosen.to_string();
        CoTaskMemFree(Some(chosen.0 as *const _));
        Ok(Some(PathBuf::from(text?)))
    }
}

/// 書き込む先に同じ名前のファイルがあったときどうするか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overwrite {
    /// 新しいファイルとしてだけ作る（既定の保存）。
    Deny,
    /// 上書きする（「名前を付けて保存」で上書きを確かめたとき）。
    Allow,
}

/// 画像を PNG にして `path` に書く（裏のスレッドで呼ぶ）。フォルダがなければ作る。失敗したら作りかけのファイルを
/// 消す。
pub fn write_png(path: &Path, image: &Image, overwrite: Overwrite) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = OpenOptions::new();
    options.write(true);
    match overwrite {
        Overwrite::Deny => options.create_new(true),
        Overwrite::Allow => options.create(true).truncate(true),
    };
    let file = options.open(path)?;
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
