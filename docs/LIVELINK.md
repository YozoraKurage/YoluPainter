# Live Link（ファイルの受け渡し）

[English](en/LIVELINK.md)

Unity エディターの Unity ブリッジ（VPM パッケージ `net.yozolab.yolupainter`）とスタンドアロンの YoluPainter が、同じ PC のフォルダに JSON のファイルを置いて受け渡す仕組みの仕様です。
Unity は、選んだ相手（シーンのオブジェクト）の **FBX の道・ボーンの値・マテリアルの値と絵の道**を「頼み」に書いて置き、スタンドアロンがそれを拾って開きます。
スタンドアロンで書き出したら、書いた PNG を「返事」に書いて置き、Unity が拾って取り込みます。メッシュや絵の画素は送らず、両方がファイルを自分で読みます。

使い方は [UNITY.md](UNITY.md#live-link) にあります。この文書は、両方の側を作る人と、受け渡しの中身を調べる人のためのものです。

## フォルダ

両方が同じ道を自分で決めます。

| OS | フォルダ |
|---|---|
| Windows | `%LOCALAPPDATA%\YoluPainter\LiveLink\` |
| macOS | `~/Library/Application Support/YoluPainter/LiveLink/` |
| Linux など | `${XDG_DATA_HOME:-~/.local/share}/YoluPainter/LiveLink/`（`XDG_DATA_HOME` が絶対の道でなければ既定） |

- 環境変数 `YOLUPAINTER_LIVELINK_DIR`（絶対の道）があれば、両方ともそのフォルダを使います（試験・2 つのアプリを並べるとき）。
- 中に `inbox/`（Unity → スタンドアロン）・`claimed/`（スタンドアロンが拾った頼みと、拾っている印 `<頼みのファイルの名前>.lock`。Unity は読み書きしません）・`outbox/`（スタンドアロン → Unity）と、起きている印 `presence.json` を置きます。
- フォルダは、そのユーザーだけが入れるものにします。Unix では 0700 で持ち主が自分（Unity は作るときに `chmod 0700`）、Windows ではそのユーザーだけを許す
  DACL。スタンドアロンは、ほかの人も入れるフォルダ・持ち主の違うフォルダを使いません（理由を入口の印に出します）。
- ファイルは必ず `<名前>.tmp` に書いて閉じてから、最終の名前へ置き換えます。読み手は `.tmp` で終わる名前を見ません（書きかけを読まない）。

## 起きている印 `presence.json`

スタンドアロンが頼みを受けている間、2 秒ごとに書き直します。受けるのをやめる・終わるときに消します。

```json
{ "format": 1, "app": "YoluPainter", "version": "0.6.0", "pid": 1234, "updated": "2026-10-07T12:00:00Z" }
```

Unity は `updated`（UTC）が 6 秒より新しければ、スタンドアロンが起きていると見ます。古い・無いときは、スタンドアロンを `--livelink` を付けて起動してから頼みを置きます
（起動を待たずに置いてよい。スタンドアロンは起きてから拾います）。

## 頼み `inbox/<id>.json`（Unity → スタンドアロン）

```json
{
  "format": 1,
  "kind": "open",
  "id": "8f0c2a4e-0000-4000-8000-000000000001",
  "bridge": { "version": "0.6.0", "unity": "2022.3.22f1" },
  "project": { "root": "C:/Work/MyProject", "name": "MyProject" },
  "target": {
    "key": "GlobalObjectId_V1-2-…",
    "name": "Avatar",
    "export_dir": "C:/Work/MyProject/Assets/YoluPainter/Avatar"
  },
  "root": { "world": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1] },
  "models": [
    { "id": 0, "fbx": "C:/Work/MyProject/Assets/Avatar/Body.fbx", "guid": "0123456789abcdef0123456789abcdef",
      "import": { "global_scale": 1.0, "use_file_scale": true, "bake_axis_conversion": false,
                  "import_blend_shapes": true, "preserve_hierarchy": false } }
  ],
  "renderers": [
    { "path": "Body", "model": 0, "node": "Body", "enabled": true, "skinned": true,
      "blend_shapes": { "eye_close": 0.0, "smile": 35.0 }, "materials": [0, 1] }
  ],
  "bones": [
    { "model": 0, "node": "Armature/Hips", "local": { "t": [0.0, 0.9, 0.0], "r": [0.0, 0.0, 0.0, 1.0], "s": [1.0, 1.0, 1.0] } }
  ],
  "materials": [
    { "key": "guid:0123456789abcdef0123456789abcdef/fileid:2100000", "name": "Body",
      "shader": { "name": "Hidden/lilToonCutout", "guid": "…", "package": "jp.lilxyzw.liltoon", "version": "2.3.4", "keywords": [], "render_queue": 2450 },
      "values": { "floats": { "_Cutoff": 0.5 }, "colors": { "_Color": [1, 1, 1, 1] }, "vectors": {}, "ints": { "_lilToonVersion": 45 } },
      "textures": [
        { "property": "_MainTex", "path": "C:/Work/MyProject/Assets/Avatar/body.png", "guid": "…",
          "srgb": true, "normal_map": false, "scale": [1, 1], "offset": [0, 0] },
        { "property": "_MatCapTex", "path": null }
      ] }
  ],
  "refused": [ { "path": "Accessory", "reason": "mesh_not_from_fbx" } ]
}
```

- 文字は UTF-8 の JSON。道はすべて絶対の道で、区切りは `/`。数は JSON の数（NaN・無限は書かない。f32 に入らない大きさも有限でないとして断ります）。
- `format` は 1、`kind` は `"open"`（開く。同じ `target.key` の文書を開いていれば送り直し）だけ。知らないキーは読み飛ばします。
- `id`: 頼みごとに新しい ID（1〜64 文字の英数字と `-`・`_`）。返事の名前に使います。
- `target.key`: 相手のシーンのオブジェクトの身元（`GlobalObjectId`）。同じ相手の送り直しを見分けます。`name` は表示用、`export_dir` は書き出しの既定の置き場。
- `root.world`（任意）: Unity のワールドから相手の根への 4×4（列優先。相手の根の `worldToLocalMatrix`）。
- `models[]`: 相手が使う FBX。`id` は `renderers[].model`・`bones[].model` が指す番号。同じ FBX を 2 つの組で使うときは 2 つの項目にします（スタンドアロンは 1 回読んで、
  ボーンとメッシュを 2 つ置きます）。`import` は Unity の ModelImporter の設定（無い欄は Unity の既定）。
- `renderers[]`: レンダラー。`path` は相手の根からの道（表示と理由用）、`node` は FBX の中のメッシュのノードの道（下の「道」）、`enabled` は表示の入切、
  `blend_shapes` は BlendShape の名前 → 重み（Unity と同じ 0〜100）、`materials` はサブメッシュの順の `materials[]` の番号。
- `bones[]`: ボーンの値。`node` は FBX の中のボーンのノードの道（空はその FBX の根に当たるノード）、`local` は FBX の親のボーンに対するローカル（`t`・`r`（x, y, z, w）・`s`）。
  頼みに無いボーンは FBX のままの値。FBX の根に当たる Transform が相手そのもの（か相手の先祖）のとき、Unity はその根のノード（`node` が空）を送りません。
- `materials[]`: Unity のマテリアル。`key` はマテリアルの身元（テクスチャセットを結ぶ鍵。下の表）、`shader` はシェーダーの名前・アセットの GUID・
  `package`（シェーダーのアセットが入っている UPM パッケージの名前。`Assets` の中・組み込みのシェーダーは空か無し）・版・キーワード・描画の順。
  `values` はプロパティの値（`floats`・`colors`・`vectors`・`ints`）、`textures` はテクスチャのプロパティごとの絵のファイル
  （`srgb`・`normal_map` は TextureImporter の設定、`scale`・`offset` は拡大とずらし）。Unity の中にしかない絵（ファイルの無い生成物・RenderTexture）は `path` を `null` にします。
- `refused[]`: Unity が送れなかったレンダラー（理由は下の表）。

| `materials[].key` | マテリアル |
|---|---|
| `guid:<32 桁の 16 進>/fileid:<数>` | マテリアルのアセット |
| `object:<GlobalObjectId>` | シーンの中のマテリアル（アセットでない） |
| `instance:<InstanceID>` | GlobalObjectId が空のマテリアル（その Unity のセッションの中だけの身元） |
| `none` | マテリアルの無いサブメッシュ（スタンドアロンはマテリアルなしの組にします） |

スタンドアロンは、送り直しや `.ylp` を開き直したときに、アセットの鍵は識別子で、ほかの鍵はマテリアルの名前でセットに結び直します。
- 上限（超えると `too_large` で断ります）: ファイル 16 MiB、レンダラー・モデル・送れなかったレンダラー 1,024、ボーン 16,384、マテリアル 1,024、1 つのマテリアルのテクスチャ 256、
  1 つのレンダラーの BlendShape・1 つのマテリアルの値の種類ごとの数 4,096、文字列 8 KiB。

### 道（FBX の中のノード）

- 道は、Unity が取り込んだモデルの根からの名前を `/` でつないだものです。
- Unity は、取り込みの設定 `preserveHierarchy` が切れていて（既定）FBX の根の子が 1 つだけのとき、その子をプレハブの根にします（その子の名前は道から消え、その子の
  変換はプレハブの根に移ります）。スタンドアロンは `import.preserve_hierarchy` を見て同じ根から道をたどります。
- 同じ名前の兄弟があって 1 つに決まらない道は `ambiguous_bone`、見つからない道は `bone_not_found` です。
- Unity は取り込みで同じ名前の兄弟を `Twin`・`Twin 1` のように付け直します。FBX に無い `<名前> <数>` の節で、FBX にその名前の兄弟が 2 つ以上あるときも
  `ambiguous_bone` です（どの兄弟に付け直したかは FBX からは決まりません）。

### ボーンの値の決め方（Unity の側）

- メッシュのアセットの道（`AssetDatabase.GetAssetPath(mesh)`）が `.fbx` のレンダラーだけを送り、ほかは `refused` に `mesh_not_from_fbx`。
- ボーンの FBX の中の道は、`PrefabUtility.GetCorrespondingObjectFromOriginalSource(transform)` で FBX のモデルのアセットの中の Transform を取り、その根からの名前の道にします。
  取れない（プレハブを展開してある など）ときは、FBX の根に当たる Transform からの名前の道で探し、1 つに決まらなければそのレンダラーを `refused`（`bone_not_found`、
  名前が重なる兄弟がいるときは `ambiguous_bone`）。
- `local` は、FBX の中の親のノードに当たる Unity の Transform に対する、そのボーンの Transform の相対（`parent.worldToLocalMatrix * bone.localToWorldMatrix` を T・R・S に）。
  Unity の階層で親を付け替えてあっても、FBX の親に対する値です。FBX の根に当たるノードは、相手の根に対する値。
- 座標は Unity のまま（左手系・Y が上・メートル）。

### 取り込みの設定

- スタンドアロンは FBX を Unity と同じ向き・大きさで読みます: Unity の単位 = メートル × `global_scale` ×（`use_file_scale` なら 1、切っていれば 1 ÷ ファイルの単位のメートル。
  UnitScaleFactor が 1（センチメートル）の FBX なら 100）。
- `bake_axis_conversion` が true の FBX は合わせられないので、そのレンダラーを `unsupported_import` にして入れません。
- `import_blend_shapes` が false の FBX は、BlendShape を入れません。
- サブメッシュの並びは Unity と同じです（FBX のノードのマテリアルのスロットの順。面の無いスロットは飛ばす）。

## スタンドアロンの受け方

- 設定「Unity の Live Link を受け付ける」（既定は入）か `--livelink` で起動したとき、受け付けます。受け付けている間、0.5 秒ごとに `inbox/*.json` を見ます。
- 拾うときは、まず `claimed/<頼みのファイルの名前>.lock`（拾っている印）を、同じ名前があれば必ず失敗する作り方（排他的な作成）で作ります。作れた受け手だけが
  `inbox/` から `claimed/` へ頼みを移して受けるので、2 つのスタンドアロンが起きていても、1 つの頼みを受けるのは、印を作れた 1 つだけです（名前を変えられたかでは決めません）。
  当て終えたら、`claimed/` から頼みを消し、その後で印を消します。
- `claimed/` に 1 日より長く残った頼みと印（落ちたプロセスが残した物）は、受け付けを始めるときに片付けます。頼みが `inbox/` に残ったまま、印だけが 1 日より長く
  残っていた（印を作った直後に落ちた）ときも、片付けた後は次の受け手が拾えます。
- 描いている最中・保存の途中・ほかのモデルの読み込みの間に来た頼みは、断らずに `claimed/` に置いたまま待ち、終わってから当てます。
- 同じ `target.key` の文書を開いていれば**送り直し**: ポーズ・BlendShape・マテリアルの値・表示の入切を当てます。FBX は道と `guid` が同じなら読み直しません
  （表示の入切・使うメッシュ・マテリアルの付け方が変われば、読んだ FBX から作り直します）。送り直しのポーズは、手で動かした分も上書きします（1 つの取り消しの段）。
- 違う相手で、保存していない変更があれば「開く」と同じ確かめをします。開くのをやめれば `refused`（`declined`）を返し、捨てれば新しいプロジェクトに開きます。
- 読み込みの途中に利用者が文書を替えた（新規・開く）ときは、結果を新しい文書に入れず `refused`（`declined`）を返します。受け付けをやめたときも、まだ当てていない頼みと
  読み込みの途中の頼みは取り消して `declined` を返します（`.ylp` の開き直しは続けます）。
- 複数の FBX は 1 つのモデルにまとめます（FBX が 2 つ以上なら、ボーンの根の名前の頭に FBX の番号を付けて分けます）。ポーズのパネルでボーンを動かせます。
- テクスチャセットはマテリアルの `key` ごとに 1 つ（同じ Unity のマテリアルを使うレンダラーは同じセット）。
- 元の絵: Color の流し込み先（`_MainTex`）の絵のファイルを読み（PNG・TGA・JPG・PSD）、新しく作ったセットと何も触っていない最初のセットの
  一番下のレイヤー「元の絵」に入れます。絵の無いマテリアル・Unity の中にしかない絵は白、読めない絵は白で始めて理由を知らせます。
- 元の絵が PSD（sRGB）のときは、平らな「元の絵」ではなく、PSD のレイヤーのままセットに入れます。読み方は「ファイル → インポート」の PSD の取り込みと同じ写しで
  （[PSD.md](PSD.md)）、PSD のレイヤーが下に並び、セットの空のレイヤーはその上に残ります。セットの大きさは PSD のキャンバスのまま
  です（レイヤーを拡大縮小しません）。取り込みで落とす物・変わる物は、確認のウィンドウと同じ名前で知らせに出します（ウィンドウは出さずに入れます）。元の PSD は読むだけで、
  同じファイルへ PSD を書き出すときは置き換える前に確かめます。予算などでレイヤーのまま取り込めない PSD は、理由を知らせて平らな「元の絵」にします。
  1 つの頼みで読む元の絵の画素（平らな絵と、レイヤーのまま入れる PSD の文書）は合計 512 MiB までで、残りを超える PSD は平らにします（平らにしても入らなければ
  白で、理由を知らせます）。同じ PSD を使うマテリアルが複数あれば、マテリアルごとに別の文書として読み、それぞれ数えます。
  リニアの PSD（`srgb` が false）と、Color 以外のスロットの PSD は、レイヤーを重ねた 1 枚として読みます。
- 送り直しで元の絵のファイル（道・更新時刻・大きさ・`srgb`）が変わっていれば、元の絵を入れた直後のまま（描いていない・何も変えていない）のセットには
  新しい元の絵を入れ直します（絵が無くなれば白）。変わったファイルを読めなかったとき（壊れている・書き込みの途中など）は、セットを変えず（白にしません）、
  「<セット名>: 元の絵「body.psd」を読めないので、セットはそのままです」と知らせ、次の送り直しでもう一度読みます。触ったセットは変えず、「<セット名>: 元の絵「body.psd」が変わりました」と知らせます（同じ変化は 1 度だけ）。
  入れた直後かどうかはアプリを開いている間だけ覚えるので、`.ylp` から開き直したセットは触ったセットとして扱います（開き直した後の変化を知らせます）。
- lilToon のマテリアルの値は、セットの「受けた見た目」になります（3D ビューの lilToon の再現が読む。`_MainTex` はスタンドアロンの Color で描き、ほかのスロットの絵は
  ファイルから読みます（長い辺 2048 まで、合計 256 MiB まで）。送り直しでは、道・更新時刻・大きさが同じファイルは読み直しません）。lilToon でないマテリアルは受けた見た目を外します。
- lilToon かどうかは、シェーダーの名前では決めません。`values` に `_lilToonVersion`（lilToon のシェーダーが持つ版の値）があるか、`shader.package` が
  `jp.lilxyzw.liltoon` のときだけ lilToon として描きます。
- `.ylp` には、当てた頼みに今のポーズを入れ、マテリアルの値（`_lilToonVersion` のほか）を除いた形を根の `livelink.json` に残し、Unity なしで開き直せます（[YLP_FORMAT.md](YLP_FORMAT.md#livelinkjson)）。
  受けたマテリアルの値は、設定「Unity から受けたマテリアルの値を保存する」（既定は入）の間、セットの `look.json` に残ります（テクスチャの画素は入れず、開き直すとファイルから読みます）。
  「配布用に保存」で「モデルの参照」を除いた写しには、`livelink.json` も入りません（[SAVE_FOR_DISTRIBUTION.md](SAVE_FOR_DISTRIBUTION.md)）。

## 返事 `outbox/<id>-<n>.json`（スタンドアロン → Unity）

```json
{ "format": 1, "request": "8f0c2a4e-0000-4000-8000-000000000001", "kind": "exported",
  "app": { "version": "0.6.0" },
  "problems": [ { "path": "Accessory", "reason": "bone_not_found" } ],
  "files": [ { "material": "guid:0123456789abcdef0123456789abcdef/fileid:2100000", "property": "_MainTex",
               "path": "C:/Work/MyProject/Assets/YoluPainter/Avatar/Avatar_Body_Main.png", "srgb": true, "normal_map": false } ] }
```

- `kind`: `opened`（受けて開いた・送り直しを当てた。合わなかった物は `problems`）、`refused`（受けなかった。理由は `problems`）、`exported`（利用者が書き出した）。
- `<n>` は頼みごとの 0 からの通し番号。読めない頼みへの返事は、`inbox/` のファイルの名前（拡張子の前）を `request` にします。
- `exported`: Live Link の相手の文書を書き出すと、「テクスチャを書き出す」ウィンドウの出力先の既定は `target.export_dir`（利用者が選び直したらそちら）です。書いた PNG のうち、lilToon の
  テンプレートの画像（`Main` → `_MainTex`・`Normal` → `_BumpMap`・`Smoothness` → `_SmoothnessTex`・`Metallic` → `_MetallicGlossMap`・`Emission` → `_EmissionMap`）と
  lilToon の詰め方のスロットの画像を `files` に入れます。対応の無い画像と、鍵が `none` の組の画像は入れません。

## Unity の受け方

- 頼みを送った後・Live Link のウィンドウが開いている間、`outbox/` を 1 秒ごとに見て、読んだ返事を消します。
- `exported` の `files` を取り込み、`srgb`・`normal_map` を TextureImporter に当てます（`Assets` の外の道は取り込まず、理由を出します）。マテリアルに当てるかは、変える組
  （マテリアル・プロパティ・前の絵 → 新しい絵）の一覧で確かめ、当てるときは Unity の取り消しで戻せます。

## 理由の言葉（`reason`）

| 言葉 | 意味 |
|---|---|
| `mesh_not_from_fbx` | メッシュが FBX から来ていない（.asset など） |
| `bone_not_found` | ボーン・メッシュのノードが FBX の中に見つからない |
| `ambiguous_bone` | 同じ名前の兄弟があり、1 つに決まらない |
| `unsupported_import` | 取り込みの設定（`bake_axis_conversion`）に合わせられない |
| `fbx_unreadable` | FBX を読めない |
| `texture_unreadable` | 絵のファイルを読めない |
| `too_large` | 上限を超えた |
| `format_unknown` | 形式の版・種類が違う・JSON として読めない・決まりに合わない |
| `busy` | 描いている最中・保存の途中（今のスタンドアロンは返さず、終わってから当てます） |
| `declined` | 利用者が開くのをやめた: 違う相手の頼みで保存していない変更を捨てなかった・読み込みの途中に文書を替えた・受け付けをやめた |

知らない言葉は、そのまま見せます。

## 0.4 までの Live Link からの違い

- Unity でポーズを動かすとすぐ映る・描いている最中に Unity の画面に映る、はありません（送り直しで当て、Unity へは書き出した PNG を返します）。
- FBX から来ていないメッシュ（.asset など）と、取り込みで軸を焼いたモデル（`bake_axis_conversion`）は開けません。
- つなぎっぱなしの通信（ネイティブのブリッジのライブラリ・共有メモリ）は使いません。
