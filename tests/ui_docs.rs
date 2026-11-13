#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "common/web_source.rs"]
mod web_source;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

fn code_spans(markdown: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut fence = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            spans.push(line.to_string());
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find('`') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('`') else { break };
            spans.push(after[..end].to_string());
            rest = &after[end + 1..];
        }
    }
    spans
}

fn guide_classes(markdown: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for span in code_spans(markdown) {
        let trimmed = span.trim();
        if let Some(selector) = trimmed.strip_prefix('.') {
            for part in selector.split('.') {
                let name = part
                    .split(|ch: char| !is_token_char(ch))
                    .next()
                    .unwrap_or_default();
                if name
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_lowercase())
                {
                    tokens.insert(name.to_string());
                }
            }
        }
        let mut rest = span.as_str();
        while let Some(start) = rest.find("class=\"") {
            let after = &rest[start + "class=\"".len()..];
            let Some(end) = after.find('"') else { break };
            for name in after[..end].split_whitespace() {
                if name.chars().all(is_token_char) {
                    tokens.insert(name.to_string());
                }
            }
            rest = &after[end..];
        }
    }
    tokens
}

fn guide_tokens(markdown: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for span in code_spans(markdown) {
        let mut rest = span.as_str();
        while let Some(start) = rest.find("--") {
            let after = &rest[start..];
            let name = after
                .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
                .next()
                .unwrap_or_default();
            if name.len() > 2 {
                tokens.insert(name.to_string());
            }
            rest = &after[name.len().max(1)..];
        }
    }
    tokens
}

fn contains_token(source: &str, token: &str) -> bool {
    let mut rest = source;
    while let Some(index) = rest.find(token) {
        let before = rest[..index].chars().next_back();
        let after = rest[index + token.len()..].chars().next();
        if !before.is_some_and(is_token_char) && !after.is_some_and(is_token_char) {
            return true;
        }
        rest = &rest[index + token.len()..];
    }
    false
}

#[test]
fn guide_class_names_exist_in_the_ui() {
    let guide = read("docs/ui-style-guide.md");
    let css = read("web/app.css");
    let script = web_source::source();
    let missing: Vec<String> = guide_classes(&guide)
        .into_iter()
        .filter(|token| !contains_token(&css, token) && !contains_token(&script, token))
        .collect();
    assert!(
        missing.is_empty(),
        "class names in the style guide are missing from web/app.css or web/js: {missing:?}"
    );
}

#[test]
fn guide_custom_properties_are_defined() {
    let guide = read("docs/ui-style-guide.md");
    let css = read("web/app.css");
    let missing: Vec<String> = guide_tokens(&guide)
        .into_iter()
        .filter(|token| !css.contains(&format!("{token}:")))
        .collect();
    assert!(
        missing.is_empty(),
        "custom properties in the style guide are not defined in web/app.css: {missing:?}"
    );
}

#[test]
fn agent_and_contributor_docs_link_the_guide() {
    for path in ["AGENTS.md", "CONTRIBUTING.md"] {
        let text = read(path);
        assert!(
            text.contains("docs/ui-style-guide.md"),
            "{path} must link the UI style guide"
        );
    }
}
