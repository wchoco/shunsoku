//! 自動起動の登録（`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）。
//!
//! - するべきことは core の `autostart::plan` が決め、ここは Run キーの値を読んで当てるだけ。管理者権限は要らない。
//! - タスク マネージャーで無効にした状態（`...\Explorer\StartupApproved\Run`）には触れない。
//! - 値の名前は `shunsoku`。実機の試験でユーザーの登録を書き換えないよう、環境変数
//!   `SHUNSOKU_AUTOSTART_VALUE` で名前を差し替えられる（`docs/agents/windows-dev.md`）。

use std::path::Path;

use shunsoku_core::APP_NAME;
use shunsoku_core::autostart::{Action, VALUE_NAME_VARIABLE, plan, without_verbatim_prefix};
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

/// Run キーを設定（`enabled`）に合わせる。したことを返す。
pub fn sync(enabled: bool) -> Result<Action, String> {
    let exe = std::env::current_exe().map_err(|e| format!("could not get the exe path: {e}"))?;
    sync_value(&value_name(), &exe, enabled)
}

/// Run キーの値 `name` を、`exe` の設定（`enabled`）に合わせる。したことを返す。
fn sync_value(name: &str, exe: &Path, enabled: bool) -> Result<Action, String> {
    let current = read(name).map_err(|e| format!("could not read the Run key: {e:?}"))?;
    let action = plan(enabled, current.as_deref(), exe, normalize);
    match &action {
        Action::Register { command } | Action::Reregister { command } => write(name, command)
            .map_err(|e| format!("could not register {command} in the Run key: {e:?}"))?,
        Action::Remove => {
            remove(name).map_err(|e| format!("could not remove from the Run key: {e:?}"))?
        }
        Action::Nothing => {}
    }
    Ok(action)
}

/// Run キーの値の exe といまの exe を比べる前に、パスの書き方をそろえる。ファイルがあれば実際のパスにする
/// （8.3 の短い名前、`..`、シンボリック リンクやジャンクションを解く）。無ければ書いたままの文字列で比べる。
fn normalize(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|real| without_verbatim_prefix(&real.display().to_string()))
        .unwrap_or_else(|_| path.to_owned())
}

fn value_name() -> String {
    std::env::var(VALUE_NAME_VARIABLE)
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

/// Run キーに触れるので、実機でだけ走らせる（`docs/agents/windows-dev.md`）。値の名前は試験用のものだけを使い、
/// ユーザーの登録（`shunsoku`）には触れない。
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// 試験用の値の名前。落ちても消す。
    struct TestValue(String);

    impl TestValue {
        fn new() -> TestValue {
            let name = format!("shunsoku-test-{}", std::process::id());
            remove(&name).unwrap();
            TestValue(name)
        }
    }

    impl Drop for TestValue {
        fn drop(&mut self) {
            let _ = remove(&self.0);
        }
    }

    #[test]
    #[ignore = "HKCU の Run キーに書く。実機でだけ走らせる"]
    fn off_from_another_portable_copy_leaves_the_registration_of_the_copy_that_is_on() {
        let value = TestValue::new();
        let on = PathBuf::from(r"C:\shunsoku-test\a\shunsoku.exe");
        let off = PathBuf::from(r"C:\shunsoku-test\b\shunsoku.exe");

        let registered = sync_value(&value.0, &on, true).unwrap();
        assert!(
            matches!(registered, Action::Register { .. }),
            "{registered:?}"
        );
        assert_eq!(
            read(&value.0).unwrap().as_deref(),
            Some(r#""C:\shunsoku-test\a\shunsoku.exe""#)
        );

        assert_eq!(sync_value(&value.0, &off, false).unwrap(), Action::Nothing);
        assert_eq!(
            read(&value.0).unwrap().as_deref(),
            Some(r#""C:\shunsoku-test\a\shunsoku.exe""#)
        );

        assert_eq!(sync_value(&value.0, &on, false).unwrap(), Action::Remove);
        assert_eq!(read(&value.0).unwrap(), None);
    }
}
