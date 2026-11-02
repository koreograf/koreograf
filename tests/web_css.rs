use std::fs;
use std::path::PathBuf;

fn without_comments(css: &str) -> String {
    let mut result = String::new();
    let mut chars = css.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = '\0';
            for ch in chars.by_ref() {
                if previous == '*' && ch == '/' {
                    break;
                }
                previous = ch;
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn rule_selectors(css: &str) -> Vec<String> {
    let mut selectors = Vec::new();
    let mut prelude = String::new();
    for ch in without_comments(css).chars() {
        match ch {
            '{' => {
                let value = prelude.trim();
                if !value.is_empty() && !value.starts_with('@') {
                    selectors.push(value.to_string());
                }
                prelude.clear();
            }
            '}' => prelude.clear(),
            ';' if prelude.trim_start().starts_with('@') => prelude.clear(),
            _ => prelude.push(ch),
        }
    }
    selectors
}

fn is_boundary(ch: Option<char>) -> bool {
    ch.is_none_or(|ch| !(ch.is_alphanumeric() || ch == '-' || ch == '_'))
}

fn mentions_form_control(selector: &str) -> bool {
    ["input", "select", "textarea"].iter().any(|name| {
        let mut rest = selector;
        while let Some(index) = rest.find(name) {
            let before = rest[..index].chars().next_back();
            let after = rest[index + name.len()..].chars().next();
            if is_boundary(before) && is_boundary(after) {
                return true;
            }
            rest = &rest[index + name.len()..];
        }
        false
    })
}

#[test]
fn form_control_rules_do_not_start_with_id_selectors() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/app.css");
    let css = fs::read_to_string(path).expect("read web/app.css");
    let offenders: Vec<String> = rule_selectors(&css)
        .into_iter()
        .flat_map(|rule| {
            rule.split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|selector| selector.starts_with('#') && mentions_form_control(selector))
        .collect();
    assert!(
        offenders.is_empty(),
        "form control rules must not start with an id selector: {offenders:?}"
    );
}
