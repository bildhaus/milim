//! `memory_register` and `list_agents`.

use super::*;

pub(crate) struct MemoryRegisterTool {
    pub(crate) store: Arc<milim_memory::MemoryStore>,
    pub(crate) context: AgentMemoryContext,
}

pub(super) struct ListAgentsTool {
    pub(super) store: Arc<milim_agents::AgentStore>,
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn description(&self) -> &str {
        "List reusable Milim Agents and compact tool/skill capability summaries. System prompts are never returned."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
        let agents = self
            .store
            .list()?
            .into_iter()
            .map(|agent| {
                json!({
                    "id": agent.id,
                    "name": agent.name,
                    "description": agent.description,
                    "avatar": agent.avatar,
                    "tools": {
                        "mode": agent.tool_mode,
                        "count": agent.enabled_tools.len(),
                        "names": agent.enabled_tools,
                    },
                    "skills": {
                        "mode": agent.skill_mode,
                        "count": agent.enabled_skills.len(),
                        "names": agent.enabled_skills,
                    }
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "agents": agents }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemoryRegisterArgs {
    #[serde(default)]
    scope: Option<String>,
    content: String,
    #[serde(default)]
    title: Option<String>,
}

#[async_trait]
impl Tool for MemoryRegisterTool {
    fn name(&self) -> &str {
        "memory_register"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Save concise durable context to Personal or Project memory. Use this only for facts, decisions, preferences, and project context likely to help future turns."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["personal", "project"],
                    "description": "Where to store the memory. Defaults to project when a workspace folder exists, otherwise personal."
                },
                "content": { "type": "string", "description": "One or two sentences with the useful durable context." },
                "title": { "type": "string", "description": "Optional short human-readable title." }
            },
            "required": ["content"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: MemoryRegisterArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid memory_register arguments: {e}"))
        })?;
        let content = trim_required_tool_arg(args.content, "content")?;
        let title = args
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                content
                    .lines()
                    .next()
                    .unwrap_or("Memory")
                    .chars()
                    .take(80)
                    .collect()
            });
        let requested_scope = args
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| {
                if self.context.project_locator.is_some() {
                    "project".to_string()
                } else {
                    "personal".to_string()
                }
            });

        let (scope_kind, locator, label) = match requested_scope.as_str() {
            "project" => {
                let locator = self.context.project_locator.clone().ok_or_else(|| {
                    Error::InvalidRequest(
                        "project memory requires an active project folder".to_string(),
                    )
                })?;
                let label = self
                    .context
                    .project_label
                    .clone()
                    .unwrap_or_else(|| locator.clone());
                ("project".to_string(), locator, label)
            }
            "personal" => (
                "global".to_string(),
                "personal".to_string(),
                "Personal".to_string(),
            ),
            _ => {
                return Err(Error::InvalidRequest(
                    "memory_register scope must be personal or project".to_string(),
                ))
            }
        };

        let registration = self
            .store
            .register(
                &self.context.model,
                milim_memory::MemoryScopeInput {
                    kind: scope_kind,
                    label,
                    locator,
                },
                milim_memory::MemoryNodeInput {
                    kind: "fact".to_string(),
                    title,
                    body: content,
                    confidence: 0.85,
                    source: "agent".to_string(),
                },
                Vec::new(),
                milim_memory::MemoryEventInput {
                    thread_id: self.context.thread_id.clone().unwrap_or_default(),
                    message_id: self.context.message_id.clone().unwrap_or_default(),
                    summary: String::new(),
                },
            )
            .await?;
        Ok(json!({
            "ok": true,
            "memory": registration.node,
            "scope": registration.scope,
            "memory_notice": registration.notice
        }))
    }
}
