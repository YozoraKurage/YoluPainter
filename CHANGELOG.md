# Changelog

## 0.6.0

### 日本語

- **定規**: 定規をレイヤー（グループも）に付けて、文書と一緒に保存するようにしました。種類は直線定規・平行線・同心円・パース・**対称**です。対称は、線の本数（2〜16）と線対称・回転対称で決まる定規で、斜めの軸や 6 本以上にも写せます。どの描くレイヤーで見えて効くかは、定規ごとの「表示の範囲」（すべてのレイヤー・同じグループの中・選んでいるときだけ）と、付けたレイヤーの目で決まります。レイヤーの一覧に「定規」のアイコンが出て、押す・`Shift` ＋クリックで表示を全部入り切り・ドラッグで別のレイヤーへ移す・右クリックで表示と範囲と削除ができます。プロパティに「定規」の区分（行ごとの表示・印・削除と、種類ごとの値）を追加しました。
- **スナップ**: 「定規にスナップ」（`Ctrl+1`、直線定規）と「特殊定規にスナップ」（`Ctrl+2`、平行線・同心円・パース・対称）に分けました。直線定規は、描き始めが定規から画面で 26 点以内のときだけ寄せます。特殊定規は、見えている中で印のある 1 つだけが効き（2D と 3D は別に 1 つずつ）、「スナップする特殊定規の切り替え」（`Ctrl+4`）で回せます。
- **対称**: ブラシの詳細・オプションバーの対称の欄と、アプリに 1 つの対称の設定を、対称定規に替えました。1 つのテクスチャセットの対称定規は、そのセットの文書に付きます。パスの対称は、入れるときに効いている対称定規の値を写し、斜め・6 本以上の線対称も写せます。
- **ツール**: 定規のツールのツールプロパティに、線の本数・線対称・角度の刻み・「編集レイヤーに作成」を追加しました。置いた定規は、つまみ（円・四角）で動かします。サブツールに「対称定規」と「回転対称」を追加しました。
- **互換**: 定規を持つ文書と、斜めの線対称・6 本以上の線対称のパスのある文書は、新しい版の番号（正本の版 35）で保存され、0.5.x までのスタンドアロンと Unity 版では開けません。使っていない文書は今までの版のままです（縦か横の向きの 2 本・4 本の線対称と回転対称のパスも前の版のまま）。なくなったこと: 同じ空間（2D か 3D）の中で、対称とパースなど特殊定規を同時に使うこと（2D と 3D は別に 1 つずつ数えるので、2D の対称と 3D の対称の同時は残ります）、テクスチャセットをまたぐ対称、軸の線だけを隠して対称で描くこと、オプションバーのボタン 1 つでの対称の入り切り（対称定規を置き、「特殊定規にスナップ」で入り切りします）、直線定規へどこから描き始めても寄ること（描き始めが近いときだけ寄せます）、ツールで種類を替えると置いてある定規も替わること（ツールの種類は、これから作る定規のものです）。

### English

- **Rulers**: Rulers now belong to a layer (a group too) and are saved with the document. The kinds are Straight Ruler, Parallel, Concentric, Perspective and **Symmetry**. A symmetry ruler is decided by its number of lines (2 to 16) and line or rotational symmetry, and it can copy across diagonal axes and 6 or more lines. Where a ruler is seen and takes effect depends on its Visible In setting (All Layers, Within the Same Group, Only While Selected) and the eye of the layer it is on. The layer list shows a Ruler icon: click it, `Shift`+click to show or hide all, drag it onto another layer to move the rulers, or right-click for Show, Visible In and Delete. Properties gained a Ruler section (per-row eye, mark and delete, and the values for each kind).
- **Snap**: split into Snap to Ruler (`Ctrl+1`, straight rulers) and Snap to Special Ruler (`Ctrl+2`, Parallel, Concentric, Perspective and Symmetry). A straight ruler pulls only when the stroke starts within 26 points on the screen of the ruler. Of the special rulers you can see, only the one with the mark takes effect (one for 2D and one for 3D, counted separately), and Switch the Snapping Special Ruler (`Ctrl+4`) cycles through them.
- **Symmetry**: the Symmetry category of Brush Details, the symmetry control on the options bar and the app-wide symmetry setting are replaced by symmetry rulers. A texture set's symmetry rulers belong to that set's document. A path's symmetry copies the values of the symmetry ruler in effect when you turn it on, and can copy diagonal and 6-or-more-line symmetry too.
- **Tools**: the Ruler tool's tool properties gained Lines, Line Symmetry, Angle Step and Create on Edit Layer. Move a placed ruler by its handles (circles and squares). Sub tools gained Symmetry Ruler and Rotational Symmetry.
- **Compatibility**: a document with rulers, or with paths that use a diagonal line symmetry or a line symmetry of 6 or more lines, is saved under a newer version number (canonical version 35) that the standalone app up to 0.5.x and the Unity version cannot open. Documents that use neither keep their version (paths with a 2- or 4-line symmetry along the vertical or horizontal direction, and rotational symmetry, keep it too). What is gone: using two special rulers, such as symmetry and perspective, together in one space (2D or 3D; the two spaces are counted separately, so a 2D and a 3D symmetry can still be used together), symmetry across texture sets, painting with symmetry while only its axis lines are hidden, turning symmetry on and off with one button on the options bar (place a symmetry ruler and turn it on and off with Snap to Special Ruler), a straight ruler pulling a stroke that starts anywhere (it pulls only when the stroke starts near it), and changing the kind in the tool also changing the rulers already placed (the kind in the tool is for the rulers you create next).

## 0.5.0

### 日本語

- **テキストツール**（`T`）: キャンバスで打ってテキストレイヤーを作り、あとからフォント・サイズ・色・行間・字間・揃え・折り返しを直せます。フォントは同梱の 2 つ・PC に入っているフォント・ファイルから選べます。フォントは .ylp に入れず、見つからないときは描いた画素のまま見せます。ステンシルを押したまま動かすキーは `Y` に移りました。
- **効果**: フィルターを 10 種（値の切り出し・値の幅・ノイズに沿ったぼかし・方向ぼかし・ゆがみ・太らせる・細らせる・輪郭の検出・ハイパス・メディアン・グロー）、ジェネレーターを 5 種（模様・ライト・マスクの組み立て・画像・アイランドごとのばらつき）追加しました。「フィルターを追加」と「ジェネレーターを追加」の入り口を分け、レイヤーとマスクのどちらに付けるかはサムネイルで選びます。ぼかし・シャープは UV の継ぎ目をまたいで、3D で隣の面の画素を読みます。
- **パス**: 1 つのレイヤーに何本ものパス・角と取っ手・種類（ストローク・塗り・指先・消しゴム・リボン）・筆先・対称。塗りつぶしレイヤーにもパスを置けます。
- **塗りつぶし**: 点のグラデーション（3D ビューと 2D で点を置く）、画像の異方性のフィルター、移動・回転・大きさを 1 つにまとめたギズモ。
- **ツールの並び**: 左のツールバーとブラシのグループを、追加・削除・名前の変更・並べ替え・ほかのツールへの移動で自由に組み替えられます。並びは設定のフォルダの `tools.json` に残ります（0.4.x の並びは初回に引き継ぎます）。
- **別ウィンドウ**: ドックのパネルを OS の別ウィンドウへ出して、別のモニターに置けます。メニューバーに「ウィンドウ」を追加し、パネルの一覧と「パネルの並びを戻す」（「表示」から移しました）を置きました。
- **アクション**: レイヤー・マスク・効果の操作を、コマンドライン・MCP と同じ命令の列として記録し、取り消し 1 回で戻せる形で再生します（「ウィンドウ」→「アクション」）。
- **アセット**: 塗りつぶしレイヤーをマテリアルとしてライブラリへ保存し、置けます。プロジェクトの品は「アセット」、自分のフォルダは「ライブラリ」と呼ぶようにしました。
- **ポーズ**: FBX の中のテイク（アニメ）とフレームを選んで、そのポーズにできます。
- **重なった UV**: ベイクで重なったテクセルにどのアイランドの値を焼くかを選べます。重なりを 2D のキャンバスとベイクのウィンドウで見られます。
- **Live Link**: ファイルの受け渡しに作り直しました。Unity は FBX の場所と値を渡し、スタンドアロンが FBX と絵を自分で読みます。Color の元の絵が PSD ならレイヤーのまま入れ、送り直しで元の絵が変わったら、触っていないテクスチャセットへ入れ直します。**Unity のブリッジも 0.5.0 に上げてください**（Unity の中で描く機能は外し、Live Link とマテリアルへの適用だけのパッケージになりました）。
- **MCP**: アプリが `127.0.0.1` の HTTP で MCP を受けます（設定で入れたときだけ）。Claude Code・Codex のプラグインと .mcpb は、起動しているアプリへつなぎます。
- **大きな文書**: メモリの上限を超えたタイルをディスクへ逃がして、続けて描けます。
- **知らせとログ**: 知らせに種類と出どころを付け、断り・失敗は「何が（なぜ）」の 1 文にしました。注意と失敗は「ログ」のパネルに残ります。
- **色のウィンドウ**: 色の値の欄を押すと、その場で色相の円・16 進・描画色・カラーセットから当てられます。
- **速さ**: 全レイヤーの結合・PSD の取り込み・PNG の書き出し・PSD の保存の確かめ・合成（1 スレッドで 1.6〜2.2 倍）・Normal のチャンネル・カラーバランス・色相/彩度・ブラシを速くしました。
- **描き心地**: 描いている間にパネルが灰色になってチカチカしないようにしました。キャンバスと 3D ビューを並べているときも、手ぶれ補正などの 2D の設定を変えられます。描いている線と画面の反応の遅れを少し縮めました。
- **言語**: 初めての起動は OS の言語に合わせ、日本語でなければ英語で始めます。
- **そのほか**: 3D ビューの表示のテクスチャを UV の外へ塗り広げ、離れて見てもアイランドの縁がにじまないようにしました。光の強さ 1 は Unity のディレクショナルライトと同じ明るさです。Linux では Wayland の机でも X11（XWayland）で開きます。手動の ID の色を .ylp に保存できるようになりました。
- **互換**: 新しい機能（テキストレイヤー・パスの一覧・0.5.0 の効果・点のグラデーション・継ぎ目の設定・ベイクの優先）を使った .ylp は新しい版で保存され、0.4.x と Unity 版では開けません。使っていない文書は今までどおりの版で保存します。合成の計算を変えたので、保存済みの文書の合成モードを重ねた所で、ごく一部の画素が 1 段変わることがあります。大きな PNG の書き出し（おおよそ 512 × 512 以上）はバイト列が変わります（画素は同じ）。

### English

- **Text tool** (`T`): Type on the canvas to create a text layer, and change its font, size, color, line spacing, tracking, alignment and wrapping later. Pick a bundled font, a font installed on your PC, or a font file. Fonts are not stored in the .ylp; when a font cannot be found, the drawn pixels are kept. Holding the key to move the stencil is now `Y`.
- **Effects**: 10 new filters (Histogram Scan, Histogram Range, Slope Blur, Directional Blur, Warp, Dilate / Erode, Edge Detect, High Pass, Median, Glow) and 5 new generators (Pattern, Light, Mask Builder, Image, UV Island Variation). Add Filter and Add Generator are separate entrances, and thumbnails choose whether an effect goes on the layer or its mask. Blur and sharpen read across UV seams from the neighboring faces in 3D.
- **Paths**: Several paths per layer, corners and handles, kinds (stroke, fill, smudge, eraser, ribbon), tips and symmetry. Fill layers can have paths too.
- **Fill layers**: Point gradients (place points in the 3D view or in 2D), anisotropic filtering for images, and one gizmo that moves, rotates and scales.
- **Tool layout**: Freely add, remove, rename, reorder and move the tools on the left toolbar and the brush groups between tools. The layout is kept in `tools.json` in the settings folder (the 0.4.x order is carried over on first start).
- **Separate windows**: Move dock panels into separate OS windows and place them on another monitor. A new Window menu lists the panels and holds Reset Panel Layout (moved from View).
- **Actions**: Record layer, mask and effect operations as a list of the same commands as the command line and MCP, and play them back as one undo step (Window → Actions).
- **Assets**: Save a fill layer as a material in the library and place it. Project items are now called Assets, and your own folder is the Library.
- **Pose**: Pick a take (animation) and frame inside the FBX to use that pose.
- **Overlapping UVs**: Choose which island's value bakes into overlapping texels, and see the overlaps on the 2D canvas and in the bake window.
- **Live Link**: Rebuilt on file exchange. Unity passes the FBX location and values, and the standalone reads the FBX and images itself. A PSD as the Color source comes in with its layers, and when the source image changes on resend, untouched texture sets are refilled. **Update the Unity bridge to 0.5.0 as well** (painting inside Unity was removed; the package now only does Live Link and applying to materials).
- **MCP**: The app serves MCP over HTTP on `127.0.0.1` (only when enabled in the settings). The Claude Code and Codex plugins and the .mcpb connect to the running app.
- **Large documents**: Tiles beyond the memory limit move to disk, so you can keep painting.
- **Notices and log**: Notices show their kind and source, refusals and failures are one sentence saying what and why, and warnings and failures stay in the Log panel.
- **Color window**: Clicking a color value opens a color window right there, with the hue wheel, hex, the paint colors and color sets.
- **Speed**: Faster merging of all layers, PSD import, PNG export, PSD save verification, compositing (1.6–2.2× on one thread), Normal channels, color balance, hue/saturation and brushes.
- **Painting feel**: Panels no longer flicker gray while you paint. Stabilizer and other 2D settings can be changed while the canvas and the 3D view are side by side. The delay between your input and the stroke and UI on screen is a little shorter.
- **Language**: The first start follows the OS language, and starts in English unless it is Japanese.
- **Other**: The 3D view's display textures are padded beyond the UVs so island edges do not bleed from a distance. Light intensity 1 matches a Unity directional light. On Linux, the app opens through X11 (XWayland) on Wayland desktops. Manual ID colors are now saved in the .ylp.
- **Compatibility**: A .ylp that uses the new features (text layers, path lists, 0.5.0 effects, point gradients, the seam setting, bake priority) is saved in a newer version that 0.4.x and the Unity version cannot open. Documents that do not use them keep their version. Because compositing changed, a very small number of pixels where blend modes stack in saved documents may change by one step. Large PNG exports (roughly 512 × 512 and up) have different bytes (the same pixels).

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
