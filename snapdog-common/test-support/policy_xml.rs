// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Test support for the D-Bus policy files, shared by `snapdog` and `snapdog-client`
//! through `#[path]` so that neither needs an XML crate as a dev-dependency (which would
//! change `Cargo.lock`). Not part of `snapdog-common`'s own build.
//!
//! The policy files are small and regular, so a strict check is enough: one root element,
//! tags balanced and nested, attributes `name="value"` in double quotes without
//! duplicates, no text between tags, and comments without `--`. dbus-daemon and
//! dbus-broker refuse a file that is not well-formed, and so do the units that load it.

/// `xml` without its comments. Panics on an unterminated comment, or one that contains
/// `--`, which XML does not allow.
pub fn strip_comments(xml: &str) -> String {
    let mut text = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        text.push_str(&rest[..start]);
        let body = &rest[start + 4..];
        let end = body.find("-->").expect("unterminated comment");
        assert!(
            !body[..end].contains("--"),
            "a comment contains `--`, which is not well-formed XML"
        );
        rest = &body[end + 3..];
    }
    text.push_str(rest);
    text
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

/// Check the attributes of one start tag, the part after the element name.
fn check_attributes(tag: &str, mut rest: &str) {
    let mut seen: Vec<&str> = Vec::new();
    loop {
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            return;
        }
        assert!(
            trimmed.len() < rest.len() || seen.is_empty(),
            "attributes must be separated by whitespace in <{tag}>"
        );
        let eq = trimmed
            .find('=')
            .unwrap_or_else(|| panic!("attribute without `=` in <{tag}>"));
        let name = trimmed[..eq].trim_end();
        assert!(is_name(name), "bad attribute name {name:?} in <{tag}>");
        assert!(
            !seen.contains(&name),
            "duplicate attribute {name:?} in <{tag}>"
        );
        seen.push(name);
        let value = trimmed[eq + 1..].trim_start();
        assert!(
            value.starts_with('"'),
            "attribute {name:?} in <{tag}> is not in double quotes"
        );
        let close = value[1..]
            .find('"')
            .unwrap_or_else(|| panic!("unterminated attribute value in <{tag}>"));
        let content = &value[1..=close];
        assert!(
            !content.contains(['<', '&']),
            "attribute {name:?} in <{tag}> contains `<` or `&`"
        );
        rest = &value[close + 2..];
    }
}

/// Panics unless `xml` is a well-formed document of the kind the policy files are.
pub fn assert_well_formed(xml: &str) {
    let text = strip_comments(xml);
    let mut rest = text.as_str();
    let mut open: Vec<&str> = Vec::new();
    let mut roots = 0;
    while let Some(lt) = rest.find('<') {
        assert!(
            rest[..lt].trim().is_empty(),
            "text between tags: {:?}",
            rest[..lt].trim()
        );
        let tail = &rest[lt + 1..];
        // The end of the tag is the first `>` outside quotes.
        let mut quote = false;
        let gt = tail
            .char_indices()
            .find(|&(_, c)| {
                if c == '"' {
                    quote = !quote;
                }
                c == '>' && !quote
            })
            .map(|(i, _)| i)
            .expect("unterminated tag");
        let tag = &tail[..gt];
        rest = &tail[gt + 1..];

        if let Some(decl) = tag.strip_prefix('!') {
            assert!(
                decl.starts_with("DOCTYPE ") && open.is_empty() && roots == 0,
                "only a DOCTYPE before the root element is allowed, got <!{decl}>"
            );
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            assert!(is_name(name), "bad closing tag </{name}>");
            let top = open
                .pop()
                .unwrap_or_else(|| panic!("</{name}> closes nothing"));
            assert_eq!(top, name, "</{name}> closes <{top}>");
            continue;
        }
        let (tag, self_closing) = tag
            .strip_suffix('/')
            .map_or((tag, false), |inner| (inner, true));
        let (name, attributes) = tag.split_at(tag.find(char::is_whitespace).unwrap_or(tag.len()));
        assert!(is_name(name), "bad element name {name:?}");
        check_attributes(name, attributes);
        if open.is_empty() {
            roots += 1;
            assert_eq!(roots, 1, "more than one root element");
        }
        if !self_closing {
            open.push(name);
        }
    }
    assert!(rest.trim().is_empty(), "text after the last tag");
    assert!(open.is_empty(), "unclosed elements: {open:?}");
    assert_eq!(roots, 1, "no root element");
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "<!DOCTYPE a PUBLIC \"x\"\n \"http://y/z.dtd\">\n<!-- c -->\n\
        <a>\n  <b k=\"v\" l=\"w\"/>\n  <c></c>\n</a>\n";

    #[test]
    fn accepts_a_well_formed_document() {
        assert_well_formed(GOOD);
        assert_eq!(strip_comments("a<!-- x -->b"), "ab");
    }

    #[test]
    fn rejects_what_is_not_well_formed() {
        for (why, bad) in [
            ("unclosed element", "<a><b></a>"),
            ("unclosed root", "<a>"),
            ("stray closing tag", "<a></a></b>"),
            ("mismatched tags", "<a></b>"),
            ("two roots", "<a/><b/>"),
            ("no root", "<!DOCTYPE a>"),
            ("text between tags", "<a>x</a>"),
            ("text after the root", "<a/>x"),
            ("unquoted attribute", "<a k=v/>"),
            ("single-quoted attribute", "<a k='v'/>"),
            ("unterminated value", "<a k=\"v/>"),
            ("attribute without value", "<a k/>"),
            ("duplicate attribute", "<a k=\"1\" k=\"2\"/>"),
            ("no space between attributes", "<a k=\"1\"l=\"2\"/>"),
            ("`<` in a value", "<a k=\"<\"/>"),
            ("`&` in a value", "<a k=\"&\"/>"),
            ("bad element name", "<1a/>"),
            ("unterminated tag", "<a"),
            ("DOCTYPE after the root", "<a/><!DOCTYPE a>"),
            ("`--` in a comment", "<!-- a -- b --><a/>"),
            ("unterminated comment", "<a/><!-- x"),
        ] {
            let outcome = std::panic::catch_unwind(|| assert_well_formed(bad));
            assert!(outcome.is_err(), "{why}: {bad:?} was accepted");
        }
    }
}
