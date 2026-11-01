# PROTOTYPE: 範囲選択の画面を出す撮影と表示の方式（shunsoku #14）

捨てるコード。main にはマージしない。結論は issue #14 の解決コメントと `docs/adr/0002-*.md` にある。
`prototype/5-stack-bench`（#5 の計測プロトタイプ）を複製して、`raw/` に次のフラグを足した。

- `--par`: Desktop Duplication の取得をモニタごとにスレッドで並列に行う（D3D11 デバイスは `SetMultithreadProtected(true)`）。
- `--display gdi|gpu-per|gpu-one`: 範囲選択の画面の出し方。
  - `gdi`（既定）: 従来どおり CPU に書き戻した全画面の DIB を GDI で描く。
  - `gpu-per`: 撮った画像を GPU 上のテクスチャに置いたまま、モニタごとの最前面ウィンドウに flip-model のスワップチェーン（`FLIP_DISCARD`）で出す。選択枠は `ClearView` の矩形で描く。CPU には切り抜いた範囲だけを書き戻す。
  - `gpu-one`: 同上で、仮想スクリーン全体を 1 枚のウィンドウで覆う。

既存のフラグ（`--capture dd|dd-keep|gdi|wgc`、`--trim`、`--lazy`、`--prewarm`）はそのまま。

ビルドと計測（WSL から。zsh では `${=cfg}` で引数を分割すること）:

```sh
CARGO_TARGET_DIR='C:\Users\<you>\AppData\Local\Temp\shunsoku-14-target' WSLENV=CARGO_TARGET_DIR cargo.exe build --release -p bench-raw -p harness
harness.exe latency <bench-raw.exe の Windows パス> 20 --capture dd --display gpu-per --par --trim
harness.exe memory  <bench-raw.exe の Windows パス> --capture dd --display gpu-per --par --trim
```

## 結果（Win11、Iris Xe、4K 150% ×2 + FHD 100%、20 回の中央値）

| 方式 | 入力→撮影 | 入力→表示 | 入力→DwmFlush | 撮影 10 回後のメモリ |
|---|---|---|---|---|
| `--trim`（現行） | 101ms | 122ms | 136ms | 15.8MB |
| `--par --trim` | 103ms | 129ms | 140ms | — |
| `--display gpu-per --trim` | 62ms | 99ms | 130ms | — |
| `--display gpu-per --par --trim` | 44ms | 84ms | 103ms | 20.6MB |
| `--display gpu-one --par --trim` | 44ms | 97ms | 126ms | 20.4MB |
