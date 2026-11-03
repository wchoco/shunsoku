//! 安全の決まり（`docs/agents/manual-check.md`）のうち、Win32 によらない判断。
//!
//! - 入力の宛先: キーやマウスを送る前に、毎回前面のウィンドウを確かめる。テスト対象（自分が起動したプロセス）
//!   でなければ送らない。
//! - 止める操作: 自分が起動した PID でだけ止める。PID は使い回されるので、起動した時刻も控えて突き合わせる。

/// 自分が起動したプロセス。`<root>\launched.txt` に 1 行ずつ控える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launched {
    pub pid: u32,
    /// プロセスの作られた時刻（`GetProcessTimes` の FILETIME）。
    pub created: u64,
}

impl Launched {
    pub fn to_line(self) -> String {
        format!("{} {}\n", self.pid, self.created)
    }
}

/// 控えのファイルの中身を読む。読めない行は飛ばす。
pub fn parse_launched(content: &str) -> Vec<Launched> {
    content
        .lines()
        .filter_map(|line| {
            let (pid, created) = line.trim().split_once(' ')?;
            Some(Launched {
                pid: pid.parse().ok()?,
                created: created.parse().ok()?,
            })
        })
        .collect()
}

/// 入力を確かめるときに見るウィンドウ。
#[derive(Debug, Clone)]
pub struct Seen {
    pub pid: u32,
    /// 断るときに出す説明（クラス、タイトルなど）。
    pub describe: String,
}

/// 入力を送ってよいか。`targets` はテスト対象の PID、`foreground` は前面のウィンドウ、
/// `under_cursor` はマウスを送るときのカーソルの下のウィンドウ。
pub fn check_input(
    targets: &[u32],
    foreground: &Seen,
    under_cursor: Option<&Seen>,
) -> Result<(), String> {
    if !targets.contains(&foreground.pid) {
        return Err(format!(
            "前面のウィンドウがテスト対象でないので送らない: {}",
            foreground.describe
        ));
    }
    if let Some(w) = under_cursor
        && !targets.contains(&w.pid)
    {
        return Err(format!(
            "カーソルの下のウィンドウがテスト対象でないので送らない: {}",
            w.describe
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(pid: u32, what: &str) -> Seen {
        Seen {
            pid,
            describe: what.into(),
        }
    }

    #[test]
    fn launched_lines_round_trip_and_bad_lines_are_skipped() {
        let a = Launched {
            pid: 1200,
            created: 134_000_000_000_000_000,
        };
        let b = Launched { pid: 7, created: 1 };
        let content = format!("{}garbage\n\n{}", a.to_line(), b.to_line());
        assert_eq!(parse_launched(&content), vec![a, b]);
    }

    #[test]
    fn keys_to_the_test_instance_are_allowed() {
        assert_eq!(
            check_input(&[1200], &seen(1200, "class=#32770"), None),
            Ok(())
        );
    }

    #[test]
    fn keys_are_refused_when_a_terminal_is_in_front() {
        let terminal = seen(900, "class=CASCADIA_HOSTING_WINDOW_CLASS");
        let err = check_input(&[1200], &terminal, None).unwrap_err();
        assert!(err.contains("CASCADIA_HOSTING_WINDOW_CLASS"), "{err}");
    }

    #[test]
    fn nothing_is_allowed_before_anything_is_launched() {
        assert!(check_input(&[], &seen(0, "no window"), None).is_err());
    }

    #[test]
    fn clicks_need_the_window_under_the_cursor_to_be_the_test_instance_too() {
        let front = seen(1200, "class=#32770");
        assert!(check_input(&[1200], &front, Some(&seen(1200, "Button"))).is_ok());
        let err = check_input(&[1200], &front, Some(&seen(4, "class=Shell_TrayWnd"))).unwrap_err();
        assert!(err.contains("Shell_TrayWnd"), "{err}");
    }
}
