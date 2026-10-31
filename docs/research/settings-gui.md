# 設定 GUI の実装手段の比較調査

issue: [#15](https://github.com/wchoco/shunsoku/issues/15)（地図 [#2](https://github.com/wchoco/shunsoku/issues/2)）
調査日: 2026-11-01

前提: 技術スタックは Rust + windows-rs で Win32 を直接呼ぶ（[ADR-0001](../adr/0001-rust-windows-rs-win32.md)）。#5 の計測で、常駐時の Private Working Set は 15.8MB だった。目標は常駐時 30MB 以下、コールドスタート 500ms 以下。UI は日英対応で、文言は外部ファイルに置く。Per-Monitor V2（PMv2）は必須。

この文書は材料と推奨候補までを示す。決定はしない。

## 結論（先に要約）

- **推奨: Win32 コモンコントロール（comctl32 v6）を windows-rs から直接使う**。メモリの増分が最も小さく（開いている間も閉じた後も +0.8MB。下の計測）、IME と PMv2 は OS 側の支援が最も厚い。弱点は 2 つある。
  1. **ダークモード**: 公式に文書化されているのはタイトルバーだけ。コントロールの中身を暗くするには、非公開 API を使うか自前で描く必要がある。
  2. **ホットキーの入力欄**: 標準の hot key control は Win キーを修飾キーとして表せない。サブクラス化した入力欄を自作することになる。
- **次点: Slint を software renderer で使う**。計測では、開いている間 +2.9MB、閉じた後 +1.6MB。ダークモードは既定でシステムに追従し、翻訳の仕組み（`@tr` と gettext）、Windows キーの修飾子もある。ただし既定の GPU レンダラ（femtovg）では閉じた後も約 17MB が残る。exe は 12MB（既定 feature）。ライセンスは Royalty-free（Slint 使用の表示が条件）で使う必要がある。
- **egui（eframe）は不利**。閉じた後も約 15〜16MB が残る（常駐 15.8MB と合わせると 30MB 前後になる。推測）。日本語フォントを読むと開いている間 +46MB。入力の型に Windows キーと PrintScreen がない。
- **別プロセス案**は、どの GUI 手段とも組み合わせられる。閉じればメモリは確実に OS に返る。常駐側の目標を GUI の手段から切り離したい場合の保険になる。代わりに、設定の反映（常駐プロセスへの通知）と二重起動の制御が必要になる。

## 比較表

凡例: 「計測」は本調査の手元計測（後述の条件）。「推測」は一次情報や計測で裏づけていない見込み。

| 観点 | Win32 コモンコントロール | Slint（software renderer） | Slint（femtovg、既定） | egui（eframe + glow） | 別プロセス |
| --- | --- | --- | --- | --- | --- |
| 開いている間の増分（Private WS） | +0.8MB（計測） | +2.9MB（計測） | +29MB（計測） | 日本語フォントありで +46MB、なしで +27MB（計測） | 常駐側は 0（設定プロセス側に上の値） |
| 閉じた後の残り（Private WS） | +0.8MB（計測） | +1.6MB（計測） | +17MB（計測） | +15〜16MB（計測） | 0（プロセス終了で OS が回収） |
| 開閉を繰り返したときの増加 | なし（2 回目も同じ値。計測） | なし（計測） | なし（計測） | なし（計測） | — |
| PMv2 | OS が子ウィンドウにも DPI 変更を通知。ダイアログ（`CreateDialog`）と、コモンコントロールのテーマ描画部品は OS が自動でスケール。その他のレイアウトはアプリの責任 [1] | winit 経由で対応（推測。#5 で winit の PMv2 自動設定は確認済み） | 同左 | winit 経由。`pixels_per_point` で拡大率を扱う [8] | 採用した手段に従う |
| ダークモード | 公式に文書化されているのはタイトルバー（`DWMWA_USE_IMMERSIVE_DARK_MODE`）だけ [2]。コントロールは非公開 API か自前描画 [3][4] | 既定でシステムの明暗に追従（fluent スタイル）[11] | 同左 | 既定でシステムに追従（`ThemePreference::System`）[8] | 採用した手段に従う |
| IME | 標準の EDIT コントロールで OS が処理（IMM/TSF の既定の IME ウィンドウ）[5] | Windows の不具合報告は過去にあり、多くは修正済み。未解決の報告もある [13] | 同左 | Windows を含む IME の不具合報告が多い。2026 年に修正が続いたが未解決も残る [9] | 採用した手段に従う |
| 文言の外部ファイル化と日英切り替え | 文字列は実行時に `SetWindowTextW` などで入れるだけなので、形式は自由（TOML、Fluent など） | `@tr` + gettext（`.mo` を実行時に読む）か、バイナリ埋め込み + `select_bundled_translation` で実行中に切り替え [12]。Rust 側からプロパティで流し込むこともできる | 同左 | 即時モードなので、毎フレーム渡す文字列を差し替えるだけ。形式は自由 | 採用した手段に従う |
| ホットキー入力欄 | 標準の hot key control（`HOTKEY_CLASS`）は修飾キーが Ctrl/Alt/Shift だけで、Win キーを表せない [6][7]。自作が現実的 | `KeyEvent` の修飾子に `meta`（Windows では Windows キー）がある [14]。PrintScreen は `SysReq` として届く [15]。自作の部品になる | 同左 | `Modifiers` に Windows キーがない [10a]。egui-winit は Windows で super キーを捨てている [10b]。`Key` に PrintScreen、Pause、ScrollLock がない [10c] | 採用した手段に従う |
| exe サイズ（計測、release、strip、thin LTO、CRT 静的リンク） | 408KB（設定画面だけの最小構成） | 12.1MB（既定 feature。femtovg と software の両方を含む） | 同左 | 5.8MB（フォントは同梱せず、実行時に `meiryo.ttc` を読む） | 別 exe にする場合は 2 本になる。同じ exe を引数で起動し分けるなら 1 本 |
| 保守性 | windows-rs だけで完結し、依存は増えない。レイアウト、DPI 対応の再配置、ダークモードは自前 | 24.3k stars、v1.18.1（2026-09）。宣言的 UI で部品が揃う。GPLv3 / Royalty-free / 商用の 3 択 [16] | 同左 | 31.9k stars、0.37.0（2026-10）。MIT/Apache-2.0 | プロセス間の連携のコードが増える |

## 計測

### 条件

- 3 つの最小の設定画面を作り、同じ手順で測った（コードは使い捨て。リポジトリには入れていない）。
  - 画面の中身: ラベル、テキスト入力（日本語の初期値）、ホットキー表示、言語のコンボボックス、チェックボックス、OK ボタン。Win32 版はリストビューと hot key control も置いた。
  - 手順: 起動して 1.5 秒待って測る（開く前）→ 設定画面を開いて 3 秒後に測る（開いている間）→ 閉じて 3 秒後に測る（閉じた後）→ もう 1 回開閉する。
  - 測り方: #5 の harness と同じく、`Win32_PerfRawData_PerfProc_Process` の `WorkingSetPrivate`（Private WS）を外から取る。
- バージョン: windows 0.61、eframe 0.33.3（既定 feature、glow）、slint 1.16.1（既定 feature、winit + femtovg）。手元の Windows 側 Rust が 1.91 で、最新の eframe 0.37（MSRV 1.95）と slint 1.18（MSRV 1.92）はビルドできなかった。そのため、最新版の数値は未確認。
- マニフェストで comctl32 v6 と PMv2 を宣言。release、`strip = true`、`lto = "thin"`、`+crt-static`。
- 計測機の GPU と Windows のビルド番号は記録していない。GPU レンダラの残りメモリはドライバに左右される見込みが高い（推測）。

### 結果（Private WS、MB）

| 構成 | 開く前 | 開いている間（1 回目 / 2 回目） | 閉じた後（1 回目 / 2 回目） |
| --- | --- | --- | --- |
| Win32 コモンコントロール | 0.8 | 1.6 / 1.7 | 1.6 / 1.6 |
| Slint、software renderer（`SLINT_BACKEND=winit-software`） | 1.3 | 4.2 / 4.3 | 2.9 / 2.9 |
| Slint、femtovg（既定） | 1.3 | 30.4 / 29.8 | 18.0 / 18.2 |
| egui、日本語フォントなし | 1.2 | 27.8 / 27.5 | 16.2 / 16.5 |
| egui、`meiryo.ttc`（9.5MB）を読み込み | 1.2 | 47.5 / 48.8 | 16.8 / 16.8 |

読み取り:

- 閉じた後に残る量は、GPU レンダラ（OpenGL）を使う構成で 15〜17MB。Slint を software renderer に替えると 1.6MB まで下がる。このことから、残りの大半は GL の初期化（ドライバの DLL と確保した領域）と見る（推測。DLL ごとの内訳は取っていない）。
- 開閉を 2 回繰り返しても増えないので、リークではなく、初回に一度だけ払う固定のコストになっている。
- egui は日本語を表示するためにフォントを自分で用意する必要がある（既定のフォントに CJK がない [10]）。フォントのファイルを丸ごと読むと、その分が開いている間のメモリに乗る。
- eframe 0.33 の `run_native` は、閉じた後に同じプロセスでもう一度呼べた（`run_and_return` が既定で true [8a]）。
- **未確認**: どの構成でも、描画結果（日本語の字形、DPI の違うモニタ間の移動、ダークモード）は目視していない。WSL から起動したウィンドウが前面にならず、スクリーンショットを取れなかったため。Slint の software renderer で日本語が正しく出るかは要確認。

## 候補ごとの詳細

### Win32 コモンコントロール（windows-rs から直接）

- **PMv2**: PMv2 では、トップレベルと子の HWND の両方に DPI の変更が通知される。非クライアント領域、`CreateDialog` で作ったダイアログ、comctl32 v6 のテーマ描画の部品（チェックボックスやボタン背景など）は OS が自動でスケールする。一方で、「Windows common controls (comctl32) ... do not support automatic DPI scaling」とも書かれている。つまり、子コントロールの位置と大きさを `WM_DPICHANGED` で直すのはアプリの責任になる [1]。
  - ダイアログのリソース（`.rc` の `DIALOGEX`）で作れば、ダイアログ単位のレイアウトごと OS がスケールするので負担は小さい（[1] の「Dialogs (CreateDialog)」の自動スケールからの推測。要試作）。
- **ダークモード**: Microsoft の文書が扱うのは、システムの明暗の検出（`UISettings` の前景色）とタイトルバーの暗色化（`DwmSetWindowAttribute` + `DWMWA_USE_IMMERSIVE_DARK_MODE`）だけ。「It does not cover specifics of how to repaint and render your app UI using a Dark mode color set」と明記されている [2]。
  - コントロールの中身を暗くする方法として広く使われているのは、uxtheme.dll の非公開 API（序数 135 の `SetPreferredAppMode` など）と、`SetWindowTheme(hwnd, L"DarkMode_Explorer", ...)` を組み合わせるやり方。サンプルの説明でも「undocumented dark mode API introduced in Windows 10 1809」とされている [3]。
  - Notepad++ 系のライブラリ win32-darkmodelib（MPL-2.0）は主要なコントロールに対応しているが、「For some controls darkmodelib uses IAT hooks」と書いている [4]。
  - 選択肢は 3 つ: ①ダークモードはタイトルバーだけにする、②非公開 API を使う（Windows の更新で壊れうる）、③背景・文字色を `WM_CTLCOLOR*` とオーナードローで自前で塗る（推測。手間は部品数に比例する）。
- **IME**: IMM は既定で IME のウィンドウを用意し、入力と候補の選択はそこで行われる [5]。標準の EDIT コントロールなら、アプリ側で IME の処理を書かずに日本語入力が動く（Windows の標準的な挙動。本調査では未計測）。
- **文言**: コントロールの文字列は実行時に設定するので、外部ファイル（TOML や Fluent など）から読んで `SetWindowTextW` / `SetDlgItemTextW` で入れればよい。言語を切り替えるときは入れ直す。ダイアログのリソースに書いた文字列は既定値として扱い、起動時に上書きする形になる（推測）。
- **ホットキーの入力欄**: 標準の hot key control（`HOTKEY_CLASS`）がある。ただし `HKM_GETHOTKEY` の修飾フラグは `HOTKEYF_ALT` / `HOTKEYF_CONTROL` / `HOTKEYF_EXT` / `HOTKEYF_SHIFT` だけで、Win キーがない [6][7]。`RegisterHotKey` は `MOD_WIN` を受け付けるので、Win キーを含む割り当てを許すなら、EDIT をサブクラス化して `WM_KEYDOWN` / `WM_SYSKEYDOWN` から組み立てる入力欄を自作する（推測。数十〜百行規模の見込み）。
  - どの手段でも共通の注意: 入力中に、すでに `RegisterHotKey` で登録済みの組み合わせを押すと、`WM_HOTKEY` として横取りされて入力欄に届かない（推測）。入力欄にフォーカスがある間は一時的に登録を外す必要がある。
- **exe とメモリ**: 依存は増えない。最小構成で 408KB。開いている間も閉じた後も +0.8MB（計測）。
- **保守性**: 部品は OS が持つので、見た目と挙動は Windows 標準と揃う。レイアウトの計算（DPI 変更時の再配置）、タブ切り替えなどの画面構成、ダークモードは自前で書く。小さな設定画面なら規模は限定的と見る（推測）。Rust の薄いラッパーとして winsafe（702 stars、2026-10 に更新、MIT）もあるが、独自の FFI を持つので windows-rs の型と二重になる。native-windows-gui は最終コミットが 2023-02 で、止まっている [17]。

### Slint

- **ライセンス**: Royalty-free / GPLv3 / 商用の 3 択。Royalty-free はデスクトップ用途なら無償で、条件は「disclose that you use Slint (for example with the `AboutSlint` widget or the Slint badge)」。組み込み用途は対象外 [16]。shunsoku は MIT の OSS なので、GPLv3 ではなく Royalty-free を選び、設定画面などに Slint の表記を出すことになる（推測。ライセンスの解釈は採用時に確認）。
- **描画**: winit バックエンドの renderer は femtovg（OpenGL）、femtovg-wgpu、Skia、software から選べる。Skia が有効ならそれが優先され、だめなら femtovg、最後に software にフォールバックする [11a]。メモリの点では software 一択（上の計測）。設定画面は静的なフォームなので、CPU 描画の速さで困ることはないと見る（推測）。
- **ダークモード**: 既定の fluent スタイルはシステムの明暗に追従する。`fluent-dark` などで固定もできる [11]。
- **文言**: `@tr("...")` で印を付け、`slint-tr-extractor` で `.pot` を作る。実行時に gettext で `.mo` を読む方式（`slint::init_translations!`）と、バイナリに埋め込んで `slint::select_bundled_translation` で実行中に切り替える方式がある [12]。「文言は外部ファイル」の要件を満たすのは gettext 方式。gettext 方式での実行中の切り替え方法は文書から読み取れなかった（未確認）。Rust 側から文字列をプロパティとして流し込めば、形式は自由にできる（推測）。
- **IME**: Windows に関する過去の報告（Windows 11 で韓国語を入力できない #1706、Microsoft Pinyin で固まる #8716 など）は閉じている。未解決の報告として「IME の変換中にフォーカスを失うと最後の文字が落ちる」#10861 がある [13]。
- **ホットキーの入力欄**: `FocusScope` の `KeyEvent` に `alt` / `control` / `shift` / `meta` があり、`meta` は Windows では Windows キー [14]。PrintScreen は `SysReq` として届く対応表になっている [15]。部品は自作する。
- **イベントループ**: Slint の winit バックエンドは、同じスレッドでイベントループを回す。shunsoku の常駐側（自前の Win32 メッセージループ）と同じスレッドで動かしたときに、ホットキーやトレイのメッセージが問題なく配られるかは未確認。計測では `run()` を 2 回呼んでも動いた。
- **exe**: 12.1MB（既定 feature）。software renderer だけに絞れば減る見込み（推測。未計測）。

### egui（eframe）

- **メモリ**: 閉じた後に約 15〜16MB が残る（計測）。常駐の 15.8MB と単純に足すと 30MB 前後になり、一度設定を開いただけで目標に届かなくなる恐れがある（推測）。software 描画の公式なバックエンドはなく、eframe は glow か wgpu を使う [10]。
- **日本語**: 既定のフォントは Hack、Ubuntu-Light、絵文字だけで、CJK を出すには自分でフォントを入れる必要がある [10]。同梱すれば exe が数 MB 増える。システムのフォントを読めば、その分メモリが増える（計測で +20MB）。
- **IME**: egui のリポジトリには IME の報告が多い。2026 年にも、日本語 IME で確定するとフォーカスが外れる #7809、変換中の Backspace の誤動作 #7908、Enter での確定で改行が入る #7876 などが修正された。一方で、Windows 上の IME の報告（#3532、#2317）は Open のまま [9]。
- **ホットキーの入力欄**: `Modifiers` は `alt` / `ctrl` / `shift` / `mac_cmd` / `command` の 5 つで、`mac_cmd` は「Should always be set to `false` on other platforms」[10a]。egui-winit は winit の super キーを読むが、Windows では `mac_cmd` にも `command` にも使わない（`cfg!(target_os = "macos") && super_`）[10b]。`Key` に PrintScreen、Pause、ScrollLock がない [10c]。スクリーンショットツールで定番の PrintScreen を割り当てる入力欄を作るには、egui の外で生のキー入力を取る必要がある。
- **ダークモード / DPI**: `ThemePreference::System` が既定で、システムに追従する。DPI は `pixels_per_point = zoom_factor * native_pixels_per_point` で扱う [8]。
- **イベントループ**: eframe 0.37 の `NativeOptions::run_and_return` は既定で true で、閉じた後も処理が続く [8a]。winit の `EventLoop` は「only once per application」で、複数作れない [18]。eframe はこれを内部で使い回しているので、同じプロセスで開き直せた（計測）。

### 別プロセス（設定画面だけを別プロセスで開く）

- 同じ exe を `--settings` のような引数で起動し直すか、設定用の exe を別に用意する。プロセスが終われば、メモリは OS が回収する。GPU レンダラの初期化コストや、egui / Slint の残りメモリを常駐側に持ち込まない。
- 必要になるもの（推測）:
  - 設定の反映: 設定プロセスがファイルを保存し、常駐側に通知する（`WM_COPYDATA`、登録したウィンドウメッセージ、またはファイル変更の監視）。
  - 二重起動の制御（名前付きミューテックスなど）。
  - ホットキーの入力中は、常駐側のホットキーを一時的に外す連携。
- コールドスタートの 500ms は常駐側の起動の話なので、設定画面の起動時間は別の扱いになる。ただし、Win32 なら起動はほぼ一瞬（#5 で 51ms）。GPU レンダラの初期化は数百 ms かかる可能性がある（推測。未計測）。
- Win32 で作る場合、増分は 0.8MB しかないので、別プロセスにする利点は小さい。egui / Slint（GPU）を選ぶ場合の逃げ道として意味がある。

## 未確定点と、次に試すこと

- ダークモードの要求水準: 設定画面でコントロールの中身まで暗くするか、タイトルバーだけでよいか。Win32 案の手間は、これでほぼ決まる。
- Win32 案の試作:
  - `DIALOGEX` のリソースと `CreateDialog` で、DPI の違うモニタ間を移動したときの見え方（4K 150% と FHD 100%）。
  - 自作のホットキー入力欄（Win キーと PrintScreen を受けられるか。登録済みのホットキーとの干渉）。
- Slint を選ぶ場合:
  - software renderer での日本語の表示と IME の確認。
  - 常駐側の Win32 メッセージループと同じスレッドで共存できるか。
  - feature を絞ったときの exe サイズ。
  - 最新版（1.18）での再計測。
- 描画結果の目視確認（本調査ではできていない）。

## 出典

1. High DPI Desktop Application Development on Windows（PMv2 の機能一覧、フレームワーク別の対応表）— https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows
2. Support Dark and Light themes in Win32 apps — https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-windows-themes
3. ysc3839/win32-darkmode（「undocumented dark mode API introduced in Windows 10 1809」）— https://github.com/ysc3839/win32-darkmode
4. ozone10/win32-darkmodelib README（主要コントロール対応、IAT フック使用）— https://github.com/ozone10/win32-darkmodelib
5. About Input Method Manager — https://learn.microsoft.com/en-us/windows/win32/intl/about-input-method-manager
6. About Hot Key Controls — https://learn.microsoft.com/en-us/windows/win32/controls/hot-key-controls
7. HKM_GETHOTKEY message（修飾フラグ）— https://learn.microsoft.com/en-us/windows/win32/controls/hkm-gethotkey
8. egui `Options`（theme_preference、zoom_factor）— https://docs.rs/egui/0.33.3/egui/struct.Options.html
   - 8a. eframe 0.37.0 `NativeOptions::run_and_return` — https://docs.rs/eframe/latest/eframe/struct.NativeOptions.html
9. emilk/egui の IME に関する issue（2026-11-01 に検索）: #3532 / #2317（Open）、#7809 / #7876 / #7908（2026 に Closed）— https://github.com/emilk/egui/issues?q=IME
10. emilk/egui README（既定のフォント、eframe のバックエンド。31.9k stars、0.37.0 は 2026-10-15）— https://github.com/emilk/egui
    - 10a. egui 0.33.3 `Modifiers` — https://docs.rs/egui/0.33.3/egui/struct.Modifiers.html
    - 10b. egui-winit 0.33.3 `lib.rs`（ModifiersChanged の処理）— https://github.com/emilk/egui/blob/0.33.3/crates/egui-winit/src/lib.rs
    - 10c. egui 0.33.3 `Key` — https://docs.rs/egui/0.33.3/egui/enum.Key.html
11. Slint widget styles — https://docs.slint.dev/latest/docs/slint/reference/std-widgets/style/
    - 11a. Slint winit backend（renderer の種類と既定の選択）— https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/
12. Slint Translations — https://docs.slint.dev/latest/docs/slint/guide/development/translations/
13. slint-ui/slint の IME に関する issue（2026-11-01 に検索）: #10861（Open）、#1706 / #8716 / #1644（Closed）— https://github.com/slint-ui/slint/issues?q=IME
14. Slint builtin structs（KeyEvent / KeyboardModifiers）— https://docs.slint.dev/latest/docs/slint/reference/property-types/builtin-structs/
15. slint v1.18.1 `internal/common/key_codes.rs`（winit の PrintScreen → SysReq）— https://github.com/slint-ui/slint/blob/v1.18.1/internal/common/key_codes.rs
16. Slint LICENSE.md — https://github.com/slint-ui/slint/blob/master/LICENSE.md
17. GitHub リポジトリ情報（2026-11-01 に取得）: slint-ui/slint 24,340 stars・v1.18.1（2026-09-21）/ rodrigocfd/winsafe 702 stars・MIT / gabdube/native-windows-gui 2,063 stars・最終コミット 2023-02-14
18. winit 0.30.13 `EventLoopBuilder::build`（「only once per application」）— https://docs.rs/winit/0.30.13/winit/event_loop/struct.EventLoopBuilder.html
