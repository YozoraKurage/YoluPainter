//! 試しのスタンドアロンを別のプロセスで起こし、モデルを送ると模様のテクスチャセットが共有メモリで届き、描き足した所だけが返ること。

use std::process::{Command, Stdio};
#[path = "../../yolu-protocol/tests/support/wait.rs"]
mod wait;
use wait::ChildGuard;

use yolu_protocol::*;

fn material(name: &str, color_route: bool) -> MaterialInfo {
    MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: 256,
            height: 256,
        }],
        routes: if color_route {
            vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }]
        } else {
            vec![]
        },
    }
}

#[test]
fn the_demo_paints_each_material_and_returns_only_changed_tiles() {
    let name = format!("ylp-demo-test-{}", std::process::id());
    let mut child = ChildGuard::spawn(
        Command::new(env!("CARGO_BIN_EXE_yolu-link-demo"))
            .args(["--name", &name, "--animate", "--once"])
            .stdout(Stdio::null()),
    );
    let (conn, mut reader, _) = wait::connect(&name, Some(&mut child));
    conn.send(&Message::Model(Model {
        generation: 1,
        name: "試し".into(),
        materials: vec![material("Body", true), material("NoColor", false)],
        meshes: vec![MeshData {
            key: "0".into(),
            name: "Tri".into(),
            skinned: false,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            submeshes: vec![
                Submesh {
                    material: 0,
                    indices: vec![0, 1, 2],
                },
                Submesh {
                    material: 1,
                    indices: vec![0, 2, 1],
                },
            ],
        }],
    }))
    .unwrap();
    let mut set: Option<(TextureSet, SharedImageReader)> = None;
    let mut first_tiles = 0usize;
    let mut later: Vec<usize> = Vec::new();
    while later.len() < 3 {
        match reader.next("デモのセット・下地・3回の描き足し", Some(&mut child)) {
            Received::Message(Message::TextureSet(s)) => {
                assert!(
                    set.is_none(),
                    "Color の流し込み先の無いマテリアルには作らない"
                );
                assert_eq!((s.material, s.width, s.height), (0, 256, 256));
                let img =
                    SharedImageReader::open(std::path::Path::new(&s.channels[0].path)).unwrap();
                set = Some((s, img));
            }
            Received::Message(Message::TilesChanged(t)) => {
                if first_tiles == 0 {
                    first_tiles = t.tiles.len();
                } else {
                    later.push(t.tiles.len());
                }
            }
            Received::Idle => {}
            other => panic!("{other:?}"),
        }
    }
    // 最初は下地で全部（2 × 2 タイル）、描き足しは一部だけ
    assert_eq!(first_tiles, 4);
    assert!(later.iter().all(|n| (1..4).contains(n)), "{later:?}");
    conn.send(&Message::Bye).unwrap();
    assert!(child.finish().success());
    let (_, img) = set.unwrap();
    let mut image = vec![0u8; 256 * 256 * 4];
    for y in 0..2 {
        for x in 0..2 {
            assert_eq!(
                img.read_tile_into_image(x, y, &mut image).unwrap(),
                TileRead::Complete,
                "書き手の終了後は競合しない"
            );
        }
    }
    // 対角の太い線の真ん中はマテリアルの色、隅は下地（色を薄めたもの）。どれも不透明
    let px = |x: usize, y: usize| image[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].to_vec();
    let mid = px(77, 77);
    assert!(
        mid[0].abs_diff(220) <= 2 && mid[1].abs_diff(50) <= 2 && mid[2].abs_diff(60) <= 2,
        "{mid:?}"
    );
    assert_eq!(px(250, 5)[3], 255);
    assert!(px(250, 5)[0] > 230, "{:?}", px(250, 5));
}
