use super::*;

/// 設計の例の頼み（固有名詞は汎用名）。
const EXAMPLE: &str = r#"{
  "format": 1,
  "kind": "open",
  "id": "8f0c2a4e-0000-4000-8000-000000000001",
  "bridge": { "version": "0.5.0", "unity": "2022.3.22f1" },
  "project": { "root": "C:/Work/MyProject", "name": "MyProject" },
  "target": { "key": "GlobalObjectId_V1-2-abc-123-0", "name": "Avatar", "export_dir": "C:/Work/MyProject/Assets/YoluPainter/Avatar" },
  "root": { "world": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1] },
  "models": [
    { "id": 0, "fbx": "C:/Work/MyProject/Assets/Avatar/Body.fbx", "guid": "0123456789abcdef0123456789abcdef",
      "import": { "global_scale": 1.0, "use_file_scale": true, "bake_axis_conversion": false, "import_blend_shapes": true } }
  ],
  "renderers": [
    { "path": "Body", "model": 0, "node": "Body", "enabled": true, "skinned": true,
      "blend_shapes": { "eye_close": 0.0, "smile": 35.0 }, "materials": [0, 1] }
  ],
  "bones": [
    { "model": 0, "node": "Armature/Hips", "local": { "t": [0, 1, 0], "r": [0, 0, 0, 1], "s": [1, 1, 1] } }
  ],
  "materials": [
    { "key": "guid:0123456789abcdef0123456789abcdef/fileid:2100000", "name": "Body",
      "shader": { "name": "lilToon", "guid": "fedcba9876543210fedcba9876543210", "package": "jp.lilxyzw.liltoon", "version": "2.3.4", "keywords": ["_A"], "render_queue": 2000 },
      "values": { "floats": {"_Cutoff": 0.5}, "colors": {"_Color": [1,1,1,1]}, "vectors": {}, "ints": {} },
      "textures": [
        { "property": "_MainTex", "path": "C:/Work/MyProject/Assets/Avatar/body.png", "guid": "00000000000000000000000000000001",
          "srgb": true, "normal_map": false, "scale": [1,1], "offset": [0,0] },
        { "property": "_MatCapTex", "path": null }
      ] },
    { "key": "guid:0123456789abcdef0123456789abcdef/fileid:2100002", "name": "Face" },
    { "key": "none" }
  ],
  "refused": [ { "path": "Accessory", "reason": "mesh_not_from_fbx" } ],
  "somethingNew": { "ignored": true }
}"#;

fn example() -> serde_json::Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn read(v: &serde_json::Value) -> Result<Request, Refusal> {
    read_request(&serde_json::to_vec(v).unwrap())
}

#[test]
fn the_example_request_reads_and_unknown_keys_are_skipped() {
    let r = read_request(EXAMPLE.as_bytes()).unwrap();
    assert_eq!(r.kind, KIND_OPEN);
    assert_eq!(r.target.name, "Avatar");
    assert_eq!(r.models[0].import, ImportSettings::default());
    assert!(
        !r.models[0].import.preserve_hierarchy,
        "無ければ Unity の既定"
    );
    assert_eq!(r.renderers[0].blend_shapes["smile"], 35.0);
    assert_eq!(r.renderers[0].materials, [0, 1]);
    assert_eq!(r.bones[0].local.t, [0.0, 1.0, 0.0]);
    let m = &r.materials[0];
    assert_eq!(m.shader.render_queue, Some(2000));
    assert_eq!(m.shader.package, "jp.lilxyzw.liltoon");
    assert_eq!(
        r.materials[1].shader.package, "",
        "無ければ空（Assets の中・組み込み）"
    );
    assert_eq!(r.materials[2].key, KEY_NONE);
    assert_eq!(m.values.floats["_Cutoff"], 0.5);
    assert_eq!(m.textures[1].path, None, "ファイルの無い絵");
    assert!(m.textures[1].srgb, "無ければ sRGB");
    assert_eq!(r.materials[1].values, Values::default());
    assert_eq!(r.refused[0].known_reason(), Some(Reason::MeshNotFromFbx));
    assert_eq!(r.model_index(0), Some(0));
}

#[test]
fn a_request_of_another_format_or_kind_is_refused_as_unknown() {
    let mut v = example();
    v["format"] = 2.into();
    assert_eq!(read(&v).unwrap_err().reason, Reason::FormatUnknown);
    let mut v = example();
    v.as_object_mut().unwrap().remove("format");
    assert_eq!(read(&v).unwrap_err().reason, Reason::FormatUnknown);
    let mut v = example();
    v["kind"] = "close".into();
    let e = read(&v).unwrap_err();
    assert_eq!(e.reason, Reason::FormatUnknown);
    assert!(e.detail.contains("close"), "{e}");
    // 壊れた JSON・NaN の字句（JSON の数ではない）
    assert_eq!(
        read_request(b"{\"format\":1,").unwrap_err().reason,
        Reason::FormatUnknown
    );
    let nan = EXAMPLE.replace("\"_Cutoff\": 0.5", "\"_Cutoff\": NaN");
    assert_eq!(
        read_request(nan.as_bytes()).unwrap_err().reason,
        Reason::FormatUnknown
    );
}

#[test]
fn numbers_that_are_not_finite_after_reading_are_refused() {
    // f32 に入らない大きさは無限になる
    for (path, value) in [
        ("/materials/0/values/floats/_Cutoff", "1e39"),
        ("/bones/0/local/t/0", "-1e40"),
        ("/renderers/0/blend_shapes/smile", "3.5e38"),
        ("/root/world/5", "1e300"),
    ] {
        let mut v = example();
        *v.pointer_mut(path).unwrap() = serde_json::from_str(value).unwrap();
        let e = read(&v).unwrap_err();
        assert_eq!(e.reason, Reason::FormatUnknown, "{path}: {e}");
    }
    let mut v = example();
    v["models"][0]["import"]["global_scale"] = 0.into();
    assert_eq!(read(&v).unwrap_err().reason, Reason::FormatUnknown);
}

type Change = Box<dyn Fn(&mut serde_json::Value)>;

#[test]
fn shapes_that_do_not_hold_together_are_refused() {
    let cases: Vec<(&str, Change)> = vec![
        (
            "レンダラーのモデル",
            Box::new(|v| v["renderers"][0]["model"] = 5.into()),
        ),
        (
            "骨のモデル",
            Box::new(|v| v["bones"][0]["model"] = 9.into()),
        ),
        (
            "マテリアルの番号",
            Box::new(|v| v["renderers"][0]["materials"][1] = 3.into()),
        ),
        (
            "id の重なり",
            Box::new(|v| {
                let m = v["models"][0].clone();
                v["models"].as_array_mut().unwrap().push(m);
            }),
        ),
        ("鍵なし", Box::new(|v| v["target"]["key"] = " ".into())),
        ("名前にできない id", Box::new(|v| v["id"] = "../x".into())),
        (
            "必須の欄が無い",
            Box::new(|v| {
                v.as_object_mut().unwrap().remove("renderers");
            }),
        ),
        (
            "root の数",
            Box::new(|v| v["root"]["world"] = serde_json::json!([1, 0])),
        ),
    ];
    for (what, change) in cases {
        let mut v = example();
        change(&mut v);
        assert_eq!(
            read(&v).unwrap_err().reason,
            Reason::FormatUnknown,
            "{what}"
        );
    }
}

#[test]
fn requests_over_the_limits_are_refused_as_too_large() {
    let big = vec![b' '; MAX_REQUEST_BYTES as usize + 1];
    assert_eq!(read_request(&big).unwrap_err().reason, Reason::TooLarge);
    let mut v = example();
    let r = v["renderers"][0].clone();
    v["renderers"] = serde_json::Value::Array(vec![r; MAX_RENDERERS + 1]);
    assert_eq!(read(&v).unwrap_err().reason, Reason::TooLarge);
    let mut v = example();
    let b = v["bones"][0].clone();
    v["bones"] = serde_json::Value::Array(vec![b; MAX_BONES + 1]);
    assert_eq!(read(&v).unwrap_err().reason, Reason::TooLarge);
    let mut v = example();
    let t = v["materials"][0]["textures"][0].clone();
    v["materials"][0]["textures"] = serde_json::Value::Array(vec![t; MAX_TEXTURES + 1]);
    assert_eq!(read(&v).unwrap_err().reason, Reason::TooLarge);
    let mut v = example();
    let m = v["materials"][1].clone();
    v["materials"] = serde_json::Value::Array(vec![m; MAX_MATERIALS + 1]);
    assert_eq!(read(&v).unwrap_err().reason, Reason::TooLarge);
    let mut v = example();
    v["target"]["name"] = "x".repeat(MAX_TEXT_BYTES + 1).into();
    assert_eq!(read(&v).unwrap_err().reason, Reason::TooLarge);
}

#[test]
fn reasons_round_trip_through_their_words() {
    for r in Reason::ALL {
        assert_eq!(Reason::parse(r.as_str()), Some(r));
    }
    assert_eq!(Reason::parse("something_new"), None);
    assert_eq!(Reason::Declined.as_str(), "declined");
}

#[test]
fn the_folder_follows_each_os_and_the_override() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| PathBuf::from(v))
        }
    };
    let tail = |p: PathBuf| p.to_string_lossy().replace('\\', "/");
    assert_eq!(
        tail(folder_for("linux", env(&[("HOME", "/home/u")])).unwrap()),
        "/home/u/.local/share/YoluPainter/LiveLink"
    );
    assert_eq!(
        tail(
            folder_for(
                "linux",
                env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/data")])
            )
            .unwrap()
        ),
        "/data/YoluPainter/LiveLink"
    );
    assert_eq!(
        tail(
            folder_for(
                "linux",
                env(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "rel")])
            )
            .unwrap()
        ),
        "/home/u/.local/share/YoluPainter/LiveLink",
        "絶対でない XDG_DATA_HOME は使わない"
    );
    assert_eq!(
        tail(folder_for("macos", env(&[("HOME", "/Users/u")])).unwrap()),
        "/Users/u/Library/Application Support/YoluPainter/LiveLink"
    );
    assert_eq!(folder_for("linux", env(&[])), None);
    assert_eq!(
        tail(folder_for("linux", env(&[("HOME", "/home/u"), (ENV_DIR, "/tmp/ll")])).unwrap()),
        "/tmp/ll"
    );
    // 絶対の道かどうかは、走っている OS ではなく引数の OS の決まりで見る（どの OS でも、ほかの OS の場合を確かめられる）
    assert_eq!(
        folder_for("linux", env(&[("HOME", "C:\\Users\\u")])),
        None,
        "linux はドライブ文字の道を絶対と見ない"
    );
    assert_eq!(folder_for("macos", env(&[("HOME", "Users/u")])), None);
    for (local, expected) in [
        (
            "C:\\Users\\u\\AppData\\Local",
            "C:/Users/u/AppData/Local/YoluPainter/LiveLink",
        ),
        (
            "d:/Users/u/AppData/Local",
            "d:/Users/u/AppData/Local/YoluPainter/LiveLink",
        ),
        (
            "\\\\?\\C:\\Users\\u\\AppData\\Local",
            "//?/C:/Users/u/AppData/Local/YoluPainter/LiveLink",
        ),
        (
            "\\\\server\\share\\Local",
            "//server/share/Local/YoluPainter/LiveLink",
        ),
    ] {
        let env = |key: &str| (key == "LOCALAPPDATA").then(|| PathBuf::from(local));
        assert_eq!(
            tail(folder_for("windows", env).unwrap()),
            expected,
            "{local}"
        );
    }
    for relative in [
        "AppData\\Local",
        "/home/u",
        "C:Users",
        "\\Users\\u",
        "\\",
        "\\\\",
        "",
    ] {
        let env = |key: &str| (key == "LOCALAPPDATA").then(|| PathBuf::from(relative));
        assert_eq!(
            folder_for("windows", env),
            None,
            "windows は絶対でない道を使わない: {relative:?}"
        );
    }
    for (dir, expected) in [
        ("D:\\ll", "D:/ll"),
        ("ll", "C:/Local/YoluPainter/LiveLink"), // 絶対でない差し替えは使わない
    ] {
        let env = |key: &str| match key {
            "LOCALAPPDATA" => Some(PathBuf::from("C:\\Local")),
            ENV_DIR => Some(PathBuf::from(dir)),
            _ => None,
        };
        assert_eq!(tail(folder_for("windows", env).unwrap()), expected, "{dir}");
    }
}

/// 走っている OS が Windows のとき、`windows` の決まりは本物の `Path::is_absolute` と同じ答え（本物の `default_folder` の振る舞いが
/// 試験用の決まりに替わっても変わらない）。
#[cfg(windows)]
#[test]
fn the_windows_rule_agrees_with_the_real_path_check() {
    for path in [
        "C:\\Users\\u",
        "c:/Users/u",
        "\\\\?\\C:\\Users\\u",
        "\\\\server\\share\\x",
        "//server/share/x",
        "C:Users",
        "\\Users\\u",
        "\\\\",
        "/home/u",
        "Users\\u",
        "",
    ] {
        let path = Path::new(path);
        assert_eq!(
            absolute_for("windows", path),
            path.is_absolute(),
            "{path:?}"
        );
    }
}

#[test]
fn utc_text_is_rfc3339_and_reads_back() {
    assert_eq!(utc_text(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    let t = UNIX_EPOCH + Duration::from_secs(1_791_374_400); // 2026-10-07T12:00:00Z
    assert_eq!(utc_text(t), "2026-10-07T12:00:00Z");
    assert_eq!(parse_utc("2026-10-07T12:00:00Z"), Some(t));
    assert_eq!(parse_utc("2026-10-07T12:00:00.250+00:00"), Some(t));
    assert_eq!(
        parse_utc("2024-02-29T23:59:59Z").map(utc_text).as_deref(),
        Some("2024-02-29T23:59:59Z")
    );
    for bad in [
        "",
        "2026-10-07 12:00:00Z",
        "2026-13-07T12:00:00Z",
        "2026-10-07T12:00:00+09:00",
        "x",
    ] {
        assert_eq!(parse_utc(bad), None, "{bad}");
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("LiveLink")
}

fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn the_folder_is_made_private_with_its_three_boxes() {
    let root = temp("open");
    let f = Folder::open(&root).unwrap();
    for dir in [f.root().to_path_buf(), f.inbox(), f.claimed(), f.outbox()] {
        assert!(dir.is_dir(), "{dir:?}");
        check_private_dir(&dir).unwrap();
    }
    // 2 度目も同じフォルダを使える
    Folder::open(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(f.inbox(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            Folder::open(&root).is_err(),
            "ほかの人も入れるフォルダは使わない"
        );
        std::fs::set_permissions(f.inbox(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    cleanup(&root);
}

#[test]
fn presence_is_written_through_a_tmp_name_and_removed() {
    let root = temp("presence");
    let f = Folder::open(&root).unwrap();
    assert_eq!(f.read_presence().unwrap(), None);
    let p = Presence::now("0.5.0");
    f.write_presence(&p).unwrap();
    assert_eq!(f.read_presence().unwrap(), Some(p.clone()));
    assert!(!root.join("presence.json.tmp").exists());
    let back = parse_utc(&p.updated).unwrap();
    assert!(SystemTime::now().duration_since(back).unwrap() < PRESENCE_FRESH);
    let text = std::fs::read_to_string(f.presence_path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    for key in ["format", "app", "version", "pid", "updated"] {
        assert!(v.get(key).is_some(), "{key}");
    }
    f.remove_presence().unwrap();
    f.remove_presence().unwrap();
    assert!(!f.presence_path().exists());
    cleanup(&root);
}

/// `claimed/` の拾っている印（`.lock`）の名前を、古い順に。
fn marks(f: &Folder) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(f.claimed())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(LOCK_SUFFIX))
        .collect();
    names.sort();
    names
}

/// ファイルの更新の時刻を 3 日前にする。
fn make_old(path: &Path) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(3 * 86_400))
        .unwrap();
}

#[test]
fn half_written_files_are_not_seen_and_claiming_moves_the_request() {
    let root = temp("claim");
    let f = Folder::open(&root).unwrap();
    std::fs::write(f.inbox().join("a.json.tmp"), b"{").unwrap();
    std::fs::write(f.inbox().join("note.txt"), b"x").unwrap();
    std::fs::create_dir(f.inbox().join("dir.json")).unwrap();
    assert!(
        f.waiting().unwrap().is_empty(),
        "書きかけ・ほかの名前・フォルダは見ない"
    );
    std::fs::write(f.inbox().join("b.json"), EXAMPLE).unwrap();
    let waiting = f.waiting().unwrap();
    assert_eq!(waiting.len(), 1);
    let claimed = f.claim(&waiting[0]).unwrap().unwrap();
    assert_eq!(claimed.stem, "b");
    assert!(!waiting[0].exists() && claimed.path.exists());
    assert_eq!(f.claimed_files().unwrap(), vec![claimed.path.clone()]);
    assert_eq!(marks(&f), ["b.json.lock"], "拾っている間は印を持つ");
    let request = claimed.read().unwrap();
    assert_eq!(claimed.reply_id(Some(&request)).unwrap(), request.id);
    assert_eq!(claimed.reply_id(None).unwrap(), "b");
    // 2 度目は拾えない（もう無い）
    assert!(f.claim(&waiting[0]).unwrap().is_none());
    assert_eq!(
        marks(&f),
        ["b.json.lock"],
        "拾えなかった受け手は印に触れない"
    );
    claimed.finish().unwrap();
    claimed.finish().unwrap();
    assert!(f.claimed_files().unwrap().is_empty());
    assert!(marks(&f).is_empty(), "終えたら印も消える");
    cleanup(&root);
}

#[test]
fn finishing_removes_the_request_and_then_the_mark_and_only_once() {
    let root = temp("finish");
    let f = Folder::open(&root).unwrap();
    let inbox = f.inbox().join("b.json");
    std::fs::write(&inbox, EXAMPLE).unwrap();
    let first = f.claim(&inbox).unwrap().unwrap();
    first.finish().unwrap();
    assert!(!first.path.exists() && marks(&f).is_empty());
    // 同じ名前の頼みをもう一度拾った後で、前の持ち主がまた `finish` や落とすことをしても、今の持ち主の頼みと印は消えない
    std::fs::write(&inbox, EXAMPLE).unwrap();
    let second = f.claim(&inbox).unwrap().unwrap();
    first.finish().unwrap();
    drop(first);
    assert!(second.path.exists());
    assert_eq!(marks(&f), ["b.json.lock"]);
    second.finish().unwrap();
    assert!(marks(&f).is_empty() && f.claimed_files().unwrap().is_empty());
    cleanup(&root);
}

#[test]
fn dropping_a_claim_releases_its_mark_and_leaves_the_request_for_the_sweep() {
    let root = temp("drop");
    let f = Folder::open(&root).unwrap();
    let inbox = f.inbox().join("b.json");
    std::fs::write(&inbox, EXAMPLE).unwrap();
    let claimed = f.claim(&inbox).unwrap().unwrap();
    let path = claimed.path.clone();
    assert_eq!(marks(&f).len(), 1);
    drop(claimed);
    assert!(marks(&f).is_empty(), "落とせば印は残らない");
    assert!(
        path.exists(),
        "頼みは claimed/ に残す（古くなれば片付ける）"
    );
    cleanup(&root);
}

#[test]
fn claiming_what_is_already_gone_leaves_no_mark() {
    let root = temp("gone");
    let f = Folder::open(&root).unwrap();
    assert!(f.claim(&f.inbox().join("gone.json")).unwrap().is_none());
    assert!(marks(&f).is_empty(), "頼みが無かったときは印を消す");
    // 印を作れない理由が「もう有る」でなければ、拾えなかったことにせず断る（頼みには触れない）
    let inbox = f.inbox().join("b.json");
    std::fs::write(&inbox, EXAMPLE).unwrap();
    std::fs::remove_dir(f.claimed()).unwrap();
    assert!(f.claim(&inbox).is_err());
    assert!(inbox.exists());
    cleanup(&root);
}

#[test]
fn a_leftover_mark_blocks_the_claim_until_it_is_old_enough_to_sweep() {
    let root = temp("mark");
    let f = Folder::open(&root).unwrap();
    let inbox = f.inbox().join("m.json");
    let mark = f.claimed().join("m.json.lock");
    std::fs::write(&inbox, EXAMPLE).unwrap();
    // 印だけが残った（印を作った直後に落ちた）形
    std::fs::write(&mark, b"").unwrap();
    assert!(f.claim(&inbox).unwrap().is_none());
    assert!(
        inbox.exists() && mark.exists(),
        "ほかの受け手の印と頼みには触れない"
    );
    assert_eq!(f.sweep_claimed(Duration::from_secs(86_400)).unwrap(), 0);
    assert!(f.claim(&inbox).unwrap().is_none(), "新しい印は消えない");
    make_old(&mark);
    assert_eq!(f.sweep_claimed(Duration::from_secs(86_400)).unwrap(), 1);
    assert!(!mark.exists() && inbox.exists());
    let claimed = f.claim(&inbox).unwrap().expect("古い印を片付ければ拾える");
    assert_eq!(claimed.read().unwrap().target.name, "Avatar");
    cleanup(&root);
}

#[test]
fn marks_are_not_listed_as_requests() {
    let root = temp("lock-names");
    let f = Folder::open(&root).unwrap();
    std::fs::write(f.claimed().join("x.json.lock"), b"").unwrap();
    std::fs::write(f.inbox().join("y.json.lock"), b"").unwrap();
    std::fs::write(f.outbox().join("z.json.lock"), b"").unwrap();
    assert!(f.claimed_files().unwrap().is_empty());
    assert!(f.waiting().unwrap().is_empty());
    assert!(f.replies().unwrap().is_empty());
    std::fs::write(f.inbox().join("y.json"), EXAMPLE).unwrap();
    assert_eq!(f.waiting().unwrap(), vec![f.inbox().join("y.json")]);
    cleanup(&root);
}

/// `receivers` 個の受け手（それぞれ別の `Folder`）が、同じ頼みを同時に拾いに行く。`hold` なら勝った受け手が印を持ったまま全員の結果を待ち、
/// そうでなければ拾ってすぐ落とす（遅れた受け手が、印の消えた後で「頼みはもう無い」に当たる道も通る）。毎回、勝ったのは 1 つだけで、
/// 全員が手放した後に印が残らないこと。
fn race(name: &str, receivers: usize, rounds: usize, hold: bool) {
    let root = temp(name);
    let f = Folder::open(&root).unwrap();
    let folders: Vec<Folder> = (0..receivers)
        .map(|_| Folder::open(&root).unwrap())
        .collect();
    for round in 0..rounds {
        let file = f.inbox().join(format!("r{round}.json"));
        std::fs::write(&file, EXAMPLE).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(receivers));
        let handles: Vec<_> = folders
            .iter()
            .map(|g| {
                let (g, file, barrier) = (g.clone(), file.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let claimed = g.claim(&file).unwrap();
                    (claimed.is_some(), hold.then_some(claimed))
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let won = results.iter().filter(|(won, _)| *won).count();
        assert_eq!(won, 1, "回 {round}");
        drop(results);
        assert!(marks(&f).is_empty(), "回 {round}: 印が残った");
        assert!(!file.exists(), "回 {round}: 頼みが inbox に残った");
    }
    assert_eq!(f.claimed_files().unwrap().len(), rounds);
    cleanup(&root);
}

#[test]
fn two_receivers_claiming_at_once_get_it_only_once() {
    race("race", 2, 300, true);
}

#[test]
fn receivers_that_let_go_at_once_still_leave_it_to_one() {
    race("race-drop", 4, 150, false);
}

#[test]
fn a_request_file_that_is_too_large_or_unreadable_is_refused() {
    let root = temp("bad");
    let f = Folder::open(&root).unwrap();
    let big = f.inbox().join("big.json");
    std::fs::write(&big, vec![b' '; MAX_REQUEST_BYTES as usize + 10]).unwrap();
    let c = f.claim(&big).unwrap().unwrap();
    assert_eq!(c.read().unwrap_err().reason, Reason::TooLarge);
    let broken = f.inbox().join("broken.json");
    std::fs::write(&broken, b"not json").unwrap();
    let c = f.claim(&broken).unwrap().unwrap();
    assert_eq!(c.read().unwrap_err().reason, Reason::FormatUnknown);
    #[cfg(unix)]
    {
        let target = f.root().join("elsewhere.json");
        std::fs::write(&target, EXAMPLE).unwrap();
        let link = f.claimed().join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let c = Claimed {
            path: link,
            stem: "link".into(),
            mark: Mutex::new(None),
        };
        assert!(c.read().is_err(), "シンボリックリンクは辿らない");
    }
    cleanup(&root);
}

#[test]
fn replies_are_named_by_the_request_and_their_number() {
    let root = temp("reply");
    let f = Folder::open(&root).unwrap();
    let mut reply = Reply::new("8f0c", ReplyKind::Exported, "0.5.0");
    reply.files.push(ExportedFile {
        material: "guid:0/fileid:1".into(),
        property: "_MainTex".into(),
        path: "C:/x/Body_Color.png".into(),
        srgb: true,
        normal_map: false,
    });
    reply
        .problems
        .push(Problem::new("Accessory", Reason::BoneNotFound));
    let path = f.write_reply(&reply, 0).unwrap();
    assert_eq!(path, f.outbox().join("8f0c-0.json"));
    let back: Reply = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(back, reply);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("\"kind\": \"exported\"") && text.contains("\"reason\": \"bone_not_found\""),
        "{text}"
    );
    f.write_reply(&reply, 1).unwrap();
    assert_eq!(f.replies().unwrap().len(), 2);
    assert!(std::fs::read_dir(f.outbox()).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
    reply.request = "../x".into();
    assert!(f.write_reply(&reply, 0).is_err());
    cleanup(&root);
}

#[test]
fn leftover_claims_are_swept_only_when_old() {
    let root = temp("sweep");
    let f = Folder::open(&root).unwrap();
    let old = f.claimed().join("old.json");
    let new = f.claimed().join("new.json");
    std::fs::write(&old, b"{}").unwrap();
    std::fs::write(&new, b"{}").unwrap();
    make_old(&old);
    assert_eq!(f.sweep_claimed(Duration::from_secs(86_400)).unwrap(), 1);
    assert!(!old.exists() && new.exists());
    // 印も同じ。古い印は頼みといっしょに消え、新しい印は残る
    let old_mark = f.claimed().join("old.json.lock");
    let new_mark = f.claimed().join("new.json.lock");
    std::fs::write(&old, b"{}").unwrap();
    make_old(&old);
    std::fs::write(&old_mark, b"").unwrap();
    std::fs::write(&new_mark, b"").unwrap();
    make_old(&old_mark);
    assert_eq!(f.sweep_claimed(Duration::from_secs(86_400)).unwrap(), 2);
    assert!(!old.exists() && !old_mark.exists());
    assert!(new.exists() && new_mark.exists());
    cleanup(&root);
}
