//! 自動起動（`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`）の登録を設定に合わせる。
//!
//! - 正本は設定ファイル（`general.autostart`）。アプリは起動したときと適用したときに、Run キーの今の値を読んで
//!   `plan` に渡し、返ってきた `Action` をレジストリに当てる。
//! - 登録する値は、いまの exe のパスを `"` で囲んだもの（`command`）。exe を動かしても、動かした先で 1 回
//!   起動すれば登録し直す。
//! - OFF のときに消すのは、いまの exe を指す登録だけ（引数が付いていても、パスの書き方が違っても）。同じ PC の
//!   別のフォルダに置いたコピーの登録は消さない。
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
/// - 今の値は exe の部分だけを取り出して（`RunValue`）いまの exe と比べる。比べる前に `normalize` で両方の
///   パスを正規化し（Win32 層が 8.3 の短い名前などを実際のパスにする）、Windows のパスに合わせて大文字と
///   小文字を区別せずに比べる。
/// - ON のときは、今の値が `command` の書き方どおり（`"` で囲み、引数が無い）でいまの exe を指すなら何もしない。
///   それ以外は `command` で登録し直す。
/// - OFF のときは、今の値がいまの exe を指すときだけ消す（`"` の囲みや引数の有無は問わない）。別の場所に
///   置いたポータブル版のコピーの登録は、そのコピーのものなので残す。
pub fn plan(
    enabled: bool,
    current: Option<&str>,
    exe: &Path,
    normalize: impl Fn(&str) -> String,
) -> Action {
    let exe_path = exe.display().to_string();
    let points_to_exe = |value: &RunValue| {
        normalize(value.exe).to_lowercase() == normalize(&exe_path).to_lowercase()
    };
    match (enabled, current.map(RunValue::parse)) {
        (false, Some(now)) if points_to_exe(&now) => Action::Remove,
        (false, _) => Action::Nothing,
        (true, None) => Action::Register {
            command: command(exe),
        },
        (true, Some(now)) if now.is_command() && points_to_exe(&now) => Action::Nothing,
        (true, Some(_)) => Action::Reregister {
            command: command(exe),
        },
    }
}

/// `std::fs::canonicalize` が付ける `\\?\`（ネットワークのパスは `\\?\UNC\`）を外し、Run キーに書く
/// ふつうの書き方にそろえる。付いていなければそのまま。
pub fn without_verbatim_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
    }
}

/// Run キーの値を、exe のパスと引数に分けたもの（`command` の逆）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RunValue<'a> {
    exe: &'a str,
    /// exe のパスを `"` で囲んでいたか。
    quoted: bool,
    /// exe の後ろの引数（前後の空白を除く）。
    args: &'a str,
}

impl<'a> RunValue<'a> {
    /// `"` で囲んでいれば、閉じる `"` までを exe とする。囲みの無い古い書き方は、`.exe` の後ろが空白か
    /// 終わりになる最初のところまでを exe とする（無ければ全体）。
    fn parse(value: &'a str) -> RunValue<'a> {
        let value = value.trim();
        if let Some(rest) = value.strip_prefix('"') {
            let (exe, args) = rest.split_once('"').unwrap_or((rest, ""));
            return RunValue {
                exe,
                quoted: true,
                args: args.trim(),
            };
        }
        let end = value
            .to_ascii_lowercase()
            .match_indices(".exe")
            .map(|(i, ext)| i + ext.len())
            .find(|&end| value[end..].starts_with(char::is_whitespace) || end == value.len())
            .unwrap_or(value.len());
        RunValue {
            exe: &value[..end],
            quoted: false,
            args: value[end..].trim(),
        }
    }

    /// `command` が書く形（`"` で囲み、引数が無い）か。
    fn is_command(&self) -> bool {
        self.quoted && self.args.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Tools\shunsoku\shunsoku.exe";

    fn plan_for(enabled: bool, current: Option<&str>) -> Action {
        plan(enabled, current, Path::new(EXE), str::to_owned)
    }

    /// 8.3 の短い名前を長い名前にする（Win32 層の正規化の代わり）。
    fn long_name(path: &str) -> String {
        path.replace(r"C:\TOOLS~1\SHUNSO~1", r"C:\Tools\shunsoku")
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
    fn off_removes_a_registration_of_the_current_exe_with_arguments() {
        assert_eq!(
            plan_for(false, Some(r#""C:\Tools\shunsoku\shunsoku.exe" --args"#)),
            Action::Remove
        );
        assert_eq!(
            plan_for(false, Some(r"C:\Tools\shunsoku\shunsoku.exe --args")),
            Action::Remove
        );
        // exe の名前の続きが違えば、別のファイル
        assert_eq!(
            plan_for(false, Some(r#""C:\Tools\shunsoku\shunsoku.exe.old""#)),
            Action::Nothing
        );
    }

    #[test]
    fn on_with_arguments_registers_again_without_them() {
        assert_eq!(
            plan_for(true, Some(r#""C:\Tools\shunsoku\shunsoku.exe" --args"#)),
            Action::Reregister {
                command: r#""C:\Tools\shunsoku\shunsoku.exe""#.to_owned()
            }
        );
    }

    #[test]
    fn paths_written_differently_are_compared_after_normalizing_both() {
        let short = r#""C:\TOOLS~1\SHUNSO~1\shunsoku.exe""#;
        assert_eq!(
            plan(false, Some(short), Path::new(EXE), long_name),
            Action::Remove
        );
        assert_eq!(
            plan(true, Some(short), Path::new(EXE), long_name),
            Action::Nothing
        );
        // いまの exe の側が短い名前でも同じ
        assert_eq!(
            plan(
                false,
                Some(r#""C:\Tools\shunsoku\shunsoku.exe""#),
                Path::new(r"C:\TOOLS~1\SHUNSO~1\shunsoku.exe"),
                long_name
            ),
            Action::Remove
        );
    }

    #[test]
    fn the_verbatim_prefix_of_a_canonical_path_is_taken_off() {
        assert_eq!(
            without_verbatim_prefix(r"\\?\C:\Tools\shunsoku\shunsoku.exe"),
            r"C:\Tools\shunsoku\shunsoku.exe"
        );
        assert_eq!(
            without_verbatim_prefix(r"\\?\UNC\server\share\shunsoku.exe"),
            r"\\server\share\shunsoku.exe"
        );
        assert_eq!(
            without_verbatim_prefix(r"C:\Tools\shunsoku\shunsoku.exe"),
            r"C:\Tools\shunsoku\shunsoku.exe"
        );
    }

    #[test]
    fn off_without_a_registration_does_nothing() {
        assert_eq!(plan_for(false, None), Action::Nothing);
    }
}
