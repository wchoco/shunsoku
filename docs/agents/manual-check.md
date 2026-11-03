# 実機の手動確認（エージェント向け）

`docs/manual-checks.md` の項目を実機で確かめるときの決まり。操作は `tools/win-driver` を通す（組み方と呼び方は [windows-dev.md](windows-dev.md) の 5.、コマンドの一覧は `scripts/win-driver.sh --help`）。

## 安全の決まり

同じ PC でユーザーの shunsoku が常駐し、ほかのエージェントも確かめていることがある。次は道具の中に持っていて、破ると道具が終了コード 3 で止める。道具を通さずに同じことをしない。

- 起動: `launch` / `run` は必ずテスト用の環境を付ける。`APPDATA`（設定とログ）と `TEMP` / `TMP` は `DRV_ROOT` の下、`USERNAME`（多重起動の判定の名前）と `SHUNSOKU_AUTOSTART_VALUE`（Run キーの値の名前）は `shunsoku-test-<DRV_ROOT のフォルダ名>`。ポータブルモードの項目（`--portable`）だけ `APPDATA` を付けない。
- 入力: キー・文字・クリックの前に、毎回前面のウィンドウ（クリックはカーソルの下のウィンドウも）が自分の起動したプロセスのものか確かめる。違えば送らない。ターミナルが前面のままなら、先に `activate <hwnd>` で前面にする。
- 停止: `kill` は自分が起動した PID（`<DRV_ROOT>\launched.txt` に起動時刻と一緒に控える）だけを止める。`taskkill /IM` や PID の指定で、道具の外から止めない。
- 待つ: 固定の `sleep` ではなく `wait-*` で待つ。時間切れ（既定 5 秒、`--timeout ms`）は終了コード 1。時間切れを「動かない」と判定する前に、待つ条件が合っているか（ログの文言、ウィンドウのクラス）を確かめる。
- 片付け: 終わったら `pids` が空になるまで `kill` し、`DRV_ROOT` のフォルダを消す。ユーザーの `%APPDATA%\shunsoku` は読むだけにする（ポータブルモードの項目では、前後の大きさと更新時刻を控えて比べる）。

## 判定の仕方

`docs/manual-checks.md` の「判定」の列に従う。

- 値: 道具で読んだ値で判定する。画像は撮らない。
  - コントロールの文字・状態: `kids <hwnd>`（ID、クラス、文字、位置、有効か、チェック、選択、ホットキー）、`text <hwnd>`、`fg`（前面とフォーカス）。
  - メニュー: `tray-menu <pid>` か右クリックで開いてから `menus`。
  - タブ: `tabs <hwnd>`。一覧: `listview <hwnd>`（文字が切れていれば `CLIPPED` を出して終了コード 1）。
  - ファイルとログ: `now` で時刻を控えてから `wait-file <path> --since <時刻>`、`lines <file>` で行数を控えてから `wait-log <file> <部分> --skip <行数>`。
- 目視: 画像を見ないと判定できないものだけを `scripts/win-screenshot.sh` で撮って見る。目視の項目でも、値で決まる部分（例: M5-10 の、一覧の文字が切れないこと）は値で確かめる。
- 人: エージェントは確かめない。結果に「人」と書いて残す。

## 結果の書き方

項目ごとに 1 行で、判定と根拠の値を書く。

| # | 結果 | 根拠 |
|---|---|---|
| M5-24 | OK | 不透明度に `500` を入れて Tab: 欄が `100` に戻り、`tooltips_class32` に「10 から 100 までの整数…」 |
| M5-15 | OK | 保存後 `portable\settings.toml` が更新され、`portable\shunsoku.log` に `settings written to`。`%APPDATA%\shunsoku` の大きさと更新時刻は前後で同じ |
| M5-19 | 人 | VC++ ランタイムのない PC が要る |

- NG なら、期待と違った値と、それを出したコマンドを書く。
- 環境（モニタの構成、UI の言語、確かめた exe）を表の前に書く。
