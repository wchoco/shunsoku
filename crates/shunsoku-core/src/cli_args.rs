//! `shunsoku-cli` の引数の解釈（ADR-0003）。`clap` は使わない。

use crate::geometry::Rect;

/// 解釈した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `capture <対象>`: 常駐本体にキャプチャを依頼する。
    Capture(Capture),
    /// `monitors`: モニタの番号・名前・解像度・拡大率を出す。
    Monitors,
    /// `open <path>`: 常駐本体にスクラップファイルを開かせる。
    Open(Open),
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
  shunsoku-cli capture region <名前> [出力のオプション]
      定型領域を撮ります。--out を省いた保存は、その領域の保存先とファイル名テンプレートです（領域の撮影後の動作は見ません）。
  shunsoku-cli capture select [出力のオプション]
      範囲選択の画面を出し、選び終えたら撮ります。Esc で取り消すと終了コード 4 です。
      10 分たっても選び終えなければ、取りやめて終了コード 1 です。
  shunsoku-cli capture active [--delay <秒>] [出力のオプション]
      アクティブウィンドウを撮ります。--delay（0〜60 秒）で、撮る前に待ちます。
  shunsoku-cli monitors
      モニタの番号・名前・解像度・拡大率を出します。
  shunsoku-cli open <path>
      スクラップファイル（拡張子は .shunsoku だけ）を、新しいスクラップとして開きます。
      カーソルのあるモニタの中央に、倍率 100%、見た目は全体の既定値で浮かびます。
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
  open の失敗: 5 ファイルを読めない / 6 スクラップファイルとして正しくない（壊れている・スクラップファイルでない）
  7 新しい版の shunsoku で作られたファイル / 8 読めたがスクラップを出せない
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
    /// `region <名前>`: 定型領域（`settings::Region::name`）。名前の領域があるかは常駐本体が調べる。
    Region { name: String },
    /// `select`: 範囲選択の画面を出し、ユーザーが選び終えるまで待つ。
    Select,
    /// `active [--delay <秒>]`: アクティブウィンドウ。`delay` 秒（0〜`MAX_DELAY`）待ってから撮る。
    Active { delay: u32 },
}

/// `open` の依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    /// 開くスクラップファイル。CLI は引数のまま持ち、常駐本体に送る前に絶対パスにする。
    pub path: String,
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
        [
            "capture",
            kind @ ("rect" | "region" | "select" | "active"),
            rest @ ..,
        ] => {
            let mut output = Output::default();
            let target = match *kind {
                "rect" => parse_rect(rest, &mut output)?,
                "region" => parse_region(rest, &mut output)?,
                "select" => parse_select(rest, &mut output)?,
                _ => parse_active(rest, &mut output)?,
            };
            check_output(&output)?;
            Ok(Command::Capture(Capture { target, output }))
        }
        ["capture", target, ..] => Err(error(&format!("不明なキャプチャの対象です: {target}"))),
        ["capture"] => Err(error(
            "キャプチャの対象（rect / region / select / active）を指定してください",
        )),
        ["monitors"] => Ok(Command::Monitors),
        ["monitors", extra, ..] => Err(error(&format!("余分な引数があります: {extra}"))),
        ["open", path] if !path.starts_with("--") => {
            check_scrap_file(path)?;
            Ok(Command::Open(Open {
                path: (*path).to_owned(),
            }))
        }
        ["open", arg] => Err(unexpected(arg)),
        ["open"] => Err(error("開くスクラップファイルを指定してください")),
        ["open", _, extra, ..] => Err(error(&format!("余分な引数があります: {extra}"))),
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

/// `region` のあとの引数: 定型領域の名前を 1 つ。
fn parse_region(args: &[&str], output: &mut Output) -> Result<Target, ArgError> {
    let mut name = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if parse_output_option(arg, &mut rest, output)? {
            continue;
        }
        if arg.starts_with("--") || name.is_some() {
            return Err(unexpected(arg));
        }
        name = Some(arg.to_owned());
    }
    let name = name.ok_or_else(|| error("定型領域の名前を指定してください"))?;
    Ok(Target::Region { name })
}

/// `select` のあとの引数: 出力のオプションだけ。
fn parse_select(args: &[&str], output: &mut Output) -> Result<Target, ArgError> {
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if !parse_output_option(arg, &mut rest, output)? {
            return Err(unexpected(arg));
        }
    }
    Ok(Target::Select)
}

/// `active` のあとの引数: `--delay <秒>`。
fn parse_active(args: &[&str], output: &mut Output) -> Result<Target, ArgError> {
    let mut delay = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if parse_output_option(arg, &mut rest, output)? {
            continue;
        }
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
    let n = value.parse::<u32>().map_err(|_| delay_error(value))?;
    check_delay(n)?;
    Ok(n)
}

/// 待つ秒数が 0〜`MAX_DELAY` か。
fn check_delay(n: u32) -> Result<(), ArgError> {
    if n > MAX_DELAY {
        return Err(delay_error(&n.to_string()));
    }
    Ok(())
}

fn delay_error(value: &str) -> ArgError {
    error(&format!(
        "--delay は 0〜{MAX_DELAY} の整数（秒）で指定してください: {value}"
    ))
}

/// 対象が受け付けない引数。
fn unexpected(arg: &str) -> ArgError {
    if arg.starts_with("--") {
        error(&format!("不明なオプションです: {arg}"))
    } else {
        error(&format!("余分な引数があります: {arg}"))
    }
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
    check_png(path)?;
    Ok(path.to_owned())
}

/// `--out` の拡張子が `.png`（大文字・小文字は問わない）か。
fn check_png(path: &str) -> Result<(), ArgError> {
    if !has_extension(path, "png") {
        return Err(error(&format!(
            "--out の拡張子は .png にしてください: {path}"
        )));
    }
    Ok(())
}

/// `open` のパスの拡張子が `.shunsoku`（大文字・小文字は問わない）か。
fn check_scrap_file(path: &str) -> Result<(), ArgError> {
    if !has_extension(path, crate::scrap_file::EXTENSION) {
        return Err(error(&format!(
            "開けるのはスクラップファイル（拡張子 .{}）だけです: {path}",
            crate::scrap_file::EXTENSION
        )));
    }
    Ok(())
}

/// ファイル名の拡張子が `extension` か（大文字・小文字は問わない）。区切りは `\` と `/` のどちらでも、
/// ホストによらず同じに判定する。拡張子だけの名前（`.png`）は拡張子を持たないとみなす。
fn has_extension(path: &str, extension: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case(extension))
}

/// 常駐本体に届いた `open` の依頼を確かめ直す（パイプの相手を信じきらない）。常駐本体は管理者として
/// 動くことがあるので、読みに行く先を絞る: CLI が絶対パスにしてから送るので絶対パスであること、
/// デバイスの名前空間（`\\.\`・`\\?\`・`\??\`）と、UNC の名前付きパイプ・メールスロット
/// （`\\<サーバー>\pipe\`・`\\<サーバー>\mailslot\`）でないこと、拡張子が `.shunsoku` であること。
pub fn check_open(open: &Open) -> Result<(), ArgError> {
    let path = open.path.as_str();
    if !is_absolute_file_path(path) || is_ipc_share(path) {
        return Err(error(&format!(
            "開くファイルは絶対パス（ドライブ名か \\\\<サーバー>\\<共有> から）にしてください: {path}"
        )));
    }
    check_scrap_file(path)
}

/// UNC の `\\<サーバー>\pipe\...`・`\\<サーバー>\mailslot\...`（ファイルではなく、プロセス間の通信の口）か。
fn is_ipc_share(path: &str) -> bool {
    let separator = |c: char| c == '\\' || c == '/';
    let mut parts = path.split(separator);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(""), Some(""), Some(_server), Some(share)) => {
            share.eq_ignore_ascii_case("pipe") || share.eq_ignore_ascii_case("mailslot")
        }
        _ => false,
    }
}

/// 常駐本体に届いた `capture` の依頼を、CLI の引数と同じ規則で確かめ直す（パイプの相手を信じきらない）。
/// `--out` は、CLI が絶対パスにしてから送るので、絶対パスであることも求める。
pub fn check_capture(capture: &Capture) -> Result<(), ArgError> {
    match &capture.target {
        Target::Rect { monitor, rect } => {
            if let Some(n) = monitor {
                check_monitor(*n)?;
            }
            if rect.width() < 1 || rect.height() < 1 {
                return Err(error(&format!(
                    "範囲の幅と高さは 1 以上にしてください: {rect:?}"
                )));
            }
        }
        Target::Active { delay } => check_delay(*delay)?,
        Target::Region { .. } | Target::Select => {}
    }
    check_output(&capture.output)?;
    if let Save::To(path) = &capture.output.save {
        if !is_absolute_file_path(path) {
            return Err(error(&format!(
                "--out は絶対パス（ドライブ名か \\\\<サーバー>\\<共有> から）にしてください: {path}"
            )));
        }
        check_png(path)?;
    }
    Ok(())
}

/// Windows のファイルの絶対パスか（`C:\...` か `\\<サーバー>\<共有>\...`）。ホストによらず同じに判定する。
/// デバイスの名前空間（`\\.\`・`\\?\`・`\??\`）は、ふつうのファイルではないので断る。
fn is_absolute_file_path(path: &str) -> bool {
    let separator = |c: char| c == '\\' || c == '/';
    let mut chars = path.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(drive), Some(':'), Some(c)) if drive.is_ascii_alphabetic() && separator(c) => true,
        (Some(a), Some(b), Some(_)) if separator(a) && separator(b) => {
            let mut parts = path[2..].split(separator);
            let server = parts.next().unwrap_or("");
            let share = parts.next().unwrap_or("");
            !server.is_empty() && !matches!(server, "." | "?") && !share.is_empty()
        }
        _ => false,
    }
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
    let n = value.parse::<u32>().map_err(|_| monitor_error(value))?;
    check_monitor(n)?;
    Ok(n)
}

/// モニタの番号が 1 以上か。
fn check_monitor(n: u32) -> Result<(), ArgError> {
    if n < 1 {
        return Err(monitor_error(&n.to_string()));
    }
    Ok(())
}

fn monitor_error(value: &str) -> ArgError {
    error(&format!(
        "モニタの番号は 1 以上の整数で指定してください: {value}"
    ))
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

    fn capture_of(target: Target) -> Result<Command, ArgError> {
        Ok(Command::Capture(Capture {
            target,
            output: Output::default(),
        }))
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
    fn region_select_and_active_take_output_options_too() {
        let scrap_only = Output {
            save: Save::No,
            scrap: true,
            ..Output::default()
        };
        let cases: [(&[&str], Target); 3] = [
            (
                &["capture", "region", "--no-save", "a", "--scrap"],
                Target::Region {
                    name: "a".to_owned(),
                },
            ),
            (
                &["capture", "select", "--scrap", "--no-save"],
                Target::Select,
            ),
            (
                &["capture", "active", "--no-save", "--delay", "2", "--scrap"],
                Target::Active { delay: 2 },
            ),
        ];
        for (args, target) in cases {
            assert_eq!(
                parse(args),
                Ok(Command::Capture(Capture {
                    target,
                    output: scrap_only.clone(),
                })),
                "{args:?}"
            );
        }
        // 組み合わせの検査もどの対象にも効く
        assert!(parse(&["capture", "select", "--no-save"]).is_err());
        assert!(parse(&["capture", "active", "--no-save"]).is_err());
        assert!(parse(&["capture", "region", "a", "--no-save"]).is_err());
    }

    #[test]
    fn capture_select_takes_only_output_options() {
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

    fn open_of(path: &str) -> Result<Command, ArgError> {
        Ok(Command::Open(Open {
            path: path.to_owned(),
        }))
    }

    #[test]
    fn open_takes_the_path_of_a_scrap_file() {
        assert_eq!(
            parse(&["open", r"shots\a.shunsoku"]),
            open_of(r"shots\a.shunsoku")
        );
        assert_eq!(
            parse(&["open", "C:/a b/メモ.SHUNSOKU"]),
            open_of("C:/a b/メモ.SHUNSOKU")
        );
    }

    #[test]
    fn open_needs_exactly_one_scrap_file() {
        let bad: [&[&str]; 6] = [
            &["open"],
            &["open", "a.shunsoku", "b.shunsoku"],
            &["open", "--scrap", "a.shunsoku"],
            &["open", "a.png"],
            &["open", ".shunsoku"],
            &["open", r"C:\a.shunsoku\"],
        ];
        for args in bad {
            assert!(parse(args).is_err(), "{args:?} should be rejected");
        }
    }

    #[test]
    fn the_resident_accepts_only_an_absolute_scrap_file_path() {
        let check = |path: &str| {
            check_open(&Open {
                path: path.to_owned(),
            })
        };
        for path in [
            r"C:\shots\a.shunsoku",
            "d:/A.Shunsoku",
            r"\\server\share\a.shunsoku",
        ] {
            assert!(check(path).is_ok(), "{path} should be accepted");
        }
        for path in [
            "a.shunsoku",
            r"C:a.shunsoku",
            r"\a.shunsoku",
            r"\\.\C:\a.shunsoku",
            r"\\?\C:\a.shunsoku",
            r"\??\C:\a.shunsoku",
            r"\\.\pipe\a.shunsoku",
            r"\\localhost\pipe\a.shunsoku",
            r"\\localhost\PIPE\a.shunsoku",
            r"\\localhost\mailslot\a.shunsoku",
            r"C:\Windows\a.dll",
        ] {
            assert!(check(path).is_err(), "{path} should be refused");
        }
    }
}
