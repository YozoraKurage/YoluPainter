# 文書の層の操作

`Document` の編集は単独の書き手から行います。ストローク中は構造の編集・変形・サイズ変更・Undo/Redo を断ります。

- `set_layer_locks` / `change_layer_locks`: 透明部分・画像・位置・すべてのロック。親グループのロックも効きます。拒否は `CoreError::LayerLocked` に対象とロックの持ち主を返します。
- `topmost_of`: 複数選択の重複と、選んだグループに含まれる子を除きます。削除・複製・表示・移動・一段移動は複数でも一回の Undo です。
- `transform_layer` / `transform_layers`: 全チャンネルとマスクを移動・回転・拡大縮小します。選択範囲があると、その部分だけを持ち上げ、選択範囲も同じ段で移動します。`transform_layer_region` は明示範囲との交差を移し、選択範囲自体は動かしません。
- `resize_image`: 最近傍・双線形・面積平均で画像と選択範囲を変更します。透明画素の RGB と法線の正規化を保ち、Height → Normal の強さを縮尺に合わせます。`resize_canvas` は画素を補間せず、指定位置へ置いて画布の外を切り落とします。どちらも `ResizeReport` を返し、層・マスク・選択範囲を同じ道（タイルごとに、読む元が無ければ飛ばし、全部が同じ一様な色なら計算せず埋める。まとまりごとに取消を確認）で作ります。
- `merge_down` / `merge_visible` / `merge_group` / `merge_layers`: 結合した結果と元の合成を比較し、許容差を超える場合は拒否します。`LayerMergeReport` は比較・変化した画素数と最大の保存値の差・見える差を返します。許容差の既定相当は `Document::MERGE_ROUNDING_TOLERANCE`（2）、255 はすべての差を許します。

画素を書く入口は、書く前に `Document::pixel_write_guard`（ロックで断り、透明部分のロックでアルファを守るかを返す）を通し、返した値を `StrokeState::new` の `keep_alpha` か塗りつぶしの式（`fill_pixel`）へ渡します。どちらも `keep_alpha` は必須の引数なので、新しい入口が渡し忘れるとコンパイルが落ちますが、守れるのはそこまでです（`false` を書く入口や、`edit_material_region` のクロージャで引数を無視する入口は通り、`edit_region`・`edit_region_tiles` は関門を通さずに呼べます）。入口を足し忘れても見つける仕組みは無く、後述の `write_entries` の表に足すことだけが防波堤です。断るのは、無効のチャンネルを有効にしたり面を作ったりする前です（断ったあとに何も残りません）。マスクへ書く入口はすべてのロックだけで断ります（`refuse_lock`）。

検査の順は C# と同じです。画素の入口は、ラスターの層かどうかの型の検査がロックより先で、断ったときにロックの名指しは出ません（`begin_stroke` の 4 入口だけは、調整・グループの層ではロックが先です。C# の `BeginStroke` が面を取る `GetChannel` の型の拒否をロックの検査のあとに置くためで、塗りつぶしの層だけ型が先です）。無効のチャンネルの拒否は、単チャンネルの範囲の塗り（`fill`・`gradient`・`begin_triangle_fill`）ではロックより先、単チャンネルのストロークではロックのあとです。マスクへ書く入口とマスクを変える入口（`set_layer_mask_*`・`remove_layer_mask`）は、マスクの有無の確認がロックより先で、マスクの無い層はどのロックでも「マスクが無い」で断ります。`add_layer_mask` は逆に、もうマスクがあるかの確認が先です。

| 入口 | 検査 |
| --- | --- |
| `begin_stroke` / `begin_stroke_in` / `begin_brush_stroke` | `pixel_write_guard` |
| `begin_material_stroke` / `begin_material_brush_stroke` | `pixel_write_guard`（全チャンネルが `keep_alpha`） |
| `begin_triangle_fill` / `begin_material_triangle_fill` | 上の入口を通り、返った `keep_alpha` を三角形の式へ |
| `fill` / `fill_material` / `gradient` / `gradient_material` | `pixel_write_guard`（範囲の式は `fill_pixel`） |
| `begin_mask_stroke` / `begin_brush_mask_stroke` / `begin_mask_triangle_fill` / `fill_mask` / `gradient_mask` / `apply_smart_mask` | `refuse_lock(ALL)` |

`tests/docops.rs` の `every_pixel_write_entry_goes_through_the_layer_locks` が、入口ごとに（層自身のロックと親グループのロックで）断ること・断ったあとに何も変わらないこと（履歴・変更番号・全チャンネルの有効と画素・マスク）・ロックを外せば同じ入口が通ることを確かめます。入口を足したら `write_entries` へ足します。

透明部分のロックは描画・塗りつぶしのアルファを固定し、アルファ 0 の RGB もそのまま残します。消去と選択部分の変形は拒否します。画像ロックでも層全体の整数画素の移動とマスクへの描画は可能です。位置ロックは変形を拒否します。全ロックでも名前・表示・層順・複製・削除は可能です。読み込み用の `import_tile` / `set_channel_pixel` はロック検査を迂回するため、読み込みの最後に `set_locks_for_load` を使います。

原子的な複数操作は、タイルを共有する準備用の状態（`Document::edit_copy`）で計算し、成功した状態だけを交換します（`State`）。Undo は再計算せず元の状態へ戻します。`edit_copy` と `State::take` は `Document` の項目を `..` なしで全部挙げ、準備用へ写さない・交換しない項目には理由を書いてあります。`Document` に項目を足すとここでコンパイルが落ちるので、準備用の文書だけが変わって本物に反映されない、ということが黙って起きません。交換するのは層・寸法・Normal の設定・選択範囲だけで、チャンネルの表・履歴・進行中のストローク（マテリアルのストロークと三角形の塗りを含む）・変化の記録・予算・手動の ID 色は本物の文書のものです（ID 色は層の操作・変形・サイズ変更で変わりません）。

変化の記録（`changed_tiles`）の印は、交換が変え得る層だけに前後の両方で付けます（`Dirty`）。ロックだけの変更は何も付けず（単独の `set_layer_locks` と同じ）、削除・複製・表示・一段移動・結合・変形は動いた層（グループは中身ごと）、サイズ変更は全層です。クリッピングの組は、層が 1 つでも動けば付けます。画素予算と一操作の予算は別です。変形は対象タイルの元の格納量 + タイルごと 64 バイト、結合は結果の格納量 + タイルごと 64 バイトを一操作の予算に数えます。層の複製は共有した画素も画素予算に数えます。取消可能な変形・画像サイズ変更はタイルのまとまりごとに取消を確認します。

C# の `Resampled` は履歴なしの別文書を返しますが、Rust のサイズ変更は寸法・画素・選択範囲・設定を一回の Undo で戻すため、前後の格納量（+128）を履歴の費用に数えます。この費用は文書の 2 倍以上になるので、履歴の予算（既定 64 MiB）に入らないことがあります（4096² の 1 層を 2048² へ縮めると前後で約 80 MiB）。そのときも断らず、そのサイズ変更の 1 段だけは予算を超えて残し（古い段を落とす）、`ResizeReport::history_over_budget` で知らせます。残るのはその段が最新の間だけで、次の編集の整理では普通の段として落とし得ます。断る案（4096² 級の文書が既定の予算で大きさを変えられなくなる）と、黙って Undo を落とす案（サイズ変更の取り返しが付かなくなる）は採りませんでした。画素の予算を超える・取り消すときは、履歴を落とさずに断ります。解像度変更自体は C# と同様に層のロックを無視します。幅・高さは 1〜8192 です。

層のロックを保存する際は I/O 側で `locks()` と `set_locks_for_load` の接続が必要です。文書に保持していないパス・層の効果スタックの拡大や焼き込みは、これらの操作の対象に含みません。

照合は `tools/csharp-golden/run.sh docops` で実 C# Core の人工データを生成し、`cargo test -p yolu-core --test docops_golden` で実行します。582 事例を、並列度 1 と 4 で全バイト比較します。生画素・層の属性・親子・マスク・選択範囲・合成・結合報告・履歴バイト数・Undo/Redo を比べますが、画像サイズ変更（`resize`・`selected_resize` の 108 事例）は C# の `Resampled` が履歴なしの別文書を返すため、変更直後の画素・属性だけです（サイズ変更の Undo・履歴・予算は `tests/docops.rs` と `resize.rs` の試験が見ます）。結合がロックで断られる・断られない 34 通り（`mergelock`。断った層・ロックの持ち主・ロックと、断ったあとの文書が変わらないこと、親グループの継承を含む）と、動く画素の外接矩形 `transform_bounds`（`bounds`・`selected_bounds`）も C# と比べます。

ロックを立てた層への塗り（マテリアルのストローク・範囲の塗り・グラデーション 2 種・三角形の塗り・単チャンネルのグラデーション・三角形の塗り・ストローク、マスクへの 4 種）は `tools/csharp-golden/run-material.sh` が書く `tests/golden/material.txt` の `lock`（1024 事例）・`lockmask`（512 事例）を `cargo test -p yolu-core --test material_golden` が並列度 1 と 4 で全バイト比較します。断った層・ロックの持ち主・ロック、断ったあとの Undo/Redo の可否と全チャンネルの有効・画素、通ったときの Undo/Redo の後の画素まで見ます。層の画素は、透明な画素がチャンネルごとに違い（アルファの式にチャンネル番号が入る）、文書の選択範囲を立てた事例、無効にしたチャンネルへ書く単チャンネルの事例、マスクの無い層の事例を含みます。塗りつぶし・調整・グループの層への書き込みで、型の拒否とロックの拒否のどちらが先かは `locknr`（576 事例。並列度 1 だけ）が照らします。

この照合の射程の外: 予算の拒否（ストローク・画素・履歴）は C# と照らしません。巻き戻しの数え方が C# と違う（Rust はチャンネルごとに覆いを持つ。`rollback` の行が C# の値と倍率を固定する）ため、ロックの下で予算が尽きる境目は C# と一致せず、Rust の試験（`locked_writes_refused_by_the_stroke_budget_leave_everything_as_it_was` ほか）が、断ったあとに元へ戻ることだけを見ます。マスクを持つ層への `apply_smart_mask`・手動の ID 色（層の操作で変わらないことは `manual_id_colours_survive_layer_operations_transforms_and_resizes`。C# の `Resampled` は持ち越さない）も、C# との全バイトの照合には入っていません。

計測は `cargo run -p yolu-core --release --example docops_bench` と `tools/csharp-golden/run.sh docops-bench` で、人工二層の 512² / 1024²、各三回の中央値を出します。疎な層（8192²・描いたタイルが 1 枚、残りは無いか一様）のサイズ変更は `-- --sparse` です（計測の値は STATUS に書きます）。
