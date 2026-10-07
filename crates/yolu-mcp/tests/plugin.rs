//! 配るプラグイン（リポジトリの `plugin/` と根の `.claude-plugin/marketplace.json`）が、この版の受け口と合っていること:
//! 目録の版がアプリと同じ・MCP の設定が既定の番号の受け口を指す・Claude Code と Codex の目録が同じスキルと MCP の設定を指す・
//! スキルが書くツールの名前・誤りの種類・引数・資料が本当にある。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use yolu_ops::commands;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn plugin() -> PathBuf {
    root().join("plugin")
}

#[test]
fn the_marketplace_lists_the_plugin_folder() {
    let market = json(&root().join(".claude-plugin/marketplace.json"));
    assert_eq!(market["name"], "yolupainter");
    assert!(market["owner"]["name"].is_string());
    let plugins = market["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0]["name"], "yolupainter");
    let source = plugins[0]["source"].as_str().unwrap();
    assert!(source.starts_with("./"), "相対の道は ./ で始める: {source}");
    assert_eq!(
        root().join(source).canonicalize().unwrap(),
        plugin().canonicalize().unwrap()
    );
}

#[test]
fn both_manifests_carry_the_apps_version_and_point_at_the_shared_parts() {
    let claude = json(&plugin().join(".claude-plugin/plugin.json"));
    let codex = json(&plugin().join(".codex-plugin/plugin.json"));
    for manifest in [&claude, &codex] {
        assert_eq!(manifest["name"], "yolupainter");
        assert_eq!(
            manifest["version"],
            env!("CARGO_PKG_VERSION"),
            "プラグインの版はアプリと同じ（版を上げたら plugin/ の 2 つの plugin.json も上げる）"
        );
        assert!(manifest["description"].as_str().unwrap().len() > 20);
        assert!(manifest["author"]["name"].is_string());
    }
    assert_eq!(claude["description"], codex["description"]);
    // Claude Code は根の .mcp.json と skills/ を自分で読む。Codex は目録で同じ物を指す
    assert_eq!(codex["skills"], "./skills/");
    assert_eq!(codex["mcpServers"], "./.mcp.json");
    let interface = &codex["interface"];
    for (field, limit) in [
        ("displayName", 30),
        ("shortDescription", 30),
        ("longDescription", 4000),
    ] {
        let text = interface[field].as_str().unwrap();
        assert!(
            !text.is_empty() && text.chars().count() <= limit,
            "{field}: {text}"
        );
    }
    let prompts = interface["defaultPrompt"].as_array().unwrap();
    assert!(
        prompts.len() <= 3
            && prompts
                .iter()
                .all(|p| p.as_str().unwrap().chars().count() <= 128)
    );
    for icon in ["composerIcon", "logo"] {
        let path = interface[icon].as_str().unwrap();
        assert!(plugin().join(path).is_file(), "{icon}: {path}");
    }
    // 絵はアプリのロゴそのまま（差し替えたら、ここも同じ物にする）
    assert_eq!(
        std::fs::read(plugin().join("assets/logo.png")).unwrap(),
        std::fs::read(root().join("crates/yolu-app/assets/logo/yolupainter-256.png")).unwrap()
    );
}

#[test]
fn the_claude_desktop_extension_defaults_to_the_same_port() {
    // xtask は yolu-mcp に依らないので、.mcpb の manifest の既定の番号を文字で照らす
    let xtask = std::fs::read_to_string(root().join("crates/xtask/src/main.rs")).unwrap();
    assert!(
        xtask.contains(&format!(
            "const MCPB_DEFAULT_PORT: u16 = {};",
            yolu_mcp::DEFAULT_PORT
        )),
        "crates/xtask/src/main.rs の MCPB_DEFAULT_PORT を {} にする",
        yolu_mcp::DEFAULT_PORT
    );
}

#[test]
fn the_mcp_setting_points_at_the_default_endpoint() {
    let config = json(&plugin().join(".mcp.json"));
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 1);
    let server = &servers["yolupainter"];
    // Claude Code は type が無いと標準入出力のサーバーとして読む。Codex は http を受ける
    assert_eq!(server["type"], "http");
    assert_eq!(server["url"], yolu_mcp::endpoint(yolu_mcp::DEFAULT_PORT));
}

#[test]
fn the_skill_names_only_tools_codes_arguments_and_resources_that_exist() {
    let text = std::fs::read_to_string(plugin().join("skills/yolupainter/SKILL.md")).unwrap();
    let front = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map(|(front, _)| front)
        .expect("頭の YAML");
    let field = |name: &str| {
        front
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{name}: ")))
            .unwrap_or_else(|| panic!("{name} が無い"))
    };
    assert_eq!(field("name"), "yolupainter", "フォルダの名前と同じ");
    let description = field("description");
    assert!(
        description.len() > 40 && description.len() <= 1024,
        "{description}"
    );

    let mut known: BTreeSet<String> = BTreeSet::new();
    for spec in commands() {
        known.insert(spec.tool_name());
        if let Some(props) = spec.args_schema()["properties"].as_object() {
            known.extend(props.keys().cloned());
        }
    }
    let codes = yolu_ops::error_schema()["$defs"]["ErrorCode"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["const"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    known.extend(codes);
    // MCP のサーバーの名前（.mcp.json の鍵）
    known.insert("yolupainter".into());
    let mut checked = 0;
    for (i, part) in text.split('`').enumerate() {
        if i % 2 == 0 {
            continue;
        }
        if let Some(rest) = part.strip_prefix("yolupainter://") {
            let ok = match rest.strip_prefix("docs/") {
                Some(name) => yolu_mcp::docs::find(name).is_some(),
                None => [
                    yolu_mcp::server::URI_COMMANDS,
                    yolu_mcp::server::URI_EFFECT_KINDS,
                ]
                .contains(&part),
            };
            assert!(ok, "スキルの資料 {part} が無い");
            checked += 1;
            continue;
        }
        if part.starts_with("http") {
            assert_eq!(
                part,
                yolu_mcp::endpoint(yolu_mcp::DEFAULT_PORT),
                "スキルの URL は既定の受け口"
            );
            checked += 1;
            continue;
        }
        let word = part.split(':').next().unwrap();
        if !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            assert!(
                known.contains(word),
                "スキルが書く {word} は、ツール・引数・誤りの種類のどれでもない"
            );
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked}");
}
