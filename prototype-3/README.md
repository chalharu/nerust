# Wakui Prototype 3

第2試作を土台に、所有権・入力・終了契約を整理し、実用設定画面を通して検証する第3試作。
既存nerustとは未接続。`prototype_1/`と`prototype-2/`は比較基準として変更していない。
継承クレート名は`wakui2-*`のまま、設定アプリは`wakui3-settings`。独立workspaceで相互依存しない。
**アプリにbuild.rs・契約crate・入力enum・手書きBinding接続は不要。**

## まず動かす

Rust 1.95以上、GTK4開発環境（Tao/LinuxはGTK3）が必要。
このディレクトリで実行する。親のnerust workspaceとは独立している。

```sh
cargo run --locked -p wakui3-settings
cargo run --locked -p wakui3-settings -- --config /path/to/settings.json
cargo run --locked -p wakui3-settings --no-default-features --features backend-tao
cargo run --locked -p wakui2-demo --example minimal
cargo run --locked -p wakui2-demo
cargo run --locked -p wakui2-demo --no-default-features --features backend-tao
```

最小例は`demo/examples/minimal.rs`と`demo/ui/minimal.yaml`。
Cloneな状態型と`#[commands]`メソッドを書き、Storeを作り、`owned_view!`でYAMLを読み込む。
共有するモデルには`view!`を使う。共有Viewの終了は他の所有者のStoreを閉じない。
`owned_view!`はそのViewをモデルの終了責任者とする明示的な選択である。
画面接続とCommandの定型コードはマクロが生成し、field・型・署名はRustが検査する。
YAML変更を`include_str!`で追跡し、実行時に定義を再解析しない。
YAMLだけの変更で再コンパイルされることも実際の子Cargo試験で確認する。

## 第2試作から変えたこと

| 課題 | 第3試作 |
|---|---|
| モデルの寿命 | 共有`view!`と専用`owned_view!`を分離。共有View終了後の書き込み・effect継続を試験 |
| 読み取りコスト | Bindingのget/初期購読は`Rc<S>`から射影し、状態全体をCloneしない |
| Command | 明示引数は`Binding<A>`。整数・enumをRustが型検査。UIには引数消去済みCommandを渡す |
| 入力途中状態 | 設定アプリの文字列Draftと保存済みConfigを分離。空文字・`-`・不正文字列を保持 |
| 終了要求 | 両backendのWM close/Quitを`request_close`へ。強制`close`とは別契約 |
| native所有権 | `WeakView`を公開。close-ready購読から強参照循環なしで終了 |
| 共通UI | `Primary+S/Q`等の共通shortcut、Grid等幅列・明示placement、Flexの限定的契約 |
| 実用画面 | 名前・音量・倍率、保存・取消、破損設定の拒否、未保存終了確認、保存中の再編集 |

構文と生成規則は`macros/README.md`、解析制限とschemaは`markup/`、
所有権・上限・終了契約は`runtime/README.md`、native差は`gtk/README.md`・`tao/README.md`を参照。

## 実用設定画面と回帰gallery

入口は[settings/README.md](settings/README.md)。既定保存先は作業ディレクトリの
`wakui3-settings.json`。起動時は読み込みのみで、存在しなければ既定値を表示する。
壊れた既存JSONは起動を拒否し、自動上書きしない。
通常の終了は「保存して終了／破棄して終了／編集を続ける」の画面内確認へ接続する。
保存中の終了要求は完了を待ち、新しい編集があれば再確認する。
設定入力の検証は同期式。非同期保存・失敗・受付拒否・再編集の状態機械を試験する。
APIキーや`.env`は不要。

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
Tao native試験、最小例の両backendビルド、両galleryとclipboard/hotkey/About/Quit、
両構成の設定試験と実画面スモークを確認する。
154種類成功。設定23件の両feature実行とGridの倍率2再実行を含む実行数は178件。
終了コード0と`VERIFY3 PASS`を確認済み。継承galleryの`PROTOTYPE2`/`PERF2`マーカーは
このworkspaceでの回帰を表し、保存済みの第2試作を再実行した意味ではない。
Xvfbと`timeout`が必要。GTK試験は初期化スレッド制約から一件ずつ別プロセスで実行する。
`GDK_BACKEND=x11`を明示し、継承Wayland displayがXvfbを迂回しないようにする。

Alpine/muslでの検証設定：

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/wakui3-target \
  RUSTFLAGS='-C target-feature=-crt-static -C prefer-dynamic=no' sh scripts/verify.sh
```

## 境界を明示する

- `view!`はアプリ型をHOST側で実行しない。任意typetag拡張crateを自動ロードする方式ではなく、
  明示factory pathを生成し、TARGETのRust型検査へ渡す。
- Templateは対応するControlの任意合成。nested Window、循環Templateは拒否する。
  項目keyは空文字・`/`・重複を拒否。仮想化は固定行高で、可変行高は未対応。
- Command引数はClone + 'staticな型を利用できるが、動的Menuの引数はStringのまま。
  genericモデル、readonly field属性、完全なsource map/editor LSPは未対応。
- 設定画面は明示的なDraftで編集bufferを保持する。汎用converterの失敗時は従来どおり
  正規値へ戻るため、自動的に全TextBoxが編集bufferを持つわけではない。
- 更新時のSのCloneと全購読の射影・比較は残る。`snapshot()`も所有Cloneを返す。
  16 MiB状態の射影読み取りでClone 0回を試験したが、O(1)更新やフレーム性能は保証しない。
  `runtime/examples/scaling.rs`は状態0/1/16 MiB×購読1/100/1000の9条件で、
  読み取り50,000回と更新20回を計測し、Clone回数・通知回数をassertする。
  総合検証でも実行し、時間に環境依存の合否閾値を置かない。
- async validationはModelへの反映後検証。古い結果は捨て、退去時に中断する。
- GTKの既存loop接続は`GtkHost`＋runtime `run_async`。
  `coordinate_close`を使うとnative close要求を多窓coordinatorへ接続できる。
  アプリ固有の確認・保存drain hookは明示登録する。
- Taoの外部reactor接続は明示的unsupported。Tao 0.37にpreedit eventがなく、
  in-place preedit、physical-key identity、OS accessibilityは対応済みとしない。
- system clipboardは実displayへ接続する。Waylandはseat/focus条件が別途必要。
  GTK3のQuitは明示native shim。一部predefined roleの無効化等はmuda制限で拒否する。
- 開始済みblocking I/Oをabort/timeoutで停止・rollbackすることはできない。
  設定保存は同一directoryの一時ファイルからrenameする。Windowsの既存ファイル置換、
  directory fsync・電源断耐性・複数process競合の保証はない。詳細は設定アプリREADMEを参照。
- 通常終了はrequest経路、`close()`は明示的な強制終了。外部destroy/process終了は
  graceful shutdownではない。Tao多窓loop/外部host、IME preeditは未対応のまま。
- Linux/X11での実証をWindows/macOS・Wayland実機対応の証拠にはしない。
  1000件の動作は性能保証でもない。未検証事項と今後の条件は設計書へ記録する。

設計・実証記録の正本は隣接`nerust-docs/explanation/wakui-prototype-3.md`。