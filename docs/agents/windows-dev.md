# WSL から Windows 向けに開発する

shunsoku は Windows 専用。開発は WSL2 で行い、Win32 層の確認と実機用の exe は次の 3 段で行う。

## 1. Linux のツールチェーンで回す（毎回）

コマンドは CLAUDE.md の「WSL では、push の前に」の 3 つ。

- `windows` クレートは `cfg(windows)` の依存なので、Linux の `cargo test --workspace` では Win32 層がコンパイルされない。走るのは core のテストだけ。
- Win32 層の型検査と clippy は `--target x86_64-pc-windows-msvc` で行う。windows-rs は `windows-link`（raw-dylib）を使うので import lib は要らない。
- この target で `cargo build` / `cargo test` は link で落ちる（link.exe がない）。exe は 3. で組む。
- `build.rs` は Linux のホストで走る。target の判定は `cfg!` ではなく `CARGO_CFG_TARGET_OS` / `CARGO_CFG_TARGET_ENV` で行う。
- `-D warnings` で Win32 のコードによく出る lint: `field_reassign_with_default`（`X { cb: .., ..Default::default() }` と書く）、`unnecessary_mut_passed`。

## 2. CI と同じ確認（任意）

CI（`windows-latest`）は CLAUDE.md の 4 コマンドを Windows で回す。手元で同じものを見たいときは、Windows 側の `cargo.exe` を 3. と同じ環境変数で呼ぶ（`cargo.exe clippy --workspace --all-targets -- -D warnings`、`cargo.exe test --workspace`）。

## 3. 実機で動かす exe を組む

```sh
scripts/win-build.sh [worktree-dir=.]   # env: PKG（既定 shunsoku）、PROFILE（既定 release）
# -> %USERPROFILE%\shunsoku-build\<worktree 名>\shunsoku.exe
```

- WSL の cwd のまま `cargo.exe` で組める（rsync は要らない）。`CARGO_TARGET_DIR` は NTFS 上に worktree ごとに置く。初回は 1 分ほど、以降は差分だけ。
- 動いている `shunsoku.exe` が出力をロックすると `LNK1104` で落ちる。スクリプトが先に `taskkill.exe` で止める。
- ユーザーに起動してもらうとき: `cmd.exe /c start "" "<exe のパス>"`。`windows_subsystem = "windows"` なので標準出力は出ない。ログを見る。
- `cargo.exe` は WSL の PATH にない。スクリプトの外で使うなら `%USERPROFILE%\.cargo\bin` を足す。
- zsh では `time` が予約語で、`VAR=x time cmd` は構文エラーになる。先に `export` する。

## Win32 の落とし穴

実装で踏んだもの。該当する API を触るときに読む。

- **マウスの座標**: lParam の座標は符号つき 16 bit（`lp.0 as i16`）。主モニタの左や上のモニタでは負になる。
- **モニタをまたぐドラッグ**: 始めたウィンドウで `SetCapture` し、client 座標はそのウィンドウの原点で screen 座標に直す。
- **キー入力**: オーバーレイではフォーカスが 1 枚のウィンドウにしか来ない。キーはウィンドウごとではなく共有の選択状態に流す。Alt つきは `WM_SYSKEYDOWN` で来る。
- **Desktop Duplication**: 出力はアダプタに属し、D3D11 デバイスはアダプタごとに作る（別アダプタのデバイスでは `DXGI_ERROR_UNSUPPORTED`）。`DuplicateOutput` 直後のフレームは `LastPresentTime == 0` で空のことがある。`DXGI_ERROR_ACCESS_LOST` は作り直し、セキュアデスクトップでの `E_ACCESSDENIED` は「いまは撮れない」として扱う。回転したモニタのテクスチャは回転前の向き。
- **flip model のスワップチェーン**: `WS_EX_LAYERED` とは併用できない。Alt+Enter を止めるため `MakeWindowAssociation(DXGI_MWA_NO_ALT_ENTER | DXGI_MWA_NO_WINDOW_CHANGES)`。
- **レイヤードウィンドウ**: `UpdateLayeredWindow` には premultiplied alpha の 32bpp DIB を渡す。alpha 0 の画素はクリックが下に抜ける。
- **DPI（PMv2）**: manifest で設定済み（`crates/shunsoku/build.rs`）。`SetProcessDpiAwarenessContext` のエラーは無視してよい。スクラップは `WM_DPICHANGED` で提案された矩形を当てない（画素を 1:1 に保つ）。
- **タスクトレイ**: `HWND_MESSAGE` のウィンドウにはブロードキャストの `TaskbarCreated` が来ない。`TrackPopupMenu` の前に `SetForegroundWindow`、後に `PostMessageW(WM_NULL)`。
- **クリップボード**: `SetClipboardData` が成功したら HGLOBAL はシステムのもの。失敗したら `GlobalFree`。`CF_DIBV5` は bottom-up で置く。
- **設定ファイルの監視**: `ReadDirectoryChangesW` はディレクトリを見る。エディタの保存は一時ファイル + rename なので `FILE_NOTIFY_CHANGE_FILE_NAME` も要る。1 回の保存で何度も来るのでまとめる。
- **メモリの計測**: `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` は private commit で、タスクマネージャの「メモリ」（private working set）ではない。`docs/perf.md` の数字と比べるなら同じカウンタを使う。
- **windows 0.62**: `BOOL` は `windows::core::BOOL`。多くの関数が `Option<HWND>` を取る。COM のインターフェイスは `!Send`。
