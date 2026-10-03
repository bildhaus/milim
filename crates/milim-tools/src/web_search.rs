//! `web_search`: keyword web search through a configured API or DuckDuckGo.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use milim_core::{Error, Result};

use crate::builtins::public_http_client;
use crate::html::strip_tags;
use crate::{Tool, ToolConcurrency, ToolEffect};

const DEFAULT_RESULTS: usize = 8;
const MAX_RESULTS: usize = 20;
const MAX_QUERY_CHARS: usize = 400;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const DUCKDUCKGO_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const BRAVE_DEFAULT_URL: &str = "https://api.search.brave.com/res/v1";
const TAVILY_DEFAULT_URL: &str = "https://api.tavily.com";
const USER_AGENT: &str = "Mozilla/5.0 (compatible; milim web_search)";

/// A keyed search API the user configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSearchProvider {
    Brave,
    Tavily,
}

impl WebSearchProvider {
    fn label(self) -> &'static str {
        match self {
            Self::Brave => "Brave Search",
            Self::Tavily => "Tavily",
        }
    }
}

/// Credentials for a keyed search API. `base_url` overrides the provider's
/// default API root when set.
#[derive(Debug, Clone)]
pub struct WebSearchApi {
    pub provider: WebSearchProvider,
    pub api_key: String,
    pub base_url: Option<String>,
}

/// Resolves the configured search API at call time, so key changes apply to
/// the next search without rebuilding the registry.
#[allow(
    clippy::double_must_use,
    reason = "async_trait expansion triggers rust-clippy#17529"
)]
#[async_trait]
pub trait WebSearchApiSource: Send + Sync {
    async fn api(&self) -> Option<WebSearchApi>;
}

/// Applies the outbound privacy policy to a query before it leaves the
/// machine: returns the text to send, or an error to refuse the search.
pub type WebSearchQueryFilter = Arc<dyn Fn(&str) -> Result<String> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Search the web and return titles, URLs, and snippets.
#[derive(Clone, Default)]
pub struct WebSearchTool {
    api_source: Option<Arc<dyn WebSearchApiSource>>,
    query_filter: Option<WebSearchQueryFilter>,
}

impl WebSearchTool {
    /// Use a keyed API when `source` yields one; DuckDuckGo otherwise.
    pub fn with_api_source(mut self, source: Arc<dyn WebSearchApiSource>) -> Self {
        self.api_source = Some(source);
        self
    }

    /// Run every query through `filter` before any request is sent.
    pub fn with_query_filter(mut self, filter: WebSearchQueryFilter) -> Self {
        self.query_filter = Some(filter);
        self
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web. Returns a numbered list of result titles, URLs, and snippets; use http_fetch to read a result."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search keywords." },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS,
                    "description": "Number of results to return (default 8)."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| Error::InvalidRequest("missing 'query' argument".to_string()))?;
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(Error::InvalidRequest(format!(
                "query must be at most {MAX_QUERY_CHARS} characters"
            )));
        }
        let max_results = match args.get("max_results") {
            None | Some(Value::Null) => DEFAULT_RESULTS,
            Some(value) => value
                .as_u64()
                .filter(|count| (1..=MAX_RESULTS as u64).contains(count))
                .ok_or_else(|| {
                    Error::InvalidRequest(format!(
                        "max_results must be an integer from 1 to {MAX_RESULTS}"
                    ))
                })? as usize,
        };
        let sent = match &self.query_filter {
            Some(filter) => filter(query)?,
            None => query.to_string(),
        };
        let api = match &self.api_source {
            Some(source) => source.api().await,
            None => None,
        };
        let (backend, mut results) = match api {
            Some(api) => {
                let results = match api.provider {
                    WebSearchProvider::Brave => search_brave(&api, &sent, max_results).await?,
                    WebSearchProvider::Tavily => search_tavily(&api, &sent, max_results).await?,
                };
                (api.provider.label(), results)
            }
            None => ("DuckDuckGo", search_duckduckgo(&sent).await?),
        };
        results.truncate(max_results);
        Ok(json!({
            "query": sent,
            "query_redacted": sent != query,
            "backend": backend,
            "results": results,
        }))
    }

    fn model_text(&self, result: &Value) -> Option<String> {
        let query = result.get("query")?.as_str()?;
        let backend = result.get("backend")?.as_str()?;
        let results = result.get("results")?.as_array()?;
        let mut text = format!("Web results for \"{query}\" via {backend}");
        if result.get("query_redacted").and_then(Value::as_bool) == Some(true) {
            text.push_str(" (query redacted by the privacy gate)");
        }
        text.push_str(":\n");
        if results.is_empty() {
            text.push_str("\nNo results.");
        }
        for (index, item) in results.iter().enumerate() {
            let field = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or("");
            text.push_str(&format!(
                "\n{}. {}\n   {}",
                index + 1,
                field("title"),
                field("url")
            ));
            let snippet = field("snippet");
            if !snippet.is_empty() {
                text.push_str(&format!("\n   {snippet}"));
            }
            text.push('\n');
        }
        Some(text.trim_end().to_string())
    }
}

async fn read_limited(mut response: reqwest::Response) -> Result<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| Error::Upstream(error.to_string()))?
    {
        let remaining = MAX_RESPONSE_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() >= remaining {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn send_checked(request: reqwest::RequestBuilder, service: &str) -> Result<String> {
    let response = request
        .send()
        .await
        .map_err(|error| Error::Upstream(format!("{service}: {error}")))?;
    let status = response.status();
    let body = read_limited(response).await?;
    if !status.is_success() {
        let detail: String = strip_tags(&body).chars().take(200).collect();
        return Err(Error::Upstream(format!(
            "{service} returned HTTP {}{}",
            status.as_u16(),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )));
    }
    Ok(body)
}

fn api_url(api: &WebSearchApi, default: &str, path: &str) -> Result<reqwest::Url> {
    let root = api
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .unwrap_or(default)
        .trim_end_matches('/');
    reqwest::Url::parse(&format!("{root}{path}"))
        .map_err(|error| Error::InvalidRequest(format!("invalid search API URL: {error}")))
}

async fn search_brave(api: &WebSearchApi, query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let mut url = api_url(api, BRAVE_DEFAULT_URL, "/web/search")?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("count", &count.to_string());
    let client = public_http_client(&url).await?;
    let body = send_checked(
        client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header("X-Subscription-Token", &api.api_key),
        "Brave Search",
    )
    .await?;
    parse_brave(&body)
}

async fn search_tavily(api: &WebSearchApi, query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let url = api_url(api, TAVILY_DEFAULT_URL, "/search")?;
    let client = public_http_client(&url).await?;
    let body = send_checked(
        client.post(url).bearer_auth(&api.api_key).json(&json!({
            "query": query,
            "max_results": count,
            "search_depth": "basic",
        })),
        "Tavily",
    )
    .await?;
    parse_tavily(&body)
}

async fn search_duckduckgo(query: &str) -> Result<Vec<SearchResult>> {
    let url = reqwest::Url::parse_with_params(DUCKDUCKGO_HTML_URL, [("q", query)])
        .map_err(|error| Error::Other(format!("search URL: {error}")))?;
    let client = public_http_client(&url).await?;
    let body = send_checked(
        client
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT, "text/html"),
        "DuckDuckGo",
    )
    .await?;
    parse_duckduckgo(&body)
}

fn parse_brave(body: &str) -> Result<Vec<SearchResult>> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| Error::Upstream(format!("Brave Search returned invalid JSON: {error}")))?;
    Ok(value
        .pointer("/web/results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| json_result(item, "description"))
                .collect()
        })
        .unwrap_or_default())
}

fn parse_tavily(body: &str) -> Result<Vec<SearchResult>> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| Error::Upstream(format!("Tavily returned invalid JSON: {error}")))?;
    Ok(value
        .get("results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| json_result(item, "content"))
                .collect()
        })
        .unwrap_or_default())
}

fn json_result(item: &Value, snippet_key: &str) -> Option<SearchResult> {
    let url = item.get("url")?.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .map(strip_tags)
            .unwrap_or_default()
    };
    Some(SearchResult {
        title: text("title"),
        url: url.to_string(),
        snippet: truncate_chars(&text(snippet_key), 400),
    })
}

/// Parse DuckDuckGo's no-JavaScript results page.
fn parse_duckduckgo(body: &str) -> Result<Vec<SearchResult>> {
    if body.contains("anomaly-modal") || body.contains("challenge-form") {
        return Err(Error::Upstream(
            "DuckDuckGo refused the automated search (rate limited or challenged). Try again later, or add a Brave Search or Tavily key under Providers.".to_string(),
        ));
    }
    let mut results = Vec::new();
    let mut search = 0;
    const TITLE_MARKER: &str = "result__a";
    while let Some(found) = body[search..].find(TITLE_MARKER) {
        let marker = search + found;
        search = marker + TITLE_MARKER.len();
        let Some(anchor_start) = body[..marker].rfind("<a") else {
            continue;
        };
        let Some(tag_close) = body[marker..].find('>').map(|end| marker + end) else {
            break;
        };
        let Some(anchor_end) = body[tag_close..].find("</a>").map(|end| tag_close + end) else {
            break;
        };
        let href = attribute(&body[anchor_start..tag_close], "href").unwrap_or_default();
        let next = body[anchor_end..]
            .find(TITLE_MARKER)
            .map(|offset| anchor_end + offset)
            .unwrap_or(body.len());
        search = anchor_end;
        let Some(url) = duckduckgo_target(&href) else {
            continue;
        };
        let snippet = body[anchor_end..next]
            .find("result__snippet")
            .map(|offset| anchor_end + offset)
            .and_then(|at| {
                let open = at + body[at..].find('>')? + 1;
                if open > next {
                    return None;
                }
                let close = ["</a>", "</div>", "</td>"]
                    .iter()
                    .filter_map(|closer| body[open..next].find(closer))
                    .min()
                    .map(|end| open + end)?;
                Some(strip_tags(&body[open..close]))
            })
            .unwrap_or_default();
        results.push(SearchResult {
            title: strip_tags(&body[tag_close + 1..anchor_end]),
            url,
            snippet: truncate_chars(&snippet, 400),
        });
    }
    Ok(results)
}

/// Resolve DuckDuckGo's `/l/?uddg=` redirect wrapper and drop ad links.
fn duckduckgo_target(href: &str) -> Option<String> {
    let href = crate::html::decode_entities(href.trim());
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href
    };
    let url = reqwest::Url::parse(&absolute).ok()?;
    let is_duckduckgo = url
        .host_str()
        .is_some_and(|host| host == "duckduckgo.com" || host.ends_with(".duckduckgo.com"));
    if !is_duckduckgo {
        return matches!(url.scheme(), "http" | "https").then(|| url.to_string());
    }
    if url.path().starts_with("/y.js") {
        return None;
    }
    let target = url
        .query_pairs()
        .find(|(key, _)| key == "uddg")
        .map(|(_, value)| value.into_owned())?;
    let target = reqwest::Url::parse(&target).ok()?;
    if target
        .host_str()
        .is_some_and(|host| host.ends_with("duckduckgo.com"))
        && target.path().starts_with("/y.js")
    {
        return None;
    }
    matches!(target.scheme(), "http" | "https").then(|| target.to_string())
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name) {
        let at = search + found;
        search = at + name.len();
        let boundary = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = lower[search..].trim_start();
        if !boundary || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let value = tag[value_start..].trim_start();
        let (quote, body) = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => (Some(quote), &value[1..]),
            _ => (None, value),
        };
        let end = match quote {
            Some(quote) => body.find(quote).unwrap_or(body.len()),
            None => body
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(body.len()),
        };
        return Some(body[..end].to_string());
    }
    None
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG_PAGE: &str = r#"<html><body>
<div class="result results_links results_links_deep result--ad">
  <a rel="nofollow" class="result__a" href="https://duckduckgo.com/y.js?ad_domain=ads.example&amp;u3=x">Sponsored</a>
  <a class="result__snippet" href="https://duckduckgo.com/y.js?ad">Buy now</a>
</div>
<div class="result results_links results_links_deep web-result">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2Flearn&amp;rut=abc">Learn <b>Rust</b> - Rust Programming Language</a>
  </h2>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rust-lang.org%2Flearn">Get started with <b>Rust</b> &amp; its book.</a>
</div>
<div class="result results_links results_links_deep web-result">
  <h2 class="result__title"><a rel="nofollow" class="result__a" href="https://doc.rust-lang.org/book/">The Rust Book</a></h2>
  <div class="result__snippet">An introductory book.</div>
</div>
</body></html>"#;

    #[test]
    fn duckduckgo_html_parses_results_and_skips_ads() {
        let results = parse_duckduckgo(DDG_PAGE).unwrap();
        assert_eq!(
            results,
            vec![
                SearchResult {
                    title: "Learn Rust - Rust Programming Language".into(),
                    url: "https://www.rust-lang.org/learn".into(),
                    snippet: "Get started with Rust & its book.".into(),
                },
                SearchResult {
                    title: "The Rust Book".into(),
                    url: "https://doc.rust-lang.org/book/".into(),
                    snippet: "An introductory book.".into(),
                },
            ]
        );
    }

    #[test]
    fn duckduckgo_challenge_page_is_an_error() {
        let error = parse_duckduckgo("<div class=\"anomaly-modal\"></div>").unwrap_err();
        assert!(error.to_string().contains("Brave Search or Tavily"));
    }

    #[test]
    fn brave_and_tavily_json_parse() {
        let brave = parse_brave(
            r#"{"web":{"results":[
                {"title":"Tokio","url":"https://tokio.rs/","description":"An <strong>async</strong> runtime."},
                {"title":"No URL","description":"skipped"}
            ]}}"#,
        )
        .unwrap();
        assert_eq!(brave.len(), 1);
        assert_eq!(brave[0].snippet, "An async runtime.");
        assert!(parse_brave(r#"{"query":{}}"#).unwrap().is_empty());

        let tavily = parse_tavily(
            r#"{"results":[{"title":"Axum","url":"https://docs.rs/axum","content":"Web framework."}]}"#,
        )
        .unwrap();
        assert_eq!(tavily[0].title, "Axum");
        assert_eq!(tavily[0].url, "https://docs.rs/axum");
        assert!(parse_tavily("not json").is_err());
    }

    #[test]
    fn model_text_numbers_results() {
        let tool = WebSearchTool::default();
        let text = tool
            .model_text(&json!({
                "query": "rust",
                "query_redacted": false,
                "backend": "DuckDuckGo",
                "results": [
                    { "title": "Rust", "url": "https://www.rust-lang.org/", "snippet": "A language." },
                    { "title": "Crates", "url": "https://crates.io/", "snippet": "" }
                ]
            }))
            .unwrap();
        assert_eq!(
            text,
            "Web results for \"rust\" via DuckDuckGo:\n\n1. Rust\n   https://www.rust-lang.org/\n   A language.\n\n2. Crates\n   https://crates.io/"
        );
    }

    #[tokio::test]
    async fn arguments_and_query_filter_are_checked_before_any_request() {
        let tool = WebSearchTool::default();
        assert!(tool.invoke(json!({})).await.is_err());
        assert!(tool.invoke(json!({ "query": "  " })).await.is_err());
        assert!(tool
            .invoke(json!({ "query": "rust", "max_results": 0 }))
            .await
            .is_err());
        assert!(tool
            .invoke(json!({ "query": "rust", "max_results": 21 }))
            .await
            .is_err());

        let blocked = WebSearchTool::default().with_query_filter(Arc::new(|query: &str| {
            if query.contains('@') {
                Err(Error::InvalidRequest("blocked by the privacy gate".into()))
            } else {
                Ok(query.to_string())
            }
        }));
        let error = blocked
            .invoke(json!({ "query": "mail me@example.com" }))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("privacy gate"));
    }

    #[test]
    fn tool_metadata_is_read_only_and_parallel() {
        let tool = WebSearchTool::default();
        assert_eq!(tool.effect(), ToolEffect::ReadOnly);
        assert_eq!(tool.concurrency(), ToolConcurrency::Parallel);
    }
}
