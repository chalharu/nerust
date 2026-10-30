# Wakui Prototype 3

第2試作を土台に、所有権・入力・終了契約を整理し、実用設定画面を通して検証する第3試作。
既存nerustとは未接続。`prototype_1/`と`prototype-2/`は比較基準として変更していない。
継承クレート名は`wakui2-*`のまま、実用アプリは`wakui3-settings`と`wakui3-library`。
独立workspaceで相互依存しない。
**UI接続用のbuild.rs・契約crate・入力enum・手書きBinding接続は不要。**

## まず動かす

Rust 1.95以上、GTK4開発環境（Tao/LinuxはGTK3）が必要。
このディレクトリで実行する。親のnerust workspaceとは独立している。
macOSの主backendとしてAppKit版を追加した。GTKをインストールせず、
`--no-default-features --features backend-macos`で設定・一覧を実行できる。
起動環境、native入力の検証範囲、未実施の手動ゲートは[macos/README.md](macos/README.md)。

```sh
cargo run --locked -p wakui3-settings
cargo run --locked -p wakui3-settings -- --config /path/to/settings.json
cargo run --locked -p wakui3-settings --no-default-features --features backend-tao
cargo run --locked -p wakui3-library -- --folder /path/to/roms
cargo run --locked -p wakui3-library --no-default-features --features backend-tao -- --folder /path/to/roms
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
| 過負荷回復 | scoped `Store::watch`で派生状態・effect開始を通常FIFOへ配送。容量1でも保存成功／task拒否から復帰 |
| 第2実用画面 | フォルダー・filterの非同期走査、安定keyの仮想化一覧、古い結果の除外、エラー復帰 |

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

第2画面は[library/README.md](library/README.md)。指定フォルダーの対応拡張子を
非再帰で走査し、ファイル名・サイズ・systemを表示する。ROM内容を読まず、
ユーザーフォルダーへ書かない。8192項目訪問・256結果の上限があり、検索中も編集可能。
設定と検索で共通の`Store::watch`を使い、アプリ固有の状態機械・表示内容は残す。
watchは最新値へ統合し、容量が空いた後に公開完了後のFIFOで反応する。
正のingress容量が必要で、解除は未実行反応を無効化するが開始済みeffectまでは止めない。

GTKはネイティブ部品。Taoはnative Window/Menuとsoftware iced部品で、同じ部品ではない。
GTK4/GTK3は同じプロセスへロードせず、backend featureはどちらか一つを選ぶ。

## 検証

macOSでは対話デスクトップ上で`sh scripts/verify-macos.sh`を使う。
共通runtime/macro/実用画面の回帰、strict Clippy、AppKit両画面smoke、
field editorの注入入力、native Menu、bitmap、反応連鎖中の合成入力を検査する。
Taoも別processのmain-thread runnerでstrict Clippy、実窓描画、system clipboard、
gallery、設定/一覧smokeを検査する。Taoだけなら`sh scripts/verify-tao-macos.sh`。
Mac/Taoのclose/menuはAPI操作であり、実OSのWM/accelerator配送とは区別する。
`sh scripts/measure-macos.sh`はreleaseの描画CPU・合成入力・検索連打と射影を観測する。
外部IMEやVoiceOverの実操作、compositor frame時間を測ったものではない。
両macOSアプリは64イベントの明示的な配送予算を使い、既存Storeの同期契約は維持する。
Linux版の既存総合手順と過去の成功数は以下のとおりで、macOSで再実行した意味ではない。

Dockerが利用可能なら、このMacから`sh scripts/verify-docker.sh`でDebian/bookworm、
Rust 1.99、GTK4/GTK3、Xvfb/XTestの検証imageを構築して同じLinux正式ゲートを実行できる。
続けてrelease性能ゲートも実行する。hostのtargetは共有せず、専用Docker volumeを使う。
`--init`、native試験の45秒上限、失敗ログ表示を含む。Linux/X11の証拠であり、
macOS実IME・VoiceOverやWayland実機の証拠とはしない。
今回このMac上のDockerで`VERIFY3 PASS`と`MEASURE3 PASS`を確認した。
GTK共有二窓、動的menu、Taoの拒否/復帰、設定/一覧の両backend native smokeを含む。
GTKのmenu/controllerをshortcutより先に切り離し、終了時のcriticalを修正した。
AT-SPI bus未導入の警告は残るため、Linux accessibilityの合格証拠とはしない。

YAML診断は`sh scripts/check-yaml.sh check ...`で有効化する（Python 3）。
typo・型不一致・未知command・参照シグネチャ不一致がYAML行列をprimary spanに持つ。
stable proc-macro単体では外部ファイルspanを作れないためwrapperが必要で、LSPではない。

`sh scripts/verify.sh`で整形、Clippy、core・macro試験、GTK個別native試験、
Tao native試験、最小例の両backendビルド、両galleryとclipboard/hotkey/About/Quit、
両構成の設定／検索試験と実画面スモークを確認する。
過去の基準記録は185種類成功、設定25件・検索14件の両feature実行とGridの倍率2再実行を含め225実行。
設定アプリの実GTK二窓試験も専用プロセスで実行し、共有Viewの一方を閉じても
他方の入力と非同期結果反映が続くことを確認する。
終了コード0と`VERIFY3 PASS`を確認済み。継承galleryの`PROTOTYPE2`/`PERF2`マーカーは
このworkspaceでの回帰を表し、保存済みの第2試作を再実行した意味ではない。
Xvfbと`timeout`が必要。新しいX11入力回帰にはlibX11/libXtstとXTEST拡張も必要。
総合手順は`WAKUI_REQUIRE_XTEST=1`で欠落を成功扱いにしない。
GTK試験は初期化スレッド制約から一件ずつ別プロセスで実行する。
`GDK_BACKEND=x11`を明示し、継承Wayland displayがXvfbを迂回しないようにする。

Alpine/muslでの検証設定：

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=/tmp/wakui3-target \
  RUSTFLAGS='-C target-feature=-crt-static -C prefer-dynamic=no' sh scripts/verify.sh
```

同じ環境設定で`sh scripts/measure.sh`を実行すると、releaseの9条件scalingと、
実App・配線を使った設定300更新／検索20回×256行のCPU観測を再現できる。
保存・描画・実入力の遅延測定ではない。Draft・射影・行Store同一性に加え、
scoped更新とsettings/libraryのp50/p95 CPU閾値をassertし、超過は非ゼロ終了する。
既定予算（p50/p95）はscoped 100/500 us、settings 1/5 ms、library 5/20 ms。
`WAKUI_SCOPED_P50_NS`/`P95_NS`、`WAKUI_SETTINGS_P50_NS`/`P95_NS`、
`WAKUI_LIBRARY_P50_NS`/`P95_NS`で対象環境の予算へ変更する。
ゼロ閾値で`MEASURE3 FAIL`となる失敗経路も検査する。

## 境界を明示する

- `view!`はアプリ型をHOST側で実行しない。任意typetag拡張crateを自動ロードする方式ではなく、
  明示factory pathを生成し、TARGETのRust型検査へ渡す。
- Templateは対応するControlの任意合成。nested Window、循環Templateは拒否する。
  項目keyは空文字・`/`・重複を拒否。仮想化は固定行高で、可変行高は未対応。
- Commandと動的Menu引数はClone + 'staticな型を利用できる。Menuは`MenuItem<A=String>`、
  生成コードは`typed_menu`を使い、既存Stringの`menu`入口も維持する。
  引数内の`Self`は元モデル型へ再帰置換する。不透明な型macro内の`Self`は明示拒否する。
  genericモデル、readonly field属性、完全なsource map/editor LSPは未対応。
- 設定画面は明示的なDraftで編集bufferを保持する。汎用converterの失敗時は従来どおり
  正規値へ戻るため、自動的に全TextBoxが編集bufferを持つわけではない。
- 任意closure更新時のSのCloneと全購読の射影・比較は残る。`snapshot()`も所有Cloneを返す。
  `update_scoped`のowned置換はCloneを呼ばず、field別indexで無関係な購読を評価しない。
  Noneならno-op generation。変更fieldの完全な宣言と、Rc/Arc等による不変payload共有が必要。
  生成bindingも`bind_scoped`を使うが、通常のTwoWay setterでは状態Cloneは残る。
  16 MiB状態の射影読み取りでClone 0回を試験したが、O(1)更新やフレーム性能は保証しない。
  `runtime/examples/scaling.rs`は状態0/1/16 MiB×購読1/100/1000の9条件で、
  読み取り50,000回と更新20回を計測し、Clone回数・通知回数をassertする。
  総合検証でも実行する。比較用scoped計測は1万購読まで検査し、release性能ゲートで判定する。
- async validationはModelへの反映後検証。古い結果は捨て、退去時に中断する。
  AppKitでもTemplate中のtokenは行scopeへ保持し、再mountで購読を蓄積しない。
- 多窓終了の確認待ちは無期限で、承認後のdrainだけに期限を置く。
  MacHostは1 Sessionを複数Windowで共有し、別Sessionのattach/窓登録を明示拒否する。
- GTKの既存loop接続は`GtkHost`＋runtime `run_async`。
  `coordinate_close`を使うとnative close要求を多窓coordinatorへ接続できる。
  アプリ固有の確認・保存drain hookは明示登録する。
- Taoの外部reactor接続は明示的unsupported。Tao 0.37にpreedit eventがなく、
  in-place preedit、physical-key identity、OS accessibilityは対応済みとしない。
  GTKはXTestによる日本語Unicode compose・Tab・clipboardを確認したが外部IME変換ではない。
  Taoは実ASCIIキー／日本語clipboardとsignal注入commitを区別して検証し、
  focus喪失時のiced修飾キー残留を修正した。TaoのCtrl+Shift+U composeは成立していない。
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

## 本実装への出口条件

2026-10-31のレビュー指摘3件は修正し、native回帰へ追加した。
macOS通常151試験とnative回帰、Docker/Linux正式ゲート、両環境のrelease性能予算が成功した。
追加レビュー2件も修正し、closed View登録の無副作用拒否と、active窓close後の
menu/Quit引き継ぎを再presentなしのnative回帰で確認した。TaoのMac専用ゲートも追加し、
AppKit/TaoのMac総合ゲート、GTK/TaoのDocker総合ゲートと両CPU予算を再実行して成功した。
方式比較を広げるための新prototypeは追加せず、AppKitの実IME・Full Keyboard Access・
VoiceOver・実入力/描画の未実施ゲートを完了する。詳細は[macos/README.md](macos/README.md)。
本体の映像surfaceと同居する場合はattach/detach・resize・終了の最小統合試験も必要。
その後、設定など一画面から段階的に本実装へ進む。API固定・全面置換の承認ではない。