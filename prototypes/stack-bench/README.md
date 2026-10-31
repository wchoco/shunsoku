# PROTOTYPE: 技術スタックの計測（shunsoku #5）

捨てるコード。main にはマージしない。結論は issue #5 の解決コメントと `docs/adr/0001-*.md` にある。

- `raw/`: Rust + windows-rs で Win32 を直接呼ぶ。常駐 → Ctrl+Shift+F9 → 画面を固定して範囲選択 → 最前面のスクラップ。
  `--capture dd|dd-keep|gdi|wgc`、`--trim`（撮影後に IDXGIDevice3::Trim）、`--lazy`（D3D デバイスを毎回作る）、`--prewarm`（範囲選択のウィンドウを先に作っておく）。
- `winit-min/`: 比較用。winit + global-hotkey + tray-icon + softbuffer で、ホットキーから最前面ウィンドウを出すまで（撮影なし）。
- `harness/`: 計測ドライバ。`harness startup|latency|memory <exe> ...`。SendInput でホットキーを送り、アプリが書く QPC ログから集計する。

ビルド（WSL から Windows の cargo を使う場合）:

```sh
CARGO_TARGET_DIR='C:\...\target' WSLENV=CARGO_TARGET_DIR cargo.exe build --release
```

注意: Desktop Duplication は開いた直後のフレームが `LastPresentTime=0`（中身が空）なので、画像が届くまで取り直している。
