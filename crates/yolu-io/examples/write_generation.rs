//! cargo run -p yolu-io --example write_generation -- input.ylp output-folder
//!
//! 開いた `.ylp` のエントリをそのまま世代に入れた置き場（`as-opened`）と、`sets/<ID>/` の下の入れ子の名前（合成の PNG など）を
//! 除いた置き場（`flat-names`）を、出力のフォルダの下に新しく書く。Unity 版の `GenerationStore.Load` に読ませて、名前の範囲の
//! 違いを記録するため（`tools/io-fixtures/generate.py --generation`）。
use std::fs;
use std::path::Path;
use yolu_io::{CommitOptions, Error, Files, GenerationStore, Project, Result};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(Error::InvalidData(
            "使い方: write_generation <入力.ylp> <出力フォルダ>（出力は新しいフォルダ）".into(),
        ));
    }
    let output = Path::new(&args[1]);
    if output.exists() {
        return Err(Error::InvalidData("出力のフォルダが既にあります".into()));
    }
    let project = Project::read(&fs::read(&args[0])?)?;
    let as_opened: Files = project.original_archive().entries().clone();
    let flat_names: Files = as_opened
        .iter()
        .filter(|(name, _)| name.matches('/').count() <= 2)
        .map(|(name, data)| (name.clone(), data.clone()))
        .collect();
    for (folder, files) in [("as-opened", &as_opened), ("flat-names", &flat_names)] {
        GenerationStore::new(output.join(folder))
            .commit(
                files,
                &CommitOptions {
                    expected: None,
                    keep: None,
                    share: true,
                },
            )
            .map_err(|e| Error::InvalidData(format!("{folder}: {e}")))?;
        eprintln!("{folder}: {} エントリ", files.len());
    }
    Ok(())
}
