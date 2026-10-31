# Windows のキャプチャ API の調査

- 対象 issue: [#4](https://github.com/wchoco/shunsoku/issues/4)（地図 [#2](https://github.com/wchoco/shunsoku/issues/2)）
- 調査日: 2026-11-01
- 前提（地図より）: 対象 OS は Windows 10/11 のみ。DPI が異なるモニタの混在は MVP 必須。ホットキーから範囲選択の画面表示まで 100ms 以下、常駐メモリ 30MB 以下。ウィンドウキャプチャは他のウィンドウに隠れた部分も撮れること。

表記: 出典のある事実には脚注番号 [n] を付ける。**（推測）** と書いたものは一次情報で確認できていない見立てで、実機での確認が要る。

## 結論

| キャプチャ方式 | 推奨 API | 予備（フォールバック） |
| --- | --- | --- |
| 範囲選択キャプチャ | **DXGI Desktop Duplication**（`IDXGIOutput5::DuplicateOutput1`）でホットキー押下時に全モニタを撮り、その静止画を範囲選択の画面に表示して切り抜く | GDI `BitBlt`（画面 DC から、`SRCCOPY \| CAPTUREBLT`） |
| ウィンドウキャプチャ | **Windows.Graphics.Capture**（`IGraphicsCaptureItemInterop::CreateForWindow`）で 1 フレームだけ取得 | `PrintWindow`（`PW_RENDERFULLCONTENT`、未文書化） |
| 定型領域キャプチャ | 範囲選択と同じ取得経路（Desktop Duplication で該当モニタを撮って切り抜き） | GDI `BitBlt` |

理由の要約:

- **隠れたウィンドウを撮れるのはウィンドウ単位の API（WGC の `CreateForWindow`、`PrintWindow`）だけ**。画面を撮る API（Desktop Duplication、画面 DC への `BitBlt`）は、見えているピクセルしか得られない。よってウィンドウキャプチャは WGC が本命になる。
- **WGC には黄色い枠線がある**。枠を消す `IsBorderRequired` は Windows 10 の一般向けビルド（19041〜19045）には無く [3]、Windows 10 では枠が必ず出る（推測ではなく、プロパティの導入ビルドが 20348 のため）。範囲選択の静止画づくりにモニタ単位の WGC を使うと、Windows 10 では押すたびにモニタ全体に枠が出る。これを避けるため、画面全体の取得には枠も同意も無い Desktop Duplication を使う。
- **HDR の色を失わずに撮れるのは、ピクセル形式を指定できる WGC と `DuplicateOutput1` だけ**と Microsoft が明記している [12]。GDI と旧 `DuplicateOutput` は 8bit BGRA に固定される [4][6]。
- **プロセスは Per-Monitor V2 DPI aware にする**ことが全 API 共通の前提。そうしないと座標が仮想化され、DPI 混在環境で位置とサイズがずれる [11]。

## API ごとの比較

### 一覧

| 観点 | GDI BitBlt（画面 DC） | GDI PrintWindow | DXGI Desktop Duplication | Windows.Graphics.Capture |
| --- | --- | --- | --- | --- |
| 撮れる単位 | 画面上の矩形 | 1 ウィンドウ | 1 モニタ（出力）単位 [5] | 1 ウィンドウ / 1 モニタ [7][1] |
| 最低 OS | Windows 2000 | Windows XP [8] | Windows 8 [5] | 1903 (18362)、Win32 からの HWND/HMONITOR 指定 [7] |
| 遅延 | 同期呼び出し。大きい領域ほど遅い（推測） | 同期で、対象アプリが描画を処理するので返りが読めない [8] | 次のフレーム更新を待つ方式。静止した画面では最初の 1 枚までの時間を実測する必要あり（推測） | セッション開始から最初の `FrameArrived` まで非同期。D3D デバイス生成のコストもある（推測） |
| 隠れたウィンドウ | 不可（見えているピクセルのみ） | 可。ただし DirectComposition を使うアプリは `PW_RENDERFULLCONTENT` が要る [9] | 不可 | 可（ウィンドウ単位で撮る） |
| 最小化されたウィンドウ | 不可 | 不可（推測） | 不可 | 実質不可。OS が最小化ウィンドウを描画しないため黒またはフレームが来ない [13]。OBS も `IsIconic` のときはフレームを捨てている [14] |
| DPI 混在 | PMv2 なら物理ピクセルの仮想スクリーン座標で扱える [11] | ウィンドウの物理サイズ（PMv2 前提、推測） | 出力ごとの `DesktopCoordinates` で配置。回転も自前で処理 [4] | 物理ピクセルのフレーム（推測）。`ContentSize` で切り出す [1] |
| HDR | 8bit に落ちる（推測。HDR 表示中の色は白飛びしうる） | 同左（推測） | `DuplicateOutput` は常に BGRA8 に変換 [6]。`DuplicateOutput1` で FP16 などを受け取れる [6][12] | フレームプールを `R16G16B16A16_FLOAT` にすれば保持できる [1][12] |
| カーソル | 含まれない。必要なら自分で描く（推測） | 含まれない（推測） | 画像に焼き込まれる場合と別に返る場合がある。位置と形状が別途取れる [4] | `IsCursorCaptureEnabled` で切り替え（19041〜）[2] |
| 黄色い枠 / 同意 | なし | なし | なし | 枠あり [1]。消すには 20348〜 で `RequestAccessAsync(Borderless)` + `IsBorderRequired = false` [3][10] |
| DRM 保護 | 黒くなる（推測） | 黒くなる（推測） | 黒く塗られ、`ProtectedContentMaskedOut` で分かる [15] | 黒くなる（推測） |
| `SetWindowDisplayAffinity` | `WDA_MONITOR` は中身なし、`WDA_EXCLUDEFROMCAPTURE` は映らない [16] | 同左 | 同左 | 同左 |
| UAC 昇格ウィンドウ | 表示されている画素は撮れる（推測） | 不明（要検証） | 通常デスクトップ上なら撮れる（推測） | 不明（要検証） |
| UAC のセキュアデスクトップ | 不可 | 不可 | 不可。`LOCAL_SYSTEM` でないとアクセス不可、デスクトップ切替で `ACCESS_LOST` [5][17] | 不可（推測） |

### GDI BitBlt / PrintWindow

- `BitBlt` の `CAPTUREBLT` は「自分のウィンドウの上に重なるレイヤードウィンドウも含める」フラグ [18]。画面 DC から撮るときはこれを付けないと半透明ウィンドウなどが欠ける。
- `BitBlt` は色管理をしない（"No color management is performed when blits occur"）[18]。
- `PrintWindow` は同期関数で、対象ウィンドウを持つアプリが `WM_PRINT` を処理して描く [8]。対象が応答しないと戻りが遅れる。UI スレッドから呼ぶべきではないと明記されている [8]。
- WebRTC と Chromium は、隠れたウィンドウを撮るために `PrintWindow` に未文書化の `PW_RENDERFULLCONTENT` を付けている。このフラグが無いと DirectComposition で描くアプリの取得に失敗する、と WebRTC のコミットに書かれている [9]。未文書化なので将来の互換性は保証されない。

### DXGI Desktop Duplication

- モニタ（出力）単位でデスクトップ画像を取得する。全画面を撮るには出力ごとにインターフェースを作る [5]。1 プロセスは 1 出力に 1 つまで、同時に使えるプロセスは既定で 4 つまで [5]。
- `DuplicateOutput` で得る画像は表示モードによらず常に `DXGI_FORMAT_B8G8R8A8_UNORM` [4]。`DuplicateOutput1` は受け取れる形式のリストを渡せて、R10G10B10A2 などの高色域をそのまま受け取れる [6]。HDR を損失なく撮れる API として Microsoft が名指ししている [12]。
- カーソル: GPU がカーソルをオーバーレイしている場合は画像に含まれず、`PointerPosition` と `GetFramePointerShape` で別に返る [4]。つまり「カーソルを除く」は自然にでき、「含める」は自分で合成する。ただし画像に焼き込まれて返る場合もあり、その場合は除けない [4]。
- 回転したモニタでは、返る画像は回転前の向き。自分で回す必要がある [4]。
- デスクトップ切替（UAC のセキュアデスクトップを含む）、モード変更、全画面アプリへの切替で `DXGI_ERROR_ACCESS_LOST` になり、作り直しが要る [17]。8bpp や DWM 無効の画面は `DXGI_ERROR_UNSUPPORTED` [5]。
- DRM 等の保護コンテンツは黒く塗られた状態で返り、`ProtectedContentMaskedOut` が TRUE になる [15]。
- D3D11 デバイスは出力がつながるアダプタから作る必要がある [5]。複数 GPU 環境ではアダプタごとにデバイスが要る（推測）。

### Windows.Graphics.Capture（WGC）

- 選んだウィンドウ / モニタを撮る API。システムがキャプチャ中の対象の周りに黄色い枠を描く [1]。
- Win32 アプリは `GraphicsCapturePicker`（ユーザーが選ぶシステム UI）を使わず、`IGraphicsCaptureItemInterop::CreateForWindow` / `CreateForMonitor` で HWND / HMONITOR から直接作れる（1903〜）[7][19]。この経路では同意ダイアログは出ない（OBS が同じ経路で動いている [14]）。
- 枠の制御: `IsBorderRequired` は 20348（Windows Server 2022。クライアントでは Windows 11）で導入 [3]。枠を消すには `GraphicsCaptureAccess.RequestAccessAsync(GraphicsCaptureAccessKind.Borderless)` でユーザーの同意を得る必要があり、プロンプトが出る。また **パッケージマニフェストに `graphicsCaptureWithoutBorder` 機能の宣言が要る** [3][10]。拒否されると `false` を設定しても無視される [3]。他アプリが同じ対象に `true` を設定していると枠は出る [3]。
  - **非パッケージ（素の exe）アプリでの挙動は Microsoft の文書に記載がない**。OBS は非パッケージのまま `RequestAccessAsync(Borderless)` を呼んで `IsBorderRequired(false)` を設定している [14]。Windows 11 でプロンプトが出るか、設定が効くかは、OSS 実装の間でも報告が割れている [20]。**要検証**。
  - Windows 10（19041〜19045）にはプロパティ自体が無いので、枠は消せない [3]。
- カーソル: `IsCursorCaptureEnabled`（19041〜）で含める / 除くを選べる [2]。
- HDR: HD color 有効時は内容のピクセル形式が BGRA8 とは限らず、白っぽく見える（過剰クリップ）のを避けるにはパイプライン全体を `R16G16B16A16_FLOAT` にする。保存時には HDR→SDR のトーンマップが要る [1]。OBS はモニタが HDR のときだけ FP16 を選んでいる [14]。
- フレームのサーフェスは作成時のサイズで、内容がそれより小さいと余りは未定義。`ContentSize` で切り出す [1]。
- `GraphicsCaptureSession.IsSupported()` で利用可否を確認できる [1]。
- 最小化されたウィンドウはフレームが更新されない / 黒になる。OS が最小化ウィンドウを描画しないため、と ScreenRecorderLib のメンテナが説明している [13]（一次情報ではない）。
- Alt+Tab に出ないウィンドウでは `CreateForWindow` が `E_INVALIDARG` を返すという報告がある（非公式、未確認）。

### DPI が混在する環境

- 既定ではデスクトップアプリは DPI 非対応とみなされ、ビットマップ拡大される。DPI 非対応または System DPI aware のスレッドから呼んだ API は、戻り値が仮想化されうる。どの API が仮想化するかは十分に文書化されていない、と Microsoft 自身が書いている [11]。
- Per-Monitor V2（1703〜）で登録すると、アプリは各ディスプレイの生のピクセルを見る（"The application seeing the raw pixels of each display"）[11]。
- したがって shunsoku はマニフェストで PMv2 を宣言し、仮想スクリーン座標（物理ピクセル）を一貫して使う。範囲選択の画面のウィンドウも PMv2 で、モニタごとに静止画を 1:1 で表示する。

### SetWindowDisplayAffinity

- `WDA_MONITOR` のウィンドウはモニタ以外では中身なしで現れ、`WDA_EXCLUDEFROMCAPTURE`（2004〜）のウィンドウはキャプチャにまったく現れない [16]。これは DWM が合成しているときだけ働く [16]。
- 逆に shunsoku 自身の範囲選択の画面やスクラップをキャプチャから外したいときに使える（スクラップを撮れるべきかは仕様判断）。

## 範囲選択の画面に表示する静止画の用意

推奨手順:

1. ホットキー押下時に、全モニタを Desktop Duplication で撮る。モニタごとに `DuplicateOutput1` を作り、`AcquireNextFrame` で 1 枚取る。HDR のモニタは FP16、SDR のモニタは BGRA8 を受け取る（[6][12] の指定方法に従う）。
2. 出力ごとの `DesktopCoordinates` と回転を使って、仮想スクリーン座標上に並べる [4]。
3. モニタごとにボーダーレスの最前面ウィンドウ（PMv2）を出し、その静止画を等倍で表示する。ユーザーはこの上で範囲を選ぶので、選択中に画面が変わっても結果は押した瞬間の画像になる。
4. 確定した矩形を静止画から切り抜く。範囲選択キャプチャの結果はこの切り抜きそのもの。
5. 「範囲選択の画面でウィンドウをハイライトしてクリック」でウィンドウキャプチャに切り替えた場合は、静止画ではなく WGC の `CreateForWindow` でそのウィンドウを撮る（隠れた部分を含めるため）。WGC はウィンドウ単位なので、上にかぶさる範囲選択の画面は写らない（推測）。

GDI `BitBlt` を予備に置く理由: Desktop Duplication はセッションが切断中・非 DWM・8bpp などで使えない [5]。リモートデスクトップや仮想 GPU 環境で失敗した場合に `BitBlt(CAPTUREBLT)` で仮想スクリーンを撮る（推測。実環境での失敗条件は要確認）。

遅延について: 100ms の目標に対し、Desktop Duplication を毎回作るか常駐中に保持するかで、初回フレームまでの時間とメモリが変わる（推測）。一次情報には数値が無いため、実測が必要。

## 方式ごとの推奨（詳細）

### 範囲選択キャプチャ

- **Desktop Duplication（`DuplicateOutput1`）** で静止画を取得し、切り抜く。
- 枠も同意も出ない。HDR を保持できる。カーソルは別に返るので既定で除ける [4][6]。
- 予備: GDI `BitBlt`。HDR 表示中は色が落ちる（推測）。

### ウィンドウキャプチャ

- **WGC（`CreateForWindow`）** で 1 フレーム取得し、すぐセッションを閉じる。
- `IsCursorCaptureEnabled = false`（19041〜）[2]。HDR モニタ上では FP16 で受ける [1]。
- 枠: Windows 11 では `RequestAccessAsync(Borderless)` と `IsBorderRequired = false` を試す [3][10]。Windows 10 では枠が一瞬出るのを受け入れるか、`PrintWindow(PW_RENDERFULLCONTENT)` に切り替える [9]。
- 最小化されたウィンドウは撮れない [13][14]。復元してから撮るか、エラーとして扱うかは仕様判断。
- 予備: `PrintWindow(PW_RENDERFULLCONTENT)`。未文書化フラグで、対象アプリが応答しないと戻らない [8][9]。

### 定型領域キャプチャ

- 範囲選択と同じく **Desktop Duplication** で、矩形が重なるモニタだけ撮って切り抜く。取得経路を共通にできる。
- 予備: GDI `BitBlt`。領域が小さければ十分速い（推測）。
- 定型領域は「見えているもの」を撮る方式なので、DRM 保護や `WDA_*` の影響は範囲選択と同じ。

## 未解決の問い（要検証）

- 非パッケージの exe で、Windows 11 上の `RequestAccessAsync(Borderless)` がプロンプトを出すか、マニフェスト無しで `IsBorderRequired = false` が効くか。配布形態（MSIX にするか）に影響する。
- UAC 昇格したウィンドウを、非昇格の shunsoku から WGC / `PrintWindow` で撮れるか。
- Desktop Duplication / WGC / GDI の、ホットキーから静止画表示までの実測遅延と常駐メモリ（D3D デバイスを保持した場合）。
- HDR で撮った画像を PNG に保存するときのトーンマップ方針。

## 出典

1. Microsoft Learn, "Screen capture" — https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture
2. Microsoft Learn, `GraphicsCaptureSession.IsCursorCaptureEnabled` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled
3. Microsoft Learn, `GraphicsCaptureSession.IsBorderRequired` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired
4. Microsoft Learn, "Desktop Duplication API" — https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api
5. Microsoft Learn, `IDXGIOutput1::DuplicateOutput` — https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput
6. Microsoft Learn, `IDXGIOutput5::DuplicateOutput1` — https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_5/nf-dxgi1_5-idxgioutput5-duplicateoutput1
7. Microsoft Learn, `IGraphicsCaptureItemInterop::CreateForWindow` — https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow
8. Microsoft Learn, `PrintWindow` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-printwindow
9. WebRTC commit f89110d "Use PW_RENDERFULLCONTENT to capture occluded windows on Win8.1 and Win10" — https://webrtc.googlesource.com/src/+/f89110d67902e787f6745ad2b52f7f09fc808512%5E%21 ／ Chromium `ui/snapshot/snapshot_win.cc` — https://sources.debian.org/src/chromium/139.0.7258.127-1/ui/snapshot/snapshot_win.cc
10. Microsoft Learn, `GraphicsCaptureAccess.RequestAccessAsync` / `GraphicsCaptureAccessKind` — https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccess.requestaccessasync ／ https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscaptureaccesskind
11. Microsoft Learn, "High DPI Desktop Application Development on Windows" — https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows
12. Microsoft Learn, "Use DirectX with Advanced Color on high/standard dynamic range displays"（"Capturing HDR and WCG screen content" 節）— https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range
13. ScreenRecorderLib issue #303（最小化ウィンドウが黒くなる件、メンテナの説明）— https://github.com/sskodje/ScreenRecorderLib/issues/303
14. OBS Studio `libobs-winrt/winrt-capture.cpp` — https://github.com/obsproject/obs-studio/blob/master/libobs-winrt/winrt-capture.cpp
15. Microsoft Learn, `DXGI_OUTDUPL_FRAME_INFO` — https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/ns-dxgi1_2-dxgi_outdupl_frame_info
16. Microsoft Learn, `SetWindowDisplayAffinity` — https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity
17. Microsoft Learn, `IDXGIOutputDuplication::AcquireNextFrame` — https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe
18. Microsoft Learn, `BitBlt` — https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-bitblt
19. robmikh/Win32CaptureSample（Microsoft のエンジニアによる Win32 からの WGC サンプル）— https://github.com/robmikh/Win32CaptureSample
20. 非パッケージアプリでの枠なしキャプチャの報告例: webrtc-sdk/libwebrtc PR #155 — https://github.com/webrtc-sdk/libwebrtc/pull/155 ／ NinjaGoldfinch/ninja-recorder issue #219 — https://github.com/NinjaGoldfinch/ninja-recorder/issues/219
