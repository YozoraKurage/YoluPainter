//! 3D モデルのファイルを読み、yolu-core のスキン（`yolu_core::skin::Rig`: 骨・メッシュ・UV・サブメッシュ・ウェイト・BlendShape）に
//! する。今は FBX だけ（`fbx`。C の ufbx を使う）。
//!
//! - 読むだけ。元のファイルは書き換えない（読み込みの間も開くのは読むためだけ）。
//! - 座標は Unity の FBX の読み込みと同じ: メートルに直し、右手系の Y が上（+Z が前）にそろえてから X を反転して左手系にし、
//!   三角形の巡りを逆にする（Unity の表 = 外から見て時計回り）。UV は FBX のまま（v = 0 が下）。
//! - ポリゴンは三角形に分け、位置・法線・UV の組が違う所で頂点を分ける（Unity と同じ考え方）。ウェイトと BlendShape は元の頂点
//!   （コントロールポイント）から分けた頂点へ写す。
//! - 大きすぎる・壊れたファイルは `ModelLimits` で断る（ファイルの大きさ・パーサーのメモリ・ノードの深さ・スキンの予算）。

pub mod fbx;

pub use fbx::{load_fbx, load_fbx_bytes, LoadReport, LoadedModel, ModelError, ModelLimits};
