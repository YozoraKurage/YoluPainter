//! 試験の束（`tests/<束>/main.rs`）の置き方の確かめ。ファイルを置いたのに `main.rs` へ `mod` を足し忘れると、その試験は組まれず、
//! 走らないのに通ったように見える。足し忘れ・消し忘れ（`mod` があるのにファイルが無い）を、ソースで確かめる。
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// 束のフォルダ（`main.rs` を持つもの）の名前。
fn bundles() -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(tests_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && e.path().join("main.rs").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// `main.rs` の `mod 名;`（行頭。`#[path]` で別の場所を指すもの・`pub` つきも数える）。
fn declared_modules(main: &str) -> BTreeSet<String> {
    main.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("pub mod ")
                .or_else(|| line.strip_prefix("mod "))?;
            Some(rest.strip_suffix(';')?.trim().to_string())
        })
        .collect()
}

#[test]
fn the_bundles_are_found() {
    let bundles = bundles();
    for expected in ["headless", "gui_canvas", "gui_shell", "gui_view3d"] {
        assert!(
            bundles.iter().any(|b| b == expected),
            "束が見つからない: {expected}（見つかった: {bundles:?}）"
        );
    }
}

#[test]
fn every_file_in_a_bundle_is_declared_in_its_main_rs_and_every_declared_module_has_its_file() {
    let mut problems = Vec::new();
    for bundle in bundles() {
        let dir = tests_dir().join(&bundle);
        let main = std::fs::read_to_string(dir.join("main.rs")).unwrap();
        let declared = declared_modules(&main);
        let mut files = BTreeSet::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_file() && name.ends_with(".rs") && name != "main.rs" {
                files.insert(name.trim_end_matches(".rs").to_string());
            } else if path.is_dir() && path.join("mod.rs").is_file() {
                files.insert(name);
            }
        }
        for missing in files.difference(&declared) {
            problems.push(format!("{bundle}/{missing}: ファイルはあるが {bundle}/main.rs に `mod {missing};` が無い（この試験は走らない）"));
        }
        for gone in declared.difference(&files) {
            // `common` など束の外の部品は `#[path]` で指す
            if gone != "common" {
                problems.push(format!(
                    "{bundle}/main.rs: `mod {gone};` のファイルが {bundle}/ に無い"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
