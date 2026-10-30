# Wakui standalone prototype

**新規構築の垂直スライス。既存nerustとは未接続。**
このディレクトリは独立したCargoワークスペースであり、親のCargo.toml・旧`gui/wakui*`は変更しない。
旧クレートやnerustのViewModel、Shell、エミュレータへ依存しない。
設計の正本は隣接リポジトリ`nerust-docs/explanation/wakui-architecture.md`以下。
これは完成した汎用フレームワークではなく、成立性を実際のUIで確認するための試作である。

## 構成

| ディレクトリ | 役割 |
|---|---|
| `contract` | host/target共有の型付きproperty/command契約。native/runtime依存なし |
| `definition` | Serde/typetagのControl・Menu DTO、lowering、明示的登録 |
| `build` | JSON/YAML解析、Binding型検証、選択backendのRustコード生成 |
| `runtime` | 型付きMVVM、RAII購読、再入キュー、最新要求の中断と世代検証、有界worker pool |
| `gtk` | GTK4 Widget、GLib local Future、GMenu/GAction、キー付き行の保持 |
| `tao` | Tao loop、iced/tiny-skia、muda、proxy起床型local executor |
| `demo-contract` | 専用アプリの共有契約。同じ宣言からkeyとmetadataを生成 |
| `demo-controls` | 本体に未知の`sample.Heading`。build依存にだけ置く拡張例 |
| `demo` | UI定義、build.rs、UI非依存Model、専用アプリ、生成器の回帰試験 |

`demo/ui/settings.yaml`を変更すると、アプリの`build.rs`が`OUT_DIR`に
`view_generated.rs`と`view_manifest.json`を生成する。
生成コードはWidget構築・Binding接続関数を直接呼び、runtimeでDTOを解析しない。
typetag拡張は標準Labelへloweringされ、アプリ通常依存にはparserも拡張DTOも不要。
JSONも同じcompiler APIで読み込め、YAMLとの生成物同値性を試験する。

## 動かす

Rust 1.95以上、GTK版はGTK4開発ライブラリ、LinuxのTao版はGTK3開発ライブラリが必要。
TaoはOS Window基盤であり、個別部品はiced描画であってOSネイティブ部品ではない。
GTK4とTao/GTK3を同じプロセスへロードしない。

```sh
cd wakui
cargo run --locked -p wakui-demo
cargo run --locked -p wakui-demo --no-default-features --features backend-tao
```

名前を編集するとTwoWayでModelへ反映され、ラベルとOneWayの読み取り専用欄へ伝播する。
Saveはメニューとボタンで共通のCommandを使用する。保存中は両方を無効化する。
一覧の追加・逆順・消去は安定した項目keyで更新する。
保存先は既定ではOSの一時ディレクトリの`wakui-demo-<pid>.txt`。
`--save-file PATH`で変更できる。アプリは一時ファイルへの書き込み・sync・renameで保存する。
`--help`で引数を確認でき、秘密情報・APIキー・`.env`は不要。

## 実証と試験

```sh
cargo run --locked -p wakui-demo -- --headless
GSK_RENDERER=cairo xvfb-run -a cargo run --locked -p wakui-demo -- --smoke
xvfb-run -a cargo run --locked -p wakui-demo --no-default-features --features backend-tao -- --smoke
sh scripts/verify.sh
```

全検証には`xvfb-run`と`timeout`が必要。
GTKのGPU検出警告を避けるため、仮想ディスプレイでは`GSK_RENDERER=cairo`を明示する。
スモークは実Window上で名前入力、OneWay拒否、メニュー保存、二重実行拒否、
ボタン保存、一覧の追加・並べ替え・消去、冪等な終了を検査する。
保存内容も検査し、成功時は`SMOKE PASS backend=...`を出力して通常終了する。
GTKは実Entryのchanged・Buttonのclicked・GActionのactivateを発火する。
Taoの自動操作はiced入力と同じdispatcherを使うが、OSのマウス/キー合成や画面画像比較ではない。
その違いを「全UIのE2E」とは扱わない。

Alpine/muslで静的CRT・動的Rust設定が競合する場合、この環境での検証設定は次のとおり。
通常の環境で必須の設定ではない。

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/wakui-prototype-target \
  RUSTFLAGS='-C target-feature=-crt-static -C prefer-dynamic=no' sh scripts/verify.sh
```

## 試作の制限と次の課題

- 共有契約deriveの一般化は未実装。専用アプリのマクロでkey/metadataの二重管理を避けている。
- 対応部品はWindow、縦Panel、Label、TextBox、Button、ItemsControl、1階層のMenu。
  属性・Command引数・変換・validationの汎用化、native role、shortcutは未実装。
- DataTemplateは型を検証するLabel行に限定。任意ControlのTemplate、項目ViewModel、
  ContentControl、仮想化は未実装。未対応Templateはビルド時エラー。
- GTKは行Widgetをkeyで再利用する。Taoはkey/orderを保持するstatelessラベル行で、
  編集可能な行状態やnative行identityを保証しない。
- 診断は構文位置またはファイル名＋ノード/属性。完全なsource map/editor schemaは未実装。
- YAMLのalias/tag拒否、正規化schema指紋、全入力の解析資源上限は今後の強化対象。
- `run`はネイティブループを駆動し、Model内のFutureを標準で実行する。
  任意の既存loopへattachする公開`run_async(UiHost, ...)`はまだ提供しない。
  pollsterやTokioをUIスレッドのblocking待機に使わない。
- asyncポリシーはLatestとCommand側の実行中gateを実証。Serial/Parallel、汎用async検証、
  非同期保存のclose確認画面・timeout coordinatorは未実装。
- poolは2 worker・有界queue。デモはUI終了後に明示joinするため、終了時の保存完了を待つ。
  実行中blocking jobは中断できない。汎用runtimeのDropだけではjoinせず、利用側のshutdownが必須。
- GTKタスクとTaoタスクはdriver終了時に中断する。GTKは完了済みhandleも終了まで保持する。
- フォーカス/IME preedit、system clipboard、アクセシビリティ、入力遅延/描画性能は未検証。
- 実機検証はLinux/Xvfb。Windows/macOSのコンパイル・起動・終了、クロスコンパイルは未検証。
- build時featureはGTK/Taoのどちらか一つだけ。`--all-features`は意図的に不可。

旧コードの削除や既存nerustへの統合は、このプロトタイプには含めない。