//! 自動起動（`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）の登録を設定に合わせる。
//!
//! - 正本は設定ファイル（`general.autostart`）。アプリは起動したときと適用したときに、Run キーの今の値を読んで
//!   `plan` に渡し、返ってきた `Action` をレジストリに当てる。
//! - 登録する値は、いまの exe のパスを `"` で囲んだもの（`command`）。exe を動かしても、動かした先で 1 回
//!   起動すれば登録し直す。
//! - タスク マネージャーで無効にした状態（`StartupApproved`）には触れない。

use std::path::Path;

/// Run キーにするべきこと。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 登録が無いので、`command` で登録する。
    Register { command: String },
    /// 違うパスで登録されているので、`command` で登録し直す。
    Reregister { command: String },
    /// 自動起動が OFF なのに登録があるので、消す。
    Remove,
    /// 設定どおりなので、何もしない。
    Nothing,
}

/// Run キーに書く値。exe のパスを `"` で囲む（パスに空白があっても 1 つの実行ファイルとして読まれるように）。
pub fn command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// 設定（`enabled`）と Run キーの今の値（`current`。無ければ None）から、するべきことを決める。
///
/// 今の値がいまの exe と同じかは、Windows のパスに合わせて大文字と小文字を区別せずに比べる。
pub fn plan(enabled: bool, current: Option<&str>, exe: &Path) -> Action {
    let command = command(exe);
    match (enabled, current) {
        (false, None) => Action::Nothing,
        (false, Some(_)) => Action::Remove,
        (true, None) => Action::Register { command },
        (true, Some(now)) if same_command(now, &command) => Action::Nothing,
        (true, Some(_)) => Action::Reregister { command },
    }
}

fn same_command(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Tools\shunsoku\shunsoku.exe";

    fn plan_for(enabled: bool, current: Option<&str>) -> Action {
        plan(enabled, current, Path::new(EXE))
    }

    #[test]
    fn the_command_quotes_the_exe_path() {
        assert_eq!(
            command(Path::new(r"C:\Program Files\shunsoku\shunsoku.exe")),
            r#""C:\Program Files\shunsoku\shunsoku.exe""#
        );
    }

    #[test]
    fn on_without_a_registration_registers_the_current_exe() {
        assert_eq!(
            plan_for(true, None),
            Action::Register {
                command: r#""C:\Tools\shunsoku\shunsoku.exe""#.to_owned()
            }
        );
    }

    #[test]
    fn on_with_the_same_path_does_nothing() {
        assert_eq!(
            plan_for(true, Some(r#""C:\Tools\shunsoku\shunsoku.exe""#)),
            Action::Nothing
        );
        // Windows のパスは大文字と小文字を区別しない
        assert_eq!(
            plan_for(true, Some(r#""c:\tools\SHUNSOKU\shunsoku.exe""#)),
            Action::Nothing
        );
    }

    #[test]
    fn on_with_another_path_registers_again_with_the_current_exe() {
        // ポータブルモードで exe を動かした
        assert_eq!(
            plan_for(true, Some(r#""E:\old\shunsoku.exe""#)),
            Action::Reregister {
                command: r#""C:\Tools\shunsoku\shunsoku.exe""#.to_owned()
            }
        );
        // 囲みの無い古い書き方も、違う値として書き直す
        assert_eq!(
            plan_for(true, Some(EXE)),
            Action::Reregister {
                command: r#""C:\Tools\shunsoku\shunsoku.exe""#.to_owned()
            }
        );
    }

    #[test]
    fn off_removes_any_registration() {
        assert_eq!(
            plan_for(false, Some(r#""C:\Tools\shunsoku\shunsoku.exe""#)),
            Action::Remove
        );
        assert_eq!(
            plan_for(false, Some(r#""E:\old\shunsoku.exe""#)),
            Action::Remove
        );
    }

    #[test]
    fn off_without_a_registration_does_nothing() {
        assert_eq!(plan_for(false, None), Action::Nothing);
    }
}
