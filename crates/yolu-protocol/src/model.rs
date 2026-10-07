//! モデルの形: メッシュ（頂点・法線・UV0・三角形・サブメッシュ）、マテリアルの組（名前・安定した鍵・シェーダーとテクスチャの
//! プロパティの名前・Unity 側が見せられるチャンネル）、ポーズの変化（焼いたメッシュの位置）。アプリの中で 3D ビューとテクスチャセットの
//! 結びつけが共有する型で、通信の形ではない（Live Link の受け渡しは `files`）。
//!
//! 位置は Unity の座標（左手系、メートル）で、読み込んだモデルの根のゲームオブジェクトのローカルの空間。三角形の巻きは、鏡に映した
//! （行列式が負の）レンダラーでも表を同じ向きにそろえる。

/// Unity 版のチャンネル（C# の PaintChannel・yolu-core の Channel と同じ番号）。
pub mod channel {
    pub const COLOR: u8 = 0;
    pub const ROUGHNESS: u8 = 1;
    pub const METALLIC: u8 = 2;
    pub const HEIGHT: u8 = 3;
    /// 合成ではなく Normal の出力（接空間・OpenGL の Y+・不透明。R = x・G = y・B = z・A = 255）。
    pub const NORMAL: u8 = 4;
    pub const EMISSION: u8 = 5;
    pub const COUNT: u8 = 6;
}

/// モデルのマテリアルの組を指す鍵（Unity 版の .ylp の形式 7 の `material` と同じ考え方）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKey {
    /// マテリアルの無いスロットの全部。
    Unassigned,
    /// マテリアル。アセットなら GUID（小文字の 16 進 32 文字）と localFileId。
    Material {
        name: String,
        asset: Option<(String, i64)>,
    },
}

/// シェーダーの 2D テクスチャのプロパティと、マテリアルに今入っているテクスチャの大きさ（無ければ 0）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureProperty {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// Unity 側が見せられるチャンネルと、その流し込み先のプロパティ（YoluPainter の流し込みの決まりで決めたもの）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRoute {
    pub channel: u8,
    pub property: String,
}

/// マテリアルの組 1 つの情報。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialInfo {
    pub key: MaterialKey,
    /// シェーダーの名前（マテリアルが無ければ空）。
    pub shader: String,
    pub textures: Vec<TextureProperty>,
    /// Unity 側が見せられるチャンネル（ここに無いチャンネルを描いても Unity には見えない）。
    pub routes: Vec<ChannelRoute>,
}

/// 三角形の組 1 つ（1 つのマテリアルで描く）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submesh {
    /// モデルのマテリアルの並びの番号。
    pub material: u32,
    /// 三角形の頂点の添字（3 つずつ）。
    pub indices: Vec<u32>,
}

/// レンダラー 1 つのメッシュ（スキンメッシュは読んだ時のポーズで焼いた形）。
#[derive(Clone, Debug, PartialEq)]
pub struct MeshData {
    /// モデルの中で安定した鍵（根からの子の番号の道。名前が重なっても違う）。
    pub key: String,
    /// レンダラーのゲームオブジェクトの名前（表示用）。
    pub name: String,
    pub skinned: bool,
    pub positions: Vec<[f32; 3]>,
    /// 空か、positions と同じ数。
    pub normals: Vec<[f32; 3]>,
    /// 空か、positions と同じ数。
    pub uv0: Vec<[f32; 2]>,
    pub submeshes: Vec<Submesh>,
}

/// モデルの全体。前のモデルを置き換える。
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// モデルの世代（読み直すたびに増やす）。`Pose`・`MaterialsUpdate` はこの番号で同じモデルを指す。
    pub generation: u32,
    pub name: String,
    pub materials: Vec<MaterialInfo>,
    pub meshes: Vec<MeshData>,
}

/// 1 つのメッシュの新しい形。
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPose {
    /// `Model` の `meshes` の番号。
    pub mesh: u32,
    /// そのメッシュの頂点と同じ数。
    pub positions: Vec<[f32; 3]>,
    /// 空か、positions と同じ数。
    pub normals: Vec<[f32; 3]>,
}

/// ポーズの変化（変わったメッシュだけ）。
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub generation: u32,
    pub meshes: Vec<MeshPose>,
}

/// マテリアルの情報の更新（シェーダーの差し替えなど。数と並びは `Model` と同じ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialsUpdate {
    pub generation: u32,
    pub materials: Vec<MaterialInfo>,
}
