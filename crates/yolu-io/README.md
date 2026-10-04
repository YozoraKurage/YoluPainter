# yolu-io

Unity 版 `.ylp` の形式1〜7と `document.utpaint` の版1〜21、Rust 版が足した版22（ユーザーチャンネル。下）を読み書きする。層（ラスター・塗りつぶし・調整・グループ、マスク、クリッピング、チャンネルごとの合成）の文書は `yolu-core::Document` と相互変換できる。

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
- `Project::create(writer, sets, current)` は形式7の新しいプロジェクトを作る（各セットは `SetSpec` で、ID・名前・`MaterialRef`・正本・使っているチャンネルごとの合成PNG）。`with_sets(writer, sets, current)` は形式7のセットの並び・名前・マテリアル参照・現在のセットを置き換え、正本と合成を差し替える。元のセットは全部が並びに要る（セットを消す口は無い）。正本が `None` のセットはエントリをバイト列のまま残し、正本を替えたセットの `composite/` は消してから渡されたPNG（`composite_pngs` の出力。標準のチャンネルだけ、同じチャンネルは1回）を書く。各セットと根の知らないJSONキー、選択範囲・メッシュマップ・PSD原本・根のほかのエントリは残し、`savedBy` を書き手にする。どちらも書いたものを読み直して検証する。サムネイル（`thumbnail.png`）は作り直さない。
- 選択範囲（`selection.bin`）は `Selection` で読み書きし、`Selection::from_core` / `to_core` でcoreの `SelectionMask` と行き来する。書くバイト列はC#の `SelectionBinary.Write` と同じ（量のあるタイルだけを (y, x) の順に並べる）。`Project::with_selection(set_id, Some(&selection))` はセットの選択範囲だけを差し替え、`None` は選択なし（エントリを消す）。正本と大きさが違う選択範囲は再検証で断り、ほかのエントリには触らない。読み込み直後の文書へは `Document::restore_selection` で戻す（Undoの段は増えない）。
- `Project::mesh_map(set_id, kind, max_bytes)` はセットのメッシュマップ（`meshmap-<種類>.bin`、版1〜3）を予算付きで読む。形式1・2は根のエントリ、形式3以降はセットの下。`with_mesh_map(set_id, map)` は派生物だけを置き換えて版3で書き、文書と未知のエントリを保つ。書けるのは形式7だけで、旧形式は先に `upgraded` で移行する（形式5・6のメッシュマップは版2・スロット1つのまま移行で変わらず、置き換えたものから版3になる）。
- `Archive` は外側だけを検証する低水準API。各正本やリソースの検証まで必要な場合は `Project` を使う。

## coreとの変換・合成PNG

`NativeDocument::to_core()` は C# の `DocumentBinary.Read` と同じ意味で、正本をcoreの文書にする。

| 正本 | core |
|---|---|
| 層の種類（ラスター・塗りつぶし・調整・グループ）、入れ子、通過・分離（合成モード） | 層の並びと親。ID・名前・表示・不透明度・26合成モード・クリッピング |
| ラスターマスク（有効・反転・濃度・画素） | マスク |
| チャンネルごとの面（Color〜Emission とユーザーチャンネル）と有効の印、無効にしたチャンネルの画素 | 層のチャンネルの面と有効 |
| チャンネルごとの合成モード・不透明度（版14） | `ChannelBlend` |
| 塗りつぶしのチャンネルごとの値と有効 | 塗りつぶしの値と有効 |
| 調整（反転・レベル補正・色相/彩度/明度）と対象のチャンネル | 調整の設定と対象 |
| Normal の出力の設定（版7） | `NormalSettings` |
| ユーザーチャンネルの一覧（版22） | 文書のチャンネルの一覧（番号・名前・種類・色空間・既定値） |

文書・層のIDと透明画素のRGBを保ち、読み込み操作をUndo履歴に残さない。読み込みの予算はcoreの画素の予算（256 MiB）で、超えたら層・タイルを添えて断る。

`core_issues()` は変換を断る項目を、層の機能ごとに1つ（`layers[2].filters（フィルター・Generator）` の形）返す。coreに無い機能 — フィルター・Generator、2D/3Dのパス、Anchor、塗りつぶしの画像・投影・グラデーション、レイヤーロック、手動ID色 — と、タイル外周の非ゼロ余白、coreの予算超過、**調整の種類が使わない値が既定ではない正本**が対象。最後のものは、C#の読み手が種類ごとの作り方で黙って既定に戻すので、coreへ渡すと保存で値が変わるために断る。非表示の層・無効にしたマスクの中身でも断り、黙って捨てない。正本の読み書きは引き続き可能で、断った文書は元のバイト列のまま保存される。

`NativeDocument::from_core(&document)` は C# の `DocumentBinary.Write` と同じ並びで正本を作る。ユーザーチャンネルが無ければ**版21**（Unity 0.2.0 が読める）、あれば**版22**。寸法は8192以下、タイル寸法は8〜512の2の累乗、層は2048まで、文字列はUTF-8で4096バイトまで。描画中のストロークは保存を拒否する。値の無い塗りつぶしのチャンネルと、グループの有効の印は、合成に効かず C# の書き手も書かないので書かない（C#が書いた正本を読んで書き戻すとバイト一致する）。旧版や空タイルのディスク配置まで再現するAPIではない。原本の版・項目・並びをバイト一致で戻すときは、元の `NativeDocument::to_bytes()` を使う。

```rust,no_run
use yolu_io::{NativeDocument, Project};
let project = Project::read(&std::fs::read("sample.ylp")?)?;
let set = project.sets().iter().find(|s| s.id == project.current_set()).unwrap();
let mut core = set.document.to_core()?; // 対応外の項目があれば、編集を始める前に断る。
core.add_group("グループ", None)?;
let native = NativeDocument::from_core(&core)?;
let changed = project.with_document(&set.id, &native)?; // 他セット・リソース・未知エントリを保持。
# Ok::<(), yolu_io::Error>(())
```

### ユーザーチャンネル（正本の版 22）

Unity 版の形式にはユーザーチャンネル（coreの番号6〜63）が無い。Rust 版が、ユーザーチャンネルのある文書だけを**正本の版22**で書く。ユーザーチャンネルが無い文書は今までどおり版21で、.ylp の形式は**7のまま**（YLP_FORMAT の「エントリの中身の版だけを変えるときは、そのエントリの版を上げる」）。

位置は、版7以降のNormalの設定（algorithm・derive・strength・edges・direction）の直後、層の数の直前:

| 欄 | 中身 |
|---|---|
| int 数 | 1〜58。0個の一覧は書かない（ユーザーチャンネルが無ければ版21で書く） |
| 数回、番号の昇順 | int 番号（6〜63。標準の0〜5は一覧に置かない。歯抜けでよい。重複・降順は断る）、string 名前（1〜128文字、制御文字なし、標準の6つ・ほかのユーザーチャンネルと重ならない）、int 種類（0 色・1 スカラー・2 法線）、int 色空間（0 sRGB・1 線形）、4 byte 既定のRGBA |

層の中でチャンネルの番号を持つ欄 — チャンネルごとの合成、塗りつぶしの値、調整の対象、ラスターのチャンネル — は、標準の0〜5に加えて一覧にある番号を置ける（個数の上限は 6 が 6＋一覧の数になる）。一覧に無い番号は断る。色相/彩度の対象は色（種類 0）のチャンネルだけ（標準ではColorとEmission）。標準のチャンネルだけの欄（塗りつぶしの画像・グラデーション、フィルター、パスのマテリアル）は0〜5のまま。版21以前に一覧は無く、標準以外の番号は断る。版23以降、版を22にしないで一覧を差した正本、一覧の破損はどれも理由を添えて断る。

Unity 0.2.0 は版22を「Unsupported archive version; source retained unchanged.」で断る。C#の `DocumentBinary.Read` / `ReadId` / `YlpFormat.Open`（`Runtime/Core` をそのままコンパイル）にRustが書いた版22を読ませた記録が [`user-channels-v22.unity.txt`](tests/fixtures/user-channels-v22.unity.txt) で、外側の .ylp と `project.json` は読め、開く手順のセットの正本の読みで断る（全部のセットを読めてから入れ替えるので、Unityの状態は変わらず、ファイルも書き換えない）。記録の最後の行は、ウィンドウ（`TexturePaintWindow`）が正本の読みの失敗に付ける文を、読み手の例外から同じ形に組み立てたもの。

選んだ理由と、採らなかった案:

- **版を上げる（採用）。** Unity版が新しい機能を足すたびに使ってきた決まりで、古い読み手は版の数だけで、ファイルに触れずに断る。ユーザーチャンネルが無い文書は版21のままなので、既存の文書、Rust版で保存した大多数の文書をUnity 0.2.0が開ける。
- **全部の文書を版22（0個の一覧）で書く。** Unity 0.2.0がRust版で保存した文書を1つも開けなくなる。ユーザーチャンネルを使わない人まで巻き込むので採らない。失うもの: チャンネルを足すと版が22に、全部消すと21に変わる（保存のたびに今の中身で決まる）。
- **.ylp の形式を8にする。** Unity 0.2.0は `ylp.json` の時点でファイル全体を断る。ユーザーチャンネルを持たないセットも巻き添えになる。形式を上げる決まり（エントリの追加・意味の変更）に当たらない。
- **ユーザーチャンネルの画素を別のエントリに置き、`document.utpaint` は版21のままにする。** Unity 0.2.0は知らないエントリとして一覧に出すだけで、そのまま開き、保存するとそのエントリを落とす。ユーザーチャンネルを黙って失う保存ができてしまうので、「未対応の情報を黙って捨てない」に反する。
- **版21のまま番号6〜63を許す。** Unity 0.2.0は `Invalid or duplicate channel` の汎用の文で断り、新しい版が要るとは言えず、破損と区別できない。名前・種類・色空間・既定値の置き場も無い。
- **`ylp.json` に機能の旗を足す。** Unity 0.2.0は知らないキーを読み飛ばすので、効かない。

移行: 版1〜21はそのまま読め（ユーザーチャンネルの一覧は空）、coreを通して保存すると版21（ユーザーチャンネルがあれば版22）になる。版22を読むRust版は、版21までを読む規則を変えない。版22から21へ戻すには、ユーザーチャンネルを消して保存し直す。

保証の射程と失うもの:

- 1つのセットでも版22なら、その .ylp 全体をUnity 0.2.0は開けない（セットごとには開けない）。Unityで開く必要があるときは、ユーザーチャンネルを消したファイルを保存する。版22から21へ直す移行はUnity側には無い。
- 版22の意味はこの節の表に固定する。Unity版が版22以降を別の意味で使うなら、同じ形式を仕様にして揃える（版の数が食い違うと、どちらの書き手も相手の文書を読み違える）。
- ユーザーチャンネルはC#に書き手が無いので、その合成はC#の合成との一致を保証しない（coreの合成の式と、往復のバイト一致の範囲）。合成のPNG（`composite/<チャンネル>.png`）は標準のチャンネルだけで、ユーザーチャンネルの分は書かない。
- 保証の範囲は `Runtime/Core` の読み手（`DocumentBinary`・`YlpFormat`）の結果まで。Unityのウィンドウ（EditorWindow）の操作の結果は範囲に含めない。

現在のセットのColor合成をPNGにする例（出力先は新規ファイル）:

```sh
cargo run -p yolu-io --example composite_png -- sample.ylp output.png
```

`composite_png(&core)` は下原点のRGBAをPNGの上からの行へ並べ、Unity版 `RgbaPng` と同じ適応フィルター・チャンク配置で出力する。`composite_pngs(&core)` は、どれかの層が有効にしている標準チャンネル（Height → Normal を作る設定で Height を使っていれば Normal も。Normal は Unity 向けの OpenGL の出力）ごとに同じ形式のPNGを返す。Color は使う層が無くても含める。Unity版のインポーターは `composite/<チャンネル>.png` の並びからセットのチャンネルを出すので、`SetSpec` にはこれを渡す。flate2のzlibバックエンド（libz-sys）を使う。圧縮結果はdeflate実装の版にも依存し、すべてのランタイム・画像での一致を保証するものではない。

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
    let mut document = origin.to_core()?; // 変換対象外の情報があれば理由を添えて拒否。
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

`ReadResult::to_core` と `psd::Document::to_core/from_core` はColor のラスター層、26合成モード、クリッピング、表示、不透明度を変換する。読み込み時にUndo履歴を残さず、PSDのIDはcoreの永続IDに埋め込んで再出力する。同名レイヤーもIDで区別する。変換対象外のグループ・調整・塗りつぶし・マスク・ロック、キャンバス外の画素、非Color/無効Colorは変換を拒否する。`core_issues()` で拒否する項目を確認できる。coreからの出力矩形はタイルの範囲になり、不透明度は最も近い1/255へ丸める。原本の圧縮や矩形までバイト一致で戻すAPIではない。

既定の `Limits` は原本・出力各128 MiB、寸法8192、キャンバス16,777,216画素、256レイヤー記録（区切りを含む）、復号256 MiB、メタデータ4 MiB、名前4096 UTF-16単位、診断128件、グループ32段。書き出しは確保前に構造・画素・出力長を検証する。全体をメモリで扱うため、ストリーミング型の大規模文書向けではなく、これらはプロセス全体の使用メモリの保証ではない。

統合画像は参照合成と比較する。通常合成だけで大きく違えば編集を止め、合成モード・マスク・グループ・調整等があれば `CompositeDiffers` で見え方の差を知らせる。参照合成はYoluPainterの式であり、Photoshop/CSPの見え方の再現を保証しない。再生成・検証の条件は [PSDフィクスチャ](tests/fixtures/psd/README.md) を参照。

## リソースと未対応の中身

画像はRGBA8のPNGを復号し、寸法と下の行からの画素ハッシュを検証する。スマートマテリアル・スマートマスク・マテリアル・ブラシは埋め込まれたファイル全体を保持する。内側のmanifest、正本、種別、チャンネル、画像、ブラシのschema・画像参照も検証する。

フィルターや Generator などの効果の実行、ブラシ設定の描画上の意味、.ylp内のPSD原本・合成PNG・メッシュマップ・画面設定の自動復号は未実装。PSD単体は上記のAPIで読み書きできる。これらは読み飛ばして破棄せず、原本のエントリとして残す。`notes()` を呼び出し側の画面に提示できる。画像リソース以外のPNGをすべて復号するAPIではない。

形式8以降、正本23以降、未知の列挙値・アルゴリズム版は理由を添えて断る。ZIP64・暗号化・分割ZIP、RGBA8以外のリソースPNGは扱わない。正本とZIPの1エントリは512 MiB、ZIPの展開総量は768 MiB・1000エントリ、復号リソースも768 MiBまで。これらはデータ量の上限であり、プロセスの最大メモリ使用量の保証ではない。

## 保存

`SaveTarget::open` はSHA-256・長さ・更新時刻を記録する。`create` は新規保存専用。保存ではメモリで検証し、保存先と同じフォルダの一時ファイルに書いて `sync_all`、読み直して検証、外部改変を再確認してから `rename` で一度だけ確定する。置換に失敗しても削除してから移動する手順には切り替えない。

直前の版は `<ファイル名>-backups~/` にSHA-256名で残し、自動削除しない。保存先ごとの排他的なロックファイルを作り、通常終了・エラー時には片付ける。プロセス強制終了後のロック回収は呼び出し側で扱う必要がある。非協調プロセスが最後の検査と置換の間に書く競合と、電源断時のディレクトリ永続性は保証の対象外。

## 書き出し（テンプレートとパディング）

`export` は書き出しのテンプレート（`yolu_core::export`。Unity Standard / URP Lit・HDRP Lit・lilToon）の画像をフォルダへPNGで書く。ファイル名はUnity版と同じ `<名前>[_<セット名>]_<接尾辞>.png`（セットが複数のときだけ `_<セット名>`。使えない文字は `_`）。

```rust,no_run
use yolu_core::export::ExportTemplate;
use yolu_core::padding::Reach;
use yolu_io::export::{write_template, PaddingSpec, SetExport, WriteOptions};

# let (document, coverage): (yolu_core::Document, Vec<bool>) = unimplemented!();
let report = write_template(
    std::path::Path::new("out"),
    &ExportTemplate::unity_standard(),
    &SetExport {
        stem: "Texture",
        set_name: None,
        document: &document,
        occlusion: None,
        padding: Some(PaddingSpec { coverage: &coverage, reach: Reach::Fill }),
        max_working_bytes: 1 << 30,
    },
    &WriteOptions::default(),
)?;
for image in &report.written {
    // Unityの取り込み設定は書かない。ここで渡す。
    println!("{} srgb={} normal_map={}", image.file_name, image.srgb, image.normal_map);
}
# Ok::<(), yolu_io::export::ExportError>(())
```

- 書く画像は、読むものがある画像だけ（`should_write`）。使っていないチャンネルしか読まない画像は書かず、`skipped` に接尾辞を返す。
- 手順は、(1) 名前・大きさ・上書きを確かめる、(2) 画像を1枚ずつ作って隠しの一時ファイルへ書き、`sync_all` して読み戻し、RGBAが一致することを確かめる、(3) 全部が済んでから1枚ずつ `rename` で置き換える。(2) までに失敗・取消すれば元のファイルは1つも変わらず、一時ファイルも残らない。(3) の途中の失敗だけ `ExportError::Partial` で済んだ分を知らせる（新しく作った分は消す）。
- 既にあるファイルは、既定では置き換えず `WouldReplace`（何も書かない）。`existing_files` で見せて確かめてから `Overwrite::Replace`。置き換えない設定では `hard_link` で置くので、調べた後に他のプロセスが作ったファイルも上書きしない。フォルダ・リンクは置き換えない。
- 複数のテクスチャセットは `plan_template` で書く画像とファイル名を出し、`clashes` で名前の重なりを確かめ、`write_images`（中身は呼び手の関数から1枚ずつ）で全部を1回の手順にする。
- 画像の中身は1枚ずつしか持たない。上限は1辺8192（Unity版の `RgbaPng` と同じ）。PNGは `composite_png` と同じエンコーダ（行ごとの適応フィルター、圧縮バイトはdeflate実装の版に依存）。
- Unityのインポートの設定（sRGB・ノーマルマップ・アルファを透明度に）はファイルに書かない。`WrittenImage` の `srgb`・`normal_map`・`alpha_is_transparency` を後で使う。
- 取消は `WriteOptions::cancel` の旗（置き換えを始める前まで）。塗り広げの途中でも旗を見て、どこで止まっても `ExportError::Cancelled` で返る。

## 検証

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

層の種類・チャンネルの正本は、C#の実際の書き手が作った文書と、coreにして書き戻した正本の全バイト一致、全チャンネルの合成とNormalのファイル出力の全バイト一致で照合する。Rustが書いた正本は、C#の読み手に読ませた結果を記録して固定する（版21は読めて、書き直すと同じバイト列になり、全チャンネルの合成が一致する。版22は断られる）。フィクスチャの対応範囲と再生成方法は [fixtures/README.md](tests/fixtures/README.md) を参照。

直接依存する flate2、crc32fast、sha2、serde、serde_json、png と、zlib接続用の libz-sys・vcpkg はMITまたはApache-2.0。解決済みの間接依存にはMIT・Apache-2.0・Zlib・Unicode-3.0などの許諾が含まれる。
