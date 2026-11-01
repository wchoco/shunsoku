//! キーの組み合わせ（ホットキーとショートカット）。設定ファイルの文字列から読み、表示用の文字列に戻す。
//!
//! 書き方は `Ctrl+Shift+A` のように、修飾キーとキー 1 つを `+` でつなぐ。大文字と小文字は区別せず、
//! `+` の前後の空白は無視する。表示は `Win+Ctrl+Alt+Shift+<キー>` の順で、キーの名前は表の最初の名前にする。
//! 設定 GUI の入力欄では入れられない Win キーと PrintScreen を含む組み合わせも書ける（ADR-0004）。
//!
//! キーは Windows の仮想キーコードで持つ。core は `windows` に依存しないので、番号だけを表に持つ。

use std::fmt;
use std::str::FromStr;

/// 修飾キー。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Modifiers {
    pub win: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// 修飾キーでないキー 1 つ。Windows の仮想キーコード（`VK_*`）で持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(u16);

impl Key {
    /// 仮想キーコードからキーを作る。名前の表にないキー（修飾キーなど）は None。
    pub fn from_vk(vk: u16) -> Option<Key> {
        name_of(vk).map(|_| Key(vk))
    }

    /// 定数の表（`scrap::ops` の既定のショートカット）に書くためのもの。名前の表にあるキーだけを渡す
    /// （`scrap::ops` のテストで、表示の名前があることを確かめる）。
    pub(crate) const fn known(vk: u16) -> Key {
        Key(vk)
    }

    /// 仮想キーコード。
    pub fn vk(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Key は表にあるものからしか作れない
        f.write_str(name_of(self.0).unwrap_or("?"))
    }
}

/// キーの組み合わせ。修飾キー（なくてもよい）とキー 1 つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Chord {
    /// 修飾キーなしの組み合わせ（定数の表用）。
    pub(crate) const fn plain(key: Key) -> Chord {
        Chord {
            modifiers: Modifiers {
                win: false,
                ctrl: false,
                alt: false,
                shift: false,
            },
            key,
        }
    }

    /// `Ctrl` を足した組み合わせ（定数の表用）。
    pub(crate) const fn ctrl(self) -> Chord {
        let mut chord = self;
        chord.modifiers.ctrl = true;
        chord
    }

    /// `Shift` を足した組み合わせ（定数の表用）。
    pub(crate) const fn shift(self) -> Chord {
        let mut chord = self;
        chord.modifiers.shift = true;
        chord
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.win, "Win"),
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// キーの組み合わせとして読めない文字列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    input: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a key combination: {:?}", self.input)
    }
}

impl std::error::Error for ParseError {}

impl FromStr for Chord {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Chord, ParseError> {
        let err = || ParseError {
            input: s.to_owned(),
        };
        let mut modifiers = Modifiers::default();
        let mut key = None;
        for part in s.split('+').map(str::trim) {
            // キーのあとに何か続くのは不正（`A+Ctrl`、`A+B`）
            if key.is_some() {
                return Err(err());
            }
            let flag = match part.to_ascii_lowercase().as_str() {
                "win" => Some(&mut modifiers.win),
                "ctrl" | "control" => Some(&mut modifiers.ctrl),
                "alt" => Some(&mut modifiers.alt),
                "shift" => Some(&mut modifiers.shift),
                _ => None,
            };
            match flag {
                // 同じ修飾キーを 2 回書くのは書き間違いとみなす
                Some(flag) if *flag => return Err(err()),
                Some(flag) => *flag = true,
                None => key = Some(Key(vk_of(part).ok_or_else(err)?)),
            }
        }
        let key = key.ok_or_else(err)?;
        Ok(Chord { modifiers, key })
    }
}

/// キーの名前と仮想キーコード。同じコードの名前が複数あるときは、最初のものを表示に使う。
const NAMED_KEYS: &[(&str, u16)] = &[
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Return", 0x0D),
    ("Pause", 0x13),
    ("Esc", 0x1B),
    ("Escape", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PgUp", 0x21),
    ("PageDown", 0x22),
    ("PgDn", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("PrintScreen", 0x2C),
    ("PrtSc", 0x2C),
    ("Insert", 0x2D),
    ("Ins", 0x2D),
    ("Delete", 0x2E),
    ("Del", 0x2E),
    ("NumpadMultiply", 0x6A),
    ("NumpadAdd", 0x6B),
    ("NumpadSubtract", 0x6D),
    ("NumpadDecimal", 0x6E),
    ("NumpadDivide", 0x6F),
    // 記号のキーは配列で刻印が変わるので、日本語配列と英語配列で同じ記号が載っているものだけ
    ("Plus", 0xBB),
    ("Comma", 0xBC),
    ("Minus", 0xBD),
    ("Period", 0xBE),
];

/// 名前（大文字と小文字を区別しない）から仮想キーコードを引く。
fn vk_of(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    // A〜Z と 0〜9 は仮想キーコードが ASCII の大文字・数字と同じ
    if let [c] = bytes {
        if c.is_ascii_lowercase() {
            return Some(u16::from(c.to_ascii_uppercase()));
        }
        if c.is_ascii_digit() {
            return Some(u16::from(*c));
        }
    }
    if let Some(n) = number_after(&lower, "f").filter(|n| (1..=24).contains(n)) {
        return Some(0x70 + n - 1);
    }
    if let Some(n) = number_after(&lower, "numpad").filter(|n| *n <= 9) {
        return Some(0x60 + n);
    }
    NAMED_KEYS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|&(_, vk)| vk)
}

/// `prefix` のあとが 10 進の数字だけなら、その値。
fn number_after(s: &str, prefix: &str) -> Option<u16> {
    let digits = s.strip_prefix(prefix)?;
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// 仮想キーコードの表示用の名前。
fn name_of(vk: u16) -> Option<&'static str> {
    const LETTERS: [&str; 26] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z",
    ];
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const FUNCTIONS: [&str; 24] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14",
        "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
    ];
    const NUMPAD: [&str; 10] = [
        "Numpad0", "Numpad1", "Numpad2", "Numpad3", "Numpad4", "Numpad5", "Numpad6", "Numpad7",
        "Numpad8", "Numpad9",
    ];
    let i = usize::from(vk);
    match vk {
        0x41..=0x5A => Some(LETTERS[i - 0x41]),
        0x30..=0x39 => Some(DIGITS[i - 0x30]),
        0x70..=0x87 => Some(FUNCTIONS[i - 0x70]),
        0x60..=0x69 => Some(NUMPAD[i - 0x60]),
        _ => NAMED_KEYS.iter().find(|&&(_, v)| v == vk).map(|&(n, _)| n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_displays_ctrl_shift_letter() {
        let chord: Chord = "Ctrl+Shift+A".parse().unwrap();
        assert_eq!(chord.to_string(), "Ctrl+Shift+A");
    }

    fn roundtrip(s: &str) -> String {
        s.parse::<Chord>().unwrap().to_string()
    }

    #[test]
    fn win_key_and_print_screen_combinations_roundtrip() {
        assert_eq!(roundtrip("Win+Shift+S"), "Win+Shift+S");
        assert_eq!(roundtrip("PrintScreen"), "PrintScreen");
        assert_eq!(roundtrip("Win+PrintScreen"), "Win+PrintScreen");
        assert_eq!(roundtrip("Ctrl+Alt+PrintScreen"), "Ctrl+Alt+PrintScreen");
    }

    #[test]
    fn win_and_print_screen_map_to_their_virtual_keys() {
        let chord: Chord = "Win+PrintScreen".parse().unwrap();
        assert_eq!(
            chord.modifiers,
            Modifiers {
                win: true,
                ..Default::default()
            }
        );
        assert_eq!(chord.key.vk(), 0x2C); // VK_SNAPSHOT
    }

    #[test]
    fn display_is_normalized_in_case_order_spacing_and_aliases() {
        assert_eq!(roundtrip("shift + ctrl + a"), "Ctrl+Shift+A");
        assert_eq!(
            roundtrip("Alt+Win+Control+Shift+f12"),
            "Win+Ctrl+Alt+Shift+F12"
        );
        assert_eq!(roundtrip("PrtSc"), "PrintScreen");
        assert_eq!(roundtrip("Ctrl+Escape"), "Ctrl+Esc");
    }

    #[test]
    fn keys_map_to_windows_virtual_key_codes() {
        let vk = |s: &str| s.parse::<Chord>().unwrap().key.vk();
        assert_eq!(vk("A"), 0x41);
        assert_eq!(vk("Z"), 0x5A);
        assert_eq!(vk("0"), 0x30);
        assert_eq!(vk("F1"), 0x70);
        assert_eq!(vk("F24"), 0x87);
        assert_eq!(vk("Numpad0"), 0x60);
        assert_eq!(vk("Esc"), 0x1B);
        assert_eq!(vk("Space"), 0x20);
        assert_eq!(vk("Left"), 0x25);
        assert_eq!(vk("Plus"), 0xBB);
    }

    #[test]
    fn key_from_virtual_key_code_displays_its_name() {
        assert_eq!(Key::from_vk(0x53).unwrap().to_string(), "S");
        assert_eq!(Key::from_vk(0x2C).unwrap().to_string(), "PrintScreen");
        assert_eq!(Key::from_vk(0x10), None); // VK_SHIFT は修飾キー
    }

    #[test]
    fn rejects_invalid_strings() {
        for s in [
            "",
            "   ",
            "Ctrl",
            "Ctrl+Shift",
            "Ctrl+",
            "+A",
            "Ctrl++A",
            "Ctrl+Ctrl+A",
            "A+B",
            "A+Ctrl",
            "Ctrl+Foo",
            "F0",
            "F25",
            "Numpad10",
            "Hyper+A",
            "Ｃtrl+A",
        ] {
            assert!(s.parse::<Chord>().is_err(), "{s:?} should be rejected");
        }
    }
}
