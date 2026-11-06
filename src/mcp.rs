use std::collections::BTreeMap;
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use reqwest::Method;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::schemars::JsonSchema;
use rmcp::service::RequestContext;
use rmcp::RoleServer;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tower::ServiceExt;

const DEFAULT_URL: &str = "http://127.0.0.1:8080";

#[derive(Debug, Clone)]
pub struct Error {
    pub status: Option<u16>,
    pub key: String,
    pub vars: BTreeMap<String, String>,
}

impl Error {
    fn new(key: impl Into<String>) -> Self {
        Self {
            status: None,
            key: key.into(),
            vars: BTreeMap::new(),
        }
    }

    pub fn text(&self) -> String {
        json!({"error": self.key, "vars": self.vars}).to_string()
    }
}

#[derive(Debug, Clone)]
enum Backend {
    Remote { http: reqwest::Client, base: String },
    Local { app: axum::Router },
}

#[derive(Debug, Clone)]
pub struct Client {
    backend: Backend,
    token: String,
    team_id: Option<String>,
}

fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

impl Client {
    pub fn new(
        base: impl Into<String>,
        token: impl Into<String>,
        team_id: Option<String>,
    ) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| Error::new("http_client_failed"))?;
        let base = base.into().trim_end_matches('/').to_string();
        Ok(Self {
            backend: Backend::Remote { http, base },
            token: token.into(),
            team_id,
        })
    }

    pub fn local(app: axum::Router) -> Self {
        Self {
            backend: Backend::Local { app },
            token: String::new(),
            team_id: None,
        }
    }

    fn with_token(mut self, token: String) -> Self {
        self.token = token;
        self
    }

    fn called_team(&self, team_id: Option<&str>) -> Option<String> {
        team_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| self.team_id.clone())
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        team_id: Option<&str>,
    ) -> Result<String, Error> {
        match &self.backend {
            Backend::Remote { http, base } => {
                let mut request = http
                    .request(method, format!("{base}{path}"))
                    .bearer_auth(&self.token);
                if !query.is_empty() {
                    request = request.query(query);
                }
                if let Some(team) = self.called_team(team_id) {
                    request = request.header("X-Team-Id", team);
                }
                if let Some(body) = body {
                    request = request.json(body);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|_| Error::new("request_failed"))?;
                let status = response.status();
                let text = response
                    .text()
                    .await
                    .map_err(|_| Error::new("invalid_response"))?;
                if status.is_success() {
                    return Ok(text);
                }
                Err(response_error(status.as_u16(), &text))
            }
            Backend::Local { app } => {
                self.local_request(app, method, path, query, body, team_id)
                    .await
            }
        }
    }

    async fn local_request(
        &self,
        app: &axum::Router,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        team_id: Option<&str>,
    ) -> Result<String, Error> {
        let mut target = path.to_string();
        if !query.is_empty() {
            target.push('?');
            target.push_str(
                &query
                    .iter()
                    .map(|(key, value)| format!("{}={}", path_segment(key), path_segment(value)))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri(target)
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", self.token),
            );
        if let Some(team) = self.called_team(team_id) {
            builder = builder.header("X-Team-Id", team);
        }
        let request = if let Some(body) = body {
            builder
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .map_err(|_| Error::new("request_failed"))?
        } else {
            builder
                .body(Body::empty())
                .map_err(|_| Error::new("request_failed"))?
        };
        let response = app
            .clone()
            .oneshot(request)
            .await
            .map_err(|_| Error::new("request_failed"))?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|_| Error::new("invalid_response"))?
            .to_bytes();
        let text = String::from_utf8_lossy(&bytes).to_string();
        if (200..300).contains(&status) {
            return Ok(text);
        }
        Err(response_error(status, &text))
    }

    async fn json(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        team_id: Option<&str>,
    ) -> Result<Value, Error> {
        let text = self.request(method, path, query, body, team_id).await?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| Error::new("invalid_response"))
    }

    pub async fn whoami(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/me", &[], None, team_id).await
    }

    pub async fn version(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/version", &[], None, team_id)
            .await
    }

    pub async fn list_projects(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/projects", &[], None, team_id)
            .await
    }

    pub async fn list_inventories(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/inventories", &[], None, team_id)
            .await
    }

    pub async fn list_credentials(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/credentials", &[], None, team_id)
            .await
    }

    pub async fn list_templates(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/templates", &[], None, team_id)
            .await
    }

    pub async fn list_workflows(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/workflows", &[], None, team_id)
            .await
    }

    pub async fn list_schedules(&self, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(Method::GET, "/api/schedules", &[], None, team_id)
            .await
    }

    pub async fn list_jobs(&self, query: JobQuery<'_>) -> Result<Value, Error> {
        let mut params = Vec::new();
        if let Some(limit) = query.limit {
            params.push(("limit", limit.to_string()));
        }
        if let Some(offset) = query.offset {
            params.push(("offset", offset.to_string()));
        }
        if let Some(value) = query.q.filter(|value| !value.is_empty()) {
            params.push(("q", value.to_string()));
        }
        if let Some(value) = query.status.filter(|value| !value.is_empty()) {
            params.push(("status", value.to_string()));
        }
        if let Some(value) = query.tool.filter(|value| !value.is_empty()) {
            params.push(("tool", value.to_string()));
        }
        if let Some(value) = query.template_id.filter(|value| !value.is_empty()) {
            params.push(("template_id", value.to_string()));
        }
        self.json(Method::GET, "/api/jobs", &params, None, query.team_id)
            .await
    }

    pub async fn get_job(&self, id: &str, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(
            Method::GET,
            &format!("/api/jobs/{}", path_segment(id)),
            &[],
            None,
            team_id,
        )
        .await
    }

    pub async fn job_output(&self, id: &str, team_id: Option<&str>) -> Result<String, Error> {
        self.request(
            Method::GET,
            &format!("/api/jobs/{}/output", path_segment(id)),
            &[],
            None,
            team_id,
        )
        .await
    }

    pub async fn get_workflow_job(&self, id: &str, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(
            Method::GET,
            &format!("/api/workflow-jobs/{}", path_segment(id)),
            &[],
            None,
            team_id,
        )
        .await
    }

    pub async fn launch_template(
        &self,
        id: &str,
        extra_vars: Map<String, Value>,
        team_id: Option<&str>,
    ) -> Result<Value, Error> {
        let body = json!({"extra_vars": extra_vars});
        self.json(
            Method::POST,
            &format!("/api/templates/{}/launch", path_segment(id)),
            &[],
            Some(&body),
            team_id,
        )
        .await
    }

    pub async fn launch_workflow(
        &self,
        id: &str,
        extra_vars: Map<String, Value>,
        team_id: Option<&str>,
    ) -> Result<Value, Error> {
        let body = json!({"extra_vars": extra_vars});
        self.json(
            Method::POST,
            &format!("/api/workflows/{}/launch", path_segment(id)),
            &[],
            Some(&body),
            team_id,
        )
        .await
    }

    pub async fn cancel_job(&self, id: &str, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(
            Method::POST,
            &format!("/api/jobs/{}/cancel", path_segment(id)),
            &[],
            None,
            team_id,
        )
        .await
    }

    pub async fn relaunch_job(&self, id: &str, team_id: Option<&str>) -> Result<Value, Error> {
        let body = json!({});
        self.json(
            Method::POST,
            &format!("/api/jobs/{}/relaunch", path_segment(id)),
            &[],
            Some(&body),
            team_id,
        )
        .await
    }

    pub async fn sync_project(&self, id: &str, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(
            Method::POST,
            &format!("/api/projects/{}/sync", path_segment(id)),
            &[],
            None,
            team_id,
        )
        .await
    }

    pub async fn sync_inventory(&self, id: &str, team_id: Option<&str>) -> Result<Value, Error> {
        self.json(
            Method::POST,
            &format!("/api/inventories/{}/sync", path_segment(id)),
            &[],
            None,
            team_id,
        )
        .await
    }
}

#[derive(Default)]
pub struct JobQuery<'a> {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub q: Option<&'a str>,
    pub status: Option<&'a str>,
    pub tool: Option<&'a str>,
    pub template_id: Option<&'a str>,
    pub team_id: Option<&'a str>,
}

fn response_error(status: u16, text: &str) -> Error {
    if let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(text) {
        if let Some(key) = map
            .remove("error")
            .and_then(|value| value.as_str().map(str::to_string))
        {
            let vars = map
                .remove("vars")
                .and_then(|value| value.as_object().cloned())
                .map(|vars| {
                    vars.into_iter()
                        .filter_map(|(name, value)| {
                            value.as_str().map(|value| (name, value.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Error {
                status: Some(status),
                key,
                vars,
            };
        }
    }
    let mut vars = BTreeMap::new();
    vars.insert("status".to_string(), status.to_string());
    Error {
        status: Some(status),
        key: "request_failed".to_string(),
        vars,
    }
}

fn json_text(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "null".to_string())
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TeamArgs {
    #[serde(default)]
    team_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct IdArgs {
    id: String,
    #[serde(default)]
    team_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LaunchArgs {
    id: String,
    #[serde(default)]
    extra_vars: Map<String, Value>,
    #[serde(default)]
    team_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListJobsArgs {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    template_id: Option<String>,
    #[serde(default)]
    team_id: Option<String>,
}

#[derive(Clone)]
pub struct Server {
    client: Client,
    per_request_token: bool,
    tool_router: ToolRouter<Self>,
}

impl Server {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            per_request_token: false,
            tool_router: Self::tool_router(),
        }
    }

    pub fn http(state: crate::Shared) -> Self {
        Self {
            client: Client::local(crate::http::api_router(state)),
            per_request_token: true,
            tool_router: Self::tool_router(),
        }
    }

    fn client(&self, context: &RequestContext<RoleServer>) -> Result<Client, String> {
        if !self.per_request_token {
            return Ok(self.client.clone());
        }
        let parts = context
            .extensions
            .get::<axum::http::request::Parts>()
            .ok_or_else(|| Error::new("sign_in_is_required").text())?;
        let token = crate::auth::bearer_token(&parts.headers)
            .ok_or_else(|| Error::new("sign_in_is_required").text())?;
        Ok(self.client.clone().with_token(token))
    }
}

#[tool_router]
impl Server {
    #[tool(description = "Return the signed-in account from GET /api/me.")]
    async fn whoami(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .whoami(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(description = "Return the server version from GET /api/version.")]
    async fn version(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .version(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(description = "List the projects visible to the selected team.")]
    async fn list_projects(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .list_projects(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(description = "List the inventories visible to the selected team.")]
    async fn list_inventories(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .list_inventories(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "List the credentials of the selected team. Secrets stay hidden by the API."
    )]
    async fn list_credentials(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .list_credentials(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "List the templates of the selected team. Template JSON can contain secret values in extra_vars; do not repeat those values to the user verbatim."
    )]
    async fn list_templates(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .list_templates(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "List the workflows of the selected team. Node overrides can contain extra_vars, which may hold secrets; never repeat those values verbatim."
    )]
    async fn list_workflows(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .list_workflows(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(description = "List the schedules of the selected team.")]
    async fn list_schedules(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .list_schedules(args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "List jobs of the selected team with optional limit, offset, q, status, tool, and template_id filters. Job JSON can contain secret values in extra_vars; do not repeat those values to the user verbatim."
    )]
    async fn list_jobs(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<ListJobsArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .list_jobs(JobQuery {
                limit: args.limit,
                offset: args.offset,
                q: args.q.as_deref(),
                status: args.status.as_deref(),
                tool: args.tool.as_deref(),
                template_id: args.template_id.as_deref(),
                team_id: args.team_id.as_deref(),
            })
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "Return one job from GET /api/jobs/{id}. Job JSON can contain secret values in extra_vars; do not repeat those values to the user verbatim."
    )]
    async fn get_job(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .get_job(&args.id, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "Return the capped text output from GET /api/jobs/{id}/output. The output can contain secret values; do not repeat them verbatim."
    )]
    async fn job_output(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .job_output(&args.id, args.team_id.as_deref())
            .await
            .map_err(|error| error.text())
    }

    #[tool(
        description = "Return one workflow job from GET /api/workflow-jobs/{id}. Job JSON can contain secret values in extra_vars; do not repeat those values to the user verbatim."
    )]
    async fn get_workflow_job(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .get_workflow_job(&args.id, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "Launch a template with POST /api/templates/{id}/launch. extra_vars is a JSON object; it can contain secret values and must not be repeated to the user verbatim."
    )]
    async fn launch_template(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<LaunchArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .launch_template(&args.id, args.extra_vars, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "Launch a workflow with POST /api/workflows/{id}/launch. extra_vars is a JSON object; it can contain secret values and must not be repeated to the user verbatim."
    )]
    async fn launch_workflow(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<LaunchArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .launch_workflow(&args.id, args.extra_vars, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "Cancel a job with POST /api/jobs/{id}/cancel. The response is job JSON that can contain secret values in extra_vars; do not repeat those values to the user verbatim."
    )]
    async fn cancel_job(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .cancel_job(&args.id, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(
        description = "Relaunch a job with POST /api/jobs/{id}/relaunch. The response is job JSON that can contain secret values in extra_vars; do not repeat those values to the user verbatim."
    )]
    async fn relaunch_job(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .relaunch_job(&args.id, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(description = "Sync a project with POST /api/projects/{id}/sync.")]
    async fn sync_project(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .sync_project(&args.id, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }

    #[tool(description = "Sync an inventory with POST /api/inventories/{id}/sync.")]
    async fn sync_inventory(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<String, String> {
        self.client(&context)?
            .sync_inventory(&args.id, args.team_id.as_deref())
            .await
            .map(json_text)
            .map_err(|error| error.text())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {}

pub fn requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .nth(1)
        .is_some_and(|arg| arg.as_ref() == "mcp")
}

pub async fn run() -> anyhow::Result<()> {
    let token = std::env::var("KOREOGRAF_TOKEN").unwrap_or_default();
    if token.trim().is_empty() {
        anyhow::bail!("KOREOGRAF_TOKEN is required");
    }
    let base = std::env::var("KOREOGRAF_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string());
    let team = std::env::var("KOREOGRAF_TEAM_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let client = Client::new(base, token, team)
        .map_err(|_| anyhow::anyhow!("could not create the HTTP client"))?;
    let server = Server::new(client);
    let running = rmcp::serve_server(server, rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::path_segment;

    #[test]
    fn ids_cannot_leave_the_route() {
        assert_eq!(path_segment("abc-123_X~"), "abc-123_X~");
        assert_eq!(path_segment("../admin"), "%2E%2E%2Fadmin");
        assert_eq!(path_segment("a/b"), "a%2Fb");
        assert_eq!(path_segment("a?b#c"), "a%3Fb%23c");
        assert_eq!(path_segment("a b"), "a%20b");
    }
}
