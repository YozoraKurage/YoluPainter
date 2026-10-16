//! 画面の文の表（`locales/<言語>/<モジュール>.ftl` の Fluent の文）。build.rs が .ftl を `include_str!` で埋めるので、実行の時にファイルを
//! 読まない。言語ごとに、初めて引いたときに 1 度だけ全部を読み、引数の無い文は整形した文字の表に置く（`Lang::t` は `&'static str` を返す）。
//! 引数のある文（`Lang::f`）は、引くたびに束で整形する。引数は Rust で書式を済ませた文字列で渡し、英語の複数形の選びに使う数だけを
//! [`Count`] で数として渡す。向きの印（FSI・PDI）は入れない。

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use fluent_syntax::ast;

use super::Lang;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/locales.rs"));
}

/// 1 つの言語の文: 束（引数のある文）と、引数の無い文を整形した表。
struct Catalog {
    bundle: FluentBundle<FluentResource>,
    plain: HashMap<String, String>,
}

/// 文の引数。`Display` の値は書式を済ませた文字列として、[`Count`] は数として渡す。
pub trait Arg {
    fn value(&self) -> FluentValue<'_>;
}

impl<T: Display + ?Sized> Arg for T {
    fn value(&self) -> FluentValue<'_> {
        FluentValue::String(self.to_string().into())
    }
}

/// 英語の複数形の選び（`{ $n -> [one] layer *[other] layers }`）に使う数。
#[derive(Clone, Copy, Debug)]
pub struct Count(pub usize);

impl Arg for Count {
    fn value(&self) -> FluentValue<'_> {
        FluentValue::from(self.0)
    }
}

fn sources(lang: Lang) -> &'static [(&'static str, &'static str)] {
    match lang {
        Lang::Ja => embedded::JA,
        Lang::En => embedded::EN,
    }
}

fn locale(lang: Lang) -> unic_langid::LanguageIdentifier {
    let tag = match lang {
        Lang::Ja => "ja",
        Lang::En => "en",
    };
    tag.parse().unwrap_or_default()
}

fn build(lang: Lang) -> Catalog {
    let mut bundle = FluentBundle::new_concurrent(vec![locale(lang)]);
    bundle.set_use_isolating(false);
    let mut ids = Vec::new();
    for (_, source) in sources(lang) {
        // 読めない所はテスト（`ftl_keys`）が落とす。本番は読めた所だけを使う
        let resource = FluentResource::try_new((*source).to_owned()).unwrap_or_else(|(r, _)| r);
        for entry in resource.entries() {
            if let ast::Entry::Message(message) = entry {
                ids.push(message.id.name.to_owned());
            }
        }
        let _ = bundle.add_resource(resource);
    }
    let mut plain = HashMap::with_capacity(ids.len());
    for id in ids {
        if let Some(text) = format(&bundle, &id, None) {
            plain.insert(id, text);
        }
    }
    Catalog { bundle, plain }
}

fn format(
    bundle: &FluentBundle<FluentResource>,
    key: &str,
    args: Option<&FluentArgs>,
) -> Option<String> {
    let pattern = bundle.get_message(key)?.value()?;
    let mut errors = Vec::new();
    Some(
        bundle
            .format_pattern(pattern, args, &mut errors)
            .into_owned(),
    )
}

fn catalog(lang: Lang) -> &'static Catalog {
    static JA: OnceLock<Catalog> = OnceLock::new();
    static EN: OnceLock<Catalog> = OnceLock::new();
    match lang {
        Lang::Ja => JA.get_or_init(|| build(Lang::Ja)),
        Lang::En => EN.get_or_init(|| build(Lang::En)),
    }
}

/// 引数の無い文。キーが無ければキーの文字（テストが無いキーを落とすので、本番では来ない）。
pub(super) fn text(lang: Lang, key: &'static str) -> &'static str {
    lookup(lang, key).unwrap_or(key)
}

fn lookup(lang: Lang, key: &str) -> Option<&'static str> {
    catalog(lang).plain.get(key).map(String::as_str)
}

/// 引数のある文。キーが無ければキーの文字。
pub(super) fn format_with(lang: Lang, key: &str, args: &[(&str, &dyn Arg)]) -> String {
    let mut fluent = FluentArgs::with_capacity(args.len());
    for (name, value) in args {
        fluent.set(*name, value.value());
    }
    format(&catalog(lang).bundle, key, Some(&fluent)).unwrap_or_else(|| key.to_owned())
}

/// core の断りの理由（`Reason`）の文のキー: `reason-` と、理由の名前（英語の CamelCase）を小文字と `-` にした物
/// （`ColorBalanceRange` → `reason-color-balance-range`、`For2DDabs` → `for2-d-dabs`）。
pub fn reason_key(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut key = String::from("reason-");
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            let before = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if before.is_ascii_lowercase()
                || before.is_ascii_digit()
                || (before.is_ascii_uppercase() && next_lower)
            {
                key.push('-');
            }
        }
        key.push(c.to_ascii_lowercase());
    }
    key
}

/// core の断りの理由の文。
pub(super) fn reason(lang: Lang, reason: yolu_core::Reason) -> &'static str {
    let key = reason_key(&format!("{reason:?}"));
    lookup(lang, &key).unwrap_or(match lang {
        Lang::Ja => reason.ja(),
        Lang::En => "Unsupported value or operation",
    })
}
