//! 画面の文の .ftl（`locales/<言語>/<モジュール>.ftl`）を読む部品と、キーの見張り。ソースを読む決まりのテスト（`no_instruction_text`・
//! `no_developer_words`・`notice_rules`）は、ソースの直書きの文に加えて、ここで読んだ .ftl の文も見る。
//! 見張り: 日英の .ftl がどれも読めて誤りが無い／日英でキーの組と引数の名前の組が同じ／コードが引くキーが日英の両方にあり、
//! 渡す引数の名前が文の引数と同じ／どこからも引かれないキーが無い／`reason-*` の日本語が core の `Reason::ja` と同じ。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fluent_syntax::ast;

/// .ftl の 1 つの文（1 つの言語の 1 つのキー）。
#[derive(Clone, Debug)]
pub(crate) struct Message {
    /// `ja`・`en`
    pub(crate) lang: String,
    /// `locales/ja/panels.ftl` のような道
    pub(crate) file: String,
    pub(crate) key: String,
    /// 値の文（選びの枝ごとに 1 つ。引数の置き場は `{ $名前 }`）
    pub(crate) texts: Vec<String>,
    /// 値が使う引数の名前
    pub(crate) vars: BTreeSet<String>,
}

pub(crate) const LANGS: [&str; 2] = ["ja", "en"];

fn locales() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("locales")
}

/// 言語ごとの .ftl の道（ファイルの名前の順）。
pub(crate) fn ftl_files(lang: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(locales().join(lang))
        .unwrap_or_else(|e| panic!("locales/{lang} を読めない: {e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ftl"))
        .collect();
    files.sort();
    files
}

/// 文字列のリテラル（`{"…"}`）の中身を、実際の文字へ戻す（`\\`・`\"`・`\uXXXX`・`\UXXXXXX`）。
fn unescape(raw: &str) -> String {
    fluent_syntax::unicode::unescape_unicode_to_string(raw).into_owned()
}

fn inline_texts(
    expression: &ast::InlineExpression<&str>,
    vars: &mut BTreeSet<String>,
) -> Vec<String> {
    use ast::InlineExpression as I;
    match expression {
        I::StringLiteral { value } => vec![unescape(value)],
        I::NumberLiteral { value } => vec![(*value).to_owned()],
        I::VariableReference { id } => {
            vars.insert(id.name.to_owned());
            vec![format!("{{ ${} }}", id.name)]
        }
        I::Placeable { expression } => expression_texts(expression, vars),
        I::MessageReference { id, .. } => vec![format!("{{ {} }}", id.name)],
        I::TermReference { id, .. } => vec![format!("{{ -{} }}", id.name)],
        I::FunctionReference { id, .. } => vec![format!("{{ {}() }}", id.name)],
    }
}

fn expression_texts(
    expression: &ast::Expression<&str>,
    vars: &mut BTreeSet<String>,
) -> Vec<String> {
    match expression {
        ast::Expression::Inline(inline) => inline_texts(inline, vars),
        ast::Expression::Select { selector, variants } => {
            inline_texts(selector, vars);
            variants
                .iter()
                .flat_map(|variant| pattern_texts(&variant.value, vars))
                .collect()
        }
    }
}

/// 値の文（選びの枝を全部たどる）。
fn pattern_texts(pattern: &ast::Pattern<&str>, vars: &mut BTreeSet<String>) -> Vec<String> {
    let mut texts = vec![String::new()];
    for element in &pattern.elements {
        let pieces = match element {
            ast::PatternElement::TextElement { value } => vec![(*value).to_owned()],
            ast::PatternElement::Placeable { expression } => expression_texts(expression, vars),
        };
        texts = texts
            .iter()
            .flat_map(|head| pieces.iter().map(move |piece| format!("{head}{piece}")))
            .collect();
    }
    texts
}

/// 1 つの .ftl を読む（読めない所は誤りの文）。
pub(crate) fn parse(lang: &str, path: &Path) -> (Vec<Message>, Vec<String>) {
    let source = std::fs::read_to_string(path).unwrap();
    let file = format!(
        "locales/{lang}/{}",
        path.file_name().unwrap().to_string_lossy()
    );
    let (resource, mut errors) = match fluent_syntax::parser::parse(source.as_str()) {
        Ok(resource) => (resource, Vec::new()),
        Err((resource, errors)) => (
            resource,
            errors
                .into_iter()
                .map(|e| format!("{file}: {e:?}"))
                .collect(),
        ),
    };
    let mut out = Vec::new();
    for entry in &resource.body {
        match entry {
            ast::Entry::Message(message) => {
                let mut vars = BTreeSet::new();
                let texts = match &message.value {
                    Some(pattern) => pattern_texts(pattern, &mut vars),
                    None => {
                        errors.push(format!("{file}: {} に値が無い", message.id.name));
                        Vec::new()
                    }
                };
                if !message.attributes.is_empty() {
                    errors.push(format!("{file}: {} に属性がある", message.id.name));
                }
                out.push(Message {
                    lang: lang.to_owned(),
                    file: file.clone(),
                    key: message.id.name.to_owned(),
                    texts,
                    vars,
                });
            }
            ast::Entry::Junk { content } => errors.push(format!("{file}: 読めない所: {content}")),
            ast::Entry::Term(term) => {
                errors.push(format!("{file}: 項（-{}）は使わない", term.id.name))
            }
            _ => {}
        }
    }
    (out, errors)
}

/// 日英の .ftl の全部の文。
pub(crate) fn messages() -> Vec<Message> {
    let mut out = Vec::new();
    for lang in LANGS {
        for path in ftl_files(lang) {
            out.extend(parse(lang, &path).0);
        }
    }
    out
}

/// キー → その日英の文（ソースを読むテストが、キーで引いた所の文として見る）。
pub(crate) fn texts_by_key() -> BTreeMap<String, Vec<String>> {
    let mut table: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for message in messages() {
        table.entry(message.key).or_default().extend(message.texts);
    }
    table
}

// ───────── キーの見張り ─────────

/// 1 つの言語の全部の文（キー → 文）と、読めなかった所。
fn language(lang: &str) -> (BTreeMap<String, Message>, Vec<String>) {
    let mut table = BTreeMap::new();
    let mut errors = Vec::new();
    for path in ftl_files(lang) {
        let (messages, file_errors) = parse(lang, &path);
        errors.extend(file_errors);
        for message in messages {
            if let Some(before) = table.get(&message.key) {
                let before: &Message = before;
                errors.push(format!(
                    "{}: {} が {} と重なる",
                    message.file, message.key, before.file
                ));
            }
            table.insert(message.key.clone(), message);
        }
    }
    (table, errors)
}

#[test]
fn every_ftl_file_reads_without_errors() {
    for lang in LANGS {
        let (table, errors) = language(lang);
        assert!(errors.is_empty(), "{lang} の .ftl:\n{}", errors.join("\n"));
        // 読む所が空振りしていない
        assert!(ftl_files(lang).len() > 50, "{lang} の .ftl が少ない");
        assert!(table.len() > 3000, "{lang} の文 {}", table.len());
    }
}

#[test]
fn japanese_and_english_have_the_same_keys_and_arguments() {
    let names = |lang| -> Vec<String> {
        ftl_files(lang)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(names("ja"), names("en"), "日英の .ftl のファイルの組");
    let (ja, _) = language("ja");
    let (en, _) = language("en");
    let only = |a: &BTreeMap<String, Message>, b: &BTreeMap<String, Message>| -> Vec<String> {
        a.keys().filter(|k| !b.contains_key(*k)).cloned().collect()
    };
    assert!(
        only(&ja, &en).is_empty(),
        "日本語だけのキー: {:?}",
        only(&ja, &en)
    );
    assert!(
        only(&en, &ja).is_empty(),
        "英語だけのキー: {:?}",
        only(&en, &ja)
    );
    let mut found = Vec::new();
    for (key, j) in &ja {
        let e = &en[key];
        if j.file.rsplit('/').next() != e.file.rsplit('/').next() {
            found.push(format!("{key}: ファイルが違う（{} と {}）", j.file, e.file));
        }
        if j.vars != e.vars {
            found.push(format!("{key}: 引数が違う（{:?} と {:?}）", j.vars, e.vars));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// 本番のソースの `.f("キー", &[("名前", …), …])` が渡す引数の名前（キーの所から読む。入れ子の呼び出しの引数は数えない）。
fn passed_arguments(text: &str, line: usize, key: &str) -> Option<BTreeSet<String>> {
    let line_start: usize = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let at = text[line_start..].find(&format!("\"{key}\""))? + line_start + key.len() + 2;
    let rest = text[at..].trim_start().strip_prefix(',')?.trim_start();
    let rest = rest.strip_prefix("&[")?;
    let chars: Vec<char> = rest.chars().collect();
    let (mut depth, mut i) = (1i32, 0usize);
    let mut names = BTreeSet::new();
    let mut expect_name = false;
    while i < chars.len() && depth > 0 {
        let c = chars[i];
        match c {
            '"' => {
                let mut j = i + 1;
                let mut value = String::new();
                while j < chars.len() && chars[j] != '"' {
                    if chars[j] == '\\' {
                        j += 1;
                    }
                    value.push(chars[j]);
                    j += 1;
                }
                if expect_name {
                    names.insert(value);
                }
                expect_name = false;
                i = j + 1;
                continue;
            }
            '(' | '[' | '{' => {
                depth += 1;
                expect_name = c == '(' && depth == 2;
            }
            ')' | ']' | '}' => depth -= 1,
            c if c.is_whitespace() => {}
            _ => expect_name = false,
        }
        i += 1;
    }
    Some(names)
}

#[test]
fn the_keys_the_code_uses_are_in_both_languages_and_every_key_is_used() {
    let (ja, _) = language("ja");
    let (en, _) = language("en");
    let sources: BTreeMap<String, String> = crate::no_instruction_text::production_sources()
        .into_iter()
        .collect();
    let uses = crate::notice_rules::key_uses();
    // 読む所が空振りしていない
    assert!(uses.len() > 3000, "キーを引く所 {}", uses.len());
    let mut found = Vec::new();
    let mut used = BTreeSet::new();
    for (file, line, key) in &uses {
        used.insert(key.clone());
        let (Some(j), Some(_)) = (ja.get(key), en.get(key)) else {
            found.push(format!("{file}:{line}: キー {key} が日英の .ftl に無い"));
            continue;
        };
        let text = &sources[file];
        let call = text.split_inclusive('\n').nth(line - 1).unwrap_or_default();
        let is_f = call.contains(".f(") && !call.contains(&format!(".t(\"{key}\""));
        if is_f {
            match passed_arguments(text, *line, key) {
                Some(names) if names == j.vars => {}
                Some(names) => found.push(format!(
                    "{file}:{line}: {key} に渡す引数 {names:?} が文の引数 {:?} と違う",
                    j.vars
                )),
                None => found.push(format!("{file}:{line}: {key} の引数を読めない")),
            }
        } else if !j.vars.is_empty() {
            found.push(format!(
                "{file}:{line}: {key} は引数 {:?} のある文（`f` で引く）",
                j.vars
            ));
        }
    }
    // core の断りの理由（`reason-*`）は、理由の種類から引く（下の試験が core の理由と 1 対 1 かを見る）
    for key in ja.keys() {
        if !key.starts_with("reason-") && !used.contains(key) {
            found.push(format!("{}: {key} はどこからも引かれない", ja[key].file));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// core の `Reason::ja` の腕（理由の名前, 文）。
fn core_reasons() -> Vec<(String, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-core/src/reason.rs");
    let text = std::fs::read_to_string(path).unwrap();
    let start = text.find("pub const fn ja(self)").expect("Reason::ja");
    let body = &text[start..];
    let body = &body[..body.find("\n    }\n}\n").expect("Reason::ja の終わり")];
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("Reason::") else {
            continue;
        };
        let (name, value) = rest.split_once(" => ").expect("1 行の腕");
        let value = value
            .strip_suffix(',')
            .and_then(|v| v.strip_prefix('"'))
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or_else(|| panic!("文字列の腕: {line}"));
        assert!(!value.contains('\\'), "逃がしは読まない: {line}");
        out.push((name.to_owned(), value.to_owned()));
    }
    out
}

#[test]
fn the_japanese_reasons_are_the_core_texts() {
    let (ja, _) = language("ja");
    let reasons = core_reasons();
    assert!(reasons.len() > 500, "core の理由 {}", reasons.len());
    let mut keys = BTreeSet::new();
    let mut found = Vec::new();
    for (name, text) in &reasons {
        let key = yolu_app::lang::reason_key(name);
        if !keys.insert(key.clone()) {
            found.push(format!("{name}: キー {key} が重なる"));
        }
        match ja.get(&key) {
            Some(message) if message.texts == [text.clone()] => {}
            Some(message) => found.push(format!(
                "{key}: 日本語 {:?} が core の文 {text:?} と違う",
                message.texts
            )),
            None => found.push(format!("{name}: キー {key} が無い")),
        }
    }
    for key in ja.keys().filter(|k| k.starts_with("reason-")) {
        if !keys.contains(key) {
            found.push(format!("{key}: core に無い理由"));
        }
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
    // 名前の写し方
    assert_eq!(
        yolu_app::lang::reason_key("ColorBalanceRange"),
        "reason-color-balance-range"
    );
    assert_eq!(
        yolu_app::lang::reason_key("StencilTransformMissingFor2DDabs"),
        "reason-stencil-transform-missing-for2-d-dabs"
    );
    assert_eq!(
        yolu_app::lang::reason_key("UvComparisonResolution"),
        "reason-uv-comparison-resolution"
    );
}

/// 埋め込んだ文（`Lang::t`・`Lang::f`）が、ファイルの .ftl を読んだ文と同じ（build.rs が全部のファイルを埋めている）。
#[test]
fn the_embedded_texts_are_the_files() {
    use yolu_app::lang::{Arg, Lang};
    for (lang, name) in [(Lang::Ja, "ja"), (Lang::En, "en")] {
        let (table, _) = language(name);
        for (key, message) in table {
            let key: &'static str = Box::leak(key.into_boxed_str());
            if message.vars.is_empty() {
                assert_eq!([lang.t(key).to_owned()], message.texts.as_slice(), "{key}");
            } else if message.texts.len() == 1 {
                let markers: Vec<(&'static str, String)> = message
                    .vars
                    .iter()
                    .map(|v| {
                        (
                            &*Box::leak(v.clone().into_boxed_str()),
                            format!("{{ ${v} }}"),
                        )
                    })
                    .collect();
                let args: Vec<(&'static str, &dyn Arg)> =
                    markers.iter().map(|(n, v)| (*n, v as &dyn Arg)).collect();
                assert_eq!(lang.f(key, &args), message.texts[0], "{key}");
            }
        }
    }
    // 無いキーは、落ちずにキーの文字
    assert_eq!(Lang::Ja.t("no-such-key"), "no-such-key");
    assert_eq!(Lang::En.f("no-such-key", &[]), "no-such-key");
}
