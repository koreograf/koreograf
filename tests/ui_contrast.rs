use std::fs;
use std::path::PathBuf;

const AA_NORMAL_TEXT: f64 = 4.5;

fn stylesheet() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/app.css");
    fs::read_to_string(path).expect("read web/app.css")
}

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

fn root_tokens(css: &str) -> Vec<(String, String)> {
    let css = without_comments(css);
    let start = css.find(":root").expect("web/app.css declares :root");
    let open = start + css[start..].find('{').expect(":root block") + 1;
    let close = open + css[open..].find('}').expect("closed :root block");
    css[open..close]
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn token<'a>(tokens: &'a [(String, String)], name: &str) -> &'a str {
    tokens
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or_else(|| panic!("missing token {name}"))
}

fn rules(css: &str) -> Vec<(String, String)> {
    let css = without_comments(css);
    let mut result = Vec::new();
    for segment in css.split('}') {
        let Some(open) = segment.rfind('{') else {
            continue;
        };
        let selector = segment[..open]
            .rsplit(['{', '}'])
            .next()
            .unwrap_or_default()
            .trim();
        if selector.is_empty() || selector.starts_with('@') {
            continue;
        }
        result.push((selector.to_string(), segment[open + 1..].to_string()));
    }
    result
}

fn rgb(hex: &str) -> [f64; 3] {
    let hex = hex.trim().trim_start_matches('#');
    assert_eq!(hex.len(), 6, "expected a six digit hex colour, got {hex}");
    let channel = |index: usize| {
        u8::from_str_radix(&hex[index..index + 2], 16).expect("hex channel") as f64 / 255.0
    };
    [channel(0), channel(2), channel(4)]
}

fn luminance(hex: &str) -> f64 {
    let [red, green, blue] = rgb(hex);
    let linear = |channel: f64| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
}

fn contrast(first: &str, second: &str) -> f64 {
    let first = luminance(first);
    let second = luminance(second);
    let (lighter, darker) = if first > second {
        (first, second)
    } else {
        (second, first)
    };
    (lighter + 0.05) / (darker + 0.05)
}

#[test]
fn muted_text_meets_wcag_aa_on_light_surfaces() {
    let css = stylesheet();
    let tokens = root_tokens(&css);
    let muted = token(&tokens, "--muted");
    let surfaces = [
        ("--paper", token(&tokens, "--paper")),
        ("--panel", token(&tokens, "--panel")),
        ("--line", token(&tokens, "--line")),
        ("--accent-soft", token(&tokens, "--accent-soft")),
        ("--good-bg", token(&tokens, "--good-bg")),
        ("--bad-bg", token(&tokens, "--bad-bg")),
        ("--info-bg", token(&tokens, "--info-bg")),
        ("--warn-bg", token(&tokens, "--warn-bg")),
        ("white", "#ffffff"),
        ("skipped pill", "#f5f5f4"),
    ];
    for (name, surface) in surfaces {
        let ratio = contrast(muted, surface);
        assert!(
            ratio >= AA_NORMAL_TEXT,
            "--muted {muted} on {name} {surface} is {ratio:.2}:1, need {AA_NORMAL_TEXT}:1"
        );
    }
}

#[test]
fn side_muted_text_meets_wcag_aa_on_dark_surfaces() {
    let css = stylesheet();
    let tokens = root_tokens(&css);
    let muted = token(&tokens, "--side-muted");
    let surfaces = [
        ("--side", token(&tokens, "--side")),
        ("sidebar control", "#292524"),
    ];
    for (name, surface) in surfaces {
        let ratio = contrast(muted, surface);
        assert!(
            ratio >= AA_NORMAL_TEXT,
            "--side-muted {muted} on {name} {surface} is {ratio:.2}:1, need {AA_NORMAL_TEXT}:1"
        );
    }
}

#[test]
fn muted_text_stays_quieter_than_primary_text() {
    let css = stylesheet();
    let tokens = root_tokens(&css);
    let paper = token(&tokens, "--paper");
    let side = token(&tokens, "--side");
    let muted = contrast(token(&tokens, "--muted"), paper);
    let ink = contrast(token(&tokens, "--ink"), paper);
    let side_muted = contrast(token(&tokens, "--side-muted"), side);
    let side_text = contrast(token(&tokens, "--side-text"), side);
    assert!(
        muted < ink,
        "--muted {muted:.2}:1 must stay below --ink {ink:.2}:1"
    );
    assert!(
        side_muted < side_text,
        "--side-muted {side_muted:.2}:1 must stay below --side-text {side_text:.2}:1"
    );
}

#[test]
fn dark_surfaces_do_not_use_the_light_muted_token() {
    let css = stylesheet();
    let offenders: Vec<String> = rules(&css)
        .into_iter()
        .filter(|(selector, declarations)| {
            (selector
                .split(',')
                .any(|part| part.trim().starts_with(".side"))
                || selector.contains("pre.vars")
                || selector.contains(".log"))
                && declarations.contains("var(--muted)")
        })
        .map(|(selector, _)| selector)
        .collect();
    assert!(
        offenders.is_empty(),
        "dark surfaces must use --side-muted, not --muted: {offenders:?}"
    );
}
