//! 自動起動の登録（`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）。
//!
//! - するべきことは core の `autostart::plan` が決め、ここは Run キーの値を読んで当てるだけ。管理者権限は要らない。
//! - タスク マネージャーで無効にした状態（`...\Explorer\StartupApproved\Run`）には触れない。
//! - 値の名前は `shunsoku`。実機の試験でユーザーの登録を書き換えないよう、環境変数
//!   `SHUNSOKU_AUTOSTART_VALUE` で名前を差し替えられる（`docs/agents/windows-dev.md`）。

use shunsoku_core::APP_NAME;
use shunsoku_core::autostart::{Action, plan};
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, ERROR_UNSUPPORTED_TYPE, WIN32_ERROR,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::PCWSTR;

use crate::wide::to_wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 値の名前を差し替える環境変数（実機の試験用）。
const VALUE_NAME_ENV: &str = "SHUNSOKU_AUTOSTART_VALUE";

/// Run キーを設定（`enabled`）に合わせる。したことを返す。
pub fn sync(enabled: bool) -> Result<Action, String> {
    let exe = std::env::current_exe().map_err(|e| format!("could not get the exe path: {e}"))?;
    let name = value_name();
    let current = read(&name).map_err(|e| format!("could not read the Run key: {e:?}"))?;
    let action = plan(enabled, current.as_deref(), &exe);
    match &action {
        Action::Register { command } | Action::Reregister { command } => write(&name, command)
            .map_err(|e| format!("could not register {command} in the Run key: {e:?}"))?,
        Action::Remove => {
            remove(&name).map_err(|e| format!("could not remove from the Run key: {e:?}"))?
        }
        Action::Nothing => {}
    }
    Ok(action)
}

fn value_name() -> String {
    std::env::var(VALUE_NAME_ENV)
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| APP_NAME.to_owned())
}

/// Run キーの値。無ければ None。文字列でない値は空の文字列として読む（違う値として登録し直すか消す）。
fn read(name: &str) -> Result<Option<String>, WIN32_ERROR> {
    let key = to_wide(RUN_KEY);
    let name = to_wide(name);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut size = 0u32;
    // SAFETY: 文字列は呼び出しの間生きている。大きさだけを問う
    let e = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            None,
            Some(&mut size),
        )
    };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if e != ERROR_SUCCESS {
        return if e == ERROR_UNSUPPORTED_TYPE {
            Ok(Some(String::new()))
        } else {
            Err(e)
        };
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: buf は size バイト。文字列は呼び出しの間生きている
    let e = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if e != ERROR_SUCCESS {
        return Err(e);
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(Some(String::from_utf16_lossy(&buf[..len])))
}

fn write(name: &str, command: &str) -> Result<(), WIN32_ERROR> {
    let key = to_wide(RUN_KEY);
    let name = to_wide(name);
    let data = to_wide(command);
    // SAFETY: data は NUL まで含めて (len * 2) バイト。文字列は呼び出しの間生きている
    let e = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    if e == ERROR_SUCCESS { Ok(()) } else { Err(e) }
}

fn remove(name: &str) -> Result<(), WIN32_ERROR> {
    let key = to_wide(RUN_KEY);
    let name = to_wide(name);
    // SAFETY: 文字列は呼び出しの間生きている
    let e = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
        )
    };
    // 読んでから消すまでに消えていてもよい
    if e == ERROR_SUCCESS || e == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(e)
    }
}
