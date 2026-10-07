//! 層のフィルターが UV の継ぎ目をまたぐのに使う、モデルの UV の位相の持ち方（`fx::inputs` の `refresh_topologies`）: ポーズで位置だけ
//! 変わったモデルでは前の位相を使い続け、UV の並びが違うモデルでは、前のモデルが解放されて同じ番地に別のモデルが作られても作り直す。
use std::sync::Arc;

use yolu_app::state::AppState;
use yolu_app::view3d::model::ViewModel;
use yolu_core::geometry::{ModelMesh, Submesh, UvTopology};
use yolu_core::glam::{Vec2, Vec3};

/// 四角 1 枚のモデル。`shift` は位置、`uv` は UV の大きさ（違えば UV の並びが違う）。Live Link の世代を付けると、世代を指して閉じられる。
fn plate(name: &str, shift: f32, uv: f32, generation: Option<u32>) -> ViewModel {
    let p = |x: f32, y: f32| Vec3::new(shift + x, y, 0.0);
    let t = |x: f32, y: f32| Vec2::new(0.1 + uv * x, 0.1 + uv * y);
    let mesh = ModelMesh {
        name: name.into(),
        positions: vec![p(0., 0.), p(1., 0.), p(0., 1.), p(1., 1.)],
        normals: Vec::new(),
        uvs: vec![t(0., 0.), t(1., 0.), t(0., 1.), t(1., 1.)],
        submeshes: vec![Submesh {
            material: 0,
            indices: vec![0, 2, 1, 2, 3, 1],
        }],
    };
    let mut model = ViewModel::new(name, vec![mesh], vec![Some("材".into())], 1).unwrap();
    model.link_generation = generation;
    model
}

/// 位相を作らせて（効果の入力を見る）、文書が受けた位相を返す。
fn topology_of(state: &mut AppState) -> Arc<UvTopology> {
    state.sync_effect_inputs_with(true);
    state
        .doc
        .effect_inputs()
        .topology()
        .cloned()
        .expect("モデルがあれば文書に位相が渡る")
}

#[test]
fn a_model_with_the_same_layout_keeps_the_topology() {
    let mut s = AppState::new(64, 64);
    s.view3d.set_model(plate("板", 0.0, 0.3, None));
    let first = topology_of(&mut s);
    // 位置だけが違う（ポーズ）: 同じ位相のまま（島・帯の写しを作り直さない）
    s.view3d.set_model(plate("板", 5.0, 0.3, None));
    let moved = topology_of(&mut s);
    assert!(Arc::ptr_eq(&first, &moved));
    // 同じモデルを見続けても同じ
    assert!(Arc::ptr_eq(&moved, &topology_of(&mut s)));
}

#[test]
fn another_layout_gets_a_new_topology_even_when_the_old_model_is_gone() {
    // 閉じて（前のモデルを手放して）すぐ別のモデルを読む。同期の前に起きるので、前のモデルの番地が使い回されうる
    for round in 0..8u32 {
        let mut s = AppState::new(64, 64);
        s.view3d.set_model(plate("板", 0.0, 0.3, Some(round)));
        let first = topology_of(&mut s);
        s.view3d.close_live_link(round);
        s.view3d.set_model(plate("別の板", 0.0, 0.5, None));
        let second = topology_of(&mut s);
        let model = s.view3d.full_model().unwrap().clone();
        assert!(!Arc::ptr_eq(&first, &second), "round {round}");
        assert!(
            Arc::ptr_eq(second.geometry(), &model.geometry),
            "round {round}: 位相が今のモデルの形から作られていない"
        );
        assert!(!first.same_layout(second.geometry()), "round {round}");
    }
}
