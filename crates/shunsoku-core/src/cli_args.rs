//! `shunsoku-cli` の引数の解釈（ADR-0003）。`clap` は使わない。

use std::path::Path;

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
  shunsoku-cli capture rect [--monitor <番号>] <x>,<y>,<w>,<h> [出力のオプション]
      モニタ相対の物理ピクセルの範囲を撮り、保存した絶対パスを出します。
      --monitor を省くとメインモニタです。番号は monitors で調べられます。
  shunsoku-cli monitors
      モニタの番号・名前・解像度・拡大率を出します。
  shunsoku-cli --help | --version

capture の出力のオプション（既定は保存だけ）:
  --scrap        スクラップを出して、フォーカスを移します。
  --copy         クリップボードにコピーします。
  --no-save      保存しません（--scrap か --copy と合わせて使います）。標準出力には何も出しません。
  --out <path>   このファイルに保存します（あれば上書きします。拡張子は .png だけ）。
                 省くと、設定の保存先にファイル名テンプレートで名前を付けて保存します。
  --quiet        この回はトレイ通知を出しません。
  保存に失敗したら、スクラップには逃がさず終了コード 1 で終わります。

終了コード:
  0 成功 / 1 キャプチャの失敗 / 2 引数の誤り / 3 常駐本体に依頼できない・バージョンが合わない
  4 ユーザーが取り消した
";

/// `capture` の依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub target: Target,
    pub output: Output,
}

/// 撮ったものの出し先（対象の後ろのオプション）。既定は保存だけ。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub save: Save,
    /// `--scrap`: スクラップを出してフォーカスを移す。
    pub scrap: bool,
    /// `--copy`: クリップボードにコピーする。
    pub copy: bool,
    /// `--quiet`: その回はトレイ通知を出さない。
    pub quiet: bool,
}

/// 保存するか、どこへ保存するか。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Save {
    /// 全体設定の保存先に、ファイル名テンプレートで名前を付けて保存する。
    #[default]
    Default,
    /// `--out <path>`: そのファイルに保存する（上書きする）。
    To(String),
    /// `--no-save`: 保存しない。
    No,
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
}

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
            let mut output = Output::default();
            let target = parse_rect(rest, &mut output)?;
            check_output(&output)?;
            Ok(Command::Capture(Capture { target, output }))
        }
        ["capture", target, ..] => Err(error(&format!("不明なキャプチャの対象です: {target}"))),
        ["capture"] => Err(error("キャプチャの対象（rect）を指定してください")),
        ["monitors"] => Ok(Command::Monitors),
        ["monitors", extra, ..] => Err(error(&format!("余分な引数があります: {extra}"))),
        [command, ..] => Err(error(&format!("不明なコマンドです: {command}"))),
        [] => Err(error("コマンドを指定してください")),
    }
}

/// `rect` のあとの引数。`--monitor <番号>` は範囲の前にも後ろにも置ける。
fn parse_rect(args: &[&str], output: &mut Output) -> Result<Target, ArgError> {
    let mut monitor = None;
    let mut rect = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if parse_output_option(arg, &mut rest, output)? {
            continue;
        }
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

/// 出力のオプションなら `output` に入れて true を返す。どの対象でも、対象の引数の前にも後ろにも置ける。
fn parse_output_option(
    arg: &str,
    rest: &mut std::slice::Iter<&str>,
    output: &mut Output,
) -> Result<bool, ArgError> {
    let flag = match arg {
        "--scrap" => &mut output.scrap,
        "--copy" => &mut output.copy,
        "--quiet" => &mut output.quiet,
        "--no-save" | "--out" => {
            if output.save != Save::Default {
                return Err(error(
                    "--no-save と --out は合わせて 1 回だけ指定できます（保存しないなら保存先は指定できません）",
                ));
            }
            output.save = if arg == "--no-save" {
                Save::No
            } else {
                Save::To(parse_out(rest.next().copied())?)
            };
            return Ok(true);
        }
        _ => return Ok(false),
    };
    if *flag {
        return Err(error(&format!("{arg} が 2 回指定されています")));
    }
    *flag = true;
    Ok(true)
}

/// `--out` の値。拡張子は `.png`（大文字・小文字は問わない）だけ。
fn parse_out(value: Option<&str>) -> Result<String, ArgError> {
    let path = value.ok_or_else(|| error("--out に保存先のファイルを指定してください"))?;
    let is_png = Path::new(path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err(error(&format!(
            "--out の拡張子は .png にしてください: {path}"
        )));
    }
    Ok(path.to_owned())
}

/// 出力のオプションの組み合わせを調べる。
fn check_output(output: &Output) -> Result<(), ArgError> {
    if output.save == Save::No && !output.scrap && !output.copy {
        return Err(error(
            "--no-save を付けるときは --scrap か --copy も付けてください（何も出さないことになります）",
        ));
    }
    Ok(())
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
                output: Output::default(),
            }))
        );
    }

    fn rect_on(monitor: Option<u32>, rect: Rect) -> Result<Command, ArgError> {
        Ok(Command::Capture(Capture {
            target: Target::Rect { monitor, rect },
            output: Output::default(),
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

    fn capture_with(output: Output) -> Result<Command, ArgError> {
        Ok(Command::Capture(Capture {
            target: Target::Rect {
                monitor: None,
                rect: Rect::new(0, 0, 100, 100),
            },
            output,
        }))
    }

    #[test]
    fn scrap_shows_a_scrap_and_still_saves() {
        assert_eq!(
            parse(&["capture", "rect", "0,0,100,100", "--scrap"]),
            capture_with(Output {
                scrap: true,
                ..Output::default()
            })
        );
    }

    #[test]
    fn copy_and_quiet_can_come_before_the_rect() {
        assert_eq!(
            parse(&["capture", "rect", "--copy", "--quiet", "0,0,100,100"]),
            capture_with(Output {
                copy: true,
                quiet: true,
                ..Output::default()
            })
        );
    }

    #[test]
    fn no_save_with_another_output_does_not_save() {
        assert_eq!(
            parse(&[
                "capture",
                "rect",
                "0,0,100,100",
                "--scrap",
                "--copy",
                "--no-save"
            ]),
            capture_with(Output {
                save: Save::No,
                scrap: true,
                copy: true,
                quiet: false,
            })
        );
    }

    #[test]
    fn out_saves_to_the_given_png() {
        assert_eq!(
            parse(&["capture", "rect", "--out", r"shots\a b.png", "0,0,100,100"]),
            capture_with(Output {
                save: Save::To(r"shots\a b.png".to_owned()),
                ..Output::default()
            })
        );
        // 拡張子の大文字・小文字は問わない
        assert_eq!(
            parse(&["capture", "rect", "0,0,100,100", "--out", "A.PNG"]),
            capture_with(Output {
                save: Save::To("A.PNG".to_owned()),
                ..Output::default()
            })
        );
    }

    #[test]
    fn contradictory_or_malformed_outputs_are_argument_errors() {
        let bad: [&[&str]; 9] = [
            // 何も出さないことになる
            &["capture", "rect", "0,0,1,1", "--no-save"],
            &["capture", "rect", "0,0,1,1", "--no-save", "--quiet"],
            // 保存しないのに保存先を指定している
            &[
                "capture",
                "rect",
                "0,0,1,1",
                "--scrap",
                "--no-save",
                "--out",
                "a.png",
            ],
            &[
                "capture",
                "rect",
                "0,0,1,1",
                "--out",
                "a.png",
                "--no-save",
                "--copy",
            ],
            // 拡張子は .png だけ
            &["capture", "rect", "0,0,1,1", "--out", "x.jpg"],
            &["capture", "rect", "0,0,1,1", "--out", "png"],
            &["capture", "rect", "0,0,1,1", "--out"],
            // 同じオプションを 2 回
            &[
                "capture", "rect", "0,0,1,1", "--out", "a.png", "--out", "b.png",
            ],
            &["capture", "rect", "0,0,1,1", "--scrap", "--scrap"],
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

    #[test]
    fn a_rect_reaching_past_i32_is_an_argument_error() {
        assert!(parse(&["capture", "rect", "2147483000,0,1000,1"]).is_err());
    }
}
