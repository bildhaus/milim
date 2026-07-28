use super::*;

// ----- MCP (tools) -----

fn mcp_registry(st: &AppState) -> ToolRegistry {
    let mut reg = static_registry_for_run(st);
    register_skill_tools(&mut reg, st, "auto", &[]);
    if let Some(hub) = &st.mcp {
        register_mcp_server_tools(&mut reg, hub.clone());
        for tool in hub.tools() {
            if let Err(error) = reg.try_register(tool) {
                tracing::warn!("skipping colliding MCP tool: {error}");
            }
        }
    }
    reg.without(HASHLINE_TOOL_NAMES)
}

const MAX_SKILL_READ_CHARS: usize = 40_000;

#[derive(Clone)]
struct MilimSkillScope {
    allowed_ids: Option<Arc<HashSet<String>>>,
}

struct MilimSkillSearchTool {
    store: Arc<milim_skills::SkillStore>,
    scope: MilimSkillScope,
}

struct MilimSkillReadTool {
    store: Arc<milim_skills::SkillStore>,
    scope: MilimSkillScope,
}

#[cfg(test)]
mod skill_tool_tests {
    use super::*;
    use milim_storage::Database;

    #[tokio::test]
    async fn lazy_skill_tools_enforce_custom_agent_scope() {
        let store =
            Arc::new(milim_skills::SkillStore::new(Database::open_in_memory().unwrap()).unwrap());
        let allowed = store
            .create(
                "Code Review",
                "Review source changes",
                "List findings first.",
            )
            .unwrap();
        let blocked = store
            .create("Deployment", "Deploy releases", "Push the release.")
            .unwrap();
        let scope = MilimSkillScope {
            allowed_ids: Some(Arc::new(HashSet::from([allowed.id.clone()]))),
        };
        let search = MilimSkillSearchTool {
            store: store.clone(),
            scope: scope.clone(),
        };
        let found = search
            .invoke(json!({ "query": "review source changes", "limit": 10 }))
            .await
            .unwrap();
        assert_eq!(found["skills"].as_array().unwrap().len(), 1);
        assert_eq!(found["skills"][0]["id"], allowed.id);

        let read = MilimSkillReadTool { store, scope };
        let loaded = read.invoke(json!({ "id": allowed.id })).await.unwrap();
        assert_eq!(loaded["instructions"], "List findings first.");
        assert!(read.invoke(json!({ "id": blocked.id })).await.is_err());
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilimSkillSearchArgs {
    query: String,
    #[serde(default = "default_skill_select_limit")]
    limit: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilimSkillReadArgs {
    id: String,
}

pub(crate) fn register_skill_tools(
    registry: &mut ToolRegistry,
    st: &AppState,
    skill_mode: &str,
    enabled_skills: &[String],
) {
    let Some(store) = st.skills.as_ref().cloned() else {
        return;
    };
    let allowed_ids = match skill_mode {
        "none" => return,
        "custom" if enabled_skills.is_empty() => return,
        "custom" => Some(Arc::new(enabled_skills.iter().cloned().collect())),
        _ => None,
    };
    let scope = MilimSkillScope { allowed_ids };
    registry.register(Arc::new(MilimSkillSearchTool {
        store: store.clone(),
        scope: scope.clone(),
    }));
    registry.register(Arc::new(MilimSkillReadTool { store, scope }));
}

fn skill_allowed(scope: &MilimSkillScope, skill: &milim_skills::SkillDef) -> bool {
    skill.enabled
        && scope
            .allowed_ids
            .as_ref()
            .is_none_or(|ids| ids.contains(&skill.id))
}

fn compact_skill_description(value: &str) -> String {
    const MAX_CHARS: usize = 220;
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }
    format!(
        "{}...",
        normalized
            .chars()
            .take(MAX_CHARS)
            .collect::<String>()
            .trim_end()
    )
}

#[async_trait]
impl Tool for MilimSkillSearchTool {
    fn name(&self) -> &str {
        "milim_skill_search"
    }

    fn description(&self) -> &str {
        "Find relevant enabled Milim skills without loading their instruction bodies."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Current task or capability to match." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 3 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: MilimSkillSearchArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid milim_skill_search arguments: {error}"))
        })?;
        let query = trim_required_tool_arg(args.query, "query")?;
        let allowed = self
            .scope
            .allowed_ids
            .as_ref()
            .map(|ids| ids.iter().cloned().collect::<Vec<_>>());
        let skills =
            self.store
                .select_filtered(&query, args.limit.clamp(1, 10), allowed.as_deref())?;
        Ok(json!({
            "skills": skills.into_iter().map(|skill| json!({
                "id": skill.id,
                "name": skill.name,
                "description": compact_skill_description(&skill.description),
            })).collect::<Vec<_>>()
        }))
    }
}

#[async_trait]
impl Tool for MilimSkillReadTool {
    fn name(&self) -> &str {
        "milim_skill_read"
    }

    fn description(&self) -> &str {
        "Load the complete instructions for one enabled Milim skill selected by id."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Skill id returned by milim_skill_search or the turn's skill candidates." }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: MilimSkillReadArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid milim_skill_read arguments: {error}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let skill = self
            .store
            .get(&id)?
            .filter(|skill| skill_allowed(&self.scope, skill))
            .ok_or_else(|| Error::ModelNotFound(format!("skill {id}")))?;
        if skill.instructions.chars().count() > MAX_SKILL_READ_CHARS {
            return Err(Error::InvalidRequest(format!(
                "skill {} exceeds the {} character read limit; move detailed material into referenced files",
                skill.name, MAX_SKILL_READ_CHARS
            )));
        }
        Ok(json!({
            "id": skill.id,
            "name": skill.name,
            "description": skill.description,
            "instructions": skill.instructions,
        }))
    }
}

fn mcp_catalog_registry(st: &AppState) -> ToolRegistry {
    let mut registry = mcp_registry(st);
    if let Some(store) = st.schedules.as_ref() {
        register_schedule_tools(&mut registry, store.clone(), workspace_snapshot(st));
    }
    if let Some(store) = st.memory.as_ref() {
        registry.register(Arc::new(MemoryRegisterTool {
            store: store.clone(),
            context: AgentMemoryContext::default(),
        }));
    }
    if let Some(supervisor) = st.threads.as_ref() {
        register_child_thread_tools(
            &mut registry,
            st.clone(),
            supervisor.clone(),
            AgentMemoryContext::default(),
            ToolRegistry::new(),
            false,
        );
    }
    registry
}

/// `GET /mcp/tools` — list available tools.
#[derive(Default, Deserialize)]
pub(crate) struct McpToolsQuery {
    #[serde(default)]
    callable: bool,
}

pub(crate) async fn mcp_tools(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<McpToolsQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let tools = if query.callable {
        mcp_registry(&st).read_only().list()
    } else {
        mcp_catalog_registry(&st).list()
    };
    Ok(Json(json!({ "tools": tools })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct McpCallRequest {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// `POST /mcp/call` — invoke a tool by name.
pub(crate) async fn mcp_call(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpCallRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let registry = mcp_registry(&st).read_only();
    if registry.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "no tools registered".to_string(),
        )));
    }
    let result = registry
        .call(&req.name, req.arguments)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({ "result": result })).into_response())
}

const MCP_APP_HTML_MIME: &str = "text/html;profile=mcp-app";
const MCP_APP_HTML_LIMIT: usize = 5 * 1024 * 1024;
const MCP_APP_RESULT_LIMIT: usize = 1024 * 1024;

#[derive(Deserialize)]
pub(crate) struct McpAppResourceReadRequest {
    server_id: String,
    uri: String,
    #[serde(default)]
    render: bool,
}

#[derive(Deserialize)]
pub(crate) struct McpAppToolCallRequest {
    server_id: String,
    name: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    approval: Option<String>,
    #[serde(default)]
    approval_granted: bool,
}

/// `POST /mcp/apps/resources/read` — fetch one server-advertised MCP App document.
pub(crate) async fn mcp_app_resource_read(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpAppResourceReadRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP is not configured".to_string())))?;
    let result = hub
        .read_app_resource(&req.server_id, &req.uri)
        .await
        .map_err(ApiError)?;
    let document = validate_mcp_app_resource(&result, &req.uri).map_err(ApiError)?;
    let view_path = req.render.then(|| {
        let id = st.mcp_app_views.insert(document);
        format!("/mcp/apps/views/{id}")
    });
    let result = (!req.render).then_some(result);
    Ok(Json(json!({ "result": result, "view_path": view_path })).into_response())
}

/// Ephemeral capability URL for one validated App document.
pub(crate) async fn mcp_app_view(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let document = st.mcp_app_views.take(&id).ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "MCP App view expired or does not exist".to_string(),
        ))
    })?;
    let mut response = Html(document.html).into_response();
    response.headers_mut().insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&document.csp)
            .map_err(|_| ApiError(Error::Other("invalid generated MCP App CSP".to_string())))?,
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// `POST /mcp/apps/tools/call` — invoke an app-visible tool on its fixed server.
pub(crate) async fn mcp_app_tool_call(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpAppToolCallRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP is not configured".to_string())))?;
    let tool = hub.app_tool(&req.server_id, &req.name).ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "tool is not app-visible on this MCP server".to_string(),
        ))
    })?;
    enforce_mcp_app_approval(req.approval.as_deref(), req.approval_granted, tool.effect)
        .map_err(ApiError)?;
    let result = hub
        .call_app_tool(&req.server_id, &req.name, req.arguments)
        .await
        .map_err(ApiError)?;
    let size = serde_json::to_vec(&result)
        .map_err(|error| {
            ApiError(Error::Other(format!(
                "MCP App result encoding failed: {error}"
            )))
        })?
        .len();
    if size > MCP_APP_RESULT_LIMIT {
        return Err(ApiError(Error::InvalidRequest(
            "MCP App tool result exceeds the 1 MiB limit".to_string(),
        )));
    }
    Ok(Json(json!({ "result": result })).into_response())
}

fn enforce_mcp_app_approval(
    approval: Option<&str>,
    approval_granted: bool,
    effect: ToolEffect,
) -> Result<(), Error> {
    match approval {
        Some("review") if !approval_granted => Err(Error::InvalidRequest(
            "this MCP App tool call requires approval".to_string(),
        )),
        Some("review") => Ok(()),
        Some("open") => Ok(()),
        _ if effect == ToolEffect::ReadOnly => Ok(()),
        _ => Err(Error::InvalidRequest(
            "guarded mode only allows read-only MCP App tools".to_string(),
        )),
    }
}

fn validate_mcp_app_resource(
    result: &Value,
    requested_uri: &str,
) -> Result<McpAppViewDocument, Error> {
    if !requested_uri.starts_with("ui://") {
        return Err(Error::InvalidRequest(
            "MCP App resources must use ui:// URIs".to_string(),
        ));
    }
    let content = result
        .get("contents")
        .and_then(Value::as_array)
        .and_then(|contents| {
            contents
                .iter()
                .find(|content| content.get("uri").and_then(Value::as_str) == Some(requested_uri))
        })
        .ok_or_else(|| {
            Error::InvalidRequest(
                "MCP App resource response did not contain the requested URI".to_string(),
            )
        })?;
    if content.get("mimeType").and_then(Value::as_str) != Some(MCP_APP_HTML_MIME) {
        return Err(Error::InvalidRequest(format!(
            "MCP App resource must use MIME type {MCP_APP_HTML_MIME}"
        )));
    }
    let html = if let Some(text) = content.get("text").and_then(Value::as_str) {
        text.to_string()
    } else if let Some(blob) = content.get("blob").and_then(Value::as_str) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(blob)
            .map_err(|_| {
                Error::InvalidRequest("MCP App resource blob is not valid base64".to_string())
            })?;
        String::from_utf8(bytes).map_err(|_| {
            Error::InvalidRequest("MCP App resource blob is not UTF-8 HTML".to_string())
        })?
    } else {
        return Err(Error::InvalidRequest(
            "MCP App resource must contain text or blob HTML".to_string(),
        ));
    };
    if html.len() > MCP_APP_HTML_LIMIT {
        return Err(Error::InvalidRequest(
            "MCP App HTML exceeds the 5 MiB limit".to_string(),
        ));
    }
    Ok(McpAppViewDocument {
        html,
        csp: mcp_app_content_security_policy(content),
    })
}

fn mcp_app_content_security_policy(content: &Value) -> String {
    let csp = content.pointer("/_meta/ui/csp").unwrap_or(&Value::Null);
    let resources = csp.get("resourceDomains");
    [
        "default-src 'none'".to_string(),
        format!(
            "script-src {}",
            mcp_app_csp_sources(resources, &["'unsafe-inline'"])
        ),
        format!(
            "style-src {}",
            mcp_app_csp_sources(resources, &["'unsafe-inline'"])
        ),
        format!(
            "img-src {}",
            mcp_app_csp_sources(resources, &["data:", "blob:"])
        ),
        format!("font-src {}", mcp_app_csp_sources(resources, &["data:"])),
        format!(
            "media-src {}",
            mcp_app_csp_sources(resources, &["data:", "blob:"])
        ),
        format!(
            "connect-src {}",
            mcp_app_csp_sources(csp.get("connectDomains"), &[])
        ),
        format!(
            "frame-src {}",
            mcp_app_csp_sources(csp.get("frameDomains"), &[])
        ),
        format!(
            "base-uri {}",
            mcp_app_csp_sources(csp.get("baseUriDomains"), &[])
        ),
        "form-action 'none'".to_string(),
        "object-src 'none'".to_string(),
    ]
    .join("; ")
}

fn mcp_app_csp_sources(value: Option<&Value>, defaults: &[&str]) -> String {
    let mut sources = defaults
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    if let Some(values) = value.and_then(Value::as_array) {
        sources.extend(
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| valid_mcp_app_origin(value))
                .map(str::to_string),
        );
    }
    if sources.is_empty() {
        "'none'".to_string()
    } else {
        sources.join(" ")
    }
}

fn valid_mcp_app_origin(value: &str) -> bool {
    if value
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, ';' | '\'' | '"'))
    {
        return false;
    }
    let Some((scheme, authority)) = value.split_once("://") else {
        return false;
    };
    matches!(scheme, "http" | "https" | "ws" | "wss")
        && !authority.is_empty()
        && !authority.contains(['/', '?', '#'])
}

#[cfg(test)]
mod mcp_app_tests {
    use super::*;

    #[test]
    fn app_resource_requires_matching_uri_mime_and_size() {
        let valid = json!({"contents":[{
            "uri":"ui://fixture/chart",
            "mimeType":MCP_APP_HTML_MIME,
            "text":"<h1>Chart</h1>",
            "_meta":{"ui":{"csp":{
                "connectDomains":["https://api.example.com","https://bad.example;script-src *"],
                "resourceDomains":["https://cdn.example.com"]
            }}}
        }]});
        let document = validate_mcp_app_resource(&valid, "ui://fixture/chart").unwrap();
        assert!(document.csp.contains("connect-src https://api.example.com"));
        assert!(document
            .csp
            .contains("script-src 'unsafe-inline' https://cdn.example.com"));
        assert!(!document.csp.contains("bad.example"));
        let wrong_mime = json!({"contents":[{
            "uri":"ui://fixture/chart",
            "mimeType":"text/html",
            "text":"<h1>Chart</h1>"
        }]});
        assert!(validate_mcp_app_resource(&wrong_mime, "ui://fixture/chart").is_err());
        let oversized = json!({"contents":[{
            "uri":"ui://fixture/chart",
            "mimeType":MCP_APP_HTML_MIME,
            "text":"x".repeat(MCP_APP_HTML_LIMIT + 1)
        }]});
        assert!(validate_mcp_app_resource(&oversized, "ui://fixture/chart").is_err());
        assert!(validate_mcp_app_resource(&valid, "https://fixture/chart").is_err());
    }

    #[test]
    fn app_approval_enforces_review_guarded_and_open() {
        assert!(enforce_mcp_app_approval(Some("review"), false, ToolEffect::ReadOnly).is_err());
        assert!(enforce_mcp_app_approval(Some("review"), true, ToolEffect::Mutating).is_ok());
        assert!(enforce_mcp_app_approval(Some("guarded"), false, ToolEffect::ReadOnly).is_ok());
        assert!(enforce_mcp_app_approval(Some("guarded"), false, ToolEffect::Mutating).is_err());
        assert!(enforce_mcp_app_approval(Some("open"), false, ToolEffect::Mutating).is_ok());
    }
}

// ----- MCP servers (external MCP client connections) -----

#[derive(Deserialize)]
pub(crate) struct McpServerUpsert {
    #[serde(default)]
    id: Option<String>,
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<milim_mcp_client::McpEnvVar>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

/// `GET /mcp/servers` — list configured MCP servers with connection status.
pub(crate) async fn mcp_servers_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let list = match &st.mcp {
        Some(hub) => hub.list(),
        None => Vec::new(),
    };
    Ok(Json(json!({ "servers": list })).into_response())
}

/// `POST /mcp/servers` — add or update an MCP server (connects immediately).
pub(crate) async fn mcp_server_upsert(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpServerUpsert>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    let cfg = milim_mcp_client::McpServerConfig {
        id: req.id.unwrap_or_default(),
        name: req.name,
        command: req.command,
        args: req.args,
        cwd: req.cwd,
        env: req.env,
        enabled: req.enabled,
    };
    let saved = hub.upsert(cfg).await.map_err(ApiError)?;
    let info = hub.list().into_iter().find(|s| s.id == saved.id);
    Ok(Json(json!({ "server": info })).into_response())
}

/// `POST /mcp/servers/test` — test a draft MCP server without saving/enabling it.
pub(crate) async fn mcp_server_test_draft(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpServerUpsert>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    let cfg = milim_mcp_client::McpServerConfig {
        id: req.id.unwrap_or_default(),
        name: req.name,
        command: req.command,
        args: req.args,
        cwd: req.cwd,
        env: req.env,
        enabled: req.enabled,
    };
    Ok(Json(hub.test_config(cfg).await).into_response())
}

/// `POST /mcp/servers/{id}/test` — test a saved MCP server without enabling it.
pub(crate) async fn mcp_server_test_saved(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    let cfg = hub
        .config(&id)
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("mcp server {id}"))))?;
    Ok(Json(hub.test_config(cfg).await).into_response())
}

/// `DELETE /mcp/servers/{id}` — remove an MCP server (disconnects it).
pub(crate) async fn mcp_server_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    Ok(Json(json!({ "deleted": hub.remove(&id).map_err(ApiError)? })).into_response())
}

pub(crate) fn register_mcp_server_tools(
    registry: &mut ToolRegistry,
    hub: Arc<milim_mcp_client::McpHub>,
) {
    registry.register(Arc::new(McpServerListTool { hub: hub.clone() }));
    registry.register(Arc::new(McpServerTestTool { hub: hub.clone() }));
    registry.register(Arc::new(McpServerSaveTool { hub: hub.clone() }));
    registry.register(Arc::new(McpServerDeleteTool { hub }));
}

struct McpServerListTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

struct McpServerTestTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

struct McpServerSaveTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

struct McpServerDeleteTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerToolEnv {
    key: String,
    value: String,
    #[serde(default)]
    required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerToolSecretEnv {
    key: String,
    #[serde(default = "default_true")]
    required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerToolConfig {
    #[serde(default)]
    id: Option<String>,
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<McpServerToolEnv>,
    #[serde(default)]
    secret_env: Vec<McpServerToolSecretEnv>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerDeleteToolArgs {
    id: String,
}

impl McpServerToolConfig {
    fn into_config(
        self,
        hub: &milim_mcp_client::McpHub,
        update_must_exist: bool,
    ) -> milim_core::Result<milim_mcp_client::McpServerConfig> {
        let id = self
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        if update_must_exist && id.as_deref().is_some_and(|id| hub.config(id).is_none()) {
            return Err(Error::ModelNotFound(format!(
                "mcp server {}",
                id.as_deref().unwrap_or_default()
            )));
        }

        let mut keys = HashSet::new();
        let mut env = Vec::with_capacity(self.env.len() + self.secret_env.len());
        for item in self.env {
            let key = trim_required_tool_arg(item.key, "env[].key")?;
            if milim_mcp_client::secret_env_key(&key) {
                return Err(Error::InvalidRequest(format!(
                    "environment variable {key} looks secret; declare it in secret_env without a value"
                )));
            }
            if !keys.insert(key.clone()) {
                return Err(Error::InvalidRequest(format!(
                    "duplicate environment variable: {key}"
                )));
            }
            env.push(milim_mcp_client::McpEnvVar {
                key,
                value: Some(item.value),
                secret: false,
                required: item.required,
                has_value: false,
            });
        }
        for item in self.secret_env {
            let key = trim_required_tool_arg(item.key, "secret_env[].key")?;
            if !keys.insert(key.clone()) {
                return Err(Error::InvalidRequest(format!(
                    "duplicate environment variable: {key}"
                )));
            }
            env.push(milim_mcp_client::McpEnvVar {
                key,
                value: None,
                secret: true,
                required: item.required,
                has_value: false,
            });
        }

        Ok(milim_mcp_client::McpServerConfig {
            id: id.unwrap_or_default(),
            name: trim_required_tool_arg(self.name, "name")?,
            command: trim_required_tool_arg(self.command, "command")?,
            args: self.args,
            cwd: self
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            env,
            enabled: self.enabled,
        })
    }
}

fn mcp_server_config_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Existing server id. Omit to create or test a new server." },
            "name": { "type": "string", "description": "Human-readable server name." },
            "command": { "type": "string", "description": "Executable to run, such as npx, uvx, node, or an absolute executable path." },
            "args": { "type": "array", "items": { "type": "string" }, "description": "Exact command arguments." },
            "cwd": { "type": ["string", "null"], "description": "Optional working directory. Use the active workspace for a locally-authored server." },
            "env": {
                "type": "array",
                "description": "Non-secret environment variables. Credential-looking keys are rejected.",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "value": { "type": "string" },
                        "required": { "type": "boolean" }
                    },
                    "required": ["key", "value"],
                    "additionalProperties": false
                }
            },
            "secret_env": {
                "type": "array",
                "description": "Secret environment variable placeholders. Values must be entered later in Milim's encrypted MCP Manager.",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "required": { "type": "boolean", "description": "Defaults to true." }
                    },
                    "required": ["key"],
                    "additionalProperties": false
                }
            },
            "enabled": { "type": "boolean", "description": "Connect after saving. Defaults to true." }
        },
        "required": ["name", "command"],
        "additionalProperties": false
    })
}

#[async_trait]
impl Tool for McpServerListTool {
    fn name(&self) -> &str {
        "mcp_server_list"
    }

    fn description(&self) -> &str {
        "List Milim-managed MCP servers, connection status, capabilities, and sanitized environment metadata. Secret values are never returned."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
        Ok(json!({ "ok": true, "servers": self.hub.list() }))
    }
}

#[async_trait]
impl Tool for McpServerTestTool {
    fn name(&self) -> &str {
        "mcp_server_test"
    }

    fn description(&self) -> &str {
        "Launch and test an MCP server configuration without saving it. Use an existing id to reuse its encrypted secret placeholders. Requires command approval."
    }

    fn input_schema(&self) -> Value {
        mcp_server_config_schema()
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: McpServerToolConfig = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid mcp_server_test arguments: {error}"))
        })?;
        Ok(json!(
            self.hub
                .test_config(args.into_config(&self.hub, false)?)
                .await
        ))
    }
}

#[async_trait]
impl Tool for McpServerSaveTool {
    fn name(&self) -> &str {
        "mcp_server_save"
    }

    fn description(&self) -> &str {
        "Create or fully replace a Milim-managed MCP server. Omit id to create; list first and include id to update. Connected tools become callable on the next chat turn. Requires command approval."
    }

    fn input_schema(&self) -> Value {
        mcp_server_config_schema()
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: McpServerToolConfig = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid mcp_server_save arguments: {error}"))
        })?;
        let saved = self.hub.upsert(args.into_config(&self.hub, true)?).await?;
        let server = self
            .hub
            .list()
            .into_iter()
            .find(|server| server.id == saved.id)
            .ok_or_else(|| Error::ModelNotFound(format!("mcp server {}", saved.id)))?;
        let tools_available_next_turn = server.connected && server.tool_count > 0;
        Ok(json!({
            "ok": true,
            "server": server,
            "tools_available_next_turn": tools_available_next_turn
        }))
    }
}

#[async_trait]
impl Tool for McpServerDeleteTool {
    fn name(&self) -> &str {
        "mcp_server_delete"
    }

    fn description(&self) -> &str {
        "Disconnect and permanently remove a Milim-managed MCP server by id."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Existing server id." } },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: McpServerDeleteToolArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid mcp_server_delete arguments: {error}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        if !self.hub.remove(&id)? {
            return Err(Error::ModelNotFound(format!("mcp server {id}")));
        }
        Ok(json!({ "ok": true, "deleted": true, "id": id }))
    }
}

#[cfg(test)]
mod mcp_server_tool_tests {
    use super::*;

    fn hub() -> (PathBuf, Arc<milim_mcp_client::McpHub>) {
        let root = std::env::temp_dir().join(format!(
            "milim-mcp-chat-tools-{}",
            uuid::Uuid::new_v4().simple()
        ));
        (root.clone(), Arc::new(milim_mcp_client::McpHub::open(root)))
    }

    #[tokio::test]
    async fn manages_servers_and_keeps_secret_values_hidden() {
        let (root, hub) = hub();
        let mut tools = ToolRegistry::new();
        register_mcp_server_tools(&mut tools, hub.clone());

        assert_eq!(tools.effect("mcp_server_list"), Some(ToolEffect::ReadOnly));
        assert_eq!(tools.effect("mcp_server_test"), Some(ToolEffect::Command));
        assert_eq!(tools.effect("mcp_server_save"), Some(ToolEffect::Command));
        assert_eq!(
            tools.effect("mcp_server_delete"),
            Some(ToolEffect::Mutating)
        );
        assert!(tools.read_only().contains("mcp_server_list"));
        assert!(!tools.read_only().contains("mcp_server_save"));

        let created = tools
            .call(
                "mcp_server_save",
                json!({
                    "name": "Local fixture",
                    "command": "missing-but-disabled",
                    "env": [{ "key": "BASE_URL", "value": "https://example.com" }],
                    "enabled": false
                }),
            )
            .await
            .unwrap();
        let id = created["server"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["server"]["enabled"], false);
        assert_eq!(created["tools_available_next_turn"], false);

        let updated = tools
            .call(
                "mcp_server_save",
                json!({
                    "id": id,
                    "name": "Renamed fixture",
                    "command": "missing-but-disabled",
                    "enabled": false
                }),
            )
            .await
            .unwrap();
        assert_eq!(updated["server"]["name"], "Renamed fixture");

        tools
            .call("mcp_server_delete", json!({ "id": id }))
            .await
            .unwrap();
        assert!(hub.list().is_empty());

        hub.upsert(milim_mcp_client::McpServerConfig {
            id: "secret-server".into(),
            name: "Secret fixture".into(),
            command: "missing-but-disabled".into(),
            args: Vec::new(),
            cwd: None,
            env: vec![milim_mcp_client::McpEnvVar {
                key: "API_TOKEN".into(),
                value: Some("encrypted-value".into()),
                secret: true,
                required: true,
                has_value: false,
            }],
            enabled: false,
        })
        .await
        .unwrap();
        tools
            .call(
                "mcp_server_save",
                json!({
                    "id": "secret-server",
                    "name": "Secret fixture updated",
                    "command": "missing-but-disabled",
                    "secret_env": [{ "key": "API_TOKEN" }],
                    "enabled": false
                }),
            )
            .await
            .unwrap();
        let listed = tools.call("mcp_server_list", json!({})).await.unwrap();
        let secret = &listed["servers"][0]["env"][0];
        assert_eq!(secret["key"], "API_TOKEN");
        assert_eq!(secret["has_value"], true);
        assert!(secret.get("value").is_none());

        let rejected = tools
            .call(
                "mcp_server_save",
                json!({
                    "name": "Leaky fixture",
                    "command": "missing-but-disabled",
                    "env": [{ "key": "API_KEY", "value": "must-not-save" }],
                    "enabled": false
                }),
            )
            .await
            .unwrap_err();
        assert!(rejected.to_string().contains("declare it in secret_env"));

        let missing_update = tools
            .call(
                "mcp_server_save",
                json!({
                    "id": "missing",
                    "name": "Missing",
                    "command": "missing-but-disabled",
                    "enabled": false
                }),
            )
            .await
            .unwrap_err();
        assert!(missing_update.to_string().contains("mcp server missing"));

        drop(tools);
        drop(hub);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn tests_drafts_without_saving_them() {
        let (root, hub) = hub();
        let mut tools = ToolRegistry::new();
        register_mcp_server_tools(&mut tools, hub.clone());

        let missing_env = tools
            .call(
                "mcp_server_test",
                json!({
                    "name": "Missing secret",
                    "command": "unused",
                    "secret_env": [{ "key": "API_TOKEN" }]
                }),
            )
            .await
            .unwrap();
        assert_eq!(missing_env["ok"], false);
        assert_eq!(missing_env["missing_env"][0], "API_TOKEN");

        let failed = tools
            .call(
                "mcp_server_test",
                json!({
                    "name": "Missing command",
                    "command": "__milim_missing_mcp_command__"
                }),
            )
            .await
            .unwrap();
        assert_eq!(failed["ok"], false);
        assert!(!failed["error"].as_str().unwrap().is_empty());
        assert!(hub.list().is_empty());

        drop(tools);
        drop(hub);
        let _ = std::fs::remove_dir_all(root);
    }
}
