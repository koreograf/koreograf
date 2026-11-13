use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde_json::json;

use super::*;

#[test]
fn reads_hosts_groups_and_skips_children() {
    let body = "\
# demo
[web]
web-1 ansible_host=10.0.0.5
web-2

[db]
db-1 ansible_user=ops

[web:vars]
ansible_port=22

[app:children]
web
db

[all]
web-1 ansible_connection=local
";
    let hosts = parse_hosts(body);
    assert_eq!(hosts.len(), 3);
    assert_eq!(hosts[0].name, "web-1");
    assert_eq!(hosts[0].groups, vec!["web".to_string(), "all".to_string()]);
    assert_eq!(
        hosts[0].vars.get("ansible_host").map(String::as_str),
        Some("10.0.0.5")
    );
    assert_eq!(
        hosts[0].vars.get("ansible_connection").map(String::as_str),
        Some("local")
    );
    assert_eq!(hosts[1].name, "web-2");
    assert_eq!(hosts[2].name, "db-1");
    assert!(hosts
        .iter()
        .all(|host| host.name != "web" && host.name != "db"));

    let hosts = parse_hosts("[web]\nweb-1 ansible_ssh_common_args=\"-o ProxyCommand =x\"\n");
    assert_eq!(
        hosts[0]
            .vars
            .get("ansible_ssh_common_args")
            .map(String::as_str),
        Some("-o ProxyCommand =x")
    );
}

#[test]
fn reads_yaml_inventory_and_skips_plugin_files() {
    let body = "\
all:
  hosts:
    web-1:
      ansible_host: 10.0.0.8
  children:
    db:
      hosts:
        db-1:
          ansible_user: ops
";
    let hosts = parse_hosts(body);
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0].name, "web-1");
    assert_eq!(hosts[0].groups, vec!["all".to_string()]);
    assert_eq!(
        hosts[0].vars.get("ansible_host").map(String::as_str),
        Some("10.0.0.8")
    );
    assert_eq!(hosts[1].name, "db-1");
    assert_eq!(hosts[1].groups, vec!["db".to_string()]);
    assert!(
        parse_hosts("plugin: netbox.netbox.nb_inventory\napi_endpoint: https://nb\n").is_empty()
    );
}

#[test]
fn reads_yaml_unsafe_strings_without_the_tag() {
    let body = "\
all:
  hosts:
    web-1:
      motd: !unsafe \"{{ lookup('pipe', 'id') }}\"
      ntp:
        server: !unsafe pool.ntp.org
";
    let hosts = parse_hosts(body);
    assert_eq!(hosts.len(), 1);
    assert_eq!(
        hosts[0].vars.get("motd").map(String::as_str),
        Some("{{ lookup('pipe', 'id') }}")
    );
    assert_eq!(
        hosts[0].vars.get("ntp.server").map(String::as_str),
        Some("pool.ntp.org")
    );
}

#[test]
fn computes_effective_variables_for_ini() {
    let body = "\
[all:vars]
ansible_user=ansible
shared=all

[app:children]
web

[app:vars]
app_port=1

[web:vars]
http_port=8080
shared=web

[web]
web-1 ansible_user=web-user shared=host
";
    let data = parse_inventory(body);
    let vars = effective_vars(&data, "web-1");
    let find = |name: &str| vars.iter().find(|item| item.name == name).unwrap();
    assert_eq!(find("ansible_user").value, "web-user");
    assert_eq!(find("ansible_user").origin, "host");
    assert_eq!(find("shared").value, "host");
    assert_eq!(find("http_port").value, "8080");
    assert_eq!(find("http_port").origin, "web");
    assert_eq!(find("app_port").origin, "app");
    assert_eq!(host_groups(&data, "web-1"), vec!["all", "app", "web"]);
    assert_eq!(group_hosts(&data, "app").len(), 1);
}

#[test]
fn child_group_overrides_its_parent() {
    let body = "\
[app:children]
web

[app:vars]
shared=app

[web:vars]
shared=web

[web]
web-1
";
    let data = parse_inventory(body);
    let vars = effective_vars(&data, "web-1");
    let shared = vars.iter().find(|item| item.name == "shared").unwrap();
    assert_eq!(shared.value, "web");
    assert_eq!(shared.origin, "web");
}

#[test]
fn computes_effective_variables_for_yaml() {
    let body = "\
all:
  vars:
    ansible_user: ansible
    shared: all
  children:
    app:
      vars:
        app_port: \"1\"
      children:
        web:
          vars:
            http_port: \"8080\"
            shared: web
          hosts:
            web-1:
              ansible_user: web-user
              shared: host
";
    let data = parse_inventory(body);
    let vars = effective_vars(&data, "web-1");
    let find = |name: &str| vars.iter().find(|item| item.name == name).unwrap();
    assert_eq!(find("ansible_user").value, "web-user");
    assert_eq!(find("ansible_user").origin, "host");
    assert_eq!(find("shared").value, "host");
    assert_eq!(find("http_port").value, "8080");
    assert_eq!(find("http_port").origin, "web");
    assert_eq!(find("app_port").origin, "app");
    assert_eq!(find("ansible_user").origin, "host");
    assert_eq!(host_groups(&data, "web-1"), vec!["all", "app", "web"]);
    assert_eq!(group_hosts(&data, "all").len(), 1);
}

#[test]
fn merges_hosts_from_several_sources() {
    let hosts = merge_inventory(&[
        "[web]\nweb-1\n",
        "[db]\ndb-1\n[web]\nweb-1 ansible_host=10.0.0.8\n",
    ])
    .hosts;
    assert_eq!(hosts.len(), 2);
    let web = hosts.iter().find(|host| host.name == "web-1").unwrap();
    assert_eq!(web.groups, vec!["web".to_string()]);
    assert_eq!(
        web.vars.get("ansible_host").map(String::as_str),
        Some("10.0.0.8")
    );
    assert!(hosts.iter().any(|host| host.name == "db-1"));
}

#[test]
fn rejects_broken_ini_lines_with_their_line_number() {
    assert_eq!(find_ini_error("[all\nh1 ansible_host=\n[[["), Some(1));
    assert_eq!(find_ini_error("[web]\nweb-1\n[db\ndb-1\n"), Some(3));
    assert_eq!(find_ini_error("[web]\nansible_host=10.0.0.5\n"), Some(2));
    assert_eq!(find_ini_error("[web]\nweb-1 =\n"), Some(2));
    assert_eq!(find_ini_error("[]\n"), Some(1));
}

#[test]
fn accepts_valid_ini_and_non_ini_bodies() {
    let body = "\
# demo
[web]
web-1 ansible_host=10.0.0.5
web-2

[db]
db-1 ansible_user=ops

[web:vars]
ansible_port=22

[app:children]
web
db
";
    assert_eq!(find_ini_error(body), None);
    assert_eq!(find_ini_error(""), None);
    assert_eq!(find_ini_error("# only a comment\n"), None);
    assert_eq!(
        find_ini_error("plugin: netbox.netbox.nb_inventory\napi_endpoint: https://nb\n"),
        None
    );
    assert_eq!(
        find_ini_error("all:\n  hosts:\n    web-1:\n      ansible_host: 10.0.0.8\n"),
        None
    );
    assert_eq!(
        find_ini_error("[web]\nweb-1 ansible_ssh_common_args=\"-o ProxyCommand =x\"\n"),
        None
    );
}

#[derive(Clone)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn renders_groups_like_the_ansible_plugin() {
    let device = json!({
        "id": 7,
        "name": "web 1",
        "primary_ip4": {"address": "10.0.0.8/24"},
        "site": {"slug": "berlin"},
        "role": {"slug": "web"},
        "status": {"value": "active"},
        "tags": [{"slug": "prod"}, {"slug": "edge"}]
    });
    let host = host_from_json(
        &device,
        false,
        &["sites".into(), "device_roles".into(), "tags".into()],
        false,
        false,
    )
    .unwrap();
    let ini = render_ini(&[host]);
    assert!(ini.contains("web_1 ansible_host=10.0.0.8 netbox_id=7"));
    assert!(ini.contains("[sites_berlin]\nweb_1\n"));
    assert!(ini.contains("[device_roles_web]\nweb_1\n"));
    assert!(ini.contains("[tags_prod]\nweb_1\n"));
    assert!(ini.contains("[tags_edge]\nweb_1\n"));
}

#[test]
fn uses_primary_ip_when_the_primary_ip4_key_is_absent() {
    let device = json!({
        "id": 13,
        "name": "legacy",
        "primary_ip": {"address": "2001:db8::13/64"}
    });
    let host = host_from_json(&device, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("2001:db8::13"));
}

#[test]
fn sets_ansible_host_from_primary_ip_when_primary_ip4_is_null() {
    let device = json!({
        "id": 8,
        "name": "v6-only",
        "primary_ip4": null,
        "primary_ip": {"address": "2001:db8::8/64"},
        "primary_ip6": {"address": "2001:db8::99/64"}
    });
    let host = host_from_json(&device, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("2001:db8::8"));
    let ini = render_ini(&[host]);
    assert!(ini.contains("v6-only ansible_host=2001:db8::8 netbox_id=8\n"));
    let listed = parse_hosts(&ini);
    assert_eq!(
        listed[0].vars.get("ansible_host").map(String::as_str),
        Some("2001:db8::8")
    );
}

#[test]
fn sets_ansible_host_from_primary_ip6_when_that_is_the_only_address() {
    let device = json!({
        "id": 9,
        "name": "v6-field",
        "primary_ip4": null,
        "primary_ip": null,
        "primary_ip6": {"address": "2001:db8::9/64"}
    });
    let host = host_from_json(&device, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("2001:db8::9"));
    let ini = render_ini(&[host]);
    assert!(ini.contains("v6-field ansible_host=2001:db8::9 netbox_id=9\n"));
}

#[test]
fn skips_a_primary_ip4_object_that_has_no_address() {
    let device = json!({
        "id": 12,
        "name": "blank-v4",
        "primary_ip4": {"address": null},
        "primary_ip": null,
        "primary_ip6": {"address": "2001:db8::12/64"}
    });
    let host = host_from_json(&device, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("2001:db8::12"));
}

#[test]
fn prefers_ipv4_when_both_families_are_set() {
    let device = json!({
        "id": 11,
        "name": "dual",
        "primary_ip4": {"address": "10.0.0.8/24"},
        "primary_ip": {"address": "2001:db8::8/64"},
        "primary_ip6": {"address": "2001:db8::8/64"}
    });
    let host = host_from_json(&device, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("10.0.0.8"));
    let ini = render_ini(&[host]);
    assert!(ini.contains("dual ansible_host=10.0.0.8 netbox_id=11\n"));
}

#[test]
fn keeps_only_an_ip_address_as_ansible_host() {
    let v4 = json!({
        "id": 21,
        "name": "v4",
        "primary_ip4": {"address": "10.1.2.3"}
    });
    let host = host_from_json(&v4, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("10.1.2.3"));

    let v6 = json!({
        "id": 22,
        "name": "v6",
        "primary_ip4": null,
        "primary_ip6": {"address": "2001:db8::20"}
    });
    let host = host_from_json(&v6, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("2001:db8::20"));

    let prefixed = json!({
        "id": 23,
        "name": "prefixed",
        "primary_ip4": {"address": "192.0.2.10/24"}
    });
    let host = host_from_json(&prefixed, false, &[], false, false).unwrap();
    assert_eq!(host.ansible_host.as_deref(), Some("192.0.2.10"));
    assert!(render_ini(&[host]).contains("prefixed ansible_host=192.0.2.10 netbox_id=23\n"));

    let bad_address = "10.0.0.1 = injected\n[all]\nhacked ansible_host=1.2.3.4";
    let bad = json!({
        "id": 24,
        "name": "odd one",
        "primary_ip4": {"address": bad_address},
        "primary_ip": {"address": "10.9.9.9/32"},
        "primary_ip6": {"address": "2001:db8::99/64"},
        "site": {"slug": "berlin"}
    });
    let clean = json!({
        "id": 24,
        "name": "odd one",
        "primary_ip4": null,
        "site": {"slug": "berlin"}
    });
    let groups = ["sites".to_string()];
    let buf = LogBuf(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(buf.clone())
        .finish();
    let (bad_host, clean_host) = tracing::subscriber::with_default(subscriber, || {
        (
            host_from_json(&bad, false, &groups, false, false).unwrap(),
            host_from_json(&clean, false, &groups, false, false).unwrap(),
        )
    });
    assert!(bad_host.ansible_host.is_none());
    let ini = render_ini(&[bad_host]);
    assert_eq!(ini, render_ini(&[clean_host]));
    assert!(ini.contains("odd_one netbox_id=24\n"));
    assert!(ini.contains("[sites_berlin]\nodd_one\n"));
    assert!(!ini.contains("ansible_host"));
    assert!(!ini.contains("hacked"));
    let text = String::from_utf8(buf.0.lock().expect("log lock").clone()).expect("utf-8");
    assert!(text.contains("odd one"), "{text}");
    assert!(
        text.contains("NetBox address is not an IP address, so ansible_host was left unset"),
        "{text}"
    );
    assert!(!text.contains("hacked"), "{text}");
    assert_eq!(text.matches("not an IP address").count(), 1, "{text}");
}

#[test]
fn lists_a_device_with_no_address_and_no_ansible_host() {
    let device = json!({
        "id": 10,
        "name": "lonely",
        "primary_ip4": null,
        "primary_ip": null,
        "primary_ip6": null
    });
    let host = host_from_json(&device, false, &[], false, false).unwrap();
    assert!(host.ansible_host.is_none());
    let ini = render_ini(&[host]);
    assert!(ini.contains("lonely netbox_id=10\n"));
    assert!(!ini.contains("ansible_host"));
}

#[test]
fn replaces_hyphens_and_dots_in_netbox_group_names() {
    let device = json!({
        "name": "web-1.example",
        "tags": [
            {"slug": "prod-fake"},
            {"slug": "a--b"},
            {"slug": "-edge"},
            {"slug": "lab.west"}
        ]
    });
    let host = host_from_json(&device, false, &["tags".into()], false, false).unwrap();
    assert_eq!(
        host.groups,
        vec![
            "tags_prod_fake".to_string(),
            "tags_a_b".to_string(),
            "tags_edge".to_string(),
            "tags_lab_west".to_string(),
        ]
    );
    let ini = render_ini(std::slice::from_ref(&host));
    let groups: Vec<&str> = ini
        .lines()
        .filter_map(|line| {
            line.strip_prefix('[')
                .and_then(|item| item.strip_suffix(']'))
        })
        .collect();
    assert!(
        groups.iter().all(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        }),
        "{groups:?}"
    );
}

#[test]
fn keeps_plain_group_names_when_the_option_is_on() {
    let device = json!({
        "id": 30,
        "name": "web-1",
        "tags": [{"slug": "webserver"}],
        "role": {"slug": "web"},
        "site": {"slug": "berlin"}
    });
    let prefixed = host_from_json(
        &device,
        false,
        &["tags".into(), "device_roles".into()],
        false,
        false,
    )
    .unwrap();
    assert!(prefixed.groups.contains(&"tags_webserver".into()));
    assert!(prefixed.groups.contains(&"device_roles_web".into()));
    let plain = host_from_json(
        &device,
        false,
        &["tags".into(), "device_roles".into()],
        true,
        false,
    )
    .unwrap();
    assert_eq!(plain.groups, vec!["webserver", "web"]);
    let ini = render_ini(&[plain]);
    assert!(ini.contains("[webserver]\nweb-1\n"));
    assert!(ini.contains("[web]\nweb-1\n"));
}

#[test]
fn detects_group_name_collisions_between_keys() {
    let device = json!({
        "id": 31,
        "name": "web-1",
        "tags": [{"slug": "web"}],
        "role": {"slug": "web"}
    });
    let host = host_from_json(
        &device,
        false,
        &["tags".into(), "device_roles".into()],
        true,
        false,
    )
    .unwrap();
    assert_eq!(host.groups, vec!["web"]);
    assert_eq!(collisions(&[host]), vec!["web (device_roles, tags)"]);
}

#[test]
fn renders_config_context_as_yaml_and_parses_it_back() {
    let device = json!({
        "id": 32,
        "name": "web-1",
        "primary_ip4": {"address": "10.0.0.8/24"},
        "tags": [{"slug": "webserver"}],
        "config_context": {
            "http_port": 8080,
            "ntp": {"server": "pool.ntp.org"}
        }
    });
    let host = host_from_json(&device, false, &["tags".into()], false, true).unwrap();
    let body = render_yaml(&[host]);
    assert!(body.starts_with("all:\n  hosts:\n"));
    assert!(!body.contains("[tags_webserver]"));
    assert!(body.contains("      http_port: 8080\n"), "{body}");
    assert!(
        body.contains("      ntp:\n        server: !unsafe pool.ntp.org\n"),
        "{body}"
    );
    let parsed = parse_hosts(&body);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "web-1");
    assert_eq!(
        parsed[0].vars.get("ansible_host").map(String::as_str),
        Some("10.0.0.8")
    );
    assert_eq!(
        parsed[0].vars.get("http_port").map(String::as_str),
        Some("8080")
    );
    assert_eq!(
        parsed[0].vars.get("ntp.server").map(String::as_str),
        Some("pool.ntp.org")
    );
    assert_eq!(parsed[0].groups, vec!["all", "tags_webserver"]);
}

#[test]
fn keeps_a_templated_config_context_value_as_an_unsafe_string() {
    let device = json!({
        "id": 33,
        "name": "web-1",
        "config_context": {
            "motd": "{{ lookup('pipe', 'id') }}",
            "ntp": {"server": "{% for x in [] %}{% endfor %}"}
        }
    });
    let host = host_from_json(&device, false, &[], false, true).unwrap();
    let body = render_yaml(&[host]);
    assert!(
        body.contains(r#"motd: !unsafe "{{ lookup('pipe', 'id') }}""#),
        "{body}"
    );
    assert!(
        body.contains(r#"server: !unsafe "{% for x in [] %}{% endfor %}""#),
        "{body}"
    );
    let parsed = parse_hosts(&body);
    assert_eq!(
        parsed[0].vars.get("motd").map(String::as_str),
        Some("{{ lookup('pipe', 'id') }}")
    );
    assert_eq!(
        parsed[0].vars.get("ntp.server").map(String::as_str),
        Some("{% for x in [] %}{% endfor %}")
    );
}

#[test]
fn drops_and_reports_ansible_keys_from_config_context() {
    let device = json!({
        "id": 34,
        "name": "web-1",
        "config_context": {
            "ansible_connection": "local",
            "ansible_ssh_common_args": "-o ProxyCommand=curl evil",
            "ansible_python_interpreter": "/tmp/evil",
            "http_port": 8080
        }
    });
    let host = host_from_json(&device, false, &[], false, true).unwrap();
    assert_eq!(
        host.dropped_vars,
        vec![
            "ansible_connection".to_string(),
            "ansible_python_interpreter".to_string(),
            "ansible_ssh_common_args".to_string(),
        ]
    );
    assert_eq!(host.vars.len(), 1);
    assert!(host.vars.contains_key("http_port"));
    let body = render_yaml(&[host]);
    assert!(!body.contains("ansible_connection"), "{body}");
    assert!(!body.contains("ansible_ssh_common_args"), "{body}");
    assert!(!body.contains("ansible_python_interpreter"), "{body}");
    assert!(body.contains("http_port: 8080"), "{body}");
}

#[tokio::test]
async fn reports_dropped_config_context_keys_in_the_result() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/api/dcim/devices/",
        get(|| async {
            axum::Json(json!({
                "next": null,
                "results": [{
                    "id": 1,
                    "name": "web-1",
                    "config_context": {
                        "ansible_connection": "local",
                        "ansible_ssh_common_args": "-o ProxyCommand=curl evil"
                    }
                }]
            }))
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let source = NetboxSource {
        api_endpoint: format!("http://{addr}"),
        validate_certs: true,
        group_by: Vec::new(),
        group_names_raw: false,
        load_config_context: true,
        max_hosts: DEFAULT_MAX_HOSTS,
        query: Vec::new(),
        virtual_machines: false,
    };
    let proxy = crate::features::operations::ProxySettings::inherit();
    let result = fetch(&source, "token", &proxy).await.unwrap();
    assert_eq!(
        result.dropped_vars,
        vec![
            "ansible_connection".to_string(),
            "ansible_ssh_common_args".to_string(),
        ]
    );
    assert!(
        !result.body.contains("ansible_connection"),
        "{}",
        result.body
    );
    assert!(!result.body.contains("ProxyCommand"), "{}", result.body);
}

#[test]
fn quotes_yaml_float_words_and_unsafe_strings() {
    let device = json!({
        "id": 35,
        "name": "web-1",
        "config_context": {
            "low": "-.inf",
            "nan": ".nan",
            "safe": "pool.ntp.org"
        }
    });
    let host = host_from_json(&device, false, &[], false, true).unwrap();
    let body = render_yaml(&[host]);
    assert!(body.contains("low: !unsafe \"-.inf\""), "{body}");
    assert!(body.contains("nan: !unsafe \".nan\""), "{body}");
    assert!(body.contains("safe: !unsafe pool.ntp.org"), "{body}");
    let parsed = parse_hosts(&body);
    assert_eq!(parsed[0].vars.get("low").map(String::as_str), Some("-.inf"));
    assert_eq!(parsed[0].vars.get("nan").map(String::as_str), Some(".nan"));
}

#[tokio::test]
async fn stops_at_the_configurable_host_limit() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/api/dcim/devices/",
        get(|| async {
            axum::Json(json!({
                "next": null,
                "results": [
                    {"id": 1, "name": "web-1"},
                    {"id": 2, "name": "web-2"},
                    {"id": 3, "name": "web-3"}
                ]
            }))
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let source = NetboxSource {
        api_endpoint: format!("http://{addr}"),
        validate_certs: true,
        group_by: Vec::new(),
        group_names_raw: false,
        load_config_context: false,
        max_hosts: 2,
        query: Vec::new(),
        virtual_machines: false,
    };
    let proxy = crate::features::operations::ProxySettings::inherit();
    let error = fetch(&source, "token", &proxy).await.unwrap_err();
    assert!(
        error.starts_with("the_netbox_host_limit_of_was_reached_raise_max_hosts"),
        "{error}"
    );
    assert!(error.contains("\"limit\":\"2\""), "{error}");
    assert!(error.contains("max_hosts"), "{error}");
}

#[tokio::test]
async fn stops_paging_on_a_short_page() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/api/dcim/devices/",
        get(|| async {
            axum::Json(json!({
                "next": "http://127.0.0.1:9/api/dcim/devices/?limit=100",
                "results": [{"id": 1, "name": "web-1"}]
            }))
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let source = NetboxSource {
        api_endpoint: format!("http://{addr}"),
        validate_certs: true,
        group_by: Vec::new(),
        group_names_raw: false,
        load_config_context: false,
        max_hosts: DEFAULT_MAX_HOSTS,
        query: Vec::new(),
        virtual_machines: false,
    };
    let proxy = crate::features::operations::ProxySettings::inherit();
    let ini = fetch_ini(&source, "token", &proxy).await.unwrap();
    assert!(ini.contains("web-1"), "{ini}");
}

#[tokio::test]
async fn stops_paging_on_a_repeated_page() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let next = format!("{base}/api/dcim/devices/?limit=100");
    let app = Router::new().route(
        "/api/dcim/devices/",
        get(move || {
            let next = next.clone();
            async move {
                let results: Vec<Value> = (0..PAGE_SIZE)
                    .map(|id| json!({"id": id, "name": format!("web-{id}")}))
                    .collect();
                axum::Json(json!({"next": next, "results": results}))
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let source = NetboxSource {
        api_endpoint: base,
        validate_certs: true,
        group_by: Vec::new(),
        group_names_raw: false,
        load_config_context: false,
        max_hosts: DEFAULT_MAX_HOSTS,
        query: Vec::new(),
        virtual_machines: false,
    };
    let proxy = crate::features::operations::ProxySettings::inherit();
    let ini = fetch_ini(&source, "token", &proxy).await.unwrap();
    assert!(ini.contains("[all]\nweb-0"), "{ini}");
    assert!(ini.contains("web-99"), "{ini}");
}

#[test]
fn keeps_hyphens_and_dots_in_a_netbox_host_name() {
    let device = json!({ "name": "web-1.example" });
    let host = host_from_json(&device, false, &[], false, false).unwrap();
    assert_eq!(host.name, "web-1.example");
}

#[test]
fn drops_a_group_slug_that_has_no_letters_or_digits() {
    let device = json!({
        "name": "web-1",
        "tags": [{"slug": "---"}, {"slug": "..."}, {"slug": "-_-"}]
    });
    let host = host_from_json(&device, false, &["tags".into()], false, false).unwrap();
    assert_eq!(host.name, "web-1");
    assert!(host.groups.is_empty(), "{:?}", host.groups);
}

#[test]
fn maps_netbox_http_statuses_to_clear_messages() {
    let forbidden = status_error(reqwest::StatusCode::FORBIDDEN, "denied");
    assert!(
        forbidden.starts_with("netbox.token_rejected"),
        "{forbidden}"
    );
    assert!(forbidden.contains("\"status\":\"403\""), "{forbidden}");
    let unauthorized = status_error(reqwest::StatusCode::UNAUTHORIZED, "");
    assert!(
        unauthorized.starts_with("netbox.token_rejected"),
        "{unauthorized}"
    );
    let missing = status_error(reqwest::StatusCode::NOT_FOUND, "");
    assert!(missing.starts_with("netbox.address_not_found"), "{missing}");
    assert!(missing.contains("\"status\":\"404\""), "{missing}");
    let other = status_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "boom");
    assert!(other.starts_with("netbox.bad_status"), "{other}");
    assert!(other.contains("\"status\":\"500\""), "{other}");
    assert!(other.contains("boom"), "{other}");
}

#[tokio::test]
async fn fetches_devices_and_virtual_machines() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/api/dcim/devices/", get(devices))
        .route("/api/virtualization/virtual-machines/", get(vms));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let source = NetboxSource {
        api_endpoint: format!("http://{addr}"),
        validate_certs: true,
        group_by: vec!["sites".into()],
        group_names_raw: false,
        load_config_context: false,
        max_hosts: DEFAULT_MAX_HOSTS,
        query: vec![("status".into(), "active".into())],
        virtual_machines: true,
    };
    let proxy = crate::features::operations::ProxySettings::inherit();
    let ini = fetch_ini(&source, "secret-token", &proxy).await.unwrap();
    assert!(ini.contains("web-1 ansible_host=10.0.0.8"));
    assert!(ini.contains("web-2"));
    assert!(ini.contains("vm-1 ansible_host=10.1.0.4"));
    assert!(ini.contains("[sites_berlin]"));
    let denied = fetch_ini(&source, "wrong-token", &proxy).await.unwrap_err();
    assert!(denied.contains("403"), "{denied}");
}

#[tokio::test]
async fn an_unreachable_netbox_gives_up_before_the_total_timeout() {
    let source = NetboxSource {
        api_endpoint: "http://192.0.2.1:8080".into(),
        validate_certs: true,
        group_by: Vec::new(),
        group_names_raw: false,
        load_config_context: false,
        max_hosts: DEFAULT_MAX_HOSTS,
        query: Vec::new(),
        virtual_machines: false,
    };
    let proxy = crate::features::operations::ProxySettings::inherit();
    let started = std::time::Instant::now();
    let error = fetch_ini(&source, "token", &proxy).await.unwrap_err();
    assert!(error.contains("netbox_is_not_reachable"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "elapsed {:?}",
        started.elapsed()
    );
}

async fn devices(headers: HeaderMap, request: Request) -> axum::response::Response {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Token secret-token")
    {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    let query = request.uri().query().unwrap_or("");
    if !query.contains("status=active") {
        return (StatusCode::BAD_REQUEST, "filter").into_response();
    }
    axum::Json(json!({
        "next": null,
        "results": [
            {
                "id": 1,
                "name": "web-1",
                "primary_ip4": {"address": "10.0.0.8/24"},
                "site": {"slug": "berlin"}
            },
            {
                "id": 2,
                "name": "web-2",
                "primary_ip4": {"address": "10.0.0.9/24"},
                "site": {"slug": "berlin"}
            }
        ]
    }))
    .into_response()
}

async fn vms(headers: HeaderMap) -> axum::response::Response {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Token secret-token")
    {
        return (StatusCode::FORBIDDEN, "no").into_response();
    }
    axum::Json(json!({
        "next": null,
        "results": [{
            "id": 9,
            "name": "vm-1",
            "primary_ip4": {"address": "10.1.0.4/24"},
            "site": {"slug": "berlin"}
        }]
    }))
    .into_response()
}

#[test]
fn demo_project_lists_the_playbook_and_the_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo");
    let found = scan_project(&root).unwrap();
    assert_eq!(found.playbooks, vec!["site.yml".to_string()]);
    assert!(found
        .inventories
        .iter()
        .any(|item| item.path == "inventory/hosts.ini" && item.format == "ini"));
    assert!(found.playbooks.iter().all(|path| !path.contains("tasks/")));
}

#[test]
#[ignore = "needs ansible-playbook"]
fn demo_playbook_keeps_role_defaults_and_accepts_alert_vars() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work = std::env::temp_dir().join(format!("koreograf-demo-{nanos}"));
    fs::create_dir_all(&work).unwrap();
    let missing = run_demo_playbook(&root, &work, &[]);
    assert!(missing.contains("alertname: manual"), "{missing}");
    assert!(missing.contains("instance: localhost"), "{missing}");
    assert!(missing.contains("action: noop"), "{missing}");
    let injected = run_demo_playbook(
        &root,
        &work,
        &[
            "alertname=DiskFull",
            "instance=web-1",
            "heal_action=cleanup-tmp",
        ],
    );
    assert!(injected.contains("alertname: DiskFull"), "{injected}");
    assert!(injected.contains("instance: web-1"), "{injected}");
    assert!(injected.contains("action: cleanup-tmp"), "{injected}");
    let _ = fs::remove_dir_all(work);
}

fn run_demo_playbook(root: &Path, work: &Path, extra: &[&str]) -> String {
    let mut cmd = std::process::Command::new("ansible-playbook");
    cmd.arg("-i")
        .arg("localhost,")
        .arg("-c")
        .arg("local")
        .arg("site.yml")
        .arg("-e")
        .arg(format!("koreograf_work={}", work.display()))
        .current_dir(root);
    for item in extra {
        cmd.arg("-e").arg(*item);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_to_string(work.join("incident-report.txt")).unwrap()
}

#[test]
fn classifies_a_netbox_plugin_file() {
    let text = "\
plugin: netbox.netbox.nb_inventory
api_endpoint: https://netbox.example.com
validate_certs: false
group_by:
  - sites
  - device_roles
query_filters:
  - status: active
";
    let found = classify_inventory("netbox.yml", text).unwrap();
    assert_eq!(found.format, "plugin");
    assert_eq!(found.plugin.as_deref(), Some("netbox.netbox.nb_inventory"));
    assert_eq!(
        found.api_endpoint.as_deref(),
        Some("https://netbox.example.com")
    );
    assert_eq!(
        found.group_by,
        vec!["sites".to_string(), "device_roles".to_string()]
    );
    assert_eq!(found.query_filters, vec!["status=active".to_string()]);
    assert_eq!(found.validate_certs, Some(false));
}
