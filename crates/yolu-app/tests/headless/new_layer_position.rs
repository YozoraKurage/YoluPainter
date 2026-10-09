//! 「新規レイヤー」の並び（メニューの「レイヤー」・レイヤーの一覧の右クリック）の入り口が、選んでいるレイヤーのすぐ上（グループの中を選んでいれば
//! そのグループの中）に新しいレイヤーを作る試験。何も選んでいなければ一番上。取り消し 1 回で戻る。
use yolu_app::engine::LayerId;
use yolu_app::lang::Lang;
use yolu_app::layermenu::Op;
use yolu_app::m2::{AdjustmentKind, Edit};
use yolu_app::state::{Action, AppState};
use yolu_core::fill_image::ProjectionMode;
use yolu_core::generator::Shape;

/// 下から a・b・c の 3 つ。選んでいるのは a（一番下）。
fn three() -> (AppState, [LayerId; 3]) {
    let mut s = AppState::new(64, 64);
    let a = s.selected_layer.expect("最初のレイヤー");
    s.apply(Action::NewLayer);
    let b = s.selected_layer.unwrap();
    s.apply(Action::NewLayer);
    let c = s.selected_layer.unwrap();
    s.selected_layer = Some(a);
    (s, [a, b, c])
}

fn ids(s: &AppState) -> Vec<LayerId> {
    s.doc.layers().iter().map(|l| l.id()).collect()
}

fn index(s: &AppState, id: LayerId) -> usize {
    ids(s).iter().position(|x| *x == id).expect("レイヤー")
}

/// 画像（2 × 2）をアセットへ入れて、レイヤーの画像の ID を返す。
fn shelf_image(s: &mut AppState) -> yolu_core::ImageId {
    let rid = s
        .shelf
        .add_image(Lang::Ja, "画像", &[255u8; 16], 2, 2)
        .expect("アセットへ入る");
    yolu_app::fillfx::inputs::image_id(&rid).expect("GUID")
}

/// 新しいレイヤーを作る操作の一覧（名前つき）。
fn creations(s: &mut AppState) -> Vec<(String, Action)> {
    let image = shelf_image(s);
    let mut v = vec![
        ("新規レイヤー".to_owned(), Action::NewLayer),
        ("グループ".to_owned(), Action::M2(Edit::NewGroup)),
        ("単色の塗りつぶし".to_owned(), Action::M2(Edit::NewFill)),
        (
            "グラデーションデカール".to_owned(),
            Action::LayerMenu(Op::FillGradient(Shape::Box)),
        ),
        (
            "画像の塗りつぶし".to_owned(),
            Action::LayerMenu(Op::FillImage {
                image,
                mode: ProjectionMode::Uv,
            }),
        ),
        (
            "デカール".to_owned(),
            Action::LayerMenu(Op::FillImage {
                image,
                mode: ProjectionMode::Decal,
            }),
        ),
    ];
    v.push(("パスレイヤー".to_owned(), Action::LayerMenu(Op::PathLayer)));
    for kind in AdjustmentKind::ALL {
        v.push((
            format!("調整レイヤー {kind:?}"),
            Action::M2(Edit::NewAdjustment(kind)),
        ));
    }
    v
}

#[test]
fn every_new_layer_entrance_makes_the_layer_right_above_the_selected_one() {
    let (mut probe, _) = three();
    let names: Vec<String> = creations(&mut probe).into_iter().map(|(n, _)| n).collect();
    for (i, name) in names.iter().enumerate() {
        let (mut s, [a, b, c]) = three();
        let action = creations(&mut s).swap_remove(i).1;
        let before = ids(&s);
        s.apply(action);
        let made = s.selected_layer.expect("作ったレイヤーを選ぶ");
        assert!(!before.contains(&made), "{name}: 新しいレイヤーを選ぶ");
        assert_eq!(
            ids(&s)
                .into_iter()
                .filter(|id| *id != made)
                .collect::<Vec<_>>(),
            before,
            "{name}: ほかの並びは変わらない"
        );
        assert_eq!(index(&s, made), index(&s, a) + 1, "{name}: a のすぐ上");
        assert!(index(&s, made) < index(&s, b) && index(&s, b) < index(&s, c));
        // 取り消し 1 回で戻る
        s.apply(Action::Undo);
        assert_eq!(ids(&s), before, "{name}: 取り消し 1 回");
    }
}

#[test]
fn inside_a_group_every_new_layer_entrance_makes_it_in_the_same_group_above_the_selected_layer() {
    let (mut probe, _) = three();
    let names: Vec<String> = creations(&mut probe).into_iter().map(|(n, _)| n).collect();
    for (i, name) in names.iter().enumerate() {
        let (mut s, [a, _b, c]) = three();
        // a を新しいグループの中へ（グループの中に a だけ）
        s.apply(Action::M2(Edit::NewGroup));
        let group = s.selected_layer.unwrap();
        s.apply(Action::M2(Edit::Move {
            id: a,
            parent: Some(group),
            position: 0,
        }));
        s.selected_layer = Some(a);
        let action = creations(&mut s).swap_remove(i).1;
        let before = ids(&s);
        s.apply(action);
        let made = s.selected_layer.expect("作ったレイヤーを選ぶ");
        assert!(!before.contains(&made), "{name}: 新しいレイヤーを選ぶ");
        assert_eq!(
            s.doc.layer(made).unwrap().parent(),
            Some(group),
            "{name}: グループの中"
        );
        let siblings = s.doc.children_of(Some(group)).unwrap();
        // children_of は下から。a のすぐ上 = a の 1 つ後
        let at = siblings.iter().position(|x| *x == a).unwrap();
        assert_eq!(
            siblings.get(at + 1),
            Some(&made),
            "{name}: a のすぐ上 {siblings:?}"
        );
        assert_eq!(s.doc.layer(c).unwrap().parent(), None);
        s.apply(Action::Undo);
        assert_eq!(ids(&s), before, "{name}: 取り消し 1 回");
    }
}

#[test]
fn with_nothing_selected_a_new_layer_goes_on_top() {
    for i in 0..creations(&mut three().0).len() {
        let (mut s, [_, _, c]) = three();
        let action = creations(&mut s).swap_remove(i).1;
        s.selected_layer = None;
        s.apply(action);
        let made = s.selected_layer.expect("作ったレイヤーを選ぶ");
        assert_eq!(index(&s, made), ids(&s).len() - 1, "{i}: 一番上");
        assert_eq!(index(&s, c), ids(&s).len() - 2);
    }
}
