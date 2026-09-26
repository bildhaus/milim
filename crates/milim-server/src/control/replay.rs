//! Completion request (de)serialization used to persist and replay provider
//! requests byte for byte.

use milim_core::{Error, Result};
use milim_inference::{CompletionRequest, SamplingParams};
use serde_json::{json, Value};

pub(crate) fn completion_request_value(request: &CompletionRequest) -> Result<Value> {
    let mut value = json!({
        "model": request.model,
        "messages": request.messages,
        "tools": request.tools,
        "tool_choice": request.tool_choice,
        "response_format": request.response_format,
        "prompt": request.prompt,
        "suffix": request.suffix,
        "sampling": {
            "temperature": request.sampling.temperature,
            "top_p": request.sampling.top_p,
            "max_tokens": request.sampling.max_tokens,
            "stop": request.sampling.stop,
            "seed": request.sampling.seed,
            "frequency_penalty": request.sampling.frequency_penalty,
            "presence_penalty": request.sampling.presence_penalty,
            "top_k": request.sampling.top_k,
            "min_p": request.sampling.min_p,
            "repetition_penalty": request.sampling.repetition_penalty,
            "thinking_token_budget": request.sampling.thinking_token_budget,
        },
        "reasoning_effort": request.reasoning_effort,
    });
    // Added only when set, so ledgers recorded before the key existed
    // re-serialize to the same bytes.
    if let Some(key) = &request.sampling.prompt_cache_key {
        value["sampling"]["prompt_cache_key"] = json!(key);
    }
    Ok(value)
}

/// Inverse of [`completion_request_value`]: rebuilds the provider request a
/// run ledger stored, so re-serializing it yields the same bytes.
pub(crate) fn completion_request_from_value(value: &Value) -> Result<CompletionRequest> {
    fn field<T: serde::de::DeserializeOwned + Default>(value: &Value, key: &str) -> Result<T> {
        match value.get(key) {
            None | Some(Value::Null) => Ok(T::default()),
            Some(field) => serde_json::from_value(field.clone()).map_err(|error| {
                Error::Other(format!(
                    "stored provider request has invalid {key}: {error}"
                ))
            }),
        }
    }
    let sampling = value.get("sampling").unwrap_or(&Value::Null);
    let optional = |key: &str| value.get(key).filter(|field| !field.is_null()).cloned();
    Ok(CompletionRequest {
        model: field(value, "model")?,
        messages: field(value, "messages")?,
        tools: field(value, "tools")?,
        tool_choice: optional("tool_choice"),
        response_format: optional("response_format"),
        prompt: field(value, "prompt")?,
        suffix: field(value, "suffix")?,
        sampling: SamplingParams {
            temperature: field(sampling, "temperature")?,
            top_p: field(sampling, "top_p")?,
            max_tokens: field(sampling, "max_tokens")?,
            stop: field(sampling, "stop")?,
            seed: field(sampling, "seed")?,
            frequency_penalty: field(sampling, "frequency_penalty")?,
            presence_penalty: field(sampling, "presence_penalty")?,
            top_k: field(sampling, "top_k")?,
            min_p: field(sampling, "min_p")?,
            repetition_penalty: field(sampling, "repetition_penalty")?,
            thinking_token_budget: field(sampling, "thinking_token_budget")?,
            prompt_cache_key: field(sampling, "prompt_cache_key")?,
        },
        reasoning_effort: field(value, "reasoning_effort")?,
    })
}
