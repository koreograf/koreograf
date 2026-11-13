#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "common/web_source.rs"]
mod web_source;

use std::path::Path;
use std::process::Command;

fn web_file(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(root.join("web").join(name)).expect(name)
}

fn function<'a>(script: &'a str, name: &str) -> &'a str {
    let start = script
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("{name} is missing"));
    let rest = &script[start + 1..];
    let end = [rest.find("\n  function "), rest.find("\nfunction ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    &script[start..start + 1 + end]
}

#[test]
fn settings_are_one_form_with_tab_panels() {
    let script = web_source::source();
    let page = function(&script, "pageSettings");
    assert!(
        page.contains("<form class=\"card form\" id=\"settings-form\">"),
        "settings do not use one shared form"
    );
    for id in ["directory", "proxy", "jobs"] {
        assert!(
            page.contains(&format!("id=\"settings-panel-{id}\"")),
            "the {id} panel is missing"
        );
        assert!(
            page.contains(&format!("aria-labelledby=\"settings-tab-{id}\"")),
            "the {id} tab is not linked to its panel"
        );
    }
    let jobs = page.find("id=\"settings-panel-jobs\"").expect("jobs panel");
    let preflight = page.find("id=\"preflight-check\"").expect("preflight");
    let footer = page.find("id=\"settings-reset\"").expect("settings footer");
    assert!(
        jobs < preflight && preflight < footer,
        "preflight left the Jobs tab"
    );
    assert!(
        page.contains("ArrowRight")
            && page.contains("ArrowLeft")
            && page.contains("Home")
            && page.contains("End"),
        "the settings tablist does not handle arrow keys"
    );
    assert!(
        page.contains("history.replaceState") && page.contains("settingsTabAddress"),
        "the selected tab is not kept in the address"
    );

    let bar = function(&script, "settingsTabBar");
    assert!(
        bar.contains("role=\"tablist\""),
        "the settings tabs have no tablist"
    );
    assert!(
        bar.contains("data-settings-tab"),
        "the settings tabs have no marker"
    );
}

#[test]
fn settings_tab_address_round_trips() {
    let script = web_source::source();
    let tab = function(&script, "settingsTab");
    let address = function(&script, "settingsTabAddress");
    let probe = format!(
        r##"{tab}
{address}
let location = {{ hash: "#/settings" }};
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
check(settingsTab() === "directory", "the default settings tab is not Directory");
location.hash = "#/settings?tab=proxy";
check(settingsTab() === "proxy", "the proxy tab did not come from the address");
location.hash = "#/settings?tab=jobs";
check(settingsTab() === "jobs", "the jobs tab did not come from the address");
location.hash = "#/settings?tab=broken";
check(settingsTab() === "directory", "an unknown tab is not ignored");
check(settingsTabAddress("directory") === "#/settings", "Directory changed the address");
check(settingsTabAddress("proxy") === "#/settings?tab=proxy", "Proxy is missing from the address");
check(settingsTabAddress("jobs") === "#/settings?tab=jobs", "Jobs is missing from the address");
"##
    );
    let path =
        std::env::temp_dir().join(format!("koreograf-settings-tabs-{}.js", std::process::id()));
    std::fs::write(&path, probe).expect("probe");
    let output = Command::new("node").arg(&path).output().expect("node");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn settings_tab_labels_and_panel_style_are_present() {
    for language in ["de", "en"] {
        let text = web_file(&format!("i18n/{language}.json"));
        let value: serde_json::Value = serde_json::from_str(&text).expect("catalog json");
        let object = value.as_object().expect("catalog object");
        for key in ["directory", "proxy", "jobs"] {
            let entry = object
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            assert!(
                !entry.trim().is_empty(),
                "{key} is missing from {language}.json"
            );
        }
    }
    let css = web_file("app.css");
    assert!(
        css.contains(".settings-panel[hidden]"),
        "hidden settings panels still show"
    );
}
