use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use serde_json::{json, Value};

pub const DEFAULT_MAX_HOSTS: usize = 2_000;
pub const MAX_HOSTS_LIMIT: usize = 10_000;
const PAGE_SIZE: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetboxSource {
    pub api_endpoint: String,
    pub validate_certs: bool,
    pub ca_file: String,
    pub group_by: Vec<String>,
    pub group_names_raw: bool,
    pub load_config_context: bool,
    pub max_hosts: usize,
    pub query: Vec<(String, String)>,
    pub virtual_machines: bool,
}

#[derive(Clone, Debug)]
pub struct NetboxResult {
    pub body: String,
    pub collisions: Vec<String>,
    pub dropped_vars: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NetboxPreview {
    pub host_count: usize,
    pub groups: Vec<(String, usize)>,
    pub sample_host: Option<(String, BTreeMap<String, String>)>,
}

#[derive(Clone, Debug)]
struct NbHost {
    name: String,
    ansible_host: Option<String>,
    netbox_id: Option<i64>,
    groups: Vec<String>,
    group_keys: Vec<(String, String)>,
    vars: BTreeMap<String, Value>,
    dropped_vars: Vec<String>,
}

pub fn valid_endpoint(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() < 300
        && !url.contains("..")
        && !url.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}

pub fn source_json(source: &NetboxSource) -> Value {
    json!({
        "validate_certs": source.validate_certs,
        "ca_file": source.ca_file,
        "group_by": source.group_by,
        "group_names_raw": source.group_names_raw,
        "load_config_context": source.load_config_context,
        "max_hosts": source.max_hosts,
        "query": source.query,
        "virtual_machines": source.virtual_machines,
    })
}

pub fn source_json_with_collisions(source: &NetboxSource, collisions: &[String]) -> Value {
    let mut value = source_json(source);
    value["collisions"] = json!(collisions);
    value
}

pub fn loads_config_context(source_json: &str) -> bool {
    serde_json::from_str::<Value>(source_json)
        .ok()
        .and_then(|value| value.get("load_config_context").and_then(Value::as_bool))
        == Some(true)
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
        ca_file: String::new(),
        group_by: list_items(text, "group_by"),
        group_names_raw: flag(text, "group_names_raw").unwrap_or(false),
        load_config_context: flag(text, "load_config_context").unwrap_or(false),
        max_hosts: scalar(text, "max_hosts")
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(DEFAULT_MAX_HOSTS),
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

pub async fn fetch(
    source: &NetboxSource,
    token: &str,
    proxy: &crate::settings::ProxySettings,
) -> Result<NetboxResult, String> {
    if !valid_endpoint(&source.api_endpoint) {
        return Err("the_netbox_address_must_be_http_or_https".into());
    }
    if !valid_token(token) {
        return Err("the_netbox_token_is_invalid".into());
    }
    if source.max_hosts == 0 || source.max_hosts > MAX_HOSTS_LIMIT {
        let max = MAX_HOSTS_LIMIT.to_string();
        return Err(crate::error::message(
            "netbox_host_limit_is_out_of_range",
            &[("max", &max)],
        ));
    }
    for key in &source.group_by {
        if !known_group(key) {
            return Err(crate::error::message(
                "grouping_key_is_not_supported_allowed_values_are",
                &[("key", key)],
            ));
        }
    }
    let client = client(source, proxy)?;
    let mut hosts = Vec::new();
    let devices = PageQuery {
        client: &client,
        endpoint: &source.api_endpoint,
        token,
        path: "/api/dcim/devices/",
        query: &source.query,
        is_virtual: false,
        group_by: &source.group_by,
        group_names_raw: source.group_names_raw,
        load_config_context: source.load_config_context,
        max_hosts: source.max_hosts,
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
    let collisions = collisions(&hosts);
    let mut dropped_vars: Vec<String> = hosts
        .iter()
        .flat_map(|host| host.dropped_vars.iter().cloned())
        .collect();
    dropped_vars.sort();
    dropped_vars.dedup();
    let body = if source.load_config_context {
        render_yaml(&hosts)
    } else {
        render_ini(&hosts)
    };
    Ok(NetboxResult {
        body,
        collisions,
        dropped_vars,
    })
}

#[cfg(test)]
pub async fn fetch_ini(
    source: &NetboxSource,
    token: &str,
    proxy: &crate::settings::ProxySettings,
) -> Result<String, String> {
    Ok(fetch(source, token, proxy).await?.body)
}

pub fn preview(body: &str) -> NetboxPreview {
    let data = crate::inventory::parse_inventory(body);
    let mut groups = BTreeMap::new();
    for host in &data.hosts {
        for group in &host.groups {
            if group != "all" && group != "ungrouped" {
                *groups.entry(group.clone()).or_insert(0usize) += 1;
            }
        }
    }
    let sample_host = data
        .hosts
        .first()
        .map(|host| (host.name.clone(), host.vars.clone()));
    NetboxPreview {
        host_count: data.hosts.len(),
        groups: groups.into_iter().collect(),
        sample_host,
    }
}

pub async fn test_connection(
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
    crate::tls::warn_netbox(
        &source.api_endpoint,
        !source.validate_certs,
        &source.ca_file,
    );
    let client = client(source, proxy)?;
    let url = format!("{}/api/status/", source.api_endpoint.trim_end_matches('/'));
    let body = get_json(&client, &url, token).await?;
    Ok(body
        .get("netbox-version")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn client(
    source: &NetboxSource,
    proxy: &crate::settings::ProxySettings,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none());
    match crate::tls::reqwest_roots(&source.ca_file) {
        Ok(Some(certs)) => {
            for cert in certs {
                builder = builder.add_root_certificate(cert);
            }
        }
        Ok(None) => {
            builder = builder.danger_accept_invalid_certs(!source.validate_certs);
        }
        Err(error) => {
            tracing::warn!(endpoint = %source.api_endpoint, %error, "NetBox TLS setup failed");
            return Err(error);
        }
    }
    proxy
        .apply_reqwest(builder)?
        .build()
        .map_err(|err| err.to_string())
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
    group_names_raw: bool,
    load_config_context: bool,
    max_hosts: usize,
}

async fn collect(page: &PageQuery<'_>, hosts: &mut Vec<NbHost>) -> Result<(), String> {
    let mut url = Some(page_url(page.endpoint, page.path, page.query));
    let mut seen = std::collections::BTreeSet::new();
    let mut pages = 0usize;
    let max_pages = page.max_hosts.div_ceil(PAGE_SIZE).max(1);
    while let Some(current) = url.take() {
        pages += 1;
        if pages > max_pages {
            return Err(limit_error(page.max_hosts));
        }
        if !seen.insert(current.clone()) {
            break;
        }
        let body = get_json(page.client, &current, page.token).await?;
        let results = body
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| "the_netbox_response_has_no_results".to_string())?;
        for item in results {
            if hosts.len() >= page.max_hosts {
                return Err(limit_error(page.max_hosts));
            }
            if let Some(host) = host_from_json(
                item,
                page.is_virtual,
                page.group_by,
                page.group_names_raw,
                page.load_config_context,
            ) {
                hosts.push(host);
            }
        }
        if results.len() < PAGE_SIZE {
            break;
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

fn limit_error(limit: usize) -> String {
    crate::error::message(
        "the_netbox_host_limit_of_was_reached_raise_max_hosts",
        &[("limit", &limit.to_string())],
    )
}

fn host_from_json(
    value: &Value,
    is_virtual: bool,
    group_by: &[String],
    group_names_raw: bool,
    load_config_context: bool,
) -> Option<NbHost> {
    let name = sanitize(value.get("name")?.as_str()?);
    if name.is_empty() {
        return None;
    }
    let mut groups = Vec::new();
    let mut group_keys = Vec::new();
    for key in group_by {
        for group in group_names(key, value, is_virtual, group_names_raw) {
            if !groups.contains(&group) {
                groups.push(group.clone());
            }
            group_keys.push((group, key.clone()));
        }
    }
    let mut dropped_vars = Vec::new();
    let vars = if load_config_context {
        value
            .get("config_context")
            .and_then(Value::as_object)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(key, value)| {
                        if config_context_key_allowed(key) {
                            Some((key.clone(), value.clone()))
                        } else {
                            dropped_vars.push(key.clone());
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    Some(NbHost {
        name,
        ansible_host: primary_ip(value),
        netbox_id: value.get("id").and_then(Value::as_i64),
        groups,
        group_keys,
        vars,
        dropped_vars,
    })
}

fn config_context_key_allowed(key: &str) -> bool {
    !key.to_ascii_lowercase().starts_with("ansible_")
}

fn group_names(key: &str, value: &Value, is_virtual: bool, raw: bool) -> Vec<String> {
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
            } else if raw {
                Some(slug)
            } else {
                Some(format!("{key}_{slug}"))
            }
        })
        .collect()
}

fn collisions(hosts: &[NbHost]) -> Vec<String> {
    let mut keys: BTreeMap<&str, std::collections::BTreeSet<&str>> = BTreeMap::new();
    for host in hosts {
        for (group, key) in &host.group_keys {
            keys.entry(group).or_default().insert(key);
        }
    }
    keys.into_iter()
        .filter(|(_, sources)| sources.len() > 1)
        .map(|(group, sources)| {
            format!(
                "{group} ({})",
                sources.into_iter().collect::<Vec<_>>().join(", ")
            )
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
    // A present value that is not an IP address stops the search.
    let device = value.get("name").and_then(Value::as_str).unwrap_or("");
    for key in ["primary_ip4", "primary_ip", "primary_ip6"] {
        match address_host(value.get(key)) {
            HostAddress::Missing => continue,
            HostAddress::Invalid => {
                tracing::warn!(
                    device,
                    "NetBox address is not an IP address, so ansible_host was left unset"
                );
                return None;
            }
            HostAddress::Ip(address) => return Some(address),
        }
    }
    None
}

enum HostAddress {
    Missing,
    Invalid,
    Ip(String),
}

fn address_host(value: Option<&Value>) -> HostAddress {
    let Some(address) = value
        .and_then(|item| item.get("address"))
        .and_then(Value::as_str)
    else {
        return HostAddress::Missing;
    };
    let host = address.split_once('/').map_or(address, |(host, _)| host);
    let host = host.trim();
    if host.is_empty() {
        return HostAddress::Missing;
    }
    match host.parse::<IpAddr>() {
        Ok(address) => HostAddress::Ip(address.to_string()),
        Err(_) => HostAddress::Invalid,
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

fn render_yaml(hosts: &[NbHost]) -> String {
    let mut out = String::from("all:\n  hosts:\n");
    for host in hosts {
        out.push_str("    ");
        out.push_str(&yaml_key(&host.name));
        out.push_str(":\n");
        let mut vars = host.vars.clone();
        if let Some(address) = &host.ansible_host {
            vars.insert("ansible_host".into(), Value::String(address.clone()));
        }
        if let Some(id) = host.netbox_id {
            vars.insert("netbox_id".into(), Value::Number(id.into()));
        }
        write_yaml_vars(&mut out, 6, &vars, &host.vars, false);
    }
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for host in hosts {
        for group in &host.groups {
            grouped.entry(group).or_default().push(&host.name);
        }
    }
    if !grouped.is_empty() {
        out.push_str("  children:\n");
        for (group, names) in grouped {
            out.push_str("    ");
            out.push_str(&yaml_key(group));
            out.push_str(":\n      hosts:\n");
            for name in names {
                out.push_str("        ");
                out.push_str(&yaml_key(name));
                out.push_str(":\n");
            }
        }
    }
    out
}

fn write_yaml_vars(
    out: &mut String,
    indent: usize,
    vars: &BTreeMap<String, Value>,
    config: &BTreeMap<String, Value>,
    nested: bool,
) {
    for (key, value) in vars {
        out.push_str(&" ".repeat(indent));
        out.push_str(&yaml_key(key));
        match value {
            Value::Object(items) if !items.is_empty() => {
                out.push_str(":\n");
                let nested_items = items
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                write_yaml_vars(out, indent + 2, &nested_items, config, true);
            }
            _ => {
                out.push_str(": ");
                let unsafe_tag = nested
                    || config.get(key).is_some_and(|value| {
                        matches!(value, Value::String(_) | Value::Array(_) | Value::Object(_))
                    });
                out.push_str(&yaml_value(value, unsafe_tag));
                out.push('\n');
            }
        }
    }
}

fn yaml_key(value: &str) -> String {
    if plain_yaml(value) {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
    }
}

fn yaml_value(value: &Value, unsafe_tag: bool) -> String {
    match value {
        Value::String(text) if unsafe_tag => format!("!unsafe {}", yaml_key(text)),
        Value::String(text) => yaml_key(text),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "null".into(),
        Value::Array(_) | Value::Object(_) => {
            let text = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
            let quoted = yaml_key(&text);
            if unsafe_tag {
                format!("!unsafe {quoted}")
            } else {
                quoted
            }
        }
    }
}

fn plain_yaml(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "null" | "true" | "false" | "yes" | "no" | "on" | "off" | ".inf" | "-.inf" | ".nan"
        )
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '/' | '-'))
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
        return Err(status_error(status, &snippet));
    }
    serde_json::from_str(&text).map_err(|_| "the_netbox_response_is_not_json".to_string())
}

fn status_error(status: reqwest::StatusCode, snippet: &str) -> String {
    let code = status.as_u16().to_string();
    match status.as_u16() {
        401 | 403 => crate::error::message("netbox.token_rejected", &[("status", &code)]),
        404 => crate::error::message("netbox.address_not_found", &[("status", &code)]),
        _ => crate::error::message(
            "netbox.bad_status",
            &[("status", &code), ("snippet", snippet)],
        ),
    }
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
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use super::*;
    use axum::extract::Request;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use serde_json::json;

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
        let parsed = crate::inventory::parse_hosts(&body);
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
        let parsed = crate::inventory::parse_hosts(&body);
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
            ca_file: String::new(),
            group_by: Vec::new(),
            group_names_raw: false,
            load_config_context: true,
            max_hosts: DEFAULT_MAX_HOSTS,
            query: Vec::new(),
            virtual_machines: false,
        };
        let proxy = crate::settings::ProxySettings::inherit();
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
        let parsed = crate::inventory::parse_hosts(&body);
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
            ca_file: String::new(),
            group_by: Vec::new(),
            group_names_raw: false,
            load_config_context: false,
            max_hosts: 2,
            query: Vec::new(),
            virtual_machines: false,
        };
        let proxy = crate::settings::ProxySettings::inherit();
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
            ca_file: String::new(),
            group_by: Vec::new(),
            group_names_raw: false,
            load_config_context: false,
            max_hosts: DEFAULT_MAX_HOSTS,
            query: Vec::new(),
            virtual_machines: false,
        };
        let proxy = crate::settings::ProxySettings::inherit();
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
            ca_file: String::new(),
            group_by: Vec::new(),
            group_names_raw: false,
            load_config_context: false,
            max_hosts: DEFAULT_MAX_HOSTS,
            query: Vec::new(),
            virtual_machines: false,
        };
        let proxy = crate::settings::ProxySettings::inherit();
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
            ca_file: String::new(),
            group_by: vec!["sites".into()],
            group_names_raw: false,
            load_config_context: false,
            max_hosts: DEFAULT_MAX_HOSTS,
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

    #[tokio::test]
    async fn a_missing_ca_file_fails_closed_when_checks_are_off() {
        let source = NetboxSource {
            api_endpoint: "http://127.0.0.1:9".into(),
            validate_certs: false,
            ca_file: "/no/such/koreograf-ca.pem".into(),
            group_by: Vec::new(),
            group_names_raw: false,
            load_config_context: false,
            max_hosts: DEFAULT_MAX_HOSTS,
            query: Vec::new(),
            virtual_machines: false,
        };
        let proxy = crate::settings::ProxySettings::inherit();
        let error = fetch_ini(&source, "token", &proxy).await.unwrap_err();
        assert_eq!(error, "the_ca_file_cannot_be_read");
    }

    #[tokio::test]
    async fn an_unreachable_netbox_gives_up_before_the_total_timeout() {
        let source = NetboxSource {
            api_endpoint: "http://192.0.2.1:8080".into(),
            validate_certs: true,
            ca_file: String::new(),
            group_by: Vec::new(),
            group_names_raw: false,
            load_config_context: false,
            max_hosts: DEFAULT_MAX_HOSTS,
            query: Vec::new(),
            virtual_machines: false,
        };
        let proxy = crate::settings::ProxySettings::inherit();
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
}
