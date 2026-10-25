# 開発者向け情報

## ビルドと検証

Windows環境でRust、Windows SDK、Visual Studio C++ Build Toolsを用意し、リポジトリのルートで次を実行します。

    .\build.cmd

## 起動オプションと計測

実行ファイルはモニター一覧表示、初期モニター/FPS/サイズ指定、一定時間の描画計測に対応します。オプション一覧は次で確認できます。

    .\dist\exmonitor-viewer.exe --help

固定されたGitHub latest endpointを照会するheadlessコマンドです。UI、ブラウザー、設定ファイルには触れず、ASCII JSONを標準出力へ書きます。

    .\target\release\exmonitor-viewer.exe --check-updates

計測結果を書き出す例:

    .\dist\exmonitor-viewer.exe --monitor 2 --size 1920x1080 --benchmark 10 --report benchmark.txt

カーソル合成を含める診断では --benchmark-cursor を追加します。これは固定の合成カーソルをGPUへ一度だけアップロードして描画する負荷確認用で、OSのマウス位置を動かしません。

キャプチャ除外ウィンドウの合成描画負荷を確認するには、対象となる保護ウィンドウがビューアーに表示されている状態で次を使います。

    .\dist\exmonitor-viewer.exe --monitor 2 --size 1920x1080 --benchmark 20 --dwm-composition --benchmark-composition-cursor --report benchmark-composition-cursor.txt

この診断モードでは実際のOSカーソル形状を使い、プロキシ表示位置だけを検出ウィンドウ上へ動かして、ビューアー上でマゼンタ色にします。OSカーソル移動や対象アプリへの入力は行いません。proxy_shape_source と proxy_position_source で形状元と診断用位置を区別できます。proxy_visible_submits が0なら、追加レイヤーを有効にした計測にはなりません。Present submitはアプリからの提出数であり、実際のモニター走査出力数ではありません。

合成カーソル自体の固定負荷だけを確認する場合は、上記に --benchmark-cursor を追加します。この場合はOSカーソル形状の代わりにテスト用形状を使用します。

## 配布ZIP

リリース用ビルド後、次のコマンドで利用者向けZIPとSHA-256チェックサムを作成します。

    .\tools\package-release.ps1

ZIPには実行ファイル、README、LICENSE、リリースノート、チェックサムを含めます。開発用のCLIや計測レポートは含めません。
