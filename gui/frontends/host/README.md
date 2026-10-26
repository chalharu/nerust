# 薄いネイティブ View のための共通ホスト・プロトタイプ

## 結論

Tao／GTK の**実際の実行経路**で、イベント配送、ループの1ターン、drain 調停、
OSD 結線、settings の寿命管理を toolkit 非依存にできた。
さらに補助窓の存在・開閉予約・経路・世代IDと、focus／保留要求／live周期の
**状態所有権も共通ホストへ移した**。
追加検証ではdebuggerのnavigation／入力／watch／freeze／bookmarkの状態とreducerも
共有し、GTKの共通期限をGLibの起床に接続した。

**設計判断は「共通ホストが状態とevent/effect・ターン・期限を所有し、nativeは
OS pump・配置・binding・副作用の実行を担当する」を採用する。**
単一のOSメッセージポンプや汎用widget treeへの統合は採用しない。
ただし、**この方式だけではフロントエンド全体の大幅削減にはならない**。
全Rust物理行数はnative側合計5.3%減、共通側込みでは13.9%増となった。
制御の重複排除と「総保守コード量の削減」は別の評価であり、後者の成功とは判定しない。
これは採用境界と限界を確認した動作するプロトタイプで、完成済みの全画面移行ではない。

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
| `debugger` | navigation／入力／bookmark／watch／freeze／選択の状態、共通reducer、action batch／再構築範囲、owned model／split-lock bridge | widget配置／binding、入力文字列のcapture、窓生成、drain transportと描画 |
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
debuggerは別の`DebuggerEvent`を共通reducerへ渡す。
ROM picker、historyの一部操作、native resize／destroy などは従来の窓口が残っている。
意味の異なるイベントを巨大な単一enumへ入れること自体は目的にしない。

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

### Debugger の表示状態と入力判断（最終試作）

Taoのbridge／updateとGTKの`DebuggerData`／個別handlerに分散していた判断を、
共通の`DebuggerState::reduce`へ移した。更新結果はactionの順序付きbatch、
`DebuggerScope`、PPU open effectで、native側がtransport／部分再構築／窓生成を実行する。
Taoの`aux/bridge.rs`は5行のre-export、`aux/debug/update.rs`は8行の配送adapterとなった。
GTKではwidget配置・signal接続は残すが、アドレス／byte入力の検証や操作判断を持たない。

- GTKは単一所有の`DebuggerModel`、Taoは共通`DebugBridge`のsplit-lock storageを使う。
   reducerは同じで、別々の状態遷移実装は持たない。
- 入力echoはdrainもlist再構築も発生させない。画像をsmall stateから分離し、
   typing時は画像バッファをcloneしない。bridgeは変更のあるdisplay patchのみCOWする。
- `SelectRow`＋`EditStage`は同じoutbox lock内にまとめ、consumerやepoch再bindで分割しない。
- 無効epochの操作はlocal-onlyな入力／bookmark／選択も含めて拒否する。
   GTKはwidget入力のcapture前に置換を検出し、古い操作を実行せずRefreshだけを送る。
   workerがすでに受理した操作の巻き戻しまでは保証しない。
- GTKの部分再構築は画像を除くowned snapshotで行い、widget通知中にcontrollerを借用しない。
   live image更新やheader-onlyの入力エラーではlist／scrollを作り直さない。
- Freeze解除はlocal状態と状態ラベルを更新する。既に書き込んだbyteのundoではない。

従来の差異は明示的に整合した。bookmarkはアドレス重複なし・上限8件・空labelはBM番号。
上限到達後の重複追加もno-op。follow中はPC行、なければ先頭可視行、pin中は常にpinを使う。
旧Taoの重複許容／pinより可視PC優先は維持せず、共通／nativeテストで新しい契約を固定した。
入力前後の空白も許容する。振る舞いが元から完全一致していたとは扱わない。

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

Taoは共通期限を`ControlFlow::WaitUntil`へ、GTKは単一のGLib one-shotへ渡す。
GTKでは早い予約を後続frameで延期せず、期限なしで取消、発火前にSourceIdを外す。
timeoutは弱参照、窓からのframe callbackは所有参照、Surfaceから窓は弱参照として寿命を閉じる。
非表示／ゼロサイズでもturnは進めるが、rendererのattach／renderは行わない。
借用競合は30ms内に再試行する。描画自体はnative frame clockが担当する。
modalはpump中に取り出し、State／OSD／Surfaceの借用解放後に表示する。
ACKは次batchの表示へつなぎ、最後だけModal所有権を解除する。

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

## 最終判断と適用限界

**今回共通化したのは OS loop そのものではなく、その上の turn／policy**。
GTK の debugger は別窓の timer で共通 `DrainQueue` を進めるため、main turn の
debug port は no-op。250ms live timer、menu／title の周期は完全には揃えていない。
focus調停の実装は共通化したがGTKには従来どおりauto-pauseを適用しない。
File picker・About・modalのnative resource格納は未統合。
Tao bridgeのspaces／language metadataは不変で、異なるspacesへの変更は窓の再作成が必要。
全native callbackの共通化、全frontendの振る舞い統一を完了したとは扱わない。
GTKの非表示中の**既知の期限**の実行は検証したが、期限がない状態へ新着通知を届ける
worker→UI-thread postの全経路は統合していない。通知遅延／消費電力の性能保証は含まない。

判断をまとめると、次のとおり。

1. **状態所有権＋意味イベント＋effectを採用する。** helper移動だけではなく、
   補助窓・調停・debuggerの状態機械を新frontendで再実装しない境界を作れる。
   owned／split-lockのどちらも同じreducerで動くことを確認できた。
2. **OS pumpの一本化を削減の前提にしない。** 共通turn／期限決定とnativeのwake／postを
   分ければ接続できる。GTKのUI thread／frame clockをTaoのloopへ従属させる必要はない。
3. **「極薄」を業務判断がないViewという意味で目指す。** settingsは既存observable VM、
   debuggerは今回の共通modelを使い、nativeはsnapshot反映・signal変換・配置を担当する。
   この意味の薄さは成立するが、nativeコードが小さくなることと同義ではない。
4. **大幅な行数削減の仮説は、この試作では不成立。** 残る大量のwidget配置・binding・
   renderer／native lifetime glueは、event loop共有では消えない。
   さらに物理量を削るなら画面別bindingの生成や宣言的schema等を別途評価する必要があるが、
   汎用virtual widget frameworkの新設を本試作から正当化はしない。

したがって、このプロトタイプは**制御境界の検証として採用、総量削減の達成例としては不採用**。
ネイティブ化ではこの契約を土台に画面単位で移行し、配置コードを無理に共通化しない。
合格条件は「同じevent traceが同じeffect」「新frontendが調停を再実装しない」も含め、
物理行数とは独立して追跡する。追加の切り出しだけで大幅削減できると期待し続けない。

## 検証

Linux／Xvfb で実施。共通ホスト102件、Tao257件、GTK74件、**合計433件が成功**。
共通ホストには Tao から移した既存 frame／focus／debug scheduleテストを含む。
debuggerの純粋ロジック／bridgeテストも共通側へ移しており、Taoの件数減は回帰削除ではない。
GTK は初期化スレッド制約に合わせ、74件を各1プロセスで実行し、display skip は0件。
既存harnessにはGApplication startup前の窓生成に伴うGtk-CRITICAL診断が残る。
設定の重複終了／失敗 rollback、実 NES の差し替え、古い drain の破棄、順序保証、
layer 再送、modal ACK、最新 tick による wake 判断を検証している。
追加試作では、native生成中の再入抑制、失敗後の再試行、native ID衝突、
古い完了／closeの拒否、非表示PPUの再利用、native破棄後の再作成、
Settings終了時のModal所有権維持も検証した。
最終試作では、owned／bridgeのevent trace一致、入力時の無通信／画像非clone、
順序付きwrite batchとepoch再bindの競合、GTKのlocal-only操作の置換拒否、
list／scroll維持、freeze解除ラベルも検証した。
GLibの早い期限の保持／差替え／取消／Drop、非表示中の実OSD期限の消化、
借用競合後の再起床、modal map通知中の再入と連続ACKは実native contextで検証した。

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
| Tao | 17,835 | 15,567 | -2,268 | 12.7% |
| GTK | 8,081 | 8,982 | +901 | **11.1%増** |
| 新規共通 host | 0 | 4,970 | +4,970 | — |
| 合計 | 25,916 | 29,519 | +3,603 | **13.9%増** |

native frontendからは合計1,367行（5.3%）減ったが、回帰テストと新しいadapter／契約を
加えたため、**共通側を含めた総ソースは増えている**。
前段の状態所有権移管commit時点はnative側604行減／合計2,552行増だった。
最終試作ではTaoがさらに1,397行減、GTKは起床adapterと回帰テスト等で634行増、
共通側は1,814行増となった。テストの移動／追加も含むため、production-only削減率ではない。
テストを除いた率をこの表から推定せず、所有する責務の統合とは分けて評価する。