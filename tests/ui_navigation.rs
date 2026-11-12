#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

const OVERLAY_PROBE: &str = r##"
const css = __CSS__;
function check(ok, message) {
  if (!ok) { console.error(message); process.exit(1); }
}
const shell = /\.shell \{[^}]*grid-template-columns:\s*(\d+)px/.exec(css);
check(shell, "the sidebar grid width is missing");
const side = Number(shell[1]);
check(side > 0, "the sidebar width is invalid");
const wide = /@media \(min-width: (\d+)px\) \{[\s\S]*?\.overlay\[data-edit-overlay\][^{]*\{([^}]*)\}/.exec(css);
check(wide, "the edit overlay still covers the sidebar on wide windows");
check(Number(wide[1]) >= 901, "the edit overlay offset applies below the desktop breakpoint");
const left = /left:\s*(\d+)px/.exec(wide[2]);
check(left, "the edit overlay has no left offset");
check(Number(left[1]) === side, `the edit overlay starts at ${left[1]}px but the sidebar is ${side}px wide`);
const schedule = /\.overlay#schedule-form[^{]*\{([^}]*)\}/.exec(css);
check(schedule, "the schedule overlay still covers the sidebar");
const scheduleLeft = /left:\s*(\d+)px/.exec(schedule[1]);
check(scheduleLeft && Number(scheduleLeft[1]) === side, "the schedule overlay starts away from the sidebar edge");
const backdrop = /\.overlay \{[^}]*inset:\s*0/.exec(css);
check(backdrop, "the normal overlay lost its full-page backdrop");
"##;

#[test]
fn edit_overlay_leaves_the_sidebar_clickable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    let probe = OVERLAY_PROBE.replace("__CSS__", &serde_json::to_string(&css).expect("css json"));
    run_node_probe("edit-overlay-sidebar", &probe);
}

fn run_node_probe(name: &str, probe: &str) {
    let path = std::env::temp_dir().join(format!("koreograf-{}-{}.js", name, std::process::id()));
    std::fs::write(&path, probe).expect("probe");
    let output = Command::new("node").arg(&path).output().expect("node");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "probe {name} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
