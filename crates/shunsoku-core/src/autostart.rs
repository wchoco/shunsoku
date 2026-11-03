//! 自動起動（`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）の登録を設定に合わせる。
//!
//! - 正本は設定ファイル（`general.autostart`）。アプリは起動したときと適用したときに、Run キーの今の値を読んで
//!   `plan` に渡し、返ってきた `Action` をレジストリに当てる。
//! - 登録する値は、いまの exe のパスを `"` で囲んだもの（`command`）。exe を動かしても、動かした先で 1 回
//!   起動すれば登録し直す。
//! - OFF のときに消すのは、いまの exe を指す登録だけ。同じ PC の別のフォルダに置いたコピーの登録は消さない。
//! - タスク マネージャーで無効にした状態（`StartupApproved`）には触れない。

use std::path::Path;

/// Run キーの値の名前を差し替える環境変数（実機の試験と性能を測るツール用）。既定の名前は `APP_NAME`。
pub const VALUE_NAME_VARIABLE: &str = "SHUNSOKU_AUTOSTART_VALUE";

/// Run キーにするべきこと。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 登録が無いので、`command` で登録する。
    Register { command: String },
    /// 違うパスで登録されているので、`command` で登録し直す。
    Reregister { command: String },
    /// 自動起動が OFF なのに、いまの exe を指す登録があるので、消す。
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
///
/// OFF のときは、今の値がいまの exe を指すときだけ消す（`"` の囲みの有無は問わない）。別の場所に置いた
/// ポータブル版のコピーの登録は、そのコピーのものなので残す。
pub fn plan(enabled: bool, current: Option<&str>, exe: &Path) -> Action {
    let command = command(exe);
    match (enabled, current) {
        (false, Some(now)) if points_to(now, exe) => Action::Remove,
        (false, _) => Action::Nothing,
        (true, None) => Action::Register { command },
        (true, Some(now)) if same_command(now, &command) => Action::Nothing,
        (true, Some(_)) => Action::Reregister { command },
    }
}

fn same_command(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.to_lowercase()
}

/// Run キーの値 `value` が `exe` を指すか。`"` で囲んだ書き方も囲みの無い古い書き方も読む。
fn points_to(value: &str, exe: &Path) -> bool {
    let value = value.trim();
    let path = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value);
    path.to_lowercase() == exe.display().to_string().to_lowercase()
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
    fn off_removes_a_registration_of_the_current_exe() {
        assert_eq!(
            plan_for(false, Some(r#""C:\Tools\shunsoku\shunsoku.exe""#)),
            Action::Remove
        );
        // 大文字と小文字の違い、囲みの無い古い書き方、前後の空白も、いまの exe を指す
        assert_eq!(
            plan_for(false, Some(r#""c:\tools\SHUNSOKU\shunsoku.exe""#)),
            Action::Remove
        );
        assert_eq!(plan_for(false, Some(EXE)), Action::Remove);
        assert_eq!(
            plan_for(false, Some(r#"  "C:\Tools\shunsoku\shunsoku.exe" "#)),
            Action::Remove
        );
    }

    #[test]
    fn off_leaves_a_registration_of_another_exe() {
        // 別のフォルダに置いたポータブル版のコピーの登録は消さない
        assert_eq!(
            plan_for(false, Some(r#""E:\old\shunsoku.exe""#)),
            Action::Nothing
        );
        assert_eq!(
            plan_for(false, Some(r"E:\old\shunsoku.exe")),
            Action::Nothing
        );
        // 文字列でない値（空の文字列として読む）も、いまの exe ではない
        assert_eq!(plan_for(false, Some("")), Action::Nothing);
    }

    #[test]
    fn off_without_a_registration_does_nothing() {
        assert_eq!(plan_for(false, None), Action::Nothing);
    }
}
