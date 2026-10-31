# 技術スタック候補の比較調査

issue: [#3](https://github.com/wchoco/shunsoku/issues/3)（地図 [#2](https://github.com/wchoco/shunsoku/issues/2)）
調査日: 2026-11-01

## 結論（先に要約）

計測プロトタイプで試す候補を次の 2 つに絞る。

1. **Rust + windows-rs で Win32 を直接呼ぶ実装**（第一候補）
2. **C# + WinUI 3（Windows App SDK）+ Native AOT**（対抗候補）

除外する候補と理由:

- **WinForms / WPF**: どちらも .NET SDK でトリミングが無効化されており、Native AOT が使えない。JIT のままでは起動時間の目標（コールドスタート 500ms 以下）に届かない計測例がある。
- **Tauri**: WebView2（Chromium 系のマルチプロセス）を背負う。公式ベンチマークでも Windows の hello world は DOMContentLoaded まで約 0.58〜0.62 秒かかり、コールドスタートの目標を超える。メモリも 30MB 以下に収まる見込みが低い（推測。後述）。
- **winit 系**: 独立した候補ではなく、Rust 実装の中で「ウィンドウ管理を winit に任せるか、Win32 を直接呼ぶか」の選択肢として扱う（後述）。

地図 Notes の目標値（再掲）: 常駐時のメモリ 30MB 以下 / ホットキーから範囲選択の画面表示まで 100ms 以下 / コールドスタート 500ms 以下。

## 比較表

凡例: 数値は出典つき。「推測」と書いたものは一次情報で裏づけていない見込み。

| 候補 | 常駐メモリ | 起動時間 | Per-Monitor V2 | WinRT（Windows.Graphics.Capture）呼び出し | 配布サイズ | 保守性 |
| --- | --- | --- | --- | --- | --- | --- |
| WinForms (.NET) | 公開計測なし | JIT のみ（AOT 不可 [3]） | 対応（既定は SystemAware、指定が必要 [5]） | CsWinRT 経由で可 | 自己完結だと大きい（トリミング不可 [3]） | 成熟。ただし AOT 対応の予定なし [4] |
| WPF (.NET) | 非公式 AOT フォークで 56.1MB、通常版で 82.4MB [8] | 通常 652ms、非公式 AOT で 136ms [8]／.NET 8 で 0.50〜0.68 秒 [7] | 対応（フレームワークが処理 [6]） | CsWinRT 経由で可 | AOT 不可。非公式フォークの AOT exe で約 55MB [8] | 成熟。トリミング対応の issue は未解決 [3] |
| WinUI 3 + Native AOT | 公開計測なし（要計測） | 空ウィンドウで 0.09〜0.18 秒 [7] | フレームワークが処理（推測。要計測） | 最も自然（C#/WinRT 標準） | 自己完結で Windows App SDK 同梱が必要。数値は要計測 | Windows App SDK 2.x が現行。サポート期間あり [10] |
| Rust + windows-rs（Win32 直叩き） | 公開計測なし。数 MB 台の見込み（推測） | 公開計測なし。ネイティブ C++ 同等の見込み（推測） | 自前で実装（`WM_DPICHANGED` を処理）[6] | `windows` クレートが WinRT を型付きで投影 [12] | 単一 exe。数 MB 以下の見込み（推測） | Microsoft 公式。12.9k stars。リリース 74（2026-09）[12] |
| Rust + winit | 同上（推測） | 同上（推測） | winit が PMv2 を自動設定 [13] | windows-rs と併用 | 同上 | 6.2k stars。活発 [14] |
| Tauri | WebView2 のマルチプロセス [15]。実アプリで Electron と同程度の報告 [16] | hello world で DOMContentLoaded まで約 0.58〜0.62 秒（Windows, CI）[17] | WebView2 側が処理 | Rust 側で windows-rs を使えば可 | hello world の exe 約 2.8MB（WebView2 ランタイムは別）[17] | 112k stars。最も活発 [14] |

## 候補ごとの詳細

### .NET 共通: Native AOT の制約

- Native AOT は「起動が速く、メモリ使用量が小さい」と公式に説明されている。制約は、動的ロード不可、`Reflection.Emit` 不可、**組み込みの COM 不可（Windows）**、トリミングが前提、の 4 点 [1]。
- 組み込みの COM マーシャリングはトリミングと両立しない。代わりに COM Wrappers を使う [2]。
- そのため C# で Native AOT を使うなら、COM と WinRT の呼び出しをソースジェネレーター方式（CsWinRT 2.1.1 以降、CsWin32 の `GeneratedComInterface` / `LibraryImport` 出力）にそろえる必要がある [9][11]。

### WinForms

- **AOT 不可**: 「WinForms は組み込みの COM マーシャリングに強く依存し、それなしで動くアプリはほぼないため、.NET SDK でトリミングを無効化している」[2]。対応 epic（dotnet/winforms#4649）は 2026-11 時点でも Open。2026-09 のコメントでは「.NET 11 のプレリリースでも AOT の進展はない」とされている [4]。
- **DPI**: .NET 6 で PerMonitorV2 の挙動が改善された。ただし `ApplicationHighDpiMode` の既定値は `SystemAware` なので、明示的な指定が必要 [5]。Win32 の公式ドキュメントでは、WinForms の per-monitor 対応は「一部コントロールの限定的な自動スケーリング」とされている [6]。
- 評価: JIT 起動が前提になり、起動時間の目標に対して不利。.NET 8 の WPF では 0.50〜0.68 秒という計測がある [7]。WinForms は未計測だが、同じく JIT なので傾向は近いと推測する。

### WPF

- **AOT 不可**: 「ほぼすべての WPF アプリがトリミング後に動かないため、.NET SDK でトリミングを無効化している」（dotnet/wpf#3811、Open）[2]。
- 非公式の PoC フォーク（lezzi/wpfaot、.NET 10 Preview、i7-14700K、Windows 10）の計測 [8]:
  - framework-dependent: 起動 652ms（中央値）/ ワーキングセット 82.4MB
  - self-contained: 728ms / 82.8MB
  - AOT + トリミング: 136ms / 56.1MB / exe 約 55MB（IME/TSF、ドラッグ&ドロップなどを無効化した状態）
- 評価: AOT にしても 30MB の目標の約 2 倍で、しかも非公式フォーク。除外する。

### WinUI 3（Windows App SDK）+ Native AOT

- Windows App SDK 1.6 で `PublishAot` が正式に対応された。CsWinRT 2.1.1 以降が必要で、`{Binding}` などリフレクション前提の型は開発者が手動でルート指定する必要がある、と明記されている [9]。
- 現行の Stable は Windows App SDK 2.x（2.6.4、2026-10-27）。2.0 のサービス終了は 2027-04-29 [10]。
- 起動時間: 空ウィンドウの計測（Windows 11 24H2 の VM、2 コア、2025-05）で、プロセス起動から画面表示まで 0.09s / 0.18s。同じ条件で .NET 8（WPF）は 0.68s / 0.50s、.NET Framework 4.8.1 は 0.32s / 0.31s、ネイティブ C++ は 0s / 0s [7]。著者自身が「数値は正確ではない」と注記している。
- **メモリの公開計測は見つからなかった**。XAML フレームワーク、DirectX ベースの描画、Windows App SDK の DLL 群を読み込むので、30MB に収まるかがこの候補の最大の不確実点（推測）。
- 枠なし・半透明・最前面のウィンドウ: 最前面と枠なしは `AppWindow` / `OverlappedPresenter` で可能。ただし**ウィンドウ全体を透明にする正式 API は限られている**。コミュニティ製の WinUIEx の `TransparentTintBackdrop`、または Win32 の P/Invoke で対応する例が多い [18]（要プロトタイプ確認）。
- WGC の呼び出し: C#/WinRT の標準の書き方で呼べる。Microsoft の公式サンプルも WinUI 3 前提で書かれている [19]。

### Rust + windows-rs（Win32 直叩き）

- `windows` クレートは「C 形式・COM・WinRT の API の型付きバインディング」で、feature flag によって広い範囲の API を使える [12]。Microsoft 公式。2026-11 時点で 12.9k stars、最新リリースは 74（2026-09-03）[12]。
- WGC: Win32 アプリは `IGraphicsCaptureItemInterop::CreateForWindow` / `CreateForMonitor` で HWND や HMONITOR からキャプチャ対象を作れる（Windows 10 1903 以降）[20]。windows-rs ならこの COM インターフェースと WinRT 型を同じクレートで扱える。WGC と DXGI Desktop Duplication を包んだ Rust クレート `windows-capture`（524 stars、2026-10 に更新）もある [21]。
- DPI: Per-Monitor V2 はマニフェストか `SetProcessDpiAwarenessContext` で宣言する。`WM_DPICHANGED` に応じた再レイアウトはアプリの責任になる [6]。shunsoku の UI はスクラップ、範囲選択の画面、設定 GUI と小さいので、自前で処理する負担は限定的と見る（推測）。
- トレイとグローバルホットキー: Win32 の `Shell_NotifyIcon` / `RegisterHotKey` を直接使うか、tauri-apps の単体クレート `tray-icon`（414 stars）と `global-hotkey`（266 stars）を使う。どちらも Tauri 本体なしで使え、2026-10 に更新されている [14]。
- 描画: スクラップ上の書き込みは Direct2D、半透明・最前面のウィンドウは `WS_EX_LAYERED` / `WS_EX_TOPMOST`、または DirectComposition で実装する。windows-rs には direct2d と dcomp のサンプルがある [22]。
- 弱点: **設定 GUI**（フォーム、リスト、ホットキー入力欄）を自前で組む手間が大きい。comctl32 のコモンコントロールで作るか、軽い即時モード GUI（egui など）を設定画面だけに使うかは、プロトタイプで判断する（推測。egui は未調査）。
- メモリと起動時間: 公開計測は見つからなかった。ネイティブ C++ と同じ部類で、起動はほぼ 0 秒になると見込む（[7] のネイティブ C++ の結果からの類推。推測）。

### Rust + winit

- winit 0.30 は Windows で `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` を自動で呼ぶ（v0.30.13 の `platform_impl/windows/dpi.rs`）[13]。スケール係数の変化はイベントとして届く。
- ただし winit はクロスプラットフォームのウィンドウ抽象で、shunsoku は Windows 専用。レイヤードウィンドウや拡張スタイル、`WM_HOTKEY` などのメッセージを細かく制御するには、結局 HWND を取り出して Win32 を呼ぶことになる（推測）。
- 位置づけ: 独立した候補にはしない。windows-rs 案のプロトタイプで「ウィンドウとイベントループを winit に任せると楽になるか」を比べる副次的な問いとして扱う。

### Tauri

- WebView2 は Edge と同じプロセスモデルをとる。ブラウザプロセス 1 つ、レンダラープロセス 1 つ以上、GPU などのヘルパープロセスで構成される [15]。
- 公式ベンチマーク（tauri-apps/benchmark_results、Windows、GitHub Actions、2026-10-21）[17]:
  - hello world の DOMContentLoaded まで: 平均 0.579〜0.618 秒
  - hello world のバイナリ: 2,826,240 バイト（約 2.8MB）
  - Windows のメモリ（`max_memory`）は記録されていない
- メモリ: tauri-apps/tauri#5889 で、Windows 10 で同じ Web アプリを読み込んだとき Tauri 399MB / Electron 318MB という報告がある（共有メモリを考慮した計測）[16]。hello world の値ではないが、WebView2 を使う以上、Chromium 系のベースのコストがかかる。常駐 30MB は WebView2 を閉じたままトレイ常駐する設計でなければ難しい（推測）。そう設計すると、ホットキーのたびに WebView2 を起こす遅延が 100ms の目標とぶつかる（推測）。
- スクラップ上の描画、範囲選択の画面で全画面ビットマップを扱う処理では、Rust 側と WebView 側の間で大きな画像を受け渡すコストもかかる（推測）。
- 評価: 起動時間の実測値が目標を超えており、メモリでも不利。除外する。

## 計測プロトタイプで確かめること（#5 への引き継ぎ）

両候補で同じ最小構成を作り、地図の目標値と比べる。

- トレイ常駐 + `RegisterHotKey` + ホットキーで全画面の半透明・最前面ウィンドウ（範囲選択の画面の代わり）を表示 + WGC で 1 枚撮る + 枠なし・最前面のスクラップを 1 枚表示
- 計測項目: 常駐時のワーキングセットとプライベートバイト（アイドル状態、スクラップ 0 枚と 1 枚）、ホットキーから表示までの時間、コールドスタートの時間、配布物のサイズ
- DPI の異なる 2 枚のモニタ間でスクラップを移動したときの挙動
- WinUI 3 のみ: ウィンドウ全体の透明化の可否と手間、自己完結（unpackaged）で配布したときのサイズ
- windows-rs のみ: 設定 GUI をどう作るか（comctl32 か、egui などか）の当たりをつける

## 出典

1. Native AOT deployment overview — https://learn.microsoft.com/en-us/dotnet/core/deploying/native-aot/
2. Known trimming incompatibilities（WPF / Windows Forms / Built-in COM）— https://learn.microsoft.com/en-us/dotnet/core/deploying/trimming/incompatibilities
3. 同上（WinForms と WPF は SDK でトリミング無効）。WPF: https://github.com/dotnet/wpf/issues/3811（Open）
4. Epic - Make WinForms trim compatible — https://github.com/dotnet/winforms/issues/4649（Open、2026-09-14 のコメント）
5. What's new in Windows Forms .NET 6（ApplicationHighDpiMode の既定値 SystemAware、PerMonitorV2 の改善）— https://learn.microsoft.com/en-us/dotnet/desktop/winforms/whats-new/net60
6. High DPI Desktop Application Development on Windows — https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows
7. suusanex「アプリの起動速度を試すとWPFよりもWinUI 3+NativeAOTの方がだいぶ速いようです」(2025-05-23) — https://zenn.dev/suusanex/articles/f642d91df412c9
8. lezzi/wpfaot（非公式の WPF Native AOT PoC フォーク）README — https://github.com/lezzi/wpfaot
9. Windows App SDK 1.6 release notes（Native AOT support）— https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-1-6 ／ CsWinRT AOT/trimming — https://github.com/microsoft/CsWinRT/blob/master/docs/aot-trimming.md
10. Windows App SDK release channels（2.6.4 Stable、ライフサイクル表）— https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-channels
11. CsWin32 getting started「Support for trimming, AOT, and/or disabling the runtime marshaler」— https://github.com/microsoft/CsWin32/blob/main/docfx/docs/getting-started.md
12. microsoft/windows-rs — https://github.com/microsoft/windows-rs
13. winit v0.30.13 `src/platform_impl/windows/dpi.rs` — https://github.com/rust-windowing/winit/blob/v0.30.13/src/platform_impl/windows/dpi.rs
14. GitHub リポジトリ情報（2026-11-01 に取得）: rust-windowing/winit 6,242 stars / tauri-apps/tauri 112,369 stars / tauri-apps/tray-icon 414 stars / tauri-apps/global-hotkey 266 stars
15. Process model for WebView2 apps — https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model
16. tauri-apps/tauri#5889「Memory benchmark might be incorrect」— https://github.com/tauri-apps/tauri/issues/5889
17. tauri-apps/benchmark_results `tauri-recent-windows.json` — https://github.com/tauri-apps/benchmark_results
18. WinUIEx TransparentTintBackdrop — https://dotmorten.github.io/WinUIEx/api/WinUIEx.TransparentTintBackdrop.html ／ microsoft-ui-xaml discussion #9157 — https://github.com/microsoft/microsoft-ui-xaml/discussions/9157
19. Screen capture（Windows.Graphics.Capture）— https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture
20. IGraphicsCaptureItemInterop — https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nn-windows-graphics-capture-interop-igraphicscaptureiteminterop
21. NiiightmareXD/windows-capture — https://github.com/NiiightmareXD/windows-capture
22. windows-rs samples（direct2d, dcomp ほか）— https://github.com/microsoft/windows-rs/tree/master/crates/samples/windows
