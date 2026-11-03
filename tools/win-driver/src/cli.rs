//! コマンドラインの解釈。
//!
//! ```text
//! win-driver --root <テスト用のフォルダ> <コマンド> [引数...]
//! ```
//!
//! コマンドの一覧は [`USAGE`]。

use std::str::FromStr;
use std::time::Duration;

use shunsoku_core::keys::{Chord, Modifiers};

/// 待つコマンドの時間切れの既定値。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub const USAGE: &str = "\
usage: win-driver --root <テスト用のフォルダ> <コマンド> [引数...]

起動と停止（テスト用の環境を必ず付ける。PID は <root>\\launched.txt に控える）
  launch [--portable] <exe> [引数...]     起動して PID を出す。待たない
  run [--portable] [--timeout ms] <exe> [引数...]
                                         終わるまで待ち、終了コード・標準出力・標準エラーを出す
                                         exe は shunsoku.exe か shunsoku-cli.exe だけ。入力の宛先になるのは shunsoku.exe
  kill <pid>                             自分が起動した PID だけを止める
  pids                                   自分が起動して、まだ動いている PID

待つ（時間切れは --timeout ms、既定 5000。時間切れなら終了コード 1）
  wait-window [--pid P] [--class C] [--title 部分] [--timeout ms]
  wait-gone   [--pid P] [--class C] [--title 部分] [--timeout ms]
  wait-exit <pid> [--timeout ms]
  wait-log <file> <部分> [--skip 行数] [--timeout ms]   --skip は `lines` で控えた行数
  wait-file <path> [--since unix-ms] [--timeout ms]     --since は `now` で控えた時刻
  wait-focus [--pid P] [--class C] [--id N] [--hwnd H] [--timeout ms]
  lines <file>                           いまの行数
  now                                    いまの時刻（unix ms）

読む
  wins [--pid P]                         見えているトップレベルのウィンドウ
  fg                                     前面のウィンドウと、フォーカスのあるコントロール
  kids <hwnd>                            子のコントロール（ID、クラス、文字、位置、有効か、状態）
  text <hwnd>                            コントロールの文字（WM_GETTEXT）
  menus                                  開いているポップアップメニューの項目
  tabs <hwnd>                            タブの項目（番号、文字、スクリーン座標の中心）と、選ばれているタブ
  listview <hwnd> [--pad px]             列の幅・見出し・項目の文字と、文字が切れているか

入力（毎回、前面のウィンドウがテスト対象か確かめる。違えば送らずに終了コード 3）
  activate <hwnd>                        テスト対象のウィンドウを前面にする
  move <hwnd> <x> <y>                    テスト対象のウィンドウの左上をスクリーン座標へ動かす（モニタをまたぐ確認）
  keys <組み合わせ>...                   例: keys Tab Ctrl+A Enter
  type <文字>                            Unicode の文字として送る
  hold-key <組み合わせ> <ms>             押したまま ms 待ってから離す
  click <x> <y> [left|right|middle]      スクリーン座標（物理ピクセル）。カーソルの下もテスト対象か確かめる
  dblclick <x> <y> [left|right|middle]
  drag <x1> <y1> <x2> <y2> [--button left|right|middle] [--with 修飾キー]
                                         例: --with Ctrl+Shift（押したままドラッグ）。通る点すべての下を確かめる
  wheel <x> <y> <目盛り>                 正は奥へ、負は手前へ
  hold-button <x> <y> <ms> [left|right|middle]
  clip-get                               クリップボードの形式と、文字（あれば）・画像の大きさ（あれば）
  clip-set <文字>                        クリップボードに文字を入れる

入力の例外（宛先が決まっているもの）
  hotkey <組み合わせ>...                 前面がテスト対象でなければタスクバーを前面にしてから送る。前面がテスト対象・
                                         デスクトップ・タスクバーのどれでもなければ送らない
  tray-click <pid>                       テスト対象の通知領域のアイコンを左クリックする。アイコンの上が通知領域で
                                         なければクリックしない。アイコンが隠れていれば、クリックせずに
                                         NIN_SELECT を本体に送る（posted と出す）
  tray-menu <pid>                        テスト対象の通知領域のアイコンのメニューを開く（本体にメッセージを送る）

終了コード: 0 成功、1 失敗・時間切れ、2 使い方の誤り、3 安全の決まりで止めた";

/// 呼び出し 1 回の指定。
#[derive(Debug, PartialEq)]
pub struct Invocation {
    /// テスト用のフォルダ（Windows のパス）。起動した PID の控えと、テスト用の環境の置き場。
    pub root: String,
    pub command: Command,
}

/// ウィンドウの条件。書いたものすべてに合うウィンドウを探す。
#[derive(Debug, Default, PartialEq)]
pub struct WindowFilter {
    pub pid: Option<u32>,
    pub class: Option<String>,
    /// タイトルに含まれる文字。
    pub title: Option<String>,
}

/// フォーカスのあるコントロールの条件。
#[derive(Debug, Default, PartialEq)]
pub struct FocusFilter {
    pub pid: Option<u32>,
    pub class: Option<String>,
    pub id: Option<i32>,
    pub hwnd: Option<isize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// 解釈できなかった。
#[derive(Debug, PartialEq)]
pub enum Usage {
    /// `--help`。使い方を出して成功で終える。
    Help,
    /// 使い方の誤り（終了コード 2）。
    Invalid(String),
}

impl From<String> for Usage {
    fn from(m: String) -> Usage {
        Usage::Invalid(m)
    }
}

impl From<&str> for Usage {
    fn from(m: &str) -> Usage {
        Usage::Invalid(m.into())
    }
}

/// コマンド。
#[derive(Debug, PartialEq)]
pub enum Command {
    Launch {
        portable: bool,
        exe: String,
        args: Vec<String>,
    },
    Run {
        portable: bool,
        timeout: Duration,
        exe: String,
        args: Vec<String>,
    },
    Kill {
        pid: u32,
    },
    Pids,
    WaitWindow {
        filter: WindowFilter,
        timeout: Duration,
    },
    WaitGone {
        filter: WindowFilter,
        timeout: Duration,
    },
    WaitExit {
        pid: u32,
        timeout: Duration,
    },
    WaitLog {
        path: String,
        needle: String,
        skip: usize,
        timeout: Duration,
    },
    WaitFile {
        path: String,
        since: Option<u64>,
        timeout: Duration,
    },
    WaitFocus {
        filter: FocusFilter,
        timeout: Duration,
    },
    Lines {
        path: String,
    },
    Now,
    Wins {
        pid: Option<u32>,
    },
    Fg,
    Kids {
        hwnd: isize,
    },
    Text {
        hwnd: isize,
    },
    Menus,
    Tabs {
        hwnd: isize,
    },
    ListView {
        hwnd: isize,
        pad: Option<i32>,
    },
    Activate {
        hwnd: isize,
    },
    Move {
        hwnd: isize,
        x: i32,
        y: i32,
    },
    Keys {
        chords: Vec<Chord>,
    },
    Type {
        text: String,
    },
    HoldKey {
        chord: Chord,
        duration: Duration,
    },
    Click {
        x: i32,
        y: i32,
        button: Button,
        /// 2 ならダブルクリック。
        count: u32,
    },
    Drag {
        from: (i32, i32),
        to: (i32, i32),
        button: Button,
        /// ドラッグの間押したままにする修飾キー。
        with: Modifiers,
    },
    Wheel {
        x: i32,
        y: i32,
        notches: i32,
    },
    HoldButton {
        x: i32,
        y: i32,
        duration: Duration,
        button: Button,
    },
    ClipGet,
    ClipSet {
        text: String,
    },
    Hotkey {
        chords: Vec<Chord>,
    },
    TrayClick {
        pid: u32,
    },
    TrayMenu {
        pid: u32,
    },
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Invocation, Usage> {
    let mut args = args.into_iter();
    let mut root = None;
    let name = loop {
        match args.next().as_deref() {
            Some("--root") => root = Some(args.next().ok_or("--root の値がない")?),
            Some("-h" | "--help") => return Err(Usage::Help),
            None => return Err(USAGE.into()),
            Some(name) => break name.to_owned(),
        }
    };
    let root = root.ok_or("--root がない（テスト用のフォルダを指定する）")?;
    let rest: Vec<String> = args.collect();
    let command = parse_command(&name, rest)?;
    Ok(Invocation { root, command })
}

fn chords(name: &str, rest: &[String]) -> Result<Vec<Chord>, String> {
    let chords = rest
        .iter()
        .map(|s| Chord::from_str(s).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if chords.is_empty() {
        return Err(format!("{name}: 組み合わせがない"));
    }
    Ok(chords)
}

fn button(name: &str, s: Option<&str>) -> Result<Button, String> {
    match s {
        None | Some("left") => Ok(Button::Left),
        Some("right") => Ok(Button::Right),
        Some("middle") => Ok(Button::Middle),
        Some(b) => Err(format!("{name}: 知らないボタン: {b}")),
    }
}

/// `Ctrl+Shift` のような修飾キーだけの並び。
fn modifiers(s: &str) -> Result<Modifiers, String> {
    let mut m = Modifiers::default();
    for part in s.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" => m.ctrl = true,
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            "win" => m.win = true,
            _ => return Err(format!("修飾キーでない: {part}")),
        }
    }
    Ok(m)
}

fn parse_command(name: &str, rest: Vec<String>) -> Result<Command, String> {
    match name {
        // exe より後ろは、そのまま exe に渡す
        "launch" | "run" => {
            let mut rest = rest.into_iter();
            let mut portable = false;
            let mut timeout = None;
            let exe = loop {
                match rest.next() {
                    Some(a) if a == "--portable" => portable = true,
                    Some(a) if a == "--timeout" && name == "run" => {
                        timeout = Some(millis(&value(&mut rest, "--timeout")?)?)
                    }
                    Some(a) if a.starts_with("--") => {
                        return Err(format!("知らないオプション: {a}"));
                    }
                    Some(exe) => break exe,
                    None => return Err(format!("{name}: exe がない")),
                }
            };
            let args = rest.collect();
            Ok(if name == "launch" {
                Command::Launch {
                    portable,
                    exe,
                    args,
                }
            } else {
                Command::Run {
                    portable,
                    timeout: timeout.unwrap_or(DEFAULT_TIMEOUT),
                    exe,
                    args,
                }
            })
        }
        "keys" => Ok(Command::Keys {
            chords: chords(name, &rest)?,
        }),
        "hotkey" => Ok(Command::Hotkey {
            chords: chords(name, &rest)?,
        }),
        "type" => {
            let [text] = <[String; 1]>::try_from(rest).map_err(|_| "type: 文字を 1 つ渡す")?;
            Ok(Command::Type { text })
        }
        "clip-set" => {
            let [text] = <[String; 1]>::try_from(rest).map_err(|_| "clip-set: 文字を 1 つ渡す")?;
            Ok(Command::ClipSet { text })
        }
        _ => parse_with_options(name, rest),
    }
}

/// `--名前 値` のオプションと、位置の引数をとるコマンド。
fn parse_with_options(name: &str, rest: Vec<String>) -> Result<Command, String> {
    let mut o = Options::split(rest)?;
    let command = match name {
        "kill" => Command::Kill {
            pid: number(&o.positional(0, "pid")?)?,
        },
        "pids" => Command::Pids,
        "wait-window" | "wait-gone" => {
            let filter = WindowFilter {
                pid: o.take("--pid").map(|s| number(&s)).transpose()?,
                class: o.take("--class"),
                title: o.take("--title"),
            };
            if filter == WindowFilter::default() {
                return Err(format!(
                    "{name}: --pid、--class、--title のどれかを指定する"
                ));
            }
            let timeout = o.timeout()?;
            if name == "wait-window" {
                Command::WaitWindow { filter, timeout }
            } else {
                Command::WaitGone { filter, timeout }
            }
        }
        "wait-exit" => Command::WaitExit {
            pid: number(&o.positional(0, "pid")?)?,
            timeout: o.timeout()?,
        },
        "wait-log" => Command::WaitLog {
            path: o.positional(0, "file")?,
            needle: o.positional(1, "部分")?,
            skip: o
                .take("--skip")
                .map(|s| number(&s))
                .transpose()?
                .unwrap_or(0),
            timeout: o.timeout()?,
        },
        "wait-file" => Command::WaitFile {
            path: o.positional(0, "path")?,
            since: o.take("--since").map(|s| number(&s)).transpose()?,
            timeout: o.timeout()?,
        },
        "wait-focus" => {
            let filter = FocusFilter {
                pid: o.take("--pid").map(|s| number(&s)).transpose()?,
                class: o.take("--class"),
                id: o.take("--id").map(|s| number(&s)).transpose()?,
                hwnd: o.take("--hwnd").map(|s| hwnd(&s)).transpose()?,
            };
            if filter == FocusFilter::default() {
                return Err("wait-focus: --pid、--class、--id、--hwnd のどれかを指定する".into());
            }
            Command::WaitFocus {
                filter,
                timeout: o.timeout()?,
            }
        }
        "lines" => Command::Lines {
            path: o.positional(0, "file")?,
        },
        "now" => Command::Now,
        "wins" => Command::Wins {
            pid: o.take("--pid").map(|s| number(&s)).transpose()?,
        },
        "fg" => Command::Fg,
        "kids" => Command::Kids {
            hwnd: hwnd(&o.positional(0, "hwnd")?)?,
        },
        "text" => Command::Text {
            hwnd: hwnd(&o.positional(0, "hwnd")?)?,
        },
        "menus" => Command::Menus,
        "tabs" => Command::Tabs {
            hwnd: hwnd(&o.positional(0, "hwnd")?)?,
        },
        "listview" => Command::ListView {
            hwnd: hwnd(&o.positional(0, "hwnd")?)?,
            pad: o.take("--pad").map(|s| number(&s)).transpose()?,
        },
        "activate" => Command::Activate {
            hwnd: hwnd(&o.positional(0, "hwnd")?)?,
        },
        "move" => Command::Move {
            hwnd: hwnd(&o.positional(0, "hwnd")?)?,
            x: number(&o.positional(1, "x")?)?,
            y: number(&o.positional(2, "y")?)?,
        },
        "click" | "dblclick" => Command::Click {
            x: number(&o.positional(0, "x")?)?,
            y: number(&o.positional(1, "y")?)?,
            button: button(name, o.positional(2, "button").ok().as_deref())?,
            count: if name == "click" { 1 } else { 2 },
        },
        "drag" => Command::Drag {
            from: (
                number(&o.positional(0, "x1")?)?,
                number(&o.positional(1, "y1")?)?,
            ),
            to: (
                number(&o.positional(2, "x2")?)?,
                number(&o.positional(3, "y2")?)?,
            ),
            button: button(name, o.take("--button").as_deref())?,
            with: o
                .take("--with")
                .map(|s| modifiers(&s))
                .transpose()?
                .unwrap_or_default(),
        },
        "wheel" => Command::Wheel {
            x: number(&o.positional(0, "x")?)?,
            y: number(&o.positional(1, "y")?)?,
            notches: number(&o.positional(2, "目盛り")?)?,
        },
        "hold-key" => Command::HoldKey {
            chord: Chord::from_str(&o.positional(0, "組み合わせ")?).map_err(|e| e.to_string())?,
            duration: millis(&o.positional(1, "ms")?)?,
        },
        "hold-button" => Command::HoldButton {
            x: number(&o.positional(0, "x")?)?,
            y: number(&o.positional(1, "y")?)?,
            duration: millis(&o.positional(2, "ms")?)?,
            button: button(name, o.positional(3, "button").ok().as_deref())?,
        },
        "clip-get" => Command::ClipGet,
        "tray-click" => Command::TrayClick {
            pid: number(&o.positional(0, "pid")?)?,
        },
        "tray-menu" => Command::TrayMenu {
            pid: number(&o.positional(0, "pid")?)?,
        },
        _ => return Err(format!("知らないコマンド: {name}\n{USAGE}")),
    };
    o.finish(name)?;
    Ok(command)
}

/// 分けた引数。使ったものから取り除き、最後に残りがあれば誤りにする。
struct Options {
    named: Vec<(String, String)>,
    positional: Vec<Option<String>>,
}

impl Options {
    fn split(rest: Vec<String>) -> Result<Options, String> {
        let mut named = Vec::new();
        let mut positional = Vec::new();
        let mut rest = rest.into_iter();
        while let Some(a) = rest.next() {
            if a.starts_with("--") {
                let v = value(&mut rest, &a)?;
                named.push((a, v));
            } else {
                positional.push(Some(a));
            }
        }
        Ok(Options { named, positional })
    }

    fn take(&mut self, name: &str) -> Option<String> {
        let i = self.named.iter().position(|(n, _)| n == name)?;
        Some(self.named.remove(i).1)
    }

    fn positional(&mut self, i: usize, what: &str) -> Result<String, String> {
        self.positional
            .get_mut(i)
            .and_then(Option::take)
            .ok_or_else(|| format!("{what} がない"))
    }

    fn timeout(&mut self) -> Result<Duration, String> {
        Ok(match self.take("--timeout") {
            Some(s) => millis(&s)?,
            None => DEFAULT_TIMEOUT,
        })
    }

    fn finish(self, name: &str) -> Result<(), String> {
        if let Some((n, _)) = self.named.first() {
            return Err(format!("{name}: 知らないオプション: {n}"));
        }
        if let Some(a) = self.positional.into_iter().flatten().next() {
            return Err(format!("{name}: 余分な引数: {a}"));
        }
        Ok(())
    }
}

fn value(rest: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    rest.next().ok_or_else(|| format!("{name} の値がない"))
}

fn number<T: FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("数でない: {s}"))
}

fn millis(s: &str) -> Result<Duration, String> {
    number(s).map(Duration::from_millis)
}

/// ウィンドウのハンドル。`0x` で始まれば 16 進、そうでなければ 10 進。
fn hwnd(s: &str) -> Result<isize, String> {
    let parsed = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => isize::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    };
    parsed
        .filter(|&h| h != 0)
        .ok_or_else(|| format!("ウィンドウのハンドルでない: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Invocation, Usage> {
        parse(s.split_whitespace().map(str::to_owned))
    }

    fn cmd(s: &str) -> Command {
        p(&format!(r"--root C:\t {s}")).unwrap().command
    }

    #[test]
    fn launch_takes_exe_and_its_arguments() {
        assert_eq!(
            p(r"--root C:\t\m5 launch C:\b\shunsoku.exe --flag x").unwrap(),
            Invocation {
                root: r"C:\t\m5".into(),
                command: Command::Launch {
                    portable: false,
                    exe: r"C:\b\shunsoku.exe".into(),
                    args: vec!["--flag".into(), "x".into()],
                },
            }
        );
    }

    #[test]
    fn launch_without_root_is_refused() {
        assert!(p(r"launch C:\b\shunsoku.exe").is_err());
    }

    #[test]
    fn portable_comes_before_exe() {
        assert_eq!(
            cmd(r"launch --portable C:\b\shunsoku.exe"),
            Command::Launch {
                portable: true,
                exe: r"C:\b\shunsoku.exe".into(),
                args: vec![],
            }
        );
    }

    #[test]
    fn waits_take_a_timeout_in_millis_and_default_to_five_seconds() {
        assert_eq!(
            cmd("wait-window --class #32770 --pid 42 --timeout 1500"),
            Command::WaitWindow {
                filter: WindowFilter {
                    pid: Some(42),
                    class: Some("#32770".into()),
                    title: None,
                },
                timeout: Duration::from_millis(1500),
            }
        );
        assert_eq!(
            cmd(r"wait-log C:\t\shunsoku.log saved --skip 3"),
            Command::WaitLog {
                path: r"C:\t\shunsoku.log".into(),
                needle: "saved".into(),
                skip: 3,
                timeout: Duration::from_secs(5),
            }
        );
    }

    #[test]
    fn wait_window_without_any_condition_is_refused() {
        assert!(p(r"--root C:\t wait-window --timeout 100").is_err());
    }

    #[test]
    fn unknown_and_extra_arguments_are_refused() {
        assert!(p(r"--root C:\t frobnicate").is_err());
        assert!(p(r"--root C:\t kill 12 13").is_err());
        assert!(p(r"--root C:\t fg --pid 3").is_err());
    }

    #[test]
    fn move_takes_a_window_and_a_screen_position() {
        assert_eq!(
            cmd("move 0x80A0C -2000 300"),
            Command::Move {
                hwnd: 0x80A0C,
                x: -2000,
                y: 300
            }
        );
    }

    #[test]
    fn tabs_takes_the_tab_control() {
        assert_eq!(cmd("tabs 0x40D48"), Command::Tabs { hwnd: 0x40D48 });
    }

    #[test]
    fn hwnd_is_hex_with_prefix_or_decimal() {
        assert_eq!(cmd("text 0x1A2B"), Command::Text { hwnd: 0x1A2B });
        assert_eq!(cmd("text 6699"), Command::Text { hwnd: 6699 });
        assert!(p(r"--root C:\t text 0").is_err());
    }

    #[test]
    fn keys_reads_each_argument_as_a_chord() {
        let Command::Keys { chords } = cmd("keys Tab Ctrl+A") else {
            panic!()
        };
        assert_eq!(chords.len(), 2);
        assert_eq!(chords[1].key.vk(), 0x41);
        assert!(chords[1].modifiers.ctrl);
        assert!(p(r"--root C:\t keys Ctrl+").is_err());
    }

    #[test]
    fn help_is_not_an_error() {
        assert_eq!(p("--help"), Err(Usage::Help));
        assert_eq!(p("-h"), Err(Usage::Help));
        assert!(matches!(p(""), Err(Usage::Invalid(_))));
    }

    #[test]
    fn clicks_take_any_of_the_three_buttons_and_a_count() {
        assert_eq!(
            cmd("click 10 -20 middle"),
            Command::Click {
                x: 10,
                y: -20,
                button: Button::Middle,
                count: 1,
            }
        );
        assert_eq!(
            cmd("dblclick 10 20"),
            Command::Click {
                x: 10,
                y: 20,
                button: Button::Left,
                count: 2,
            }
        );
        assert!(p(r"--root C:\t click 1 2 back").is_err());
    }

    #[test]
    fn drag_takes_two_points_a_button_and_held_modifiers() {
        assert_eq!(
            cmd("drag 0 0 100 50"),
            Command::Drag {
                from: (0, 0),
                to: (100, 50),
                button: Button::Left,
                with: Modifiers::default(),
            }
        );
        assert_eq!(
            cmd("drag 0 0 100 50 --button right --with Ctrl+Shift"),
            Command::Drag {
                from: (0, 0),
                to: (100, 50),
                button: Button::Right,
                with: Modifiers {
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                },
            }
        );
        assert!(p(r"--root C:\t drag 0 0 1 1 --with Ctrl+A").is_err());
    }

    #[test]
    fn wheel_takes_a_point_and_signed_notches() {
        assert_eq!(
            cmd("wheel 5 6 -3"),
            Command::Wheel {
                x: 5,
                y: 6,
                notches: -3
            }
        );
    }

    #[test]
    fn holds_take_a_duration_in_millis() {
        let Command::HoldKey { chord, duration } = cmd("hold-key Shift+F10 800") else {
            panic!()
        };
        assert_eq!(chord.key.vk(), 0x79);
        assert_eq!(duration, Duration::from_millis(800));
        assert_eq!(
            cmd("hold-button 5 6 1200 right"),
            Command::HoldButton {
                x: 5,
                y: 6,
                duration: Duration::from_millis(1200),
                button: Button::Right,
            }
        );
    }

    #[test]
    fn hotkey_and_tray_click_and_clipboard() {
        let Command::Hotkey { chords } = cmd("hotkey Ctrl+Alt+F9 Ctrl+Alt+F9") else {
            panic!()
        };
        assert_eq!(chords.len(), 2);
        assert!(p(r"--root C:\t hotkey").is_err());
        assert_eq!(cmd("tray-click 42"), Command::TrayClick { pid: 42 });
        assert_eq!(cmd("clip-get"), Command::ClipGet);
        assert_eq!(
            cmd("clip-set こんにちは"),
            Command::ClipSet {
                text: "こんにちは".into()
            }
        );
    }
}
