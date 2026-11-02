# 設定 GUI は Win32 コモンコントロールで作り、常駐プロセスの中で開く

設定 GUI は、windows-rs から Win32 のコモンコントロール（comctl32 v6）を直接使って作る。`DIALOGEX` のリソースと `CreateDialog` によるモードレスダイアログとして、常駐プロセスの UI スレッドで開く。二重に開こうとしたら既存の画面を前に出す。

MVP では次の 2 つを割り切る。

- **ダークモードはタイトルバーだけ**（`DWMWA_USE_IMMERSIVE_DARK_MODE`）。コントロールの中身は明るいまま。
- **ショートカットの入力欄は自作しない**。標準の hot key control（`msctls_hotkey32`）をサブクラス化して使う。全体のホットキーとスクラップ操作のショートカットで共通に使う。
  - `WM_GETDLGCODE` では、標準の処理の戻り値に `DLGC_WANTALLKEYS` を足して返す。Esc と Enter をダイアログに取られないようにするため。
  - `WM_KEYDOWN` / `WM_SYSKEYDOWN` もサブクラスで受ける。修飾キーの状態と合わせて `HKM_SETHOTKEY` で値を入れ、標準の処理には渡さない。標準の処理は、Esc と Enter を「なし」として扱うため。
  - Tab / Shift+Tab だけはサブクラスで受けず、ダイアログに任せる。フォーカスを次の欄へ移すため。
  - Win キーと PrintScreen は GUI から入力できない。マウスの割り当ては入力欄ではなく、決まった候補からドロップダウンで選ぶ。

比較の材料は [設定 GUI の実装手段の比較調査](https://github.com/wchoco/shunsoku/issues/15)、決定の詳細は [設定 GUI の実装手段の決定](https://github.com/wchoco/shunsoku/issues/18) の解決コメントにある。

## Considered Options

- **Win32 コモンコントロール（採用）**: 開いている間も閉じた後も増分は約 +0.8MB で、常駐時 30MB の目標をほぼ圧迫しない。IME と Per-Monitor V2 は OS 側の支援が最も厚い。弱点は、コントロールの中身のダークモードが非公開 API か自前描画頼みなことと、標準の hot key control が Win キーを表せないこと。どちらも MVP では要求から外した。
- **Slint（software renderer）**: ダークモードはシステムに追従し、Windows キーも受け取れる。一方、増分は開いている間 +2.9MB、閉じた後 +1.6MB で、exe も大きい。日本語表示・IME と、常駐側の Win32 メッセージループとの共存は確かめられていない。上の 2 つの弱点を MVP で捨てたので、選ぶ理由がなくなった。
- **egui（eframe）**: 閉じた後も約 15〜16MB が残る。Windows キーと PrintScreen を受け取れない。IME の不具合報告も多い。
- **設定画面を別プロセスにする**: 閉じればメモリは確実に戻る。ただし、設定反映の通知と二重起動の制御が要る。Win32 案では閉じた後の残りが小さく、利点がほぼない。

## Consequences

- 設定ファイルには、GUI から入力できないホットキー（Win キー、PrintScreen を含む組み合わせ）も書けて、そのまま効く。GUI はそれを表示だけして、編集すると上書きする。既定のホットキーには、GUI で直せない組み合わせを使わない。
- 実装前に試作で 2 点を確かめた（[設定 GUI の試作](https://github.com/wchoco/shunsoku/issues/38)、`prototype/settings-dialog` ブランチ、Windows 11、US 配列）。
  - DPI の違うモニタ間でダイアログを動かしても、文字と配置は崩れず、タイトルバーのダークも保たれた。PMv2 のダイアログマネージャに任せる方式と、`WM_DPICHANGED` で手で置き直す方式のどちらでも問題はなかった。
  - 入力欄は、当初の案（`WM_GETDLGCODE` で `DLGC_WANTALLKEYS` を返すだけ）では、Esc と Enter が「なし」になった。キー押下もサブクラスで受ける方式にすると、Esc・Enter・矢印・Alt+矢印・Ctrl+`+`（テンキーを含む）・Ctrl+`=` が入った。この結果に合わせて、上の入力欄の作り方を改めた（[ADR-0004 の見直し](https://github.com/wchoco/shunsoku/issues/49)）。
- 中身のダークモードや、Win キーを受けられる入力欄が必要になったら、まずこの構成のままで、非公開 API か入力欄の自作を検討する。GPU レンダラ系の GUI ライブラリに乗り換えるなら、メモリを戻すために設定画面を別プロセスにする（外から依頼を受ける口は ADR-0003 の名前付きパイプを流用できる）。
