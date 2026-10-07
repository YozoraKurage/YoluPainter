//! README が、命令・誤りの種類・効果の種類の全部を載せていること（足したのに文書に無い、を試験で見つける）。

use yolu_ops::commands;

const README: &str = include_str!("../README.md");

#[test]
fn the_readme_lists_every_command_error_code_and_effect_kind() {
    for spec in commands() {
        assert!(
            README.contains(&format!("`{}`", spec.name)),
            "README に命令 {} が無い",
            spec.name
        );
    }
    let schema = yolu_ops::error_schema();
    let codes = schema["$defs"]["ErrorCode"]["oneOf"]
        .as_array()
        .expect("説明つきの変種は oneOf");
    assert!(codes.len() >= 19);
    for code in codes {
        let name = code["const"].as_str().unwrap();
        assert!(
            README.contains(&format!("| `{name}` |")),
            "README の誤りの表に {name} が無い"
        );
    }
    for kind in yolu_core::effects::catalog::kinds() {
        assert!(
            README.contains(&format!("`{}`", kind.id)),
            "README に効果の種類 {} が無い",
            kind.id
        );
    }
    for template in yolu_core::export::ExportTemplate::built_in() {
        assert!(
            README.contains(&format!("`{}`", template.id)),
            "README にテンプレート {} が無い",
            template.id
        );
    }
    // 壊す印: 「壊す」の命令は確認が要ることを、表で言う
    for spec in commands()
        .iter()
        .filter(|c| c.danger == yolu_ops::Danger::Always)
    {
        let row = README
            .lines()
            .find(|l| l.starts_with(&format!("| `{}`", spec.name)))
            .expect("表の行");
        assert!(row.contains("壊す"), "{row}");
    }
}
