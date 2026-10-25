# ExMonitorViewer

Windows 用の拡張モニター表示アプリです。選択したディスプレイの映像を GPU で取得し、別ウィンドウに表示します。デフォルトの表示上限は 60 FPS です。

## 起動

`dist\exmonitor-viewer.exe` を起動します。Rust と Visual Studio Build Tools がある環境では `build.cmd` でリリース版をビルドできます。表示ウィンドウは通常、プライマリディスプレイ上に開き、キャプチャ対象は 2 台目のディスプレイを優先します。

ウィンドウのメニューからキャプチャする画面、FPS 上限（30、60、無制限）、最前面表示を選べます。`Fit window to image` または `F` キーで、現在の表示枠に合わせてウィンドウを映像と同じ縦横比にします。`Scale` メニューでは映像の原寸を 100% として 25%～200% のサイズを選べます。大きすぎる倍率は作業領域内に収まる倍率へ調整し、実際の倍率をタイトルバーに表示します。通常のウィンドウリサイズは従来どおり自由に行えます。`1`～`9` キーで画面を切り替え、`F11` キーでプライマリディスプレイの全画面表示にします。

通常起動では、ウィンドウ位置・サイズ、選択画面、FPS、最前面表示を正常終了時に `%LOCALAPPDATA%\ExMonitorViewer\settings.json` へ保存し、次回起動時に復元します。最小化中やF11全画面表示中の一時的なサイズは保存せず、通常サイズと最大化状態を記録します。画面が見つからない場合は既定の画面を選び、保存位置が表示領域外なら操作できる範囲へ移動します。`--monitor`、`--fps`、`--size` は保存値より優先されます。`--settings-file PATH` で保存先を指定できます。`--benchmark`、`--list-monitors`、`--help` は設定ファイルを読み書きしません。ウィンドウの位置とサイズは物理ピクセルで保存するため、DPIが異なる画面へ移した場合、論理サイズは一致しないことがあります。

## 計測

`--list-monitors` は Windows の表示名と解像度を表示します。`--monitor` の番号は一覧と同じ 1 始まりです。`--size` はウィンドウのクライアント領域の大きさを指定します。`--fps 30`、`--fps 60`、`--fps 0` で初期表示上限を選べます（0 は無制限）。`--benchmark` を指定すると指定した秒数だけ表示して終了し、`--report` で ASCII の計測結果を書き出します。

```powershell
.\target\release\exmonitor-viewer.exe --list-monitors
.\target\release\exmonitor-viewer.exe --monitor 2 --size 1920x1080 --benchmark 10 --report benchmark.txt
.\target\release\exmonitor-viewer.exe --monitor 2 --fps 0 --size 1920x1080 --benchmark 10 --report benchmark-unlimited.txt
.\target\release\exmonitor-viewer.exe --monitor 2 --size 1920x1080 --benchmark 20 --benchmark-cursor --report benchmark-cursor.txt
```

`--benchmark-cursor` は `--benchmark` と併用する診断用です。固定の半透明矢印カーソル形状を開始時に一度だけ GPU へアップロードし、描画中に画面上を動かします。OS のマウス位置は動かさず、実カーソルの形状や位置更新も合成には使いません。レポートでは `cursor_source=synthetic_benchmark`、`fixture_shape_uploads`、`fixture_position_updates`、`fixture_visible_present_submits` を確認できます。通常実行では `cursor_source=dxgi` となり、Desktop Duplication API から取得した実カーソルを表示します。実カーソルの見え方は通常実行で確認し、合成負荷の持続計測はこのfixtureで行ってください。

配布用 ZIP は、ビルド後に計測レポートを指定して作成できます。現在のバージョンで性能を再測定したレポートには `-PerformanceRetested` を指定してください。

```powershell
.\tools\package-release.ps1 -BenchmarkPath .\benchmark-report.txt -PerformanceRetested
```

タイトルバーには `Capture` と `Present submit` の FPS が表示されます。`Capture` は Desktop Duplication API が新しい画面画像を取得した回数です。`Present submit` はウィンドウへ提出できた回数です。提出数は実際にモニターへ表示されたフレーム数を保証しません。静止画で取得数が少なくても異常とは限りません。動きのある映像で比較してください。レポートの `pointer_updates` と `cursor_shape_uploads` は実カーソルのDXGI更新、`cursor_visible_present_submits` は通常モードでカーソルが表示領域と交差していた提出数です。合成fixtureの値は別フィールドです。

画面画像の取得に Desktop Duplication API を使用します。キャプチャ画像は Direct3D 11 テクスチャ間でコピーし、ピクセルシェーダーで拡大して表示します。マウスカーソルは Desktop Duplication API の位置と形状を取得し、形状変更時だけカーソル画像を GPU テクスチャへアップロードして同じピクセルシェーダーで合成します。フレームごとの画面画素の CPU 読み戻しと画像バッファーの割り当てを行いません。既定のフレーム遅延上限は 1 です。

## 実測結果

v0.1.3をRTX 3090 Ti搭載PCで、FHD画面・FHDクライアント領域の `--benchmark-cursor` で20.003秒計測しました。Capture 59.992 FPS、Present submit 59.992 FPS（1200フレーム）で、固定カーソル形状を1回GPUへアップロードし、1200回すべてで画面内カーソルを合成しました。配布物の `BENCHMARK.txt` に詳細があります。このfixtureはGPU合成負荷を連続計測するためのもので、OSの実カーソルを動かしません。

v0.1.1の参考計測では、FHD画面とFHDクライアント領域で20.011秒間、Capture 59.967 FPS、Present submit 59.967 FPS（1200フレーム）でした。

実際のDXGIカーソル取得経路はv0.1.1で141.892秒のスモーク確認を行い、FHDキャプチャでCapture 59.841 FPS、Present submit 59.898 FPSを計測しました。位置更新8回、形状アップロード5回を確認しています。この確認は実カーソル情報の取得と形状更新の確認であり、実カーソルが全Presentにわたり表示される負荷試験ではありません。性能値は計測したPCと条件での実測であり、他の環境での性能を保証しません。

v0.1.3では保存したウィンドウ位置・サイズ、選択画面、FPS、最前面表示を通常終了後に再起動して復元することを実機で確認しました。最大化中に終了した際の通常サイズ保持と、全画面表示中に終了した際の通常サイズ保持も確認しています。設定の実機テスト後、20秒のFHD描画計測の前後で設定ファイルのSHA256が一致し、ベンチマーク実行が設定を変更しないことも確認しました。Fit の黒帯解消、50%・25% Scale の再起動後復元、メニューが複数行になる狭いウィンドウでのFitも確認しました。

## 制限

- 60 FPS は目標値です。元ディスプレイの更新頻度、アプリの画面更新、GPU、電源設定、デスクトップ合成の影響を受け、すべての PC で保証できる値ではありません。
- Windows の保護された画面やセキュアデスクトップは取得できません。HDR 対応、色域変換、複数 GPU 間のコピーは実装していません。
- Desktop Duplication API の利用数や表示モードにより初期化に失敗することがあります。ディスプレイ設定を変更した際は、画面を再選択してください。
- ディスプレイ構成変更やアクセス喪失の後に再初期化できない場合は、アプリを再起動してください。
- 全画面表示はキャプチャ対象とは別のプライマリディスプレイに表示します。ウィンドウをキャプチャ対象上に移動すると、そのウィンドウ自身が映像へ映り込みます。

アプリの実行ファイルと同じフォルダーに `exmonitor-viewer.log` を作成します。ログは ASCII 文字のみです。
