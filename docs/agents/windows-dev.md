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
- 動いている exe が出力をロックすると `LNK1104` で落ちる。スクリプトは、上書きする 2 つのパス（target の exe と `shunsoku-build\<worktree 名>\` の exe）で動いているものだけを先に止める。ほかの場所で動く shunsoku（ユーザーの常駐、他の worktree の exe）は止めない。exe を名前で止める `taskkill /IM` は使わない。
- ユーザーに起動してもらうとき: `cmd.exe /c start "" "<exe のパス>"`。`windows_subsystem = "windows"` なので標準出力は出ない。ログを見る。
- `cargo.exe` は WSL の PATH にない。スクリプトの外で使うなら `%USERPROFILE%\.cargo\bin` を足す。
- zsh では `time` が予約語で、`VAR=x time cmd` は構文エラーになる。先に `export` する。

- 自動起動を試すときは、環境変数 `SHUNSOKU_AUTOSTART_VALUE` で Run キーの値の名前を試験用のもの（`shunsoku-test` など）に差し替え、ユーザーの登録（`shunsoku`）を書き換えない。試す前に `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run` を控え、終わったら同じであることを確かめる。

## 4. 描画を見て確かめる

描画（スクラップ、範囲選択の画面、メニュー）を変えたら、3. の exe を動かして画面を撮り、Read で画像を見る。テストは見た目の違い（灰色の正方形、太さの違う枠）を捕まえない。

```sh
scripts/win-screenshot.sh <out.png> [x y width height]   # 範囲は仮想スクリーンの物理ピクセル
```

- 範囲を省くと全モニタ（この環境では 7680×3240）を撮る。Read に渡すなら、見たいところだけを範囲で撮る。
- 出力は scratchpad に置く。リポジトリには入れない。
- スクラップを出す・ドラッグするなどの操作は `SendInput` で送る。手本は `tools/perf-harness/src/measure.rs` の `drag` と `send_hotkey`。

## Win32 の落とし穴

実装で踏んだもの。該当する API を触るときに読む。

- **マウスの座標**: lParam の座標は符号つき 16 bit（`lp.0 as i16`）。主モニタの左や上のモニタでは負になる。
- **モニタをまたぐドラッグ**: 始めたウィンドウで `SetCapture` し、client 座標はそのウィンドウの原点で screen 座標に直す。
- **キー入力**: オーバーレイではフォーカスが 1 枚のウィンドウにしか来ない。キーはウィンドウごとではなく共有の選択状態に流す。Alt つきは `WM_SYSKEYDOWN` で来る。
- **Desktop Duplication**: 出力はアダプタに属し、D3D11 デバイスはアダプタごとに作る（別アダプタのデバイスでは `DXGI_ERROR_UNSUPPORTED`）。`DuplicateOutput` 直後のフレームは `LastPresentTime == 0` で空のことがある。`DXGI_ERROR_ACCESS_LOST` は作り直し、セキュアデスクトップでの `E_ACCESSDENIED` は「いまは撮れない」として扱う。回転したモニタのテクスチャは回転前の向き。
- **HDR**: `DuplicateOutput` は HDR のモニタの画像を 8bit に落として返す。FP16（scRGB、1.0 = 80 nit）で受けるには `IDXGIOutput5::DuplicateOutput1` に `[R16G16B16A16_FLOAT, B8G8R8A8_UNORM]` を渡す（SDR のモニタは BGRA8 のまま来る）。「SDR コンテンツの明るさ」は `DisplayConfigGetDeviceInfo(DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL)`（1000 が 80 nit）。HMONITOR とは `GetMonitorInfoW` の `szDevice` と、パスの `GET_SOURCE_NAME` の `viewGdiDeviceName` で突き合わせる。変換は `crates/shunsoku/src/hdr.rs`。
- **flip model のスワップチェーン**: `WS_EX_LAYERED` とは併用できない。Alt+Enter を止めるため `MakeWindowAssociation(DXGI_MWA_NO_ALT_ENTER | DXGI_MWA_NO_WINDOW_CHANGES)`。
- **レイヤードウィンドウ**: `UpdateLayeredWindow` には premultiplied alpha の 32bpp DIB を渡す。alpha 0 の画素はクリックが下に抜ける。
- **DPI（PMv2）**: manifest で設定済み（`crates/shunsoku/build.rs`）。`SetProcessDpiAwarenessContext` のエラーは無視してよい。スクラップは `WM_DPICHANGED` で提案された矩形を当てず、新しい DPI を core の `ScrapModel::set_dpi` に渡す（大きさは設定の `dpi_scaling` で決まる）。`WM_DPICHANGED` はウィンドウを動かしている間（App を借りている間）に届くので、自分に `PostMessageW` で送り直してから扱う。
- **タスクトレイ**: `HWND_MESSAGE` のウィンドウにはブロードキャストの `TaskbarCreated` が来ない。`TrackPopupMenu` の前に `SetForegroundWindow`、後に `PostMessageW(WM_NULL)`。
- **クリップボード**: `SetClipboardData` が成功したら HGLOBAL はシステムのもの。失敗したら `GlobalFree`。`CF_DIBV5` は bottom-up で置く。
- **設定ファイルの監視**: `ReadDirectoryChangesW` はディレクトリを見る。エディタの保存は一時ファイル + rename なので `FILE_NOTIFY_CHANGE_FILE_NAME` も要る。1 回の保存で何度も来るのでまとめる。
- **メモリの計測**: `PROCESS_MEMORY_COUNTERS_EX.PrivateUsage` は private commit で、タスクマネージャの「メモリ」（private working set）ではない。`docs/perf.md` の数字と比べるなら同じカウンタを使う。
- **windows 0.62**: `BOOL` は `windows::core::BOOL`。多くの関数が `Option<HWND>` を取る。COM のインターフェイスは `!Send`。
