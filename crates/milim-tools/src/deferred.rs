//! Deferred tools: registered and callable, but left out of the model
//! request's tool list. `tool_search` finds them and returns their schemas;
//! `call_tool` runs one by name. The agent loop resolves a `call_tool` call
//! to the tool it names before approval, hooks, and execution, so the request
//! keeps one stable tool list (and its provider prompt cache) however many
//! deferred tools a run has.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use milim_core::{Error, Result};

use crate::{Tool, ToolConcurrency, ToolEffect};

/// Name of the deferred-tool search tool.
pub const TOOL_SEARCH_TOOL: &str = "tool_search";
/// Name of the deferred-tool dispatch tool.
pub const CALL_TOOL_TOOL: &str = "call_tool";

const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS: usize = 20;

/// Searches one registry's deferred tools. Rebuilt whenever the registry's
/// tool set changes, so it never offers a tool the run can no longer call.
pub(crate) struct ToolSearchTool {
    pub(crate) tools: Vec<Arc<dyn Tool>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolSearchArgs {
    query: String,
    #[serde(default)]
    max_results: Option<usize>,
}

impl ToolSearchTool {
    fn find(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools
            .iter()
            .find(|tool| tool.name() == name)
            .or_else(|| {
                self.tools
                    .iter()
                    .find(|tool| tool.aliases().iter().any(|alias| alias == name))
            })
    }

    /// Deferred tools matching the query's keywords, best first. A term in
    /// a tool's name counts more than one in its description; ties keep name
    /// order so a query always returns the same tools.
    fn search(&self, query: &str, limit: usize) -> Vec<&Arc<dyn Tool>> {
        let terms = words(query);
        let mut scored: Vec<(usize, &Arc<dyn Tool>)> = self
            .tools
            .iter()
            .filter_map(|tool| {
                let name = tool.name().to_lowercase();
                let name_words = words(&name);
                let description = tool.description().to_lowercase();
                let score: usize = terms
                    .iter()
                    .map(|term| {
                        if name_words.contains(term) {
                            3
                        } else if name.contains(term.as_str()) {
                            2
                        } else if description.contains(term.as_str()) {
                            1
                        } else {
                            0
                        }
                    })
                    .sum();
                (score > 0).then_some((score, tool))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name().cmp(b.1.name())));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, tool)| tool)
            .collect()
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn describe(tool: &Arc<dyn Tool>) -> Value {
    json!({
        "name": tool.name(),
        "description": tool.description(),
        "input_schema": tool.input_schema(),
    })
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        TOOL_SEARCH_TOOL
    }

    fn description(&self) -> &str {
        "Find deferred tools (listed by name in the system prompt) and load their descriptions and input schemas. Pass keywords as `query`, or `select:name1,name2` for exact names. Then run a tool with `call_tool`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to match against deferred tool names and descriptions, or `select:name1,name2` to load exact tools."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_RESULTS,
                    "default": DEFAULT_MAX_RESULTS,
                    "description": "Most tools to return for a keyword query."
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
        let args: ToolSearchArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid tool_search arguments: {error}"))
        })?;
        let query = args.query.trim();
        if query.is_empty() {
            return Err(Error::InvalidRequest("query is required".into()));
        }
        if let Some(names) = query.strip_prefix("select:") {
            let mut tools = Vec::new();
            let mut not_found = Vec::new();
            for name in names
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                match self.find(name) {
                    Some(tool) => tools.push(describe(tool)),
                    None => not_found.push(name.to_string()),
                }
            }
            let mut result = json!({ "tools": tools });
            if !not_found.is_empty() {
                result["not_found"] = json!(not_found);
            }
            return Ok(result);
        }
        let limit = args
            .max_results
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .clamp(1, MAX_RESULTS);
        let tools: Vec<Value> = self
            .search(query, limit)
            .into_iter()
            .map(describe)
            .collect();
        let empty = tools.is_empty();
        let mut result = json!({ "tools": tools });
        if empty {
            result["hint"] = json!(
                "No deferred tool matched. Try other keywords, or `select:` a name from the system prompt's list."
            );
        }
        Ok(result)
    }
}

/// Runs a deferred tool by name. The agent loop resolves each `call_tool`
/// call to the tool it names (see `ToolRegistry::resolve_proxy_call`), so
/// approval, hooks, MCP App UI, and the transcript see the real tool; this
/// tool itself only carries the model-facing schema.
pub(crate) struct CallToolTool;

#[async_trait]
impl Tool for CallToolTool {
    fn name(&self) -> &str {
        CALL_TOOL_TOOL
    }

    fn description(&self) -> &str {
        "Run a deferred tool by name. Load its input schema with `tool_search` first; `arguments` must match that schema. Approval and results work as if you called the tool directly."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Exact name of the deferred tool to run." },
                "arguments": {
                    "type": "object",
                    "description": "Arguments for that tool, matching its input schema.",
                    "additionalProperties": true
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
        Err(Error::InvalidRequest(format!(
            "`{name}` is not a tool this run can call through call_tool"
        )))
    }
}

/// The tool a `call_tool` call runs and its encoded arguments, or `None`
/// when `arguments` do not name one (the call then fails `call_tool`'s own
/// schema). `arguments` may be an object, a JSON-encoded object, or absent.
pub(crate) fn proxy_target(arguments: &str) -> Option<(String, String)> {
    let args: Value = serde_json::from_str(arguments).ok()?;
    let name = args.get("name")?.as_str()?.trim();
    if name.is_empty() || name == CALL_TOOL_TOOL {
        return None;
    }
    let arguments = match args.get("arguments") {
        None | Some(Value::Null) => "{}".to_string(),
        Some(Value::String(encoded)) => encoded.clone(),
        Some(value) => value.to_string(),
    };
    Some((name.to_string(), arguments))
}
