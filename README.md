# shunsoku

画面の一部を撮り、付箋のように画面上へ浮かべて扱う（スクラップ）、Windows 用のスクリーンショットツールです。

- 動作対象: Windows 11 / Windows 10（x64）。確かめているのは Windows 11 です。
- ライセンス: MIT（[`LICENSE`](LICENSE)）。使っているクレートのライセンスは、zip の中の `THIRD-PARTY-LICENSES.md` にあります。

## 導入

1. [GitHub Releases](https://github.com/wchoco/shunsoku/releases) から `shunsoku-v<version>-x64.zip` を落とします。
2. 必要なら、同じリリースの `.sha256` のファイルと照らして、zip が壊れていないことを確かめます（PowerShell）。

   ```powershell
   Get-FileHash .\shunsoku-v0.1.0-x64.zip -Algorithm SHA256
   ```

3. 好きなフォルダに展開し、`shunsoku.exe` を起動します。トレイに常駐します。インストーラはなく、VC++ ランタイムも要りません。

zip の中身は `shunsoku.exe`（本体）、`shunsoku-cli.exe`（CLI）、`README.md`、`LICENSE`、`THIRD-PARTY-LICENSES.md` です。

やめるときは、トレイメニューから終了し、フォルダを消します。設定とログは `%APPDATA%\shunsoku` にあります（ポータブルモードでは exe の横）。
自動起動を ON にしていたら、先に OFF にしてから消してください。

## 使い方

### 初めての起動

範囲選択キャプチャのホットキーは既定では未設定です。未設定のまま起動すると、設定 GUI が「キャプチャ」タブで開くので、ホットキーを入れて OK を押してください。
キャンセルで閉じても常駐は続き、トレイアイコンの左クリックとトレイメニューから撮れます。

### 設定

トレイメニューの「設定…」で設定 GUI が開きます。常駐中にもう一度 `shunsoku.exe` を起動しても開きます。
「一般」「キャプチャ」「定型領域」「スクラップ」「スクラップ操作」「書き込み」の 6 つのタブがあり、適用すると再起動しなくてもすぐに反映されます。
ホットキーとショートカットの欄では、割り当てたいキーを押します。Backspace か Delete を押すと「なし」に戻ります。

設定は `%APPDATA%\shunsoku\settings.toml` に保存されます。ファイルを直接書き換えてもかまいません（保存するとすぐに反映されます）。
すべての項目と、書ける値・既定値の説明は [`docs/settings.sample.toml`](docs/settings.sample.toml) にあります。
設定 GUI で適用しても、手で書いたコメントと並びは残ります。

UI の言語は、既定では Windows の表示言語に合わせます（日本語でなければ英語）。「一般」タブで日本語・英語を選べます。

### ポータブルモード

USB メモリなどに入れて持ち運び、PC に設定とログを残さずに使うためのモードです。

- `shunsoku.exe` と同じフォルダに `settings.toml` を置くと、ポータブルモードで動きます。中身は空でもかまいません。
- 設定とログ（`shunsoku.log`）は、exe の横に書きます。`%APPDATA%` は使いません。
- exe の横の `settings.toml` に書き込めないとき（読み取り専用のメディアなど）は、通知が出ます。`%APPDATA%` には書きません。
- 閉じたスクラップの一時ファイルは、ポータブルモードでも `%TEMP%\shunsoku` に置き、終了するときと次に起動したときに消します。

zip には `settings.toml` を入れていないので、展開しただけではポータブルモードになりません。

### 自動起動

「一般」タブの自動起動を ON にすると、Windows にサインインしたときに常駐します（既定は OFF）。
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` に、そのときの exe のパスを登録します。管理者権限は要りません。

自動起動を ON にしたまま exe を別のフォルダへ動かしたら（ポータブルモードで別の PC やドライブへ持っていったときも）、**動かした先の exe を 1 回手で起動してください**。
起動したときに登録を今の exe のパスに直すので、次のサインインから動かした先の exe が常駐します。
タスク マネージャーの「スタートアップ アプリ」で無効にした状態には触れません。

### CLI

`shunsoku-cli.exe` から、常駐している本体にキャプチャを頼めます。本体が常駐していなければ、同じフォルダの `shunsoku.exe` を起動します。
CLI の文言は、UI の言語にかかわらず日本語です。

```text
shunsoku-cli capture rect [--monitor <番号>] <x>,<y>,<w>,<h> [出力のオプション]
shunsoku-cli capture region <名前> [出力のオプション]
shunsoku-cli capture select [出力のオプション]
shunsoku-cli capture active [--delay <秒>] [出力のオプション]
shunsoku-cli monitors
shunsoku-cli --help | --version
```

- 既定では、設定の保存先に保存し、保存した絶対パスを出します。
- 出力のオプション: `--scrap`（スクラップを出す）、`--copy`（クリップボードにコピー）、`--no-save`（保存しない。`--scrap` か `--copy` と合わせて使う）、`--out <path>`（このファイルに保存。`.png` だけ）、`--quiet`（トレイ通知を出さない）。
- 終了コード: 0 成功 / 1 キャプチャの失敗 / 2 引数の誤り / 3 常駐本体に依頼できない・バージョンが合わない / 4 ユーザーが取り消した。

詳しくは `shunsoku-cli --help` を見てください。

## 注意

### Windows 10 でのウィンドウキャプチャ

Windows 10 では、ウィンドウキャプチャで撮る瞬間に、撮っているウィンドウの周りに黄色い枠が出ます。
Windows の画面キャプチャの仕組み（Windows.Graphics.Capture）が出すもので、Windows 10 では消せません。Windows 11 では、shunsoku が枠を消すように頼むので出ません。

## 開発

- 開発の流れと CI: [`CLAUDE.md`](CLAUDE.md)
- 性能の記録: [`docs/perf.md`](docs/perf.md)
- リリース前に手で確かめる項目: [`docs/manual-checks.md`](docs/manual-checks.md)
- リリースの作り方: `v<version>` のタグ（試すときは `v<version>-rc.N`）を push すると、`.github/workflows/release.yml` が zip と SHA256 の付いた draft のリリースを作ります。変更点を書き足し、確かめてから手で公開します。
