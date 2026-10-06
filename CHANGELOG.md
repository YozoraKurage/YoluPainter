# Changelog

## 0.4.0

### 日本語

- **コマンドラインと AI からの操作**: 同梱の `yolupainter-cli` で、.ylp をアプリを開かずに、またはいま開いている文書を、同じ命令（読む・見本の画像・層と効果と値の編集・書き出し・保存）で操作できます。`yolupainter-cli mcp` は MCP サーバーで、Claude Desktop（Release に `.mcpb` を付けます）・Claude Code などの AI のアシスタントからつなげます。起動中のアプリへの操作は、設定「外からの操作を受ける」を入れたときだけ受けます（[詳しくは](docs/CLI.md)）。
- **速さ**: 結果の画素は変えずに速くしました。
  - 2D のブラシ: 2〜6 倍ほど（大きなブラシほど効きます。柔らかい丸・筆先・質感・指先・ぼかし・色の混ぜ）。
  - 合成・調整・フィルター・Normal の計算を CPU の SIMD（AVX2・SSE4.1）で。Generator（エッジの摩耗・汚れ・ノイズ・グランジなど）は 5〜20 倍ほど。
  - 効果のスライダーを動かしている間は粗い絵ですぐに見せ、2D の表示は見えている所から仕上げます。キャンバスの GPU の表示が、独立したグループ・調整の層・法線・効果のある文書にも効きます。
  - 保存は裏で動き、保存している間も描けます。
- **3D の塗り**: 大きなブラシ・面の重なった所でも、ストロークを取り消さずに塗ります。隠れた所・裏の面・面の向きで弱める・継ぎ目のにじみを、ブラシの「3D」の欄で選べます。
- **選択範囲を名前を付けて残す**: 文書に残して呼び戻せます。使った .ylp は形式 8 になり、0.3.x と Unity 版では開けません（配布用に保存で除けば、形式 7 の写しになります）。
- **ポーズ**: 今のポーズを .ylp に残し、開くと戻します。ポーズのプリセット（名前を付けて残し、同じボーン名のモデルへ当てる。左右反転つき）。
- **3D ビュー**: アンチエイリアスとブルーム。既定のカメラはモデルの前から、光はモデルの前の斜め上から。FBX の読み込みを途中で取り消せます。
- **CLIP STUDIO のブラシ**: 入り抜き・傾き・筆先の角度とランダム・手ぶれ補正を写し、サブツールのフォルダから選んで取り込めます。素材から原寸の筆先・質感を読みます。
- **Live Link**: スタンドアロンが要るマテリアルだけを Unity に頼み、変わらない絵は送り直しません。送りが詰まったときは、Unity の操作を止めずに少し後で送り直します。Unity ブリッジも 0.4.0 にしてください。
- **PSD**: 窓に .psd を落とすと、新しいテクスチャセットとして取り込めます。
- **試験版**: 「ヘルプ → 試験版を使う」で、alpha・beta・rc の更新も受けられます。
- **Windows**: 拡大率の違う画面・画面の増減・自動で隠すタスクバーでの窓の置き場所と、ファイルや確かめの窓が後ろに回る不具合を直しました。
- **頑丈さ**: GPU の装置を失ったとき・メモリが足りないときは、作業を復旧に残し、理由を出して終わります。閉じるときに書き出しなどの仕事が走っていれば尋ねます。
- **.ylp の形式**: 形式の仕様を [docs/YLP_FORMAT.md](docs/YLP_FORMAT.md) にまとめました。

### English

- **Command line and AI assistants**: The bundled `yolupainter-cli` runs the same commands (read, preview images, edit layers, effects and values, export, save) on a .ylp without opening the app, or on the document open in the app. `yolupainter-cli mcp` is an MCP server for AI assistants such as Claude Desktop (an `.mcpb` is attached to the release) and Claude Code. The running app accepts commands only while "Accept external commands" is on in its settings ([details](docs/en/CLI.md)).
- **Speed**: Faster without changing the resulting pixels.
  - 2D brushes: about 2–6× faster, most with large brushes (soft round, tip images, textures, smudge, blur and color mixing).
  - Compositing, adjustments, filters and Normal use CPU SIMD (AVX2, SSE4.1). Generators (edge wear, dirt, noise, grunge and so on) are about 5–20× faster.
  - While you drag an effect slider, a coarse image shows at once, and the 2D view finishes the visible area first. The canvas GPU display now also covers isolated groups, adjustment layers, normals and documents with effects.
  - Saving runs in the background, and you can keep painting while it saves.
- **3D painting**: Large brushes and overlapping faces no longer cancel strokes. Choose how hidden areas, back faces, facing angle falloff and seam bleeding behave in the brush's "3D" section.
- **Saved selections**: Name a selection, keep it in the document and recall it. A .ylp that uses them becomes format 8, which 0.3.x and the Unity version cannot open (Save for Distribution can leave them out for a format 7 copy).
- **Pose**: The current pose is saved in the .ylp and restored on open. Pose presets: name a pose and apply it to models with the same bone names, with mirroring.
- **3D view**: Anti-aliasing and bloom. The default camera looks at the model's front, and the light comes from above in front. FBX loading can be cancelled.
- **CLIP STUDIO brushes**: Taper, tilt, tip angle and its randomness, and stabilization are carried over, and brushes can be picked from a sub tool folder. Full-size tips and textures are read from the material files.
- **Live Link**: The standalone asks Unity only for the materials it needs and does not resend unchanged images. When sending is congested, Unity resends a little later without stalling. Update the Unity bridge to 0.4.0 as well.
- **PSD**: Dropping a .psd on the window imports it as a new texture set.
- **Beta versions**: "Help → Use Beta Versions" also offers alpha, beta and rc updates.
- **Windows**: Fixed window placement across displays with different scaling, displays being added or removed, and auto-hiding taskbars, and file and confirmation dialogs opening behind the window.
- **Robustness**: When the GPU device is lost or memory runs out, your work is kept for recovery and the app quits with the reason. Closing the app asks first if an export or similar job is running.
- **.ylp format**: The format specification is now in [docs/YLP_FORMAT.md](docs/YLP_FORMAT.md).

## 0.3.2

### 日本語

- **保存**: 開いている .ylp を外で名前を変える・動かす・消す・置き換えても、別名で保存できます。保存の途中でアプリが落ちても、途中で切れた退避や一時ファイルが次の保存で片付きます。Windows で保存先がほかのアプリに一時的に掴まれていても、少し待ってやり直します。
- **メモリ**: 自動の上限を物理メモリに見合う値に上げました（レイヤーのメモリは物理メモリの半分）。設定の「レイヤーの画素」は「レイヤーのメモリ」と呼びます。
- **取り込みの上限**: .ylp に保存できない大きさ（1 辺 8192 を超える）・層の数（2048 を超える）・グループの入れ子（64 段を超える）の PSD は、取り込む前に理由を出して断ります。Live Link でマテリアルが 64 を超えるときは 64 までセットを作り、数を知らせます。
- **Live Link**: スタンドアロンを最小化していても動きます。Unity で発光のテクスチャが空のマテリアルも、3D ビューで光ります。Unity の「Open in YoluPainter」が、変えた Link name で起動します。
- **lilToon**: スロットの選択肢から「新しいチャンネル」を作って割り当てられます。
- **選択**: 選択があるときの Esc で選択を解除します。
- **そのほか**: ネットワーク上のモデルの参照は、開くときに自動で読みに行きません。Ctrl+- で画面全体が縮まなくなりました。「YoluPainter について」の表示と、画面の文の言葉を直しました。

### English

- **Saving**: You can save under another name even after the open .ylp was renamed, moved, deleted or replaced outside the app. Leftovers from a save interrupted by a crash are cleaned up by the next save. On Windows, saving retries briefly when another app holds the file.
- **Memory**: Automatic limits now follow physical memory (layer memory is half of it). "Layer pixels" is now called "Layer memory".
- **Import limits**: PSDs that cannot be saved as .ylp (edge over 8192, more than 2048 layers, groups nested over 64 levels) are refused before import. Live Link creates at most 64 sets and tells you how many were left out.
- **Live Link**: Works while the standalone is minimized. Materials whose emission texture is empty in Unity now glow in the 3D view. "Open in YoluPainter" in Unity starts the standalone with the changed Link name.
- **lilToon**: Slots can create and assign a new channel.
- **Selection**: Esc deselects when there is a selection.
- **Other**: Network paths to models are not read automatically when opening. Ctrl+- no longer shrinks the whole UI. Fixed the About text and wording in messages.

## 0.3.1

### 日本語

- **アンインストール**: 「設定と復旧のデータも削除」で、アプリが作り直せる物（設定・窓の配置・復旧・クラッシュの記録・キャッシュ）だけを消し、個人のライブラリ・ブラシ・サブツール・グラデーション・カラーセットは残します（[詳しくは](docs/INSTALL.md)）。
- **更新**: ほかの YoluPainter の窓が開いているときは更新を断ります（閉じてからもう一度押せば、落とした更新がすぐ入ります）。インストーラーが待ちきれなかったときも、今のアプリを起こし直します。
- **クラッシュの記録**: 呼び出しの履歴に番地と実行ファイルの基底を残し、Release に PDB を付けます。
- **3D で描くときの知らせ**: 面が重なりすぎて取り消したときなどの文を、使う人の言葉にしました。

### English

- **Uninstall**: "Also delete settings and recovery data" now removes only what the app can recreate (settings, window layout, recovery, crash logs, caches) and keeps your library, brushes, sub tools, gradients and color sets ([details](docs/en/INSTALL.md)).
- **Updates**: Updating is refused while another YoluPainter window is open (press again after closing it; the downloaded update installs right away). If the installer cannot wait for the app to close, it restarts the current app.
- **Crash logs**: Backtraces keep frame addresses and the image base, and releases include the PDB.
- **3D painting messages**: Messages such as a stroke cancelled for too many overlapping faces now use plain words.

## 0.3.0

### 日本語

初回リリースに含まれる機能です。Windows を主な対象とし、Mac・Linux は試用向けです。

- **2D・3D のペイント**: ブラシ、消しゴム、Windows Ink の筆圧、13 個の組み込みブラシ、ブラシの詳細設定と自分のブラシの保存。ぼかし・指先・クローン、対称、ステンシル、スポイト。3D では一部の筆先・動的設定に制限があります。
- **サブツールとツールプロパティ**: 左のドックの「サブツール」に、今の道具のサブツールの一覧・ツールプロパティ・ブラシサイズを 1 か所にまとめました。ブラシと消しゴムは別の一覧で、バケツ・ポリゴン塗りつぶし・グラデーション・図形・定規・スポイト・移動・ゆがみは設定の組のプリセット、選択の道具は同じ並びの道具です。自分のサブツールは設定のフォルダに保存できます（[詳しくは](docs/SUBTOOLS.md)）。右のプロパティは選んでいる層の中身だけ、上のオプションバーはよく使う 2〜3 項目で、どちらもツールプロパティと同じ値を見せます。
- **ブラシの取り込み**: Photoshop の ABR・PAT、GIMP の GBR・GIH・VBR、PNG の筆先を取り込み。Krita 4 の既定の筆先 76 個（CC0）を同梱。
- **レイヤーの編集**: グループ、マスク、塗りつぶし、調整（階調の反転・レベル補正・色相/彩度/明度）、クリッピング、26 種類の合成モード。複数選択、結合、4 種類のロック、移動・拡大縮小・回転・反転、取り消しとやり直し。
- **選択と塗りつぶし**: 長方形・楕円形・なげなわ・多角形・自動選択、選択範囲の合成と加工。バケツ、ポリゴン塗りつぶし、ID の色で選択、2D のグラデーション。コピー・カット・結合してコピー・ペーストと OS の画像クリップボード。
- **編集できるパス**: 2D の曲線と 3D の面に結び付くパス、点ごとの太さ、複数チャンネルの描画、ラスタライズ。モデルの差し替え時にパスを付け直し、付け直せない場合はロックに応じて画素を保持します。
- **チャンネルとマテリアル**: Color・Roughness・Metallic・Height・Normal・Emission とユーザーチャンネル。複数チャンネルを 1 回のストロークで塗るマテリアル、チャンネルごとの合成、ハイト → ノーマルと OpenGL／DirectX の法線設定。
- **プロジェクトとモデル**: 新規プロジェクトのモデル・テンプレート・解像度・法線形式・ベイクの設定、マテリアルごとのテクスチャセット、セットの追加・削除・大きさの変更、モデルの差し替えと読み直し。FBX の読み込み、骨と BlendShape によるポーズ上への描画（ポーズは保存しません）。
- **3D プレビューとベイク**: PBR マテリアル・中立・チャンネルの表示、環境と光、トーンマッピング。メッシュマップの GPU／CPU ベイク、進捗と取消、GPU が使えない場合の CPU への切り替え。古いマップの識別。GPU と CPU のベイク結果は全バイト一致ではありません。
- **非破壊の効果**: フィルター、メッシュマップを読む Generator、Anchor をレイヤーとマスクへ追加し、一覧とプロパティで編集。ノイズ・グランジ、塗りつぶしの画像と投影、デカール、ワールドスペースのグラデーション。
- **アセットの棚**: スマートマテリアル・スマートマスクの保存と配置、`.ylsmart` の取り込みと書き出し、14 個の組み込みスマートマテリアル。対応しない素材は理由を表示して配置を断ります。個人のライブラリ（設定の「棚の場所」のフォルダ。Unity 版と同じ形のフォルダで、PNG・`.ylsmart`・ブラシ・マテリアルのファイルを並べます。JPEG は並べません）をプロジェクトの棚と切り替えて見られ、画像・スマート素材・ブラシ・マテリアルをプロジェクトで使う（写しを `.ylp` に保存するので、ライブラリが無い所でも開けます）・ライブラリへ入れる・消すことができ、画像とスマート素材は文書へ置けます。ライブラリへ入れるときは、同じバイト列のファイルがあれば書きません（画像は、見終えた画像と画素が同じでも書きません）。16 ビットの PNG は 8 ビットに丸めて使い、その旨を知らせます。サムネイルは別のスレッドで作り、中身ごとにキャッシュします（枚数と大きさの上限は、整理のあいだ少し超えることがあります）。
- **保存と復旧**: `.ylp` の読み込み・保存、直前の版の退避と保持数の設定、別のアプリによる変更の検出。復旧用の世代を自動で書き、次回起動時に復旧できます。復旧が使うディスクの量には上限があり（窓の「使う量」。超えたぶんは、落ちた書き込みの残りを先に片付け、なお超えれば古い世代から消し、この実行と落ちた実行ごとの最新は残します）、空きが少ないときは書き置きを見送ります（[詳しくは](docs/RECOVERY.md)）。読み取り専用セットの元データや未知の追加エントリを保存時に保持します。
- **画像の受け渡し**: RGB8 PSD のレイヤー・グループ・単色の塗りつぶし・対応する調整・マスク・クリッピング・ロックを、元のファイルへ書き戻さない写しとして取り込み（ファイルは流して読み、大きさの上限は設定の「レイヤーの画素」の予算から決まります。レイヤー効果・スマートオブジェクト・未対応の調整など取り込めないもの、無視するものは、取り込む前に層の名前つきで確かめます）、チャンネルごとの書き出し。フィルター・Generator・画像・パスなど PSD に形の無いものは画素にして書き、刻みの間の調整は丸め、グラデーションマップの値のカーブは停止点に展開します。した事は書く前の確かめの窓に層の名前つきで並びます。書き出す PSD は RLE（PackBits）で圧縮し、層を 1 枚ずつ流して書くので、大きな文書もメモリを倍にせずに書けます（上限は設定の「レイヤーの画素」の予算から。PSD の 1 辺は 30000 まで）。チャンネル別 PNG と全チャンネルの書き出し、Unity Standard／URP Lit・HDRP Lit・lilToon 向けのテンプレート、UV の余白と AO。
- **Unity との Live Link**: 同じ PC・同じユーザーの Unity エディターからモデルを受け取り、描いた色を元アセットを変えずにマテリアルへ一時表示。接続時に双方が鍵と、互いの版・使える機能を確認します。版や機能にずれがあってもつないだままにし、メニューバー右端の印を警告の色にして、ツールチップに両方の版・どちらを上げるか・使えない機能の名前を出します（プロトコルの版が重ならないときだけ断ります）。Unity のマテリアルが確かめた lilToon なら、本物のマテリアルの値と描いていないテクスチャ（影色・マットキャップなど）を受けて、そのテクスチャセットを 3D ビューで lilToon の見た目で描きます（欄で変えた項目だけがスタンドアロンの値になり、切っても最後の値が残ります。受けた値を `.ylp` に保存するかは設定で選べ、テクスチャの画素は保存しません）。新しく作ったテクスチャセットには、Unity の元のテクスチャ（Color の流し込み先）が一番下の「元の絵」のレイヤーとして入り（入るまで Unity には出さないので、つないだ瞬間に見た目は変わりません）、その上に描けます。描いたセット・開いたプロジェクトのセットには入れません。
- **画面と設定**: 日本語・英語の表示、ドッキングするパネル、2D の拡大縮小・移動・回転・反転、対応する文書の GPU 合成と CPU への切り替え。メモリの予算・CPU のスレッド・書き出しの余白・棚の場所などを設定できます。設定は「編集 → 設定…」（Ctrl+,）にあり、節に分かれた窓で、3D ビューの回転とズームの中心も変えられます。一覧と欄のスクロールはどれも同じ振る舞い（ホイール・つまみ・溝を押す）で、ペンの押して動かす入力もマウスと同じ経路で受けます。状態の帯の右端に版・ビルドと使っているメモリを出し、直前の操作の結果と理由は帯のすぐ上に小さな知らせとして数秒（断りや失敗は長め）出て、押すと消えます。ドックの並び・浮かせた窓の位置と大きさ・窓の大きさと位置・最大化は、終わるときと変えたとき（1 秒おき）に保存して次の起動で戻します（読めない・合わない並びは、既定の並びで始めます）。
- **配布と更新**: Windows の利用者ごとのインストーラーとポータブル ZIP。選択した場合だけ起動時に更新を確認し、インストーラー版は署名つきの更新情報でダウンロードを検証して更新します。実行ファイル・インストーラーのコード署名はまだありません。インストーラー・ZIP・tar.gz には説明の文書（`docs`、日英）も入り、ネットワークが無くても読めます。

互換性: `.ylp` は形式 7 で保存し、編集したセットは通常は文書形式 21、ユーザーチャンネルを使う場合は 22、ノイズ・グランジを使う場合は 23 になります。文書形式 21 までの Unity 版（0.2.0 など）は 22・23 を含むファイルを開けません。手動の ID の色がある文書は保存できません。詳細と各機能の制限は [README](README.md) を参照してください。

### English

Features included in the first release. Windows is the primary platform; Mac and Linux support is experimental.

- **2D and 3D painting**: brushes, erasing, Windows Ink pressure, 13 built-in brushes, Brush Details, and custom brush saving. Blur, Smudge, Clone, symmetry, stencils, and Eyedropper. Some brush tip and dynamics settings are limited in 3D.
- **Sub tools and tool properties**: the Tools tab on the left shows the current tool's sub tool list, tool properties, and brush size in one place. Brushes and erasers have separate lists; Fill, Polygon Fill, Gradient, Shape, Ruler, Eyedropper, Move/Transform, and Liquify use presets of their settings; selection tools list each other. Your own sub tools are saved in the settings folder ([details](docs/SUBTOOLS.md)). The Properties panel on the right shows only the selected layer, and the options bar keeps the two or three most-used items, both showing the same values as the tool properties.
- **Brush import**: import Photoshop ABR/PAT, GIMP GBR/GIH/VBR, and PNG tips. Includes 76 default Krita 4 tips (CC0).
- **Layer editing**: groups, masks, fills, adjustments (Invert, Levels, Hue/Saturation/Lightness), clipping, and 26 blend modes. Multiple selection, merging, four lock types, moving, scaling, rotation, flipping, undo, and redo.
- **Selections and fills**: Rectangle Select, Ellipse Select, Lasso, Polygon Select, Magic Wand, selection combinations and modifications. Fill, Polygon Fill, ID Color Select, and 2D gradients. Copy, Cut, Copy Merged, Paste, and OS image clipboard support.
- **Editable paths**: 2D curves and paths bound to 3D surfaces, per-point width, multichannel painting, and rasterization. Paths are rebound when replacing a model; failed rebindings preserve pixels according to the layer locks.
- **Channels and materials**: Color, Roughness, Metallic, Height, Normal, Emission, and user channels. Paint multiple channels in one material stroke, use per-channel blending, and configure Height → Normal and OpenGL/DirectX normals.
- **Projects and models**: new-project model, template, resolution, normal format, and baking settings; texture sets per material; adding, deleting, and resizing sets; replacing and reloading models. Import FBX and paint on poses made with bones and BlendShapes (poses are not saved).
- **3D preview and baking**: PBR Material, Neutral, and Channel views, environments, lighting, and tone mapping. GPU/CPU mesh map baking with progress, cancellation, and CPU fallback when the GPU is unavailable. Stale maps are identified. GPU and CPU baking results are not byte-identical.
- **Non-destructive effects**: add filters, mesh-map generators, and anchors to layers and masks, and edit them in the list and Properties. Noise/Grunge, fill images and projection, decals, and world-space gradients.
- **Asset shelf**: save and place Smart Materials and Smart Masks, import/export `.ylsmart`, and use 14 built-in Smart Materials. Unsupported assets are refused for placement with a reason. Switch between the project shelf and your personal library (the folder set as the library location; it has the same layout as the Unity version's library folder and lists PNG, `.ylsmart`, brush and material files, but not JPEG) to use images, smart assets, brushes and materials in the project (a copy is saved in the `.ylp`, so it opens without the library), put project items into the library, or remove them; images and smart assets can be placed into the document. Adding to the library skips a file whose bytes are already there (an image whose pixels match one already looked at is skipped too). A 16-bit PNG is reduced to 8 bits when used, and a note says so. Thumbnails are made on a separate thread and cached by content (the cache's size limits can be exceeded slightly between cleanups).
- **Saving and recovery**: open/save `.ylp`, retain backups of previous versions with configurable retention, and detect changes made by another application. Automatically write recovery generations and recover them on the next launch. The disk space recovery uses is capped (“Disk space” in the window; beyond it the oldest generations are removed, keeping the newest of this session and of each crashed session), and checkpoints are skipped when the disk is nearly full ([details](docs/RECOVERY.md)). Preserve original read-only set data and unknown additional entries when saving.
- **Image exchange**: import RGB8 PSD layers, groups, solid-color fills, supported adjustments, masks, clipping, and locks as a copy that never writes back to the original file (the file is read as a stream and the size limit follows the Layer pixels budget in Settings; layer effects, smart objects, unsupported adjustments and other things that cannot be kept or are ignored are listed by layer name before importing), and export one PSD per channel. Features PSD has no form for, such as filters, generators, images, and paths, are written as pixels, adjustments between PSD's steps are rounded, and Gradient Map value curves are expanded into stops. Everything done is listed by layer name in a check window before writing. Exported PSDs are RLE (PackBits) compressed and written one layer at a time, so large documents are written without doubling memory (limits follow the Layer pixels budget in Settings; a PSD side is at most 30000 pixels). Export individual or all channels as PNGs, use Unity Standard / URP Lit, HDRP Lit, and lilToon templates, and include UV padding and AO.
- **Live Link with Unity**: receive models from a Unity Editor running as the same user on the same PC and temporarily display painted colors on materials without modifying original assets. Both ends verify the connection key and exchange their versions and supported features. A version or feature mismatch keeps the link up: the mark at the right end of the menu bar turns to the warning color, and its tooltip names both versions, which side to update and the features that are unavailable (only protocol versions that do not overlap are refused). When the Unity material is a verified lilToon material, the standalone receives the real material values and the textures it does not paint (shadow colors, MatCaps and so on) and draws that texture set with the lilToon look in the 3D View (only the items changed in the panel keep the standalone's values, and the last values stay after disconnecting; a setting chooses whether received values are saved in the `.ylp`, and texture pixels are never saved). A newly created texture set starts from the Unity original texture (the Color target) as an "Original" layer at the bottom, so the model does not change when linked (Unity does not show the set until the original is in it) and you paint over it; sets that already have painting and sets from opened projects are left alone.
- **Interface and settings**: Japanese and English, dockable panels, 2D zoom/pan/rotation/flip, GPU compositing for supported documents with CPU fallback. Configure memory budgets, CPU threads, export padding, shelf location, and other settings. Settings live under Edit → Settings… (Ctrl+,) in a window split into sections, which also sets the 3D View orbit and zoom centers. Every list and panel scrolls the same way (wheel, handle, pressing the track), and pen press-and-drag goes through the same path as the mouse. The right end of the status bar shows the version, the build and the memory in use; the result or reason of the last operation appears as a small notice just above the bar for a few seconds (longer for refusals and failures) and goes away when pressed. The dock arrangement, the position and size of floating windows, and the window size, position and maximized state are saved when you quit and as they change (about once a second) and restored at the next start; an unreadable or inconsistent arrangement falls back to the default one.
- **Distribution and updates**: per-user Windows installer and portable ZIP. Startup update checks are opt-in; the installer version verifies downloads using signed update metadata before updating. Executables and installers are not yet code-signed. The installer, ZIP and tar.gz include the documentation (`docs`, Japanese and English), so it can be read offline.

Compatibility: `.ylp` files are saved in format 7. Edited sets normally use document format 21, format 22 with user channels, or format 23 with Noise/Grunge. Unity versions supporting document formats only up to 21 (such as 0.2.0) cannot open files containing formats 22 or 23. Documents with manual ID colors cannot be saved. See the [README](README.en.md) for details and feature limitations.
