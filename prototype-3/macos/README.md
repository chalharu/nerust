# macOS AppKit Backend

第3試作のmacOS向け主backend。既存nerustとは未接続。
SwiftのAppKit部品を薄いC ABI経由で共通runtime・proc macroへ接続する。
Tao/icedの置換実装ではなく、NSWindow / NSTextField / NSButton / NSMenuを使う。

## 起動

macOSの対話デスクトップ、Rust 1.95以上、Xcode Command Line ToolsのSwiftが必要。
このworkspaceのルートから、backendを一つだけ選んで実行する。

```sh
env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH -u DYLD_INSERT_LIBRARIES \
  cargo run --locked -p wakui3-settings --no-default-features --features backend-macos
env -u DYLD_LIBRARY_PATH -u DYLD_FALLBACK_LIBRARY_PATH -u DYLD_INSERT_LIBRARIES \
  cargo run --locked -p wakui3-library --no-default-features --features backend-macos -- --folder /path/to/roms
sh scripts/verify-macos.sh
sh scripts/measure-macos.sh
```

`DYLD_LIBRARY_PATH=/opt/homebrew/lib`を継承すると、OSのImageIOがHomebrewの
PNG codecへ解決され、native画像取得中にSIGBUSとなることを実機で確認した。
手順はこのプロセスのDYLD探索・注入設定だけを除き、グローバル環境は変更しない。

アプリのbuild.rsやUI契約crateは不要。backend内部のbuild.rsだけがSwiftをコンパイルする。
Swiftモード5、SDK付属のAppKit、追加Swift packageなし。
開発用dylibはCargo OUT_DIRの絶対install nameで参照するため、target内の生成物を
単体コピーして配布できる構成ではない。app bundle、codesign、notarizationは未実装。
deployment targetの既定は11.0だが、最低OSやIntelでの実機動作を確認した意味ではない。

## 契約

- 主な対象は設定と一覧の二画面。Stack、Label、TextBox、Button、固定高ItemsControl、Menu。
  Grid/Flex/Absoluteは実行時エラー。ContentControl/custom meterはAPI未実装でコンパイル不可。
  GTK/Tao全galleryとの完全な機能互換は主張しない。
- main threadで一つのWindowとloopを所有する。外部loop、複数Window、UiHost/Sessionは未対応。
- TextBoxはAppKit field editorを利用し、選択、undo、clipboard、IME marked textをOSへ委ねる。
  marked text中はモデル通知によるstringValue置換を抑止する。これは汎用編集transactionや
  外部モデル更新との競合解決を保証するものではない。
- captionが`<field_id>_caption`ならその文字列をAX labelへ渡す。
  入力はAXTextField、部品には安定したaccessibility identifierを設定する。
  宣言順のkey-view loopを作り、動的node削除時に参照も更新する。
- Cmd+S等は共通Shortcutからnative key equivalentへ変換する。
  Edit menuはnative responder chainへ接続し、WM close / Cmd+Qは終了要求へ接続する。
  動的Menuは全ID・shortcutの検証後にnative表示とhandlerを交換する。不正更新は旧Menuを保持し、
  `menu_error(id)`で報告する。正常更新でそのMenuのエラーを解除し、hostは終了しない。
- Viewは購読tokenを持ち、callback側はWeakViewを使う。強制closeは冪等。
  アプリの確認・保存待ちは既存の配線を再利用する。
- 一覧は全keyを検査し、表示範囲の行だけmountする。範囲変更・順序変更では可視行を再mountする。
  行Storeは保持するが、native field editorの選択や一時編集状態の維持は保証しない。
  32px等の固定行高、viewport count、スクロール用native scrollerを使用する。
  countは1..256、行高は有限な1..4096で、共通runtimeと静的literalが同じ条件を検査する。
  初期不正ならmountせず、不正更新は前の正常表示を保持する。正常値への復帰と末尾ページをnative検査する。
  readonly/literalでも受理済みviewportをローカルに移動し、モデルへの書込はTwoWayだけで行う。
  `item_keys`は全sourceでなくmount済みkeyを返す。行高変更は既存mountにも反映する。
  行の退避では入れ子一覧のregistry、viewport handler、scoped tokenも解放する。
- padding/marginはnative frame insetへ合算する限定契約。任意のplacement、CSS互換、
  backend間のpixel一致は対象外。幅640px以上を対象とし、縦方向はnative scrollを使う。

## 公平性

macOS版の両アプリは`Store::with_dispatch_budget(..., NonZeroUsize(64))`を選択する。
既定の`Store::new` / `with_limits`は同期契約を維持する。
予算付きStoreは1回のdrainで指定件数を処理した後、Weakな継続futureへ渡す。
FIFO、容量制限、watchのcoalescing、closeによる未実行処理の破棄は維持する。
受付済みの書き込みが呼出しから戻る時点で反映済みとは限らない。

ホストはnativeイベントを1件配送してから最大32 futureを1回ずつpollする。
新規futureは次ターンへ回し、Pendingはwakeまでpollしない。
ready taskがなければnative側で最大8ms待ち、workerのwakeも次ターンで確認する。
一つのcallback / Clone / projection / pollは分割・中断できないため、時間上限や
リアルタイム性の保証ではない。1回のpollが長ければOSイベント処理も遅れる。

## 検証範囲

実行環境: Apple Silicon、macOS 15.7.5、rustc 1.99.0、Swift 6.2.4。

| 項目 | 自動検証の範囲 |
|---|---|
| 設定 | 共通smoke: Draft保持、readonly、保存、終了veto、保存待ち、再確認、最終JSON |
| 一覧 | 共通smoke + sandboxで60編集/20 burst、Latest結果、失敗復帰、選択、終了 |
| 入力 | field editorへmarked textと日本語commitを注入、実OS clipboardでcopy/cut/paste、Tab/逆Tab |
| Menu | native key equivalentでPrimary/Cmd+S、有効/無効、動的再構築 |
| Accessibility | role、identifierの読み返し。スクリーンリーダーによる操作ではない |
| 一覧mount | 100行Model、6行mount、範囲変更、順序変更、不正key拒否と復帰 |
| Viewport契約 | 初期不正は0 mount、count/行高の範囲外を拒否してmount保持、正常値復帰、usize::MAXの末尾ページ |
| readonly viewport | ローカル移動でモデルは不変、後続のモデル通知を反映。automationはnative scrollと同じhandlerを利用 |
| Menu拒否 | 不正ID・重複ID・後続項目の不正shortcutでも旧handler/引数を保持し、正常候補へ復帰 |
| 入れ子cleanup | 同じkeyで3回mount/退避し、registry消去・古いscrollの無効化・テンプレート資源の弱参照失効 |
| 動的行高 | `frame_height`で実native frameの32→64→32を読み返し、テンプレート再生成なしを確認 |
| 過負荷 | 8万更新、20合成キーを各連鎖完了前に配送、WeakView・closeの検査 |
| 描画 | 640/960pxのnative bitmap、連鎖中のAppKit描画CPU時間 |

conformance例はmain threadで実行する。libtestのworker threadからAppKitを初期化しない。
clipboardは試験前のデータへ復元する。画像は表示された専用temp directoryへ保存する。
CPU描画はcompositor presentation/fenceではなく、合成キーは物理入力ではない。
時間に合否閾値を置かず、入力が連鎖完了前に届くこと・状態・mount数をassertする。
release観測は`measure-macos.sh`で再現し、数値を安定APIや性能保証と扱わない。

### 2026-10-30の実行結果

AppKit追加時の基準commit `bea5cc2c5`では`verify-macos.sh`が142試験成功、失敗0、計測helper 2件は通常試験ではignored。
内訳はruntime 59、markup 11、macros 31、AppKit executor 2、settings 25、library 14。
整形、strict Clippy、両native画面smoke、conformanceも成功し、最終マーカーは
`VERIFY3 MACOS PASS`。Linux GTK/Taoのnative試験をこのMacで再実行した意味ではない。

`measure-macos.sh`は2件のhelperも実行し、`MEASURE3 MACOS PASS`を確認した。

viewport共通化の検証では、これにruntimeの境界値2件、macro literalの1件を加える。
AppKit conformanceの`viewport-contract` markerは正式ゲートで必須とする。
再実行は145試験成功、失敗0、helper 2件ignored、設定/一覧smokeと全必須marker成功。
内訳はruntime 61、markup 11、macros 32、AppKit executor 2、settings 25、library 14。
GTK/Taoの全targetをmacOSでcompileできるが、native Viewを作るlibtest試験はmain-thread
制約によりこの環境では実行しない。Taoの純粋なrange試験とAppKit専用runnerを分けて検証する。
GTKのstrict Clippyは成功。Taoのstrict Clippyは既存の`driver.rs:47`の
`needless_return`で停止し、無変更のコード由来と確認した後にそのlintだけを除外して成功した。
Linux/X11でのTao拒否/復帰試験とGTKの拡張native試験は未実行で、macOSの145件へ含めない。
この追補ではrelease計測helperは再実行しておらず、下表は基準commit時の観測を保存する。

レビュー5件への回帰検査は既存conformanceへ追加し、通常の145試験とは別に実行する。
正式ゲートは`readonly-viewport`、`menu-transaction`、`nested-cleanup`、`dynamic-row-height`も
必須markerとし、mount済みkeyの内容は既存`keyed-rows`検査で確認する。

| release観測 | p50 | p95 |
|---|---:|---:|
| 反応連鎖中の合成キー配送、20回 | 12.750 ms | 26.974 ms |
| 連鎖中のAppKit描画CPU、20回 | 0.408 ms | 0.532 ms |
| sandbox検索連打の完了、20回 | 10.435 ms | 14.940 ms |
| 検索中のAppKit描画CPU、20回 | 0.176 ms | 3.625 ms |
| 設定3更新サイクル、100回 | 22.417 us | 23.500 us |
| 一覧256行の要求・結果射影、20回 | 1.379 ms | 1.896 ms |

キーはプロセス内postEventからモデル通知まで、検索は4ファイルのsandboxへの
3連続編集から最新結果まで。描画はdisplayIfNeededのCPU時間であり、表示完了ではない。
設定300更新・一覧5120行同一性検査も成功した。ホスト復帰の最大間隔には
約180 msの観測もあり、厳密なフレーム時間上限や低遅延適合を示すものではない。
macOSのfixtureコピーが古いmtimeを保持するため、子Cargo試験は初期コピー時だけ
mtimeを現在へ設定する。YAML編集時にRustをtouchして再ビルドを偽装してはいない。

## アルファ利用の手動ゲート

以下は未実施。自動試験が成功しても適合済みと扱わない。

1. macOS日本語IMEでかな入力、候補表示、かな漢字変換、確定・取消。保存中と連続検索中にも行う。
2. 変換中の別欄移動、別appへの切替、再フォーカス、Cmd操作後の修飾キー残留を実キーで確認する。
3. Full Keyboard Accessの有無でTab/Shift+Tab、Space、Cmd+S/Q、未保存終了確認を操作する。
4. VoiceOverで欄名、値、readonly、ボタン状態、一覧、保存状態とエラーの到達性を確認する。
5. Instruments等で実入力から表示までの遅延とcompositor frameを確認する。

対象OSと主backendはmacOS/AppKit、Linux/X11/GTKとするが、このmacOS実装は限定アルファ。
Windows、Wayland、外部IME、VoiceOver、配布packageまで含む安定版ではない。
上記の実利用結果を得る前にAPI固定・本体移行は行わない。