//! 名前付きパイプの相手を確かめるための、ユーザー（SID）とセッションの取得。
//!
//! 常駐本体（`pipe_server`）と `shunsoku-cli` の両方が使う。`shunsoku-core` は `windows` クレートに
//! 依存させない（ADR-0005）ので、別のクレートに置く。Windows でだけ中身がある。

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{current_session, current_user_sid, process_user_sid};
