use std::path::Path;

use crate::common::*;

#[test]
fn backup_page_carries_the_state_flag_and_the_overview_shows_missing_paths() {
    let script = web_source();
    assert!(script.contains("id=\"backup-tofu-state\""));
    assert!(script.contains("include_opentofu_state"));
    assert!(script.contains("opentofu_state_can_contain_secrets"));
    assert!(script.contains("importNoticeHtml(data.import)"));
    let functions = format!(
        "{}\n{}",
        extract_braced_function(&script, "function backupDownloadUrl"),
        extract_braced_function(&script, "function importNoticeHtml")
    );
    let probe = functions
        + r##"
function esc(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[ch]));
}
function h(text) { return esc(text); }
if (backupDownloadUrl(true) !== "/api/v1/admin/backup?tofu_state=true") {
  console.error(backupDownloadUrl(true));
  process.exit(1);
}
if (backupDownloadUrl(false) !== "/api/v1/admin/backup") {
  console.error(backupDownloadUrl(false));
  process.exit(1);
}
const notice = importNoticeHtml({missing_paths: ["/home/user/projects/demo"]});
if (!notice.includes("imported_paths_need_a_mount_or_a_change") || !notice.includes("/home/user/projects/demo")) {
  console.error(notice);
  process.exit(1);
}
if (importNoticeHtml({missing_paths: []}) !== "") {
  console.error("empty import notice");
  process.exit(1);
}
"##;
    run_node_probe("backup-import-ui", &probe);
}

#[test]
fn job_list_hash_round_trips_filters_and_pages() {
    let script = web_source();
    let functions = format!(
        "{}\n{}",
        extract_braced_function(&script, "function jobsState"),
        extract_braced_function(&script, "function jobsHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/jobs?status=failed&page=2&size=50&q=Job%2005&tool=shell&trigger=schedule&template=tpl-a" }};
const view = jobsState();
if (view.page !== 2 || view.size !== 50 || view.q !== "Job 05" || view.status !== "failed" || view.tool !== "shell" || view.trigger !== "schedule" || view.template !== "tpl-a") {{
  console.error(JSON.stringify(view));
  process.exit(1);
}}
if (jobsHash(view) !== "#/jobs?page=2&size=50&q=Job+05&status=failed&tool=shell&trigger=schedule&template=tpl-a") {{
  console.error(jobsHash(view));
  process.exit(1);
}}
location.hash = "#/jobs";
const plain = jobsState();
if (plain.page !== 1 || plain.size !== 25 || jobsHash(plain) !== "#/jobs") {{
  console.error(jobsHash(plain));
  process.exit(1);
}}
"##
    );
    run_node_probe("job-list-hash", &probe);
}

#[test]
fn job_log_summary_search_and_first_failure() {
    let script = web_source();
    let functions = format!(
        "{}\n{}\n{}",
        extract_braced_function(&script, "function parseRecapLine"),
        extract_braced_function(&script, "function recapFailed"),
        extract_braced_function(&script, "function searchSegments")
    );
    let probe = format!(
        r##"{functions}
const fixture = [
  "PLAY RECAP *********************************************************************",
  "web-1                      : ok=3    changed=1    unreachable=0    failed=0    skipped=1    rescued=0    ignored=0",
  "web-2                      : ok=1    changed=0    unreachable=1    failed=1    skipped=0    rescued=0    ignored=0"
];
const rows = fixture.map(parseRecapLine).filter(Boolean);
if (rows.length !== 2) {{
  console.error(JSON.stringify(rows));
  process.exit(1);
}}
if (rows[0].host !== "web-1" || rows[0].ok !== 3 || rows[0].changed !== 1 || recapFailed(rows[0])) {{
  console.error(JSON.stringify(rows[0]));
  process.exit(1);
}}
if (rows[1].host !== "web-2" || rows[1].unreachable !== 1 || rows[1].failed !== 1 || !recapFailed(rows[1])) {{
  console.error(JSON.stringify(rows[1]));
  process.exit(1);
}}
const raw = '<script>alert("&lt;")</script> &lt; <b>x</b>';
for (const query of ["script", "&lt;", "<b>"]) {{
  const segments = searchSegments(raw, query);
  if (segments.map((item) => item.text).join("") !== raw) {{
    console.error(JSON.stringify(segments));
    process.exit(1);
  }}
  if (!segments.some((item) => item.hit)) {{
    console.error(query);
    process.exit(1);
  }}
}}
"##
    );
    run_node_probe("job-log-summary", &probe);
    let paint = extract_braced_function(&script, "function paintLogLine");
    assert!(paint.contains("createTextNode"));
    assert!(paint.contains("textContent"));
    assert!(!paint.contains("innerHTML"));
    assert!(script.contains("id=\"log-first-failure\""));
    assert!(script.contains("id=\"log-jump-end\""));
    assert!(script.contains("id=\"log-search\""));
    assert!(script.contains("id=\"log-wrap\""));
    assert!(script.contains("id=\"log-result\""));
    assert!(script.contains("state.firstFailure.scrollIntoView"));
    assert!(script.contains("state.atBottom"));
    let parameters = extract_braced_function(&script, "function jobParameters");
    assert!(parameters.contains("paramRow(\"credentials\""));
    assert!(!parameters.contains("paramRow(\"nav.access\""));
    assert!(parameters.contains("project_commit_full"));
}

#[test]
fn ui_reads_version_and_links_the_repository() {
    let script = web_source();
    let login = extract_braced_function(&script, "function renderLogin");
    let shell = extract_braced_function(&script, "function shell");
    let version_slot = extract_braced_function(&script, "function versionSlot");
    let load = extract_braced_function(&script, "async function loadVersion");
    for body in [&login, &shell] {
        assert!(
            body.contains("versionSlot()"),
            "a page does not render the build line"
        );
        assert!(
            body.contains("loadVersion()"),
            "a page does not request the version"
        );
    }
    assert!(load.contains("/api/v1/version"));
    assert!(load.contains("data.commit"));
    assert!(version_slot.contains("https://github.com/koreograf/koreograf"));
    assert!(version_slot.contains("target=\"_blank\""));
    assert!(version_slot.contains("rel=\"noopener noreferrer\""));
}

#[test]
fn auto_mode_names_a_missing_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let runner = std::fs::read_to_string(root.join("src/runner.rs")).expect("runner.rs");
    assert!(runner
        .contains("Mount the Docker socket, or set the runner mode to docker to fail instead."));
    let script = web_source();
    assert!(script.contains("id=\"environment-runtime-warning\""));
    assert!(script.contains("this_execution_environment_is_ignored_until_docker_or"));
    assert!(script.contains("no_container_runtime"));
    assert!(script.contains("api(\"GET\", \"/api/v1/runner\")"));
    assert_environment_warning_cases(&script);
    let en = std::fs::read_to_string(root.join("web/i18n/en.json")).expect("en");
    let de = std::fs::read_to_string(root.join("web/i18n/de.json")).expect("de");
    assert!(
        en.contains("Mount the Docker socket, or set the runner mode to docker to fail instead.")
    );
    assert!(en.contains("In automatic mode a set image is also ignored"));
    assert!(de.contains("Docker-Socket"));
    assert!(de.contains("Im automatischen Modus"));
    let docs = std::fs::read_to_string(root.join("docs/configuration.md")).expect("docs");
    assert!(docs.contains("mount the Docker socket"));
    assert!(docs.contains("stops after ten seconds"));
    assert!(docs.contains("GET /api/runner"));
}

#[test]
fn schedule_overlay_uses_the_shared_wide_form() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = web_source();
    assert!(script.contains("wide: true"));
    assert!(script.contains("bodyClass: \"schedule-body\""));
    assert!(script.contains("class=\"schedule-layout\""));
    assert!(script.contains("class=\"schedule-fields\""));
    assert!(script.contains("class=\"schedule-preview\""));
    assert!(script.contains("id=\"schedule-pin\""));
    assert!(script.contains("fields_marked_with_a_star_are_required"));
    assert!(script.contains("class=\"field required\""));
    assert!(script.contains("classList.toggle(\"required\", after)"));
    assert!(script.contains("classList.toggle(\"required\", onDate)"));
    assert!(script.contains("!form.name.value.trim()"));
    assert!(!script.contains("select class=\"fit\" name=\"timezone\""));
    assert!(!script.contains("select class=\"fit\" name=\"frequency\""));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(css.contains(".schedule-layout { display: grid; grid-template-columns: minmax(0, 1fr) minmax(15rem, 19rem);"));
    assert!(css.contains(".schedule-fields { overflow-y: auto;"));
    assert!(css.contains(".schedule-preview { overflow-y: auto;"));
    assert!(css.contains("@media (max-width: 900px)"));
    assert!(css.contains(".schedule-layout { grid-template-columns: 1fr; height: auto; }"));
    assert!(css.contains(".dialog-footer { display: block; }"));
    assert!(css.contains(".schedule-pin { display: block;"));
    assert!(
        css.contains(".overlay-card.dialog input:not([type=\"checkbox\"]):not([type=\"radio\"])")
    );
    assert!(!css.contains("#schedule-form input"));
}

#[test]
fn schedule_overlay_address_opens_and_closes_the_dialog() {
    let script = web_source();
    assert!(script.contains("if (request.edit)"));
    assert!(script.contains(
        "openScheduleForm(item.target_kind || kind, item.target_id || target, item, draw);"
    ));
    assert!(script.contains("openScheduleForm(openKind, openTarget, null, draw);"));
    assert!(!script.contains("fromHash"));
    assert!(script.contains("rememberScheduleHash({ edit: item.id })"));
    assert!(script.contains("rememberScheduleHash({ new: \"1\" })"));
    assert!(script.contains("onClose: () => {"));
    assert!(script.contains("document.querySelector(\"#schedule-form\")?.remove();"));
    let functions = format!(
        "{}\n{}\n{}",
        extract_braced_function(&script, "function scheduleQuery"),
        extract_braced_function(&script, "function scheduleAddress"),
        extract_braced_function(&script, "function scheduleNewTarget")
    );
    let probe = format!(
        r##"{functions}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const edit = scheduleQuery("#/schedules?edit=s1");
check(edit.edit === "s1" && edit.create === null, "edit was not read");
const create = scheduleQuery("#/schedules?new=1");
check(create.create === "1", "new was not read");
const template = scheduleNewTarget({{ create: "template:t1", kind: "", target: "" }});
check(template && template.kind === "template" && template.target === "t1", "template target was not read");
const workflow = scheduleNewTarget({{ create: "1", kind: "workflow", target: "w1" }});
check(workflow && workflow.kind === "workflow" && workflow.target === "w1", "workflow target was not read");
check(scheduleAddress({{ edit: "s1" }}, "#/schedules?page=2") === "#/schedules?page=2&edit=s1", "edit address was not built");
check(scheduleAddress({{ new: "1" }}, "#/templates/t1") === "#/templates/t1?new=1", "new address was not built");
check(scheduleAddress({{}}, "#/schedules?edit=s1&page=2") === "#/schedules?page=2", "address was not cleared");
"##
    );
    run_node_probe("schedule-address", &probe);
}

#[test]
fn schedule_overlay_guards_unsaved_edits() {
    let script = web_source();
    assert!(script.contains("dirty: () => dirty"));
    let form = extract_braced_function(&script, "function openScheduleForm");
    assert!(form.contains("appState.setLeaveGuard(() => dialog.dirty());"));
    let submit = form
        .find("onSubmit: async")
        .expect("schedule dialog submit");
    let close = form
        .find("onClose: () => {")
        .expect("schedule dialog close");
    assert!(submit < close);
    assert!(form[submit..close].contains("appState.setLeaveGuard(null);"));
    assert!(form[close..].contains("appState.setLeaveGuard(null);"));
}

#[test]
fn edit_overlays_share_one_dialog_helper() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = web_source();
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("openDialog({"));
    assert!(helper.contains("fields_marked_with_a_star_are_required"));
    assert!(helper.contains("role=\"tablist\""));
    assert!(helper.contains("data-dialog-tab="));
    assert!(helper.contains("data-dialog-panel="));
    assert!(helper.contains("ArrowRight"));
    assert!(helper.contains("ArrowLeft"));
    assert!(helper.contains("Home"));
    assert!(helper.contains("End"));
    assert!(helper.contains("has-error"));
    assert!(helper.contains("dialog.form.addEventListener(\"invalid\""));
    assert!(helper.contains("choose(panel.dataset.dialogPanel, false)"));
    assert!(helper.contains("appState.setLeaveGuard(() => dialog.dirty());"));
    assert!(helper.contains("clearEditHash();"));
    let dialog = extract_function_with_destructured_params(&script, "function openDialog");
    assert!(dialog.contains("if (dirty && !(await confirmDialog({ title: t(\"discard_unsaved_changes\"), confirmLabel: t(\"ok\") }))) return false;"));
    assert!(dialog.contains("handle(error);"));
    assert!(dialog.contains("if (onError) onError(error);"));
    let handle = extract_braced_function(&script, "function handle");
    assert!(handle.contains("const card = topmostCard();"));
    assert!(handle.contains("card.querySelector(\".dialog-body\")"));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(css.contains(".dialog-tabs { display: flex;"));
    assert!(css.contains(".dialog-tab[aria-selected=\"true\"]"));
    assert!(css.contains(".dialog-tab.has-error::after"));
    assert!(css.contains(".dialog-panel[hidden] { display: none; }"));
    assert!(css.contains(
        ".overlay-card, .overlay-card.dialog, .overlay-card.dialog.tabbed { width: 100%;"
    ));
}

#[test]
fn edit_overlay_addresses_round_trip_and_old_pages_redirect() {
    let script = web_source();
    for redirect in [
        "registerRedirect(\"#/credentials/new\", \"#/credentials?new=1\")",
        "registerRedirect(\"#/credentials/:id\", ({ id }) => `#/credentials?edit=${encodeURIComponent(id)}`)",
        "registerRedirect(\"#/organizations/new\", \"#/organizations?new=1\")",
        "registerRedirect(\"#/organizations/:id\", ({ id }) => `#/organizations?edit=${encodeURIComponent(id)}`)",
        "registerRedirect(\"#/team/new\", \"#/team?new=1\")",
        "registerRedirect(\"#/team/:id\", ({ id }) => `#/team?edit=${encodeURIComponent(id)}`)",
        "registerRedirect(\"#/users/new\", \"#/users?new=1\")",
        "registerRedirect(\"#/users/:id\", ({ id }) => `#/users?edit=${encodeURIComponent(id)}`)",
        "registerRedirect(\"#/projects/new\", \"#/projects?new=1\")",
        "registerRedirect(\"#/inventories/new\", \"#/inventories?new=1\")",
        "registerRedirect(\"#/hooks/new\", \"#/hooks?new=1\")",
        "registerRedirect(\"#/hooks/:id\", ({ id }) => `#/hooks?edit=${encodeURIComponent(id)}`)",
        "registerRedirect(\"#/environments/new\", \"#/runtime?new=1\")",
        "registerRedirect(\"#/team/members/new\", \"#/team?new=member\")",
    ] {
        assert!(script.contains(redirect), "missing redirect {redirect}");
    }
    for old in [
        "href=\"#/credentials/${",
        "href=\"#/organizations/${",
        "href=\"#/users/${",
        "addLink(\"#/credentials/new\"",
        "addLink(\"#/organizations/new\"",
        "addLink(\"#/team/new\"",
        "addLink(\"#/users/new\"",
        "addLink(\"#/projects/new\"",
        "addLink(\"#/inventories/new\"",
        "addLink(\"#/hooks/new\"",
        "addLink(\"#/environments/new\"",
        "addLink(\"#/team/members/new\"",
        "href=\"#/projects/new\"",
        "href=\"#/inventories/new\"",
        "href=\"#/hooks/new\"",
        "href=\"#/environments/new\"",
        "id=\"hook-form\"",
        "id=\"env-form\"",
        "id=\"member-form\"",
    ] {
        assert!(
            !script.contains(old),
            "old page link {old} is still in the UI"
        );
    }
    let functions = format!(
        "{}\n{}\n{}\n{}",
        extract_braced_function(&script, "function editQuery"),
        extract_braced_function(&script, "function editAddress"),
        extract_braced_function(&script, "function rememberEditHash"),
        extract_braced_function(&script, "function clearEditHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/credentials" }};
const appState = {{ shownHash: null, setShownHash(value) {{ return (this.shownHash = value); }} }};
let pushed = 0;
let replaced = 0;
const history = {{
  pushState(_state, _title, url) {{ pushed += 1; location.hash = url; }},
  replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }},
}};
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const edit = editQuery("#/credentials?edit=c1&tab=memberships");
check(edit.edit === "c1" && edit.tab === "memberships" && edit.create === null, "edit and tab were not read");
const create = editQuery("#/organizations?new=1");
check(create.create === "1" && create.edit === "" && create.tab === "", "new was not read");
const project = editQuery("#/projects?edit=p1&tab=source");
check(project.edit === "p1" && project.tab === "source", "project edit and tab were not read");
const hook = editQuery("#/hooks?edit=h1");
check(hook.edit === "h1" && hook.create === null, "hook edit was not read");
const member = editQuery("#/team?new=member");
check(member.create === "member" && member.edit === "", "member create was not read");
check(editAddress({{ create: "1" }}, "#/credentials") === "#/credentials?new=1", "new address was not built");
check(editAddress({{ create: "1" }}, "#/runtime") === "#/runtime?new=1", "runtime new address was not built");
check(editAddress({{ edit: "e1" }}, "#/runtime") === "#/runtime?edit=e1", "runtime edit address was not built");
check(editAddress({{ create: "member" }}, "#/team") === "#/team?new=member", "member address was not built");
check(editAddress({{ create: "1" }}, "#/inventories") === "#/inventories?new=1", "inventory new address was not built");
check(editAddress({{ edit: "p1", tab: "source" }}, "#/projects") === "#/projects?edit=p1&tab=source", "project address was not built");
check(editAddress({{ edit: "u1", tab: "account" }}, "#/users?page=2") === "#/users?page=2&edit=u1&tab=account", "edit address was not built");
check(editAddress({{ edit: "t1" }}, "#/team?edit=t1&tab=general") === "#/team?edit=t1", "old tab was not cleared");
check(editAddress({{}}, "#/team?edit=t1&tab=general") === "#/team", "address was not cleared");
rememberEditHash({{ create: "1" }});
check(pushed === 1 && replaced === 0 && location.hash === "#/credentials?new=1" && appState.shownHash === location.hash, "open did not push the address");
rememberEditHash({{ edit: "c1", tab: "memberships" }}, false);
check(pushed === 1 && replaced === 1 && location.hash === "#/credentials?edit=c1&tab=memberships", "tab did not replace the address");
clearEditHash();
check(replaced === 2 && location.hash === "#/credentials" && appState.shownHash === "#/credentials", "close did not clear the address");
"##
    );
    run_node_probe("edit-address", &probe);
}

#[test]
fn credentials_and_admin_objects_use_the_edit_overlay() {
    let script = web_source();
    let credentials = extract_braced_function(&script, "async function pageCredentials");
    assert!(credentials.contains("openCredentialDialog(item, !can(\"owner\"), request.tab)"));
    assert!(credentials.contains("openCredentialDialog(null)"));
    assert!(credentials.contains("#/credentials?edit="));
    assert!(credentials.contains("#/credentials?new=1"));
    assert!(credentials.contains("aria-label=\"${h(\"search_credentials\")}\""));
    assert!(credentials.contains(
        "confirmDialog({ title: t(\"delete\"), body: t(\"delete_this_credential\", { name: button.dataset.credentialName }), confirmLabel: t(\"delete\"), danger: true })"
    ));
    let credential = extract_braced_function(&script, "function openCredentialDialog");
    assert!(credential.contains("openEditDialog({"));
    assert!(credential.contains("api(\"POST\", \"/api/v1/credentials\""));
    assert!(credential.contains("api(\"PUT\", `/api/v1/credentials/"));
    assert!(credential.contains("api(\"DELETE\", `/api/v1/credentials/"));
    assert!(credential.contains("data-credential-delete"));
    assert!(credential.contains("hint: \"fields_marked_with_a_star_are_required\""));
    assert!(credential.contains("hint: \"\","));
    assert!(credential.contains("confirmDialog({ title: t(\"delete\"), body: t(\"delete_this_credential\", { name: item.name }), confirmLabel: t(\"delete\"), danger: true })"));
    assert!(credential.contains("markSaved();"));
    assert!(credential.contains("close();"));
    assert!(credential.contains("await route();"));
    let organizations = extract_braced_function(&script, "async function pageOrganizations");
    assert!(
        organizations.contains("openOrganizationDialog(item, !appState.me.is_admin, request.tab)")
    );
    assert!(organizations.contains("openOrganizationDialog(null)"));
    let organization = extract_braced_function(&script, "function openOrganizationDialog");
    assert!(organization.contains("openEditDialog({"));
    assert!(organization.contains("api(\"POST\", \"/api/v1/organizations\""));
    assert!(organization.contains("api(\"PUT\", `/api/v1/organizations/"));
    assert!(organization.contains("{ id: \"general\", label: t(\"general\")"));
    assert!(organization.contains("{ id: \"teams\", label: t(\"teams\")"));
    assert!(organization.contains("onTab: editing ? (tab) => rememberEditHash("));
    let team = extract_braced_function(&script, "function openTeamDialog");
    assert!(team.contains("openEditDialog({"));
    assert!(team.contains("api(\"POST\", \"/api/v1/teams\""));
    assert!(team.contains("api(\"PUT\", `/api/v1/teams/"));
    assert!(!script.contains("id=\"team-form\""));
    let users = extract_braced_function(&script, "async function pageUsers");
    assert!(users.contains("openUserDialog(item, teams, request.tab)"));
    assert!(users.contains("openUserCreateDialog()"));
    let user = extract_braced_function(&script, "function openUserDialog");
    assert!(user.contains("openEditDialog({"));
    assert!(user.contains("{ id: \"account\", label: t(\"account\")"));
    assert!(user.contains("{ id: \"memberships\", label: t(\"memberships\")"));
    assert!(user.contains("api(\"PUT\", `/api/v1/users/"));
    assert!(user.contains("api(\"DELETE\", `/api/v1/users/"));
    assert!(user.contains("onTab: (tab) => rememberEditHash("));
    let create = extract_braced_function(&script, "function openUserCreateDialog");
    assert!(create.contains("api(\"POST\", \"/api/v1/users\""));
    assert!(create.contains("class=\"field required\""));
    assert!(!script.contains("id=\"create-user\""));
    for function in [&credential, &organization, &team, &create] {
        assert!(function.contains("markSaved();"));
        assert!(function.contains("await route();"));
    }
}

#[test]
fn projects_and_inventories_use_the_edit_overlay() {
    let script = web_source();
    let projects = extract_braced_function(&script, "async function pageProjects");
    assert!(projects.contains("openProjectDialog(item, request.tab)"));
    assert!(projects.contains("openProjectDialog(null)"));
    assert!(projects.contains("#/projects?edit="));
    assert!(projects.contains("#/projects?new=1"));
    assert!(projects.contains("clearEditHash();"));
    let project = extract_braced_function(&script, "async function openProjectDialog");
    assert!(project.contains("openEditDialog({"));
    assert!(project.contains("api(\"POST\", \"/api/v1/projects\""));
    assert!(project.contains("api(\"PUT\", `/api/v1/projects/"));
    assert!(project.contains("{ id: \"general\", label: t(\"general\")"));
    assert!(project.contains("{ id: \"source\", label: t(\"source_2\")"));
    assert!(project.contains("{ id: \"assignment\", label: t(\"assignment\")"));
    assert!(project.contains("onTab: editing ? (tab) => rememberEditHash("));
    assert!(project.contains("saved.sync"));
    assert!(project.contains("markSaved();"));
    assert!(project.contains("close();"));
    assert!(project.contains("await route();"));
    let detail = extract_braced_function(&script, "async function pageProject(id)");
    assert!(detail.contains("#/projects?edit="));
    assert!(detail.contains("delete-project"));
    assert!(!detail.contains("id=\"project-form\""));
    let inventories = extract_braced_function(&script, "async function pageInventories");
    assert!(inventories.contains("openInventoryDialog(item)"));
    assert!(inventories.contains("openInventoryDialog(null)"));
    assert!(inventories.contains("#/inventories?edit="));
    assert!(inventories.contains("#/inventories?new=1"));
    let inventory = extract_braced_function(&script, "async function openInventoryDialog");
    assert!(inventory.contains("openEditDialog({"));
    assert!(inventory.contains("api(\"POST\", \"/api/v1/inventories\""));
    assert!(inventory.contains("api(\"PUT\", `/api/v1/inventories/"));
    assert!(inventory.contains("bindSourcePicks(projects, credentials)"));
    assert!(inventory.contains("markSaved();"));
    assert!(inventory.contains("close();"));
    assert!(inventory.contains("await route();"));
    assert!(!inventory.contains("onTab:"));
    let inventory_detail = extract_braced_function(&script, "async function pageInventory(id)");
    assert!(inventory_detail.contains("#/inventories?edit="));
    assert!(!inventory_detail.contains("id=\"inventory-name-form\""));
    assert!(!script.contains("id=\"project-form\""));
    assert!(!script.contains("id=\"inventory-create\""));
    assert!(!script.contains("id=\"inventory-name-form\""));
    let close = extract_braced_function(&script, "function closeLive");
    assert!(close.contains("[data-edit-overlay]"));
}

#[test]
fn hooks_environments_and_team_members_use_the_edit_overlay() {
    let script = web_source();
    let hooks = extract_braced_function(&script, "async function pageHooks");
    assert!(hooks.contains("openHookDialog(item)"));
    assert!(hooks.contains("openHookDialog(null)"));
    assert!(hooks.contains("#/hooks?edit="));
    assert!(hooks.contains("#/hooks?new=1"));
    assert!(hooks.contains("clearEditHash();"));
    let hook = extract_braced_function(&script, "async function openHookDialog");
    assert!(hook.contains("openEditDialog({"));
    assert!(hook.contains("api(\"POST\", \"/api/v1/hooks\""));
    assert!(hook.contains("api(\"PUT\", `/api/v1/hooks/"));
    assert!(hook.contains("jsonObject(form.extra_vars.value"));
    assert!(hook.contains("markSaved();"));
    assert!(hook.contains("close();"));
    assert!(hook.contains("await route();"));
    assert!(!hook.contains("handle(error)"));
    let runtime = extract_braced_function(&script, "async function pageRuntime");
    assert!(runtime.contains("openEnvironmentDialog(item)"));
    assert!(runtime.contains("openEnvironmentDialog(null)"));
    assert!(runtime.contains("#/runtime?edit="));
    assert!(runtime.contains("#/runtime?new=1"));
    assert!(runtime.contains("clearEditHash();"));
    let environment = extract_braced_function(&script, "function openEnvironmentDialog");
    assert!(environment.contains("openEditDialog({"));
    assert!(environment.contains("api(\"POST\", \"/api/v1/environments\""));
    assert!(environment.contains("api(\"PUT\", `/api/v1/environments/"));
    assert!(environment.contains("environmentImagePlaceholder(dialog.form.tool.value)"));
    assert!(environment.contains("markSaved();"));
    assert!(environment.contains("close();"));
    assert!(environment.contains("await route();"));
    assert!(!environment.contains("handle(error)"));
    let detail = extract_braced_function(&script, "async function pageEnvironment");
    assert!(detail.contains("#/runtime?edit="));
    assert!(detail.contains("delete-env"));
    assert!(!detail.contains("environmentForm"));
    let team = extract_braced_function(&script, "async function pageTeam");
    assert!(team.contains("openMemberDialog()"));
    assert!(team.contains("#/team?new=member"));
    let member = extract_braced_function(&script, "function openMemberDialog");
    assert!(member.contains("openEditDialog({"));
    assert!(member
        .contains("api(\"POST\", `/api/v1/teams/${encodeURIComponent(appState.teamId)}/members`"));
    assert!(member.contains("markSaved();"));
    assert!(member.contains("close();"));
    assert!(member.contains("await route();"));
    assert!(!member.contains("handle(error)"));
    let close = extract_braced_function(&script, "function closeLive");
    assert!(close.contains("[data-edit-overlay]"));
}

#[test]
fn template_create_and_edit_use_the_tabbed_overlay() {
    let script = web_source();
    let templates = extract_braced_function(&script, "async function pageTemplates");
    assert!(templates.contains("openTemplateDialog(item, request.tab)"));
    assert!(templates.contains("openTemplateDialog(null, request.tab)"));
    assert!(templates.contains("#/templates?edit="));
    assert!(templates.contains("#/templates?new=1"));
    assert!(templates.contains("clearEditHash();"));
    assert!(script.contains("addLink(\"#/templates?new=1\", \"template_2\")"));
    assert!(!script.contains("addLink(\"#/templates/new\""));
    assert!(!script.contains("href=\"#/templates/${"));
    let dialog = extract_braced_function(&script, "async function openTemplateDialog");
    assert!(dialog.contains("openEditDialog({"));
    assert!(dialog.contains("wide: true"));
    assert!(dialog.contains("{ id: \"general\", label: t(\"general\")"));
    assert!(dialog.contains("{ id: \"target\", label: t(\"target\")"));
    assert!(dialog.contains("{ id: \"options\", label: t(\"options\")"));
    assert!(dialog.contains("{ id: \"variables\", label: t(\"variables\")"));
    assert!(dialog.contains("vaultPicker.ids()"));
    assert!(dialog
        .contains("bindVaultPicker(form, credentials, selectedCredentialIds(item), readOnly)"));
    assert!(!dialog.contains("input[name=vault_ids]"));
    assert!(!script.contains("function bindVaultChoices"));
    assert!(!script.contains("openVaultPicker("));
    assert!(dialog.contains("showToolSections"));
    assert!(dialog.contains("#ansible-options"));
    assert!(dialog.contains("#tofu-options"));
    assert!(dialog.contains("#shell-options"));
    assert!(dialog.contains("environmentOptions(tool, selected)"));
    assert!(dialog.contains("api(\"POST\", \"/api/v1/templates\""));
    assert!(dialog.contains("api(\"PUT\", `/api/v1/templates/"));
    assert!(dialog.contains("markSaved();"));
    assert!(dialog.contains("close();"));
    assert!(dialog.contains("await route();"));
    assert!(dialog.contains("onTab: (tab) => rememberEditHash("));
    assert!(!dialog.contains("handle(error)"));
    let submit = dialog
        .find("onSubmit: readOnly ? undefined : async")
        .expect("template submit");
    let tab = dialog[submit..].find("onTab:").expect("template tab") + submit;
    assert!(dialog[submit..tab].contains("throw error"));
    assert!(!dialog[submit..tab].contains("handle(error)"));
    let remove = extract_braced_function(&script, "async function deleteTemplate");
    assert!(remove.contains("api(\"DELETE\", `/api/v1/templates/"));
    assert!(remove.contains("markSaved();"));
    assert!(remove.contains("dialog.close();"));
    assert!(remove.contains("await route();"));
    let fields = extract_braced_function(&script, "function templateCredentialFields");
    assert!(!fields.contains("inlineVault"));
    assert!(!fields.contains("name=\"vault_ids\""));
    assert!(fields.contains("id=\"vault-pills\""));
    assert!(fields.contains("id=\"vault-search\""));
    assert!(fields.contains("role=\"combobox\""));
    let bind = extract_braced_function(&script, "function bindVaultPicker");
    assert!(bind.contains("data-vault-pick"));
    assert!(bind.contains("event.key === \"Enter\""));
    assert!(!bind.contains("Backspace"));
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("panel.hidden = panel.dataset.dialogPanel !== current;"));
    assert!(!helper.contains("panel.innerHTML ="));
}

#[test]
fn template_vault_pills_add_by_search_remove_and_save() {
    let script = web_source();
    let functions = format!(
        "{}\n{}\n{}",
        extract_braced_function(&script, "function vaultSuggestions"),
        extract_braced_function(&script, "function renderVaultPills"),
        extract_braced_function(&script, "function bindVaultPicker")
    );
    let probe = format!(
        r##"{functions}
function esc(value) {{ return String(value); }}
function h(key, vars) {{ return vars ? `${{key}} ${{vars.name}}` : key; }}
function element() {{
  return {{
    innerHTML: "",
    hidden: false,
    value: "",
    handlers: {{}},
    attributes: {{}},
    addEventListener(type, handler) {{ this.handlers[type] = handler; }},
    setAttribute(name, value) {{ this.attributes[name] = value; }},
    focus() {{}},
    fire(type, event) {{ if (this.handlers[type]) this.handlers[type](event || {{}}); }},
  }};
}}
const box = element();
const search = element();
const options = element();
const form = {{ querySelector(selector) {{
  if (selector === "#vault-pills") return box;
  if (selector === "#vault-search") return search;
  if (selector === "#vault-options") return options;
  return null;
}} }};
const credentials = Array.from({{ length: 30 }}, (_item, index) => ({{
  id: `vault-${{index}}`,
  kind: "vault",
  name: index === 3 ? "Alpha vault" : index === 17 ? "Beta vault" : `Vault ${{index}}`,
}}));
const picker = bindVaultPicker(form, credentials, [], false);
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
check(picker.ids().length === 0, "pills did not start empty");
search.value = "alpha";
search.fire("input");
search.fire("keydown", {{ key: "Enter", preventDefault() {{}} }});
check(picker.ids().join(",") === "vault-3", "search did not add the highlighted vault");
search.value = "beta";
search.fire("input");
search.fire("keydown", {{ key: "Enter", preventDefault() {{}} }});
check(picker.ids().join(",") === "vault-3,vault-17", "second search did not append");
const target = {{
  dataset: {{ vaultId: "vault-3" }},
  closest(selector) {{
    if (selector === ".chip-remove") return target;
    if (selector === ".chip") return {{ dataset: {{ vaultId: "vault-3" }} }};
    return null;
  }},
}};
box.fire("click", {{ target }});
check(picker.ids().join(",") === "vault-17", "remove did not drop the first pill");
search.value = "";
const before = picker.ids().join(",");
search.fire("keydown", {{ key: "Backspace" }});
check(picker.ids().join(",") === before, "Backspace removed a pill");
const body = {{ credential_ids: picker.ids() }};
check(body.credential_ids.join(",") === "vault-17", "saved credential_ids changed");
const readonlyBox = element();
const readonly = bindVaultPicker({{ querySelector(selector) {{ return selector === "#vault-pills" ? readonlyBox : null; }} }}, credentials, ["vault-3"], true);
check(readonly.ids().join(",") === "vault-3", "read-only picker lost the selected vault");
check(readonlyBox.innerHTML.includes("Alpha vault"), "read-only pills are missing");
check(!readonlyBox.innerHTML.includes("chip-remove"), "read-only pills have a remove button");
"##
    );
    run_node_probe("template-vault-pills", &probe);
}

#[test]
fn viewers_open_templates_in_a_read_only_overlay() {
    let script = web_source();
    let templates = extract_braced_function(&script, "async function pageTemplates");
    assert!(templates.contains("await openTemplateDialog(item, request.tab);"));
    assert!(
        !templates.contains("if (can(\"operator\")) await openTemplateDialog(item, request.tab);")
    );
    assert!(templates.contains("<a href=\"#/templates?edit=${esc(item.id)}\">"));
    let dialog = extract_braced_function(&script, "async function openTemplateDialog");
    assert!(dialog.contains("const readOnly = !can(\"operator\");"));
    assert!(dialog.contains("title: readOnly ? t(\"read_only_template\", { name: item.name })"));
    assert!(dialog.contains("hint: readOnly ? \"read_only_template_hint\" : undefined"));
    assert!(dialog.contains("form: true"));
    assert!(dialog.contains("submit: readOnly ? undefined : editing ? t(\"save\") : t(\"create\")"));
    assert!(dialog.contains("onSubmit: readOnly ? undefined : async"));
    assert!(dialog.contains("!readOnly && editing"));
    assert!(dialog.contains("if (readOnly) {"));
    assert!(dialog.contains("control.disabled = true;"));
    assert!(dialog.contains("button.hasAttribute(\"data-dialog-cancel\")"));
    assert!(dialog.contains("button.classList.contains(\"dialog-close\")"));
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("form = Boolean(onSubmit)"));
    assert!(helper.contains("    form,\n    actions,"));
    let probe = format!(
        r#"{helper}
const appState = {{
  editSerial: 0,
  leaveGuard: null,
  setEditSerial(value) {{ return (this.editSerial = value); }},
  setLeaveGuard(value) {{ return (this.leaveGuard = value); }},
}};
function clearEditHash() {{}}
function h(value) {{ return value; }}
let captured = null;
function openDialog(options) {{
  captured = options;
  return {{
    overlay: {{ setAttribute() {{}}, querySelectorAll() {{ return []; }} }},
    form: {{ addEventListener() {{}} }},
    dirty() {{ return false; }},
  }};
}}
openEditDialog({{ title: "read only", body: "" }});
if (captured.form !== false) {{ console.error("missing submit dialog defaulted to a form"); process.exit(1); }}
openEditDialog({{ title: "read only", body: "", form: true }});
if (captured.form !== true) {{ console.error("explicit form was not forwarded"); process.exit(1); }}
openEditDialog({{ title: "edit", body: "", onSubmit: async () => {{}} }});
if (captured.form !== true) {{ console.error("submit dialog did not default to a form"); process.exit(1); }}
"#
    );
    run_node_probe("edit-dialog-form", &probe);
}

#[test]
fn tabbed_dialogs_keep_the_tab_row_in_the_chrome() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = web_source();
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(script.contains("footer = \"\", tabs = \"\""));
    assert!(script.contains("overlay.className = tabs ? \"overlay tabbed\" : \"overlay\";"));
    assert!(script.contains("${tabs ? \" tabbed\" : \"\"}"));
    let head_at = script
        .find("</div>\n      ${tabs}")
        .expect("tabs after the head");
    let form_at = script
        .find("<${shellTag} class=\"dialog-form\">")
        .expect("form");
    assert!(
        head_at < form_at,
        "tabs must be rendered before the scrolling body"
    );
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("tabs: tabBar,"));
    assert!(
        !helper.contains("${tabBar}${panelMarkup}"),
        "tab bar still sits in the scrolling body"
    );
    assert!(css.contains(".overlay.tabbed { align-items: flex-start; }"));
    let narrow = css
        .split("@media (max-width: 600px)")
        .nth(1)
        .and_then(|rest| rest.get(..rest.len().min(400)))
        .expect("narrow overlay rule");
    assert!(
        narrow.contains(".overlay.tabbed"),
        "tabbed overlays must stay full screen"
    );
    assert!(
        narrow.contains(".overlay-card.dialog.tabbed"),
        "the narrow full-screen rule must win over the tabbed max height"
    );
    let tabs_rule = css
        .split(".dialog-tabs {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("dialog-tabs rule");
    assert!(
        tabs_rule.contains("flex: none;"),
        "the tab row must not scroll with the body"
    );
    let tab_rule = css
        .split(".dialog-tab {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("dialog-tab rule");
    assert!(
        tab_rule.contains("font-weight: 600;"),
        "all tabs keep one width"
    );
    let selected = css
        .split(".dialog-tab[aria-selected=\"true\"] {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("selected tab rule");
    assert!(
        !selected.contains("font-weight"),
        "selection must not change the label width"
    );
}

#[test]
fn template_overlay_addresses_round_trip_and_old_pages_redirect() {
    let script = web_source();
    for redirect in [
        "registerRedirect(\"#/templates/new\", \"#/templates?new=1\")",
        "registerRedirect(\"#/templates/:id\", ({ id }) => `#/templates?edit=${encodeURIComponent(id)}`)",
    ] {
        assert!(script.contains(redirect), "missing redirect {redirect}");
    }
    let functions = format!(
        "{}\n{}\n{}\n{}",
        extract_braced_function(&script, "function editQuery"),
        extract_braced_function(&script, "function editAddress"),
        extract_braced_function(&script, "function rememberEditHash"),
        extract_braced_function(&script, "function clearEditHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/templates" }};
const appState = {{ shownHash: null, setShownHash(value) {{ return (this.shownHash = value); }} }};
let pushed = 0;
let replaced = 0;
const history = {{
  pushState(_state, _title, url) {{ pushed += 1; location.hash = url; }},
  replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }},
}};
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const edit = editQuery("#/templates?edit=t1&tab=target");
check(edit.edit === "t1" && edit.tab === "target" && edit.create === null, "template edit and tab were not read");
const create = editQuery("#/templates?new=1&tab=options");
check(create.create === "1" && create.edit === "" && create.tab === "options", "template new and tab were not read");
check(editAddress({{ edit: "t1", tab: "target" }}, "#/templates") === "#/templates?edit=t1&tab=target", "template edit address was not built");
check(editAddress({{ create: "1", tab: "variables" }}, "#/templates") === "#/templates?new=1&tab=variables", "template new address was not built");
check(editAddress({{ edit: "t1" }}, "#/templates?tab=target&page=2") === "#/templates?page=2&edit=t1", "old tab was not cleared");
rememberEditHash({{ create: "1", tab: "general" }});
check(pushed === 1 && location.hash === "#/templates?new=1&tab=general" && appState.shownHash === location.hash, "template open did not push the address");
rememberEditHash({{ edit: "t1", tab: "target" }}, false);
check(pushed === 1 && replaced === 1 && location.hash === "#/templates?edit=t1&tab=target", "template tab did not replace the address");
clearEditHash();
check(replaced === 2 && location.hash === "#/templates" && appState.shownHash === "#/templates", "template close did not clear the address");
"##
    );
    run_node_probe("template-address", &probe);
}

#[test]
fn workflow_create_uses_the_edit_overlay() {
    let script = web_source();
    let workflows = extract_braced_function(&script, "async function pageWorkflows");
    assert!(workflows.contains("#/workflows?new=1"));
    assert!(workflows.contains("const request = editQuery();"));
    assert!(workflows.contains("if (request.create !== null) {"));
    assert!(workflows.contains("if (can(\"operator\")) openWorkflowDialog();"));
    assert!(script.contains("addLink(\"#/workflows?new=1\", \"workflow_2\")"));
    assert!(!script.contains("addLink(\"#/workflows/new\""));
    assert!(!script.contains("async function pageWorkflowNew"));
    assert!(!script.contains("id=\"workflow-new\""));
    let dialog = extract_braced_function(&script, "function openWorkflowDialog");
    assert!(dialog.contains("openEditDialog({"));
    assert!(dialog.contains("title: t(\"new_workflow\")"));
    assert!(dialog.contains("class=\"field required\""));
    assert!(dialog.contains("name=\"name\" required"));
    assert!(dialog.contains("maxlength=\"1000\""));
    assert!(dialog.contains("submit: t(\"create_and_open\")"));
    assert!(dialog.contains("api(\"POST\", \"/api/v1/workflows\""));
    assert!(dialog.contains("graph: { nodes: [], edges: [] }"));
    assert!(dialog.contains("markSaved();"));
    assert!(dialog.contains("close();"));
    assert!(dialog.contains("go(`#/workflows/${created.id}`);"));
    assert!(!dialog.contains("handle(error)"));
}

#[test]
fn workflow_create_overlay_submits_and_keeps_errors() {
    let script = web_source();
    let dialog = extract_braced_function(&script, "function openWorkflowDialog");
    let probe = format!(
        r##"{dialog}
function t(key) {{ return key; }}
function h(key) {{ return key; }}
let captured = null;
let posted = null;
let saved = 0;
let closed = 0;
let navigated = "";
let fail = false;
function markSaved() {{ saved += 1; }}
function go(hash) {{ navigated = hash; }}
function openEditDialog(options) {{ captured = options; }}
function api(method, path, body) {{
  posted = {{ method, path, body }};
  if (fail) return Promise.reject(new Error("workflow_error"));
  return Promise.resolve({{ id: "w1" }});
}}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
function submit() {{
  return captured.onSubmit({{
    form: {{ name: {{ value: "Heal" }}, description: {{ value: "restart" }} }},
    close: () => {{ closed += 1; }},
  }});
}}
openWorkflowDialog();
check(captured.title === "new_workflow", "workflow overlay title is missing");
submit().then(async () => {{
  check(posted.method === "POST" && posted.path === "/api/v1/workflows", "workflow create did not post");
  check(posted.body.name === "Heal" && posted.body.description === "restart", "workflow body is wrong");
  check(posted.body.graph.nodes.length === 0 && posted.body.graph.edges.length === 0, "workflow graph is not empty");
  check(saved === 1 && closed === 1 && navigated === "#/workflows/w1", "workflow create did not open the editor");
  fail = true;
  try {{
    await submit();
    console.error("workflow error was swallowed");
    process.exit(1);
  }} catch (error) {{
    check(error.message === "workflow_error", "workflow error changed");
  }}
  check(closed === 1 && navigated === "#/workflows/w1", "workflow error closed the overlay or navigated");
}}).catch((error) => {{ console.error(error); process.exit(1); }});
"##
    );
    run_node_probe("workflow-create", &probe);
}

#[test]
fn workflow_overlay_addresses_round_trip_and_old_pages_redirect() {
    let script = web_source();
    assert!(script.contains("registerRedirect(\"#/workflows/new\", \"#/workflows?new=1\")"));
    assert!(!script.contains("await pageWorkflowNew();"));
    let functions = format!(
        "{}\n{}\n{}\n{}",
        extract_braced_function(&script, "function editQuery"),
        extract_braced_function(&script, "function editAddress"),
        extract_braced_function(&script, "function rememberEditHash"),
        extract_braced_function(&script, "function clearEditHash")
    );
    let probe = format!(
        r##"{functions}
let location = {{ hash: "#/workflows" }};
const appState = {{ shownHash: null, setShownHash(value) {{ return (this.shownHash = value); }} }};
let pushed = 0;
let replaced = 0;
const history = {{
  pushState(_state, _title, url) {{ pushed += 1; location.hash = url; }},
  replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }},
}};
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const create = editQuery("#/workflows?new=1");
check(create.create === "1" && create.edit === "" && create.tab === "", "workflow new was not read");
check(editQuery("#/workflows").create === null, "workflow list was read as new");
check(editAddress({{ create: "1" }}, "#/workflows") === "#/workflows?new=1", "workflow new address was not built");
check(editAddress({{ create: "1" }}, "#/workflows?page=2") === "#/workflows?page=2&new=1", "workflow new address dropped the query");
rememberEditHash({{ create: "1" }});
check(pushed === 1 && location.hash === "#/workflows?new=1" && appState.shownHash === location.hash, "workflow open did not push the address");
clearEditHash();
check(replaced === 1 && location.hash === "#/workflows" && appState.shownHash === "#/workflows", "workflow close did not clear the address");
"##
    );
    run_node_probe("workflow-address", &probe);
}

#[test]
fn template_tool_switch_changes_options_and_environments() {
    let script = web_source();
    let dialog = extract_braced_function(&script, "async function openTemplateDialog");
    assert!(dialog.contains("form.tool.addEventListener(\"change\""));
    assert!(dialog.contains("showToolSections();"));
    assert!(dialog.contains("loadPlaybooks();"));
    assert!(dialog.contains(
        "dialog.overlay.querySelector(\"#ansible-options\").hidden = tool !== \"ansible\""
    ));
    assert!(dialog.contains(
        "dialog.overlay.querySelector(\"#tofu-options\").hidden = tool !== \"opentofu\""
    ));
    assert!(dialog
        .contains("dialog.overlay.querySelector(\"#shell-options\").hidden = tool !== \"shell\""));
    assert!(dialog.contains(
        "dialog.overlay.querySelector(\"#inventory-field\").hidden = tool !== \"ansible\""
    ));
    assert!(dialog.contains("form.environment_id.innerHTML = environmentOptions(tool, selected)"));
    assert!(dialog.contains("default_environment_${tool}"));
    assert!(dialog.contains("noEnvironmentAvailable"));
    assert!(dialog.contains("runner.local_tools"));
    let environments_for = extract_braced_function(&script, "const environmentsFor = (tool) =>");
    let probe = format!(
        r#"{environments_for}
const environments = [
  {{ id: "a1", tool: "ansible" }},
  {{ id: "t1", tool: "opentofu" }},
  {{ id: "s1", tool: "shell" }},
  {{ id: "any1", tool: "any" }},
  {{ id: "legacy" }}
];
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
check(environmentsFor("ansible").map((row) => row.id).join(",") === "a1,any1,legacy", "ansible environments do not match");
check(environmentsFor("opentofu").map((row) => row.id).join(",") === "t1,any1,legacy", "opentofu environments do not match");
check(environmentsFor("shell").map((row) => row.id).join(",") === "s1,any1,legacy", "shell environments do not match");
"#
    );
    run_node_probe("template-environments", &probe);
}

#[test]
fn template_validation_errors_open_the_matching_tab() {
    let script = web_source();
    let helper = extract_function_with_destructured_params(&script, "function openEditDialog");
    assert!(helper.contains("mark(panel.dataset.dialogPanel);"));
    assert!(helper.contains("choose(panel.dataset.dialogPanel, false);"));
    let mapping = extract_braced_function(&script, "const ERROR_FIELD =");
    let probe = format!(
        r#"{mapping}
function check(key, want) {{
  if (ERROR_FIELD[key] !== want) {{
    console.error(key, ERROR_FIELD[key], want);
    process.exit(1);
  }}
}}
check("tool_is_ansible_opentofu_or_shell", "tool");
check("project_is_missing", "project_id");
check("execution_environment_is_missing", "environment_id");
check("the_execution_environment_is_for_another_tool", "environment_id");
check("action_is_plan_or_apply", "action");
check("ansible_needs_a_playbook_as_the_entry", "entrypoint");
check("opentofu_needs_a_module_directory", "entrypoint");
check("shell_needs_a_path_or_spec_script", "entrypoint");
check("the_entry_path_must_be_relative_and_must", "entrypoint");
check("extra_variables_must_be_a_json_object", "extra_vars");
check("template.vars_not_objects", "extra_vars");
"#
    );
    run_node_probe("template-error-tabs", &probe);
}

#[test]
fn template_form_saves_the_visible_playbook_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = web_source();
    assert!(script.contains("fields_marked_with_a_star_are_required"));
    assert!(script.contains("class=\"field required\""));
    assert!(script.contains("classList.toggle(\"required\", tool !== \"shell\")"));
    let css = std::fs::read_to_string(root.join("web/app.css")).expect("app.css");
    assert!(css.contains(".field.required"));
    assert_chosen_entrypoint_cases(&script);
    assert_playbook_choice_cases(&script);
}

#[test]
fn inventory_tables_drop_variables_and_details_keep_them() {
    let script = web_source();
    let host_table = extract_braced_function(&script, "function inventoryHostTable");
    let group_table = extract_braced_function(&script, "function inventoryGroupTable");
    for table in [&host_table, &group_table] {
        assert!(
            !table.contains("h(\"variables\")"),
            "inventory table still renders a variables header"
        );
        assert!(
            !table.contains("formatVars"),
            "inventory table still renders variables"
        );
    }
    assert!(
        !script.contains("function formatVars"),
        "formatVars should be removed with the column"
    );
    assert!(host_table.contains("head(\"address\", \"address\")"));
    assert!(host_table.contains("head(\"groups\", \"groups\")"));
    assert!(group_table.contains("h(\"hosts\")"));
    assert!(group_table.contains("h(\"groups\")"));

    let host_detail = extract_braced_function(&script, "async function pageInventoryHost");
    let group_detail = extract_braced_function(&script, "async function pageInventoryGroup");
    assert!(host_detail.contains("variablesViewer(host.vars || [])"));
    assert!(group_detail.contains("variablesViewer(vars)"));
    assert!(host_detail.contains("bindVariablesViews();"));
    assert!(group_detail.contains("bindVariablesViews();"));
    assert!(script.contains(
        "variablesViewer(Object.entries(result.sample_host.vars || {}).map(([name, value]) => ({ name, value, origin: \"host\" })))"
    ));
    assert!(script.contains("bindVariablesViews(resultBox);"));

    let functions = [
        "function looksSecretVar",
        "function variableText",
        "function variableOrigin",
        "function yamlPlain",
        "function variableScalar",
        "function variableKey",
        "function effectiveVariablesYaml",
        "function jsonVariableMarkup",
        "function effectiveVariablesJson",
        "function variablesViewer",
    ]
    .iter()
    .map(|needle| extract_braced_function(&script, needle))
    .collect::<Vec<_>>()
    .join("\n");
    let probe = format!(
        r#"{functions}
const appState = {{ variablesViewSerial: 0, setVariablesViewSerial(value) {{ return (this.variablesViewSerial = value); }} }};
function t(key) {{ return key; }}
function esc(value) {{
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({{
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }}[ch]));
}}
function h(key) {{ return esc(t(key)); }}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
const vars = [
  {{ name: "ansible_host", value: "10.0.0.1", origin: "host" }},
  {{ name: "message", value: "hello world", origin: "web" }},
  {{ name: "ansible_become_password", value: "s3cret", origin: "host" }},
  {{ name: "evil", value: "<script>x &lt; y</script>", origin: "group" }},
];
const yaml = effectiveVariablesYaml(vars);
for (const want of [
  '<span class="vars-key">ansible_host</span>: <span class="vars-value">10.0.0.1</span>',
  '<span class="vars-value">&quot;hello world&quot;</span>',
  '<span class="vars-value">&quot;••••••&quot;</span>',
  '<span class="vars-comment"># host</span>',
  '<span class="vars-comment"># web</span>',
]) {{
  check(yaml.includes(want), "yaml missing " + want + "\n" + yaml);
}}
check(yaml.includes("&lt;script&gt;x &amp;lt; y&lt;/script&gt;"), "yaml value was not escaped\n" + yaml);
check(!yaml.includes("<script>"), "yaml kept raw markup\n" + yaml);

const json = effectiveVariablesJson(vars);
for (const want of [
  '<span class="vars-key">&quot;name&quot;</span>',
  '<span class="vars-key">&quot;value&quot;</span>',
  '<span class="vars-key">&quot;origin&quot;</span>',
  '<span class="vars-value">&quot;ansible_host&quot;</span>',
  '<span class="vars-value">&quot;host&quot;</span>',
  '<span class="vars-value">&quot;••••••&quot;</span>',
  '&lt;script&gt;x &amp;lt; y&lt;/script&gt;',
]) {{
  check(json.includes(want), "json missing " + want + "\n" + json);
}}
check(!json.includes("<script>"), "json kept raw markup\n" + json);
check(json.includes("\n  {{\n"), "json is not pretty printed\n" + json);

const view = variablesViewer(vars);
check(view.includes('<div class="spread tab-bar">'), "missing page tab bar\n" + view);
check(view.includes('<div class="tabs" role="tablist"'), "missing page tabs\n" + view);
check(view.includes('data-vars-tab="yaml" aria-selected="true"'), "yaml is not selected by default\n" + view);
check(view.includes('data-vars-tab="json" aria-selected="false"'), "json tab is not unselected\n" + view);
check(/<div id="vars-\d+-yaml" role="tabpanel" data-vars-panel="yaml"><pre class="vars">/.test(view), "yaml panel is missing\n" + view);
check(/<div id="vars-\d+-json" role="tabpanel" data-vars-panel="json" hidden>/.test(view), "json panel is missing\n" + view);

const weird = effectiveVariablesYaml([
  {{ name: "a", value: "1", origin: "team\n  injected: true" }},
  {{ name: "b", value: "2", origin: "team\u0085  injected: true" }},
]);
check(!weird.includes("\n  injected"), "origin line break added a mapping line\n" + weird);
check(/# team injected: true/.test(weird.replace(/\s+/g, " ")), "origin was not flattened\n" + weird);
check(!/[\u0085\u2028\u2029]/.test(weird), "origin kept a Unicode line break\n" + weird);

const empty = variablesViewer([]);
check((empty.match(/no_variables/g) || []).length === 2, "empty state is not in both formats\n" + empty);
check(!empty.includes('<pre class="vars">'), "empty state rendered a code block\n" + empty);
"#
    );
    run_node_probe("inventory-vars-yaml-json", &probe);
}

#[test]
fn route_leave_guard_only_blocks_changed_forms() {
    let script = web_source();
    let route = extract_braced_function(&script, "async function route");
    let probe = format!(
        r##"const appState = {{
  leaveGuard: null,
  shownHash: null,
  navToken: null,
  me: null,
  setLeaveGuard(value) {{ return (this.leaveGuard = value); }},
  setShownHash(value) {{ return (this.shownHash = value); }},
  setNavToken(value) {{ return (this.navToken = value); }},
}};
let location = {{ hash: "#/" }};
let replaced = 0;
const history = {{ replaceState(_state, _title, url) {{ replaced += 1; location.hash = url; }} }};
let prompts = 0;
let promptedKey = "";
let confirmAnswer = true;
async function confirmDialog({{ title }}) {{ prompts += 1; promptedKey = title; return confirmAnswer; }}
function t(key) {{ promptedKey = key; return key; }}
function closeLive() {{}}
let rendered = 0;
function renderLogin() {{ rendered += 1; }}
{route}
function reset(hash) {{
  location.hash = hash;
  appState.setShownHash(hash);
  replaced = 0;
  prompts = 0;
  promptedKey = "";
  rendered = 0;
  confirmAnswer = true;
}}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
(async () => {{
  reset("#/projects/p1");
  appState.setLeaveGuard(() => false);
  location.hash = "#/jobs";
  await route();
  check(rendered === 1 && appState.shownHash === "#/jobs" && replaced === 0 && prompts === 0, "unchanged project was blocked");

  reset("#/inventories/i1");
  appState.setLeaveGuard(() => false);
  location.hash = "#/jobs";
  await route();
  check(rendered === 1 && appState.shownHash === "#/jobs" && replaced === 0 && prompts === 0, "unchanged inventory was blocked");

  reset("#/projects/p1");
  appState.setLeaveGuard(() => true);
  confirmAnswer = false;
  location.hash = "#/jobs";
  await route();
  check(prompts === 1 && promptedKey === "discard_unsaved_changes" && replaced === 1 && location.hash === "#/projects/p1" && appState.shownHash === "#/projects/p1" && rendered === 0, "cancel left the form");

  reset("#/inventories/i1");
  appState.setLeaveGuard(() => true);
  location.hash = "#/jobs";
  await route();
  check(prompts === 1 && replaced === 0 && appState.shownHash === "#/jobs" && rendered === 1, "confirm did not navigate");

  reset("#/projects/p1");
  appState.setLeaveGuard(() => false);
  location.hash = "#/projects";
  await route();
  check(rendered === 1 && appState.shownHash === "#/projects" && replaced === 0, "browser back was blocked");
}})();
"##
    );
    run_node_probe("route-leave-guard", &probe);
}

#[test]
fn the_preflight_notice_uses_the_missing_tool_and_runtime() {
    let script = web_source();
    assert!(script.contains("id=\"preflight-check\""));
    assert!(script.contains("api(\"GET\", \"/api/v1/runner/preflight\")"));
    let function = extract_braced_function(&script, "function preflightNotices(report)");
    let probe = format!(
        r#"{function}
const cases = [
  [null, []],
  [{{ok: true, missing: []}}, []],
  [{{ok: false, missing: [{{kind: "tool", tool: "ansible-playbook", template_tool: "ansible", reason: "missing"}}]}}, ["preflight_tool_missing"]],
  [{{ok: false, missing: [{{kind: "runtime", tool: "docker/podman", template_tool: "", reason: "missing"}}, {{kind: "tool", tool: "ansible-playbook", template_tool: "ansible", reason: "missing"}}]}}, ["preflight_runtime_missing", "preflight_no_runtime_and_tool_missing"]],
  [{{ok: false, missing: [{{kind: "tool", tool: "tofu", template_tool: "opentofu", reason: "version", version: "1.5.0", minimum: "1.6"}}]}}, ["preflight_tool_too_old"]],
  [{{ok: false, missing: [{{kind: "tool", tool: "sh", template_tool: "shell", reason: "timeout"}}]}}, ["preflight_tool_unusable"]],
  [{{ok: false, missing: [{{kind: "project", tool: "git", template_tool: "", reason: "missing"}}]}}, ["preflight_project_missing"]],
  [{{ok: false, missing: [{{kind: "project", tool: "git", template_tool: "", reason: "failed"}}]}}, ["preflight_project_unusable"]],
  [{{ok: false, missing: [{{kind: "tool", tool: "ssh", template_tool: "ansible", reason: "missing"}}]}}, ["preflight_tool_missing"]]
];
for (const [report, want] of cases) {{
  const got = preflightNotices(report).map((item) => item.key);
  if (JSON.stringify(got) !== JSON.stringify(want)) {{
    console.error(JSON.stringify({{report, want, got}}));
    process.exit(1);
  }}
}}
"#
    );
    run_node_probe("preflight-notice", &probe);
}

#[test]
fn dashboard_page_has_tabs_default_range_empty_state_and_cache_numbers() {
    let script = web_source();
    let tabs = extract_braced_function(&script, "function dashboardTabs(tab)");
    let chart = extract_braced_function(
        &script,
        "function dashboardChart(series, values, heading, legend)",
    );
    let body = extract_braced_function(&script, "function dashboardBody(data, range)");
    let short = extract_braced_function(&script, "function shortDay(value)");
    let range = extract_braced_function(&script, "function dashboardRange()");
    let address = extract_braced_function(&script, "function dashboardAddress(tab, range)");
    let probe = format!(
        r##"{tabs}
{chart}
{body}
{short}
{range}
{address}
function currentLanguage() {{ return "en"; }}
let location = {{ hash: "#/" }};
function esc(value) {{ return String(value ?? ""); }}
function t(key) {{ return key; }}
function h(key) {{ return esc(t(key)); }}
function pill(status) {{ return `<span class="pill ${{status}}">${{status}}</span>`; }}
function bytes(value) {{ return `${{Number(value) || 0}} B`; }}
function check(ok, message) {{
  if (!ok) {{ console.error(message); process.exit(1); }}
}}
check(dashboardRange() === "1", "default range is not today");
location.hash = "#/?tab=dashboard&days=7";
check(dashboardRange() === "7", "range did not come from the address");
check(dashboardAddress("dashboard", "1") === "#/?tab=dashboard", "dashboard tab is not kept");
check(dashboardAddress("overview", "7") === "#/", "overview address carries dashboard state");
const tabsView = dashboardTabs("overview");
check(tabsView.includes('role="tablist"'), "missing tablist");
check(tabsView.includes('id="tab-overview" aria-selected="true"'), "overview is not selected");
check(tabsView.includes('id="tab-dashboard" aria-selected="false"'), "dashboard is not unselected");
const empty = dashboardBody({{ series: [{{ date: "2026-11-05", started: 0, successful: 0, failed: 0, canceled: 0 }}], cache: {{ collections: 0, roles: 0, tofu_bytes: 0 }} }}, "1");
check(empty.includes("no_jobs_in_this_range"), "empty range has no empty state");
check(empty.includes('data-dashboard-range="1" aria-pressed="true"'), "today is not the default range");
check(empty.includes('data-dashboard-range="7" aria-pressed="false"'), "7 days is not offered");
check(empty.includes('data-dashboard-range="14" aria-pressed="false"'), "14 days is not offered");
const filled = dashboardBody({{ series: [{{ date: "2026-11-05", started: 3, successful: 1, failed: 1, canceled: 1 }}], cache: {{ collections: 2, roles: 4, tofu_bytes: 2048 }} }}, "7");
check(filled.includes("jobs_finished_per_day"), "missing finished chart");
check(filled.includes("jobs_started_per_day"), "missing started chart");
check(filled.includes('<span class="small muted">collections</span><strong>2</strong>'), "collections number is missing");
check(filled.includes('<span class="small muted">roles</span><strong>4</strong>'), "roles number is missing");
check(filled.includes('<span class="small muted">opentofu</span><strong>2048 B</strong>'), "tofu bytes are missing");
const cachePart = filled.slice(filled.indexOf("dependency_cache"));
check(!cachePart.includes('class="chart"'), "cache drew a fake chart line");
"##
    );
    run_node_probe("dashboard-page", &probe);
}
