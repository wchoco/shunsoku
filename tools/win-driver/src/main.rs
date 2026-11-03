//! 実機の手動確認の道具（開発専用。zip には入れない）。使い方と決まりは `docs/agents/manual-check.md`、
//! コマンドの一覧は `win-driver --help`（[`cli::USAGE`]）。
//!
//! 安全の決まりを道具の中に持つ。
//!
//! - 起動するものには必ずテスト用の環境（`test_env`）を付ける。付けずに起動するコマンドはない。
//! - キーやマウスを送る前に、毎回前面のウィンドウを確かめる。テスト対象でなければ送らずに終了コード 3 で終える（`guard`）。
//! - 止めるのは自分が起動した PID だけ（`<root>\launched.txt` に控える）。

mod cli;
mod guard;
mod input;
mod list_view;
mod test_env;
mod wait;
#[cfg(windows)]
mod win;

fn main() {
    let inv = match cli::parse(std::env::args().skip(1)) {
        Ok(inv) => inv,
        Err(cli::Usage::Help) => {
            println!("{}", cli::USAGE);
            return;
        }
        Err(cli::Usage::Invalid(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    #[cfg(windows)]
    match win::run(inv) {
        Ok(()) => {}
        Err(win::Failure::Failed(m)) => {
            eprintln!("FAILED {m}");
            std::process::exit(1);
        }
        Err(win::Failure::Refused(m)) => {
            eprintln!("REFUSED {m}");
            std::process::exit(3);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = inv;
        eprintln!("win-driver は Windows でだけ動きます");
        std::process::exit(1);
    }
}
