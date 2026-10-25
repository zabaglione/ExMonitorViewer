# ExMonitorViewer

Windows 11向けの拡張モニター表示アプリです。別のディスプレイをウィンドウに表示し、会議アプリの操作パネルを手元の画面で確認できます。

## ダウンロードと起動

[最新リリース](https://github.com/zabaglione/ExMonitorViewer/releases/latest)を開き、ExMonitorViewerのWindows x64 ZIPをダウンロードして展開し、exmonitor-viewer.exe を起動してください。RustやVisual Studio Build Toolsのインストールは不要です。

初回起動時は、通常プライマリディスプレイに表示ウィンドウが開き、2台目のディスプレイが表示対象になります。ウィンドウ上部のメニューから表示対象や表示方法を変更できます。

## 主な機能

- 別ディスプレイの映像をGPUで取得して表示します。フレームレート上限は30、60、無制限から選べます。
- ウィンドウの大きさを自由に変更できます。Fit window to image または F キーで映像と同じ縦横比へ合わせ、Scale メニューで映像の原寸に対する倍率を選べます。
- Always on top で最前面表示を切り替えます。1～9 キーで表示対象を選び、F11 キーでプライマリディスプレイに全画面表示します。
- ウィンドウ位置・サイズ、表示対象、FPS、最前面表示などは通常終了時に保存し、次回起動時に復元します。

## キャプチャ除外ウィンドウの合成（DWM）

Windowsの画面共有キャプチャでは、キャプチャ除外に指定されたウィンドウは通常のキャプチャ結果に現れません。そのためWebexの操作パネルも、通常の画面複製ではビューアーに表示されないことがあります。メニューの「DWM composition」をクリックし、末尾の表示をOFFからONに切り替えると、Windowsのデスクトップ合成器（DWM）から除外対象のWebexパネルを直接ビューアーへ重ねて表示します。末尾のON/OFFは現在の状態を表します。初期状態はOFFです。

パネルとその上のカーソル表示は画面共有から引き続き除外され、ビューアー映像は通常どおり共有できます。Webexの共有設定や入力は変更せず、会議名やウィンドウタイトルも読み取りません。現在の対応対象はWebexの操作パネルです。パネルが見つからない場合やOSが除外表示に対応できない場合は、表示しません。

カラーのカーソルは形状と透明度を保って表示します。一部の反転型カーソルは、元の背景色を取得できないため、反転部分を白、輪郭を黒で表示します。

## 設定とサポート
設定は %LOCALAPPDATA%\ExMonitorViewer\settings.json に保存されます。画面構成が変わって保存先の画面が見つからない場合は、利用可能な画面を選択してください。保存位置が表示領域外にある場合は、ウィンドウを操作できる位置へ戻します。

Windows 11 x64向けです。Microsoft Visual C++ v14 Redistributable (x64) が必要です。未導入で `VCRUNTIME140.dll` のエラーが出る場合は、[Microsoftのダウンロード案内](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)からx64版をインストールしてください。60 FPSは目標値で、元画面の更新頻度、GPU、Windowsのデスクトップ合成や他のアプリの負荷により実際の速度は変わります。HDR表示と複数GPU間の転送には対応していません。

アプリと同じフォルダーの exmonitor-viewer.log をサポート時の調査に利用できます。ログに会議名やウィンドウタイトルは記録しません。

## 開発・機能改善

開発に参加する場合は、[GitHubリポジトリ](https://github.com/zabaglione/ExMonitorViewer)をcloneしてください。Rust、Windows SDK、およびVisual Studio C++ Build Toolsが必要です。

    git clone https://github.com/zabaglione/ExMonitorViewer.git
    cd ExMonitorViewer
    .\build.cmd

開発者向けのCLI、計測方法、パッケージ作成方法は[開発者向け情報](https://github.com/zabaglione/ExMonitorViewer/blob/main/docs/development.md)を参照してください。
