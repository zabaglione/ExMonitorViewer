# ExMonitorViewer

Windows 用の拡張モニター表示アプリです。選択したディスプレイの映像を GPU で取得し、別ウィンドウに表示します。デフォルトの表示上限は 60 FPS です。

## 起動

`dist\exmonitor-viewer.exe` を起動します。Rust と Visual Studio Build Tools がある環境では `build.cmd` でリリース版をビルドできます。表示ウィンドウは通常、プライマリディスプレイ上に開き、キャプチャ対象は 2 台目のディスプレイを優先します。

ウィンドウのメニューからキャプチャする画面、FPS 上限（30、60、無制限）、最前面表示を選べます。`1`～`9` キーで画面を切り替え、`F11` キーでプライマリディスプレイの全画面表示にします。縦向きモニターと縦横比の異なるウィンドウにも対応します。

## 計測

`--list-monitors` は Windows の表示名と解像度を表示します。`--monitor` の番号は一覧と同じ 1 始まりです。`--size` はウィンドウのクライアント領域の大きさを指定します。`--fps 30`、`--fps 60`、`--fps 0` で初期表示上限を選べます（0 は無制限）。`--benchmark` を指定すると指定した秒数だけ表示して終了し、`--report` で ASCII の計測結果を書き出します。

```powershell
.\target\release\exmonitor-viewer.exe --list-monitors
.\target\release\exmonitor-viewer.exe --monitor 2 --size 1920x1080 --benchmark 10 --report benchmark.txt
.\target\release\exmonitor-viewer.exe --monitor 2 --fps 0 --size 1920x1080 --benchmark 10 --report benchmark-unlimited.txt
```

配布用 ZIP は、ビルド後に計測レポートを指定して作成できます。

```powershell
.\tools\package-release.ps1 -BenchmarkPath .\benchmark-report.txt
```

タイトルバーには `Capture` と `Present submit` の FPS が表示されます。`Capture` は Desktop Duplication API が新しい画面画像を取得した回数です。`Present submit` はウィンドウへ提出できた回数です。提出数は実際にモニターへ表示されたフレーム数を保証しません。静止画で取得数が少なくても異常とは限りません。動きのある映像で比較してください。

画面画像の取得に Desktop Duplication API を使用します。キャプチャ画像は Direct3D 11 テクスチャ間でコピーし、ピクセルシェーダーで拡大して表示します。フレームごとの画素の CPU 読み戻しと画像バッファーの割り当てを行いません。既定のフレーム遅延上限は 1 です。

## 実測結果

RTX 3090 Ti 搭載環境で、動き続けるテストパターンを `\\.\DISPLAY2` から FHD でキャプチャし、FHD クライアント領域へ表示した20.008秒の計測では、Capture が59.676 FPS（1194更新）、Present submit が59.776 FPS（1196回）でした。詳細は配布物の `BENCHMARK.txt` を参照してください。この値は当該環境・条件での実測であり、他の環境での性能を保証しません。

## 制限

- 60 FPS は目標値です。元ディスプレイの更新頻度、アプリの画面更新、GPU、電源設定、デスクトップ合成の影響を受け、すべての PC で保証できる値ではありません。
- Windows の保護された画面やセキュアデスクトップは取得できません。HDR 対応、色域変換、マウスカーソルの合成、複数 GPU 間のコピーは実装していません。
- Desktop Duplication API の利用数や表示モードにより初期化に失敗することがあります。ディスプレイ設定を変更した際は、画面を再選択してください。
- ディスプレイ構成変更やアクセス喪失の後に再初期化できない場合は、アプリを再起動してください。
- 全画面表示はキャプチャ対象とは別のプライマリディスプレイに表示します。ウィンドウをキャプチャ対象上に移動すると、そのウィンドウ自身が映像へ映り込みます。

アプリの実行ファイルと同じフォルダーに `exmonitor-viewer.log` を作成します。ログは ASCII 文字のみです。
