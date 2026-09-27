//! Frozen run configuration: resolves a thread's settings into the immutable
//! config a turn runs with, including instructions, limits, and sampling.

use milim_control_contract::{
    AgentSnapshotV1, ControlAttachmentV1, FrozenRunConfigV1, GenerationSettingsV1, RunLimitsV1,
};
use milim_core::api::openai::ReasoningEffort;
use milim_core::{Error, Result};
use milim_inference::SamplingParams;
use milim_storage::{ControlThreadRecord, UserDataStore};
use serde_json::{Map, Value};

use super::native_sessions::{
    runtime_adapter, runtime_cursor_field, runtime_model, runtime_session_field,
};
use super::MODEL_FAVORITES_SETTINGS_KEY;
use crate::AppState;

const MAX_GLOBAL_INSTRUCTIONS_CHARS: usize = 32 * 1024;

pub(super) fn resolve_frozen_config(
    state: &AppState,
    store: &UserDataStore,
    thread: &ControlThreadRecord,
    attachments: Vec<ControlAttachmentV1>,
) -> Result<FrozenRunConfigV1> {
    let value: Value = serde_json::from_str(&thread.session_json)
        .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
    let settings = value.get("settings").and_then(Value::as_object);
    let instructions = settings
        .and_then(|settings| settings.get("instructions"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let selected_model = value
        .get("worker")
        .and_then(Value::as_object)
        .and_then(|worker| worker.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            settings
                .and_then(|settings| settings.get("model"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| Error::InvalidRequest("thread has no selected model".into()))?
        .to_string();
    let workspace = settings
        .and_then(|settings| settings.get("folder"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let privacy = setting_string(settings, "privacy", "off");
    let approval_mode = setting_string(settings, "toolApproval", "review");
    let plan_mode = settings
        .and_then(|settings| settings.get("planMode"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sandbox = settings
        .and_then(|settings| settings.get("sandbox"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let computer_use = settings
        .and_then(|settings| settings.get("computerUse"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let memory = settings
        .and_then(|settings| settings.get("memory"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let delegation_policy = setting_string(settings, "delegationPolicy", "ask");
    let worker_model = setting_string(settings, "workerModel", "");
    let agent_id = settings
        .and_then(|settings| settings.get("activeAgentId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let agent = agent_id
        .and_then(|id| {
            state
                .agents
                .as_ref()
                .and_then(|store| store.get(id).ok().flatten())
        })
        .map(|agent| AgentSnapshotV1 {
            id: agent.id,
            name: agent.name,
            description: agent.description,
            avatar: agent.avatar,
            system_prompt: agent.system_prompt,
            tool_mode: agent.tool_mode,
            enabled_tools: agent.enabled_tools,
            skill_mode: agent.skill_mode,
            enabled_skills: agent.enabled_skills,
        });
    let adapter = runtime_adapter(&selected_model).to_string();
    let model = runtime_model(&selected_model).to_string();
    // The account this turn runs as. `auto` is resolved here, at acceptance,
    // so the run is frozen against one account even if another finishes a
    // cooldown while the turn is in flight.
    let selected_profile = settings
        .and_then(|settings| settings.get("accountProfiles"))
        .and_then(Value::as_object)
        .and_then(|profiles| profiles.get(&adapter))
        .and_then(Value::as_str);
    let account_profile = crate::account_profiles::resolve(Some(store), &adapter, selected_profile);
    let account_runtime = value.get("accountRuntime").and_then(Value::as_object);
    // A native session lives inside one account's configuration home, so only
    // the binding recorded for this account is resumable. Another account's
    // binding is left untouched and this turn starts a fresh native session
    // with the thread's full visible history.
    let native_session_id = runtime_session_field(&adapter, &account_profile.id)
        .ok()
        .and_then(|field| account_runtime?.get(&field)?.as_str().map(str::to_string));
    let native_session_cursor = runtime_cursor_field(&adapter, &account_profile.id)
        .ok()
        .and_then(|field| account_runtime?.get(&field)?.as_str().map(str::to_string));
    let reasoning_effort = settings
        .and_then(|settings| settings.get("reasoningEffortOverrides"))
        .and_then(Value::as_object)
        .and_then(|overrides| overrides.get(&selected_model))
        .and_then(Value::as_str)
        .map(str::to_string);
    let generation = settings
        .and_then(|settings| settings.get("generationOverrides"))
        .and_then(Value::as_object)
        .and_then(|overrides| overrides.get(&selected_model))
        .map(normalize_generation_settings)
        .unwrap_or_default();
    let enabled_tools = agent
        .as_ref()
        .map(|agent| agent.enabled_tools.clone())
        .unwrap_or_default();
    let tool_mode = agent
        .as_ref()
        .map(|agent| agent.tool_mode.clone())
        .unwrap_or_else(default_control_tool_mode);
    let enabled_skills = agent
        .as_ref()
        .map(|agent| agent.enabled_skills.clone())
        .unwrap_or_default();
    let skill_mode = agent
        .as_ref()
        .map(|agent| agent.skill_mode.clone())
        .unwrap_or_else(default_control_skill_mode);
    Ok(FrozenRunConfigV1 {
        model,
        global_instructions: global_instructions(store),
        instructions,
        workspace,
        privacy,
        approval_mode,
        plan_mode,
        sandbox,
        computer_use,
        memory,
        delegation_policy,
        worker_model,
        agent,
        tool_mode,
        enabled_tools,
        skill_mode,
        enabled_skills,
        attachments,
        native_session_id,
        native_session_cursor,
        reasoning_effort,
        generation,
        run_limits: if adapter == "provider" {
            configured_run_limits(store, settings)?
        } else {
            None
        },
        adapter,
        account_profile_id: account_profile.id,
        account_profile_label: account_profile.label,
        linked_thread_grants: Vec::new(),
        claimed_mailbox_ids: Vec::new(),
    })
}

pub(super) fn configured_run_limits(
    store: &UserDataStore,
    settings: Option<&Map<String, Value>>,
) -> Result<Option<RunLimitsV1>> {
    let global = store
        .get_json(MODEL_FAVORITES_SETTINGS_KEY)?
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let value = settings
        .and_then(|settings| settings.get("runLimits"))
        .or_else(|| global.as_ref()?.get("state")?.get("runLimits"));
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if !value.is_object() {
        return Err(Error::InvalidRequest(
            "Run limits must be an object.".into(),
        ));
    }
    let integer = |key: &str, max: u64| -> Result<Option<u32>> {
        match value.get(key).filter(|value| !value.is_null()) {
            None => Ok(None),
            Some(value) => value
                .as_u64()
                .filter(|value| (1..=max).contains(value))
                .map(|value| Some(value as u32))
                .ok_or_else(|| {
                    Error::InvalidRequest(format!(
                        "{key} must be a whole number between 1 and {max}."
                    ))
                }),
        }
    };
    let max_cost_usd = match value.get("maxCostUsd").filter(|value| !value.is_null()) {
        None => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|value| value.is_finite() && *value > 0.0 && *value <= 1_000_000.0)
                .ok_or_else(|| {
                    Error::InvalidRequest(
                        "Run spend threshold must be positive and at most $1,000,000.".into(),
                    )
                })?,
        ),
    };
    Ok(Some(RunLimitsV1 {
        max_steps: integer("maxSteps", 10_000)?,
        max_seconds: integer("maxSeconds", 86_400)?,
        max_cost_usd,
    }))
}

fn global_instructions(store: &UserDataStore) -> String {
    store
        .get_json(MODEL_FAVORITES_SETTINGS_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<Value>(&value).ok())
        .and_then(|value| value.get("state")?.get("globalInstructions").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
        .chars()
        .take(MAX_GLOBAL_INSTRUCTIONS_CHARS)
        .collect()
}

pub(super) fn frozen_run_instructions(config: &FrozenRunConfigV1) -> String {
    compose_labeled_instructions(
        "Milim global instructions",
        &config.global_instructions,
        "Thread instructions",
        &config.instructions,
    )
}

pub(super) fn frozen_harness_instructions(config: &FrozenRunConfigV1) -> String {
    match config.agent.as_ref() {
        Some(agent) => compose_labeled_instructions(
            "Milim global instructions",
            &config.global_instructions,
            "Agent instructions",
            &agent.system_prompt,
        ),
        None => frozen_run_instructions(config),
    }
}

/// The instruction layers a native Milim run follows. As in the account
/// runtime harness, an active Agent's instructions take the place of the
/// thread's own instructions.
pub(super) fn frozen_instruction_layers(
    config: &FrozenRunConfigV1,
) -> crate::workspace_context::InstructionLayers {
    let (agent, thread) = match config.agent.as_ref() {
        Some(agent) => (agent.system_prompt.clone(), String::new()),
        None => (String::new(), config.instructions.clone()),
    };
    crate::workspace_context::InstructionLayers {
        milim: config.global_instructions.clone(),
        agent,
        thread,
    }
}

pub(super) fn compose_labeled_instructions(
    first_label: &str,
    first: &str,
    second_label: &str,
    second: &str,
) -> String {
    let first = first.trim();
    let second = second.trim();
    match (first.is_empty(), second.is_empty()) {
        (true, true) => String::new(),
        (false, true) => first.to_string(),
        (true, false) => second.to_string(),
        (false, false) => format!("{first_label}:\n{first}\n\n{second_label}:\n{second}"),
    }
}

pub(super) fn normalize_generation_settings(value: &Value) -> GenerationSettingsV1 {
    let value = value.as_object();
    let bounded_f32 = |key: &str, min: f64, max: f64, include_min: bool| {
        value
            .and_then(|value| value.get(key))
            .and_then(Value::as_f64)
            .filter(|number| {
                number.is_finite()
                    && if include_min {
                        *number >= min
                    } else {
                        *number > min
                    }
                    && *number <= max
            })
            .map(|number| number as f32)
    };
    let stop = value
        .and_then(|value| value.get("stop"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty() && item.chars().count() <= 256)
        .take(8)
        .map(str::to_string)
        .collect();
    GenerationSettingsV1 {
        max_tokens: value
            .and_then(|value| value.get("maxTokens"))
            .and_then(Value::as_u64)
            .filter(|number| (1..=1_000_000).contains(number))
            .and_then(|number| u32::try_from(number).ok()),
        temperature: bounded_f32("temperature", 0.0, 2.0, true),
        top_p: bounded_f32("topP", 0.0, 1.0, false),
        seed: value
            .and_then(|value| value.get("seed"))
            .and_then(Value::as_i64),
        stop,
        frequency_penalty: bounded_f32("frequencyPenalty", -2.0, 2.0, true),
        presence_penalty: bounded_f32("presencePenalty", -2.0, 2.0, true),
        top_k: value
            .and_then(|value| value.get("topK"))
            .and_then(Value::as_i64)
            .filter(|number| *number == -1 || (1..=1_000_000).contains(number))
            .and_then(|number| i32::try_from(number).ok()),
        min_p: bounded_f32("minP", 0.0, 1.0, true),
        repetition_penalty: bounded_f32("repetitionPenalty", 0.0, 2.0, false),
        thinking_token_budget: value
            .and_then(|value| value.get("thinkingTokenBudget"))
            .and_then(Value::as_u64)
            .filter(|number| *number <= 1_000_000)
            .and_then(|number| u32::try_from(number).ok()),
    }
}

/// Frozen generation controls for a run in `thread_id`. The thread id keys
/// the provider prompt cache so every turn of the thread shares it.
pub(super) fn sampling_from_generation(
    generation: &GenerationSettingsV1,
    thread_id: &str,
) -> SamplingParams {
    SamplingParams {
        temperature: generation.temperature,
        top_p: generation.top_p,
        max_tokens: generation.max_tokens,
        stop: generation.stop.clone(),
        seed: generation.seed,
        frequency_penalty: generation.frequency_penalty,
        presence_penalty: generation.presence_penalty,
        top_k: generation.top_k,
        min_p: generation.min_p,
        repetition_penalty: generation.repetition_penalty,
        thinking_token_budget: generation.thinking_token_budget,
        prompt_cache_key: Some(thread_id.to_string()),
    }
}

fn setting_string(settings: Option<&Map<String, Value>>, key: &str, fallback: &str) -> String {
    settings
        .and_then(|settings| settings.get(key))
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

pub(super) fn thread_agent_id(thread: &ControlThreadRecord) -> Option<String> {
    serde_json::from_str::<Value>(&thread.session_json)
        .ok()?
        .get("settings")?
        .get("activeAgentId")?
        .as_str()
        .map(str::to_string)
}

pub(super) fn parse_reasoning_effort(value: &str) -> Option<ReasoningEffort> {
    serde_json::from_value(Value::String(value.to_string())).ok()
}

fn default_control_tool_mode() -> String {
    "all".to_string()
}

fn default_control_skill_mode() -> String {
    "auto".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn instruction_layers_keep_global_and_let_an_agent_replace_thread_instructions() {
        let mut config: FrozenRunConfigV1 = serde_json::from_value(json!({
            "model": "model-x", "global_instructions": "Global.", "instructions": "Thread.",
            "workspace": null, "privacy": "off", "approval_mode": "review", "plan_mode": false,
            "sandbox": false, "computer_use": false, "memory": false, "delegation_policy": "ask",
            "worker_model": "", "agent": null, "enabled_tools": [], "enabled_skills": [],
            "attachments": [], "native_session_id": null, "reasoning_effort": null,
            "adapter": "provider"
        }))
        .unwrap();
        let layers = frozen_instruction_layers(&config);
        assert_eq!(
            (
                layers.milim.as_str(),
                layers.agent.as_str(),
                layers.thread.as_str()
            ),
            ("Global.", "", "Thread.")
        );
        config.agent = Some(AgentSnapshotV1 {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            description: String::new(),
            avatar: String::new(),
            system_prompt: "Review carefully.".into(),
            tool_mode: "all".into(),
            enabled_tools: Vec::new(),
            skill_mode: "auto".into(),
            enabled_skills: Vec::new(),
        });
        let layers = frozen_instruction_layers(&config);
        assert_eq!(
            (
                layers.milim.as_str(),
                layers.agent.as_str(),
                layers.thread.as_str()
            ),
            ("Global.", "Review carefully.", "")
        );
    }
}
