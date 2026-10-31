# PROTOTYPE: Windows.Graphics.Capture の実機確認（shunsoku #11）

捨てるコード。main にはマージしない。結論は issue #11 の解決コメントにある。

- `wgc-probe border [--no-request] [--border on|off]`: 非パッケージ exe から `RequestAccessAsync(Borderless)` を呼び、マゼンタのウィンドウを WGC で撮りながら、画面 DC の BitBlt で周囲の黄色い画素を数えて枠の有無を機械判定する。
- `wgc-probe elevated [--control]`: 自分を runas で昇格起動した緑のウィンドウを、非昇格のプロセスから画面 BitBlt / `PrintWindow` / WGC で撮る。左半分は非昇格の青いウィンドウで隠す。`--control` は昇格なしの比較用。

ビルド（WSL から Windows の cargo を使う場合）:

```sh
CARGO_TARGET_DIR='C:\...\target' WSLENV=CARGO_TARGET_DIR cargo.exe build --release
```
