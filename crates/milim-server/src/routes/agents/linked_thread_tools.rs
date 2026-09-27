//! Tools for reading and messaging linked chats.

use super::*;

pub(super) fn register_linked_thread_tools(
    registry: &mut ToolRegistry,
    state: AppState,
    control: Arc<crate::control::RunManager>,
    context: AgentMemoryContext,
    policy: &ToolRunPolicy,
) {
    let Some(origin_thread_id) = context.thread_id.clone() else {
        return;
    };
    let grants = context.linked_thread_grants.clone();
    registry.register(Arc::new(LinkedThreadListTool {
        control: control.clone(),
        origin_thread_id: origin_thread_id.clone(),
        grants: grants.clone(),
    }));
    registry.register(Arc::new(LinkedThreadReadTool {
        control: control.clone(),
        grants: grants.clone(),
    }));
    if !policy.plan_mode && policy.approval != ToolApprovalPolicy::Guarded {
        let origin_run_id = context.message_id.clone();
        let destinations = grants
            .iter()
            .map(|grant| {
                format!(
                    "{} [{}] in {} using {}/{}",
                    grant.title,
                    grant.target_thread_id,
                    grant.project.as_deref().unwrap_or("unknown project"),
                    grant.model.as_deref().unwrap_or("unknown model"),
                    grant.runtime
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        registry.register(Arc::new(LinkedThreadSendTool {
            state,
            control: control.clone(),
            origin_thread_id: origin_thread_id.clone(),
            origin_run_id: origin_run_id.clone(),
            grants: grants.clone(),
            description: format!(
                "Send a message to a linked chat. The chat handles it as a new request with its own model and settings: it starts a turn, joins the turn in progress when it can, or waits in the chat's queue. The reply comes back to this chat later; if you need it before you can continue, pass the returned exchange_id to linked_thread_wait. Linked chats: {destinations}"
            ),
        }));
        if let Some(origin_run_id) = origin_run_id {
            registry.register(Arc::new(LinkedThreadWaitTool {
                control,
                origin_thread_id,
                origin_run_id,
                grants,
            }));
        }
    }
}

pub(super) struct LinkedThreadListTool {
    control: Arc<crate::control::RunManager>,
    origin_thread_id: String,
    grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

#[async_trait]
impl Tool for LinkedThreadListTool {
    fn name(&self) -> &str {
        "linked_thread_list"
    }

    fn description(&self) -> &str {
        "List the chats linked to this one (id, title, project, and model) and the status of messages exchanged with them."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Parallel
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
        self.control
            .linked_thread_list(&self.origin_thread_id, &self.grants)
    }
}

pub(super) struct LinkedThreadReadTool {
    control: Arc<crate::control::RunManager>,
    grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

#[derive(Deserialize)]
pub(super) struct LinkedThreadReadArgs {
    target_thread_id: String,
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default = "default_linked_thread_read_limit")]
    limit: usize,
}

pub(super) fn default_linked_thread_read_limit() -> usize {
    20
}

#[async_trait]
impl Tool for LinkedThreadReadTool {
    fn name(&self) -> &str {
        "linked_thread_read"
    }

    fn description(&self) -> &str {
        "Read a linked chat's user and assistant messages as they stood when this run started, oldest first. Hidden prompts, reasoning, and tool activity are not included. When has_more is true, pass next_after_seq as after_seq to read on."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_thread_id": { "type": "string", "description": "Linked chat id from linked_thread_list." },
                "after_seq": { "type": "integer", "minimum": 0, "description": "Only messages after this sequence number." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 }
            },
            "required": ["target_thread_id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Parallel
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: LinkedThreadReadArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid linked_thread_read arguments: {error}"))
        })?;
        self.control.linked_thread_read(
            &self.grants,
            args.target_thread_id.trim(),
            args.after_seq,
            args.limit,
        )
    }
}

pub(super) struct LinkedThreadSendTool {
    state: AppState,
    control: Arc<crate::control::RunManager>,
    origin_thread_id: String,
    origin_run_id: Option<String>,
    grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
    description: String,
}

#[derive(Deserialize)]
pub(super) struct LinkedThreadSendArgs {
    target_thread_id: String,
    message: String,
}

#[async_trait]
impl Tool for LinkedThreadSendTool {
    fn name(&self) -> &str {
        "linked_thread_send"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_thread_id": { "type": "string", "description": "Linked chat id from linked_thread_list." },
                "message": { "type": "string", "minLength": 1, "description": "The request, written so the other chat can act on it without this conversation." }
            },
            "required": ["target_thread_id", "message"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Exclusive
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: LinkedThreadSendArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid linked_thread_send arguments: {error}"))
        })?;
        self.control
            .linked_thread_send(
                self.state.clone(),
                &self.origin_thread_id,
                self.origin_run_id.as_deref(),
                &self.grants,
                args.target_thread_id.trim(),
                &args.message,
            )
            .await
    }
}

pub(super) struct LinkedThreadWaitTool {
    pub(super) control: Arc<crate::control::RunManager>,
    pub(super) origin_thread_id: String,
    pub(super) origin_run_id: String,
    pub(super) grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

#[derive(Deserialize)]
pub(super) struct LinkedThreadWaitArgs {
    exchange_id: String,
    #[serde(default = "default_linked_thread_wait_ms")]
    timeout_ms: u64,
}

pub(super) fn default_linked_thread_wait_ms() -> u64 {
    DEFAULT_LINKED_THREAD_WAIT_MS
}

#[async_trait]
impl Tool for LinkedThreadWaitTool {
    fn name(&self) -> &str {
        "linked_thread_wait"
    }

    fn description(&self) -> &str {
        "Wait for the reply to a message sent with linked_thread_send, only when you need it to continue. If the wait times out you can wait again; a reply that arrives later still reaches this chat."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "exchange_id": { "type": "string", "minLength": 1, "description": "The exchange_id linked_thread_send returned." },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 100,
                    "maximum": crate::control::MAX_LINKED_THREAD_WAIT_MS,
                    "default": DEFAULT_LINKED_THREAD_WAIT_MS
                }
            },
            "required": ["exchange_id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn deadline_for_call(&self, args: &Value) -> Option<Duration> {
        let wait_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_LINKED_THREAD_WAIT_MS)
            .min(crate::control::MAX_LINKED_THREAD_WAIT_MS);
        Some(Duration::from_millis(wait_ms) + TOOL_WAIT_GRACE)
    }

    fn waits_on_other_runs(&self) -> bool {
        true
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Parallel
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: LinkedThreadWaitArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid linked_thread_wait arguments: {error}"))
        })?;
        self.control
            .linked_thread_wait(
                &self.origin_thread_id,
                &self.origin_run_id,
                &self.grants,
                args.exchange_id.trim(),
                args.timeout_ms,
            )
            .await
    }
}
