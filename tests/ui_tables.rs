use std::fs;
use std::path::PathBuf;

fn web_file(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web")
        .join(name);
    fs::read_to_string(path).expect("read web file")
}

fn media_block(css: &str, query: &str) -> String {
    let start = css.find(query).expect("find media query");
    let rest = &css[start..];
    let mut depth = 0;
    for (index, ch) in rest.char_indices() {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return rest[..index + 1].to_string();
            }
        }
    }
    rest.to_string()
}

#[test]
fn narrow_tables_use_card_rows() {
    let css = web_file("app.css");
    let block = media_block(&css, "@media (max-width: 640px)");
    assert!(block.contains(".table-wrap:not(.schedule-table):not(.inventory-hosts-table) .table"));
    assert!(block.contains("display: block;"));
    assert!(block.contains(
        ".table-wrap:not(.schedule-table):not(.inventory-hosts-table) .table thead { display: none; }"
    ));
    assert!(block.contains(
        ".table-wrap:not(.schedule-table):not(.inventory-hosts-table) .table .narrow-hide { display: none; }"
    ));
    assert!(block.contains("content: attr(data-label)"));
    assert!(css.contains(".schedule-table .table { min-width: 52rem; }"));
    assert!(css.contains(".inventory-hosts-table .table { min-width: 44rem; }"));
    assert!(css.contains(".table { width: 100%; border-collapse: collapse; }"));
}

#[test]
fn table_text_does_not_break_mid_word() {
    let css = web_file("app.css");
    let cells = css
        .lines()
        .find(|line| line.starts_with(".table th, .table td {"))
        .expect("find table cell rule");
    assert!(cells.contains("overflow-wrap: break-word;"));
    assert!(!cells.contains("overflow-wrap: anywhere;"));
    assert!(css.contains(".table .pill, .table time { white-space: nowrap; }"));
}

#[test]
fn list_tables_hide_low_priority_cells_on_narrow_windows() {
    let js = web_file("app.js");
    assert!(js.contains("function table(headers, rows)"));
    assert!(js.contains("class=\"table-wrap\""));
    assert!(js.contains("class=\"tool narrow-hide\""));
    assert!(js.contains("class=\"mono narrow-hide\""));
    assert!(js.contains("data-label=\"${h(\"created\")}\""));
    assert!(js.contains("data-label=\"${h(\"hosts\")}\""));
    assert!(js.matches("narrow-hide").count() >= 6);
}
