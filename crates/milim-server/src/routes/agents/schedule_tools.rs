//! Tools that manage scheduled automations from chat.

use super::*;

pub(crate) fn register_schedule_tools(
    reg: &mut ToolRegistry,
    store: Arc<milim_automation::ScheduleStore>,
    workspace: Option<PathBuf>,
    privacy: &str,
) {
    reg.register(Arc::new(ScheduleCreateTool {
        store: store.clone(),
        workspace: workspace.map(|path| path.to_string_lossy().to_string()),
        privacy: privacy.to_string(),
    }));
    reg.register(Arc::new(ScheduleUpdateTool {
        store: store.clone(),
    }));
    reg.register(Arc::new(ScheduleListTool {
        store: store.clone(),
    }));
    reg.register(Arc::new(ScheduleDeleteTool { store }));
}

pub(super) struct ScheduleCreateTool {
    store: Arc<milim_automation::ScheduleStore>,
    workspace: Option<String>,
    privacy: String,
}

pub(super) struct ScheduleUpdateTool {
    store: Arc<milim_automation::ScheduleStore>,
}

pub(super) struct ScheduleListTool {
    store: Arc<milim_automation::ScheduleStore>,
}

pub(super) struct ScheduleDeleteTool {
    store: Arc<milim_automation::ScheduleStore>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScheduleCreateToolArgs {
    name: String,
    cron: String,
    prompt: String,
    #[serde(default)]
    attachments: Vec<milim_automation::ScheduleAttachment>,
    #[serde(default)]
    agent_id: Option<String>,
    model: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScheduleUpdateToolArgs {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cron: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    attachments: Option<Vec<milim_automation::ScheduleAttachment>>,
    #[serde(default)]
    agent_id: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScheduleListToolArgs {
    #[serde(default)]
    enabled_only: bool,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScheduleDeleteToolArgs {
    id: String,
}

pub(crate) fn trim_required_tool_arg(value: String, name: &str) -> milim_core::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(Error::InvalidRequest(format!("{name} is required")));
    }
    Ok(value)
}

pub(super) fn trim_optional_agent_id(agent_id: Option<String>) -> Option<String> {
    agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn find_schedule(
    store: &milim_automation::ScheduleStore,
    id: &str,
) -> milim_core::Result<milim_automation::Schedule> {
    store
        .list()?
        .into_iter()
        .find(|schedule| schedule.id == id)
        .ok_or_else(|| Error::ModelNotFound(format!("schedule {id}")))
}

#[async_trait]
impl Tool for ScheduleCreateTool {
    fn name(&self) -> &str {
        "schedule_create"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Create a cron automation that runs a saved agent prompt. Use this when the user asks to schedule, automate, run periodically, or create a cron from chat. Cron expressions must use six fields: sec min hour day month dow."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short human-readable automation name." },
                "cron": { "type": "string", "description": "Six-field cron expression: sec min hour day month dow." },
                "prompt": { "type": "string", "description": "Self-contained prompt to run each time the automation fires." },
                "attachments": {
                    "type": "array",
                    "description": "Optional file attachments whose text content should be included when the automation runs.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "mime": { "type": "string" },
                            "size": { "type": "integer" },
                            "content": { "type": "string" },
                            "dataUrl": { "type": "string" },
                            "truncated": { "type": "boolean" },
                            "sourcePath": { "type": "string" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                },
                "agent_id": { "type": ["string", "null"], "description": "Optional named agent id. Omit for the default agent." },
                "model": { "type": "string", "description": "Model id for unattended runs." },
                "enabled": { "type": "boolean", "description": "Whether the automation should start enabled. Defaults to true." }
            },
            "required": ["name", "cron", "prompt", "model"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleCreateToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_create arguments: {e}"))
        })?;
        let name = trim_required_tool_arg(args.name, "name")?;
        let cron = trim_required_tool_arg(args.cron, "cron")?;
        let prompt = trim_required_tool_arg(args.prompt, "prompt")?;
        let model = provider_schedule_model(trim_required_tool_arg(args.model, "model")?)?;
        let schedule = self.store.create_with_run_context(
            &name,
            &cron,
            trim_optional_agent_id(args.agent_id),
            &model,
            &prompt,
            args.attachments,
            args.enabled,
            self.workspace.clone(),
            &self.privacy,
            "local",
        )?;
        Ok(json!({ "ok": true, "schedule": schedule }))
    }
}

#[async_trait]
impl Tool for ScheduleUpdateTool {
    fn name(&self) -> &str {
        "schedule_update"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Update an existing cron automation by id. Use null agent_id to clear the named agent and omit fields that should stay unchanged."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Schedule id to update." },
                "name": { "type": "string" },
                "cron": { "type": "string", "description": "Six-field cron expression: sec min hour day month dow." },
                "prompt": { "type": "string" },
                "attachments": {
                    "type": "array",
                    "description": "Replacement file attachments for the automation.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "mime": { "type": "string" },
                            "size": { "type": "integer" },
                            "content": { "type": "string" },
                            "dataUrl": { "type": "string" },
                            "truncated": { "type": "boolean" },
                            "sourcePath": { "type": "string" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                },
                "agent_id": { "type": ["string", "null"], "description": "Named agent id, or null to clear." },
                "model": { "type": "string", "description": "Model id for unattended runs." },
                "enabled": { "type": "boolean" }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleUpdateToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_update arguments: {e}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let current = find_schedule(&self.store, &id)?;
        let name = args
            .name
            .map(|value| trim_required_tool_arg(value, "name"))
            .transpose()?
            .unwrap_or_else(|| current.name.clone());
        let cron = args
            .cron
            .map(|value| trim_required_tool_arg(value, "cron"))
            .transpose()?
            .unwrap_or_else(|| current.cron.clone());
        let prompt = args
            .prompt
            .map(|value| trim_required_tool_arg(value, "prompt"))
            .transpose()?
            .unwrap_or_else(|| current.prompt.clone());
        let model = args
            .model
            .map(|value| trim_required_tool_arg(value, "model"))
            .transpose()?
            .unwrap_or_else(|| current.model.clone());
        let model = provider_schedule_model(model)?;
        let attachments = args
            .attachments
            .unwrap_or_else(|| current.attachments.clone());
        let agent_id = match args.agent_id {
            None => current.agent_id.clone(),
            Some(Value::Null) => None,
            Some(Value::String(value)) => trim_optional_agent_id(Some(value)),
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "agent_id must be a string or null".to_string(),
                ))
            }
        };
        let schedule = self.store.update(milim_automation::ScheduleUpdate {
            id: &id,
            name: &name,
            cron: &cron,
            agent_id,
            model: &model,
            prompt: &prompt,
            attachments,
            enabled: args.enabled.unwrap_or(current.enabled),
            workspace: current.workspace,
            privacy: current.privacy,
            timezone_mode: current.timezone_mode,
            created_unix: current.created_unix,
            last_run: current.last_run,
        })?;
        Ok(json!({ "ok": true, "schedule": schedule }))
    }
}

#[async_trait]
impl Tool for ScheduleListTool {
    fn name(&self) -> &str {
        "schedule_list"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn description(&self) -> &str {
        "List saved cron automations."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "enabled_only": { "type": "boolean", "description": "Only return enabled schedules." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleListToolArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid schedule_list arguments: {error}"))
        })?;
        let mut schedules = self.store.list()?;
        if args.enabled_only {
            schedules.retain(|schedule| schedule.enabled);
        }
        if let Some(limit) = args.limit {
            schedules.truncate(limit.clamp(1, 50));
        }
        Ok(json!({ "ok": true, "schedules": schedules }))
    }
}

#[async_trait]
impl Tool for ScheduleDeleteTool {
    fn name(&self) -> &str {
        "schedule_delete"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Delete a saved cron automation by id."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Schedule id to delete." }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleDeleteToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_delete arguments: {e}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let deleted = self.store.delete(&id)?;
        if !deleted {
            return Err(Error::ModelNotFound(format!("schedule {id}")));
        }
        Ok(json!({ "ok": true, "deleted": true, "id": id }))
    }
}
