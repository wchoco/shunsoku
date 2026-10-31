# 技術スタックは Rust + windows-rs で Win32 を直接呼ぶ

shunsoku は Rust で書き、`windows` クレート（windows-rs）から Win32 と WinRT を直接呼ぶ。ウィンドウ管理を winit などの汎用フレームワークに任せない。常駐時のメモリ 30MB 以下・コールドスタート 500ms 以下という軽さの目標と、最前面・枠なし・半透明のスクラップ、範囲選択の画面の出し方を細かく制御する必要から選んだ。

## Considered Options

- **Rust + winit**: 計測では、範囲選択の画面を出すまでが Win32 直接より約 25ms 遅く、exe は 2 倍（569KB / 285KB）。ホットキーから出したウィンドウにフォーカスが移らず回避策が要った。スクラップの半透明表示（layered window と `UpdateLayeredWindow`）は winit では扱えず、結局 Win32 を直接呼ぶことになる。
- **C# + WinUI 3 + Native AOT**: 常駐メモリが 30MB に収まるかが最大の不確実点で、ウィンドウ全体の透明化の正式 API も乏しい。計測せずに見送った。
- **WinForms / WPF**: Native AOT が使えない（トリミング非対応）。
- **Tauri**: 起動とメモリの目標を超える（WebView2）。
- **C++ / Electron 系**: 地図作成の時点で除外。

## Consequences

- Per-Monitor V2 の DPI 処理、タスクトレイ、グローバルホットキーは自前で書く。計測プロトタイプでは、DPI の異なるモニタが混在する環境（4K 150% が 2 枚と FHD 100% が 1 枚）で、固定画像と範囲選択の位置がずれないことを確かめた。
- 設定 GUI の作り方（Win32 のコモンコントロールか、別のライブラリか）は別に決める。
- 計測値（Win32 直接、Desktop Duplication、撮影後に `IDXGIDevice3::Trim`）: 常駐時の Private Working Set 15.8MB、起動 51ms、exe 285KB（CRT 静的リンク）。ホットキーから範囲選択の画面までは上記 3 枚構成で中央値 137ms で、100ms の目標には撮影と表示の方式の見直しが要る（技術スタックの問題ではない）。
