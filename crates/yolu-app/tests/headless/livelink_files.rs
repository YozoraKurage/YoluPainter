//! Live Link のファイルの受け渡し（スタンドアロンの側、画面なし）: 試しのフォルダに Unity と同じ手順で頼みを置き、開く・送り直し・断り・
//! 待つ・書き出しの返事・.ylp への保存と開き直しを確かめる。FBX は yolu-model の試験と同じ ASCII の腕（実のデータは使わない）。
use crate::common::livelink::{arm_request, fbx_ascii, material_key, slash, Exchange};
use crate::common::wait::WATCHDOG;

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use yolu_app::livelink::{LinkStatus, LiveLink};
use yolu_app::state::{Action, AppState};
use yolu_app::view3d::pose;
use yolu_core::glam::{Quat, Vec3};
use yolu_core::look::LookKind;
use yolu_protocol::files::{Reason, Reply, ReplyKind};

/// 画面を使わずに、AppState と LiveLink を画面のフレームと同じ順（拾う・当てる → 書き出しの返事）で回す。
struct Headless {
    state: AppState,
    link: LiveLink,
    ex: Exchange,
}

impl Headless {
    fn new(tag: &str) -> Headless {
        let ex = Exchange::new(tag);
        let mut link = LiveLink::new();
        link.set_folder(ex.root.clone()).unwrap();
        let mut state = AppState::new(64, 64);
        state.prefs.settings.livelink_on_startup = true;
        let mut h = Headless { state, link, ex };
        h.frame();
        assert_eq!(h.state.link.status, LinkStatus::Accepting);
        h
    }

    fn frame(&mut self) {
        self.link.poll(&mut self.state);
        if let Some(files) = self.state.export.take_finished() {
            self.link.exported(&mut self.state, &files);
        }
        self.state.link = self.link.view(&self.state);
    }

    fn until(&mut self, what: &str, mut cond: impl FnMut(&mut Headless) -> bool) {
        let deadline = Instant::now() + WATCHDOG;
        loop {
            self.frame();
            if cond(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what} を待ったが来ない: {}",
                self.state.message
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// 次の返事を待つ。
    fn reply(&mut self) -> Reply {
        let mut got = None;
        self.until("返事", |h| {
            got = h.ex.take_replies().into_iter().next();
            got.is_some()
        });
        got.unwrap()
    }

    /// 頼みを置き、拾って裏の仕事を始めるところまでフレームを進める（結果を入れるのは次のフレームの頭。その前に試験が文書を替える）。
    fn start_job(&mut self, request: &serde_json::Value) {
        self.ex.put(request);
        self.until("裏の仕事の開始", |h| {
            !h.ex.folder().claimed_files().unwrap().is_empty()
        });
        assert!(self.link.is_working());
    }

    /// 何も起きないことを見るために、受け付けの間隔より長くフレームを進める。
    fn settle(&mut self) {
        let until = Instant::now() + Duration::from_millis(800);
        while Instant::now() < until {
            self.frame();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn session_rig(&self) -> Arc<yolu_core::skin::Rig> {
        self.state.view3d.pose.session.as_ref().unwrap().rig.clone()
    }

    fn bone(&self, name: &str) -> usize {
        self.session_rig()
            .bones()
            .iter()
            .position(|b| b.name == name)
            .unwrap()
    }

    fn pose(&self) -> yolu_core::skin::Pose {
        self.state
            .view3d
            .pose
            .session
            .as_ref()
            .unwrap()
            .pose()
            .clone()
    }

    /// 腕の頼み（id・相手の鍵）を置く。
    fn arm(&mut self, id: &str, key: &str) -> serde_json::Value {
        let fbx = self.ex.dir.join("arm.fbx");
        if !fbx.exists() {
            self.ex.write_arm("arm.fbx");
        }
        let png = self.ex.dir.join("skin.png");
        if !png.exists() {
            self.ex.write_png("skin.png", 64, [200, 100, 50, 255]);
        }
        arm_request(id, key, &fbx, &png, &self.ex.dir.join("export"))
    }
}

const KEY: &str = "GlobalObjectId_V1-2-0123-4567-0";

#[test]
fn headless_a_request_opens_the_fbx_with_its_pose_sets_by_material_and_originals() {
    let mut h = Headless::new("open");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Opened, "{}", h.state.message);
    assert_eq!(reply.request, "r1");
    assert!(reply.problems.is_empty(), "{:?}", reply.problems);
    assert!(
        h.ex.folder().claimed_files().unwrap().is_empty(),
        "当てた頼みは消す"
    );
    // モデル: 相手の鍵とマテリアル（Unity のマテリアルの鍵ごと）。3D ビューはまとめた Rig のポーズのセッション
    let model = h.state.model.clone().unwrap();
    assert_eq!(model.link_key(), Some(KEY));
    assert_eq!(model.name, "Arm");
    assert_eq!(model.materials.len(), 2);
    let rig = h.session_rig();
    assert_eq!(rig.meshes().len(), 2, "腕と帽子");
    assert_eq!(
        h.state.view3d.full_model().unwrap().triangle_count(),
        rig.triangle_count()
    );
    // セット: マテリアルの鍵ごとに 1 つ（何も触っていない最初のセットは Skin に付く）
    assert_eq!(h.state.sets.len(), 2);
    let names: Vec<&str> = h.state.sets.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"Skin") && names.contains(&"Cloth"),
        "{names:?}"
    );
    for (i, set) in h.state.sets.iter().enumerate() {
        let m = set.bound.expect("付いている") as usize;
        let expect = material_key(m as u32 + 1);
        let guid = &expect["guid:".len()..expect.len() - "/fileid:2100000".len()];
        assert!(
            matches!(&set.material, yolu_io::MaterialRef::Material { asset: Some(a), .. } if a.guid == guid && a.file_id == 2_100_000),
            "{:?}",
            set.material
        );
        // 元の絵: 一番下のレイヤー「元の絵」（Skin は PNG の色、Cloth は絵が無いので白）
        let doc = h.state.set_doc(i);
        let bottom = &doc.layers()[0];
        assert_eq!(bottom.name(), "元の絵");
        let mut px = [0u8; 4];
        doc.composite_into(
            yolu_core::Channel::Color,
            yolu_core::Rect::new(0, 0, 1, 1),
            &mut px,
            yolu_core::RowOrder::BottomUp,
        )
        .unwrap();
        if set.name == "Skin" {
            assert_eq!(px, [200, 100, 50, 255]);
            assert_eq!(
                doc.width(),
                256,
                "最初のセットは元の絵の大きさで作り直す（256 に丸める）"
            );
            // lilToon の値は受けた見た目
            let r = doc.received_look().expect("受けた見た目");
            assert_eq!(r.look.kind, LookKind::LilToon);
            assert_eq!(r.look.float("_Cutoff", 0.5), 0.25);
            assert_eq!(r.look.render_queue, Some(2450));
        } else {
            assert_eq!(px, [255, 255, 255, 255]);
            assert!(
                doc.received_look().is_none(),
                "名前が似ているだけのシェーダーは lilToon にしない"
            );
        }
    }
    // ポーズ: Lower の骨の値と、BlendShape の重み
    let pose = h.pose();
    let lower = h.bone("Lower");
    assert!(
        pose.locals[lower]
            .rotation
            .dot(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4))
            .abs()
            > 0.9999
    );
    let arm = rig
        .meshes()
        .iter()
        .position(|m| m.mesh.name == "ArmMesh")
        .unwrap();
    let thick = rig.meshes()[arm]
        .blend_shapes
        .iter()
        .position(|s| s.name == "Thick")
        .unwrap();
    assert_eq!(pose.blend_weights[arm][thick], 50.0);
    assert!(
        !h.state.view3d.pose.session.as_ref().unwrap().can_undo(),
        "初めのポーズは取り消しの段にしない"
    );
    // 入口の様子: 相手の名前と、Unity が送れなかった物
    assert_eq!(h.state.link.target.as_deref(), Some("Arm"));
    assert!(h.state.link.problems.iter().any(|p| p.path == "Accessory"));
    assert!(!h.state.modified, "開いただけでは変更なし");
}

#[test]
fn headless_a_resend_updates_the_pose_without_reading_the_fbx_again() {
    let mut h = Headless::new("resend");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    let rig = h.session_rig();
    let sets = h.state.sets.len();
    // ポーズと BlendShape だけを変えた送り直し: 同じ Rig のまま、ポーズが変わる（手で動かした分も上書き）
    let mut again = request.clone();
    again["id"] = "r2".into();
    again["bones"][0]["local"]["r"] = json!([0.0, 0.0, 0.0, 1.0]);
    again["renderers"][0]["blend_shapes"]["Thick"] = 10.0.into();
    h.ex.put(&again);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Opened);
    assert!(
        Arc::ptr_eq(&rig, &h.session_rig()),
        "FBX も Rig も組み直さない"
    );
    let pose = h.pose();
    assert_eq!(pose.locals[h.bone("Lower")].rotation, Quat::IDENTITY);
    assert!(pose.blend_weights.iter().flatten().any(|w| *w == 10.0));
    assert_eq!(h.state.sets.len(), sets, "セットは増えない");
    assert!(h.state.modified, "送り直したポーズは保存する変更");
    // FBX のファイルを壊しても、道と guid が同じなら読み直さない（表示の入切が変わって Rig は組み直す）
    std::fs::write(h.ex.dir.join("arm.fbx"), b"broken").unwrap();
    let mut hide = again.clone();
    hide["id"] = "r3".into();
    hide["renderers"][1]["enabled"] = false.into();
    h.ex.put(&hide);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Opened, "{}", h.state.message);
    assert!(reply.problems.is_empty(), "{:?}", reply.problems);
    let shown = h.session_rig();
    assert!(!Arc::ptr_eq(&rig, &shown));
    assert_eq!(shown.meshes().len(), 1, "帽子を入れない");
    // guid が変われば読み直す（壊れた FBX は読めない）
    let mut other = hide.clone();
    other["id"] = "r4".into();
    other["models"][0]["guid"] = "ffffffffffffffffffffffffffffffff".into();
    h.ex.put(&other);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Refused);
    assert_eq!(
        reply.problems[0].known_reason(),
        Some(Reason::FbxUnreadable)
    );
}

#[test]
fn headless_another_target_with_unsaved_changes_asks_before_opening() {
    let mut h = Headless::new("other");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    h.state.modified = true;
    let mut other = request.clone();
    other["id"] = "r2".into();
    other["target"]["key"] = "GlobalObjectId_V1-2-9999-1-0".into();
    other["target"]["name"] = "Other".into();
    h.ex.put(&other);
    h.until("聞く", |h| h.link.wants_discard());
    // 捨てない: 開かずに断る
    h.link.answer_discard(false);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Refused);
    assert_eq!(reply.problems[0].known_reason(), Some(Reason::Declined));
    assert_eq!(h.state.model.as_ref().unwrap().link_key(), Some(KEY));
    // 捨てる: 新しいプロジェクトに開く
    let mut third = other.clone();
    third["id"] = "r3".into();
    h.ex.put(&third);
    h.until("聞く", |h| h.link.wants_discard());
    h.link.answer_discard(true);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    assert_eq!(
        h.state.model.as_ref().unwrap().link_key(),
        Some("GlobalObjectId_V1-2-9999-1-0")
    );
    assert!(!h.state.modified);
    assert_eq!(h.state.link.target.as_deref(), Some("Other"));
}

#[test]
fn headless_exporting_the_target_writes_an_exported_reply() {
    let mut h = Headless::new("export");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    let dir = h.state.link_export_dir().expect("Unity が知らせた置き場");
    assert_eq!(slash(&dir), slash(&h.ex.dir.join("export")));
    std::fs::create_dir_all(&dir).unwrap();
    h.state
        .apply(Action::Export(yolu_app::export::ExportAction::TemplateTo {
            id: "liltoon".into(),
            dir: dir.clone(),
        }));
    h.state.wait_export();
    h.frame();
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Exported, "{}", h.state.message);
    assert_eq!(reply.request, "r1");
    // 使っているチャンネルは Color だけ: セットごとの Main の PNG が _MainTex に当たる
    assert_eq!(reply.files.len(), 2, "{:?}", reply.files);
    for f in &reply.files {
        assert_eq!(f.property, "_MainTex");
        assert!(f.srgb && !f.normal_map);
        assert!(std::path::Path::new(&f.path).is_file(), "{}", f.path);
        assert!([material_key(1), material_key(2)].contains(&f.material));
    }
    // 利用者が置き場を選び直したら、そちらが既定
    let chosen = h.ex.dir.join("elsewhere");
    h.state.note_export_dir(&chosen);
    assert_eq!(h.state.link_export_dir(), Some(chosen));
}

#[test]
fn headless_scene_materials_and_submeshes_without_a_material_get_their_own_sets() {
    let mut h = Headless::new("keys");
    let mut request = h.arm("r1", KEY);
    // Skin はシーンの中のマテリアル（object:）、帽子はマテリアルの無いサブメッシュ（none）
    let scene = "object:GlobalObjectId_V1-2-0123-4567-0-77";
    request["materials"][0]["key"] = scene.into();
    request["materials"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "key": "none" }));
    request["renderers"][1]["materials"] = json!([2]);
    h.ex.put(&request);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Opened, "{}", h.state.message);
    assert_eq!(h.state.sets.len(), 3, "マテリアルの鍵ごと");
    let refs: Vec<&yolu_io::MaterialRef> = h.state.sets.iter().map(|s| &s.material).collect();
    assert!(refs.iter().any(
        |m| matches!(m, yolu_io::MaterialRef::Material { name, asset: None } if name == "Skin")
    ));
    assert!(
        refs.contains(&&yolu_io::MaterialRef::Unassigned),
        "none はマテリアルなしの組: {refs:?}"
    );
    // 書き出し: マテリアルの無い組の絵は返さない
    let dir = h.state.link_export_dir().unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    h.state
        .apply(Action::Export(yolu_app::export::ExportAction::TemplateTo {
            id: "liltoon".into(),
            dir,
        }));
    h.state.wait_export();
    h.frame();
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Exported, "{}", h.state.message);
    let keys: Vec<&str> = reply.files.iter().map(|f| f.material.as_str()).collect();
    assert!(keys.contains(&scene), "{keys:?}");
    assert!(!keys.contains(&"none"), "{keys:?}");
}

#[test]
fn headless_broken_large_and_unknown_requests_are_refused_with_the_reason() {
    let mut h = Headless::new("refuse");
    h.ex.put_bytes("broken.json", b"{ not json");
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Refused);
    assert_eq!(
        reply.request, "broken",
        "読めない頼みはファイルの名前で返す"
    );
    assert_eq!(
        reply.problems[0].known_reason(),
        Some(Reason::FormatUnknown)
    );
    let mut newer = h.arm("r2", KEY);
    newer["format"] = 2.into();
    h.ex.put(&newer);
    let reply = h.reply();
    assert_eq!(
        reply.problems[0].known_reason(),
        Some(Reason::FormatUnknown)
    );
    let mut large = h.arm("r3", KEY);
    let b = large["bones"][0].clone();
    large["bones"] = serde_json::Value::Array(vec![b; yolu_protocol::files::MAX_BONES + 1]);
    h.ex.put(&large);
    let reply = h.reply();
    assert_eq!(reply.problems[0].known_reason(), Some(Reason::TooLarge));
    // 軸を焼いた取り込みは合わせられない（使えるレンダラーが無いので断る）
    let mut baked = h.arm("r4", KEY);
    baked["models"][0]["import"]["bake_axis_conversion"] = true.into();
    h.ex.put(&baked);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Refused);
    assert!(reply
        .problems
        .iter()
        .all(|p| p.known_reason() == Some(Reason::UnsupportedImport)));
    assert!(h.state.model.is_none(), "何も開かない");
    assert!(h.ex.folder().claimed_files().unwrap().is_empty());
}

#[test]
fn headless_a_request_that_comes_while_drawing_is_applied_after_the_stroke() {
    let mut h = Headless::new("stroke");
    let id = h.state.selected_layer.unwrap();
    let stroke = h.state.begin_paint_stroke(id, false).unwrap();
    assert!(h.state.is_stroking());
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    h.settle();
    assert!(
        h.ex.take_replies().is_empty(),
        "描いている間は当てない（断らない）"
    );
    assert!(h.state.model.is_none());
    assert_eq!(
        h.ex.folder().claimed_files().unwrap().len(),
        1,
        "待つ間は claimed に置いたまま"
    );
    h.state.doc.cancel_stroke(stroke);
    h.state.canvas.stroke = None;
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    assert!(h.state.model.is_some());
}

#[test]
fn headless_saving_and_reopening_gives_the_same_model_and_pose_without_unity() {
    let mut h = Headless::new("ylp");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    // 手で骨を動かしてから保存する（今のポーズが残る）
    let mut pose = h.pose();
    let upper = h.bone("Upper");
    pose.locals[upper].translation = Vec3::new(0.0, 1.5, 0.0);
    yolu_app::view3d::pose::set_pose(&mut h.state.view3d, pose.clone()).unwrap();
    let path = h.ex.dir.join("arm.ylp");
    h.state.apply(Action::SaveProjectAs(path.clone()));
    assert!(
        h.state.message.starts_with("保存しました"),
        "{}",
        h.state.message
    );
    let project = yolu_io::Project::open(&path, &yolu_io::Limits::unbounded()).unwrap();
    let stored = project.livelink().unwrap().expect("livelink.json");
    let text = String::from_utf8(stored).unwrap();
    assert!(
        text.contains(KEY) && !text.contains("_Cutoff"),
        "値は look.json に"
    );
    assert!(
        text.contains("_lilToonVersion"),
        "lilToon の印の値は残す（開き直したときスロットの絵を読むか決める）"
    );
    let triangles = h.state.view3d.full_model().unwrap().triangle_count();
    // Unity なし（受け付けていない）で開き直す
    let mut open = AppState::new(64, 64);
    let mut link = LiveLink::new();
    link.set_folder(h.ex.dir.join("unused")).unwrap();
    open.prefs.settings.livelink_on_startup = false;
    open.apply(Action::OpenProject(path.clone()));
    assert!(open.project.is_some(), "{}", open.message);
    let deadline = Instant::now() + WATCHDOG;
    while open.view3d.pose.session.is_none() || link.is_working() {
        link.poll(&mut open);
        assert!(Instant::now() < deadline, "{}", open.message);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(open.model.as_ref().unwrap().link_key(), Some(KEY));
    assert_eq!(
        open.view3d.full_model().unwrap().triangle_count(),
        triangles
    );
    let again = open.view3d.pose.session.as_ref().unwrap().pose().clone();
    for (a, b) in again.locals.iter().zip(&pose.locals) {
        assert!((a.translation - b.translation).length() < 1e-5);
        assert!(a.rotation.dot(b.rotation).abs() > 0.99999);
    }
    assert_eq!(again.blend_weights, pose.blend_weights);
    assert_eq!(open.sets.len(), 2, "セットは増やさない");
    assert!(open.sets.iter().all(|s| s.bound.is_some()));
    assert!(!open.modified);
    assert!(
        !h.ex.dir.join("unused").exists(),
        "受け付けていなければフォルダも作らない"
    );
}

#[test]
fn headless_two_fbx_files_make_one_model_and_a_collapsed_root_is_followed() {
    let mut h = Headless::new("two");
    let request = h.arm("r1", KEY);
    // 2 つ目の FBX: Armature だけが根の子（Unity は既定でこれをプレハブの根に畳む）。帽子の道は Armature を除いた道になる
    let mut single = fbx_ascii::arm_scene();
    single.nodes[3].parent = Some(0);
    let second = h.ex.write_scene("single.fbx", &single);
    let mut req = request.clone();
    req["models"].as_array_mut().unwrap().push(
        json!({ "id": 7, "fbx": slash(&second), "guid": "abababababababababababababababab",
                      "import": { "global_scale": 2.0 } }),
    );
    req["renderers"].as_array_mut().unwrap().push(json!({
        "path": "Second/Hat", "model": 7, "node": "Upper/Lower/Hat", "materials": [1]
    }));
    req["renderers"].as_array_mut().unwrap().push(json!({
        "path": "Second/ArmMesh", "model": 7, "node": "ArmMesh", "materials": [0, 1]
    }));
    req["bones"].as_array_mut().unwrap().push(json!({
        "model": 7, "node": "Upper", "local": { "t": [0.0, 3.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0], "s": [1.0, 1.0, 1.0] }
    }));
    h.ex.put(&req);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Opened, "{}", h.state.message);
    assert!(reply.problems.is_empty(), "{:?}", reply.problems);
    let rig = h.session_rig();
    assert_eq!(rig.meshes().len(), 4);
    let roots: Vec<&str> = rig.roots().map(|r| rig.bones()[r].name.as_str()).collect();
    assert_eq!(
        roots,
        ["0:arm", "7:single"],
        "根の名前は FBX の番号で分ける"
    );
    // 2 つ目の Upper は、FBX の倍率 2 の Unity の値（y = 3）
    let pose = h.pose();
    let uppers: Vec<usize> = rig
        .bones()
        .iter()
        .enumerate()
        .filter(|(_, b)| b.name == "Upper")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(uppers.len(), 2);
    assert_eq!(pose.locals[uppers[1]].translation, Vec3::new(0.0, 3.0, 0.0));
    assert_eq!(
        rig.bones()[uppers[1]].rest.translation,
        Vec3::new(0.0, 2.0, 0.0),
        "休みの値も倍率 2"
    );
    // 同じマテリアル（Unity のマテリアルの鍵）は同じセット
    assert_eq!(h.state.sets.len(), 2);
    // preserve_hierarchy なら畳まない: Armature を含む道でないと見つからない
    let mut kept = req.clone();
    kept["id"] = "r2".into();
    kept["models"][1]["import"]["preserve_hierarchy"] = true.into();
    h.ex.put(&kept);
    let reply = h.reply();
    assert!(reply
        .problems
        .iter()
        .any(|p| p.path == "Second/Hat" && p.known_reason() == Some(Reason::BoneNotFound)));
}

#[test]
fn headless_presence_is_written_while_accepting_and_removed_when_turned_off() {
    let mut h = Headless::new("presence");
    let folder = h.ex.folder();
    let p = folder.read_presence().unwrap().expect("起きている印");
    assert_eq!(p.app, "YoluPainter");
    assert_eq!(p.pid, std::process::id());
    h.state.apply(Action::ToggleLiveLink);
    h.link
        .request(h.state.link_request.take().unwrap(), &mut h.state);
    h.frame();
    assert_eq!(h.state.link.status, LinkStatus::Off);
    assert!(!h.state.prefs.settings.livelink_on_startup);
    assert_eq!(folder.read_presence().unwrap(), None);
    // 受けていない間に置いた頼みは拾わない
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    h.settle();
    assert_eq!(folder.waiting().unwrap().len(), 1);
}

/// 裏の仕事が走っている間に利用者が新規・.ylp を開くと、結果は新しい文書に入れず、`declined` で断る（前は Rig の結果が開いた文書へ
/// 入り、絵だけの送り直しの結果はセッションが無くて落ちた）。
#[test]
fn headless_a_job_that_is_running_when_the_project_is_replaced_is_declined_and_enters_nothing() {
    let mut h = Headless::new("replaced");
    let other = h.ex.dir.join("other.ylp");
    let mut theirs = AppState::new(64, 64);
    theirs.apply(Action::SaveProjectAs(other.clone()));
    assert!(
        theirs.message.starts_with("保存しました"),
        "{}",
        theirs.message
    );
    let request = h.arm("r1", KEY);
    // Rig を組み直す仕事（初めて開く）: 新規にする
    h.start_job(&request);
    h.state.apply(Action::NewProject);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Refused);
    assert_eq!(reply.problems[0].known_reason(), Some(Reason::Declined));
    h.settle();
    assert!(
        h.state.model.is_none(),
        "新しい文書に Unity のモデルを入れない"
    );
    assert!(h.state.view3d.pose.session.is_none());
    assert!(h.state.link_target.is_none() && h.state.link.target.is_none());
    assert_eq!(h.state.sets.len(), 1);
    assert!(h.ex.folder().claimed_files().unwrap().is_empty());
    assert!(!h.link.is_working());
    // 同じ: .ylp を開く
    let mut again = request.clone();
    again["id"] = "r2".into();
    h.start_job(&again);
    h.state.apply(Action::OpenProject(other.clone()));
    assert!(h.state.project.is_some(), "{}", h.state.message);
    let reply = h.reply();
    assert_eq!(reply.problems[0].known_reason(), Some(Reason::Declined));
    h.settle();
    assert!(h.state.model.is_none() && h.state.view3d.pose.session.is_none());
    assert!(h.state.project.is_some(), "開いた .ylp のまま");
    assert_eq!(h.state.sets.len(), 1);
    // 絵だけを読む仕事（送り直し。Rig はそのまま）: 開いたあとで、同じ相手の送り直しを走らせている間に新規にする
    let mut third = request.clone();
    third["id"] = "r3".into();
    h.ex.put(&third);
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    let mut resend = request.clone();
    resend["id"] = "r4".into();
    resend["bones"][0]["local"]["r"] = json!([0.0, 0.0, 0.0, 1.0]);
    h.start_job(&resend);
    h.state.apply(Action::NewProject);
    let reply = h.reply();
    assert_eq!(reply.kind, ReplyKind::Refused);
    assert_eq!(reply.problems[0].known_reason(), Some(Reason::Declined));
    h.settle();
    assert!(h.state.model.is_none() && h.state.view3d.pose.session.is_none());
    assert!(h.state.link_target.is_none());
}

/// 受け付けを切ると、まだ当てていない頼み（待っている物・走っている仕事）は、取り消して `declined` を返す。.ylp の開き直しは Unity を使わないので影響しない。
#[test]
fn headless_turning_acceptance_off_declines_the_requests_that_are_not_applied_yet() {
    let mut h = Headless::new("off");
    let request = h.arm("r1", KEY);
    h.start_job(&request);
    // 描いている間は、走っている仕事も結果を入れずに待つ（2 つ目の頼みも、拾われて待つ）
    let id = h.state.selected_layer.unwrap();
    let stroke = h.state.begin_paint_stroke(id, false).unwrap();
    let mut waiting = request.clone();
    waiting["id"] = "r2".into();
    h.ex.put(&waiting);
    h.until("2 つ目の頼みを拾う", |h| {
        h.ex.folder().claimed_files().unwrap().len() == 2
    });
    h.state.apply(Action::ToggleLiveLink);
    h.link
        .request(h.state.link_request.take().unwrap(), &mut h.state);
    h.frame();
    assert_eq!(h.state.link.status, LinkStatus::Off);
    let mut replies = h.ex.take_replies();
    replies.sort_by(|a, b| a.request.cmp(&b.request));
    let kinds: Vec<(&str, ReplyKind)> = replies
        .iter()
        .map(|r| (r.request.as_str(), r.kind))
        .collect();
    assert_eq!(
        kinds,
        [("r1", ReplyKind::Refused), ("r2", ReplyKind::Refused)]
    );
    assert!(replies
        .iter()
        .all(|r| r.problems[0].known_reason() == Some(Reason::Declined)));
    h.state.doc.cancel_stroke(stroke);
    h.state.canvas.stroke = None;
    h.settle();
    assert!(h.state.model.is_none(), "切ったあとに当てない");
    assert!(!h.link.is_working());
    assert!(h.ex.folder().claimed_files().unwrap().is_empty());
}

/// 保存の途中に来た頼みは、断らずに待ち、保存が終わってから当てる。
#[test]
fn headless_a_request_that_comes_while_saving_is_applied_after_the_save() {
    let mut h = Headless::new("saving");
    h.state.save.background = true;
    let hold = h.state.save.hold_next();
    h.state
        .apply(Action::SaveProjectAs(h.ex.dir.join("while.ylp")));
    assert!(h.state.is_saving());
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    h.settle();
    assert!(
        h.ex.take_replies().is_empty(),
        "保存の間は当てない（断らない）"
    );
    assert!(h.state.model.is_none());
    assert_eq!(
        h.ex.folder().claimed_files().unwrap().len(),
        1,
        "待つ間は claimed に置いたまま"
    );
    hold.release();
    h.state.wait_save();
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    assert!(h.state.model.is_some());
}

/// 裏の仕事が走っている間に描き始めたら、仕事が終わっても、ストロークが終わるまで入れない。
#[test]
fn headless_a_finished_job_waits_for_the_stroke_that_began_while_it_ran() {
    let mut h = Headless::new("stroke-job");
    let request = h.arm("r1", KEY);
    h.start_job(&request);
    let id = h.state.selected_layer.unwrap();
    let stroke = h.state.begin_paint_stroke(id, false).unwrap();
    h.settle();
    assert!(h.ex.take_replies().is_empty(), "描いている間は入れない");
    assert!(h.state.model.is_none());
    assert!(h.link.is_working(), "仕事は結果を持ったまま待つ");
    h.state.doc.cancel_stroke(stroke);
    h.state.canvas.stroke = None;
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    assert!(h.state.model.is_some());
}

/// 送り直しのポーズは 1 つの取り消しの段で、手で動かしたポーズへ戻せる（Rig を組み直す送り直しでも）。
#[test]
fn headless_a_resend_is_one_undo_step_that_brings_back_the_hand_made_pose() {
    let mut h = Headless::new("undo");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    let mut moved = h.pose();
    let upper = h.bone("Upper");
    moved.locals[upper].translation = Vec3::new(0.0, 1.5, 0.0);
    pose::set_pose(&mut h.state.view3d, moved.clone()).unwrap();
    let steps = h.state.view3d.pose.session.as_ref().unwrap().undo_len();
    // 絵だけを読む送り直し: 手で動かした分は上書きされるが、1 つの段で戻る
    let mut again = request.clone();
    again["id"] = "r2".into();
    again["bones"][0]["local"]["r"] = json!([0.0, 0.0, 0.0, 1.0]);
    h.ex.put(&again);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    assert_ne!(h.pose(), moved);
    assert_eq!(
        h.state.view3d.pose.session.as_ref().unwrap().undo_len(),
        steps + 1
    );
    assert!(pose::undo(&mut h.state.view3d).unwrap());
    assert_eq!(h.pose(), moved);
    // Rig を組み直す送り直し（表示の入切）: 新しいセッションでも、前のポーズへ 1 つの段で戻る
    let mut hide = again.clone();
    hide["id"] = "r3".into();
    hide["renderers"][1]["enabled"] = false.into();
    h.ex.put(&hide);
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    assert_eq!(h.session_rig().meshes().len(), 1, "帽子を入れない");
    assert!(
        h.state.message.contains("更新しました"),
        "初めて開いたのではなく送り直し: {}",
        h.state.message
    );
    assert_eq!(h.state.view3d.pose.session.as_ref().unwrap().undo_len(), 1);
    assert!(pose::undo(&mut h.state.view3d).unwrap());
    assert_eq!(h.pose().locals, moved.locals, "手で動かした骨の値");
}

/// 保存した文書へ、表示の入切だけが変わる送り直しが来たら「変更あり」になり、保存すると `livelink.json` に入る。
#[test]
fn headless_a_resend_that_rebuilds_the_rig_marks_the_saved_project_modified() {
    let mut h = Headless::new("rebuilt-modified");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    let path = h.ex.dir.join("hide.ylp");
    h.state.apply(Action::SaveProjectAs(path.clone()));
    assert!(!h.state.modified, "{}", h.state.message);
    let mut hide = request.clone();
    hide["id"] = "r2".into();
    hide["renderers"][1]["enabled"] = false.into();
    h.ex.put(&hide);
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    assert!(h.state.modified, "入切は保存する変更");
    h.state.apply(Action::SaveProject);
    let project = yolu_io::Project::open(&path, &yolu_io::Limits::unbounded()).unwrap();
    let text = String::from_utf8(project.livelink().unwrap().unwrap()).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["renderers"][1]["enabled"], false);
}

/// .ylp から開き直すとき、元の絵のファイルが動いていても、使わない絵を読んで「合わない物」にしない。
#[test]
fn headless_reopening_does_not_read_the_original_pictures() {
    let mut h = Headless::new("reopen-originals");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    let path = h.ex.dir.join("moved.ylp");
    h.state.apply(Action::SaveProjectAs(path.clone()));
    assert!(
        h.state.message.starts_with("保存しました"),
        "{}",
        h.state.message
    );
    std::fs::remove_file(h.ex.dir.join("skin.png")).unwrap();
    let mut open = AppState::new(64, 64);
    let mut link = LiveLink::new();
    link.set_folder(h.ex.dir.join("unused")).unwrap();
    open.prefs.settings.livelink_on_startup = false;
    open.apply(Action::OpenProject(path));
    let deadline = Instant::now() + WATCHDOG;
    while open.view3d.pose.session.is_none() || link.is_working() {
        link.poll(&mut open);
        assert!(Instant::now() < deadline, "{}", open.message);
        std::thread::sleep(Duration::from_millis(5));
    }
    link.poll(&mut open);
    let view = link.view(&open);
    assert!(
        !view
            .problems
            .iter()
            .any(|p| p.known_reason() == Some(Reason::TextureUnreadable)),
        "{:?}",
        view.problems
    );
    assert!(!open.message.contains("元の絵"), "{}", open.message);
    assert_eq!(open.sets.len(), 2);
}

/// 新しい文書（何も触っていない）の窓から Live Link のモデルを開くと、前のモデルのファイルの参照は残らない。
#[test]
fn headless_opening_into_a_pristine_document_drops_the_previous_model_file() {
    let mut h = Headless::new("model-file");
    h.state.np.model_file = Some(h.ex.dir.join("previous.fbx"));
    assert!(h.state.is_pristine());
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    assert_eq!(h.state.np.model_file, None, "前の FBX を指さない");
    let path = h.ex.dir.join("mf.ylp");
    h.state.apply(Action::SaveProjectAs(path.clone()));
    let project = yolu_io::Project::open(&path, &yolu_io::Limits::unbounded()).unwrap();
    assert!(project.pose().unwrap().is_none(), "pose.json は書かない");
    assert!(project.livelink().unwrap().is_some());
}

/// 道で指せない骨（畳んだ FBX の根の骨）を動かして保存すると、livelink.json には書けないので、保存の知らせに出す。
#[test]
fn headless_saving_names_the_bones_that_cannot_be_kept() {
    let mut h = Headless::new("unsaved-bones");
    let mut request = h.arm("r1", KEY);
    // Armature だけが根の子の FBX（Unity が Armature をプレハブの根に畳む）。根の骨は Unity の根の外
    let mut single = fbx_ascii::arm_scene();
    single.nodes[3].parent = Some(0);
    let fbx = h.ex.write_scene("single.fbx", &single);
    request["models"][0]["fbx"] = slash(&fbx).into();
    request["renderers"][1]["node"] = "Upper/Lower/Hat".into();
    request["bones"][0]["node"] = "Upper/Lower".into();
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    assert!(
        h.state
            .link
            .problems
            .iter()
            .all(|p| p.path != "Upper/Lower"),
        "{:?}",
        h.state.link.problems
    );
    let root = h.bone("single");
    let mut moved = h.pose();
    moved.locals[root].translation = Vec3::new(0.0, 1.0, 0.0);
    pose::set_pose(&mut h.state.view3d, moved).unwrap();
    h.state
        .apply(Action::SaveProjectAs(h.ex.dir.join("unsaved.ylp")));
    assert!(
        h.state.message.starts_with("保存しました"),
        "{}",
        h.state.message
    );
    assert!(
        h.state.message.contains("保存できない項目: single"),
        "{}",
        h.state.message
    );
}

/// スロットの絵は、送り直しで同じファイルなら読み直さない。ファイルが変われば読み直す。
#[test]
fn headless_a_resend_reuses_the_slot_pictures_of_unchanged_files() {
    let mut h = Headless::new("slot-cache");
    let mut request = h.arm("r1", KEY);
    let bump = h.ex.write_png("bump.png", 8, [128, 128, 255, 255]);
    request["materials"][0]["textures"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "property": "_BumpMap", "path": slash(&bump),
                      "srgb": false, "normal_map": true }));
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened, "{}", h.state.message);
    let bump_of = |h: &Headless| {
        let skin = h
            .state
            .sets
            .iter()
            .position(|s| s.name == "Skin")
            .expect("Skin のセット");
        h.state
            .set_doc(skin)
            .received_look()
            .and_then(|r| r.images.get("_BumpMap").cloned())
            .expect("法線の絵")
    };
    let first = bump_of(&h);
    let mut again = request.clone();
    again["id"] = "r2".into();
    again["bones"][0]["local"]["r"] = json!([0.0, 0.0, 0.0, 1.0]);
    h.ex.put(&again);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    assert!(Arc::ptr_eq(&first, &bump_of(&h)), "読み直さない");
    // 中身と更新時刻が変われば読み直す
    let before = std::fs::metadata(&bump).unwrap().modified().unwrap();
    h.ex.write_png("bump.png", 8, [255, 0, 0, 255]);
    let file = std::fs::OpenOptions::new().write(true).open(&bump).unwrap();
    file.set_modified(before + Duration::from_secs(30)).unwrap();
    drop(file);
    let mut third = again.clone();
    third["id"] = "r3".into();
    third["bones"][0]["local"]["r"] = json!([0.0, 0.0, 0.38268343, 0.9238795]);
    h.ex.put(&third);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    let changed = bump_of(&h);
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.pixels[0], 255, "新しい中身");
}

/// 書き出しの窓を開くだけでは、Unity が知らせた置き場のフォルダを作らない（取り消しても Unity のプロジェクトにフォルダを残さない）。
#[test]
fn headless_the_export_dialog_start_is_the_nearest_existing_folder_and_creates_nothing() {
    let mut h = Headless::new("export-start");
    let request = h.arm("r1", KEY);
    h.ex.put(&request);
    assert_eq!(h.reply().kind, ReplyKind::Opened);
    let wanted = h.state.link_export_dir().expect("Unity が知らせた置き場");
    assert!(!wanted.exists());
    assert_eq!(
        h.state.link_export_start().as_deref(),
        Some(h.ex.dir.as_path()),
        "あるところまで遡る"
    );
    assert!(!wanted.exists(), "作らない");
    std::fs::create_dir_all(&wanted).unwrap();
    assert_eq!(h.state.link_export_start(), Some(wanted));
}
