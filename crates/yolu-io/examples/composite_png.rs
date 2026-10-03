//! cargo run -p yolu-io --example composite_png -- input.ylp output.png
use std::{fs, io::Write};
use yolu_io::{composite_png, Error, Project, Result};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(Error(
            "使い方: composite_png <入力.ylp> <出力.png>（出力は新しいファイル）".into(),
        ));
    }
    let project = Project::read(&fs::read(&args[0])?)?;
    for note in project.notes() {
        eprintln!("{note}");
    }
    let set = project
        .sets()
        .iter()
        .find(|s| s.id == project.current_set())
        .expect("読み込み時に検証済み");
    let doc = set.document.to_core()?;
    let png = composite_png(&doc)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?;
    output.write_all(&png)?;
    output.sync_all()?;
    eprintln!(
        "セット「{}」の合成を{}×{}のPNGへ書きました",
        set.name,
        doc.width(),
        doc.height()
    );
    Ok(())
}
