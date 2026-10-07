//! 試験の束（`tests/<束>/main.rs`）の置き方の確かめ。ファイルを置いたのに `main.rs` へ `mod` を足し忘れると、その試験はビルドされず、
//! 走らないのに通ったように見える。足し忘れ・消し忘れ（`mod` があるのにファイルが無い）と、直下（`tests/<名前>.rs`）に理由の無い
//! 1 ファイル 1 本の試験が増えていないかを、ソースで確かめる。
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 束の名前。
const BUNDLES: [&str; 4] = ["edit", "effects", "reference", "surface"];

/// 直下に 1 ファイル 1 本で置く試験（プロセス全体の状態を変える・数える。理由は各ファイルの頭）。ここに無いファイルが直下にあれば落とす。
const STANDALONE: [&str; 8] = [
    "brush",
    "document",
    "golden",
    "mix",
    "parallelism",
    "paths",
    "pressure",
    "seam_memory",
];

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

/// `main.rs` の `mod 名;`（行頭）を、束の中のファイルと、`#[path]` で束の外を指す共通の部品に分ける。
fn declared_modules(dir: &Path, main: &str) -> (BTreeSet<String>, Vec<String>) {
    let mut files = BTreeSet::new();
    let mut problems = Vec::new();
    let mut path: Option<String> = None;
    for line in main.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#[path = \"") {
            path = rest.strip_suffix("\"]").map(str::to_string);
            continue;
        }
        let Some(name) = line
            .strip_prefix("pub mod ")
            .or_else(|| line.strip_prefix("mod "))
            .and_then(|rest| rest.strip_suffix(';'))
        else {
            continue;
        };
        match path.take() {
            Some(p) if !dir.join(&p).is_file() => {
                problems.push(format!("`#[path = \"{p}\"] mod {name};` のファイルが無い"))
            }
            Some(_) => {}
            None => {
                files.insert(name.trim().to_string());
            }
        }
    }
    (files, problems)
}

#[test]
fn the_bundles_are_found() {
    assert_eq!(bundles(), BUNDLES.map(String::from).to_vec());
}

#[test]
fn every_file_in_a_bundle_is_declared_in_its_main_rs_and_every_declared_module_has_its_file() {
    let mut problems = Vec::new();
    for bundle in bundles() {
        let dir = tests_dir().join(&bundle);
        let main = std::fs::read_to_string(dir.join("main.rs")).unwrap();
        let (declared, bad_paths) = declared_modules(&dir, &main);
        problems.extend(bad_paths.iter().map(|p| format!("{bundle}/main.rs: {p}")));
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
            problems.push(format!(
                "{bundle}/main.rs: `mod {gone};` のファイルが {bundle}/ に無い"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn only_the_listed_tests_stand_alone_directly_under_tests() {
    let mut found: Vec<String> = std::fs::read_dir(tests_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "rs"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    found.sort();
    assert_eq!(
        found,
        STANDALONE.map(String::from).to_vec(),
        "直下の 1 ファイル 1 本の試験が一覧と違う。新しい試験は束（{BUNDLES:?}）のフォルダに置き、その main.rs に `mod` を足す。\
         プロセス全体の状態を変えるので束に入れられないときだけ、ファイルの頭に理由を書いて STANDALONE に足す"
    );
}
