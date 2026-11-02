//! `shunsoku-cli` と常駐本体の間の、名前付きパイプでの依頼と応答の形式（ADR-0003）。

use crate::cli_args::{Capture, Output, Save, Target};
use crate::geometry::Rect;
use crate::monitors::Monitor;

/// プロトコルのバージョン。形式を変えたら上げる。互換性は保たず、違えば断る。
pub const VERSION: u32 = 2;

/// 1 通の依頼・応答の大きさの上限（バイト）。これより大きいものは読まずに断る。
pub const MAX_MESSAGE: usize = 64 * 1024;

/// 名前付きパイプの名前。セッションの番号とユーザーの SID（文字列）を含め、別のセッション・別のユーザーの
/// 常駐本体とは別のパイプにする。開けるのを現在のユーザーに限るのはセキュリティ記述子の役目（`pipe_server`）。
pub fn pipe_name(session: u32, user_sid: &str) -> String {
    format!(r"\\.\pipe\shunsoku-{session}-{user_sid}")
}

/// 依頼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Capture(Capture),
    Monitors,
}

/// 読めなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// 相手のバージョンが違う（読めなければ None）。
    VersionMismatch(Option<u32>),
    /// 形式が違う。
    Malformed(String),
}

/// 依頼を書く。
///
/// 形式は UTF-8 のテキスト。1 行目が `shunsoku <バージョン>`、残りは 1 行に 1 つの `<キー> <値>`。
/// 値の中の `\`・改行・復帰は `\\`・`\n`・`\r` にする。
pub fn encode_request(request: &Request) -> String {
    let mut w = Writer::new();
    match request {
        Request::Monitors => w.field("command", "monitors"),
        Request::Capture(capture) => {
            w.field("command", "capture");
            match &capture.target {
                Target::Rect { monitor, rect } => {
                    w.field("target", "rect");
                    if let Some(n) = monitor {
                        w.field("monitor", &n.to_string());
                    }
                    w.field("rect", &rect_value(rect));
                }
                Target::Region { name } => {
                    w.field("target", "region");
                    w.field("name", name);
                }
                Target::Select => w.field("target", "select"),
                Target::Active { delay } => {
                    w.field("target", "active");
                    w.field("delay", &delay.to_string());
                }
            }
            let output = &capture.output;
            match &output.save {
                Save::Default => w.field("save", "default"),
                Save::To(path) => {
                    w.field("save", "file");
                    w.field("out", path);
                }
                Save::No => w.field("save", "none"),
            }
            w.field("scrap", flag(output.scrap));
            w.field("copy", flag(output.copy));
            w.field("quiet", flag(output.quiet));
        }
    }
    w.finish()
}

/// 依頼を読む。
pub fn decode_request(text: &str) -> Result<Request, DecodeError> {
    let mut r = Reader::new(text)?;
    let request = match r.take("command")? {
        "monitors" => Request::Monitors,
        "capture" => {
            let target = match r.take("target")? {
                "rect" => {
                    let monitor = r.take_optional("monitor").map(parse_number).transpose()?;
                    let rect = parse_rect(r.take("rect")?)?;
                    Target::Rect { monitor, rect }
                }
                "region" => Target::Region {
                    name: r.take("name")?.to_owned(),
                },
                "select" => Target::Select,
                "active" => Target::Active {
                    delay: parse_number(r.take("delay")?)?,
                },
                other => return Err(malformed(&format!("unknown target {other}"))),
            };
            let save = match r.take("save")? {
                "default" => Save::Default,
                "file" => Save::To(r.take("out")?.to_owned()),
                "none" => Save::No,
                other => return Err(malformed(&format!("bad save {other}"))),
            };
            let output = Output {
                save,
                scrap: parse_flag(r.take("scrap")?)?,
                copy: parse_flag(r.take("copy")?)?,
                quiet: parse_flag(r.take("quiet")?)?,
            };
            Request::Capture(Capture { target, output })
        }
        other => return Err(malformed(&format!("unknown command {other}"))),
    };
    r.end()?;
    Ok(request)
}

/// 応答。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// 撮れた。保存したなら、その絶対パス。
    Captured { path: Option<String> },
    /// モニタの一覧（並びは常駐本体が集めた順。番号は `monitors::numbered` で振る）。
    Monitors(Vec<Monitor>),
    /// 失敗した。
    Failed { kind: ErrorKind, message: String },
}

/// 失敗の種類。終了コードを決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// 撮れなかった（範囲選択の画面を開いている、モニタがない、保存できない、裏のスレッドのパニック など）。終了コード 1。
    CaptureFailed,
    /// 引数の誤り。終了コード 2。
    InvalidArguments,
    /// 常駐本体に依頼できない・バージョンが合わない。終了コード 3。
    Unavailable,
    /// ユーザーが取り消した。終了コード 4。
    Cancelled,
}

impl ErrorKind {
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::CaptureFailed => 1,
            ErrorKind::InvalidArguments => 2,
            ErrorKind::Unavailable => 3,
            ErrorKind::Cancelled => 4,
        }
    }

    fn name(self) -> &'static str {
        match self {
            ErrorKind::CaptureFailed => "capture-failed",
            ErrorKind::InvalidArguments => "invalid-arguments",
            ErrorKind::Unavailable => "unavailable",
            ErrorKind::Cancelled => "cancelled",
        }
    }

    fn from_name(name: &str) -> Option<ErrorKind> {
        [
            ErrorKind::CaptureFailed,
            ErrorKind::InvalidArguments,
            ErrorKind::Unavailable,
            ErrorKind::Cancelled,
        ]
        .into_iter()
        .find(|kind| kind.name() == name)
    }
}

impl Response {
    /// CLI の終了コード（0 成功 / 1 キャプチャの失敗 / 2 引数の誤り / 3 依頼できない・バージョン違い / 4 取り消し）。
    pub fn exit_code(&self) -> i32 {
        match self {
            Response::Captured { .. } | Response::Monitors(_) => 0,
            Response::Failed { kind, .. } => kind.exit_code(),
        }
    }

    /// 読めなかった依頼への応答。バージョン違いは終了コード 3、形式の誤りは 2。
    pub fn refused(error: &DecodeError) -> Response {
        match error {
            DecodeError::VersionMismatch(theirs) => Response::Failed {
                kind: ErrorKind::Unavailable,
                message: format!(
                    "shunsoku-cli と常駐本体のバージョンが合いません（プロトコル {} と {VERSION}）。同じ版の shunsoku-cli.exe と shunsoku.exe を使ってください。",
                    theirs.map_or_else(|| "?".to_owned(), |v| v.to_string())
                ),
            },
            DecodeError::Malformed(what) => Response::Failed {
                kind: ErrorKind::InvalidArguments,
                message: format!("依頼の形式が正しくありません: {what}"),
            },
        }
    }
}

/// 応答を書く。形式は `encode_request` と同じ。
pub fn encode_response(response: &Response) -> String {
    let mut w = Writer::new();
    match response {
        Response::Captured { path } => {
            w.field("status", "captured");
            if let Some(path) = path {
                w.field("path", path);
            }
        }
        Response::Monitors(monitors) => {
            w.field("status", "monitors");
            for m in monitors {
                w.field("monitor", &m.name);
                w.field("id", &m.id);
                w.field("rect", &rect_value(&m.rect));
                w.field("dpi", &m.dpi.to_string());
                w.field("primary", flag(m.primary));
            }
        }
        Response::Failed { kind, message } => {
            w.field("status", "failed");
            w.field("kind", kind.name());
            w.field("message", message);
        }
    }
    w.finish()
}

/// 応答を読む。
pub fn decode_response(text: &str) -> Result<Response, DecodeError> {
    let mut r = Reader::new(text)?;
    let response = match r.take("status")? {
        "captured" => Response::Captured {
            path: r.take_optional("path").map(str::to_owned),
        },
        "monitors" => {
            let mut monitors = Vec::new();
            while let Some(name) = r.take_optional("monitor") {
                let name = name.to_owned();
                let id = r.take("id")?.to_owned();
                let rect = parse_rect(r.take("rect")?)?;
                let dpi = parse_number(r.take("dpi")?)?;
                let primary = parse_flag(r.take("primary")?)?;
                monitors.push(Monitor {
                    id,
                    name,
                    rect,
                    dpi,
                    primary,
                });
            }
            Response::Monitors(monitors)
        }
        "failed" => {
            let kind = r.take("kind")?;
            let kind =
                ErrorKind::from_name(kind).ok_or_else(|| malformed(&format!("bad kind {kind}")))?;
            let message = r.take("message")?.to_owned();
            Response::Failed { kind, message }
        }
        other => return Err(malformed(&format!("unknown status {other}"))),
    };
    r.end()?;
    Ok(response)
}

fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn parse_flag(value: &str) -> Result<bool, DecodeError> {
    match value {
        "1" => Ok(true),
        "0" => Ok(false),
        other => Err(malformed(&format!("bad flag {other}"))),
    }
}

fn rect_value(rect: &Rect) -> String {
    format!("{} {} {} {}", rect.left, rect.top, rect.right, rect.bottom)
}

fn parse_rect(value: &str) -> Result<Rect, DecodeError> {
    let parts: Vec<i32> = value
        .split(' ')
        .map(|s| s.parse().map_err(|_| malformed("bad rect")))
        .collect::<Result<_, _>>()?;
    let [left, top, right, bottom] = parts.as_slice() else {
        return Err(malformed("bad rect"));
    };
    Ok(Rect::new(*left, *top, *right, *bottom))
}

fn parse_number<T: std::str::FromStr>(value: &str) -> Result<T, DecodeError> {
    value
        .parse()
        .map_err(|_| malformed(&format!("bad number {value}")))
}

fn malformed(what: &str) -> DecodeError {
    DecodeError::Malformed(what.to_owned())
}

/// 1 通を書く。
struct Writer(String);

impl Writer {
    fn new() -> Writer {
        Writer(format!("shunsoku {VERSION}\n"))
    }

    fn field(&mut self, key: &str, value: &str) {
        self.0.push_str(key);
        self.0.push(' ');
        for c in value.chars() {
            match c {
                '\\' => self.0.push_str("\\\\"),
                '\n' => self.0.push_str("\\n"),
                '\r' => self.0.push_str("\\r"),
                c => self.0.push(c),
            }
        }
        self.0.push('\n');
    }

    fn finish(self) -> String {
        self.0
    }
}

/// 1 通を頭から読む。キーは書いた順に読む。
struct Reader<'a> {
    lines: std::iter::Peekable<std::str::Lines<'a>>,
    /// `take` で返す値の置き場（エスケープを戻したもの）。
    values: Vec<String>,
}

impl<'a> Reader<'a> {
    /// 1 行目のバージョンを照合する。
    fn new(text: &'a str) -> Result<Reader<'a>, DecodeError> {
        let mut lines = text.lines().peekable();
        let version = lines
            .next()
            .and_then(|line| line.strip_prefix("shunsoku "))
            .map(str::parse::<u32>);
        match version {
            Some(Ok(VERSION)) => Ok(Reader {
                lines,
                values: Vec::new(),
            }),
            Some(Ok(other)) => Err(DecodeError::VersionMismatch(Some(other))),
            _ => Err(DecodeError::VersionMismatch(None)),
        }
    }

    /// 次の行のキーが `key` ならその値。
    fn take_optional(&mut self, key: &str) -> Option<&str> {
        let line = self.lines.peek()?;
        let value = line.strip_prefix(key)?.strip_prefix(' ')?;
        let value = unescape(value)?;
        self.lines.next();
        self.values.push(value);
        self.values.last().map(String::as_str)
    }

    fn take(&mut self, key: &str) -> Result<&str, DecodeError> {
        self.take_optional(key)
            .ok_or_else(|| malformed(&format!("missing {key}")))
    }

    /// 読み残しがないこと。
    fn end(mut self) -> Result<(), DecodeError> {
        match self.lines.next() {
            None => Ok(()),
            Some(line) => Err(malformed(&format!("unexpected line {line}"))),
        }
    }
}

fn unescape(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_capture(monitor: Option<u32>) -> Request {
        Request::Capture(Capture {
            target: Target::Rect {
                monitor,
                rect: Rect::new(-5, 10, 95, 210),
            },
            output: Output::default(),
        })
    }

    #[test]
    fn requests_survive_a_round_trip() {
        for request in [rect_capture(None), rect_capture(Some(3)), Request::Monitors] {
            let wire = encode_request(&request);
            assert_eq!(decode_request(&wire), Ok(request));
        }
    }

    #[test]
    fn capture_outputs_survive_a_round_trip() {
        let outputs = [
            Output {
                save: Save::No,
                scrap: true,
                copy: true,
                quiet: true,
            },
            Output {
                save: Save::To("C:\\撮影\\a\nb.png".to_owned()),
                ..Output::default()
            },
        ];
        for output in outputs {
            let request = Request::Capture(Capture {
                target: Target::Rect {
                    monitor: None,
                    rect: Rect::new(0, 0, 1, 1),
                },
                output,
            });
            let wire = encode_request(&request);
            assert_eq!(decode_request(&wire), Ok(request));
        }
    }

    #[test]
    fn region_select_and_active_requests_survive_a_round_trip() {
        let targets = [
            Target::Region {
                name: "領域 1\\a\nb".to_owned(),
            },
            Target::Select,
            Target::Active { delay: 0 },
            Target::Active { delay: 60 },
        ];
        for target in targets {
            let request = Request::Capture(Capture {
                target,
                output: Output::default(),
            });
            let wire = encode_request(&request);
            assert_eq!(decode_request(&wire), Ok(request));
        }
    }

    #[test]
    fn responses_survive_a_round_trip_including_awkward_text() {
        let responses = [
            Response::Captured {
                path: Some(r"C:\Users\me\Pictures\a b\shot 1.png".to_owned()),
            },
            Response::Captured { path: None },
            Response::Monitors(vec![
                Monitor {
                    id: r"\\?\DISPLAY#GSM1#4&1&UID1#{e6f07b5f}".to_owned(),
                    name: r"\\.\DISPLAY1".to_owned(),
                    rect: Rect::new(-1920, 0, 0, 1080),
                    dpi: 120,
                    primary: false,
                },
                Monitor {
                    id: r"\\.\DISPLAY2".to_owned(),
                    name: "DELL U2720Q".to_owned(),
                    rect: Rect::new(0, 0, 3840, 2160),
                    dpi: 144,
                    primary: true,
                },
            ]),
            Response::Monitors(Vec::new()),
            Response::Failed {
                kind: ErrorKind::CaptureFailed,
                message: "1 行目\n2 行目\\末尾\r".to_owned(),
            },
        ];
        for response in responses {
            let wire = encode_response(&response);
            assert_eq!(decode_response(&wire), Ok(response));
        }
    }

    #[test]
    fn a_message_from_another_version_is_refused() {
        let other = "shunsoku 999\ncommand monitors\n";
        assert_eq!(
            decode_request(other),
            Err(DecodeError::VersionMismatch(Some(999)))
        );
        assert_eq!(
            decode_response("shunsoku 0\nstatus ok\n"),
            Err(DecodeError::VersionMismatch(Some(0)))
        );
        assert_eq!(
            decode_request("hello\n"),
            Err(DecodeError::VersionMismatch(None))
        );
    }

    #[test]
    fn the_pipe_name_is_per_session_and_per_user() {
        assert_eq!(
            pipe_name(2, "S-1-5-21-1-2-3-1001"),
            r"\\.\pipe\shunsoku-2-S-1-5-21-1-2-3-1001"
        );
        assert_ne!(pipe_name(1, "S-1-5-21-1"), pipe_name(2, "S-1-5-21-1"));
        assert_ne!(pipe_name(1, "S-1-5-21-1"), pipe_name(1, "S-1-5-21-2"));
    }

    #[test]
    fn the_reply_to_a_refused_version_asks_for_exit_code_3() {
        let reply = Response::refused(&DecodeError::VersionMismatch(Some(999)));
        assert_eq!(reply.exit_code(), 3);
        assert_eq!(decode_response(&encode_response(&reply)), Ok(reply));
    }

    #[test]
    fn a_malformed_request_is_refused_as_an_argument_error() {
        assert!(matches!(
            decode_request(&format!("shunsoku {VERSION}\ncommand dance\n")),
            Err(DecodeError::Malformed(_))
        ));
        let reply = Response::refused(&DecodeError::Malformed("x".to_owned()));
        assert_eq!(reply.exit_code(), 2);
    }

    #[test]
    fn outcomes_map_to_exit_codes() {
        let failed = |kind| Response::Failed {
            kind,
            message: String::new(),
        };
        assert_eq!(Response::Captured { path: None }.exit_code(), 0);
        assert_eq!(Response::Monitors(Vec::new()).exit_code(), 0);
        assert_eq!(failed(ErrorKind::CaptureFailed).exit_code(), 1);
        assert_eq!(failed(ErrorKind::InvalidArguments).exit_code(), 2);
        assert_eq!(failed(ErrorKind::Unavailable).exit_code(), 3);
        assert_eq!(failed(ErrorKind::Cancelled).exit_code(), 4);
    }
}
