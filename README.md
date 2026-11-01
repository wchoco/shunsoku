# shunsoku

## 設定

設定ファイルは `%APPDATA%\shunsoku\settings.toml` です。shunsoku はこのファイルを作らないので、
[`docs/settings.sample.toml`](docs/settings.sample.toml) をこの名前でコピーし、変えたい項目を書き換えてください。
サンプルにはすべての項目と、書ける値・既定値の説明があります（サンプルの値はすべて既定値です）。

範囲選択キャプチャのホットキー（`[hotkeys]` の `select_capture`）は既定では未設定なので、まずこれを設定してください。
保存すると、再起動しなくてもすぐに反映されます。
