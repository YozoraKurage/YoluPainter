//! 知らせの決まり（ソースを読む）: 直前の操作の結果と理由（`AppState::message`）は `notice.rs` の `notify`（`info`・`refuse`・`warn`・`fail`）
//! だけが書く。種類（済んだ・断り・注意・失敗）と出どころを付けずに `message` を直に書くと、トーストの長さ・ログのウィンドウ・診断の記録から
//! 外れるので、本番のソースに直の書き込みが無いことを確かめる。断りの文（描いている間・読むだけのセット）は `lang/refusals.rs` だけが持つ。
use std::collections::BTreeMap;

/// 1 行が `message` を直に書く形か（代入・足し書き・文字列の書き換え・`&mut` で渡す・`mem::take` などで抜く）。
fn writes_message(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or(line);
    let Some(at) = code.find(".message") else {
        return false;
    };
    let after = &code[at + ".message".len()..];
    // `.message_kind()` などの別の名前は除く
    if after.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        return false;
    }
    let rest = after.trim_start();
    let assigns = (rest.starts_with('=') && !rest.starts_with("=="))
        || rest.starts_with("+=")
        || [
            ".push",
            ".clear()",
            ".insert",
            ".truncate",
            ".extend",
            ".drain",
            ".retain",
            ".clone_from",
        ]
        .iter()
        .any(|m| rest.starts_with(m));
    // `&mut self.state.message` の形: `.message` の前の道（名前と `.`）の前が `&mut`
    let path_start = code[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '.')
        .last()
        .map_or(at, |(i, _)| i);
    let borrowed = code[..path_start].trim_end().ends_with("&mut");
    assigns || borrowed
}

fn direct_writes() -> BTreeMap<String, Vec<String>> {
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (file, text) in crate::no_instruction_text::production_sources() {
        if file == "notice.rs" {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            if writes_message(line) {
                found.entry(file.clone()).or_default().push(format!(
                    "{file}:{}: {}",
                    n + 1,
                    line.trim()
                ));
            }
        }
    }
    found
}

#[test]
fn the_reader_sees_every_form_of_a_direct_write() {
    for line in [
        "app.message = text;",
        "self.message += &note;",
        "state.message.push_str(\" / \");",
        "app.message.clear();",
        "let m = &mut self.state.message;",
        "order = Some(std::mem::take(&mut self.message));",
        "message: &mut app.message,",
    ] {
        assert!(writes_message(line), "{line}");
    }
    for line in [
        "if app.message.is_empty() {",
        "let text = app.message.clone();",
        "assert_eq!(app.message, \"x\");",
        "app.message_begin();",
        "// app.message = text;",
        "let kind = self.message_kind();",
    ] {
        assert!(!writes_message(line), "{line}");
    }
}

/// 本番のソースは `message` を直に書かない（`notice.rs` のほか）。
#[test]
fn the_message_is_written_only_through_notify() {
    let found = direct_writes();
    let offenders: Vec<&String> = found.values().flatten().collect();
    assert!(
        offenders.is_empty(),
        "message を直に書いている（`AppState::info`・`refuse`・`warn`・`fail` で知らせる）:\n{}",
        offenders
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// 同じ断りの文は `lang/refusals.rs` の 1 か所だけが持つ（呼ぶ側に文字のまま書かない）。
#[test]
fn the_shared_refusals_live_in_one_place() {
    const SHARED: [&str; 3] = [
        "描いている間はできません",
        "Not while drawing",
        "テクスチャセットは読むだけです",
    ];
    let mut found = Vec::new();
    for (file, text) in crate::no_instruction_text::production_sources() {
        if file == "lang/refusals.rs" {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            for refusal in SHARED {
                if code.contains(refusal) {
                    found.push(format!("{file}:{}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "断りの文は lang::refusals の関数で:\n{}",
        found.join("\n")
    );
}

/// 1 つの字句が、文と文をコロンでつないだ断り・失敗の形か（「〜できません: 理由」「Cannot …: reason」）、何がを言わない
/// 「できません」「Cannot」だけの字句か。
fn stitched(literal: &str) -> bool {
    literal.contains("ません: ")
        || literal.contains("ません：")
        || literal == "できません"
        || literal == "Cannot"
        || (literal.starts_with("Cannot") && literal.contains(": "))
}

/// 1 行の文字列の字句（`"…"` の中身。エスケープした引用符は字句の中に残す）。
fn literals(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let body = &rest[start + 1..];
        let mut end = None;
        let mut escaped = false;
        for (i, c) in body.char_indices() {
            match c {
                '\\' if !escaped => escaped = true,
                '"' if !escaped => {
                    end = Some(i);
                    break;
                }
                _ => escaped = false,
            }
        }
        let Some(end) = end else { break };
        out.push(&body[..end]);
        rest = &body[end + 1..];
    }
    out
}

#[test]
fn the_reader_finds_stitched_refusals() {
    for line in [
        r#"format!("{}: {reason}", lang.pick("できません", "Cannot"))"#,
        r#"lang.pick(format!("ベイクできません: {e}"), format!("Cannot bake: {e}"))"#,
    ] {
        assert!(literals(line).into_iter().any(stitched), "{line}");
    }
    for line in [
        r#"lang.with_reason(lang.pick("ベイクできません", "Cannot bake"), e)"#,
        r#"lang.pick(format!("保存しました: {file}。"), format!("Saved: {file}."))"#,
    ] {
        assert!(!literals(line).into_iter().any(stitched), "{line}");
    }
}

/// 断り・失敗の文は「何が（なぜ）」の 1 つのふつうの文にする（`Lang::with_reason`）。「できません: 名前が使えません」のように文と文を
/// コロンでつないだり、何ができないかを言わない「できません」だけで始めたりしない。
#[test]
fn refusals_say_what_and_why_in_one_sentence() {
    let mut found = Vec::new();
    for (file, text) in crate::no_instruction_text::production_sources() {
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if literals(line).into_iter().any(stitched) {
                found.push(format!("{file}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "断り・失敗の文をコロンでつないでいる（Lang::with_reason で「何が（なぜ）」の 1 文に）:\n{}",
        found.join("\n")
    );
}

/// 本番のソースの知らせ（出どころを渡す `.fail(`・`.refuse(`・`.warn(`・`.info(`・`.notify(`）の文の引数（最後の引数。空白を詰める）。
fn notice_texts(code: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for head in [".fail(", ".refuse(", ".warn(", ".info(", ".notify("] {
        for (at, _) in code.match_indices(head) {
            let (mut depth, mut quoted, mut escaped) = (1, false, false);
            let mut args = vec![String::new()];
            for c in code[at + head.len()..].chars() {
                if quoted {
                    match c {
                        '\\' if !escaped => escaped = true,
                        '"' if !escaped => quoted = false,
                        _ => escaped = false,
                    }
                } else {
                    match c {
                        '"' => quoted = true,
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' => depth -= 1,
                        ',' if depth == 1 => {
                            args.push(String::new());
                            continue;
                        }
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                }
                if !c.is_whitespace() {
                    args.last_mut().unwrap().push(c);
                }
            }
            // 出どころ（`Source::…`）を渡す知らせだけ（引数の形が違う同名の関数は除く）
            if !args
                .iter()
                .take(2)
                .any(|a| a.starts_with("Source::") || a.starts_with("NoticeSource::"))
            {
                continue;
            }
            // 末尾のコンマの後ろの空の引数は、最後の引数ではない
            let last = args.into_iter().rev().find(|a| !a.is_empty());
            out.push((at, last.unwrap_or_default()));
        }
    }
    out
}

/// 失敗の知らせの文が、OS やライブラリの誤りの文そのまま（`e.to_string()`・`format!("{e}")`）か。
fn raw_error_text(text: &str) -> bool {
    ["e", "err", "error"].iter().any(|name| {
        text == format!("{name}.to_string()") || text == format!("format!(\"{{{name}}}\")")
    })
}

#[test]
fn the_reader_finds_raw_error_sentences() {
    for code in [
        "self.fail(Source::Psd, e.to_string());",
        "self.fail(\n    Source::Export,\n    format!(\"{e}\"),\n);",
        "app.refuse(Source::Fill, err.to_string());",
        "self.notify(Kind::Error, Source::Psd, e.to_string());",
    ] {
        assert!(
            notice_texts(code).iter().any(|(_, t)| raw_error_text(t)),
            "{code}"
        );
    }
    for code in [
        "self.fail(Source::Psd, lang.with_reason(lang.pick(\"a\", \"b\"), e.to_string()));",
        "self.fail(Source::Export, lang.thread_error(&e));",
        "self.info(Source::Save, format!(\"{file}, {e}\"));",
        "return Err(self.fail(&rs, doc, channel, e.to_string()));",
    ] {
        assert!(
            !notice_texts(code).iter().any(|(_, t)| raw_error_text(t)),
            "{code}"
        );
    }
}

/// 失敗の知らせは、何ができなかったかを言う文にする（`Lang::with_reason`）。OS やライブラリの誤りの文をそのまま知らせにしない
/// （日本語の画面に英語の OS の文が出て、何ができなかったかも分からない）。
#[test]
fn failures_do_not_show_a_raw_error_as_the_whole_sentence() {
    let mut found = Vec::new();
    for (file, text) in crate::no_instruction_text::production_sources() {
        for (at, arg) in notice_texts(&text) {
            if raw_error_text(&arg) {
                let line = text[..at].matches('\n').count() + 1;
                found.push(format!("{file}:{line}: {arg}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "誤りの文をそのまま知らせにしている（Lang::with_reason で「何が（なぜ）」の 1 文に）:\n{}",
        found.join("\n")
    );
}
