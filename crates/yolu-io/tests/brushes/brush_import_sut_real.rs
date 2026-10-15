//! 本物の `.sut`（CLIP STUDIO の書き出し。リポジトリには入れない）で、取り込みが写した設定の一覧を数だけで出す
//! （ブラシの名前・ファイル名・中身の文字は出さない）。環境変数 `YOLU_REAL_SUT_DIR` のフォルダの `.sut` を大きさの順に 1 から数える。
//! `cargo test -p yolu-io --test brushes brush_import_sut_real:: -- --ignored --nocapture` で表が出る。

use yolu_io::brushes::{import, ImportedBrush};

fn summary(b: &ImportedBrush) -> String {
    let br = &b.brush;
    let s = &br.base;
    let tips = br.tip.images.len() + usize::from(br.tip.image.is_some());
    let notes: Vec<String> = b.unrepresented.iter().map(|n| format!("{n:?}")).collect();
    format!(
        "r={:.1} op={:.2} fl={:.2} hd={:.2} sp={:.2} rn={:.2} ang={:.1} tips={} tex={} \
         prs={}{}{} tilt={}{}{} jit_ang={:.2} taper={:.0}/{:.0} stab={:.0} \
         clr=tip{}/sub{:.2}/stroke{}/{} clr_inf={}{}{}{} spray={} spray_inf={}{} \
         dual={} dual_inf={}{}{}{} | notes: {}",
        s.radius,
        s.opacity,
        s.flow,
        s.hardness,
        s.spacing,
        br.tip.roundness,
        br.tip.angle,
        tips,
        u8::from(br.texture.is_some()),
        u8::from(br.influences.size.pressure.on),
        u8::from(br.influences.opacity.pressure.on),
        u8::from(br.influences.flow.pressure.on),
        u8::from(br.influences.size.tilt.on),
        u8::from(br.influences.opacity.tilt.on),
        u8::from(br.influences.flow.tilt.on),
        br.jitter.angle,
        br.assist.taper_in,
        br.assist.taper_out,
        br.assist.stabilizer,
        u8::from(br.color.tip.is_some()),
        br.color.sub_mix,
        u8::from(br.color.stroke.is_some()),
        br.color.target.id(),
        u8::from(br.influences.color_hue.is_on()),
        u8::from(br.influences.color_saturation.is_on()),
        u8::from(br.influences.color_value.is_on()),
        u8::from(br.influences.color_sub_mix.is_on()),
        br.spray.map_or("-".to_string(), |p| format!(
            "{:.1}/{}/{:.0}/{:.2}/{:.1}/{}",
            p.size,
            u8::from(p.link_size),
            p.density,
            p.bias,
            p.angle,
            p.random_angle
                .map_or("-".to_string(), |r| format!("{r:.2}"))
        )),
        u8::from(br.influences.spray_size.is_on()),
        u8::from(br.influences.spray_density.is_on()),
        br.dual.as_ref().map_or("-".to_string(), |d| format!(
            "{:.1}/{}/{}/{:.2}/{:.2}/{:?}/{}{}{}/{:.2}",
            d.radius,
            u8::from(d.tip.is_some()),
            u8::from(d.link_size),
            d.flow,
            d.spacing,
            d.mode,
            u8::from(d.angle_tilt),
            u8::from(d.angle_rotation),
            u8::from(d.angle_line),
            d.angle_random
        )),
        u8::from(br.influences.dual_size.is_on()),
        u8::from(br.influences.dual_flow.is_on()),
        u8::from(br.influences.dual_spacing.is_on()),
        u8::from(br.influences.dual_thickness.is_on()),
        notes.join(", ")
    )
}

#[test]
#[ignore = "本物の .sut が要る（YOLU_REAL_SUT_DIR）"]
fn print_what_the_real_files_import_as() {
    let dir = std::env::var("YOLU_REAL_SUT_DIR")
        .expect("YOLU_REAL_SUT_DIR に .sut のあるフォルダを入れる");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sut")))
        .collect();
    files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0));
    assert!(!files.is_empty());
    for (i, path) in files.iter().enumerate() {
        match import(path) {
            Ok(set) => {
                for b in &set.brushes {
                    println!("#{}: {}", i + 1, summary(b));
                }
                let file_notes: Vec<String> = set.notes.iter().map(|n| format!("{n:?}")).collect();
                if !file_notes.is_empty() || !set.skipped.is_empty() {
                    println!(
                        "#{} file: skipped={} notes: {}",
                        i + 1,
                        set.skipped.len(),
                        file_notes.join(", ")
                    );
                }
            }
            Err(e) => println!("#{}: ERR {e:?}", i + 1),
        }
    }
}

/// 本物の `.sut` のフォルダを、手で選んだフォルダとして一覧にして覗いても、フォルダの中が何も変わらないこと（読むだけ）。
#[test]
#[ignore = "本物の .sut が要る（YOLU_REAL_SUT_DIR）"]
fn the_real_folder_is_listed_and_peeked_and_left_alone() {
    use yolu_io::brushes::clipstudio::{peek, scan, Limits};
    let dir = std::path::PathBuf::from(
        std::env::var("YOLU_REAL_SUT_DIR")
            .expect("YOLU_REAL_SUT_DIR に .sut のあるフォルダを入れる"),
    );
    let state = |dir: &std::path::Path| -> Vec<(u64, Option<std::time::SystemTime>)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    e.metadata().unwrap(),
                )
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.into_iter()
            .map(|(_, m)| (m.len(), m.modified().ok()))
            .collect()
    };
    let before = state(&dir);
    let started = std::time::Instant::now();
    let found = scan(std::slice::from_ref(&dir), Limits::default());
    println!(
        "scan: {} files, missing={:?}, truncated={}",
        found.files.len(),
        found.missing,
        found.truncated
    );
    assert!(!found.files.is_empty());
    for (i, f) in found.files.iter().enumerate() {
        let t = std::time::Instant::now();
        match peek(&f.path) {
            Ok(p) => println!(
                "#{}: brushes={} names={} preview_nonzero={} ({} ms)",
                i + 1,
                p.brushes,
                p.names.len(),
                p.preview.alpha.iter().filter(|a| **a > 0).count(),
                t.elapsed().as_millis()
            ),
            Err(e) => println!("#{}: ERR {e:?}", i + 1),
        }
    }
    println!("total {} ms", started.elapsed().as_millis());
    assert_eq!(
        state(&dir),
        before,
        "読んだだけで、更新時刻・大きさが変わらない"
    );
}
