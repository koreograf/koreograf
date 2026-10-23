use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

const MAX_PAGES: usize = 20;
const MAX_HOSTS: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetboxSource {
    pub api_endpoint: String,
    pub validate_certs: bool,
    pub group_by: Vec<String>,
    pub query: Vec<(String, String)>,
    pub virtual_machines: bool,
}

#[derive(Clone, Debug)]
struct NbHost {
    name: String,
    ansible_host: Option<String>,
    netbox_id: Option<i64>,
    groups: Vec<String>,
}

pub fn valid_endpoint(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() < 300
        && !url.contains("..")
        && !url.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}

pub fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() < 300
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

pub fn parse_plugin(text: &str) -> Option<NetboxSource> {
    let plugin = scalar(text, "plugin")?;
    if plugin != "netbox.netbox.nb_inventory" && plugin != "netbox" {
        return None;
    }
    let api_endpoint = scalar(text, "api_endpoint").unwrap_or_default();
    Some(NetboxSource {
        api_endpoint: api_endpoint.trim_end_matches('/').to_string(),
        validate_certs: flag(text, "validate_certs").unwrap_or(true),
        group_by: list_items(text, "group_by"),
        query: query_items(text, "query_filters"),
        virtual_machines: flag(text, "virtual_machines").unwrap_or(true),
    })
}

pub fn parse_filters(lines: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut query = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .or_else(|| line.split_once(':'))
            .ok_or_else(|| {
                crate::error::message("filter_line_needs_key_value", &[("line", line)])
            })?;
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() || value.is_empty() || key.contains(' ') {
            return Err(crate::error::message(
                "filter_line_is_invalid",
                &[("line", line)],
            ));
        }
        query.push((key.to_string(), value.to_string()));
    }
    Ok(query)
}

pub async fn fetch_ini(
    source: &NetboxSource,
    token: &str,
    proxy: &crate::settings::ProxySettings,
) -> Result<String, String> {
    if !valid_endpoint(&source.api_endpoint) {
        return Err("the_netbox_address_must_be_http_or_https".into());
    }
    if !valid_token(token) {
        return Err("the_netbox_token_is_invalid".into());
    }
    for key in &source.group_by {
        if !known_group(key) {
            return Err(crate::error::message(
                "grouping_key_is_not_supported_allowed_values_are",
                &[("key", key)],
            ));
        }
    }
    let client = proxy
        .apply_reqwest(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .danger_accept_invalid_certs(!source.validate_certs),
        )?
        .build()
        .map_err(|err| err.to_string())?;
    let mut hosts = Vec::new();
    let devices = PageQuery {
        client: &client,
        endpoint: &source.api_endpoint,
        token,
        path: "/api/dcim/devices/",
        query: &source.query,
        is_virtual: false,
        group_by: &source.group_by,
    };
    collect(&devices, &mut hosts).await?;
    if source.virtual_machines {
        let machines = PageQuery {
            path: "/api/virtualization/virtual-machines/",
            is_virtual: true,
            ..devices
        };
        collect(&machines, &mut hosts).await?;
    }
    Ok(render_ini(&hosts))
}

fn known_group(key: &str) -> bool {
    matches!(
        key,
        "sites"
            | "site"
            | "location"
            | "tenants"
            | "tenant"
            | "tags"
            | "tag"
            | "device_roles"
            | "role"
            | "platforms"
            | "platform"
            | "manufacturers"
            | "manufacturer"
            | "device_types"
            | "device_type"
            | "status"
            | "cluster"
            | "region"
            | "site_group"
            | "is_virtual"
    )
}

struct PageQuery<'a> {
    client: &'a reqwest::Client,
    endpoint: &'a str,
    token: &'a str,
    path: &'a str,
    query: &'a [(String, String)],
    is_virtual: bool,
    group_by: &'a [String],
}

async fn collect(page: &PageQuery<'_>, hosts: &mut Vec<NbHost>) -> Result<(), String> {
    let mut url = Some(page_url(page.endpoint, page.path, page.query));
    let mut pages = 0usize;
    while let Some(current) = url.take() {
        pages += 1;
        if pages > MAX_PAGES || hosts.len() >= MAX_HOSTS {
            return Err("the_netbox_response_is_too_large_limit_20".into());
        }
        let body = get_json(page.client, &current, page.token).await?;
        let results = body
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| "the_netbox_response_has_no_results".to_string())?;
        for item in results {
            if hosts.len() >= MAX_HOSTS {
                break;
            }
            if let Some(host) = host_from_json(item, page.is_virtual, page.group_by) {
                hosts.push(host);
            }
        }
        url = match body.get("next") {
            Some(Value::String(next)) if !next.is_empty() => {
                if origin(page.endpoint) != origin(next) {
                    return Err("the_netbox_response_points_at_another_host".into());
                }
                Some(next.clone())
            }
            _ => None,
        };
    }
    Ok(())
}

fn host_from_json(value: &Value, is_virtual: bool, group_by: &[String]) -> Option<NbHost> {
    let name = sanitize(value.get("name")?.as_str()?);
    if name.is_empty() {
        return None;
    }
    let mut groups = Vec::new();
    for key in group_by {
        groups.extend(group_names(key, value, is_virtual));
    }
    Some(NbHost {
        name,
        ansible_host: primary_ip(value),
        netbox_id: value.get("id").and_then(Value::as_i64),
        groups,
    })
}

fn group_names(key: &str, value: &Value, is_virtual: bool) -> Vec<String> {
    let slugs = match key {
        "sites" | "site" => slugs_of(value.get("site")),
        "location" => slugs_of(value.get("location")),
        "tenants" | "tenant" => slugs_of(value.get("tenant")),
        "tags" | "tag" => slugs_of(value.get("tags")),
        "device_roles" | "role" => slugs_of(value.get("role").or_else(|| value.get("device_role"))),
        "platforms" | "platform" => slugs_of(value.get("platform")),
        "manufacturers" | "manufacturer" => slugs_of(
            value
                .get("device_type")
                .and_then(|item| item.get("manufacturer"))
                .or_else(|| value.get("manufacturer")),
        ),
        "device_types" | "device_type" => slugs_of(value.get("device_type")),
        "status" => status_slug(value.get("status")),
        "cluster" => slugs_of(value.get("cluster")),
        "region" => slugs_of(
            value
                .get("site")
                .and_then(|item| item.get("region"))
                .or_else(|| value.get("region")),
        ),
        "site_group" => slugs_of(
            value
                .get("site")
                .and_then(|item| item.get("group"))
                .or_else(|| value.get("group")),
        ),
        "is_virtual" => vec![if is_virtual { "true" } else { "false" }.to_string()],
        _ => Vec::new(),
    };
    slugs
        .into_iter()
        .filter_map(|slug| {
            let slug = sanitize_group(&slug);
            if slug.is_empty() {
                None
            } else {
                Some(format!("{key}_{slug}"))
            }
        })
        .collect()
}

fn slugs_of(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(slug_of).collect(),
        Some(other) => slug_of(other).into_iter().collect(),
        None => Vec::new(),
    }
}

fn slug_of(value: &Value) -> Option<String> {
    value
        .get("slug")
        .and_then(Value::as_str)
        .or_else(|| value.get("name").and_then(Value::as_str))
        .map(str::to_string)
}

fn status_slug(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(text) = value.as_str() {
        return vec![text.to_string()];
    }
    value
        .get("value")
        .and_then(Value::as_str)
        .map(|item| vec![item.to_string()])
        .unwrap_or_default()
}

fn primary_ip(value: &Value) -> Option<String> {
    // NetBox always sends `primary_ip4`. JSON null means there is no IPv4
    // primary, so a missing or empty address must not stop the search.
    ["primary_ip4", "primary_ip", "primary_ip6"]
        .into_iter()
        .find_map(|key| address_host(value.get(key)))
}

fn address_host(value: Option<&Value>) -> Option<String> {
    let address = value?.get("address")?.as_str()?;
    let host = address.split('/').next().unwrap_or(address).trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn render_ini(hosts: &[NbHost]) -> String {
    let mut out = String::from("[all]\n");
    for host in hosts {
        out.push_str(&host.name);
        if let Some(address) = &host.ansible_host {
            out.push_str(" ansible_host=");
            out.push_str(address);
        }
        if let Some(id) = host.netbox_id {
            out.push_str(&format!(" netbox_id={id}"));
        }
        out.push('\n');
    }
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for host in hosts {
        for group in &host.groups {
            grouped.entry(group).or_default().push(&host.name);
        }
    }
    for (group, names) in grouped {
        out.push('\n');
        out.push('[');
        out.push_str(group);
        out.push_str("]\n");
        for name in names {
            out.push_str(name);
            out.push('\n');
        }
    }
    out
}

fn sanitize(value: &str) -> String {
    sanitize_with(value, |ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-')
    })
}

// Ansible warns when a group name contains `-` or `.`. Host names keep both.
fn sanitize_group(value: &str) -> String {
    sanitize_with(value, |ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn sanitize_with(value: &str, keep: impl Fn(char) -> bool) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if keep(ch) {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

fn page_url(endpoint: &str, path: &str, query: &[(String, String)]) -> String {
    let mut parts = vec!["limit=100".to_string()];
    for (key, value) in query {
        parts.push(format!("{}={}", encode(key), encode(value)));
    }
    format!(
        "{}{}?{}",
        endpoint.trim_end_matches('/'),
        path,
        parts.join("&")
    )
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() {
        None
    } else {
        Some(format!("{scheme}://{host}"))
    }
}

async fn get_json(client: &reqwest::Client, url: &str, token: &str) -> Result<Value, String> {
    let response = client
        .get(url)
        .header("Authorization", format!("Token {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|err| {
            let detail = err.to_string();
            crate::error::message("netbox_is_not_reachable_err", &[("err", &detail)])
        })?;
    let status = response.status();
    let text = response.text().await.map_err(|err| {
        let detail = err.to_string();
        crate::error::message("netbox.response_unreadable", &[("detail", &detail)])
    })?;
    if !status.is_success() {
        let snippet: String = text.chars().take(240).collect();
        let code = status.as_u16().to_string();
        return Err(crate::error::message(
            "netbox.bad_status",
            &[("status", &code), ("snippet", &snippet)],
        ));
    }
    serde_json::from_str(&text).map_err(|_| "the_netbox_response_is_not_json".to_string())
}

fn scalar(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn flag(text: &str, key: &str) -> Option<bool> {
    match scalar(text, key)?.to_ascii_lowercase().as_str() {
        "true" | "yes" | "ja" => Some(true),
        "false" | "no" | "nein" => Some(false),
        _ => None,
    }
}

fn list_items(text: &str, key: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with(&format!("{key}:")) {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if raw.trim_start().len() == raw.len() && line.ends_with(':') {
            break;
        }
        if let Some(value) = line.strip_prefix("- ") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() && !value.contains(':') {
                items.push(value.to_string());
            }
        }
    }
    items
}

fn query_items(text: &str, key: &str) -> Vec<(String, String)> {
    let mut items = Vec::new();
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with(&format!("{key}:")) {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if raw.trim_start().len() == raw.len() && line.ends_with(':') {
            break;
        }
        let Some(value) = line.strip_prefix("- ") else {
            continue;
        };
        if let Some((left, right)) = value.split_once(':').or_else(|| value.split_once('=')) {
            let left = left.trim();
            let right = right.trim().trim_matches('"').trim_matches('\'');
            if !left.is_empty() && !right.is_empty() {
                items.push((left.to_string(), right.to_string()));
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use serde_json::json;

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
        let host = host_from_json(&device, false, &[]).unwrap();
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
        let host = host_from_json(&device, false, &[]).unwrap();
        assert_eq!(host.ansible_host.as_deref(), Some("2001:db8::8"));
        let ini = render_ini(&[host]);
        assert!(ini.contains("v6-only ansible_host=2001:db8::8 netbox_id=8\n"));
        let listed = crate::inventory::parse_hosts(&ini);
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
        let host = host_from_json(&device, false, &[]).unwrap();
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
        let host = host_from_json(&device, false, &[]).unwrap();
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
        let host = host_from_json(&device, false, &[]).unwrap();
        assert_eq!(host.ansible_host.as_deref(), Some("10.0.0.8"));
        let ini = render_ini(&[host]);
        assert!(ini.contains("dual ansible_host=10.0.0.8 netbox_id=11\n"));
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
        let host = host_from_json(&device, false, &[]).unwrap();
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
        let host = host_from_json(&device, false, &["tags".into()]).unwrap();
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
    fn keeps_hyphens_and_dots_in_a_netbox_host_name() {
        let device = json!({ "name": "web-1.example" });
        let host = host_from_json(&device, false, &[]).unwrap();
        assert_eq!(host.name, "web-1.example");
    }

    #[test]
    fn drops_a_group_slug_that_has_no_letters_or_digits() {
        let device = json!({
            "name": "web-1",
            "tags": [{"slug": "---"}, {"slug": "..."}, {"slug": "-_-"}]
        });
        let host = host_from_json(&device, false, &["tags".into()]).unwrap();
        assert_eq!(host.name, "web-1");
        assert!(host.groups.is_empty(), "{:?}", host.groups);
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
            query: vec![("status".into(), "active".into())],
            virtual_machines: true,
        };
        let proxy = crate::settings::ProxySettings::inherit();
        let ini = fetch_ini(&source, "secret-token", &proxy).await.unwrap();
        assert!(ini.contains("web-1 ansible_host=10.0.0.8"));
        assert!(ini.contains("web-2"));
        assert!(ini.contains("vm-1 ansible_host=10.1.0.4"));
        assert!(ini.contains("[sites_berlin]"));
        let denied = fetch_ini(&source, "wrong-token", &proxy).await.unwrap_err();
        assert!(denied.contains("403"), "{denied}");
    }

    async fn devices(headers: HeaderMap, request: Request) -> axum::response::Response {
        if headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            != Some("Token secret-token")
        {
            return (StatusCode::FORBIDDEN, "no").into_response();
        }
        let host = headers
            .get("host")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("127.0.0.1");
        let query = request.uri().query().unwrap_or("");
        if !query.contains("status=active") {
            return (StatusCode::BAD_REQUEST, "filter").into_response();
        }
        if query.contains("offset=1") {
            return axum::Json(json!({
                "next": null,
                "results": [{
                    "id": 2,
                    "name": "web-2",
                    "primary_ip4": {"address": "10.0.0.9/24"},
                    "site": {"slug": "berlin"}
                }]
            }))
            .into_response();
        }
        axum::Json(json!({
            "next": format!("http://{host}/api/dcim/devices/?limit=1&offset=1&status=active"),
            "results": [{
                "id": 1,
                "name": "web-1",
                "primary_ip4": {"address": "10.0.0.8/24"},
                "site": {"slug": "berlin"}
            }]
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
}
