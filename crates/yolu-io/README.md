# yolu-io

Unity 版 `.ylp` の形式1〜7と `document.utpaint` の版1〜21を読み書きする。M1のラスターColor文書は `yolu-core::Document` と相互変換できる。

```rust,no_run
use yolu_io::{NativeValue, SaveTarget};

let (project, mut target) = SaveTarget::open("sample.ylp")?;
let set = &project.sets()[0];
let document = set.document.with_value(
    "layers[0].name",
    NativeValue::Text("背景".into()),
)?;
let changed = project.with_document(&set.id, &document)?;
target.save(&changed)?;
# Ok::<(), yolu_io::Error>(())
```

## 読み書き

- `Project::read` はZIPのCRC・manifestのSHA-256・長さ・名前・展開予算を検証し、テクスチャセット、正本、選択範囲、リソースを読む。
- `NativeDocument::fields()` はディスク順の型付き項目を返す。例: `layers[0].opacity`、`layers[0].channels[0].tiles[0].rgba`。整数、真偽値、倍精度数、GUID、UTF-8文字列、画素のバイト列を区別する。GUIDのバイト列はC#の `Guid.ToByteArray()` 順。
- 正本にはチャンネルとタイル、マスク、調整、フィルター、Generator、塗りつぶし画像・投影・デカール・グラデーション、2D/3Dパス、Anchor、手動ID色を保持する。`to_bytes` は各値を元の順で再符号化し、浮動小数点演算や画素変換をしない。
- `with_value` は既存項目の差し替え後に正本全体を再検証する。版や個数だけを変更して不整合になった場合は断る。層・タイルの編集は対応範囲を検査したうえでcoreのAPIを使える。
- `Project::to_bytes` は元の形式を保ち、エントリ内容とmanifestをバイト一致で再保存する。ZIPの時刻・圧縮結果・追加フィールドは一致の対象外。
- `migrated_entries` は形式7の並びへメモリ上で移行したエントリ。`ylp.json` は除き、元の形式と書いたアプリは `info()` で取得する。
- `upgraded(writer)` は明示的に形式7へ更新する。正本の版は変えない。未知のエントリ・JSONキーも保持する。知らないエントリは `unknown_entries()` と `notes()` で知らせる。
- 形式7の `TextureSet::material` は `MaterialRef`（名前と任意のアセットGUID/符号付き64 bitの`fileId`、未割当、旧スロット参照）。名前だけなら重複可、アセット・未割当・スロットは排他的。旧 `materialSlot` はメモリ上で `material: {"slot": …}` へ移行する。`with_material` は形式7の参照を検証して変更する。
- `Archive` は外側だけを検証する低水準API。各正本やリソースの検証まで必要な場合は `Project` を使う。

## coreとの変換・合成PNG

`NativeDocument::to_core()` はM1のラスター層、有効なColorのタイル、名前、表示、不透明度、26合成モード、クリッピングを変換する。文書・層のIDと透明画素のRGBを保ち、読み込み操作をUndo履歴に残さない。

`core_issues()` は扱えない項目のパスを返す。マスク、別チャンネル、無効なColor、塗りつぶし・調整・グループ、親、ロック、チャンネル別合成、パス、フィルター、Anchor、手動ID色、既定値以外のノーマル設定は変換を拒否する。非表示・無効でも捨てない。タイル外周の非ゼロ余白、coreの画素予算超過も変換時に理由を添えて断る。正本の読み書きは引き続き可能。

`NativeDocument::from_core(&document)` は正本21を作る。寸法は8192以下、タイル寸法は8〜512の2の累乗。M1以外のチャンネルや描画中のストロークは保存を拒否する。旧版や空タイルのディスク配置まで再現するAPIではない。原本の版・項目・並びをバイト一致で戻すときは、元の `NativeDocument::to_bytes()` を使う。

```rust,no_run
use yolu_io::{NativeDocument, Project};
let project = Project::read(&std::fs::read("sample.ylp")?)?;
let set = project.sets().iter().find(|s| s.id == project.current_set()).unwrap();
let mut core = set.document.to_core()?; // 対応外の項目があれば、編集を始める前に断る。
core.add_layer("新しい層")?;
let native = NativeDocument::from_core(&core)?;
let changed = project.with_document(&set.id, &native)?; // 他セット・リソース・未知エントリを保持。
# Ok::<(), yolu_io::Error>(())
```

現在のセットのColor合成をPNGにする例（出力先は新規ファイル）:

```sh
cargo run -p yolu-io --example composite_png -- sample.ylp output.png
```

`composite_png(&core)` は下原点のRGBAをPNGの上からの行へ並べ、Unity版 `RgbaPng` と同じ適応フィルター・チャンク配置で出力する。flate2のzlibバックエンド（libz-sys）を使う。合成26モードと圧縮用パターンの人工データ計27件でC#出力とPNG全バイトが一致することを検証している。圧縮結果はdeflate実装の版にも依存し、すべてのランタイム・画像での一致を保証するものではない。

## PSD・PSB

`psd` はUnity版と同じRGB8 PSDコーデック。`psd::Document`・`Layer`・`Mask`・`Adjustment` はcoreの文書に依存しないデータ型で、レイヤーとRGBAの行は上から下。名前、正の一意なPSDレイヤーID、キャンバス外の画素、透明画素のRGBを保持する。

```rust,no_run
use yolu_io::psd::{self, CompatibilityMode, Limits};
let limits = Limits::default();
let origin = psd::read(&std::fs::read("sample.psd")?, &limits)?;
for note in origin.diagnostics() {
    eprintln!("{}: {}", note.code, note.message);
}
if origin.mode() == CompatibilityMode::EditableRaster {
    let mut document = origin.to_core()?; // M1以外の情報があれば理由を添えて拒否。
    document.add_layer("新しい層")?;
    let edited = psd::Document::from_core(&document)?;
    let bytes = psd::write_edited(&origin, &edited, &limits)?;
    // bytesを新規出力または外部改変検知・一時ファイル置換を備えた保存処理へ渡す。
}
# Ok::<(), yolu_io::Error>(())
```

| 要素 | 読み取り・書き出し |
|---|---|
| PSD v1 RGB8、raw / PackBits RLE | 読める。書き出しはraw。統合画像は白背景のRGB＋元の透明度を格納 |
| ラスター、26合成モード、クリッピング、表示・不透明度、Unicode名・ID | 読み書き。ID欠落・不正・重複を名前で補修しない |
| ラスターマスク | 矩形・既定値0/255・有効/無効・濃度・画素を保持 |
| グループ | 入れ子、通過・分離、マスク、グループと区切りのIDを保持 |
| 調整 | 反転、RGB一括のレベル補正、マスターの色相/彩度。PSDの整数刻みで保持。チャンネル別補正・Colorize等は保護 |
| 単色塗りつぶし `SoCo` | RGBの色を保持。8bit未満の端数は丸めて通知。画素キャッシュを使わないことも通知 |
| レイヤーロック `lspf` | 透明・画素・位置・全体を保持。対応のないビットは通知 |
| 描画に関係しない既知メタデータ、sRGB IEC61966-2.1、1:1ピクセル比 | 編集可能として受理し、書き出しに含まれない情報を `NotCarriedIntoExport` で通知 |
| PSB、RGB8以外、ZIP圧縮、未知タグ・調整、効果、テキスト、スマートオブジェクト、ベクターマスク | `PreserveOnly` と理由を返し、原本全体だけを保持。PSBの復号・新規生成はしない |
| 構造破損・予算超過 | `Rejected`。原本保持上限内なら原本バイトは残る |

`psd::read` / `read_stream` は `ReadResult` を返す。編集可能な場合にだけ `document()` が値を持つ。`original_bytes()` / `copy_original_bytes()` は元のファイル全体で、原本保持予算を超えたときだけ保持しない。未知情報を編集後のファイルへ部分的に継ぎ足す方式ではない。**取り込んだ原本の編集保存には `write_edited` を使う**。`PreserveOnly`・`Rejected` はそこで拒否する。`write` は新しく作ったPSDスナップショット用。

原本コピーはバックアップ・保全のためで、外部で変更されたファイルを上書きする許可にはならない。PSD APIはファイルシステムを操作しない。保存先の外部改変検出・バックアップ・最後の一度の置換は呼び出し側の責務で、.ylp用の `SaveTarget` をPSDには使えない。

`ReadResult::to_core` と `psd::Document::to_core/from_core` はM1のラスターColor、26合成モード、クリッピング、表示、不透明度を変換する。読み込み時にUndo履歴を残さず、PSDのIDはcoreの永続IDに埋め込んで再出力する。同名レイヤーもIDで区別する。M1が保持できないグループ・調整・塗りつぶし・マスク・ロック、キャンバス外の画素、非Color/無効Colorは変換を拒否する。`core_issues()` で拒否する項目を確認できる。coreからの出力矩形はタイルの範囲になり、不透明度は最も近い1/255へ丸める。原本の圧縮や矩形までバイト一致で戻すAPIではない。

既定の `Limits` は原本・出力各128 MiB、寸法8192、キャンバス16,777,216画素、256レイヤー記録（区切りを含む）、復号256 MiB、メタデータ4 MiB、名前4096 UTF-16単位、診断128件、グループ32段。書き出しは確保前に構造・画素・出力長を検証する。全体をメモリで扱うため、ストリーミング型の大規模文書向けではなく、これらはプロセス全体の使用メモリの保証ではない。

統合画像は参照合成と比較する。通常合成だけで大きく違えば編集を止め、合成モード・マスク・グループ・調整等があれば `CompositeDiffers` で見え方の差を知らせる。参照合成はYoluPainterの式であり、Photoshop/CSPの見え方の再現を保証しない。C#由来の人工入力2,337件で互換モードと原本保持、拒否以外の診断コードを照合し、481件で書き戻したPSDの全バイトが一致することを確認している。再生成・検証の条件は [PSDフィクスチャ](tests/fixtures/psd/README.md) を参照。

## リソースと未対応の中身

画像はRGBA8のPNGを復号し、寸法と下の行からの画素ハッシュを検証する。スマートマテリアル・スマートマスク・マテリアル・ブラシは埋め込まれたファイル全体を保持する。内側のmanifest、正本、種別、チャンネル、画像、ブラシのschema・画像参照も検証する。

M1以外の効果の実行、ブラシ設定の描画上の意味、.ylp内のPSD原本・合成PNG・メッシュマップ・画面設定の自動復号は未実装。PSD単体は下記のAPIで読み書きできる。これらは読み飛ばして破棄せず、原本のエントリとして残す。`notes()` を呼び出し側の画面に提示できる。画像リソース以外のPNGをすべて復号するAPIではない。

形式8以降、正本22以降、未知の列挙値・アルゴリズム版は理由を添えて断る。ZIP64・暗号化・分割ZIP、RGBA8以外のリソースPNGは扱わない。正本とZIPの1エントリは512 MiB、ZIPの展開総量は768 MiB・1000エントリ、復号リソースも768 MiBまで。これらはデータ量の上限であり、プロセスの最大メモリ使用量の保証ではない。

## 保存

`SaveTarget::open` はSHA-256・長さ・更新時刻を記録する。`create` は新規保存専用。保存ではメモリで検証し、保存先と同じフォルダの一時ファイルに書いて `sync_all`、読み直して検証、外部改変を再確認してから `rename` で一度だけ確定する。置換に失敗しても削除してから移動する手順には切り替えない。

直前の版は `<ファイル名>-backups~/` にSHA-256名で残し、自動削除しない。保存先ごとの排他的なロックファイルを作り、通常終了・エラー時には片付ける。プロセス強制終了後のロック回収は呼び出し側で扱う必要がある。非協調プロセスが最後の検査と置換の間に書く競合、電源断時のディレクトリ永続性、Windows実機での保存は保証・検証の対象外。

## 検証

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Unity由来の形式1〜6、C#で検証した正本1〜21、C#書き手の拡張正本21・選択範囲・形式7のM1合成27件で互換性を確認する。フィクスチャの由来と再生成方法は [fixtures/README.md](tests/fixtures/README.md) を参照。

直接依存する flate2、crc32fast、sha2、serde、serde_json、png と、zlib接続用の libz-sys・vcpkg はMITまたはApache-2.0。解決済みの間接依存にはMIT・Apache-2.0・Zlib・Unicode-3.0などの許諾が含まれる。
