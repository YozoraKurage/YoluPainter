//! lilToon の再現のシェーダーの元（`shaders/liltoon/` の部品を、この順につないだもの）。`render.rs` が `shaders/scene.wgsl` の後ろに
//! つないで 1 つのモジュールにする（一様バッファ・束ね・色の変換・影は scene.wgsl のもの）。部品はどの順でも WGSL として同じ意味だが、
//! 説明の頭（`head.wgsl`）を先頭に、宣言は使う所より前に置く順にしてある。部品の 1 行目は出どころ（`// 出どころ: …`）。

/// つないだ元。
pub const SOURCE: &str = concat!(
    include_str!("shaders/liltoon/head.wgsl"),
    include_str!("shaders/liltoon/bindings.wgsl"),
    include_str!("shaders/liltoon/pipeline.wgsl"),
    include_str!("shaders/liltoon/params.wgsl"),
    include_str!("shaders/liltoon/slots.wgsl"),
    include_str!("shaders/liltoon/functions.wgsl"),
    include_str!("shaders/liltoon/light.wgsl"),
    include_str!("shaders/liltoon/output.wgsl"),
    include_str!("shaders/liltoon/layers.wgsl"),
    include_str!("shaders/liltoon/glitter.wgsl"),
    include_str!("shaders/liltoon/surface.wgsl"),
    include_str!("shaders/liltoon/distance_fade.wgsl"),
    include_str!("shaders/liltoon/outline.wgsl"),
    include_str!("shaders/liltoon/view.wgsl"),
);

#[cfg(test)]
mod tests {
    /// 部品の名前と中身（`SOURCE` と同じ順）。
    const PARTS: [(&str, &str); 14] = [
        ("head", include_str!("shaders/liltoon/head.wgsl")),
        ("bindings", include_str!("shaders/liltoon/bindings.wgsl")),
        ("pipeline", include_str!("shaders/liltoon/pipeline.wgsl")),
        ("params", include_str!("shaders/liltoon/params.wgsl")),
        ("slots", include_str!("shaders/liltoon/slots.wgsl")),
        ("functions", include_str!("shaders/liltoon/functions.wgsl")),
        ("light", include_str!("shaders/liltoon/light.wgsl")),
        ("output", include_str!("shaders/liltoon/output.wgsl")),
        ("layers", include_str!("shaders/liltoon/layers.wgsl")),
        ("glitter", include_str!("shaders/liltoon/glitter.wgsl")),
        ("surface", include_str!("shaders/liltoon/surface.wgsl")),
        (
            "distance_fade",
            include_str!("shaders/liltoon/distance_fade.wgsl"),
        ),
        ("outline", include_str!("shaders/liltoon/outline.wgsl")),
        ("view", include_str!("shaders/liltoon/view.wgsl")),
    ];

    #[test]
    fn every_part_in_the_folder_is_joined_once_and_names_where_it_came_from() {
        let joined: String = PARTS.iter().map(|(_, s)| *s).collect();
        assert_eq!(joined, super::SOURCE, "SOURCE と部品の並びが同じ");
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/view3d/shaders/liltoon");
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        let mut named: Vec<String> = PARTS.iter().map(|(n, _)| format!("{n}.wgsl")).collect();
        named.sort();
        assert_eq!(files, named, "フォルダの部品は全部つなぐ");
        for (name, text) in PARTS {
            let first = text.lines().next().unwrap_or_default();
            assert!(
                first.starts_with(
                    "// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）"
                ) && first.ends_with("../THIRD-PARTY-NOTICES.md。"),
                "{name}.wgsl の 1 行目が出どころ: {first}"
            );
            // 部品の境目で宣言が切れないよう、どの部品も改行で終わる
            assert!(text.ends_with('\n'), "{name}.wgsl は改行で終わる");
        }
    }
}
