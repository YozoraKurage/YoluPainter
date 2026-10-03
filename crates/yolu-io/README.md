# yolu-io

Unity 版 `.ylp` の形式1〜6と `document.utpaint` の版1〜21を読み書きする。描画用の `yolu-core` には依存しない。

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
- `with_value` は既存項目の差し替え後に正本全体を再検証する。版や個数だけを変更して不整合になった場合は断る。レイヤーを追加する文書ビルダーや描画coreへの変換はまだない。
- `Project::to_bytes` は元の形式を保ち、エントリ内容とmanifestをバイト一致で再保存する。ZIPの時刻・圧縮結果・追加フィールドは一致の対象外。
- `migrated_entries` は形式6の並びへメモリ上で移行したエントリ。`ylp.json` は除き、元の形式と書いたアプリは `info()` で取得する。
- `upgraded(writer)` は明示的に形式6へ更新する。正本の版は変えない。未知のエントリ・JSONキーも保持する。知らないエントリは `unknown_entries()` と `notes()` で知らせる。
- `Archive` は外側だけを検証する低水準API。各正本やリソースの検証まで必要な場合は `Project` を使う。

## リソースと未対応の中身

画像はRGBA8のPNGを復号し、寸法と下の行からの画素ハッシュを検証する。スマートマテリアル・スマートマスク・マテリアル・ブラシは埋め込まれたファイル全体を保持する。内側のmanifest、正本、種別、チャンネル、画像、ブラシのschema・画像参照も検証する。

描画coreへの変換、効果の実行、ブラシ設定の描画上の意味、PSD原本・合成PNG・メッシュマップ・画面設定の復号は未実装。これらは読み飛ばして破棄せず、原本のエントリとして残す。`notes()` を呼び出し側の画面に提示できる。画像リソース以外のPNGをすべて復号するAPIではない。

形式7以降、正本22以降、未知の列挙値・アルゴリズム版は理由を添えて断る。ZIP64・暗号化・分割ZIP、RGBA8以外のリソースPNGは扱わない。正本とZIPの1エントリは512 MiB、ZIPの展開総量は768 MiB・1000エントリ、復号リソースも768 MiBまで。これらはデータ量の上限であり、プロセスの最大メモリ使用量の保証ではない。

## 保存

`SaveTarget::open` はSHA-256・長さ・更新時刻を記録する。`create` は新規保存専用。保存ではメモリで検証し、保存先と同じフォルダの一時ファイルに書いて `sync_all`、読み直して検証、外部改変を再確認してから `rename` で一度だけ確定する。置換に失敗しても削除してから移動する手順には切り替えない。

直前の版は `<ファイル名>-backups~/` にSHA-256名で残し、自動削除しない。保存先ごとの排他的なロックファイルを作り、通常終了・エラー時には片付ける。プロセス強制終了後のロック回収は呼び出し側で扱う必要がある。非協調プロセスが最後の検査と置換の間に書く競合、電源断時のディレクトリ永続性、Windows実機での保存は保証・検証の対象外。

## 検証

```sh
cargo test --workspace
cargo clippy -p yolu-io --all-targets -- -D warnings
```

Unity由来の形式1〜6、C#で検証した正本1〜21、C#書き手の拡張正本21と選択範囲で互換性を確認する。フィクスチャの由来と再生成方法は [fixtures/README.md](tests/fixtures/README.md) を参照。

直接依存する flate2、crc32fast、sha2、serde、serde_json、png はMITまたはApache-2.0。解決済みの間接依存にはMIT・Apache-2.0・Zlib・Unicode-3.0などの許諾が含まれる。
