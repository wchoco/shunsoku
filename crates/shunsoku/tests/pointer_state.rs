//! 左ボタンの状態（`MK_LBUTTON`）とマウスのキャプチャ（`GetCapture()`）を Win32 層で読むのは
//! `app.rs` の `pointer_on_move` だけにする。ほかのハンドラはそこで作った `Pointer` を core に渡す
//! （CODING_STANDARDS.md の「入力で変わる状態は core に置く」）。
//!
//! Win32 層は Linux ではコンパイルされないので、ソースの文字列で確かめる。

use std::fs;
use std::path::Path;

const ALLOWED_FILE: &str = "app.rs";
const ALLOWED_FN: &str = "fn pointer_on_move(";
const WATCHED: [&str; 2] = ["MK_LBUTTON", "GetCapture()"];

#[test]
fn pointer_state_is_read_only_in_pointer_on_move() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    for entry in fs::read_dir(&src).expect("src を読めない") {
        let path = entry.expect("src の項目を読めない").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("ソースを読めない");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = if name == ALLOWED_FILE {
            without_fn(&text, ALLOWED_FN)
        } else {
            text
        };
        for line in text.lines() {
            if WATCHED.iter().any(|w| line.contains(w)) {
                violations.push(format!("{name}: {}", line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "左ボタンの状態とキャプチャは pointer_on_move で読み、Pointer を core に渡す:\n{}",
        violations.join("\n")
    );
}

/// `signature` で始まる関数の本体（対応する `}` まで）を除いた文字列。見つからなければ落とす。
fn without_fn(text: &str, signature: &str) -> String {
    let start = text.find(signature).unwrap_or_else(|| {
        panic!("{ALLOWED_FILE} に `{signature}` がない。このテストの許可先を直す")
    });
    let open = start + text[start..].find('{').expect("関数の本体がない");
    let mut depth = 0;
    for (offset, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let end = open + offset + 1;
                    return format!("{}{}", &text[..start], &text[end..]);
                }
            }
            _ => {}
        }
    }
    panic!("`{signature}` の本体が閉じていない");
}
