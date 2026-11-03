//! 安全の決まり（`docs/agents/manual-check.md`）のうち、Win32 によらない判断。
//!
//! - 起動: 起動してよいのは shunsoku の exe だけ。入力の宛先になるのは常駐の本体（`shunsoku.exe`）だけ。
//! - 入力の宛先: キーやマウスを送る前に、毎回前面のウィンドウを確かめる。テスト対象（自分が起動した本体）
//!   でなければ送らない。ホットキーとトレイのクリックだけは、決まった宛先（デスクトップ・タスクバー、
//!   テスト対象の通知領域のアイコン）にも送る。
//! - 止める操作: 自分が起動した PID でだけ止める。PID は使い回されるので、起動した時刻も控えて突き合わせる。

/// 起動したものの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 常駐の本体（`shunsoku.exe`）。入力の宛先になる。
    App,
    /// CLI（`shunsoku-cli.exe`）。ウィンドウを持たないので、入力の宛先にしない。
    Cli,
}

/// 起動してよい exe か。ファイル名が `shunsoku.exe` か `shunsoku-cli.exe`（大文字と小文字は区別しない）の
/// ものだけを起動する。ほかのアプリを起動して入力の宛先にしないため。
pub fn exe_kind(path: &str) -> Result<Kind, String> {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match name.to_ascii_lowercase().as_str() {
        "shunsoku.exe" => Ok(Kind::App),
        "shunsoku-cli.exe" => Ok(Kind::Cli),
        _ => Err(format!(
            "shunsoku.exe と shunsoku-cli.exe のほかは起動しない: {name}"
        )),
    }
}

/// 自分が起動したプロセス。`<root>\launched.txt` に 1 行ずつ控える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Launched {
    pub pid: u32,
    /// プロセスの作られた時刻（`GetProcessTimes` の FILETIME）。
    pub created: u64,
    pub kind: Kind,
}

impl Launched {
    pub fn to_line(self) -> String {
        let kind = match self.kind {
            Kind::App => "app",
            Kind::Cli => "cli",
        };
        format!("{} {} {kind}\n", self.pid, self.created)
    }
}

/// 控えのファイルの中身を読む。読めない行は飛ばす。種類のない行（前の版の控え）は、止めてはよいが
/// 入力の宛先にはしない（[`Kind::Cli`] として読む）。
pub fn parse_launched(content: &str) -> Vec<Launched> {
    content
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let created = fields.next()?.parse().ok()?;
            let kind = match fields.next() {
                Some("app") => Kind::App,
                Some("cli") | None => Kind::Cli,
                Some(_) => return None,
            };
            Some(Launched { pid, created, kind })
        })
        .collect()
}

/// `pid` が自分が起動して動いているプロセス（`alive`）か。違えば `action`（「止め」など）をしない。
pub fn check_own(alive: &[u32], pid: u32, action: &str) -> Result<(), String> {
    if alive.contains(&pid) {
        Ok(())
    } else {
        Err(format!(
            "PID {pid} は自分が起動して動いているプロセスでないので{action}ない"
        ))
    }
}

/// 入力を確かめるときに見るウィンドウ。
#[derive(Debug, Clone)]
pub struct Seen {
    pub pid: u32,
    pub class: String,
    /// 断るときに出す説明（クラス、タイトルなど）。
    pub describe: String,
}

/// 入力を送ってよいか。`targets` はテスト対象（自分が起動した本体）の PID、`foreground` は前面のウィンドウ、
/// `points` はマウスを送る座標それぞれの下のウィンドウ（キーだけなら空）。
pub fn check_input(targets: &[u32], foreground: &Seen, points: &[Seen]) -> Result<(), String> {
    if !targets.contains(&foreground.pid) {
        return Err(format!(
            "前面のウィンドウがテスト対象でないので送らない: {}",
            foreground.describe
        ));
    }
    if let Some(w) = points.iter().find(|w| !targets.contains(&w.pid)) {
        return Err(format!(
            "カーソルの下のウィンドウがテスト対象でないので送らない: {}",
            w.describe
        ));
    }
    Ok(())
}

/// デスクトップとタスクバーのクラス。
const SHELL_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

/// ホットキーを送ってよいか。登録したプロセスがあればキーは前面に届かないが、登録できていなければ前面に届く。
/// そこで、前面がテスト対象か、デスクトップ・タスクバー（届いても何も起きない）のときだけ送る。
pub fn check_hotkey(targets: &[u32], foreground: &Seen) -> Result<(), String> {
    if targets.is_empty() {
        return Err("テスト対象が動いていないので送らない".into());
    }
    if targets.contains(&foreground.pid) || SHELL_CLASSES.contains(&foreground.class.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "前面のウィンドウがテスト対象でもデスクトップ・タスクバーでもないので送らない: {}",
            foreground.describe
        ))
    }
}

/// 通知領域のアイコンを並べるウィンドウ（トップレベルのクラス）。タスクバーと、Windows 11 の隠れているアイコンの列。
const TRAY_CLASSES: [&str; 4] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
];

/// テスト対象の通知領域のアイコンの矩形（`Shell_NotifyIconGetRect`、スクリーン座標）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// トレイのアイコンをクリックする座標（矩形の中心）。`under` はその座標の下のトップレベルのウィンドウ。
/// 通知領域でなければ（アイコンがほかのウィンドウに隠れている、隠れているアイコンの列が閉じている）クリックしない。
pub fn tray_click_point(icon: &IconRect, under: &Seen) -> Result<(i32, i32), String> {
    if icon.right <= icon.left || icon.bottom <= icon.top {
        return Err(format!(
            "通知領域のアイコンの矩形が空なのでクリックしない: {icon:?}"
        ));
    }
    if !TRAY_CLASSES.contains(&under.class.as_str()) {
        return Err(format!(
            "アイコンの上が通知領域でないのでクリックしない: {}",
            under.describe
        ));
    }
    Ok(((icon.left + icon.right) / 2, (icon.top + icon.bottom) / 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(pid: u32, class: &str) -> Seen {
        Seen {
            pid,
            class: class.into(),
            describe: format!("pid={pid} class={class}"),
        }
    }

    fn app(pid: u32, created: u64) -> Launched {
        Launched {
            pid,
            created,
            kind: Kind::App,
        }
    }

    #[test]
    fn launched_lines_round_trip_and_bad_lines_are_skipped() {
        let a = app(1200, 134_000_000_000_000_000);
        let b = Launched {
            pid: 7,
            created: 1,
            kind: Kind::Cli,
        };
        let content = format!("{}garbage\n\n{}", a.to_line(), b.to_line());
        assert_eq!(parse_launched(&content), vec![a, b]);
    }

    #[test]
    fn lines_written_before_the_kind_was_recorded_are_not_input_targets() {
        // 前の版の控え（`pid created`）。止めてはよいが、入力の宛先にはしない
        assert_eq!(
            parse_launched("1200 5\n"),
            vec![Launched {
                pid: 1200,
                created: 5,
                kind: Kind::Cli,
            }]
        );
    }

    #[test]
    fn only_the_shunsoku_executables_can_be_launched() {
        assert_eq!(exe_kind(r"C:\b\shunsoku.exe"), Ok(Kind::App));
        assert_eq!(exe_kind(r"C:\b\SHUNSOKU.EXE"), Ok(Kind::App));
        assert_eq!(exe_kind("C:/b/shunsoku-cli.exe"), Ok(Kind::Cli));
        let err = exe_kind(r"C:\Windows\notepad.exe").unwrap_err();
        assert!(err.contains("notepad.exe"), "{err}");
        assert!(exe_kind(r"C:\b\shunsoku.exe.bak").is_err());
        assert!(exe_kind(r"C:\b\shunsoku").is_err());
    }

    #[test]
    fn only_launched_processes_can_be_acted_on() {
        assert_eq!(check_own(&[1200, 7], 7, "止め"), Ok(()));
        let err = check_own(&[1200], 900, "止め").unwrap_err();
        assert!(err.contains("900") && err.contains("止め"), "{err}");
    }

    #[test]
    fn keys_to_the_test_instance_are_allowed() {
        assert_eq!(check_input(&[1200], &seen(1200, "#32770"), &[]), Ok(()));
    }

    #[test]
    fn keys_are_refused_when_a_terminal_is_in_front() {
        let terminal = seen(900, "CASCADIA_HOSTING_WINDOW_CLASS");
        let err = check_input(&[1200], &terminal, &[]).unwrap_err();
        assert!(err.contains("CASCADIA_HOSTING_WINDOW_CLASS"), "{err}");
    }

    #[test]
    fn nothing_is_allowed_before_anything_is_launched() {
        assert!(check_input(&[], &seen(0, ""), &[]).is_err());
        assert!(check_hotkey(&[], &seen(0, "")).is_err());
    }

    #[test]
    fn every_point_of_a_mouse_input_must_be_over_the_test_instance() {
        let front = seen(1200, "#32770");
        let button = seen(1200, "Button");
        assert!(check_input(&[1200], &front, &[button.clone(), button.clone()]).is_ok());
        let err = check_input(&[1200], &front, &[button, seen(4, "Shell_TrayWnd")]).unwrap_err();
        assert!(err.contains("Shell_TrayWnd"), "{err}");
    }

    #[test]
    fn hotkeys_may_go_to_the_desktop_or_the_taskbar() {
        for class in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ] {
            assert_eq!(check_hotkey(&[1200], &seen(4, class)), Ok(()), "{class}");
        }
        assert_eq!(
            check_hotkey(&[1200], &seen(1200, "shunsoku.overlay")),
            Ok(())
        );
    }

    #[test]
    fn hotkeys_are_refused_when_another_app_is_in_front() {
        let err = check_hotkey(&[1200], &seen(900, "CASCADIA_HOSTING_WINDOW_CLASS")).unwrap_err();
        assert!(err.contains("CASCADIA_HOSTING_WINDOW_CLASS"), "{err}");
        assert!(check_hotkey(&[1200], &seen(900, "Notepad")).is_err());
    }

    #[test]
    fn a_tray_click_lands_on_the_test_icon_in_the_notification_area() {
        let icon = IconRect {
            left: 1800,
            top: 1040,
            right: 1824,
            bottom: 1064,
        };
        assert_eq!(
            tray_click_point(&icon, &seen(4, "Shell_TrayWnd")),
            Ok((1812, 1052))
        );
        // 隠れているアイコンの列（Windows 11 の「隠れているアイコンを表示」）
        assert!(tray_click_point(&icon, &seen(4, "TopLevelWindowForOverflowXamlIsland")).is_ok());
    }

    #[test]
    fn a_tray_click_is_refused_when_the_icon_is_covered_or_hidden() {
        let icon = IconRect {
            left: 1800,
            top: 1040,
            right: 1824,
            bottom: 1064,
        };
        let err = tray_click_point(&icon, &seen(900, "CASCADIA_HOSTING_WINDOW_CLASS")).unwrap_err();
        assert!(err.contains("CASCADIA_HOSTING_WINDOW_CLASS"), "{err}");
        let empty = IconRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        assert!(tray_click_point(&empty, &seen(4, "Shell_TrayWnd")).is_err());
    }
}
