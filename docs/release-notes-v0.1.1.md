# ExMonitorViewer v0.1.1

マウスカーソルを含めて拡張ディスプレイを表示できるようになりました。

- カーソル位置と表示状態を Desktop Duplication API から取得します。
- カーソル形状が変わった時だけ小さな形状データを GPU に転送します。
- 通常の画面キャプチャとカーソルの合成を GPU ピクセルシェーダー内で行います。
- COLOR、MONOCHROME、MASKED_COLOR の形状に対応します。
- カーソル更新の計測値 `pointer_updates`、`cursor_shape_uploads`、`cursor_visible_present_submits` をベンチマークレポートに追加しました。
- `--benchmark --benchmark-cursor` を指定すると、固定の合成カーソルを使った診断用描画負荷を測定できます。これは実 OS カーソルを移動したり、実カーソルの継続表示を証明したりするテストではありません。レポートにはfixture用shape upload、位置更新、visible present数を別に記録します。

## 検証結果

- GPU WARP のオフスクリーン描画テストを含む9件が成功しました。COLOR半透明、MONOCHROMEのAND/XOR、MASKED_COLORの置換/XOR、回転、端でのクリップ、縮小描画を検証しています。
- FHDキャプチャをFHDクライアント領域へ表示した20.011秒のfixture計測では、Capture 59.967 FPS、Present submit 59.967 FPS（1200回）でした。fixtureカーソルは1回アップロード、1193回位置更新され、1200回の成功Presentで合成されました。同梱の `BENCHMARK.txt` はこの測定値です。
- 別の141.892秒の実カーソルDXGIスモーク確認では、Capture 59.841 FPS、Present submit 59.898 FPS、位置更新8回、形状アップロード5回を確認しました。OSカーソルが入力操作後に退避するため、この数値は実カーソルの継続表示性能を表しません。

これらは計測したPCと条件での実測であり、他の環境で60 FPSを保証するものではありません。
