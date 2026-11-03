//! 引数の誤りで終わるときの終了コードと出力（docs/manual-checks.md の M3-19）。
//!
//! 引数は常駐本体に依頼する前に確かめるので、どのテストも常駐本体を起動せず、パイプにも触れない。

use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shunsoku-cli"))
        .args(args)
        .output()
        .expect("shunsoku-cli を起動できない")
}

fn assert_argument_error(args: &[&str], reason: &str) {
    let out = cli(args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
    assert!(out.stdout.is_empty(), "{args:?}: 標準出力に何か出た");
    assert!(stderr.contains(reason), "{args:?}: {stderr}");
}

#[test]
fn no_save_alone_is_an_argument_error() {
    assert_argument_error(
        &["capture", "rect", "0,0,100,100", "--no-save"],
        "--scrap か --copy",
    );
}

#[test]
fn out_other_than_png_is_an_argument_error() {
    assert_argument_error(
        &["capture", "rect", "0,0,100,100", "--out", "x.jpg"],
        "拡張子は .png",
    );
}

#[test]
fn help_prints_the_usage_and_succeeds() {
    let out = cli(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("終了コード:"));
}
