# Wakui Prototype 2

独立した第2試作。既存nerustとは未接続。第1試作は隣の`prototype_1/`に保存した。
**アプリにbuild.rs・契約crate・入力enum・手書きBinding接続は不要。**

## まず動かす

Rust 1.95以上、GTK4開発環境（Tao/LinuxはGTK3）が必要。
このディレクトリで実行する。親のnerust workspaceとは独立している。

```sh
cargo run --locked -p wakui2-demo --example minimal
cargo run --locked -p wakui2-demo
cargo run --locked -p wakui2-demo --no-default-features --features backend-tao
```

最小例は`demo/examples/minimal.rs`と`demo/ui/minimal.yaml`。
Cloneな状態型と`#[commands]`メソッドを書き、Storeを作り、`view!`でYAMLを読み込む。
画面接続とCommandの定型コードはマクロが生成し、field・型・署名はRustが検査する。
YAML変更を`include_str!`で追跡し、実行時に定義を再解析しない。
YAMLだけの変更で再コンパイルされることも実際の子Cargo試験で確認する。

## 第1試作から変えたこと

| 要求 | 第2試作 |
|---|---|
| margin/padding | scalarまたは4辺を指定。葉・containerの双方に適用 |
| 利用コード削減 | `#[commands]`＋`view!`。Modelの振る舞いと自動操作試験を分離 |
| 手続き型マクロ | OUT_DIR生成をやめ、選択backendの直接呼び出しを展開 |
| Layout | StackLayout・Grid・FlexLayout・AbsoluteLayout。BindableLayoutで項目配置 |
| Template | 任意の対応Controlを再帰合成。Data/Content、nested items、root/item binding |
| 項目Model・仮想化 | `Store<T>`の安定key、固定行高の実scroll・spacer・退去時解放 |
| native primitive | アプリ側factoryをRust pathで指定。GTK Widget・Tao iced Elementをmount |
| Command・変換・validation | String引数、gate、双方向converter、Latest async validation |
| Menu | 動的snapshot、native shortcut、Copy/Cut/Paste/SelectAll/About/Quit/Separator |
| async・終了 | 有界FIFO/tasks、Latest/Serial/Parallel、確認・cancel・drain・timeout、多窓coordinator |

構文と生成規則は`macros/README.md`、解析制限とschemaは`markup/`、
所有権・上限・終了契約は`runtime/README.md`、native差は`gtk/README.md`・`tao/README.md`を参照。

## 専用gallery

`demo/ui/gallery.yaml`は、全LayoutとTemplate、1000件の項目Modelを使用する。
表示する行だけをmountし、画面外へ出てもModelの編集状態は保持する。
名前のasync検証、数値変換、項目Command、ContentTemplate、動的Menu、
ボタン／メニュー共通の非同期保存を組み合わせる。
保存先は一時ディレクトリの`wakui2-demo-<pid>.txt`。APIキーや`.env`は不要。
保存workerのjoinはUI loopの復帰後に行う。

GTKはネイティブ部品。Taoはnative Window/Menuとsoftware iced部品で、同じ部品ではない。
GTK4/GTK3は同じプロセスへロードせず、backend featureはどちらか一つを選ぶ。

## 検証

`sh scripts/verify.sh`で整形、Clippy、core・macro試験、GTK個別native試験、
Tao native試験、最小例の両backendビルド、両galleryとclipboard/hotkey/About/Quitを確認する。
Xvfbと`timeout`が必要。GTK試験は初期化スレッド制約から一件ずつ別プロセスで実行する。
`GDK_BACKEND=x11`を明示し、継承Wayland displayがXvfbを迂回しないようにする。

Alpine/muslでの検証設定：

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/wakui2-target \
  RUSTFLAGS='-C target-feature=-crt-static -C prefer-dynamic=no' sh scripts/verify.sh
```

## 境界を明示する

- `view!`はアプリ型をHOST側で実行しない。任意typetag拡張crateを自動ロードする方式ではなく、
  明示factory pathを生成し、TARGETのRust型検査へ渡す。
- Templateは対応するControlの任意合成。nested Window、循環Templateは拒否する。
  項目keyは空文字・`/`・重複を拒否。仮想化は固定行高で、可変行高は未対応。
- Command引数は試作用String。汎用typed引数、readonly field属性、
  parse不能の編集buffer保持は未実装。変換失敗時は正規値へ戻す。
- async validationはModelへの反映後検証。古い結果は捨て、退去時に中断する。
- GTKの既存loop接続は`GtkHost`＋runtime `run_async`。
  `coordinate_close`を使うとnative close要求を多窓coordinatorへ接続できる。
  アプリ固有の確認・保存drain hookは明示登録する。
- Taoの外部reactor接続は明示的unsupported。Tao 0.37にpreedit eventがなく、
  in-place preedit、physical-key identity、OS accessibilityは対応済みとしない。
- system clipboardは実displayへ接続する。Waylandはseat/focus条件が別途必要。
  GTK3のQuitは明示native shim。一部predefined roleの無効化等はmuda制限で拒否する。
- 開始済みblocking I/Oをabort/timeoutで停止・rollbackすることはできない。
- Linux/X11での実証をWindows/macOS・Wayland実機対応の証拠にはしない。
  1000件の動作は性能保証でもない。未検証事項と今後の条件は設計書へ記録する。

設計・実証記録の正本は隣接`nerust-docs/explanation/wakui-prototype-2.md`。