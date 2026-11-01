//! ファイル名テンプレート。保存画像のファイル名を決める。
//!
//! 変数:
//!
//! - `{name}`: 定型領域の名前（M3）。範囲選択キャプチャでは空。
//! - `{date}`: `yyyyMMdd`。`{time}`: `HHmmss`。
//! - `{datetime:<書式>}`: `yyyy`（年 4 桁）、`yy`、`MM`、`dd`、`HH`、`mm`、`ss`、`fff`（ミリ秒）を置き換え、
//!   ほかの文字はそのまま。
//! - `{seq}`、`{seq:N}`（N 桁にゼロ埋め）: 連番。保存先フォルダの既存ファイル名の一覧から、同じテンプレートで
//!   付けた名前の最大の番号 + 1（なければ 1）にする。
//!
//! 知らない変数（`{foo}`）はそのまま残す。展開したあと、ファイル名に使えない文字（`<>:"/\|?*` と制御文字）を
//! `_` に置き換える。末尾の `.` と空白、`CON` などの予約された名前も避ける。
//! 同じ名前のファイルが既にあれば、`_2`、`_3` … を付けて避ける（`{seq}` があるときは連番を進める）。
//! 名前の比較は Windows に合わせて大文字と小文字を区別しない。

/// 日時（ローカル時刻）。Win32 層が `GetLocalTime` から作る。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
}

/// 変数の値。
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// `{name}` の値。
    pub name: &'a str,
    pub now: LocalTime,
}

/// テンプレートを展開したものの 1 片。連番は番号が決まるまで残しておく。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Seq { width: usize },
}

/// `template` を展開し、`existing`（保存先フォルダの既存ファイル名）と重ならないファイル名を返す。
/// `extension` は `.` なしで渡す（`"png"`）。
pub fn file_name(
    template: &str,
    context: &Context,
    existing: &[String],
    extension: &str,
) -> String {
    let pieces = expand(template, context);
    let taken = |stem: &str| {
        let name = format!("{stem}.{extension}");
        existing
            .iter()
            .any(|e| e.to_lowercase() == name.to_lowercase())
    };
    let has_seq = pieces.iter().any(|p| matches!(p, Piece::Seq { .. }));
    if has_seq {
        let mut seq = next_seq(&pieces, existing, extension);
        loop {
            let stem = render(&pieces, seq);
            if !taken(&stem) {
                return format!("{stem}.{extension}");
            }
            seq += 1;
        }
    }
    let stem = render(&pieces, 0);
    if !taken(&stem) {
        return format!("{stem}.{extension}");
    }
    (2u64..)
        .map(|n| format!("{stem}_{n}"))
        .find(|candidate| !taken(candidate))
        .map(|stem| format!("{stem}.{extension}"))
        .expect("some suffix is free")
}

/// 変数を展開する。連番だけは `Piece::Seq` で残す。
fn expand(template: &str, context: &Context) -> Vec<Piece> {
    let t = context.now;
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        text.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            text.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let var = &after[..close];
        let whole = &rest[open..open + close + 2];
        rest = &after[close + 1..];
        match var {
            "name" => text.push_str(context.name),
            "date" => text.push_str(&format_time("yyyyMMdd", t)),
            "time" => text.push_str(&format_time("HHmmss", t)),
            "seq" => {
                pieces.push(Piece::Text(std::mem::take(&mut text)));
                pieces.push(Piece::Seq { width: 0 });
            }
            _ => {
                if let Some(width) = var.strip_prefix("seq:").and_then(|n| n.parse().ok()) {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                    pieces.push(Piece::Seq { width });
                } else if let Some(format) = var.strip_prefix("datetime:") {
                    text.push_str(&format_time(format, t));
                } else {
                    text.push_str(whole);
                }
            }
        }
    }
    text.push_str(rest);
    pieces.push(Piece::Text(text));
    pieces
}

/// `{datetime:<書式>}` の書式で日時を書く。
fn format_time(format: &str, t: LocalTime) -> String {
    let tokens: [(&str, String); 8] = [
        ("yyyy", format!("{:04}", t.year)),
        ("yy", format!("{:02}", t.year % 100)),
        ("MM", format!("{:02}", t.month)),
        ("dd", format!("{:02}", t.day)),
        ("HH", format!("{:02}", t.hour)),
        ("mm", format!("{:02}", t.minute)),
        ("ss", format!("{:02}", t.second)),
        ("fff", format!("{:03}", t.millisecond)),
    ];
    let mut out = String::new();
    let mut rest = format;
    'outer: while !rest.is_empty() {
        for (token, value) in &tokens {
            if let Some(after) = rest.strip_prefix(token) {
                out.push_str(value);
                rest = after;
                continue 'outer;
            }
        }
        let c = rest.chars().next().expect("rest is not empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// 連番を `seq` にして、ファイル名に使える形にする。
fn render(pieces: &[Piece], seq: u64) -> String {
    let mut stem = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => stem.push_str(text),
            Piece::Seq { width } => stem.push_str(&format!("{seq:0width$}")),
        }
    }
    sanitize(&stem)
}

/// 既存のファイルのうち、同じテンプレートで付けた名前の最大の連番 + 1。なければ 1。
fn next_seq(pieces: &[Piece], existing: &[String], extension: &str) -> u64 {
    let suffix = format!(".{}", extension.to_lowercase());
    existing
        .iter()
        .filter_map(|name| {
            let name = name.to_lowercase();
            let stem = name.strip_suffix(&suffix)?;
            let seq = parse_seq(pieces, stem)?;
            // 読み取った番号で付け直して同じ名前になるものだけを数える
            (render(pieces, seq).to_lowercase() == stem).then_some(seq)
        })
        .max()
        .map_or(1, |max| max.saturating_add(1))
}

/// `stem`（小文字）から最初の連番の位置の数字を読む。
fn parse_seq(pieces: &[Piece], stem: &str) -> Option<u64> {
    let mut rest = stem;
    for piece in pieces {
        match piece {
            Piece::Text(text) => {
                rest = rest.strip_prefix(sanitize(text).to_lowercase().as_str())?;
            }
            Piece::Seq { .. } => {
                let digits =
                    rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                return rest[..digits].parse().ok();
            }
        }
    }
    None
}

/// ファイル名に使えない文字を `_` にする。末尾の `.` と空白、予約された名前（`CON` など）も避ける。
fn sanitize(stem: &str) -> String {
    let mut out: String = stem
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let kept = out.trim_end_matches(['.', ' ']).len();
    let trailing = out.len() - kept;
    out.truncate(kept);
    out.extend(std::iter::repeat_n('_', trailing));
    if out.is_empty() {
        out.push('_');
    }
    let base = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.insert(0, '_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: LocalTime = LocalTime {
        year: 2026,
        month: 3,
        day: 7,
        hour: 9,
        minute: 5,
        second: 4,
        millisecond: 12,
    };

    fn name(template: &str, existing: &[&str]) -> String {
        let context = Context {
            name: "area",
            now: NOW,
        };
        let existing: Vec<String> = existing.iter().map(|s| (*s).to_owned()).collect();
        file_name(template, &context, &existing, "png")
    }

    #[test]
    fn the_default_template_names_the_file_by_date_and_time() {
        assert_eq!(
            name("shunsoku_{date}-{time}", &[]),
            "shunsoku_20260307-090504.png"
        );
    }

    #[test]
    fn name_and_datetime_are_expanded() {
        assert_eq!(name("{name}", &[]), "area.png");
        assert_eq!(
            name("{datetime:yyyy-MM-dd HH.mm.ss.fff} {datetime:yy}", &[]),
            "2026-03-07 09.05.04.012 26.png"
        );
    }

    #[test]
    fn unknown_variables_and_unclosed_braces_are_kept() {
        assert_eq!(name("a{foo}b{", &[]), "a{foo}b{.png");
    }

    #[test]
    fn seq_starts_at_1_and_seq_n_is_zero_padded() {
        assert_eq!(name("shot_{seq}", &[]), "shot_1.png");
        assert_eq!(name("shot_{seq:3}", &[]), "shot_001.png");
    }

    #[test]
    fn seq_continues_from_the_largest_number_among_existing_files() {
        let existing = [
            "shot_001.png",
            "SHOT_007.PNG",
            "shot_003.png",
            "shot_abc.png",
            "other_099.png",
            "shot_050.jpg",
        ];
        assert_eq!(name("shot_{seq:3}", &existing), "shot_008.png");
        // 日付が変われば別の名前なので 1 から
        assert_eq!(
            name("{date}_{seq}", &["20260306_5.png", "20260307_2.png"]),
            "20260307_3.png"
        );
    }

    #[test]
    fn characters_not_allowed_in_file_names_become_underscores() {
        assert_eq!(
            name(r#"a<b>c:d"e/f\g|h?i*j{datetime:HH:mm}"#, &[]),
            "a_b_c_d_e_f_g_h_i_j09_05.png"
        );
        // 制御文字と、末尾の . と空白
        assert_eq!(name("{name}\t. ", &[]), "area___.png");
        assert_eq!(name("con", &[]), "_con.png");
        assert_eq!(name("", &[]), "_.png");
    }

    #[test]
    fn an_existing_file_with_the_same_name_gets_a_number() {
        let first = "shunsoku_20260307-090504.png";
        assert_eq!(
            name("shunsoku_{date}-{time}", &[first]),
            "shunsoku_20260307-090504_2.png"
        );
        assert_eq!(
            name(
                "shunsoku_{date}-{time}",
                &[first, "Shunsoku_20260307-090504_2.png"]
            ),
            "shunsoku_20260307-090504_3.png"
        );
    }

    #[test]
    fn seq_ignores_files_whose_number_does_not_round_trip_and_skips_a_taken_name() {
        // shot_01 は {seq} で付けた名前ではない（付け直すと shot_1）
        assert_eq!(
            name("shot_{seq}", &["shot_01.png", "shot_2.png"]),
            "shot_3.png"
        );
        // s10 は s{seq}0 の 1 番だが、数字が続くので番号を読み取れない。1 番は使われているので 2 番
        assert_eq!(name("s{seq}0", &["s10.png"]), "s20.png");
    }
}
