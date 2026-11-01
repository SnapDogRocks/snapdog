// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Translations of every text ETS shows for the product: object names, parameter names,
//! the group and block headings and the drop-down entries.
//!
//! The product is written in English (`DefaultLanguage="en-US"`). The texts of every other
//! language live in one file per language, `xtask/i18n/<code>.toml`, keyed by the English text:
//! adding a language is adding a file and listing it in [`SOURCES`]. This module adds a
//! `<Languages>` section with those translations to the generated XML. A text that is in no file
//! and matches no pattern stops the generation, so a new parameter cannot ship untranslated.
//! A translation that equals the English text is left out (ETS shows the base text when there is
//! no translation).
//!
//! Counted texts ("5 zones") and numbered headings ("Zone 3") are not in the `[texts]` table:
//! the file has the noun and the heading once, and the plural form of the language
//! ([`category`]) picks the word.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

/// The language files, in the order the languages appear in the product. The code is the ETS
/// language identifier.
const SOURCES: [(&str, &str); 7] = [
    ("de-DE", include_str!("../i18n/de-DE.toml")),
    ("nl-NL", include_str!("../i18n/nl-NL.toml")),
    ("es-ES", include_str!("../i18n/es-ES.toml")),
    ("fr-FR", include_str!("../i18n/fr-FR.toml")),
    ("it-IT", include_str!("../i18n/it-IT.toml")),
    ("pl-PL", include_str!("../i18n/pl-PL.toml")),
    ("pt-PT", include_str!("../i18n/pt-PT.toml")),
];

/// The forms of a counted noun, by plural category.
#[derive(Debug, serde::Deserialize)]
struct Counted {
    one: String,
    few: Option<String>,
    many: Option<String>,
    other: String,
}

#[derive(Debug, serde::Deserialize)]
struct ProductDb {
    prefix: String,
}

/// One language file.
#[derive(Debug, serde::Deserialize)]
struct Catalog {
    texts: HashMap<String, String>,
    counted: HashMap<String, Counted>,
    labels: HashMap<String, String>,
    product_db: ProductDb,
}

/// A parsed language file with the code of its language.
struct Language {
    code: &'static str,
    catalog: Catalog,
}

fn languages() -> &'static [Language] {
    static LANGUAGES: OnceLock<Vec<Language>> = OnceLock::new();
    LANGUAGES.get_or_init(|| {
        SOURCES
            .iter()
            .map(|(code, source)| Language {
                code,
                catalog: toml::from_str(source)
                    .unwrap_or_else(|e| panic!("xtask/i18n/{code}.toml is not valid: {e}")),
            })
            .collect()
    })
}

/// The plural category of a count in a language (CLDR, for whole numbers).
fn category(code: &str, n: u32) -> &'static str {
    match code {
        // Polish: 1 is one, 2 to 4 are few unless they end 12 to 14, everything else is many.
        "pl-PL" => {
            if n == 1 {
                "one"
            } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
                "few"
            } else {
                "many"
            }
        }
        _ => {
            if n == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

impl Counted {
    fn form(&self, category: &str) -> &str {
        match category {
            "one" => &self.one,
            "few" => self.few.as_deref().unwrap_or(&self.other),
            "many" => self.many.as_deref().unwrap_or(&self.other),
            _ => &self.other,
        }
    }
}

/// The English plural of a counted noun (every noun in the files takes an `s`).
fn english_plural(singular: &str) -> String {
    format!("{singular}s")
}

/// Capitalise the first character (nouns after a number are lower case, headings are not).
fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Translate a counted or numbered text into `lang`, `None` when `en` is not one.
fn pattern(lang: &Language, en: &str) -> Option<String> {
    let catalog = &lang.catalog;
    // "5 zones", "1 client", "30 minutes": a count and a noun.
    if let Some((count, noun)) = en.split_once(' ')
        && let Ok(n) = count.parse::<u32>()
    {
        for (singular, forms) in &catalog.counted {
            let plural = english_plural(singular);
            let fits = (n == 1 && noun == singular) || (n != 1 && noun == plural);
            if fits {
                return Some(format!("{n} {}", forms.form(category(lang.code, n))));
            }
        }
        // Units and bit depths read the same in every language.
        if matches!(noun, "Hz" | "Bit") {
            return Some(en.to_string());
        }
    }
    // "Station 7", "Zone 3", "Client 12: {{0: ...}}": a heading and a number, maybe a tail.
    for (label, word) in &catalog.labels {
        if let Some(rest) = en.strip_prefix(label.as_str())
            && let Some(rest) = rest.strip_prefix(' ')
        {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let tail = &rest[digits.len()..];
            if !digits.is_empty() && (tail.is_empty() || tail.starts_with(':')) {
                return Some(format!("{} {digits}{tail}", capitalised(word)));
            }
        }
    }
    // "Product DB v13 · App 0xFF01 · HW 1 · KNXnet/IP System B": the numbers stay.
    en.strip_prefix("Product DB ")
        .map(|rest| format!("{} {rest}", catalog.product_db.prefix))
}

/// The translation of one English text into `lang`, `None` when there is none.
fn translate(lang: &Language, en: &str) -> Option<String> {
    lang.catalog
        .texts
        .get(en)
        .cloned()
        .or_else(|| pattern(lang, en))
}

// ── XML ────────────────────────────────────────────────────────────

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The value of attribute `name` in the start tag `tag`, unescaped.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(unescape(&tag[start..end]))
}

/// One element whose texts get translated.
struct Item {
    id: String,
    kind: &'static str,
    /// `(attribute, English text)`.
    texts: Vec<(&'static str, String)>,
}

/// Element names with texts ETS shows, and the attributes to translate.
const TRANSLATED: &[(&str, &[&str])] = &[
    ("ComObject", &["Text", "FunctionText"]),
    ("Parameter", &["Text"]),
    ("ParameterSeparator", &["Text"]),
    ("ParameterBlock", &["Text"]),
    ("Enumeration", &["Text"]),
    ("Message", &["Text"]),
];

fn collect(xml: &str) -> (String, Vec<Item>) {
    let mut app_id = String::new();
    let mut items = Vec::new();
    for line in xml.lines() {
        let tag = line.trim_start();
        if !tag.starts_with('<') {
            continue;
        }
        if app_id.is_empty()
            && tag.starts_with("<ApplicationProgram ")
            && let Some(id) = attribute(tag, "Id")
        {
            app_id = id;
        }
        for (name, attributes) in TRANSLATED {
            let head = format!("<{name} ");
            if !tag.starts_with(&head) {
                continue;
            }
            let Some(id) = attribute(tag, "Id") else {
                continue;
            };
            let texts: Vec<(&'static str, String)> = attributes
                .iter()
                .filter_map(|a| attribute(tag, a).map(|t| (*a, t)))
                .collect();
            if !texts.is_empty() {
                items.push(Item {
                    id,
                    kind: name,
                    texts,
                });
            }
        }
    }
    (app_id, items)
}

/// The text that is looked up for an attribute: an object's function text is translated like its
/// text (both carry the object's English name).
fn lookup_key<'a>(item: &'a Item, attr: &str, en: &'a str) -> &'a str {
    if item.kind == "ComObject" && attr == "FunctionText" {
        item.texts
            .iter()
            .find(|(a, _)| *a == "Text")
            .map_or(en, |(_, t)| t.as_str())
    } else {
        en
    }
}

/// Add a `<Languages>` section with the translations of every text to the generated product XML.
///
/// # Panics
///
/// Panics with the list of texts that have no translation, so a new text cannot ship untranslated.
pub fn add_translations(xml: &str) -> String {
    let (app_id, items) = collect(xml);
    assert!(
        !app_id.is_empty(),
        "no ApplicationProgram in the generated XML"
    );

    let mut missing: Vec<String> = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    for lang in languages() {
        let mut body = String::new();
        for item in &items {
            let mut lines = String::new();
            for (attr, en) in &item.texts {
                let key = lookup_key(item, attr, en);
                let Some(text) = translate(lang, key) else {
                    let entry = format!("{}: {key}", lang.code);
                    if !missing.contains(&entry) {
                        missing.push(entry);
                    }
                    continue;
                };
                if text != *en {
                    let _ = writeln!(
                        lines,
                        "              <Translation AttributeName=\"{attr}\" Text=\"{}\" />",
                        escape(&text)
                    );
                }
            }
            if !lines.is_empty() {
                let _ = write!(
                    body,
                    "            <TranslationElement RefId=\"{}\">\n{lines}            </TranslationElement>\n",
                    item.id
                );
            }
        }
        blocks.push(format!(
            "        <Language Identifier=\"{}\">\n          <TranslationUnit RefId=\"{app_id}\">\n{body}          </TranslationUnit>\n        </Language>\n",
            lang.code
        ));
    }
    assert!(
        missing.is_empty(),
        "texts without a translation (add them to xtask/i18n/<language>.toml): {missing:?}"
    );

    let languages = format!("      <Languages>\n{}      </Languages>\n", blocks.concat());
    // Languages follows Hardware inside <Manufacturer>.
    let close = "    </Manufacturer>";
    let at = xml
        .rfind(close)
        .expect("no </Manufacturer> in the generated XML");
    let mut out = String::with_capacity(xml.len() + languages.len());
    out.push_str(&xml[..at]);
    out.push_str(&languages);
    out.push_str(&xml[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapdog::knx::group_objects::{CLIENT_GOS, GLOBAL_GOS, ZONE_GOS};

    fn lang(code: &str) -> &'static Language {
        languages()
            .iter()
            .find(|l| l.code == code)
            .unwrap_or_else(|| panic!("no language {code}"))
    }

    fn tr(code: &str, en: &str) -> String {
        translate(lang(code), en).unwrap_or_else(|| panic!("no {code} text for '{en}'"))
    }

    #[test]
    fn every_language_file_parses_and_has_the_same_texts() {
        let first = &languages()[0];
        let mut expected: Vec<&String> = first.catalog.texts.keys().collect();
        expected.sort();
        for other in languages() {
            let mut keys: Vec<&String> = other.catalog.texts.keys().collect();
            keys.sort();
            assert_eq!(
                keys, expected,
                "{} has other texts than {}",
                other.code, first.code
            );
            let mut nouns: Vec<&String> = other.catalog.counted.keys().collect();
            nouns.sort();
            let mut first_nouns: Vec<&String> = first.catalog.counted.keys().collect();
            first_nouns.sort();
            assert_eq!(nouns, first_nouns, "{} counted nouns", other.code);
            assert_eq!(
                other.catalog.labels.len(),
                first.catalog.labels.len(),
                "{} headings",
                other.code
            );
        }
    }

    #[test]
    fn every_object_has_a_translation_in_every_language() {
        for go in ZONE_GOS.iter().chain(CLIENT_GOS).chain(GLOBAL_GOS) {
            for l in languages() {
                assert!(
                    l.catalog.texts.contains_key(go.name),
                    "no {} text for the object '{}'",
                    l.code,
                    go.name
                );
            }
        }
    }

    /// The status objects follow the naming of the MDT product databases: "Status" first.
    #[test]
    fn german_status_objects_start_with_status() {
        for name in [
            "Control Status",
            "Volume Status",
            "Mute Status",
            "Shuffle Status",
            "Repeat Status",
            "Track Repeat Status",
            "Playlist Status",
            "Track Number Status",
            "Latency Status",
            "Zone Status",
        ] {
            assert!(
                tr("de-DE", name).starts_with("Status "),
                "'{name}' is '{}' in German",
                tr("de-DE", name)
            );
        }
    }

    #[test]
    fn counted_texts_follow_the_number() {
        assert_eq!(tr("de-DE", "1 zone"), "1 Zone");
        assert_eq!(tr("de-DE", "10 zones"), "10 Zonen");
        assert_eq!(tr("nl-NL", "1 station"), "1 zender");
        assert_eq!(tr("nl-NL", "7 stations"), "7 zenders");
        assert_eq!(tr("es-ES", "45 minutes"), "45 minutos");
        assert_eq!(tr("fr-FR", "96000 Hz"), "96000 Hz");
        assert_eq!(tr("it-IT", "3 clients"), "3 client");
        assert_eq!(tr("pt-PT", "2 stations"), "2 estações");
    }

    #[test]
    fn polish_counts_use_the_three_plural_forms() {
        assert_eq!(tr("pl-PL", "1 zone"), "1 strefa");
        assert_eq!(tr("pl-PL", "2 zones"), "2 strefy");
        assert_eq!(tr("pl-PL", "4 zones"), "4 strefy");
        assert_eq!(tr("pl-PL", "5 zones"), "5 stref");
        assert_eq!(tr("pl-PL", "12 zones"), "12 stref");
        assert_eq!(tr("pl-PL", "16 zones"), "16 stref");
        assert_eq!(tr("pl-PL", "22 clients"), "22 klienty");
        assert_eq!(tr("pl-PL", "64 clients"), "64 klienty");
        assert_eq!(tr("pl-PL", "30 clients"), "30 klientów");
        assert_eq!(tr("pl-PL", "50 stations"), "50 stacji");
        assert_eq!(tr("pl-PL", "1 API key"), "1 klucz API");
        assert_eq!(tr("pl-PL", "3 API keys"), "3 klucze API");
    }

    #[test]
    fn numbered_headings_follow_the_language() {
        assert_eq!(tr("de-DE", "Station 12"), "Station 12");
        assert_eq!(tr("nl-NL", "Station 12"), "Zender 12");
        assert_eq!(tr("es-ES", "Client 3: {{0: ...}}"), "Cliente 3: {{0: ...}}");
        assert_eq!(tr("fr-FR", "API key 4"), "Clé API 4");
        assert_eq!(tr("pl-PL", "Zone 3"), "Strefa 3");
        assert_eq!(tr("pt-PT", "API key 2"), "Chave API 2");
        assert_eq!(
            tr(
                "de-DE",
                "Product DB v13 · App 0xFF01 · HW 1 · KNXnet/IP System B"
            ),
            "Produkt-DB v13 · App 0xFF01 · HW 1 · KNXnet/IP System B"
        );
    }

    #[test]
    fn unknown_texts_have_no_translation() {
        assert!(translate(lang("de-DE"), "A text that does not exist").is_none());
    }

    #[test]
    fn the_generated_product_is_translated_completely() {
        let xml = crate::generate_xml();
        assert!(xml.contains("<Languages>"));
        assert!(
            xml.contains("DefaultLanguage=\"en-US\""),
            "the base language is English"
        );
        for (code, _) in SOURCES {
            assert!(xml.contains(&format!("<Language Identifier=\"{code}\">")));
        }
        assert!(
            !xml.contains("<Language Identifier=\"en-US\">"),
            "the base language is not a translation"
        );
        assert!(xml.contains("Status Wiedergabe"), "German status object");
        assert!(xml.contains("Playlist: pas"), "French object text");
        assert!(xml.contains("Volumen predeterminado"), "Spanish parameter");
        assert!(xml.contains("Numero di zone"), "Italian parameter");
        assert!(xml.contains("Liczba stref"), "Polish parameter");
        assert!(
            xml.contains("Palavra-passe do AirPlay"),
            "Portuguese parameter"
        );
    }
}
