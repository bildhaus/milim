//! OAuth for Streamable HTTP MCP servers (MCP authorization spec).
//!
//! On a `401`, milim discovers the protected resource metadata (RFC 9728) and
//! its authorization server metadata (RFC 8414 / OpenID discovery), registers
//! a public client dynamically (RFC 7591) unless the user supplied a client
//! id, and runs authorization code + PKCE (S256) through a loopback redirect.
//! Every authorization and token request carries the server's canonical
//! `resource` (RFC 8707). Tokens live in the encrypted MCP secret store and
//! refresh shortly before expiry.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use reqwest::header::{HeaderMap, ACCEPT, WWW_AUTHENTICATE};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use milim_core::{Error, Result};

use crate::transport::BearerSource;
use crate::McpSecretStore;

/// Secret-store key holding one server's OAuth client and tokens.
pub(crate) const OAUTH_SECRET_KEY: &str = "oauth:tokens";
/// How long a browser sign-in may take before the loopback listener closes.
pub(crate) const SIGN_IN_TTL: Duration = Duration::from_secs(10 * 60);
const METADATA_LIMIT: usize = 1024 * 1024;
const REFRESH_MARGIN_SECS: u64 = 60;

/// OAuth client registration plus the current tokens for one server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredTokens {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint_auth_method: Option<String>,
    pub token_endpoint: String,
    pub resource: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl StoredTokens {
    fn expiring(&self, now: u64) -> bool {
        self.expires_at
            .is_some_and(|expires_at| expires_at <= now + REFRESH_MARGIN_SECS)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AuthServerMetadata {
    pub authorization_endpoint: Url,
    pub token_endpoint: Url,
    pub registration_endpoint: Option<Url>,
    pub code_challenge_methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Discovery {
    pub resource: String,
    pub metadata: AuthServerMetadata,
    pub scope: Option<String>,
}

/// A registered or user-supplied OAuth client.
#[derive(Debug, Clone)]
pub(crate) struct OAuthClient {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// A fresh PKCE verifier and its S256 challenge.
pub(crate) fn pkce_pair() -> (String, String) {
    let verifier = format!("{}{}", random_token(), random_token());
    let challenge = pkce_challenge(&verifier);
    (verifier, challenge)
}

pub(crate) fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(crate) fn new_state() -> String {
    random_token()
}

/// Parameters of the `Bearer` challenge in a `WWW-Authenticate` header.
pub(crate) fn challenge_params(header: &str) -> HashMap<String, String> {
    let header = header.trim();
    let rest = match header.split_once(char::is_whitespace) {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("bearer") => rest,
        _ if header.eq_ignore_ascii_case("bearer") => "",
        _ => return HashMap::new(),
    };
    let mut params = HashMap::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let key: String = chars
            .by_ref()
            .take_while(|c| *c != '=')
            .collect::<String>()
            .trim()
            .to_ascii_lowercase();
        if key.is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(escaped) = chars.next() {
                            value.push(escaped);
                        }
                    }
                    '"' => break,
                    other => value.push(other),
                }
            }
        } else {
            value = chars
                .by_ref()
                .take_while(|c| *c != ',')
                .collect::<String>()
                .trim()
                .to_string();
        }
        params.insert(key, value);
    }
    params
}

/// Canonical resource identifier for an MCP server URL (RFC 8707).
pub(crate) fn canonical_resource(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    let text = url.to_string();
    if url.path() == "/" && url.query().is_none() {
        text.trim_end_matches('/').to_string()
    } else {
        text
    }
}

fn is_loopback(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]") | Some("::1")
    )
}

fn require_secure(url: &Url, what: &str) -> Result<()> {
    if url.scheme() == "https" || (url.scheme() == "http" && is_loopback(url)) {
        return Ok(());
    }
    Err(Error::InvalidRequest(format!(
        "{what} must use https: {url}"
    )))
}

fn origin_url(url: &Url) -> Result<Url> {
    Url::parse(&url.origin().ascii_serialization())
        .map_err(|error| Error::Other(format!("invalid URL origin: {error}")))
}

fn well_known(origin: &Url, name: &str, path: &str) -> Option<Url> {
    let path = path.trim_end_matches('/');
    origin.join(&format!("/.well-known/{name}{path}")).ok()
}

async fn fetch_json(http: &reqwest::Client, url: &Url) -> Result<Option<Value>> {
    let response = http
        .get(url.clone())
        .header(ACCEPT, "application/json")
        .header("MCP-Protocol-Version", crate::PROTOCOL_VERSION)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| Error::Upstream(format!("OAuth discovery failed for {url}: {error}")))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(Some(response_json(response).await?))
}

async fn response_json(mut response: reqwest::Response) -> Result<Value> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| Error::Upstream(format!("OAuth response failed: {error}")))?
    {
        if body.len() + chunk.len() > METADATA_LIMIT {
            return Err(Error::Upstream("OAuth response exceeds 1 MiB".into()));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|error| Error::Upstream(format!("invalid OAuth JSON response: {error}")))
}

/// POST an unauthenticated `initialize` to read the server's challenge.
async fn probe_challenge(
    http: &reqwest::Client,
    server: &Url,
    headers: &HeaderMap,
) -> Result<Option<String>> {
    let mut headers = headers.clone();
    headers.remove(reqwest::header::AUTHORIZATION);
    let response = http
        .post(server.clone())
        .headers(headers)
        .header(ACCEPT, "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": crate::PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "milim", "version": env!("CARGO_PKG_VERSION") }
            }
        }))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| Error::Upstream(format!("MCP server unreachable: {error}")))?;
    if response.status() != StatusCode::UNAUTHORIZED {
        return Ok(None);
    }
    Ok(response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string))
}

/// Discover how to authorize against an MCP server.
pub(crate) async fn discover(
    http: &reqwest::Client,
    server: &Url,
    headers: &HeaderMap,
) -> Result<Discovery> {
    let challenge = probe_challenge(http, server, headers)
        .await?
        .map(|header| challenge_params(&header))
        .unwrap_or_default();
    let origin = origin_url(server)?;

    let mut resource_metadata = None;
    if let Some(url) = challenge.get("resource_metadata") {
        let url = Url::parse(url)
            .map_err(|error| Error::Upstream(format!("invalid resource_metadata URL: {error}")))?;
        require_secure(&url, "Protected resource metadata")?;
        resource_metadata = fetch_json(http, &url).await?;
    }
    if resource_metadata.is_none() {
        let mut candidates = Vec::new();
        if server.path() != "/" {
            candidates.extend(well_known(
                &origin,
                "oauth-protected-resource",
                server.path(),
            ));
        }
        candidates.extend(well_known(&origin, "oauth-protected-resource", ""));
        for candidate in candidates {
            if let Some(document) = fetch_json(http, &candidate).await? {
                resource_metadata = Some(document);
                break;
            }
        }
    }

    let (resource, issuer, supported_scopes) = match &resource_metadata {
        Some(document) => {
            let resource = document
                .get("resource")
                .and_then(Value::as_str)
                .and_then(|value| Url::parse(value).ok())
                .filter(|value| value.origin() == server.origin())
                .map(|value| canonical_resource(&value))
                .unwrap_or_else(|| canonical_resource(server));
            let issuer = document
                .get("authorization_servers")
                .and_then(Value::as_array)
                .and_then(|servers| servers.iter().find_map(Value::as_str))
                .ok_or_else(|| {
                    Error::Upstream(
                        "protected resource metadata lists no authorization server".into(),
                    )
                })?;
            let issuer = Url::parse(issuer).map_err(|error| {
                Error::Upstream(format!("invalid authorization server URL: {error}"))
            })?;
            let scopes = string_list(document.get("scopes_supported"));
            (resource, issuer, scopes)
        }
        // MCP 2025-03-26 fallback: the server's origin is the authorization server.
        None => (canonical_resource(server), origin.clone(), Vec::new()),
    };
    require_secure(&issuer, "Authorization server")?;
    let metadata =
        authorization_server_metadata(http, &issuer, resource_metadata.is_none()).await?;
    let scope = challenge
        .get("scope")
        .filter(|scope| !scope.trim().is_empty())
        .cloned()
        .or_else(|| (!supported_scopes.is_empty()).then(|| supported_scopes.join(" ")));
    Ok(Discovery {
        resource,
        metadata,
        scope,
    })
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

async fn authorization_server_metadata(
    http: &reqwest::Client,
    issuer: &Url,
    allow_default_endpoints: bool,
) -> Result<AuthServerMetadata> {
    let origin = origin_url(issuer)?;
    let path = issuer.path().trim_end_matches('/');
    let mut candidates = Vec::new();
    if path.is_empty() {
        candidates.extend(well_known(&origin, "oauth-authorization-server", ""));
        candidates.extend(well_known(&origin, "openid-configuration", ""));
    } else {
        candidates.extend(well_known(&origin, "oauth-authorization-server", path));
        candidates.extend(well_known(&origin, "openid-configuration", path));
        candidates.extend(
            Url::parse(&format!(
                "{}/.well-known/openid-configuration",
                issuer.as_str().trim_end_matches('/')
            ))
            .ok(),
        );
    }
    for candidate in candidates {
        let Some(document) = fetch_json(http, &candidate).await? else {
            continue;
        };
        let endpoint = |key: &str| -> Result<Option<Url>> {
            let Some(value) = document.get(key).and_then(Value::as_str) else {
                return Ok(None);
            };
            let url = Url::parse(value)
                .map_err(|error| Error::Upstream(format!("invalid {key}: {error}")))?;
            require_secure(&url, key)?;
            Ok(Some(url))
        };
        let authorization_endpoint = endpoint("authorization_endpoint")?.ok_or_else(|| {
            Error::Upstream("authorization server metadata has no authorization_endpoint".into())
        })?;
        let token_endpoint = endpoint("token_endpoint")?.ok_or_else(|| {
            Error::Upstream("authorization server metadata has no token_endpoint".into())
        })?;
        return Ok(AuthServerMetadata {
            authorization_endpoint,
            token_endpoint,
            registration_endpoint: endpoint("registration_endpoint")?,
            code_challenge_methods: string_list(document.get("code_challenge_methods_supported")),
        });
    }
    if !allow_default_endpoints {
        return Err(Error::Upstream(format!(
            "no OAuth authorization server metadata found for {issuer}"
        )));
    }
    let join = |path: &str| {
        origin
            .join(path)
            .map_err(|error| Error::Other(format!("invalid OAuth endpoint: {error}")))
    };
    Ok(AuthServerMetadata {
        authorization_endpoint: join("/authorize")?,
        token_endpoint: join("/token")?,
        registration_endpoint: Some(join("/register")?),
        code_challenge_methods: Vec::new(),
    })
}

/// Dynamic client registration (RFC 7591) for a public loopback client.
pub(crate) async fn register_client(
    http: &reqwest::Client,
    endpoint: &Url,
    redirect_uri: &str,
    scope: Option<&str>,
) -> Result<OAuthClient> {
    let mut body = json!({
        "client_name": "milim",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    if let Some(scope) = scope {
        body["scope"] = Value::String(scope.to_string());
    }
    let response = http
        .post(endpoint.clone())
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| Error::Upstream(format!("OAuth client registration failed: {error}")))?;
    let status = response.status();
    let document = response_json(response).await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(Error::Upstream(format!(
            "OAuth client registration returned HTTP {status}: {}",
            oauth_error(&document)
        )));
    }
    let client_id = document
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Upstream("OAuth client registration returned no client_id".into()))?;
    Ok(OAuthClient {
        client_id: client_id.to_string(),
        client_secret: document
            .get("client_secret")
            .and_then(Value::as_str)
            .map(str::to_string),
        token_endpoint_auth_method: document
            .get("token_endpoint_auth_method")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn oauth_error(document: &Value) -> String {
    let error = document.get("error").and_then(Value::as_str).unwrap_or("");
    let description = document
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or("");
    match (error.is_empty(), description.is_empty()) {
        (true, true) => "no error details".to_string(),
        (false, true) => error.to_string(),
        (true, false) => description.to_string(),
        (false, false) => format!("{error}: {description}"),
    }
}

pub(crate) fn authorization_url(
    discovery: &Discovery,
    client: &OAuthClient,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String> {
    let methods = &discovery.metadata.code_challenge_methods;
    if !methods.is_empty() && !methods.iter().any(|method| method == "S256") {
        return Err(Error::Upstream(
            "the authorization server does not support PKCE S256".into(),
        ));
    }
    let mut url = discovery.metadata.authorization_endpoint.clone();
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("resource", &discovery.resource);
        if let Some(scope) = &discovery.scope {
            query.append_pair("scope", scope);
        }
    }
    Ok(url.to_string())
}

async fn token_request(
    http: &reqwest::Client,
    endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    auth_method: Option<&str>,
    mut form: Vec<(&str, String)>,
) -> Result<Value> {
    let mut request = http.post(endpoint).header(ACCEPT, "application/json");
    match (client_secret, auth_method) {
        (Some(secret), Some("client_secret_basic")) => {
            request = request.basic_auth(client_id, Some(secret));
        }
        (Some(secret), _) => {
            form.push(("client_id", client_id.to_string()));
            form.push(("client_secret", secret.to_string()));
        }
        (None, _) => form.push(("client_id", client_id.to_string())),
    }
    let response = request
        .form(&form)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| Error::Upstream(format!("OAuth token request failed: {error}")))?;
    let status = response.status();
    let document = response_json(response).await.unwrap_or(Value::Null);
    if status.is_success()
        && document
            .get("access_token")
            .and_then(Value::as_str)
            .is_some()
    {
        return Ok(document);
    }
    let message = format!(
        "OAuth token request returned HTTP {status}: {}",
        oauth_error(&document)
    );
    if status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED {
        Err(Error::Unauthorized(message))
    } else {
        Err(Error::Upstream(message))
    }
}

fn tokens_from_response(
    document: &Value,
    client: &OAuthClient,
    token_endpoint: &str,
    resource: &str,
    previous_refresh: Option<String>,
) -> Result<StoredTokens> {
    let access_token = document
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Upstream("OAuth token response has no access_token".into()))?;
    Ok(StoredTokens {
        client_id: client.client_id.clone(),
        client_secret: client.client_secret.clone(),
        token_endpoint_auth_method: client.token_endpoint_auth_method.clone(),
        token_endpoint: token_endpoint.to_string(),
        resource: resource.to_string(),
        access_token: access_token.to_string(),
        refresh_token: document
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(previous_refresh),
        expires_at: document
            .get("expires_in")
            .and_then(Value::as_u64)
            .map(|seconds| now_secs() + seconds),
        scope: document
            .get("scope")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Exchange an authorization code for tokens.
pub(crate) async fn exchange_code(
    http: &reqwest::Client,
    discovery: &Discovery,
    client: &OAuthClient,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<StoredTokens> {
    let token_endpoint = discovery.metadata.token_endpoint.to_string();
    let document = token_request(
        http,
        &token_endpoint,
        &client.client_id,
        client.client_secret.as_deref(),
        client.token_endpoint_auth_method.as_deref(),
        vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri.to_string()),
            ("code_verifier", verifier.to_string()),
            ("resource", discovery.resource.clone()),
        ],
    )
    .await?;
    tokens_from_response(
        &document,
        client,
        &token_endpoint,
        &discovery.resource,
        None,
    )
}

/// Refresh stored tokens (keeping the refresh token when it is not rotated).
pub(crate) async fn refresh_tokens(
    http: &reqwest::Client,
    stored: &StoredTokens,
) -> Result<StoredTokens> {
    let refresh_token = stored
        .refresh_token
        .clone()
        .ok_or_else(|| Error::Unauthorized("MCP sign-in expired; sign in again".into()))?;
    let document = token_request(
        http,
        &stored.token_endpoint,
        &stored.client_id,
        stored.client_secret.as_deref(),
        stored.token_endpoint_auth_method.as_deref(),
        vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.clone()),
            ("resource", stored.resource.clone()),
        ],
    )
    .await?;
    let client = OAuthClient {
        client_id: stored.client_id.clone(),
        client_secret: stored.client_secret.clone(),
        token_endpoint_auth_method: stored.token_endpoint_auth_method.clone(),
    };
    tokens_from_response(
        &document,
        &client,
        &stored.token_endpoint,
        &stored.resource,
        Some(refresh_token),
    )
}

// ----- Loopback redirect -----

#[derive(Debug, Default)]
pub(crate) struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// A one-shot `http://127.0.0.1:<port>/callback` listener.
pub(crate) struct LoopbackCallback {
    pub redirect_uri: String,
    listener: TcpListener,
}

impl LoopbackCallback {
    pub(crate) async fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            redirect_uri: format!("http://127.0.0.1:{port}/callback"),
            listener,
        })
    }

    /// Serve until a `/callback` request carrying `expected_state` arrives.
    /// Requests with another path or state are answered and ignored.
    pub(crate) async fn wait(self, expected_state: &str, ttl: Duration) -> Result<CallbackParams> {
        let deadline = tokio::time::Instant::now() + ttl;
        loop {
            let (mut socket, _) = tokio::time::timeout_at(deadline, self.listener.accept())
                .await
                .map_err(|_| Error::Other("MCP sign-in timed out".into()))??;
            let Ok(Ok(target)) =
                tokio::time::timeout(Duration::from_secs(10), read_request_target(&mut socket))
                    .await
            else {
                continue;
            };
            let params = Url::parse(&format!("http://127.0.0.1{target}"))
                .ok()
                .filter(|url| url.path() == "/callback")
                .map(|url| {
                    let mut params = CallbackParams::default();
                    for (key, value) in url.query_pairs() {
                        let value = Some(value.into_owned());
                        match key.as_ref() {
                            "code" => params.code = value,
                            "state" => params.state = value,
                            "error" => params.error = value,
                            "error_description" => params.error_description = value,
                            _ => {}
                        }
                    }
                    params
                });
            let accepted = params
                .as_ref()
                .is_some_and(|params| params.state.as_deref() == Some(expected_state));
            let (status, body) = if accepted {
                (
                    "200 OK",
                    "<!doctype html><title>milim</title><p>Sign-in finished. You can close this window and return to milim.</p>",
                )
            } else {
                (
                    "400 Bad Request",
                    "<!doctype html><title>milim</title><p>This sign-in link is not valid.</p>",
                )
            };
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncache-control: no-store\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
            if accepted {
                return Ok(params.unwrap_or_default());
            }
        }
    }
}

async fn read_request_target(socket: &mut tokio::net::TcpStream) -> std::io::Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = socket.read(&mut chunk).await?;
        if read == 0 || buffer.len() > 16 * 1024 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&buffer);
    let line = text.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Ok(target.to_string()),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unexpected callback request",
        )),
    }
}

/// Complete a sign-in once the browser returns to the loopback listener.
pub(crate) async fn finish_sign_in(
    http: &reqwest::Client,
    callback: LoopbackCallback,
    discovery: &Discovery,
    client: &OAuthClient,
    state: &str,
    verifier: &str,
) -> Result<StoredTokens> {
    let redirect_uri = callback.redirect_uri.clone();
    let params = callback.wait(state, SIGN_IN_TTL).await?;
    if let Some(error) = params.error {
        let detail = params
            .error_description
            .map(|description| format!("{error}: {description}"))
            .unwrap_or(error);
        return Err(Error::Unauthorized(format!(
            "sign-in was not completed ({detail})"
        )));
    }
    let code = params
        .code
        .filter(|code| !code.is_empty())
        .ok_or_else(|| Error::Unauthorized("sign-in returned no authorization code".into()))?;
    exchange_code(http, discovery, client, &code, verifier, &redirect_uri).await
}

// ----- Token source -----

/// Loads, refreshes, and persists one server's tokens for the HTTP transport.
pub(crate) struct TokenSource {
    secrets: Arc<McpSecretStore>,
    server_id: String,
    http: reqwest::Client,
    lock: Mutex<()>,
}

impl TokenSource {
    pub(crate) fn new(
        secrets: Arc<McpSecretStore>,
        server_id: String,
        http: reqwest::Client,
    ) -> Self {
        Self {
            secrets,
            server_id,
            http,
            lock: Mutex::new(()),
        }
    }
}

pub(crate) fn load_tokens(
    secrets: &McpSecretStore,
    server_id: &str,
) -> Result<Option<StoredTokens>> {
    secrets
        .get(server_id, OAUTH_SECRET_KEY)?
        .map(|raw| serde_json::from_str(&raw).map_err(Error::from))
        .transpose()
}

pub(crate) fn save_tokens(
    secrets: &McpSecretStore,
    server_id: &str,
    tokens: &StoredTokens,
) -> Result<()> {
    secrets.put(server_id, OAUTH_SECRET_KEY, &serde_json::to_string(tokens)?)
}

#[async_trait]
impl BearerSource for TokenSource {
    async fn bearer(&self, force_refresh: bool) -> Result<Option<String>> {
        let _guard = self.lock.lock().await;
        let Some(stored) = load_tokens(&self.secrets, &self.server_id)? else {
            return Ok(None);
        };
        if !force_refresh && !stored.expiring(now_secs()) {
            return Ok(Some(stored.access_token));
        }
        let refreshed = refresh_tokens(&self.http, &stored).await?;
        save_tokens(&self.secrets, &self.server_id, &refreshed)?;
        Ok(Some(refreshed.access_token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let (verifier, challenge) = pkce_pair();
        assert!(verifier.len() >= 43);
        assert_eq!(pkce_challenge(&verifier), challenge);
    }

    #[test]
    fn parses_bearer_challenge_parameters() {
        let params = challenge_params(
            r#"Bearer realm="mcp", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", scope="read write", error=invalid_token"#,
        );
        assert_eq!(
            params["resource_metadata"],
            "https://mcp.example.com/.well-known/oauth-protected-resource"
        );
        assert_eq!(params["scope"], "read write");
        assert_eq!(params["error"], "invalid_token");
        assert!(challenge_params("Basic realm=\"x\"").is_empty());
    }

    #[test]
    fn canonical_resource_drops_fragment_and_bare_slash() {
        let url = Url::parse("https://MCP.example.com/#frag").unwrap();
        assert_eq!(canonical_resource(&url), "https://mcp.example.com");
        let url = Url::parse("https://mcp.example.com/mcp").unwrap();
        assert_eq!(canonical_resource(&url), "https://mcp.example.com/mcp");
    }

    #[test]
    fn insecure_endpoints_are_rejected_except_loopback() {
        assert!(
            require_secure(&Url::parse("http://auth.example.com/token").unwrap(), "x").is_err()
        );
        assert!(require_secure(&Url::parse("http://127.0.0.1:9/token").unwrap(), "x").is_ok());
        assert!(
            require_secure(&Url::parse("https://auth.example.com/token").unwrap(), "x").is_ok()
        );
    }

    #[tokio::test]
    async fn loopback_callback_ignores_wrong_state_and_returns_code() {
        let callback = LoopbackCallback::bind().await.unwrap();
        let redirect = callback.redirect_uri.clone();
        let waiter =
            tokio::spawn(async move { callback.wait("expected", Duration::from_secs(5)).await });
        let http = reqwest::Client::new();
        let wrong = http
            .get(format!("{redirect}?code=bad&state=other"))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
        let ok = http
            .get(format!("{redirect}?code=good&state=expected"))
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let params = waiter.await.unwrap().unwrap();
        assert_eq!(params.code.as_deref(), Some("good"));
    }
}
