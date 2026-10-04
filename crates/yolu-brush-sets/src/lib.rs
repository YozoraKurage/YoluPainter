//! 同梱の筆先のデータ。コードも依存も持たない葉のクレート。
//!
//! 筆先の原本（Krita 4 の既定の筆先、CC0 1.0）を 1 バイトも変えずに `include_bytes!` で持つ。読む側は `yolu-io` の
//! `brushes::bundled`。別のクレートにしたのは、11 MB を超えるデータを `yolu-io` の rlib に混ぜず、`yolu-io` を
//! 再ビルドするたびに読み直さないため（リンクのとき、使わない実行ファイルからは参照されないので消える）。
//! 出どころ・許諾・SHA-256 は [`KRITA4_README`]・[`KRITA4_META`]・[`KRITA4_SHA256SUMS`]（どれも原本と一緒にある文書）。

/// 同梱の筆先 1 つ。`file` は `SHA256SUMS` の `brushes/` の下の名前。
#[derive(Clone, Copy, Debug)]
pub struct Tip {
    pub file: &'static str,
    pub bytes: &'static [u8],
}

/// 出どころ・許諾・何を入れて何を入れなかったか。
pub const KRITA4_README: &str = include_str!("../data/krita4/README.md");
/// Krita の束の `meta.xml`（許諾 CC-0 の宣言。原本のまま）。
pub const KRITA4_META: &str = include_str!("../data/krita4/meta.xml");
/// 全ファイルの SHA-256（`sha256sum -c` の形。ファイル名は `brushes/<名前>`）。
pub const KRITA4_SHA256SUMS: &str = include_str!("../data/krita4/SHA256SUMS");

/// Krita 4 の既定の筆先 76 個（`.png`・`.gih`・`.gbr`。ファイル名の順）。
#[rustfmt::skip]
pub static KRITA4: [Tip; 76] = [
    Tip { file: "abominable_snowman.png", bytes: include_bytes!("../data/krita4/brushes/abominable_snowman.png") },
    Tip { file: "bamboo_leaves_random.gih", bytes: include_bytes!("../data/krita4/brushes/bamboo_leaves_random.gih") },
    Tip { file: "bokey_circle.gbr", bytes: include_bytes!("../data/krita4/brushes/bokey_circle.gbr") },
    Tip { file: "brick.gih", bytes: include_bytes!("../data/krita4/brushes/brick.gih") },
    Tip { file: "bristle.png", bytes: include_bytes!("../data/krita4/brushes/bristle.png") },
    Tip { file: "bristles_chisel_dense.png", bytes: include_bytes!("../data/krita4/brushes/bristles_chisel_dense.png") },
    Tip { file: "bristles_circle_dense.png", bytes: include_bytes!("../data/krita4/brushes/bristles_circle_dense.png") },
    Tip { file: "bristles_circle_medium.png", bytes: include_bytes!("../data/krita4/brushes/bristles_circle_medium.png") },
    Tip { file: "bristles_circle_random.gih", bytes: include_bytes!("../data/krita4/brushes/bristles_circle_random.gih") },
    Tip { file: "bristles_circle_sparse.png", bytes: include_bytes!("../data/krita4/brushes/bristles_circle_sparse.png") },
    Tip { file: "bristles_grouped.gbr", bytes: include_bytes!("../data/krita4/brushes/bristles_grouped.gbr") },
    Tip { file: "chalk.png", bytes: include_bytes!("../data/krita4/brushes/chalk.png") },
    Tip { file: "chalk_chisel.gih", bytes: include_bytes!("../data/krita4/brushes/chalk_chisel.gih") },
    Tip { file: "chalk_chisel_random.gih", bytes: include_bytes!("../data/krita4/brushes/chalk_chisel_random.gih") },
    Tip { file: "chalk_chisel_random_small.gih", bytes: include_bytes!("../data/krita4/brushes/chalk_chisel_random_small.gih") },
    Tip { file: "chalk_round_hard.png", bytes: include_bytes!("../data/krita4/brushes/chalk_round_hard.png") },
    Tip { file: "chalk_sparse.png", bytes: include_bytes!("../data/krita4/brushes/chalk_sparse.png") },
    Tip { file: "chisel_bent_rough.gih", bytes: include_bytes!("../data/krita4/brushes/chisel_bent_rough.gih") },
    Tip { file: "chisel_dense_smear.png", bytes: include_bytes!("../data/krita4/brushes/chisel_dense_smear.png") },
    Tip { file: "chisel_eroded.png", bytes: include_bytes!("../data/krita4/brushes/chisel_eroded.png") },
    Tip { file: "chisel_knife.gbr", bytes: include_bytes!("../data/krita4/brushes/chisel_knife.gbr") },
    Tip { file: "chisel_soft.png", bytes: include_bytes!("../data/krita4/brushes/chisel_soft.png") },
    Tip { file: "chisel_streaks.png", bytes: include_bytes!("../data/krita4/brushes/chisel_streaks.png") },
    Tip { file: "circle_hard_eroded.gih", bytes: include_bytes!("../data/krita4/brushes/circle_hard_eroded.gih") },
    Tip { file: "crackles.gbr", bytes: include_bytes!("../data/krita4/brushes/crackles.gbr") },
    Tip { file: "fairy-dust.gih", bytes: include_bytes!("../data/krita4/brushes/fairy-dust.gih") },
    Tip { file: "floor.gih", bytes: include_bytes!("../data/krita4/brushes/floor.gih") },
    Tip { file: "freckles.png", bytes: include_bytes!("../data/krita4/brushes/freckles.png") },
    Tip { file: "gradient.png", bytes: include_bytes!("../data/krita4/brushes/gradient.png") },
    Tip { file: "graphite_grain.gih", bytes: include_bytes!("../data/krita4/brushes/graphite_grain.gih") },
    Tip { file: "grass.gih", bytes: include_bytes!("../data/krita4/brushes/grass.gih") },
    Tip { file: "grass_patch.gih", bytes: include_bytes!("../data/krita4/brushes/grass_patch.gih") },
    Tip { file: "hair.png", bytes: include_bytes!("../data/krita4/brushes/hair.png") },
    Tip { file: "hearts.gih", bytes: include_bytes!("../data/krita4/brushes/hearts.gih") },
    Tip { file: "impressionism_brush.gih", bytes: include_bytes!("../data/krita4/brushes/impressionism_brush.gih") },
    Tip { file: "leaves.png", bytes: include_bytes!("../data/krita4/brushes/leaves.png") },
    Tip { file: "mountains_distant.gih", bytes: include_bytes!("../data/krita4/brushes/mountains_distant.gih") },
    Tip { file: "noise.gih", bytes: include_bytes!("../data/krita4/brushes/noise.gih") },
    Tip { file: "oil_bristle.png", bytes: include_bytes!("../data/krita4/brushes/oil_bristle.png") },
    Tip { file: "oil_knife.png", bytes: include_bytes!("../data/krita4/brushes/oil_knife.png") },
    Tip { file: "paint_splats.gih", bytes: include_bytes!("../data/krita4/brushes/paint_splats.gih") },
    Tip { file: "plain_rake.png", bytes: include_bytes!("../data/krita4/brushes/plain_rake.png") },
    Tip { file: "rake_dense.png", bytes: include_bytes!("../data/krita4/brushes/rake_dense.png") },
    Tip { file: "rake_dotted.png", bytes: include_bytes!("../data/krita4/brushes/rake_dotted.png") },
    Tip { file: "rake_flat.png", bytes: include_bytes!("../data/krita4/brushes/rake_flat.png") },
    Tip { file: "rake_sparse.png", bytes: include_bytes!("../data/krita4/brushes/rake_sparse.png") },
    Tip { file: "random-debris.gbr", bytes: include_bytes!("../data/krita4/brushes/random-debris.gbr") },
    Tip { file: "random-vegetal.gih", bytes: include_bytes!("../data/krita4/brushes/random-vegetal.gih") },
    Tip { file: "random_particles.png", bytes: include_bytes!("../data/krita4/brushes/random_particles.png") },
    Tip { file: "rock.png", bytes: include_bytes!("../data/krita4/brushes/rock.png") },
    Tip { file: "rock_light.gih", bytes: include_bytes!("../data/krita4/brushes/rock_light.gih") },
    Tip { file: "rock_pitted.gih", bytes: include_bytes!("../data/krita4/brushes/rock_pitted.gih") },
    Tip { file: "rock_scraped.gih", bytes: include_bytes!("../data/krita4/brushes/rock_scraped.gih") },
    Tip { file: "scales.png", bytes: include_bytes!("../data/krita4/brushes/scales.png") },
    Tip { file: "scratches_rough.gih", bytes: include_bytes!("../data/krita4/brushes/scratches_rough.gih") },
    Tip { file: "scribbles.png", bytes: include_bytes!("../data/krita4/brushes/scribbles.png") },
    Tip { file: "shapes_mech_random.gih", bytes: include_bytes!("../data/krita4/brushes/shapes_mech_random.gih") },
    Tip { file: "shapes_round_random.gih", bytes: include_bytes!("../data/krita4/brushes/shapes_round_random.gih") },
    Tip { file: "shapes_spiked_random.gih", bytes: include_bytes!("../data/krita4/brushes/shapes_spiked_random.gih") },
    Tip { file: "smear_paint.png", bytes: include_bytes!("../data/krita4/brushes/smear_paint.png") },
    Tip { file: "smoke.png", bytes: include_bytes!("../data/krita4/brushes/smoke.png") },
    Tip { file: "snow.gih", bytes: include_bytes!("../data/krita4/brushes/snow.gih") },
    Tip { file: "sparkle.png", bytes: include_bytes!("../data/krita4/brushes/sparkle.png") },
    Tip { file: "spike_blob.png", bytes: include_bytes!("../data/krita4/brushes/spike_blob.png") },
    Tip { file: "spike_eroded.png", bytes: include_bytes!("../data/krita4/brushes/spike_eroded.png") },
    Tip { file: "spines.png", bytes: include_bytes!("../data/krita4/brushes/spines.png") },
    Tip { file: "splat_dots.png", bytes: include_bytes!("../data/krita4/brushes/splat_dots.png") },
    Tip { file: "splats_large.gih", bytes: include_bytes!("../data/krita4/brushes/splats_large.gih") },
    Tip { file: "square_eroded.png", bytes: include_bytes!("../data/krita4/brushes/square_eroded.png") },
    Tip { file: "square_rough.png", bytes: include_bytes!("../data/krita4/brushes/square_rough.png") },
    Tip { file: "square_rough_lightgrey.png", bytes: include_bytes!("../data/krita4/brushes/square_rough_lightgrey.png") },
    Tip { file: "starfield.png", bytes: include_bytes!("../data/krita4/brushes/starfield.png") },
    Tip { file: "vegetal.gbr", bytes: include_bytes!("../data/krita4/brushes/vegetal.gbr") },
    Tip { file: "vegetal_stylised.gih", bytes: include_bytes!("../data/krita4/brushes/vegetal_stylised.gih") },
    Tip { file: "water_still.gih", bytes: include_bytes!("../data/krita4/brushes/water_still.gih") },
    Tip { file: "watercolor.gih", bytes: include_bytes!("../data/krita4/brushes/watercolor.gih") },
];

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn sums() -> BTreeMap<String, String> {
        KRITA4_SHA256SUMS
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (hash, name) = l.split_once(' ').expect("sha256sum の行");
                let name = name.trim().trim_start_matches('*');
                (
                    name.strip_prefix("brushes/").unwrap_or(name).to_string(),
                    hash.to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn every_file_is_listed_and_nothing_extra_is_shipped() {
        let listed: Vec<&str> = KRITA4.iter().map(|t| t.file).collect();
        let sums = sums();
        assert_eq!(
            listed,
            sums.keys().map(String::as_str).collect::<Vec<_>>(),
            "表と SHA256SUMS"
        );
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/krita4/brushes");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        on_disk.sort();
        assert_eq!(on_disk, listed, "フォルダーと表");
    }

    #[test]
    fn every_file_is_the_unmodified_upstream_file() {
        let sums = sums();
        for tip in &KRITA4 {
            let actual: String = Sha256::digest(tip.bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert_eq!(actual, sums[tip.file], "{}", tip.file);
        }
    }

    #[test]
    fn the_licence_statement_and_the_source_stay_with_the_files() {
        assert!(KRITA4_META.contains("CC-0"));
        assert!(KRITA4_README.contains("CC0 1.0"));
        assert!(KRITA4_README
            .contains("4180f474052305e9de6eaac1d832c1ddeb0654142bcde2eb2460629cf23796d0"));
    }

    #[test]
    fn the_table_is_in_ordinal_order() {
        let names: Vec<&str> = KRITA4.iter().map(|t| t.file).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }
}
