//! 共有のキー処理を動かさず、その呼び出しから一覧の Rust 式を作る。
//! 対応外の構文は黙って飛ばさずビルドを失敗させる。
use std::{env, fs, path::PathBuf};

fn code(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn calls<'a>(source: &'a str, name: &str) -> Vec<&'a str> {
    let mut rest = source;
    let mut out = Vec::new();
    while let Some(start) = rest.find(name) {
        rest = &rest[start + name.len()..];
        let mut depth = 1;
        let end = rest
            .char_indices()
            .find_map(|(i, c)| {
                if c == '(' {
                    depth += 1;
                }
                if c == ')' {
                    depth -= 1;
                }
                (depth == 0).then_some(i)
            })
            .expect("キー定義の括弧が閉じていません");
        out.push(&rest[..end]);
        rest = &rest[end + 1..];
    }
    out
}

/// 既存の判定本体をそのまま Rust として使い、条件の優先順も共有する。
fn braced<'a>(source: &'a str, marker: &str) -> &'a str {
    let rest = source
        .split_once(marker)
        .expect("入力判定の形が変わりました")
        .1;
    let from = rest.find('{').expect("入力判定の本体がありません");
    let rest = &rest[from..];
    let mut depth = 0;
    let end = rest
        .char_indices()
        .find_map(|(i, c)| {
            if c == '{' {
                depth += 1;
            }
            if c == '}' {
                depth -= 1;
            }
            (depth == 0).then_some(i)
        })
        .expect("入力判定の括弧が閉じていません");
    &rest[..=end]
}

fn generate_gestures() {
    // 判定の本体を写すので、元の入力処理が替わったら作り直す。
    for file in ["src/view3d/input.rs", "src/stencil/input.rs"] {
        println!("cargo:rerun-if-changed={file}");
    }
    let view = code(&fs::read_to_string("src/view3d/input.rs").unwrap());
    let stencil = code(&fs::read_to_string("src/stencil/input.rs").unwrap());
    let nav = braced(&view, "fn nav_of(");
    let stencil_match = braced(&stencil, "let kind = match button");
    assert_eq!(
        stencil_match.matches("return false").count(),
        1,
        "ステンシルの判定の形が変わりました"
    );
    let stencil_match = stencil_match.replace("return false", "return None");
    let held_key = |source: &str| {
        source
            .split("i.key_down(")
            .nth(1)
            .expect("保持キーがありません")
            .split(')')
            .next()
            .unwrap()
            .to_owned()
    };
    let output = format!("fn navigation(button: PointerButton, m: &Modifiers, space: bool) -> Option<Nav> {nav}\nfn stencil(button: PointerButton, modifiers: &Modifiers) -> Option<DragKind> {{ Some(match button {stencil_match}) }}\nfn space_key() -> Key {{ {} }}\nfn stencil_key() -> Key {{ {} }}\n", held_key(&view), held_key(&stencil));
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("shortcut_gestures.rs"),
        output,
    )
    .unwrap();
}

pub fn generate() {
    generate_gestures();
    for file in [
        "shortcut_catalog_build.rs",
        "src/shell.rs",
        "src/clipboard/keys.rs",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    let shell = fs::read_to_string("src/shell.rs").unwrap();
    let handle = shell
        .split("pub fn handle_shortcuts(")
        .nth(1)
        .expect("キー処理がありません")
        .split("pub fn options_bar(")
        .next()
        .unwrap();
    let source = code(handle);
    let declaration = |source: &str| {
        let expression = source
            .split("let cmd_shift = ")
            .nth(1)
            .expect("修飾キーの定義がありません")
            .split(';')
            .next()
            .unwrap();
        format!("let cmd_shift = {expression};")
    };
    let mut output = format!(
        "pub fn bindings() -> Vec<Binding> {{ {} let mut result = vec![\n",
        declaration(&source)
    );
    let direct = calls(&source, "        key(");
    let direct_count = direct.len();
    for call in direct {
        let mut parts = call.splitn(3, ',');
        let modifiers = parts.next().unwrap().trim();
        let key = parts.next().expect("キーがありません").trim();
        let action = parts
            .next()
            .expect("操作がありません")
            .trim()
            .trim_end_matches(',');
        assert!(key.starts_with("Key::"), "キーの構文が変わりました");
        output.push_str(&format!(
            "Binding {{ modifiers: {modifiers}, key: {key}, action: {action} }},\n"
        ));
    }
    let clipboard = code(&fs::read_to_string("src/clipboard/keys.rs").unwrap());
    output.push_str(&format!("]; {} result.extend([\n", declaration(&clipboard)));
    for part in clipboard.split("if i.consume_key(").skip(1) {
        let (keys, tail) = part.split_once(')').expect("クリップボードのキーの構文");
        let (modifiers, key) = keys.split_once(',').expect("修飾キーの構文");
        let action = tail
            .split("push(")
            .nth(1)
            .unwrap()
            .split(')')
            .next()
            .unwrap();
        assert!(action.starts_with("ClipAction::"));
        output.push_str(&format!(
            "Binding {{ modifiers: {modifiers}, key: {key}, action: Action::Clip({action}) }},\n"
        ));
    }
    output.push_str("]); result }\n");
    let arrows = source
        .split("for (k, dir) in [")
        .nth(1)
        .expect("移動キーの構文")
        .split("] {")
        .next()
        .unwrap();
    let keys: Vec<_> = arrows
        .split("Key::")
        .skip(1)
        .map(|s| s.split(',').next().unwrap())
        .collect();
    assert_eq!(keys.len(), 4, "移動キーの構文が変わりました");
    assert_eq!(
        source.matches("Key::").count(),
        direct_count + keys.len(),
        "一覧に入らないキーの定義があります"
    );
    output.push_str(&format!(
        "pub fn movement_keys() -> [Key; 4] {{ [{}] }}\n",
        keys.iter()
            .map(|k| format!("Key::{k}"))
            .collect::<Vec<_>>()
            .join(",")
    ));
    let text = source
        .split("egui::Event::Text(s) if s == ")
        .nth(1)
        .expect("回転の文字の構文")
        .split(')')
        .next()
        .unwrap();
    output.push_str(&format!(
        "pub fn rotation_text() -> &'static str {{ {text} }}\n"
    ));
    output.push_str("pub fn context_keys() -> Vec<(&'static str, Key)> { vec![\n");
    for (scope, file) in [
        ("canvas", "src/canvas/mod.rs"),
        ("view3d", "src/view3d/input.rs"),
        ("stencil", "src/stencil/input.rs"),
    ] {
        println!("cargo:rerun-if-changed={file}");
        let source = code(&fs::read_to_string(file).unwrap());
        let source = source.split("#[cfg(test)]").next().unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for marker in ["key_down(Key::", "key: Key::"] {
            for part in source.split(marker).skip(1) {
                let key = part
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .next()
                    .unwrap();
                if seen.insert(key) {
                    output.push_str(&format!("({scope:?}, Key::{key}),\n"));
                }
            }
        }
    }
    output.push_str("] }\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("shortcut_catalog.rs"),
        output,
    )
    .unwrap();
}
