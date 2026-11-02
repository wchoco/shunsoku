//! `shunsoku-cli` の引数の解釈（ADR-0003）。`clap` は使わない。

use crate::geometry::Rect;

/// 解釈した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `capture <対象>`: 常駐本体にキャプチャを依頼する。
    Capture(Capture),
    /// `monitors`: モニタの番号・名前・解像度・拡大率を出す。
    Monitors,
    /// `--help`（`-h`）。どこにあっても、ほかの引数より優先する。
    Help,
    /// `--version`（`-V`）。どこにあっても、ほかの引数より優先する。
    Version,
}

/// `--help` で出す使い方。
pub const USAGE: &str = "\
使い方:
  shunsoku-cli capture rect [--monitor <番号>] <x>,<y>,<w>,<h>
      モニタ相対の物理ピクセルの範囲を撮り、保存した絶対パスを出します。
      --monitor を省くとメインモニタです。番号は monitors で調べられます。
  shunsoku-cli capture region <名前>
      定型領域を撮り、その領域の保存先とファイル名テンプレートで保存します（領域の撮影後の動作は見ません）。
  shunsoku-cli capture select
      範囲選択の画面を出し、選び終えたら撮って保存します。Esc で取り消すと終了コード 4 です。
  shunsoku-cli capture active [--delay <秒>]
      アクティブウィンドウを撮って保存します。--delay（0〜60 秒）で、撮る前に待ちます。
  shunsoku-cli monitors
      モニタの番号・名前・解像度・拡大率を出します。
  shunsoku-cli --help | --version

終了コード:
  0 成功 / 1 キャプチャの失敗 / 2 引数の誤り / 3 常駐本体に依頼できない・バージョンが合わない
  4 ユーザーが取り消した
";

/// `capture` の依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub target: Target,
}

/// 何を撮るか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `rect [--monitor <番号>] <x>,<y>,<w>,<h>`。
    Rect {
        /// `monitors` の番号（1 から）。None ならメインモニタ。
        monitor: Option<u32>,
        /// モニタ相対の物理ピクセル。
        rect: Rect,
    },
    /// `region <名前>`: 定型領域（`settings::Region::name`）。名前の領域があるかは常駐本体が調べる。
    Region { name: String },
    /// `select`: 範囲選択の画面を出し、ユーザーが選び終えるまで待つ。
    Select,
    /// `active [--delay <秒>]`: アクティブウィンドウ。`delay` 秒（0〜`MAX_DELAY`）待ってから撮る。
    Active { delay: u32 },
}

/// `--delay` の上限（秒）。
pub const MAX_DELAY: u32 = 60;

/// 引数の誤り（終了コード 2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgError(pub String);

/// 引数（exe の名前を除く）を解釈する。
pub fn parse<S: AsRef<str>>(args: &[S]) -> Result<Command, ArgError> {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    if args.iter().any(|a| matches!(*a, "--help" | "-h")) {
        return Ok(Command::Help);
    }
    if args.iter().any(|a| matches!(*a, "--version" | "-V")) {
        return Ok(Command::Version);
    }
    match args.as_slice() {
        ["capture", "rect", rest @ ..] => {
            parse_rect(rest).map(|target| Command::Capture(Capture { target }))
        }
        ["capture", "region", rest @ ..] => {
            parse_region(rest).map(|target| Command::Capture(Capture { target }))
        }
        ["capture", "select"] => Ok(Command::Capture(Capture {
            target: Target::Select,
        })),
        ["capture", "select", extra, ..] => Err(unexpected(extra)),
        ["capture", "active", rest @ ..] => {
            parse_active(rest).map(|target| Command::Capture(Capture { target }))
        }
        ["capture", target, ..] => Err(error(&format!("不明なキャプチャの対象です: {target}"))),
        ["capture"] => Err(error(
            "キャプチャの対象（rect / region / select / active）を指定してください",
        )),
        ["monitors"] => Ok(Command::Monitors),
        ["monitors", extra, ..] => Err(error(&format!("余分な引数があります: {extra}"))),
        [command, ..] => Err(error(&format!("不明なコマンドです: {command}"))),
        [] => Err(error("コマンドを指定してください")),
    }
}

/// `rect` のあとの引数。`--monitor <番号>` は範囲の前にも後ろにも置ける。
fn parse_rect(args: &[&str]) -> Result<Target, ArgError> {
    let mut monitor = None;
    let mut rect = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if arg == "--monitor" {
            let value = rest
                .next()
                .ok_or_else(|| error("--monitor にモニタの番号を指定してください"))?;
            if monitor.is_some() {
                return Err(error("--monitor が 2 回指定されています"));
            }
            monitor = Some(parse_monitor(value)?);
        } else if arg.starts_with("--") {
            return Err(error(&format!("不明なオプションです: {arg}")));
        } else if rect.is_some() {
            return Err(error(&format!("余分な引数があります: {arg}")));
        } else {
            rect = Some(parse_rect_spec(arg)?);
        }
    }
    let rect = rect.ok_or_else(|| error("範囲を <x>,<y>,<w>,<h> で指定してください"))?;
    Ok(Target::Rect { monitor, rect })
}

/// `region` のあとの引数: 定型領域の名前を 1 つ。
fn parse_region(args: &[&str]) -> Result<Target, ArgError> {
    let mut name = None;
    for &arg in args {
        if arg.starts_with("--") || name.is_some() {
            return Err(unexpected(arg));
        }
        name = Some(arg.to_owned());
    }
    let name = name.ok_or_else(|| error("定型領域の名前を指定してください"))?;
    Ok(Target::Region { name })
}

/// `active` のあとの引数: `--delay <秒>`。
fn parse_active(args: &[&str]) -> Result<Target, ArgError> {
    let mut delay = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if arg != "--delay" {
            return Err(unexpected(arg));
        }
        let value = rest
            .next()
            .ok_or_else(|| error("--delay に待つ秒数を指定してください"))?;
        if delay.is_some() {
            return Err(error("--delay が 2 回指定されています"));
        }
        delay = Some(parse_delay(value)?);
    }
    Ok(Target::Active {
        delay: delay.unwrap_or(0),
    })
}

/// 待つ秒数（0〜`MAX_DELAY` の整数）。
fn parse_delay(value: &str) -> Result<u32, ArgError> {
    match value.parse::<u32>() {
        Ok(n) if n <= MAX_DELAY => Ok(n),
        _ => Err(error(&format!(
            "--delay は 0〜{MAX_DELAY} の整数（秒）で指定してください: {value}"
        ))),
    }
}

/// 対象が受け付けない引数。
fn unexpected(arg: &str) -> ArgError {
    if arg.starts_with("--") {
        error(&format!("不明なオプションです: {arg}"))
    } else {
        error(&format!("余分な引数があります: {arg}"))
    }
}

/// モニタの番号（1 から）。
fn parse_monitor(value: &str) -> Result<u32, ArgError> {
    match value.parse::<u32>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(error(&format!(
            "モニタの番号は 1 以上の整数で指定してください: {value}"
        ))),
    }
}

/// `<x>,<y>,<w>,<h>`。x と y は負でもよい（モニタからはみ出すかは常駐本体が調べる）。幅と高さは 1 以上。
fn parse_rect_spec(spec: &str) -> Result<Rect, ArgError> {
    let invalid = || {
        error(&format!(
            "範囲は <x>,<y>,<w>,<h>（整数、幅と高さは 1 以上）で指定してください: {spec}"
        ))
    };
    let parts: Vec<&str> = spec.split(',').collect();
    let [x, y, w, h] = parts.as_slice() else {
        return Err(invalid());
    };
    let x: i32 = x.parse().map_err(|_| invalid())?;
    let y: i32 = y.parse().map_err(|_| invalid())?;
    let w: i32 = w.parse().map_err(|_| invalid())?;
    let h: i32 = h.parse().map_err(|_| invalid())?;
    if w < 1 || h < 1 {
        return Err(invalid());
    }
    let right = x.checked_add(w).ok_or_else(invalid)?;
    let bottom = y.checked_add(h).ok_or_else(invalid)?;
    Ok(Rect::new(x, y, right, bottom))
}

fn error(message: &str) -> ArgError {
    ArgError(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_rect_without_monitor_targets_the_main_monitor() {
        assert_eq!(
            parse(&["capture", "rect", "10,20,300,200"]),
            Ok(Command::Capture(Capture {
                target: Target::Rect {
                    monitor: None,
                    rect: Rect::new(10, 20, 310, 220),
                },
            }))
        );
    }

    fn rect_on(monitor: Option<u32>, rect: Rect) -> Result<Command, ArgError> {
        Ok(Command::Capture(Capture {
            target: Target::Rect { monitor, rect },
        }))
    }

    #[test]
    fn capture_rect_takes_the_monitor_number_before_or_after_the_rect() {
        let expected = rect_on(Some(2), Rect::new(0, 0, 100, 100));
        assert_eq!(
            parse(&["capture", "rect", "--monitor", "2", "0,0,100,100"]),
            expected
        );
        assert_eq!(
            parse(&["capture", "rect", "0,0,100,100", "--monitor", "2"]),
            expected
        );
    }

    #[test]
    fn capture_rect_passes_negative_origins_through_for_the_monitor_check() {
        // はみ出す範囲は引数の誤りではなくキャプチャの失敗（終了コード 1）にするため、ここでは通す
        assert_eq!(
            parse(&["capture", "rect", "-5,-1,10,10"]),
            rect_on(None, Rect::new(-5, -1, 5, 9))
        );
    }

    #[test]
    fn malformed_capture_rect_is_an_argument_error() {
        let bad: [&[&str]; 11] = [
            &["capture", "rect"],
            &["capture", "rect", "0,0,100"],
            &["capture", "rect", "0,0,100,100,1"],
            &["capture", "rect", "a,0,100,100"],
            &["capture", "rect", "0,0,0,100"],
            &["capture", "rect", "0,0,100,-1"],
            &["capture", "rect", "0, 0,100,100"],
            &["capture", "rect", "0,0,100,100", "1,1,2,2"],
            &["capture", "rect", "--monitor", "0", "0,0,100,100"],
            &["capture", "rect", "0,0,100,100", "--monitor"],
            &[
                "capture",
                "rect",
                "--monitor",
                "1",
                "--monitor",
                "1",
                "0,0,1,1",
            ],
        ];
        for args in bad {
            assert!(parse(args).is_err(), "{args:?} should be rejected");
        }
    }

    #[test]
    fn monitors_lists_the_monitors() {
        assert_eq!(parse(&["monitors"]), Ok(Command::Monitors));
    }

    #[test]
    fn help_and_version_win_over_everything_else() {
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["-h"]), Ok(Command::Help));
        assert_eq!(parse(&["capture", "rect", "--help"]), Ok(Command::Help));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(parse(&["-V"]), Ok(Command::Version));
    }

    #[test]
    fn no_command_or_an_unknown_one_is_an_argument_error() {
        let bad: [&[&str]; 5] = [
            &[],
            &["shoot"],
            &["capture"],
            &["capture", "everything"],
            &["monitors", "extra"],
        ];
        for args in bad {
            assert!(parse(args).is_err(), "{args:?} should be rejected");
        }
    }

    fn capture_of(target: Target) -> Result<Command, ArgError> {
        Ok(Command::Capture(Capture { target }))
    }

    #[test]
    fn capture_region_takes_the_region_name() {
        assert_eq!(
            parse(&["capture", "region", "領域 1"]),
            capture_of(Target::Region {
                name: "領域 1".to_owned(),
            })
        );
    }

    #[test]
    fn capture_region_needs_exactly_one_name() {
        let bad: [&[&str]; 3] = [
            &["capture", "region"],
            &["capture", "region", "a", "b"],
            &["capture", "region", "--monitor", "1"],
        ];
        for args in bad {
            assert!(parse(args).is_err(), "{args:?} should be rejected");
        }
    }

    #[test]
    fn capture_select_takes_no_arguments() {
        assert_eq!(parse(&["capture", "select"]), capture_of(Target::Select));
        assert!(parse(&["capture", "select", "extra"]).is_err());
        assert!(parse(&["capture", "select", "--delay", "1"]).is_err());
    }

    #[test]
    fn capture_active_waits_for_the_given_seconds() {
        assert_eq!(
            parse(&["capture", "active"]),
            capture_of(Target::Active { delay: 0 })
        );
        assert_eq!(
            parse(&["capture", "active", "--delay", "3"]),
            capture_of(Target::Active { delay: 3 })
        );
        assert_eq!(
            parse(&["capture", "active", "--delay", "0"]),
            capture_of(Target::Active { delay: 0 })
        );
        assert_eq!(
            parse(&["capture", "active", "--delay", "60"]),
            capture_of(Target::Active { delay: 60 })
        );
    }

    #[test]
    fn capture_active_rejects_a_delay_outside_0_to_60_seconds() {
        let bad: [&[&str]; 7] = [
            &["capture", "active", "--delay", "61"],
            &["capture", "active", "--delay", "-1"],
            &["capture", "active", "--delay", "1.5"],
            &["capture", "active", "--delay"],
            &["capture", "active", "--delay", "1", "--delay", "2"],
            &["capture", "active", "3"],
            &["capture", "active", "--monitor", "1"],
        ];
        for args in bad {
            assert!(parse(args).is_err(), "{args:?} should be rejected");
        }
    }

    #[test]
    fn a_rect_reaching_past_i32_is_an_argument_error() {
        assert!(parse(&["capture", "rect", "2147483000,0,1000,1"]).is_err());
    }
}
