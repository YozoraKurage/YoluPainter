//! 本物の `.sut`（CLIP STUDIO の書き出し。リポジトリには入れない）で、取り込みが写した設定の一覧を数だけで出す
//! （ブラシの名前・ファイル名・中身の文字は出さない）。環境変数 `YOLU_REAL_SUT_DIR` のフォルダの `.sut` を大きさの順に 1 から数える。
//! `cargo test -p yolu-io --test brush_import_sut_real -- --ignored --nocapture` で表が出る。

use yolu_io::brushes::{import, ImportedBrush};

fn summary(b: &ImportedBrush) -> String {
    let br = &b.brush;
    let s = &br.base;
    let tips = br.tip.images.len() + usize::from(br.tip.image.is_some());
    let notes: Vec<String> = b.unrepresented.iter().map(|n| format!("{n:?}")).collect();
    format!(
        "r={:.1} op={:.2} fl={:.2} hd={:.2} sp={:.2} rn={:.2} ang={:.1} tips={} tex={} \
         prs={}{}{} tilt={}{}{} jit_ang={:.2} taper={:.0}/{:.0} stab={:.0} | notes: {}",
        s.radius,
        s.opacity,
        s.flow,
        s.hardness,
        s.spacing,
        br.tip.roundness,
        br.tip.angle,
        tips,
        u8::from(br.texture.is_some()),
        u8::from(s.pressure_size),
        u8::from(s.pressure_opacity),
        u8::from(s.pressure_flow),
        u8::from(br.controls.tilt_size),
        u8::from(br.controls.tilt_opacity),
        u8::from(br.controls.tilt_flow),
        br.jitter.angle,
        br.assist.taper_in,
        br.assist.taper_out,
        br.assist.stabilizer,
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
