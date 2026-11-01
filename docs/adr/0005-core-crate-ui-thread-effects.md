# Win32 に触れない core crate、状態を持つのは UI スレッドだけ、スクラップ操作は静的な表と Effect

cargo workspace を `shunsoku-core`（lib）、`shunsoku`（常駐本体）、`shunsoku-cli` の 3 crate に分け、`shunsoku-core` は `windows` クレートに依存させない。アプリの状態（スクラップの一覧、ごみ箱、現在の設定）を持つのは UI スレッド 1 本だけにする。スクラップ操作は core の静的な表で表し、Win32 が要る仕事は実行せず `Effect` として返す。[MVP の範囲とフェーズ分け](https://github.com/wchoco/shunsoku/issues/9) で「Win32 に触れないロジックを TDD の境目にする」と決めたので、その境目を規約ではなくコンパイラに守らせ、操作の追加とテストをしやすくするために選んだ。

決定の詳細は [アーキテクチャとモジュール分割](https://github.com/wchoco/shunsoku/issues/27) の解決コメントにある。

## Considered Options

- **1 crate の中でモジュールを分け、規約で境目を守る**: Win32 を呼ぶ処理がロジックへ少しずつ紛れ込んでも気づけず、テストが Windows API に依存し始める。
- **プロトコル用の crate を別に切る**: core は純 Rust なので、CLI が core に依存しても exe はほとんど太らない。分ける利点がない。
- **スレッドごとに状態を分け、ロックで共有する**: Win32 のウィンドウは作ったスレッドに縛られる。UI の状態を複数のスレッドから触れるようにすると、ロックの順序と再入の問題を抱える。裏のスレッドは結果を `std::sync::mpsc` に入れ、`PostMessage` で UI スレッドを起こすだけにした。tokio などの async ランタイムは、メモリとサイズの目標のために入れない。
- **スクラップ操作を trait object（`Box<dyn ScrapOperation>`）にする**: MVP の操作は 15 程度に決まっており、外から操作を足す口もないので、静的な表のほうが一覧性が高い。操作の中で Win32 を直接呼ぶと、テストできなくなる。

## Consequences

- core の `cargo test` は Windows API なしで走る。core が Win32 の情報（モニタやウィンドウの一覧）を必要とするときは、Win32 層が集めた値を引数で受け取る。
- スクラップ操作の `run` は `fn(&mut ScrapModel, &OpContext) -> Vec<Effect>` の形にする。クリップボード、保存、ごみ箱への移動、トリムや編集モードへの切り替え、再描画などは `Effect` として返し、Win32 層の `app` が実行する。右クリックメニュー、ショートカットの対応表、設定 GUI の「スクラップ操作」タブは、この表から作る。
- 描画（Direct2D/DirectWrite）、DXGI・WGC による取得、ウィンドウ、トレイ、ホットキー、名前付きパイプは Win32 層に置き、テストの境目の外になる。
- Win32 層のモジュール同士は直接呼び合わず、`app` を通す。
