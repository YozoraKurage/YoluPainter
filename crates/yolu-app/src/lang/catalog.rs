//! 画面の文の表（`locales/<タグ>/<モジュール>.ftl` の Fluent の文）。build.rs が `locales/` の下の全部のフォルダの .ftl を `include_str!` で
//! 埋めるので、実行の時にファイルを読まない。その言語に無いキーは英語の文を使い、英語にも無いときだけキーの文字を出す。言語ごとに、初めて引いたときに 1 度だけ全部を読み、引数の無い文は整形した文字の表に置く（`Lang::t` は `&'static str` を返す）。
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

/// 埋めた言語のフォルダの数。
pub(super) const LOCALE_COUNT: usize = embedded::COUNT;

/// そのタグのフォルダを埋めたか（const。`Lang::ALL` を作る）。
pub(super) const fn has_locale(tag: &str) -> bool {
    let mut i = 0;
    while i < embedded::LOCALES.len() {
        if super::same(embedded::LOCALES[i].0, tag) {
            return true;
        }
        i += 1;
    }
    false
}

fn sources(lang: Lang) -> &'static [(&'static str, &'static str)] {
    embedded::LOCALES
        .iter()
        .find(|(tag, _)| *tag == lang.tag())
        .map_or(&[], |(_, files)| files)
}

fn locale(lang: Lang) -> unic_langid::LanguageIdentifier {
    lang.tag().parse().unwrap_or_default()
}

fn build(lang: Lang) -> Catalog {
    build_from(locale(lang), sources(lang))
}

fn build_from(locale: unic_langid::LanguageIdentifier, sources: &[(&str, &str)]) -> Catalog {
    let mut bundle = FluentBundle::new_concurrent(vec![locale]);
    bundle.set_use_isolating(false);
    let mut ids = Vec::new();
    for (_, source) in sources {
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
    static CATALOGS: [OnceLock<Catalog>; Lang::EVERY.len()] =
        [const { OnceLock::new() }; Lang::EVERY.len()];
    CATALOGS[lang as usize].get_or_init(|| build(lang))
}

/// 引数の無い文。その言語に無ければ英語、英語にも無ければキーの文字（テストが英語に無いキーを落とすので、本番では来ない）。
pub(super) fn text(lang: Lang, key: &'static str) -> &'static str {
    lookup_or_english(lang, key).unwrap_or(key)
}

/// その言語の文か、無ければ英語の文。
pub(super) fn lookup_or_english(lang: Lang, key: &str) -> Option<&'static str> {
    plain_in(catalog(lang), catalog(Lang::En), key)
}

/// 引数の無い文を、言語の表から、無ければ英語の表から。
fn plain_in<'a>(lang: &'a Catalog, english: &'a Catalog, key: &str) -> Option<&'a str> {
    lang.plain
        .get(key)
        .or_else(|| english.plain.get(key))
        .map(String::as_str)
}

/// 引数のある文を、言語の束から、無ければ英語の束から。
fn format_in(lang: &Catalog, english: &Catalog, key: &str, args: &FluentArgs) -> Option<String> {
    format(&lang.bundle, key, Some(args)).or_else(|| format(&english.bundle, key, Some(args)))
}

/// 引数のある文。その言語に無ければ英語、英語にも無ければキーの文字。
pub(super) fn format_with(lang: Lang, key: &str, args: &[(&str, &dyn Arg)]) -> String {
    let mut fluent = FluentArgs::with_capacity(args.len());
    for (name, value) in args {
        fluent.set(*name, value.value());
    }
    format_in(catalog(lang), catalog(Lang::En), key, &fluent).unwrap_or_else(|| key.to_owned())
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
    lookup_or_english(lang, &key).unwrap_or(match lang {
        Lang::Ja => reason.ja(),
        _ => "Unsupported value or operation",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 訳のフォルダの無い言語（表に行だけある言語）の文は、どのキーも英語の文。英語にも無いキーはキーの文字。
    #[test]
    fn a_language_without_texts_reads_english() {
        let missing: Vec<Lang> = Lang::EVERY
            .into_iter()
            .filter(|l| !l.is_available())
            .collect();
        if missing.is_empty() {
            eprintln!("飛ばす: 表の全部の言語に訳のフォルダがある");
        }
        for lang in missing {
            assert!(sources(lang).is_empty());
            assert_eq!(
                text(lang, "lang-cancelled"),
                text(Lang::En, "lang-cancelled")
            );
            assert_eq!(
                format_with(lang, "library-files-count", &[("count", &3)]),
                "3 files"
            );
            assert_eq!(text(lang, "no-such-key"), "no-such-key");
            assert_eq!(format_with(lang, "no-such-key", &[]), "no-such-key");
        }
    }

    /// 訳のある言語でも、その言語に無いキーは英語の文（訳の抜けをキーの文字で出さない）。テスト用の言語の表で確かめる。
    #[test]
    fn a_key_missing_from_a_translation_reads_english() {
        let test = build_from(
            "zh-TW".parse().unwrap(),
            &[("t", "only-here = 只在這裡\nwith-arg = 有 { $n } 個\n")],
        );
        let english = catalog(Lang::En);
        assert_eq!(plain_in(&test, english, "only-here"), Some("只在這裡"));
        assert_eq!(
            plain_in(&test, english, "lang-cancelled"),
            Some("Cancelled")
        );
        assert_eq!(plain_in(&test, english, "no-such-key"), None);
        let mut args = FluentArgs::new();
        args.set("n", FluentValue::from("2"));
        assert_eq!(
            format_in(&test, english, "with-arg", &args).as_deref(),
            Some("有 2 個")
        );
        let mut args = FluentArgs::new();
        args.set("count", FluentValue::from("3"));
        assert_eq!(
            format_in(&test, english, "library-files-count", &args).as_deref(),
            Some("3 files")
        );
    }
}
