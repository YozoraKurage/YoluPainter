//! 画面の文の .ftl（`locales/<言語>/<モジュール>.ftl`）を読む部品。ソースを読む決まりのテスト（`no_instruction_text`・
//! `no_developer_words`・`notice_rules`）は、ソースの直書きの文に加えて、ここで読んだ .ftl の文も見る。

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
