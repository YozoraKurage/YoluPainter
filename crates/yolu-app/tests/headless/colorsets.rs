use std::path::PathBuf;
use yolu_app::{
    colorsets::{
        self, format, ColorSets, Edit, Error, Palette, Swatch, MAX_COLORS, MAX_FILE_BYTES,
        MAX_NAME_BYTES, MAX_SETS,
    },
    lang::Lang,
    state::ColorState,
};

fn palette() -> Palette {
    Palette {
        name: "試験 Sample".into(),
        colors: vec![
            Swatch {
                name: "透明色".into(),
                rgba: [0.125, 0.25, 0.75, 0.0],
            },
            Swatch::new([1.0, 0.5, 0.0, 1.0]),
        ],
    }
}
fn directory(tag: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/colorsets-tests")
        .join(format!("{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn native_preserves_names_float_alpha_and_rejects_invalid_data() {
    let p = palette();
    let bytes = format::encode(&p).unwrap();
    assert_eq!(format::decode(&bytes), Ok(p.clone()));
    for n in 0..bytes.len() {
        assert!(format::decode(&bytes[..n]).is_err(), "{n}");
    }
    let mut bad = bytes.clone();
    bad.push(0);
    assert_eq!(format::decode(&bad), Err(Error::Trailing));
    let mut bad = bytes.clone();
    bad[5] = 2;
    assert_eq!(format::decode(&bad), Err(Error::Version));
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let mut p = p.clone();
        p.colors[0].rgba[0] = value;
        assert_eq!(format::encode(&p), Err(Error::Color));
    }
    let mut p = p;
    p.name = "x".repeat(MAX_NAME_BYTES + 1);
    assert_eq!(format::encode(&p), Err(Error::Name));
}
#[test]
fn gpl_old_new_round_trip_names_and_rejections() {
    let bytes = b"GIMP Palette\r\nName: Sample\r\nColumns: 7\r\n# note\r\n255 0 128\tRose name\r\n0 1 2\r\n";
    let p = format::read_gpl(bytes, "fallback").unwrap();
    assert_eq!(p.colors[0].name, "Rose name");
    assert_eq!(p.colors[0].rgba, [1.0, 0.0, 128.0 / 255.0, 1.0]);
    assert_eq!(
        format::read_gpl(&format::write_gpl(&p).unwrap(), "other"),
        Ok(p)
    );
    assert_eq!(
        format::read_gpl(b"GIMP Palette\n0 1 2\n", "old")
            .unwrap()
            .name,
        "old"
    );
    for bad in [
        b"GIMP Palette\nName: X\nColumns: 256\n".as_slice(),
        b"GIMP Palette\n-1 0 0\n",
        b"GIMP Palette\n256 0 0\n",
        b"GIMP Palette\n0 1\n",
        b"GIMP Palette\n0 0 0\xff\n",
        b"not gpl",
    ] {
        assert!(format::read_gpl(bad, "X").is_err());
    }
    assert_eq!(format::write_gpl(&palette()), Err(Error::Alpha));
}
fn aco(version: u16, space: u16, values: [u16; 4], name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for n in [version, 1, space].into_iter().chain(values) {
        out.extend(n.to_be_bytes());
    }
    if version == 2 {
        let units: Vec<_> = name.encode_utf16().chain([0]).collect();
        out.extend((units.len() as u32).to_be_bytes());
        for u in units {
            out.extend(u.to_be_bytes());
        }
    }
    out
}
#[test]
fn aco_versions_names_spaces_and_dual_section() {
    let v1 = aco(1, 0, [65535, 0, 32768, 0], "");
    let v2 = aco(2, 0, [65535, 0, 32768, 0], "色 🎨");
    let mut both = v1.clone();
    both.extend(&v2);
    let p = format::read_aco(&both, "ACO").unwrap();
    assert_eq!(p.colors[0].name, "色 🎨");
    assert_eq!(p.colors[0].rgba, [1.0, 0.0, 32768.0 / 65535.0, 1.0]);
    assert_eq!(format::read_aco(&v2, "ACO"), Ok(p));
    assert!(format::read_aco(&v1, "ACO").is_ok());
    assert_eq!(
        format::read_aco(&aco(1, 1, [0, 65535, 65535, 0], ""), "X")
            .unwrap()
            .colors[0]
            .rgba,
        [1.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        format::read_aco(&aco(1, 8, [5000, 0, 0, 0], ""), "X")
            .unwrap()
            .colors[0]
            .rgba,
        [0.5, 0.5, 0.5, 1.0]
    );
    for space in [2, 7, 3, 65535] {
        assert_eq!(
            format::read_aco(&aco(1, space, [0; 4], ""), "X"),
            Err(Error::ColorSpace(space))
        );
    }
}
#[test]
fn aco_rejects_truncation_unicode_mismatch_and_bounds() {
    let v2 = aco(2, 0, [0; 4], "x");
    for n in 0..v2.len() {
        assert!(format::read_aco(&v2[..n], "X").is_err(), "{n}");
    }
    let mut bad = v2.clone();
    bad.push(0);
    assert_eq!(format::read_aco(&bad, "X"), Err(Error::Trailing));
    let mut bad = v2.clone();
    *bad.last_mut().unwrap() = 1;
    assert_eq!(format::read_aco(&bad, "X"), Err(Error::Encoding));
    let mut bad = v2.clone();
    bad[18] = 0xd8;
    bad[19] = 0;
    assert_eq!(format::read_aco(&bad, "X"), Err(Error::Encoding));
    let mut bad = aco(1, 0, [0; 4], "");
    bad.extend(aco(2, 0, [1, 0, 0, 0], "x"));
    assert_eq!(format::read_aco(&bad, "X"), Err(Error::Mismatch));
    assert_eq!(
        format::read_aco(&aco(1, 8, [10001, 0, 0, 0], ""), "X"),
        Err(Error::Color)
    );
    assert_eq!(format::read_aco(&[0, 1, 255, 255], "X"), Err(Error::Limit));
}
#[test]
fn all_file_and_color_limits_are_enforced() {
    let bytes = vec![0; MAX_FILE_BYTES + 1];
    assert_eq!(format::decode(&bytes), Err(Error::Limit));
    assert_eq!(format::read_gpl(&bytes, "X"), Err(Error::Limit));
    assert_eq!(format::read_aco(&bytes, "X"), Err(Error::Limit));
    let p = Palette {
        name: "X".into(),
        colors: vec![Swatch::new([0.0; 4]); MAX_COLORS + 1],
    };
    assert_eq!(format::encode(&p), Err(Error::Limit));
    let text = format!("GIMP Palette\n{}", "0 0 0\n".repeat(MAX_COLORS + 1));
    assert_eq!(format::read_gpl(text.as_bytes(), "X"), Err(Error::Limit));
    let dir = directory("bounds");
    let path = dir.join("large.gpl");
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(colorsets::read_bounded(&path), Err(Error::Limit));
}
#[test]
fn edit_sets_save_reload_delete_and_failed_save_preserves_memory_and_file() {
    let dir = directory("store");
    let mut sets = ColorSets::default();
    let mut recent = vec![];
    assert!(sets.attach(dir.clone(), &mut recent).is_empty());
    sets.add_set(palette()).unwrap();
    sets.edit(Edit::Add([0.2, 0.3, 0.4, 1.0])).unwrap();
    sets.edit(Edit::Move(2, 0)).unwrap();
    sets.edit(Edit::Replace(1, [0.9, 0.8, 0.7, 0.6])).unwrap();
    sets.edit(Edit::Remove(2)).unwrap();
    sets.edit(Edit::Rename("名前".into())).unwrap();
    sets.duplicate(Lang::En).unwrap();
    assert_eq!(sets.entries().len(), 3);
    sets.remove_set().unwrap();
    sets.switch(1);
    let expected = sets.palette().clone();
    let colors = vec![[0.25, 0.5, 0.75, 0.0]; 64];
    sets.corners[0] = [0.1, 0.2, 0.3, 0.4];
    sets.persist_state(&colors).unwrap();
    let mut reload = ColorSets::default();
    assert!(reload.attach(dir.clone(), &mut recent).is_empty());
    assert_eq!(reload.entries().len(), 2);
    assert_eq!(reload.palette(), &expected);
    assert_eq!(recent, colors);
    assert_eq!(reload.corners, sets.corners);
    let id = reload.entries()[1].id;
    let path = dir.join(format!("{id:016x}.ycolors"));
    let previous = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert_eq!(reload.edit(Edit::Add([0.0; 4])), Err(Error::Io));
    assert_eq!(reload.palette(), &expected);
    assert!(path.is_dir());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, previous).unwrap();
    reload.remove_set().unwrap();
    assert_eq!(reload.remove_set(), Err(Error::LastSet));
}
#[test]
fn invalid_import_and_invalid_stored_files_do_not_get_overwritten() {
    let dir = directory("invalid");
    let path = dir.join("0000000000000001.ycolors");
    std::fs::write(&path, b"broken").unwrap();
    std::fs::write(dir.join("state.conf"), b"broken").unwrap();
    let mut sets = ColorSets::default();
    assert!(!sets.attach(dir.clone(), &mut vec![]).is_empty());
    sets.edit(Edit::Add([0.0; 4])).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    assert!(sets.persist_state(&[[1.0; 4]]).is_err());
    assert_eq!(std::fs::read(dir.join("state.conf")).unwrap(), b"broken");
    let before = sets.palette().clone();
    let bad = dir.join("bad.gpl");
    std::fs::write(&bad, b"bad").unwrap();
    assert!(sets.import(&bad).is_err());
    assert_eq!(sets.palette(), &before);
}
#[test]
fn set_limit_and_invalid_edits_are_transactional() {
    let mut sets = ColorSets::default();
    let before = sets.palette().clone();
    for edit in [
        Edit::Remove(100),
        Edit::Move(0, 100),
        Edit::Replace(100, [0.0; 4]),
        Edit::Rename("\n".into()),
        Edit::Add([f32::NAN; 4]),
    ] {
        assert!(sets.edit(edit).is_err());
        assert_eq!(sets.palette(), &before);
    }
    for _ in 1..MAX_SETS {
        sets.add_set(Palette {
            name: "X".into(),
            colors: vec![],
        })
        .unwrap();
    }
    assert_eq!(sets.add_set(Palette::default()), Err(Error::Limit));
}
#[test]
fn history_remembers_64_moves_existing_to_front_and_grid_reflows() {
    let mut c = ColorState::default();
    for i in 0..70 {
        c.set_main([i as f32 / 100.0, 0.0, 0.0, 1.0]);
        c.remember();
    }
    assert_eq!(c.recent.len(), 64);
    assert_eq!(c.recent.last().unwrap()[0], 0.06);
    c.set_main([0.2, 0.0, 0.0, 1.0]);
    c.remember();
    assert_eq!(c.recent.len(), 64);
    assert_eq!(c.recent[0][0], 0.2);
    assert_eq!(colorsets::columns(24.0), 1);
    assert_eq!(colorsets::columns(52.0), 2);
    assert_eq!(colorsets::columns(51.0), 1);
    assert_eq!(colorsets::columns(276.0), 10);
}
#[test]
fn intermediate_bilinear_srgb_and_straight_alpha_values() {
    let corners = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.25],
        [0.0, 0.0, 1.0, 0.5],
        [1.0, 1.0, 1.0, 1.0],
    ];
    for (i, (x, y)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        assert_eq!(colorsets::intermediate(corners, x, y), corners[i]);
    }
    assert_eq!(
        colorsets::intermediate(corners, 0.5, 0.5),
        [0.5, 0.5, 0.5, 0.4375]
    );
    assert_eq!(
        colorsets::intermediate(corners, 0.25, 0.5),
        [0.5, 0.25, 0.5, 0.34375]
    );
}

#[test]
fn imported_files_and_gpl_export_round_trip_through_disk() {
    let dir = directory("import");
    let mut sets = ColorSets::default();
    assert!(sets.attach(dir.clone(), &mut vec![]).is_empty());
    let path = dir.join("input.ACO");
    std::fs::write(&path, aco(2, 0, [65535, 0, 65535, 0], "Magenta")).unwrap();
    sets.import(&path).unwrap();
    assert_eq!(sets.palette().colors[0].name, "Magenta");
    let out = dir.join("export.gpl");
    sets.export(&out).unwrap();
    let expected = sets.palette().clone();
    sets.import(&out).unwrap();
    assert_eq!(sets.palette(), &expected);
}
#[test]
fn folder_scan_limit_blocks_writes_and_preserves_existing_files() {
    let dir = directory("scan-limit");
    for i in 0..260 {
        std::fs::write(dir.join(format!("ignore-{i}")), b"x").unwrap();
    }
    let mut sets = ColorSets::default();
    assert!(sets
        .attach(dir.clone(), &mut vec![])
        .contains(&Error::Limit));
    assert_eq!(sets.edit(Edit::Add([0.0; 4])), Err(Error::Io));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 260);
}

#[test]
fn startup_preserves_settings_and_brush_errors_when_color_set_loading_also_fails() {
    use yolu_app::{pen::PenInput, settings::Problem, YoluApp};
    for lang in Lang::ALL {
        let dir = directory(lang.pick("startup-errors-ja", "startup-errors-en"));
        let settings = dir.join("settings.conf");
        std::fs::write(
            &settings,
            format!(
                "language={}\nmin_undo_steps=invalid\n",
                lang.pick("ja", "en")
            ),
        )
        .unwrap();
        // ブラシのフォルダの代わりにファイルを置き、実際の起動処理で読込失敗を起こす。
        std::fs::write(dir.join("brushes"), b"invalid").unwrap();
        std::fs::create_dir(dir.join("colorsets")).unwrap();
        std::fs::write(dir.join("colorsets/0000000000000001.ycolors"), b"broken").unwrap();
        let app = YoluApp::for_context_with_settings(
            &egui::Context::default(),
            Some(settings),
            PenInput::detached(),
        );
        let settings_reason = Problem::Invalid {
            key: "min_undo_steps",
            value: "invalid".into(),
        }
        .text(lang);
        let brush_reason = app
            .state
            .brush_problem_message()
            .expect("ブラシの読込エラー");
        let color_reason = Error::Header.message(lang);
        let message = &app.state.message;
        for reason in [&settings_reason, &brush_reason, &color_reason] {
            assert!(
                message.contains(reason),
                "通知から理由が消えた: {reason}; 通知: {message}"
            );
        }
        assert!(message.find(&settings_reason).unwrap() < message.find(&brush_reason).unwrap());
        assert!(message.find(&brush_reason).unwrap() < message.find(&color_reason).unwrap());
    }
}
