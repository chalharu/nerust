# 薄いネイティブ View のための共通ホスト・プロトタイプ

## 結論

Tao／GTK の**実際の実行経路**で、イベント配送、ループの1ターン、drain 調停、
OSD 結線、settings の寿命管理を toolkit 非依存にできた。
さらに補助窓の存在・開閉予約・経路・世代IDと、focus／保留要求／live周期の
**状態所有権も共通ホストへ移した**。
ただし、**フロントエンド全体の大幅削減にはまだ達していない**。
ループの処理順序を共有するだけでは、補助ウィンドウや View 内の制御状態まで消えない。
これは最終アーキテクチャではなく、次の設計を判断するための動作する縦断プロトタイプ。

## 境界

- `kernel`: 軽量な純粋ロジック。依存関係は増やしていない。
- `frontend_host`（本クレート）: 共通イベントと高レベルな実行規約。
- `gui_shell`／`gui_runtime`／`gui_viewmodel`: セッション、設定、表示用モデルの既存責務。
- Tao／GTK: OS メッセージの受信・変換、widget 配置、native window／dialog、GPU 接続。

本クレートは Tao、GTK、Iced、winit に依存しない。
OS のメッセージポンプを共通クレートで所有するのではなく、各ポンプが共通 driver を呼ぶ。
この分離なら、アプリケーションのメインループを所有するネイティブ toolkit にも接続できる。
OS 固有の起動・終了、UI スレッド制約を無理に共通 loop に押し込まない。

## 実装した共通ロジック

| モジュール | 共通側が所有するもの | native adapter に残すもの |
| --- | --- | --- |
| `aux` | `AuxWindows<N,H>` の開閉予約、存在、世代ID、native routing、handle格納、重複／古い完了の拒否 | 窓の生成／再表示／破棄、native surface の生存確認、signal変換 |
| `focus` | `FocusCoordinator` のmain-active／pending／protocol-pause状態とburst-end判定 | native focus通知の変換、pause effectの適用 |
| `debug_schedule` | open要求の集約、最後のdrain pause、記憶座標、live周期・起床関心・thread登録 | timer／event送信、native窓の配置 |
| `session` | `SessionEvent` 配送、shortcut 解釈、pause 所有権との結線、settings 開始／失敗／終了、設定適用時の再調停 | キーコード変換、fullscreen／debugger open の実行、menu／title 反映 |
| `loop_driver` | `drive_turn` の処理順序、wake 集約、30ms の flight backstop、更新後の tick による repaint 判定 | OS loop、wake 予約、native redraw |
| `drain` | FIFO、pause 境界での batch 分割、同時 flight 1件、epoch 無効化、非ブロッキング poll | debugger 入力 snapshot の生成、pause port、表示結果適用、native timer |
| `frame` | settings／slots の Arc 寿命、変更時のみの chrome 再構築、OSD 派生設定、毎回新しい tick | native geometry の取得 |
| `osd` | 通知 pump、modal ACK 後の継続判断、strip 計算、presenter／raster／renderer sink 結線、描画先無効化 | 実ダイアログ、ACK 通知、modal pause effect の適用 |

`SessionDriver` は既存の `SessionHandle`／`PausePolicy` を借用する。
prototype のために全セッションを二重所有する巨大な application object は作らない。
`impl_session_controls!` は既存 shell の narrow trait に対する互換配線で、業務判断は
`SessionEvent` に集約している。

### イベントと副作用

メインウィンドウのキーとセッションコマンドは両 frontend から同じ配送口へ渡す。
fullscreen／debugger open は `SessionEffect` として返し、native 側が実行する。
Settings のキー capture と各 widget の値編集は既存 `SettingsViewModel` に渡す。

すべての UI イベントを単一の enum に統合したわけではない。
ROM picker、history、補助ウィンドウの navigation、native resize／destroy などは
従来の窓口が残っている。`SessionEvent` をアプリ全イベントの完成形と見なさない。

### 補助窓の状態所有権（追加試作）

初回はTaoのroute tableとGTKの個別`Option`が存在の真実だった。
今回はその重複をなくし、両者が共通の`AuxWindows<N,H>`を使用する。
`N`はnative identity、`H`はadapterの窓controller／GTK resource。
共通側が格納と寿命の判断を所有するが、`H`のnative操作はadapterのみが行う。

- `request_open(kind)` は `Create(id)`／`Present(id)`／`Pending(id)` を返す。
   **native生成前に予約**するので、生成中の再入では窓を増やさない。
- `complete_open(id, native, handle)` は現在の予約だけを受理する。
   古い世代・重複完了・native identity衝突では、既存状態を変えずhandleを返す。
- 作成失敗は `cancel_open(id)`。次回は新しいIDを発行して再試行できる。
- 終了callbackはkindではなくIDを保持して `remove_id(id)` へ。
   古い窓のdestroyが再作成後の窓を閉じず、Settingsのpause所有権も解除しない。
- IDは同一registry内で再利用しない。`clear()` 後も採番を継続する。
- 返されたhandleは`RefCell`借用解除後に破棄する。native callbackの同期再入を許容する。
   `clear()` は直接dropするため、GTKの借用中の破棄には使わない。

Taoの`AuxRegistry`はnative capability adapterに縮退し、独自map・counterを削除した。
GTKはDebugger／History／Settingsをメイン窓の同じregistryへ、PPUをdebuggerスコープの
同じ共通registryへ格納する。GTKのDebugger／PPUは従来どおりcloseで非表示にし、
その場合は存在を保持して再表示する。nativeが実際に破棄された場合にだけ再作成する。

`FocusCoordinator`と`DebugSchedule`も、状態と遷移をTaoから移管した。
Taoの`runtime/debug_windows.rs`は互換re-exportのみとなった。
focus通知順序や250ms live周期は維持し、GTKに新たなauto-pause動作は追加していない。
これは全frontendの振る舞いを強制的に同一化する変更ではない。

### 共通ターンの順序

1. burst 終了時の focus effect を確定。
2. 不変 chrome と最新 tick を1回 capture。
3. menu／title の同期。
4. debugger／live image flight を poll。
5. OSD／modal を pump。
6. native aux／render retry の期限を取得。
7. **poll 後の最新 tick を再読**して wake／redraw を決定。

最後の再読が重要。modal ACK や drain で再開した core に対して、capture 時の
paused 状態を使って待機すると更新が止まる。
flight backstop 用の現在時刻も pump 後に読み、時間のかかった poll の影響を避ける。

### Settings の寿命

- native 作成前に `SettingsOpened`。所有権取得とキークリアを共通側で行う。
- 再 open はregistryの `Present`／`Pending` で生成を抑制する。
   `SettingsOpened`自体も冪等で、重複取得は `settings_changed == false`。
- native 作成失敗は予約を解除し、`SettingsOpenFailed`または世代確認済みの
   `SettingsClosed` callbackで所有権を解放する（同じ解放規約）。
- apply は hold を保持したまま実行し、失敗時も pause を再調停。
- cancel／close／destroy は `SettingsClosed`。重複通知は no-op。
- Modal／Focus／User の所有権は決して巻き添えに解除しない。

GTK の response／close-request／unrealize／destroy は一度限りの完了 callback に接続。
callback の実行前に `RefCell` の借用を解放して、同期的な再入にも対応する。
Tao の native 作成失敗も同じ共通終了規約に戻す。

### Drain と OSD の重要な規約

- enqueue 時の epoch と snapshot を保持。置換された core の待機 batch は開始しない。
- flight の poll 前後でも epoch を確認し、古い表示を公開しない。
- epoch 無効化は、worker がすでに受理した操作の巻き戻しではない。
- 前の write／step が終わる前に次の pause 所有権変更を先行させない。
- modal queue が空になるまで Modal 所有権を保持し、ACK で次を表示するときは解放しない。
- 描画先交換時は `OsdLayerSync::invalidate`。settings 世代とは独立した寿命であり、
  settings の変更がなくても新しい描画先へ layer／strip を送り直す。

## 残る制限と次段階

**今回共通化したのは OS loop そのものではなく、その上の turn／policy**。
Tao は共通 deadline を `ControlFlow::WaitUntil` に反映する。
GTK は既存 frame clock を維持し、deadline の GLib timer 化はまだ行わない。
GTK の debugger は別窓の timer で共通 `DrainQueue` を進めるため、main turn の
debug port は no-op。menu／title の周期、focus／aux lifetime も完全には揃えていない。
focus調停の実装は共通化したがGTKには従来どおりauto-pauseを適用しない。
File picker・About・debuggerのnavigation／入力状態、modalのnative flight格納は未統合。
最小化・非表示時の通知配送／wake 遅延の改善を検証できたとは主張しない。

大幅削減に進むなら、次の順序を推奨する。

1. **共通 application event/effect**: open／close／history／debugger／settings／focus／
   native lifetime を含め、native callback が共通 dispatcher に配送するだけの形へ。
   toolkit の raw event や widget を enum に混ぜず、意味と geometry のみ渡す。
2. **残る実行・表示状態の所有権を共通化**: 補助窓のkind・存在・開閉予約は実施済み。
   次はdebugger navigation／入力検証・File picker／About・modal flight格納を対象にする。
   native handleの表現と配置はadapterに残し、bridge／UI依存は共通側へ持ち込まない。
3. **wake adapter**: `wake_at(Option<Instant>)` と UI-thread `post(event)` の小さな port を作り、
   GTK の main／debugger／live follow を共通期限で動かす。native thread からは enqueue のみ。
   idle 時の起床数と非表示時の ACK／通知遅延を計測する。
4. **View の最終境界**: 共通の表示 snapshot と差分反映の契約を定める。
   settings は既存 observable VM を維持し、各 widget の配置と binding だけ native に残す。
   画面全体の仮想 widget tree を新設することは、この検証からは正当化できない。

単に関数を共通ファイルへ移すだけでは、Tao/GTK が保持する調停状態は消えない。
次の合格条件は行数だけでなく「新しい frontend に drain／modal／settings の状態機械を
再実装させない」「View が起床順序を決めない」「同じ event trace が同じ effect を返す」。

## 検証

Linux／Xvfb で実施。共通ホスト57件、Tao286件、GTK64件、**合計407件が成功**。
共通ホストには Tao から移した既存 frame／focus／debug scheduleテストを含む。
GTK は初期化スレッド制約に合わせ、64件を各1プロセスで実行し、display skip は0件。
設定の重複終了／失敗 rollback、実 NES の差し替え、古い drain の破棄、順序保証、
layer 再送、modal ACK、最新 tick による wake 判断を検証している。
追加試作では、native生成中の再入抑制、失敗後の再試行、native ID衝突、
古い完了／closeの拒否、非表示PPUの再利用、native破棄後の再作成、
Settings終了時のModal所有権維持も検証した。

- `cargo test --offline -p nerust_frontend_host -p nerust_tao --lib`
- `cargo test --offline -p nerust_gtk --lib --no-run` の artifact を Xvfb 上で各 test ごとに
  `--exact <test-name> --nocapture` で実行。Cargo 外から動的リンクの artifact を動かす場合は
  Rust target-libdir、sysroot/lib、debug/deps を `LD_LIBRARY_PATH` に含める。
- `cargo clippy --offline -p nerust_frontend_host -p nerust_tao -p nerust_gtk --all-targets -- -D warnings`

この検証は全 renderer での実ゲーム操作、macOS／Windows の native UI、
消費電力や event latency の性能保証を含まない。

## 規模の再計測

`python3 gui/frontends/host/measure.py` で変更前 commit と現在のソースを比較できる。
`--base <revision>` で比較元を変更可能。
物理行数（コメント、空行、テストを含む）であり、短く書き直して削減率を稼ぐものではない。
新規 host の行数と合計も表示するため、移動をリポジトリ全体の削減と誤認しない。

比較元: `77a6d7b1a2260f10d1fa5ac0d59cd5b386cef54c`。
各 `src/**/*.rs` の全ファイルを比較した結果:

| 対象 | 変更前 | 変更後 | 増減 | 削減率 |
| --- | ---: | ---: | ---: | ---: |
| Tao | 17,835 | 16,964 | -871 | 4.9% |
| GTK | 8,081 | 8,348 | +267 | **3.3%増** |
| 新規共通 host | 0 | 3,156 | +3,156 | — |
| 合計 | 25,916 | 28,468 | +2,552 | **9.8%増** |

native frontendからは合計604行（2.3%）減ったが、回帰テストと新しいadapter／契約を
加えたため、**共通側を含めた総ソースは増えている**。
初回試作はnative側614行減／合計1,667行増だった。今回の状態移管では、
Taoがさらに332行減り、GTKはadapterと回帰テストで342行増え、共通側は875行増えた。
したがって状態所有権の統合は前進したが、この追加試作を行数削減の成功とは評価しない。
この時点で「保守対象の総量が減った」「大幅に薄くできた」とは結論しない。
残る大きな領域はsettings／debuggerのwidget構築・bindingと入力変換、native lifecycle／
renderer結線。次は共通表示状態とevent/effect境界を詰め、既存adapterも本当に消せるかを
同じ方法で再計測する。