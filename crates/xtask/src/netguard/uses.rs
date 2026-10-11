//! `use` で入る名前（別名を含む）を読む。`use std::process::{Command as Cmd, Stdio};` → （`Cmd`, `std::process::Command`）・（`Stdio`, `std::process::Stdio`）。
use super::lex::*;

/// `use` が入れる名前と、その道。`self` は親の名前、`*` は道の最後に `*` を置いた 1 件にする。
pub(super) fn use_bindings(t: &[Token]) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if ident_at(t, i) == Some("use") && !is_punct_before(t, i, '.') {
            let (next, found) = tree(t, i + 1, &[]);
            out.extend(found);
            i = next.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

fn is_punct_before(t: &[Token], i: usize, p: char) -> bool {
    i > 0 && is_punct(t, i - 1, p)
}

fn is_sep(t: &[Token], i: usize) -> bool {
    matches!(t.get(i), Some(Token { tok: Tok::Sep, .. }))
}

/// `pos` から始まる 1 つの木（`a::b::{c, d as e}`）。戻りは（次の位置・名前と道の並び）。
fn tree(t: &[Token], mut pos: usize, prefix: &[String]) -> (usize, Vec<(String, Vec<String>)>) {
    let mut path: Vec<String> = prefix.to_vec();
    if is_sep(t, pos) {
        pos += 1;
    }
    loop {
        if is_punct(t, pos, '{') {
            // `{ a, b::c, d as e }`
            let mut out = Vec::new();
            pos += 1;
            while pos < t.len() && !is_punct(t, pos, '}') {
                let (next, found) = tree(t, pos, &path);
                out.extend(found);
                pos = next.max(pos + 1);
                if is_punct(t, pos, ',') {
                    pos += 1;
                }
            }
            return (pos + 1, out);
        }
        if is_punct(t, pos, '*') {
            path.push("*".into());
            return (pos + 1, vec![("*".into(), path)]);
        }
        let Some(segment) = ident_at(t, pos) else {
            return (pos, Vec::new());
        };
        pos += 1;
        path.push(segment.to_owned());
        if is_sep(t, pos) {
            pos += 1;
            continue;
        }
        // 木の終わり: `as 別名` があれば別名が名前
        let mut name = path.last().cloned().unwrap_or_default();
        if name == "self" {
            path.pop();
            name = path.last().cloned().unwrap_or_default();
        }
        if ident_at(t, pos) == Some("as") {
            if let Some(alias) = ident_at(t, pos + 1) {
                name = alias.to_owned();
                pos += 2;
            }
        }
        return (pos, vec![(name, path)]);
    }
}
