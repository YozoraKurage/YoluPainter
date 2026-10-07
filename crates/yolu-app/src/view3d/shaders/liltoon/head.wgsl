// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の Shader/Includes から移した式（部品のつなぎ方は view3d/liltoon_wgsl.rs）。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
// lilToon の再現。scene.wgsl の後ろにつないで 1 つのモジュールにする（一様バッファ・束ね・色の変換・影は scene.wgsl のもの）。
//
// 式は lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の Shader/Includes から移した: lil_pass_forward_normal.hlsl の frag の順、
// lil_common_frag.hlsl の lilGetMain2nd・lilGetMain3rd・lilGetShading・lilGetRimShade・lilBacklight・lilReflection・lilCalcSpecular・
// lilGetMatCap・lilGetMatCap2nd・lilGetRim・lilGlitter・lilEmission・lilEmission2nd・lilDistanceFade・アルファマスク・ノーマルマップ 2nd・
// 異方性反射・輪郭線の色、lil_common_functions.hlsl の lilTooning*・lilToneCorrection・lilBlendColor・lilBlendNormal・lilCalcMatCapUV・
// lilUnpackNormalScale・lilCalcDecalUV・lilCalcAtlasAnimation・lilGetSubTex・lilIsIn0to1・lilMSDF・lilVoronoi・lilCalcGlitter・
// lilGetAnisotropyNormalWS・lilFresnelTerm・lilFresnelLerp・GSAA・lilGetOutlineWidth・lilCalcOutlinePosition、
// lil_common_functions_thirdparty.hlsl の lilHashRGB4、ビルトインのレンダーパイプラインの光（openlit_core.hlsl の ComputeLights。
// OpenLit は CC0）と lil_common_macro.hlsl の LIL_CORRECT_LIGHTCOLOR。機能の入切は、エディタの既定（全部の機能を組み込んだ版）と同じ。
// 時間で動く値（スクロール・回転の速さ・点滅・デカールのアニメーション・ラメの点滅）は時刻 0 の値で描く。
//
// 再現しないもの（値は持つが描かない）: グラデーションマップ、ディゾルブ、ディザー、視差、AudioLink、ID マスク、ファー・宝石・屈折、
// 影色の LUT、UV1〜UV3（モデルは UV0 だけ。UV0 で読む）、頂点カラー、ラメの形のテクスチャ、環境光の反射のキューブマップの差し替え、
// 追加のライト（頂点ライト・ForwardAdd）、ライトマップ、霧。
//
// 色の約束: lilToon はリニアで解き、最後に scene.wgsl と同じくガンマにして書く。半透明は、描き先を sRGB の見え方にして（`*_linear` の
// 入り口）リニアの乗算済みで重ねる（Unity と同じ重ね方）。トーンマッピングのとき（描き先が HDR）は、ガンマの値のまま重ねる。
//
// パイプラインの定数（`LIL_FEATURES`・`LIL_SINGLE*`・`LIL_LOOP*`）: ソフトの描画（llvmpipe）は一様な分岐の先も全部実行するので、
// 入にしていない機能と、割り当ての無いスロットの読み方を、パイプラインを作るときに外す（render.rs）。既定は全部入りで、実機は
// その 1 本を一様な分岐で描く（入切のたびにシェーダーを作り直さない）。どちらも同じ値の分岐を通るので、描く絵は同じ。

