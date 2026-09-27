//! An OpenAI-compatible upstream backend.
//!
//! Translates a backend-neutral [`CompletionRequest`] into an OpenAI Chat
//! Completions request, forwards it to any OpenAI-compatible base URL
//! (OpenAI, Ollama's `/v1`, vLLM, OpenRouter, …), and re-parses the SSE
//! stream back into [`StreamEvent`]s. OpenAI's own reasoning models on
//! api.openai.com use the Responses API instead (see
//! [`crate::openai_responses`]).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use milim_core::api::openai::{
    ChatCompletionRequest, ChatMessage, Content, ContentPart, DeltaFunction, DeltaToolCall, Model,
    ModelCapabilities, ModelReasoningMetadata, ModelsResponse, ReasoningEffort, StreamOptions,
    StringOrArray, Tool, Usage,
};
use milim_core::provider_error::upstream_stream_error;
use milim_core::{Error, Result};
use serde_json::{json, Map, Value};

use crate::http_error::HttpFailure;
use crate::openai_responses::{self, is_openai_reasoning_model, ResponsesTurn};
use crate::service::{
    normalize_finish_reason, CompletionRequest, DeltaEvent, EventStream, ModelService, StreamEvent,
};
use crate::stall;

/// This adapter's key in `ChatMessage::provider_state` for OpenRouter
/// `reasoning_details`.
const OPENROUTER_STATE_KEY: &str = "openrouter";

/// Forwards generation to an OpenAI-compatible HTTP endpoint.
#[derive(Debug, Clone)]
pub struct RemoteBackend {
    /// Base URL including the version segment, e.g. `https://api.openai.com/v1`.
    base_url: String,
    api_key: Option<String>,
    label: String,
    client: reqwest::Client,
    /// Generation streams, whose idle budget is enforced per request.
    stream_client: reqwest::Client,
    /// Models OpenAI refused reasoning summaries for (they need a verified
    /// organization); later requests for them stop asking.
    models_without_summaries: Arc<Mutex<HashSet<String>>>,
}

#[cfg(not(test))]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_millis(50);

const OPENROUTER_HTTP_REFERER: &str = "https://milim.ai/";
const OPENROUTER_TITLE: &str = "milim";

#[cfg(not(test))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(100);

impl RemoteBackend {
    /// Build a backend pointing at `base_url` (no trailing slash) with an
    /// optional bearer key. `label` is the [`ModelService::name`] value.
    pub fn new(
        label: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self::with_client(label, base_url, api_key, default_client())
    }

    fn with_client(
        label: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
        client: reqwest::Client,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            label: label.into(),
            client,
            stream_client: stall::streaming_client(),
            models_without_summaries: Arc::default(),
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn api_root_endpoint(&self, path: &str) -> String {
        let root = self.base_url.strip_suffix("/v1").unwrap_or(&self.base_url);
        format!(
            "{}/{}",
            root.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = if self.is_openrouter() {
            rb.header("HTTP-Referer", OPENROUTER_HTTP_REFERER)
                .header("X-OpenRouter-Title", OPENROUTER_TITLE)
        } else {
            rb
        };
        match &self.api_key {
            Some(k) => rb.bearer_auth(k),
            None => rb,
        }
    }

    /// Build the OpenAI wire body from a neutral request.
    fn build_body(&self, req: &CompletionRequest, stream: bool) -> ChatCompletionRequest {
        let s = &req.sampling;
        let mut extra = serde_json::Map::new();
        let reasoning_effort = self.reasoning_effort_for_body(req.reasoning_effort);
        if let Some(effort) = reasoning_effort.openrouter {
            extra.insert(
                "reasoning".to_string(),
                if effort == ReasoningEffort::None {
                    json!({ "effort": effort.as_str(), "exclude": true })
                } else {
                    json!({ "effort": effort.as_str() })
                },
            );
        }
        if let Some(value) = s.top_k {
            extra.insert("top_k".to_string(), json!(value));
        }
        if let Some(value) = s.min_p {
            extra.insert("min_p".to_string(), json!(value));
        }
        if let Some(value) = s.repetition_penalty {
            extra.insert("repetition_penalty".to_string(), json!(value));
        }
        if let Some(value) = s.thinking_token_budget {
            extra.insert("thinking_token_budget".to_string(), json!(value));
        }
        // Only OpenAI itself is sent `prompt_cache_key`; other compatible
        // servers may reject unknown fields.
        if self.is_openai() {
            if let Some(key) = prompt_cache_key(req) {
                extra.insert("prompt_cache_key".to_string(), json!(key));
            }
        }
        // OpenAI replaced `max_tokens` with `max_completion_tokens`, and its
        // reasoning models (also behind proxies) reject `max_tokens`,
        // `temperature`, and `top_p`. OpenRouter maps these fields itself.
        let reasoning_model = is_openai_reasoning_model(&req.model);
        let completion_tokens = !self.is_openrouter() && (self.is_openai() || reasoning_model);
        ChatCompletionRequest {
            model: req.model.clone(),
            // Provider state is replayed by the adapter that owns it, never
            // sent as a message field.
            messages: req
                .messages
                .iter()
                .cloned()
                .map(|mut message| {
                    message.provider_state = None;
                    message
                })
                .collect(),
            temperature: s.temperature.filter(|_| !reasoning_model),
            top_p: s.top_p.filter(|_| !reasoning_model),
            max_tokens: s.max_tokens.filter(|_| !completion_tokens),
            max_completion_tokens: s.max_tokens.filter(|_| completion_tokens),
            n: None,
            stream: Some(stream),
            stop: (!s.stop.is_empty()).then(|| StringOrArray::Array(s.stop.clone())),
            frequency_penalty: s.frequency_penalty,
            presence_penalty: s.presence_penalty,
            seed: s.seed,
            tools: (!req.tools.is_empty()).then(|| req.tools.clone()),
            tool_choice: req.tool_choice.clone(),
            response_format: req.response_format.clone(),
            reasoning_effort: reasoning_effort.openai,
            stream_options: stream.then_some(StreamOptions {
                include_usage: Some(true),
            }),
            extra,
        }
    }

    /// The Chat Completions wire body, with OpenRouter's per-message extras.
    fn chat_body(&self, req: &CompletionRequest, stream: bool) -> Result<Value> {
        let mut body = serde_json::to_value(self.build_body(req, stream))?;
        if self.is_openrouter() {
            if let Some(Value::Array(wire)) = body.get_mut("messages") {
                apply_openrouter_messages(wire, &req.messages, &req.model);
            }
        }
        Ok(body)
    }

    /// OpenAI's reasoning models on api.openai.com go through the Responses
    /// API; everything else (proxies included) stays on Chat Completions.
    fn should_use_openai_responses(&self, req: &CompletionRequest) -> bool {
        self.is_openai() && is_openai_reasoning_model(&req.model)
    }

    fn reasoning_effort_for_body(&self, effort: Option<ReasoningEffort>) -> RemoteReasoningEffort {
        let Some(effort) = effort.filter(|e| !e.is_auto()) else {
            return RemoteReasoningEffort::default();
        };
        if self.is_openrouter() {
            return RemoteReasoningEffort {
                openrouter: Some(effort),
                openai: None,
            };
        }
        if self.is_lm_studio() {
            return RemoteReasoningEffort::default();
        }
        RemoteReasoningEffort {
            openrouter: None,
            openai: Some(effort),
        }
    }

    fn is_openrouter(&self) -> bool {
        self.label.trim().eq_ignore_ascii_case("openrouter")
            || self
                .base_url
                .to_ascii_lowercase()
                .contains("openrouter.ai/")
    }

    fn is_openai(&self) -> bool {
        reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| {
                url.host_str()
                    .map(|host| host.eq_ignore_ascii_case("api.openai.com"))
            })
            .unwrap_or(false)
    }

    fn is_ollama(&self) -> bool {
        let label = self.label.to_ascii_lowercase();
        let base = self.base_url.to_ascii_lowercase();
        label.contains("ollama") || base.contains(":11434/")
    }

    fn is_lm_studio(&self) -> bool {
        let label = self.label.to_ascii_lowercase();
        let base = self.base_url.to_ascii_lowercase();
        label.contains("lm studio") || label.contains("lmstudio") || base.contains(":1234/")
    }

    async fn vllm_reasoning_efforts(&self) -> Vec<ReasoningEffort> {
        let Ok(response) = self
            .auth(self.client.get(self.api_root_endpoint("openapi.json")))
            .send()
            .await
        else {
            return Vec::new();
        };
        if !response.status().is_success() {
            return Vec::new();
        }
        let Ok(spec) = response.json::<Value>().await else {
            return Vec::new();
        };
        vllm_reasoning_efforts_from_openapi(&spec)
    }

    fn ollama_generate_endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        let root = base
            .strip_suffix("/v1")
            .or_else(|| base.strip_suffix("/api"))
            .unwrap_or(base)
            .trim_end_matches('/');
        format!("{root}/api/generate")
    }

    fn ollama_show_endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        let root = base
            .strip_suffix("/v1")
            .or_else(|| base.strip_suffix("/api"))
            .unwrap_or(base)
            .trim_end_matches('/');
        format!("{root}/api/show")
    }

    fn lm_studio_api_endpoint(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let root = base
            .strip_suffix("/v1")
            .or_else(|| base.strip_suffix("/api/v1"))
            .unwrap_or(base)
            .trim_end_matches('/');
        format!("{root}/api/v1/{}", path.trim_start_matches('/'))
    }

    fn should_use_lm_studio_responses(&self, req: &CompletionRequest) -> bool {
        if !self.is_lm_studio() {
            return false;
        }
        let Some(effort) = req.reasoning_effort.filter(|e| !e.is_auto()) else {
            return false;
        };
        if (!req.tools.is_empty() || req.tool_choice.is_some())
            && matches!(
                effort,
                ReasoningEffort::None
                    | ReasoningEffort::Low
                    | ReasoningEffort::Medium
                    | ReasoningEffort::High
                    | ReasoningEffort::On
            )
        {
            return true;
        }
        is_gpt_oss_model(&req.model)
            && matches!(
                effort,
                ReasoningEffort::Low | ReasoningEffort::Medium | ReasoningEffort::High
            )
    }

    fn should_use_lm_studio_native_chat(&self, req: &CompletionRequest) -> bool {
        self.is_lm_studio()
            && req.reasoning_effort.is_some_and(|e| !e.is_auto())
            && !request_has_image_parts(req)
    }
}

#[derive(Default)]
struct RemoteReasoningEffort {
    openrouter: Option<ReasoningEffort>,
    openai: Option<ReasoningEffort>,
}

fn default_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .read_timeout(DEFAULT_READ_TIMEOUT)
        .build()
        .expect("valid reqwest client timeout configuration")
}

fn has_modality(modalities: &[String], modality: &str) -> bool {
    modalities
        .iter()
        .any(|item| item.trim().eq_ignore_ascii_case(modality))
}

fn normalize_model_capabilities(model: &mut Model) {
    let Some(architecture) = model.architecture.as_ref() else {
        return;
    };
    let derived = ModelCapabilities {
        image_input: Some(has_modality(&architecture.input_modalities, "image")),
        image_output: Some(has_modality(&architecture.output_modalities, "image")),
        video_output: Some(has_modality(&architecture.output_modalities, "video")),
        tool_use: None,
    };
    match model.capabilities.as_mut() {
        Some(capabilities) => {
            if capabilities.image_input.is_none() {
                capabilities.image_input = derived.image_input;
            }
            if capabilities.image_output.is_none() {
                capabilities.image_output = derived.image_output;
            }
            if capabilities.video_output.is_none() {
                capabilities.video_output = derived.video_output;
            }
            if capabilities.tool_use.is_none() {
                capabilities.tool_use = derived.tool_use;
            }
        }
        None => model.capabilities = Some(derived),
    }
}

#[async_trait]
impl ModelService for RemoteBackend {
    fn name(&self) -> &str {
        &self.label
    }

    fn requires_privacy_gate(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return true;
        };
        let Some(host) = url.host_str() else {
            return true;
        };
        if host.eq_ignore_ascii_case("localhost")
            || host
                .to_ascii_lowercase()
                .strip_suffix(".localhost")
                .is_some()
        {
            return false;
        }
        host.parse::<std::net::IpAddr>()
            .map(|ip| !ip.is_loopback())
            .unwrap_or(true)
    }

    async fn list_models(&self) -> Result<Vec<Model>> {
        let resp = self
            .auth(self.client.get(self.endpoint("models")))
            .send()
            .await
            .map_err(upstream)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "{} GET /models -> {}",
                self.label,
                resp.status()
            )));
        }
        let mut parsed: ModelsResponse = resp.json().await.map_err(upstream)?;
        let is_vllm = parsed
            .data
            .iter()
            .any(|model| model.owned_by.eq_ignore_ascii_case("vllm"));
        if is_vllm {
            let efforts = self.vllm_reasoning_efforts().await;
            for model in &mut parsed.data {
                if model.reasoning.is_none()
                    && looks_reasoning_model(&model.id)
                    && !efforts.is_empty()
                {
                    model.reasoning = Some(ModelReasoningMetadata {
                        default_effort: efforts
                            .contains(&ReasoningEffort::Medium)
                            .then_some(ReasoningEffort::Medium),
                        default_enabled: Some(true),
                        mandatory: Some(!efforts.contains(&ReasoningEffort::None)),
                        supported_efforts: efforts.clone(),
                    });
                }
            }
        }
        if self.is_lm_studio() {
            if let Ok(metadata) = self.lm_studio_native_metadata().await {
                for model in &mut parsed.data {
                    if let Some(meta) = metadata.reasoning.get(&model.id).cloned() {
                        model.reasoning = Some(meta);
                    }
                    if let Some(capabilities) = metadata.capabilities.get(&model.id).cloned() {
                        merge_model_capabilities(model, capabilities);
                    }
                }
            }
        }
        if self.is_ollama() {
            for model in &mut parsed.data {
                if let Ok(Some(capabilities)) = self.ollama_model_capabilities(&model.id).await {
                    merge_model_capabilities(model, capabilities);
                }
            }
        }
        for model in &mut parsed.data {
            normalize_model_capabilities(model);
        }
        Ok(parsed.data)
    }

    async fn ollama_keep_alive(&self, model: &str, keep_alive: Option<Value>) -> Result<bool> {
        if !self.is_ollama() {
            return Ok(false);
        }
        let mut body = Map::new();
        body.insert("model".to_string(), Value::String(model.to_string()));
        body.insert("prompt".to_string(), Value::String(String::new()));
        body.insert("stream".to_string(), Value::Bool(false));
        if let Some(value) = keep_alive {
            body.insert("keep_alive".to_string(), value);
        }
        let resp = self
            .auth(self.client.post(self.ollama_generate_endpoint()))
            .json(&Value::Object(body))
            .send()
            .await
            .map_err(upstream)?;
        if !resp.status().is_success() {
            return Err(crate::http_error::http_status_error(
                &self.label,
                "api/generate keep_alive",
                resp,
            )
            .await);
        }
        Ok(true)
    }

    async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
        crate::image_input::validate_request_images(&req)?;
        if req.prompt.is_some() {
            return self.stream_legacy_completion(req).await;
        }
        if self.should_use_lm_studio_responses(&req) {
            return self.stream_lm_studio_responses(req).await;
        }
        if self.should_use_lm_studio_native_chat(&req) {
            return self.stream_lm_studio_native_chat(req).await;
        }
        if self.should_use_openai_responses(&req) {
            return self.stream_openai_responses(req).await;
        }
        let body = self.chat_body(&req, true)?;
        let idle = stall::stream_idle_timeout(&req.model, req.reasoning_effort);
        let resp = stall::send(
            self.auth(self.stream_client.post(self.endpoint("chat/completions")))
                .json(&body),
            idle,
            &self.label,
            "chat/completions",
        )
        .await?;

        if !resp.status().is_success() {
            return Err(crate::http_error::http_status_error(
                &self.label,
                "chat/completions",
                resp,
            )
            .await);
        }

        let label = self.label.clone();
        let mut chat = ChatStream::new(self.is_openrouter().then(|| req.model.clone()));
        // OpenAI itself always returns structured calls.
        if !self.is_openai() && req.tool_choice.as_ref().and_then(Value::as_str) != Some("none") {
            chat = chat.with_text_tool_calls(&req.tools);
        }
        let stream = async_stream::stream! {
            let mut lines = stall::SseLines::new(resp.bytes_stream(), idle, &label);
            let mut saw_done = false;

            while let Some(line) = lines.next().await {
                let line = match line {
                    Ok(line) => line,
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                };
                match parse_sse_line(&line) {
                    LineOutcome::Done => {
                        saw_done = true;
                        break;
                    }
                    LineOutcome::Error(e) => {
                        yield Err(e.into_error(&label, "chat/completions"));
                        return;
                    }
                    LineOutcome::Event(chunk) => {
                        let delta = chat.apply(&chunk);
                        if !delta.is_empty() {
                            yield Ok(StreamEvent::Delta(delta));
                        }
                    }
                    LineOutcome::Ignore => {}
                }
            }

            // Local servers sometimes omit `[DONE]` or the finish reason, but
            // a stream with neither was cut off.
            if !saw_done && chat.finish_reason.is_none() {
                yield Err(stall::ended_early());
                return;
            }
            for event in chat.finish() {
                yield Ok(event);
            }
        };

        Ok(Box::pin(stream))
    }

    async fn embed(&self, model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
        #[derive(serde::Serialize)]
        struct EmbedReq<'a> {
            model: &'a str,
            input: Vec<String>,
        }
        #[derive(Deserialize)]
        struct EmbedResp {
            data: Vec<EmbedItem>,
        }
        #[derive(Deserialize)]
        struct EmbedItem {
            embedding: Vec<f32>,
            #[serde(default)]
            index: usize,
        }

        let resp = self
            .auth(self.client.post(self.endpoint("embeddings")))
            .json(&EmbedReq {
                model,
                input: inputs,
            })
            .send()
            .await
            .map_err(upstream)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "{} embeddings -> {}",
                self.label,
                resp.status()
            )));
        }
        let mut parsed: EmbedResp = resp.json().await.map_err(upstream)?;
        parsed.data.sort_by_key(|i| i.index);
        Ok(parsed.data.into_iter().map(|i| i.embedding).collect())
    }
}

impl RemoteBackend {
    async fn ollama_model_capabilities(&self, model: &str) -> Result<Option<ModelCapabilities>> {
        let resp = self
            .auth(self.client.post(self.ollama_show_endpoint()))
            .json(&json!({ "model": model }))
            .send()
            .await
            .map_err(upstream)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "{} POST /api/show -> {}",
                self.label,
                resp.status()
            )));
        }
        let parsed: OllamaShowResponse = resp.json().await.map_err(upstream)?;
        Ok(ollama_capabilities(&parsed.capabilities))
    }

    async fn stream_legacy_completion(&self, req: CompletionRequest) -> Result<EventStream> {
        let body = build_legacy_completion_body(&req, true)?;
        let idle = stall::stream_idle_timeout(&req.model, req.reasoning_effort);
        let resp = stall::send(
            self.auth(self.stream_client.post(self.endpoint("completions")))
                .json(&body),
            idle,
            &self.label,
            "completions",
        )
        .await?;

        if !resp.status().is_success() {
            return Err(
                crate::http_error::http_status_error(&self.label, "completions", resp).await,
            );
        }

        let label = self.label.clone();
        let stream = async_stream::stream! {
            let mut lines = stall::SseLines::new(resp.bytes_stream(), idle, &label);
            let mut saw_done = false;
            let mut last_finish: Option<String> = None;
            let mut last_usage: Option<Usage> = None;

            while let Some(line) = lines.next().await {
                let line = match line {
                    Ok(line) => line,
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                };
                match parse_completion_sse_line(&line) {
                    CompletionLineOutcome::Done => {
                        saw_done = true;
                        break;
                    }
                    CompletionLineOutcome::Event(value) => {
                        if let Some(e) = StreamErrorPayload::from_value(&value) {
                            yield Err(e.into_error(&label, "completions"));
                            return;
                        }
                        if let Some(text) = value.pointer("/choices/0/text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                yield Ok(StreamEvent::Delta(DeltaEvent::text(text)));
                            }
                        }
                        if let Some(finish) = value.pointer("/choices/0/finish_reason").and_then(Value::as_str) {
                            last_finish = Some(finish.to_string());
                        }
                        if let Some(usage) = completion_usage(&value) {
                            last_usage = Some(usage);
                        }
                    }
                    CompletionLineOutcome::Ignore => {}
                }
            }

            if !saw_done && last_finish.is_none() {
                yield Err(stall::ended_early());
                return;
            }
            yield Ok(StreamEvent::Done {
                finish_reason: normalize_finish_reason(last_finish.as_deref()).to_string(),
                usage: last_usage.unwrap_or_default(),
            });
        };

        Ok(Box::pin(stream))
    }

    async fn stream_openai_responses(&self, req: CompletionRequest) -> Result<EventStream> {
        let idle = stall::stream_idle_timeout(&req.model, req.reasoning_effort);
        let mut summaries = self
            .models_without_summaries
            .lock()
            .is_ok_and(|models| !models.contains(&req.model));
        let resp = loop {
            let body = openai_responses::build_body(&req, summaries, prompt_cache_key(&req))?;
            let resp = stall::send(
                self.auth(self.stream_client.post(self.endpoint("responses")))
                    .json(&body),
                idle,
                &self.label,
                "responses",
            )
            .await?;
            if resp.status().is_success() {
                break resp;
            }
            let failure = HttpFailure::read(resp).await;
            // Unverified organizations may not request reasoning summaries;
            // continue without them rather than fail every request.
            if summaries
                && failure.status == reqwest::StatusCode::BAD_REQUEST
                && openai_responses::rejects_reasoning_summary(&failure.body)
            {
                if let Ok(mut models) = self.models_without_summaries.lock() {
                    models.insert(req.model.clone());
                }
                summaries = false;
                continue;
            }
            return Err(failure.into_error(&self.label, "responses"));
        };

        let label = self.label.clone();
        let mut turn = ResponsesTurn::new(&label, &req.model);
        let stream = async_stream::stream! {
            let mut lines = stall::SseLines::new(resp.bytes_stream(), idle, &label);

            while let Some(line) = lines.next().await {
                let line = match line {
                    Ok(line) => line,
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                };
                let ResponsesLineOutcome::Event(value) = parse_responses_sse_line(&line) else {
                    continue;
                };
                match turn.handle(&value) {
                    Ok(events) => {
                        for event in events {
                            yield Ok(event);
                        }
                    }
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                }
                if turn.is_finished() {
                    return;
                }
            }

            yield Err(stall::ended_early());
        };

        Ok(Box::pin(stream))
    }

    async fn stream_lm_studio_responses(&self, req: CompletionRequest) -> Result<EventStream> {
        let body = build_lm_studio_responses_body(&req, true)?;
        let idle = stall::stream_idle_timeout(&req.model, req.reasoning_effort);
        let resp = stall::send(
            self.auth(self.stream_client.post(self.endpoint("responses")))
                .json(&body),
            idle,
            &self.label,
            "responses",
        )
        .await?;

        if !resp.status().is_success() {
            return Err(crate::http_error::http_status_error(&self.label, "responses", resp).await);
        }

        let label = self.label.clone();
        let stream = async_stream::stream! {
            let mut lines = stall::SseLines::new(resp.bytes_stream(), idle, &label);
            let mut usage = Usage::default();
            let mut saw_done = false;
            let mut saw_tool_call = false;

            while let Some(line) = lines.next().await {
                let line = match line {
                    Ok(line) => line,
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                };
                match parse_responses_sse_line(&line) {
                    ResponsesLineOutcome::Done => {
                        saw_done = true;
                    }
                    ResponsesLineOutcome::Event(value) => match responses_event_to_stream_event(&value) {
                        Ok(Some(StreamEvent::Delta(delta))) => {
                            saw_tool_call |= !delta.tool_calls.is_empty();
                            if !delta.is_empty() {
                                yield Ok(StreamEvent::Delta(delta));
                            }
                        }
                        Ok(Some(StreamEvent::Done { usage: done_usage, .. })) => {
                            usage = done_usage;
                            saw_done = true;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            yield Err(e);
                            return;
                        }
                    },
                    ResponsesLineOutcome::Ignore => {}
                }
                if saw_done {
                    break;
                }
            }

            if !saw_done {
                yield Err(stall::ended_early());
                return;
            }
            yield Ok(StreamEvent::Done {
                finish_reason: if saw_tool_call { "tool_calls" } else { "stop" }.to_string(),
                usage,
            });
        };

        Ok(Box::pin(stream))
    }

    async fn stream_lm_studio_native_chat(&self, req: CompletionRequest) -> Result<EventStream> {
        let body = build_lm_studio_native_chat_body(&req, true)?;
        let idle = stall::stream_idle_timeout(&req.model, req.reasoning_effort);
        let resp = stall::send(
            self.auth(self.stream_client.post(self.lm_studio_api_endpoint("chat")))
                .json(&body),
            idle,
            &self.label,
            "api/v1/chat",
        )
        .await?;

        if !resp.status().is_success() {
            return Err(
                crate::http_error::http_status_error(&self.label, "api/v1/chat", resp).await,
            );
        }

        let label = self.label.clone();
        let stream = async_stream::stream! {
            let mut lines = stall::SseLines::new(resp.bytes_stream(), idle, &label);
            let mut usage = Usage::default();
            let mut saw_done = false;

            while let Some(line) = lines.next().await {
                let line = match line {
                    Ok(line) => line,
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                };
                match parse_native_sse_line(&line) {
                    NativeLineOutcome::Event(value) => match native_chat_event_to_stream_event(&value) {
                        Ok(Some(StreamEvent::Delta(delta))) => {
                            if !delta.is_empty() {
                                yield Ok(StreamEvent::Delta(delta));
                            }
                        }
                        Ok(Some(StreamEvent::Done { usage: done_usage, .. })) => {
                            usage = done_usage;
                            saw_done = true;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            yield Err(e);
                            return;
                        }
                    },
                    NativeLineOutcome::Ignore => {}
                }
                if saw_done {
                    break;
                }
            }

            if !saw_done {
                yield Err(stall::ended_early());
                return;
            }
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage,
            });
        };

        Ok(Box::pin(stream))
    }

    async fn lm_studio_native_metadata(&self) -> Result<LmStudioNativeMetadata> {
        let resp = self
            .auth(self.client.get(self.lm_studio_api_endpoint("models")))
            .send()
            .await
            .map_err(upstream)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "{} GET /api/v1/models -> {}",
                self.label,
                resp.status()
            )));
        }
        let parsed: LmStudioNativeModelsResponse = resp.json().await.map_err(upstream)?;
        Ok(lm_studio_native_metadata_map(parsed))
    }
}

fn merge_model_capabilities(model: &mut Model, incoming: ModelCapabilities) {
    match model.capabilities.as_mut() {
        Some(capabilities) => {
            if capabilities.image_input.is_none() {
                capabilities.image_input = incoming.image_input;
            }
            if capabilities.image_output.is_none() {
                capabilities.image_output = incoming.image_output;
            }
            if capabilities.video_output.is_none() {
                capabilities.video_output = incoming.video_output;
            }
            if capabilities.tool_use.is_none() {
                capabilities.tool_use = incoming.tool_use;
            }
        }
        None => model.capabilities = Some(incoming),
    }
}

#[derive(Deserialize)]
struct OllamaShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
}

fn ollama_capabilities(capabilities: &[String]) -> Option<ModelCapabilities> {
    let has = |needle: &str| {
        capabilities
            .iter()
            .any(|item| item.trim().eq_ignore_ascii_case(needle))
    };
    let image_input = has("vision").then_some(true);
    let tool_use = (has("tools") || has("tool_use") || has("tool-use")).then_some(true);
    if image_input.is_none() && tool_use.is_none() {
        return None;
    }
    Some(ModelCapabilities {
        image_input,
        image_output: None,
        video_output: None,
        tool_use,
    })
}

#[derive(Deserialize)]
struct LmStudioNativeModelsResponse {
    #[serde(default)]
    models: Vec<LmStudioNativeModel>,
}

#[derive(Clone, Deserialize)]
struct LmStudioNativeModel {
    key: String,
    #[serde(default)]
    selected_variant: Option<String>,
    #[serde(default)]
    loaded_instances: Vec<LmStudioLoadedInstance>,
    #[serde(default)]
    capabilities: Option<LmStudioCapabilities>,
}

#[derive(Clone, Deserialize)]
struct LmStudioLoadedInstance {
    id: String,
}

#[derive(Clone, Deserialize)]
struct LmStudioCapabilities {
    #[serde(default)]
    vision: Option<bool>,
    #[serde(default)]
    trained_for_tool_use: Option<bool>,
    #[serde(default)]
    reasoning: Option<LmStudioReasoningCapability>,
}

#[derive(Clone, Deserialize)]
struct LmStudioReasoningCapability {
    #[serde(default)]
    allowed_options: Vec<String>,
    #[serde(default)]
    default: Option<String>,
}

#[derive(Default)]
struct LmStudioNativeMetadata {
    reasoning: BTreeMap<String, ModelReasoningMetadata>,
    capabilities: BTreeMap<String, ModelCapabilities>,
}

fn lm_studio_native_metadata_map(parsed: LmStudioNativeModelsResponse) -> LmStudioNativeMetadata {
    let mut out = LmStudioNativeMetadata::default();
    for model in parsed.models {
        let ids = lm_studio_model_ids(&model);
        if let Some(meta) = model
            .capabilities
            .as_ref()
            .and_then(|cap| cap.reasoning.clone())
            .and_then(lm_studio_reasoning_meta)
        {
            for id in &ids {
                out.reasoning.insert(id.clone(), meta.clone());
            }
        }
        if let Some(capabilities) = model.capabilities.as_ref().and_then(lm_studio_capabilities) {
            for id in ids {
                out.capabilities.insert(id, capabilities.clone());
            }
        }
    }
    out
}

fn lm_studio_model_ids(model: &LmStudioNativeModel) -> Vec<String> {
    let mut out = vec![model.key.clone()];
    if let Some(selected) = &model.selected_variant {
        out.push(selected.clone());
    }
    out.extend(
        model
            .loaded_instances
            .iter()
            .map(|loaded| loaded.id.clone()),
    );
    out
}

fn lm_studio_capabilities(capability: &LmStudioCapabilities) -> Option<ModelCapabilities> {
    if capability.vision.is_none() && capability.trained_for_tool_use.is_none() {
        return None;
    }
    Some(ModelCapabilities {
        image_input: capability.vision,
        image_output: None,
        video_output: None,
        tool_use: capability.trained_for_tool_use,
    })
}

fn request_has_image_parts(req: &CompletionRequest) -> bool {
    req.messages.iter().any(|message| {
        matches!(
            &message.content,
            Some(Content::Parts(parts))
                if parts
                    .iter()
                    .any(|part| matches!(part, ContentPart::ImageUrl { .. }))
        )
    })
}

fn lm_studio_reasoning_meta(
    capability: LmStudioReasoningCapability,
) -> Option<ModelReasoningMetadata> {
    let supported_efforts = capability
        .allowed_options
        .iter()
        .filter_map(|value| lm_studio_reasoning_effort(value))
        .collect::<Vec<_>>();
    if supported_efforts.is_empty() {
        return None;
    }
    let default_effort = capability
        .default
        .as_deref()
        .and_then(lm_studio_reasoning_effort);
    let mandatory = !supported_efforts.contains(&ReasoningEffort::None);
    Some(ModelReasoningMetadata {
        supported_efforts,
        default_effort,
        default_enabled: Some(default_effort != Some(ReasoningEffort::None)),
        mandatory: Some(mandatory),
    })
}

fn lm_studio_reasoning_effort(value: &str) -> Option<ReasoningEffort> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => Some(ReasoningEffort::None),
        "on" => Some(ReasoningEffort::On),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
struct LmStudioNativeChatRequest {
    model: String,
    input: String,
    stream: bool,
    store: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning: Option<&'static str>,
}

fn build_lm_studio_native_chat_body(
    req: &CompletionRequest,
    stream: bool,
) -> Result<LmStudioNativeChatRequest> {
    if !req.tools.is_empty() || req.tool_choice.is_some() {
        return Err(Error::InvalidRequest(
            "LM Studio native reasoning does not support milim function tools yet; use a gpt-oss low/medium/high effort model or Auto.".to_string(),
        ));
    }
    if req.response_format.is_some() {
        return Err(Error::InvalidRequest(
            "LM Studio native reasoning does not support structured output yet; use Auto or remove response_format.".to_string(),
        ));
    }
    if !req.sampling.stop.is_empty()
        || req.sampling.seed.is_some()
        || req.sampling.frequency_penalty.is_some()
        || req.sampling.presence_penalty.is_some()
    {
        return Err(Error::InvalidRequest(
            "LM Studio native reasoning only maps temperature, top_p, and max tokens.".to_string(),
        ));
    }
    Ok(LmStudioNativeChatRequest {
        model: req.model.clone(),
        input: lm_studio_native_input(&req.messages)?,
        stream,
        store: false,
        system_prompt: lm_studio_system_prompt(&req.messages),
        temperature: req.sampling.temperature,
        top_p: req.sampling.top_p,
        max_output_tokens: req.sampling.max_tokens,
        reasoning: match req.reasoning_effort {
            Some(effort) => lm_studio_native_reasoning(effort)?,
            None => None,
        },
    })
}

fn lm_studio_native_reasoning(effort: ReasoningEffort) -> Result<Option<&'static str>> {
    match effort {
        ReasoningEffort::Auto => Ok(None),
        ReasoningEffort::None => Ok(Some("off")),
        ReasoningEffort::Low => Ok(Some("low")),
        ReasoningEffort::Medium => Ok(Some("medium")),
        ReasoningEffort::High => Ok(Some("high")),
        ReasoningEffort::On => Ok(Some("on")),
        ReasoningEffort::Minimal | ReasoningEffort::Xhigh | ReasoningEffort::Max => {
            Err(Error::InvalidRequest(format!(
                "LM Studio native reasoning supports off, on, low, medium, and high (got {}).",
                effort.as_str()
            )))
        }
    }
}

fn lm_studio_native_input(messages: &[milim_core::api::openai::ChatMessage]) -> Result<String> {
    let mut lines = Vec::new();
    for message in messages.iter().filter(|m| m.role != "system") {
        if message.tool_calls.is_some() || message.tool_call_id.is_some() {
            return Err(Error::InvalidRequest(
                "LM Studio native reasoning does not support tool-call history yet.".to_string(),
            ));
        }
        let text = lm_studio_text_content(message)?;
        if !text.trim().is_empty() {
            lines.push(format!("{}: {text}", message.role));
        }
    }
    Ok(lines.join("\n\n"))
}

fn lm_studio_system_prompt(messages: &[milim_core::api::openai::ChatMessage]) -> Option<String> {
    let systems = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.text_content())
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>();
    (!systems.is_empty()).then(|| systems.join("\n\n"))
}

fn lm_studio_text_content(message: &milim_core::api::openai::ChatMessage) -> Result<String> {
    match &message.content {
        Some(Content::Text(text)) => Ok(text.clone()),
        Some(Content::Parts(parts)) => {
            let mut out = String::new();
            for part in parts {
                match part {
                    ContentPart::Text { text } => out.push_str(text),
                    ContentPart::ImageUrl { .. }
                    | ContentPart::InputAudio { .. }
                    | ContentPart::Unknown => {
                        return Err(Error::InvalidRequest(
                            "LM Studio native reasoning currently supports text-only messages."
                                .to_string(),
                        ));
                    }
                }
            }
            Ok(out)
        }
        None => Ok(String::new()),
    }
}

enum NativeLineOutcome {
    Event(Value),
    Ignore,
}

fn parse_native_sse_line(line: &str) -> NativeLineOutcome {
    let Some(data) = line.strip_prefix("data:") else {
        return NativeLineOutcome::Ignore;
    };
    let data = data.trim();
    if data.is_empty() {
        return NativeLineOutcome::Ignore;
    }
    match serde_json::from_str::<Value>(data) {
        Ok(value) => NativeLineOutcome::Event(value),
        Err(_) => NativeLineOutcome::Ignore,
    }
}

fn native_chat_event_to_stream_event(value: &Value) -> Result<Option<StreamEvent>> {
    match value.get("type").and_then(Value::as_str) {
        Some("message.delta") => Ok(value
            .get("content")
            .and_then(Value::as_str)
            .map(DeltaEvent::text)
            .map(StreamEvent::Delta)),
        Some("reasoning.delta") => Ok(value.get("content").and_then(Value::as_str).map(|text| {
            StreamEvent::Delta(DeltaEvent {
                reasoning: Some(text.to_string()),
                ..Default::default()
            })
        })),
        Some("chat.end") => Ok(Some(StreamEvent::Done {
            finish_reason: "stop".to_string(),
            usage: native_chat_usage(value),
        })),
        Some("error") => Err(Error::Upstream(format!(
            "LM Studio native chat failed: {}",
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
        ))),
        _ => Ok(None),
    }
}

fn native_chat_usage(value: &Value) -> Usage {
    let stats = value.pointer("/result/stats").unwrap_or(&Value::Null);
    let prompt = stats
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let completion = stats
        .get("total_output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    Usage::new(prompt, completion)
}

fn upstream(e: impl std::fmt::Display) -> Error {
    Error::Upstream(e.to_string())
}

fn is_gpt_oss_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().contains("gpt-oss")
}

pub(crate) fn looks_reasoning_model(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.contains("/o1")
        || id.contains("/o3")
        || id.contains("/o4")
        || id.contains("gpt-5")
        || id.contains("gpt-oss")
        || id.contains("deepseek-r")
        || id.contains("deepseek-v3.1")
        || id.contains("qwen3")
        || id.contains("reason")
}

fn vllm_reasoning_efforts_from_openapi(spec: &Value) -> Vec<ReasoningEffort> {
    let Some(variants) = spec
        .pointer("/components/schemas/ChatCompletionRequest/properties/reasoning_effort/anyOf")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    variants
        .iter()
        .filter_map(|variant| variant.get("enum").and_then(Value::as_array))
        .flatten()
        .filter_map(|effort| serde_json::from_value(effort.clone()).ok())
        .collect()
}

#[derive(Serialize)]
struct LegacyCompletionRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    suffix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stop: Option<StringOrArray>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

fn build_legacy_completion_body(
    req: &CompletionRequest,
    stream: bool,
) -> Result<LegacyCompletionRequest> {
    let s = &req.sampling;
    Ok(LegacyCompletionRequest {
        model: req.model.clone(),
        prompt: req.prompt.clone().ok_or_else(|| {
            Error::InvalidRequest("legacy completion prompt is required".to_string())
        })?,
        stream,
        suffix: req.suffix.clone(),
        temperature: s.temperature,
        top_p: s.top_p,
        max_tokens: s.max_tokens,
        stop: (!s.stop.is_empty()).then(|| StringOrArray::Array(s.stop.clone())),
        frequency_penalty: s.frequency_penalty,
        presence_penalty: s.presence_penalty,
        seed: s.seed,
    })
}

enum CompletionLineOutcome {
    Done,
    Event(Value),
    Ignore,
}

fn parse_completion_sse_line(line: &str) -> CompletionLineOutcome {
    let Some(data) = line.strip_prefix("data:") else {
        return CompletionLineOutcome::Ignore;
    };
    let data = data.trim();
    if data.is_empty() {
        return CompletionLineOutcome::Ignore;
    }
    if data == "[DONE]" {
        return CompletionLineOutcome::Done;
    }
    match serde_json::from_str::<Value>(data) {
        Ok(value) => CompletionLineOutcome::Event(value),
        Err(_) => CompletionLineOutcome::Ignore,
    }
}

fn completion_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage").filter(|usage| usage.is_object())?;
    let (cache_read_tokens, cache_write_tokens) = openai_cache_tokens(usage);
    Some(Usage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        completion_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        cost_usd: usage
            .get("cost_usd")
            .or_else(|| usage.get("cost"))
            .and_then(Value::as_f64),
        cache_read_tokens,
        cache_write_tokens,
    })
}

/// Cached prompt tokens as OpenAI (`prompt_tokens_details.cached_tokens`,
/// or `input_tokens_details` on the Responses API), OpenRouter
/// (`cache_write_tokens`), and DeepSeek (`prompt_cache_hit_tokens`) report
/// them. All are already included in the prompt token count.
fn openai_cache_tokens(usage: &Value) -> (Option<u32>, Option<u32>) {
    let count = |pointer: &str| {
        usage
            .pointer(pointer)
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .map(|n| n as u32)
    };
    let read = count("/prompt_tokens_details/cached_tokens")
        .or_else(|| count("/input_tokens_details/cached_tokens"))
        .or_else(|| count("/prompt_cache_hit_tokens"));
    let write = count("/prompt_tokens_details/cache_write_tokens");
    (read, write)
}

/// A stable routing key for OpenAI's prompt cache. A caller-supplied key
/// (the canonical thread id) is hashed so every turn of one thread shares a
/// cache without sending the id itself. Otherwise the key hashes the leading
/// system messages and the first conversation message, which stay fixed for
/// every step of one conversation.
fn prompt_cache_key(req: &CompletionRequest) -> Option<String> {
    if let Some(key) = req
        .sampling
        .prompt_cache_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        return Some(format!("milim-{:016x}", fnv1a(key.bytes())));
    }
    let end = req
        .messages
        .iter()
        .position(|m| m.role != "system")
        .map_or(req.messages.len(), |index| index + 1);
    if end == 0 {
        return None;
    }
    let hash = fnv1a(req.messages[..end].iter().flat_map(|message| {
        message
            .role
            .bytes()
            .chain([0])
            .chain(message.text_content().into_bytes())
            .chain([0])
            .collect::<Vec<_>>()
    }));
    Some(format!("milim-{hash:016x}"))
}

/// FNV-1a keeps prompt cache keys stable across processes and Rust versions.
fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// An error object a provider sent inside an open stream: OpenAI's
/// `{"error":{...}}`, OpenRouter's chunk-level `error` with a numeric code, or
/// vLLM's `{"object":"error",...}`.
struct StreamErrorPayload {
    status: Option<u16>,
    kind: Option<String>,
    message: String,
}

impl StreamErrorPayload {
    fn from_value(value: &Value) -> Option<Self> {
        let error = match value.get("error") {
            Some(error @ Value::Object(_)) => error,
            Some(Value::String(message)) => {
                return Some(Self {
                    status: None,
                    kind: None,
                    message: message.clone(),
                })
            }
            _ if value.get("object").and_then(Value::as_str) == Some("error") => value,
            _ => return None,
        };
        let code = error.get("code");
        let status = code
            .and_then(Value::as_u64)
            .or_else(|| code.and_then(Value::as_str).and_then(|c| c.parse().ok()))
            .and_then(|code| u16::try_from(code).ok());
        let kind = error
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| {
                code.and_then(Value::as_str)
                    .filter(|c| c.parse::<u16>().is_err())
            })
            .map(str::to_string);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Some(Self {
            status,
            kind,
            message,
        })
    }

    fn into_error(self, label: &str, operation: &str) -> Error {
        upstream_stream_error(
            label,
            operation,
            self.status,
            self.kind.as_deref(),
            &self.message,
        )
    }
}

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    input: Vec<Value>,
    stream: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning: Option<ResponsesReasoning>,
}

#[derive(Serialize)]
struct ResponsesReasoning {
    effort: &'static str,
}

fn build_lm_studio_responses_body(
    req: &CompletionRequest,
    stream: bool,
) -> Result<ResponsesRequest> {
    Ok(ResponsesRequest {
        model: req.model.clone(),
        input: responses_input(&req.messages)?,
        stream,
        temperature: req.sampling.temperature,
        top_p: req.sampling.top_p,
        max_output_tokens: req.sampling.max_tokens,
        tools: responses_tools(&req.tools),
        tool_choice: req.tool_choice.clone(),
        reasoning: lm_studio_responses_reasoning(req.reasoning_effort),
    })
}

fn lm_studio_responses_reasoning(effort: Option<ReasoningEffort>) -> Option<ResponsesReasoning> {
    let effort = effort?;
    match effort {
        ReasoningEffort::None
        | ReasoningEffort::Low
        | ReasoningEffort::Medium
        | ReasoningEffort::High => Some(ResponsesReasoning {
            effort: effort.as_str(),
        }),
        ReasoningEffort::Auto
        | ReasoningEffort::On
        | ReasoningEffort::Minimal
        | ReasoningEffort::Xhigh
        | ReasoningEffort::Max => None,
    }
}

fn responses_input(messages: &[milim_core::api::openai::ChatMessage]) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for message in messages {
        if message.role == "tool" {
            let call_id = message.tool_call_id.clone().ok_or_else(|| {
                Error::InvalidRequest(
                    "LM Studio Responses requires tool messages to include tool_call_id"
                        .to_string(),
                )
            })?;
            out.push(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": message.text_content(),
            }));
            continue;
        }

        if let Some(content) = responses_message_content(message)? {
            out.push(json!({
                "type": "message",
                "role": message.role.clone(),
                "content": content,
            }));
        }

        if let Some(tool_calls) = &message.tool_calls {
            for tool_call in tool_calls {
                out.push(json!({
                    "type": "function_call",
                    "call_id": tool_call.id.clone().unwrap_or_else(|| "call_0".to_string()),
                    "name": tool_call.function.name.clone(),
                    "arguments": tool_call.function.arguments.clone(),
                }));
            }
        }
    }
    Ok(out)
}

pub(crate) fn responses_message_content(
    message: &milim_core::api::openai::ChatMessage,
) -> Result<Option<Value>> {
    let Some(content) = &message.content else {
        return Ok(None);
    };
    match content {
        Content::Text(text) => Ok(Some(Value::String(text.clone()))),
        Content::Parts(parts) => {
            let mut out = Vec::new();
            for part in parts {
                match part {
                    ContentPart::Text { text } => out.push(json!({
                        "type": "input_text",
                        "text": text,
                    })),
                    ContentPart::ImageUrl { image_url } => {
                        let mut item = Map::new();
                        item.insert("type".to_string(), Value::String("input_image".to_string()));
                        item.insert(
                            "image_url".to_string(),
                            Value::String(image_url.url.clone()),
                        );
                        if let Some(detail) = &image_url.detail {
                            item.insert("detail".to_string(), Value::String(detail.clone()));
                        }
                        out.push(Value::Object(item));
                    }
                    ContentPart::InputAudio { .. } | ContentPart::Unknown => {
                        return Err(Error::InvalidRequest(
                            "The Responses API path only supports text and image_url message parts"
                                .to_string(),
                        ));
                    }
                }
            }
            Ok(Some(Value::Array(out)))
        }
    }
}

pub(crate) fn responses_tools(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .filter(|tool| tool.kind == "function")
        .map(|tool| {
            let mut out = Map::new();
            out.insert("type".to_string(), Value::String("function".to_string()));
            out.insert(
                "name".to_string(),
                Value::String(tool.function.name.clone()),
            );
            if let Some(description) = &tool.function.description {
                out.insert(
                    "description".to_string(),
                    Value::String(description.clone()),
                );
            }
            out.insert(
                "parameters".to_string(),
                tool.function
                    .parameters
                    .clone()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            );
            out.insert("strict".to_string(), Value::Bool(false));
            Value::Object(out)
        })
        .collect()
}

enum ResponsesLineOutcome {
    Done,
    Event(Value),
    Ignore,
}

fn parse_responses_sse_line(line: &str) -> ResponsesLineOutcome {
    let Some(data) = line.strip_prefix("data:") else {
        return ResponsesLineOutcome::Ignore;
    };
    let data = data.trim();
    if data.is_empty() {
        return ResponsesLineOutcome::Ignore;
    }
    if data == "[DONE]" {
        return ResponsesLineOutcome::Done;
    }
    match serde_json::from_str::<Value>(data) {
        Ok(value) => ResponsesLineOutcome::Event(value),
        Err(_) => ResponsesLineOutcome::Ignore,
    }
}

fn responses_event_to_stream_event(value: &Value) -> Result<Option<StreamEvent>> {
    match value.get("type").and_then(Value::as_str) {
        Some("response.output_text.delta") => Ok(value
            .get("delta")
            .and_then(Value::as_str)
            .map(|text| StreamEvent::Delta(DeltaEvent::text(text)))),
        Some("response.reasoning_text.delta") | Some("response.reasoning_summary_text.delta") => {
            Ok(value.get("delta").and_then(Value::as_str).map(|text| {
                StreamEvent::Delta(DeltaEvent {
                    reasoning: Some(text.to_string()),
                    ..Default::default()
                })
            }))
        }
        Some("response.function_call_arguments.done") => {
            Ok(response_tool_call_delta(value).map(|delta| {
                StreamEvent::Delta(DeltaEvent {
                    tool_calls: vec![delta],
                    ..Default::default()
                })
            }))
        }
        Some("response.output_item.done") => Ok(value
            .get("item")
            .and_then(response_tool_call_delta)
            .map(|delta| {
                StreamEvent::Delta(DeltaEvent {
                    tool_calls: vec![delta],
                    ..Default::default()
                })
            })),
        Some("response.completed") => Ok(Some(StreamEvent::Done {
            finish_reason: "stop".to_string(),
            usage: response_usage(value),
        })),
        Some("error") => Err(upstream_stream_error(
            "LM Studio",
            "responses",
            None,
            value.get("code").and_then(Value::as_str),
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )),
        Some("response.failed") => Err(Error::Upstream(format!(
            "LM Studio response failed: {}",
            response_error_message(value)
        ))),
        Some("response.incomplete") => Err(Error::Upstream(format!(
            "LM Studio response incomplete: {}",
            value
                .pointer("/response/incomplete_details/reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ))),
        _ => Ok(None),
    }
}

fn response_tool_call_delta(value: &Value) -> Option<DeltaToolCall> {
    let item_type = value.get("type").and_then(Value::as_str);
    if item_type != Some("function_call")
        && item_type != Some("response.function_call_arguments.done")
    {
        return None;
    }
    let name = value.get("name").and_then(Value::as_str)?;
    let arguments = value
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(DeltaToolCall {
        index: value
            .get("output_index")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        id: value
            .get("call_id")
            .or_else(|| value.get("item_id"))
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        kind: Some("function".to_string()),
        function: DeltaFunction {
            name: Some(name.to_string()),
            arguments: Some(arguments.to_string()),
        },
    })
}

pub(crate) fn response_usage(value: &Value) -> Usage {
    let usage = value.pointer("/response/usage").unwrap_or(&Value::Null);
    let prompt = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let completion = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| u64::from(prompt + completion)) as u32;
    let (cache_read_tokens, cache_write_tokens) = openai_cache_tokens(usage);
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cost_usd: usage
            .get("cost_usd")
            .or_else(|| usage.get("cost"))
            .and_then(Value::as_f64),
        cache_read_tokens,
        cache_write_tokens,
    }
}

fn response_error_message(value: &Value) -> String {
    value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

/// Outcome of interpreting one SSE line.
enum LineOutcome {
    /// The terminal `data: [DONE]` sentinel.
    Done,
    /// A parsed `chat.completion.chunk` object, read leniently by
    /// [`ChatStream::apply`].
    Event(Value),
    /// An error object the provider sent instead of (or inside) a chunk.
    Error(StreamErrorPayload),
    /// Comment, blank line, keepalive, or unparseable fragment.
    Ignore,
}

/// Interpret one trimmed SSE line.
fn parse_sse_line(line: &str) -> LineOutcome {
    let Some(data) = line.strip_prefix("data:") else {
        return LineOutcome::Ignore;
    };
    let data = data.trim();
    if data.is_empty() {
        return LineOutcome::Ignore;
    }
    if data == "[DONE]" {
        return LineOutcome::Done;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return LineOutcome::Ignore;
    };
    if let Some(error) = StreamErrorPayload::from_value(&value) {
        return LineOutcome::Error(error);
    }
    if value.is_object() {
        LineOutcome::Event(value)
    } else {
        LineOutcome::Ignore
    }
}

/// Per-stream state for reading `chat.completion.chunk`s. Fields are read
/// one at a time, so a chunk with an odd or missing field (no `id`, no tool
/// call `index`, object-valued arguments) still delivers the rest.
struct ChatStream {
    finish_reason: Option<String>,
    usage: Option<Usage>,
    /// Call ids by slot, for servers that omit tool-call `index`.
    unindexed_ids: Vec<String>,
    last_slot: u32,
    /// The model, when this is an OpenRouter stream whose
    /// `reasoning_details` are kept for the next request.
    openrouter_model: Option<String>,
    reasoning_details: Vec<Value>,
    structured_calls: bool,
    text_calls: TextToolCalls,
}

impl ChatStream {
    fn new(openrouter_model: Option<String>) -> Self {
        Self {
            finish_reason: None,
            usage: None,
            unindexed_ids: Vec::new(),
            last_slot: 0,
            openrouter_model,
            reasoning_details: Vec::new(),
            structured_calls: false,
            text_calls: TextToolCalls::default(),
        }
    }

    /// Recover tool calls written out as text for a request offering `tools`.
    fn with_text_tool_calls(mut self, tools: &[Tool]) -> Self {
        self.text_calls.names = tools
            .iter()
            .map(|tool| tool.function.name.clone())
            .collect();
        self
    }

    /// Fold one chunk in, returning the delta it carries.
    fn apply(&mut self, chunk: &Value) -> DeltaEvent {
        if let Some(usage) = completion_usage(chunk) {
            self.usage = Some(usage);
        }
        let Some(choice) = chunk.pointer("/choices/0") else {
            return DeltaEvent::default();
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        let text = |key: &str| delta.get(key).and_then(Value::as_str).map(str::to_string);
        let content = text("content");
        let reasoning = text("reasoning_content")
            .or_else(|| text("reasoning"))
            .filter(|reasoning| {
                !reasoning.trim().is_empty() && Some(reasoning) != content.as_ref()
            });
        let mut out = DeltaEvent {
            content: content.and_then(|content| self.text_calls.filter(content)),
            reasoning,
            ..Default::default()
        };
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            out.tool_calls = calls
                .iter()
                .enumerate()
                .filter_map(|(position, call)| self.tool_call(position as u32, call))
                .collect();
            self.structured_calls |= !out.tool_calls.is_empty();
        }
        if self.openrouter_model.is_some() {
            if let Some(details) = delta.get("reasoning_details").and_then(Value::as_array) {
                for detail in details {
                    merge_reasoning_detail(&mut self.reasoning_details, detail);
                }
            }
        }
        out
    }

    fn tool_call(&mut self, position: u32, call: &Value) -> Option<DeltaToolCall> {
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let index = match call.get("index").and_then(Value::as_u64) {
            Some(index) => index as u32,
            // Without `index`, a new id opens a new call and an id-less
            // fragment continues the latest one.
            None => match &id {
                Some(id) => {
                    let slot = match self.unindexed_ids.iter().position(|seen| seen == id) {
                        Some(slot) => slot,
                        None => {
                            self.unindexed_ids.push(id.clone());
                            self.unindexed_ids.len() - 1
                        }
                    } as u32;
                    self.last_slot = slot;
                    slot
                }
                None => self.last_slot + position,
            },
        };
        let function = call.get("function").unwrap_or(&Value::Null);
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let arguments = match function.get("arguments") {
            None | Some(Value::Null) => None,
            Some(Value::String(arguments)) => Some(arguments.clone()),
            // Some servers send the arguments as a JSON object.
            Some(arguments) => Some(arguments.to_string()),
        };
        if id.is_none() && name.is_none() && arguments.is_none() {
            return None;
        }
        Some(DeltaToolCall {
            index,
            id,
            kind: call.get("type").and_then(Value::as_str).map(str::to_string),
            function: DeltaFunction { name, arguments },
        })
    }

    /// The terminal events: any held-back text (or the tool calls it
    /// spelled out), OpenRouter reasoning state, then `Done`.
    fn finish(mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let (content, tool_calls) = self.text_calls.finish(self.structured_calls);
        if !tool_calls.is_empty() {
            self.finish_reason = Some("tool_calls".to_string());
        }
        if content.is_some() || !tool_calls.is_empty() {
            events.push(StreamEvent::Delta(DeltaEvent {
                content,
                tool_calls,
                ..Default::default()
            }));
        }
        if let Some(model) = self
            .openrouter_model
            .filter(|_| !self.reasoning_details.is_empty())
        {
            events.push(StreamEvent::Delta(DeltaEvent {
                provider_state: Some(json!({
                    OPENROUTER_STATE_KEY: {
                        "model": model,
                        "reasoning_details": self.reasoning_details,
                    }
                })),
                ..Default::default()
            }));
        }
        events.push(StreamEvent::Done {
            finish_reason: normalize_finish_reason(self.finish_reason.as_deref()).to_string(),
            usage: self.usage.unwrap_or_default(),
        });
        events
    }
}

const TOOL_CALL_OPEN: &str = "<tool_call>";
const TOOL_CALL_CLOSE: &str = "</tool_call>";

/// Recovers tool calls that a local model wrote out as text
/// (`<tool_call>{"name":..,"arguments":..}</tool_call>`) and its server did
/// not parse. Visible text that may still be such a call is held back; at the
/// end it becomes tool calls only if it is nothing but those blocks, each
/// naming an offered tool, and no structured call arrived. Otherwise it is
/// released unchanged.
#[derive(Default)]
struct TextToolCalls {
    /// The offered tools; empty turns recovery off.
    names: Vec<String>,
    held: Option<String>,
    /// Visible text already passed through, so this turn is not a call.
    released: bool,
}

impl TextToolCalls {
    /// Pass visible text through, or hold it while it may still be a call.
    fn filter(&mut self, text: String) -> Option<String> {
        if self.names.is_empty() || self.released {
            return Some(text);
        }
        let held = self.held.get_or_insert_with(String::new);
        held.push_str(&text);
        let start = held.trim_start();
        if TOOL_CALL_OPEN.starts_with(start) || start.starts_with(TOOL_CALL_OPEN) {
            return None;
        }
        self.released = true;
        self.held.take()
    }

    /// The held text at the end of the stream, as text or as tool calls.
    fn finish(&mut self, structured_calls: bool) -> (Option<String>, Vec<DeltaToolCall>) {
        let Some(held) = self.held.take() else {
            return (None, Vec::new());
        };
        match (!structured_calls).then(|| self.parse(&held)).flatten() {
            Some(calls) => (None, calls),
            None => ((!held.is_empty()).then_some(held), Vec::new()),
        }
    }

    fn parse(&self, text: &str) -> Option<Vec<DeltaToolCall>> {
        let mut blocks = text.trim().split(TOOL_CALL_OPEN);
        if !blocks.next()?.trim().is_empty() {
            return None;
        }
        let mut calls = Vec::new();
        for (index, block) in blocks.enumerate() {
            let block = block.trim();
            // Chat templates often stop on the closing tag itself.
            let block = block.strip_suffix(TOOL_CALL_CLOSE).unwrap_or(block);
            let call: Value = serde_json::from_str(block.trim()).ok()?;
            let name = call.get("name")?.as_str()?;
            if !self.names.iter().any(|offered| offered == name) {
                return None;
            }
            let arguments = match call.get("arguments")? {
                Value::String(arguments) => arguments.clone(),
                arguments @ Value::Object(_) => arguments.to_string(),
                _ => return None,
            };
            calls.push(DeltaToolCall {
                index: index as u32,
                id: None,
                kind: Some("function".to_string()),
                function: DeltaFunction {
                    name: Some(name.to_string()),
                    arguments: Some(arguments),
                },
            });
        }
        (!calls.is_empty()).then_some(calls)
    }
}

/// Fold one streamed OpenRouter `reasoning_details` entry in. OpenRouter
/// streams a detail in pieces with the same `type` and `index`; their text is
/// concatenated and later non-null fields (such as the signature) kept.
fn merge_reasoning_detail(details: &mut Vec<Value>, incoming: &Value) {
    let Some(incoming) = incoming.as_object() else {
        return;
    };
    if let Some(Value::Object(last)) = details.last_mut() {
        if last.get("type") == incoming.get("type") && last.get("index") == incoming.get("index") {
            for (key, value) in incoming {
                match (key.as_str(), last.get_mut(key), value) {
                    (_, _, Value::Null) => {}
                    (
                        "text" | "summary" | "data",
                        Some(Value::String(text)),
                        Value::String(more),
                    ) => text.push_str(more),
                    _ => {
                        last.insert(key.clone(), value.clone());
                    }
                }
            }
            return;
        }
    }
    details.push(Value::Object(incoming.clone()));
}

/// OpenRouter extras on the wire messages: each assistant turn's saved
/// `reasoning_details` (in place of its plain reasoning text), and prompt
/// cache breakpoints on the system prompt and newest user message for
/// Anthropic models, which cache only at explicit breakpoints.
fn apply_openrouter_messages(wire: &mut [Value], messages: &[ChatMessage], model: &str) {
    for (wire, message) in wire.iter_mut().zip(messages) {
        let Some(details) = message
            .provider_state
            .as_ref()
            .and_then(|state| state.get(OPENROUTER_STATE_KEY))
            .filter(|state| state.get("model").and_then(Value::as_str) == Some(model))
            .and_then(|state| state.get("reasoning_details"))
        else {
            continue;
        };
        if let Some(wire) = wire.as_object_mut() {
            wire.remove("reasoning_content");
            wire.insert("reasoning_details".to_string(), details.clone());
        }
    }
    let id = model.trim().trim_start_matches('~').to_ascii_lowercase();
    if !id.starts_with("anthropic/") {
        return;
    }
    let leading_system = messages.iter().take_while(|m| m.role == "system").count();
    let newest_user = messages.iter().rposition(|m| m.role == "user");
    for index in [leading_system.checked_sub(1), newest_user]
        .into_iter()
        .flatten()
    {
        if let Some(message) = wire.get_mut(index) {
            mark_cache_breakpoint(message);
        }
    }
}

/// Put an ephemeral `cache_control` on a message's last text block.
fn mark_cache_breakpoint(message: &mut Value) {
    let cache_control = json!({ "type": "ephemeral" });
    match message.get_mut("content") {
        Some(Value::String(text)) if !text.is_empty() => {
            let text = std::mem::take(text);
            message["content"] =
                json!([{ "type": "text", "text": text, "cache_control": cache_control }]);
        }
        Some(Value::Array(parts)) => {
            if let Some(part) = parts
                .iter_mut()
                .rev()
                .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            {
                part["cache_control"] = cache_control;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn classifies_sse_lines() {
        assert!(matches!(parse_sse_line(": ping"), LineOutcome::Ignore));
        assert!(matches!(parse_sse_line(""), LineOutcome::Ignore));
        assert!(matches!(parse_sse_line("data: [DONE]"), LineOutcome::Done));
        assert!(matches!(parse_sse_line("event: foo"), LineOutcome::Ignore));
        let line = r#"data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hi"}}]}"#;
        assert!(matches!(parse_sse_line(line), LineOutcome::Event(_)));
    }

    #[test]
    fn privacy_gate_boundary_distinguishes_loopback_endpoints() {
        assert!(
            !RemoteBackend::new("local", "http://localhost:11434/v1", None).requires_privacy_gate()
        );
        assert!(
            !RemoteBackend::new("local", "http://127.0.0.1:1234/v1", None).requires_privacy_gate()
        );
        assert!(
            RemoteBackend::new("remote", "https://api.openai.com/v1", None).requires_privacy_gate()
        );
        assert!(RemoteBackend::new("invalid", "not a valid url", None).requires_privacy_gate());
    }

    #[test]
    fn openai_compatible_body_preserves_image_url_parts() {
        let backend = RemoteBackend::new("OpenAI", "https://api.openai.com/v1", None);
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC";
        let mut req = empty_req();
        req.messages = vec![milim_core::api::openai::ChatMessage {
            role: "user".to_string(),
            content: Some(Content::Parts(vec![
                ContentPart::Text {
                    text: "What colors are present?".to_string(),
                },
                ContentPart::ImageUrl {
                    image_url: milim_core::api::openai::ImageUrl {
                        url: format!("data:image/png;base64,{png}"),
                        detail: Some("high".to_string()),
                    },
                },
            ])),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            provider_state: None,
        }];
        let value = serde_json::to_value(backend.build_body(&req, true)).unwrap();
        assert_eq!(value["messages"][0]["content"][1]["type"], "image_url");
        assert_eq!(
            value["messages"][0]["content"][1]["image_url"]["url"],
            format!("data:image/png;base64,{png}")
        );
    }

    #[test]
    fn extracts_content_and_finish_from_chunk() {
        let line = r#"data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"cost":0.1745104}}"#;
        let LineOutcome::Event(chunk) = parse_sse_line(line) else {
            panic!("expected event");
        };
        let mut chat = ChatStream::new(None);
        let delta = chat.apply(&chunk);
        assert_eq!(delta.content.as_deref(), Some("hi"));
        assert_eq!(chat.finish_reason.as_deref(), Some("stop"));
        let usage = chat.usage.unwrap();
        assert_eq!(usage.total_tokens, 4);
        assert_eq!(usage.cost_usd, Some(0.1745104));
    }

    #[test]
    fn extracts_reasoning_from_openrouter_chunk() {
        let line = r#"data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"reasoning":"checking options"}}]}"#;
        let LineOutcome::Event(chunk) = parse_sse_line(line) else {
            panic!("expected event");
        };
        let delta = ChatStream::new(None).apply(&chunk);
        assert_eq!(delta.reasoning.as_deref(), Some("checking options"));
    }

    #[test]
    fn drops_duplicate_openrouter_reasoning_channel() {
        let line = r#"data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hello","reasoning":"hello"}}]}"#;
        let LineOutcome::Event(chunk) = parse_sse_line(line) else {
            panic!("expected event");
        };
        let delta = ChatStream::new(None).apply(&chunk);
        assert_eq!(delta.content.as_deref(), Some("hello"));
        assert_eq!(delta.reasoning, None);
    }

    #[test]
    fn builds_openai_body_with_stream_options() {
        let backend = RemoteBackend::new("openai", "https://api.openai.com/v1/", None);
        let req = CompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: crate::service::SamplingParams {
                temperature: Some(0.5),
                stop: vec!["X".into()],
                ..Default::default()
            },
            reasoning_effort: None,
        };
        let body = backend.build_body(&req, true);
        assert_eq!(body.stream, Some(true));
        assert!(body.stream_options.is_some());
        assert!(matches!(body.stop, Some(StringOrArray::Array(_))));
        assert_eq!(
            backend.endpoint("chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn attributes_only_openrouter_requests_to_milim() {
        let openrouter = RemoteBackend::new(
            "OpenRouter",
            "https://openrouter.ai/api/v1",
            Some("secret".into()),
        );
        let request = openrouter
            .auth(openrouter.client.get(openrouter.endpoint("models")))
            .build()
            .unwrap();
        assert_eq!(
            request.headers()["http-referer"].to_str().unwrap(),
            OPENROUTER_HTTP_REFERER
        );
        assert_eq!(
            request.headers()["x-openrouter-title"].to_str().unwrap(),
            OPENROUTER_TITLE
        );

        let openai = RemoteBackend::new("OpenAI", "https://api.openai.com/v1", None);
        let request = openai
            .auth(openai.client.get(openai.endpoint("models")))
            .build()
            .unwrap();
        assert!(!request.headers().contains_key("http-referer"));
        assert!(!request.headers().contains_key("x-openrouter-title"));
    }

    #[test]
    fn builds_ollama_native_generate_endpoint() {
        let backend = RemoteBackend::new("Ollama", "http://localhost:11434/v1", None);
        assert_eq!(
            backend.ollama_generate_endpoint(),
            "http://localhost:11434/api/generate"
        );
    }

    #[test]
    fn builds_lm_studio_native_endpoints() {
        let backend = RemoteBackend::new("LM Studio", "http://localhost:1234/v1", None);
        assert_eq!(
            backend.lm_studio_api_endpoint("models"),
            "http://localhost:1234/api/v1/models"
        );
        assert_eq!(
            backend.lm_studio_api_endpoint("/chat"),
            "http://localhost:1234/api/v1/chat"
        );
    }

    #[test]
    fn parses_lm_studio_native_metadata() {
        let parsed: LmStudioNativeModelsResponse = serde_json::from_value(json!({
            "models": [
                {
                    "key": "google/gemma-4-26b-a4b",
                    "selected_variant": "google/gemma-4-26b-a4b@q4_k_m",
                    "loaded_instances": [{"id":"google/gemma-4-26b-a4b-loaded"}],
                    "capabilities": {
                        "vision": true,
                        "trained_for_tool_use": true,
                        "reasoning": {
                            "allowed_options": ["off", "on"],
                            "default": "on"
                        }
                    }
                },
                {
                    "key": "deepseek-r1",
                    "capabilities": {
                        "reasoning": {
                            "allowed_options": ["on"],
                            "default": "on"
                        }
                    }
                },
                {
                    "key": "plain",
                    "capabilities": {"vision": false}
                }
            ]
        }))
        .unwrap();

        let metadata = lm_studio_native_metadata_map(parsed);
        let gemma = metadata.reasoning.get("google/gemma-4-26b-a4b").unwrap();
        assert_eq!(
            gemma.supported_efforts,
            vec![ReasoningEffort::None, ReasoningEffort::On]
        );
        assert_eq!(gemma.default_effort, Some(ReasoningEffort::On));
        assert_eq!(gemma.mandatory, Some(false));
        assert!(metadata
            .reasoning
            .contains_key("google/gemma-4-26b-a4b@q4_k_m"));
        assert!(metadata
            .reasoning
            .contains_key("google/gemma-4-26b-a4b-loaded"));
        let capabilities = metadata
            .capabilities
            .get("google/gemma-4-26b-a4b-loaded")
            .unwrap();
        assert_eq!(capabilities.image_input, Some(true));
        assert_eq!(capabilities.tool_use, Some(true));

        let deepseek = metadata.reasoning.get("deepseek-r1").unwrap();
        assert_eq!(deepseek.supported_efforts, vec![ReasoningEffort::On]);
        assert_eq!(deepseek.mandatory, Some(true));
        assert!(!metadata.reasoning.contains_key("plain"));
        assert_eq!(
            metadata.capabilities.get("plain").unwrap().image_input,
            Some(false)
        );
    }

    #[tokio::test]
    async fn list_models_enriches_lm_studio_native_metadata() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = if req.starts_with("GET /v1/models ") {
                    json!({
                        "object":"list",
                        "data":[{"id":"deepseek-r1","object":"model","created":0,"owned_by":"lmstudio"}]
                    })
                } else {
                    json!({
                        "models":[{
                            "key":"deepseek-r1",
                            "capabilities":{
                                "vision": true,
                                "trained_for_tool_use": true,
                                "reasoning":{
                                    "allowed_options":["on"],
                                    "default":"on"
                                }
                            }
                        }]
                    })
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let backend = RemoteBackend::new("LM Studio", format!("http://{addr}/v1"), None);
        let models = backend.list_models().await.unwrap();
        let reasoning = models[0].reasoning.as_ref().unwrap();
        assert_eq!(reasoning.supported_efforts, vec![ReasoningEffort::On]);
        assert_eq!(reasoning.default_effort, Some(ReasoningEffort::On));
        let capabilities = models[0].capabilities.as_ref().unwrap();
        assert_eq!(capabilities.image_input, Some(true));
        assert_eq!(capabilities.tool_use, Some(true));
    }

    #[tokio::test]
    async fn list_models_enriches_ollama_native_capabilities() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = if req.starts_with("GET /v1/models ") {
                    json!({
                        "object":"list",
                        "data":[
                            {"id":"llava","object":"model","created":0,"owned_by":"ollama"},
                            {"id":"llama3","object":"model","created":0,"owned_by":"ollama"}
                        ]
                    })
                } else if req.contains(r#""model":"llava""#) {
                    json!({"capabilities":["completion","vision","tools"]})
                } else {
                    json!({"capabilities":["completion"]})
                }
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let backend = RemoteBackend::new("Ollama", format!("http://{addr}/v1"), None);
        let models = backend.list_models().await.unwrap();
        let llava = models.iter().find(|model| model.id == "llava").unwrap();
        let capabilities = llava.capabilities.as_ref().unwrap();
        assert_eq!(capabilities.image_input, Some(true));
        assert_eq!(capabilities.tool_use, Some(true));
        let llama3 = models.iter().find(|model| model.id == "llama3").unwrap();
        assert!(llama3.capabilities.is_none());
    }

    #[test]
    fn builds_reasoning_effort_for_reasoning_openai_models() {
        let backend = RemoteBackend::new("openai", "https://api.openai.com/v1/", None);
        let mut req = empty_req();
        req.model = "gpt-5".into();
        req.reasoning_effort = Some(ReasoningEffort::High);
        let body = backend.build_body(&req, true);
        assert_eq!(body.reasoning_effort, Some(ReasoningEffort::High));
        assert!(body.extra.get("reasoning").is_none());
    }

    #[test]
    fn builds_reasoning_object_for_openrouter() {
        let backend = RemoteBackend::new("OpenRouter", "https://openrouter.ai/api/v1", None);
        let mut req = empty_req();
        req.model = "anthropic/claude-sonnet-4".into();
        req.reasoning_effort = Some(ReasoningEffort::Max);
        let body = backend.build_body(&req, true);
        assert!(body.reasoning_effort.is_none());
        assert_eq!(body.extra["reasoning"]["effort"], "max");
        assert!(body.extra["reasoning"].get("exclude").is_none());
    }

    #[test]
    fn excludes_openrouter_reasoning_when_effort_is_off() {
        let backend = RemoteBackend::new("OpenRouter", "https://openrouter.ai/api/v1", None);
        let mut req = empty_req();
        req.model = "z-ai/glm-5.3".into();
        req.reasoning_effort = Some(ReasoningEffort::None);
        let body = backend.build_body(&req, true);
        assert_eq!(body.extra["reasoning"]["effort"], "none");
        assert_eq!(body.extra["reasoning"]["exclude"], true);
    }

    #[test]
    fn sends_reasoning_effort_for_ollama_openai_compatible() {
        let backend = RemoteBackend::new("Ollama", "http://localhost:11434/v1", None);
        let mut req = empty_req();
        req.model = "deepseek-r1".into();
        req.reasoning_effort = Some(ReasoningEffort::High);
        let body = backend.build_body(&req, true);
        assert_eq!(body.reasoning_effort, Some(ReasoningEffort::High));
        assert!(body.extra.is_empty());
    }

    #[test]
    fn sends_reasoning_and_sampling_extensions_for_vllm() {
        let backend = RemoteBackend::new("vLLM (local)", "http://localhost:8000/v1", None);
        let mut req = empty_req();
        req.model = "qwen3.8-27b".into();
        req.reasoning_effort = Some(ReasoningEffort::High);
        req.sampling.top_k = Some(50);
        req.sampling.min_p = Some(0.1);
        req.sampling.repetition_penalty = Some(1.05);
        req.sampling.thinking_token_budget = Some(2_048);

        let body = backend.build_body(&req, true);

        assert_eq!(body.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(body.extra["top_k"], 50);
        assert!((body.extra["min_p"].as_f64().unwrap() - 0.1).abs() < 0.000_001);
        assert!((body.extra["repetition_penalty"].as_f64().unwrap() - 1.05).abs() < 0.000_001);
        assert_eq!(body.extra["thinking_token_budget"], 2_048);
    }

    #[test]
    fn reads_vllm_reasoning_efforts_from_openapi() {
        let spec = json!({
            "components": { "schemas": { "ChatCompletionRequest": {
                "properties": { "reasoning_effort": { "anyOf": [
                    { "enum": ["none", "minimal", "low", "medium", "high", "xhigh", "max"] },
                    { "type": "null" }
                ] } }
            } } }
        });

        assert_eq!(
            vllm_reasoning_efforts_from_openapi(&spec),
            vec![
                ReasoningEffort::None,
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
                ReasoningEffort::Max,
            ]
        );
    }

    #[test]
    fn respects_explicit_reasoning_effort_for_generic_openai_compatible() {
        let backend = RemoteBackend::new("custom", "http://localhost:9999/v1", None);
        let mut req = empty_req();
        req.model = "deepseek-r1".into();
        req.reasoning_effort = Some(ReasoningEffort::High);
        let body = backend.build_body(&req, true);
        assert_eq!(body.reasoning_effort, Some(ReasoningEffort::High));
        assert!(body.extra.is_empty());
    }

    #[test]
    fn lm_studio_chat_completions_omits_reasoning_effort() {
        let backend = RemoteBackend::new("LM Studio", "http://localhost:1234/v1", None);
        let mut req = empty_req();
        req.model = "openai/gpt-oss-20b".into();
        req.reasoning_effort = Some(ReasoningEffort::High);
        let body = backend.build_body(&req, true);
        assert!(body.reasoning_effort.is_none());
        assert!(body.extra.is_empty());
        assert!(backend.should_use_lm_studio_responses(&req));
    }

    #[test]
    fn lm_studio_tool_native_reasoning_uses_responses() {
        let backend = RemoteBackend::new("LM Studio", "http://localhost:1234/v1", None);
        let mut req = empty_req();
        req.model = "google/gemma-4-26b-a4b-qat".into();
        req.reasoning_effort = Some(ReasoningEffort::On);
        req.tools = vec![Tool {
            kind: "function".into(),
            function: milim_core::api::openai::ToolFunction {
                name: "lookup".into(),
                description: None,
                parameters: None,
            },
        }];

        assert!(backend.should_use_lm_studio_responses(&req));
        let body = build_lm_studio_responses_body(&req, true).unwrap();
        let value = serde_json::to_value(body).unwrap();
        assert!(value.get("reasoning").is_none());
        assert_eq!(value["tools"][0]["name"], "lookup");
    }

    #[test]
    fn lm_studio_image_reasoning_skips_text_only_native_chat() {
        let backend = RemoteBackend::new("LM Studio", "http://localhost:1234/v1", None);
        let mut req = empty_req();
        req.model = "google/gemma-4-26b-a4b".into();
        req.reasoning_effort = Some(ReasoningEffort::On);
        req.messages = vec![milim_core::api::openai::ChatMessage {
            role: "user".into(),
            content: Some(Content::Parts(vec![
                ContentPart::Text {
                    text: "look".into(),
                },
                ContentPart::ImageUrl {
                    image_url: milim_core::api::openai::ImageUrl {
                        url: "data:image/png;base64,AAAA".into(),
                        detail: None,
                    },
                },
            ])),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            provider_state: None,
        }];

        assert!(!backend.should_use_lm_studio_native_chat(&req));
        assert!(!backend.should_use_lm_studio_responses(&req));
        let body = backend.build_body(&req, true);
        assert!(body.reasoning_effort.is_none());
    }

    #[test]
    fn lm_studio_rejects_native_unsupported_reasoning_effort() {
        let mut req = empty_req();
        req.model = "deepseek-r1".into();
        req.reasoning_effort = Some(ReasoningEffort::Max);
        let err = build_lm_studio_native_chat_body(&req, true).unwrap_err();
        assert!(err.to_string().contains("off, on, low, medium, and high"));
    }

    #[test]
    fn builds_lm_studio_native_chat_body() {
        let mut req = empty_req();
        req.model = "deepseek-r1".into();
        req.messages = vec![
            milim_core::api::openai::ChatMessage::text("system", "brief"),
            milim_core::api::openai::ChatMessage::text("user", "hello"),
            milim_core::api::openai::ChatMessage::text("assistant", "hi"),
        ];
        req.sampling.temperature = Some(0.2);
        req.sampling.top_p = Some(0.9);
        req.sampling.max_tokens = Some(128);
        req.reasoning_effort = Some(ReasoningEffort::On);

        let body = build_lm_studio_native_chat_body(&req, true).unwrap();
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(value["model"], "deepseek-r1");
        assert_eq!(value["stream"], true);
        assert_eq!(value["store"], false);
        assert_eq!(value["system_prompt"], "brief");
        assert_eq!(value["input"], "user: hello\n\nassistant: hi");
        assert_eq!(value["max_output_tokens"], 128);
        assert_eq!(value["reasoning"], "on");
    }

    #[test]
    fn rejects_lm_studio_native_unsafe_request_shape() {
        let mut req = empty_req();
        req.model = "deepseek-r1".into();
        req.reasoning_effort = Some(ReasoningEffort::On);
        req.response_format = Some(json!({"type":"json_object"}));
        let err = build_lm_studio_native_chat_body(&req, true).unwrap_err();
        assert!(err.to_string().contains("structured output"));
    }

    #[test]
    fn builds_lm_studio_responses_body() {
        let mut req = empty_req();
        req.model = "openai/gpt-oss-20b".into();
        req.messages = vec![
            milim_core::api::openai::ChatMessage::text("system", "brief"),
            milim_core::api::openai::ChatMessage {
                role: "user".into(),
                content: Some(Content::Parts(vec![
                    ContentPart::Text {
                        text: "look".into(),
                    },
                    ContentPart::ImageUrl {
                        image_url: milim_core::api::openai::ImageUrl {
                            url: "data:image/png;base64,abc".into(),
                            detail: Some("low".into()),
                        },
                    },
                ])),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                provider_state: None,
            },
        ];
        req.tools = vec![Tool {
            kind: "function".into(),
            function: milim_core::api::openai::ToolFunction {
                name: "lookup".into(),
                description: Some("find a value".into()),
                parameters: Some(json!({"type":"object","properties":{"id":{"type":"string"}}})),
            },
        }];
        req.sampling.max_tokens = Some(64);
        req.reasoning_effort = Some(ReasoningEffort::Low);

        let body = build_lm_studio_responses_body(&req, true).unwrap();
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(value["model"], "openai/gpt-oss-20b");
        assert_eq!(value["stream"], true);
        assert_eq!(value["max_output_tokens"], 64);
        assert_eq!(value["reasoning"]["effort"], "low");
        assert_eq!(value["input"][0]["role"], "system");
        assert_eq!(value["input"][1]["content"][0]["type"], "input_text");
        assert_eq!(value["input"][1]["content"][1]["type"], "input_image");
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tools"][0]["strict"], false);
    }

    #[test]
    fn parses_lm_studio_responses_events() {
        let text = json!({"type":"response.output_text.delta","delta":"hi"});
        let reasoning = json!({"type":"response.reasoning_text.delta","delta":"thinking"});
        let tool = json!({
            "type":"response.function_call_arguments.done",
            "output_index":2,
            "call_id":"call_abc",
            "name":"lookup",
            "arguments":"{\"id\":\"1\"}"
        });
        let done = json!({
            "type":"response.completed",
            "response":{"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}
        });

        let Some(StreamEvent::Delta(delta)) = responses_event_to_stream_event(&text).unwrap()
        else {
            panic!("expected text delta");
        };
        assert_eq!(delta.content.as_deref(), Some("hi"));

        let Some(StreamEvent::Delta(delta)) = responses_event_to_stream_event(&reasoning).unwrap()
        else {
            panic!("expected reasoning delta");
        };
        assert_eq!(delta.reasoning.as_deref(), Some("thinking"));

        let Some(StreamEvent::Delta(delta)) = responses_event_to_stream_event(&tool).unwrap()
        else {
            panic!("expected tool delta");
        };
        assert_eq!(delta.tool_calls[0].id.as_deref(), Some("call_abc"));
        assert_eq!(delta.tool_calls[0].function.name.as_deref(), Some("lookup"));
        assert_eq!(
            delta.tool_calls[0].function.arguments.as_deref(),
            Some("{\"id\":\"1\"}")
        );

        let Some(StreamEvent::Done { usage, .. }) = responses_event_to_stream_event(&done).unwrap()
        else {
            panic!("expected done");
        };
        assert_eq!(usage.total_tokens, 7);
    }

    #[test]
    fn parses_lm_studio_native_chat_events() {
        let text = json!({"type":"message.delta","content":"hi"});
        let reasoning = json!({"type":"reasoning.delta","content":"thinking"});
        let done = json!({
            "type":"chat.end",
            "result":{"stats":{"input_tokens":3,"total_output_tokens":4,"reasoning_output_tokens":2}}
        });

        let Some(StreamEvent::Delta(delta)) = native_chat_event_to_stream_event(&text).unwrap()
        else {
            panic!("expected text delta");
        };
        assert_eq!(delta.content.as_deref(), Some("hi"));

        let Some(StreamEvent::Delta(delta)) =
            native_chat_event_to_stream_event(&reasoning).unwrap()
        else {
            panic!("expected reasoning delta");
        };
        assert_eq!(delta.reasoning.as_deref(), Some("thinking"));

        let Some(StreamEvent::Done { usage, .. }) =
            native_chat_event_to_stream_event(&done).unwrap()
        else {
            panic!("expected done");
        };
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 4);
        assert_eq!(usage.total_tokens, 7);
    }

    #[tokio::test]
    async fn stream_times_out_when_upstream_never_responds() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            if let Ok((_socket, _peer)) = listener.accept().await {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        let backend = RemoteBackend::new("silent-upstream", format!("http://{addr}/v1"), None);

        let start = std::time::Instant::now();
        let err = match tokio::time::timeout(Duration::from_secs(1), backend.stream(empty_req()))
            .await
            .expect("backend stream should return before the outer timeout")
        {
            Ok(_) => panic!("silent upstream should produce a timeout error"),
            Err(e) => e,
        };

        assert!(start.elapsed() < Duration::from_secs(1));
        let msg = err.to_string();
        assert!(msg.contains("timed out"), "expected a timeout, got: {msg}");
        assert!(
            milim_core::provider_error::retry_hint(&err)
                .unwrap()
                .retryable
        );
    }

    #[test]
    fn stream_error_objects_become_typed_upstream_errors() {
        use milim_core::provider_error::{classify_provider_error, retry_hint, ProviderErrorKind};

        let openai = r#"data: {"error":{"message":"The server had an error processing your request.","type":"server_error","param":null,"code":null}}"#;
        let LineOutcome::Error(error) = parse_sse_line(openai) else {
            panic!("expected an error line");
        };
        let error = error.into_error("OpenAI", "chat/completions");
        assert!(error.to_string().contains("-> 500 server_error"), "{error}");
        assert!(retry_hint(&error).unwrap().retryable);

        let openrouter = r#"data: {"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}],"error":{"code":429,"message":"Provider returned error"}}"#;
        let LineOutcome::Error(error) = parse_sse_line(openrouter) else {
            panic!("expected an error line");
        };
        let info = classify_provider_error(
            &error
                .into_error("OpenRouter", "chat/completions")
                .to_string(),
        );
        assert_eq!(info.kind, ProviderErrorKind::RateLimited);
        assert_eq!(info.status, Some(429));

        let vllm = r#"data: {"object":"error","message":"This model's maximum context length is 8192 tokens","type":"BadRequestError","code":400}"#;
        let LineOutcome::Error(error) = parse_sse_line(vllm) else {
            panic!("expected an error line");
        };
        let error = error.into_error("vLLM", "chat/completions");
        assert_eq!(
            classify_provider_error(&error.to_string()).kind,
            ProviderErrorKind::ContextLength
        );
        assert!(!retry_hint(&error).unwrap().retryable);
    }

    #[test]
    fn extracts_cached_prompt_tokens_from_chunk_usage() {
        let line = r#"data: {"id":"x","object":"chat.completion.chunk","created":1,"model":"m","choices":[],"usage":{"prompt_tokens":2000,"completion_tokens":10,"total_tokens":2010,"prompt_tokens_details":{"cached_tokens":1536}}}"#;
        let LineOutcome::Event(chunk) = parse_sse_line(line) else {
            panic!("expected a chunk");
        };
        let mut chat = ChatStream::new(None);
        chat.apply(&chunk);
        let usage = chat.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 2000);
        assert_eq!(usage.cache_read_tokens, Some(1536));
        assert_eq!(usage.cache_write_tokens, None);

        let responses = json!({
            "response": { "usage": {
                "input_tokens": 50, "output_tokens": 5,
                "input_tokens_details": { "cached_tokens": 32 }
            }}
        });
        assert_eq!(response_usage(&responses).cache_read_tokens, Some(32));
    }

    #[test]
    fn sends_prompt_cache_key_only_to_openai() {
        let mut req = empty_req();
        req.messages = vec![
            milim_core::api::openai::ChatMessage::text("system", "Base."),
            milim_core::api::openai::ChatMessage::text("user", "Fix it."),
        ];
        let openai = RemoteBackend::new("openai", "https://api.openai.com/v1", None);
        let key = openai.build_body(&req, true).extra["prompt_cache_key"].clone();
        assert!(key.as_str().unwrap().starts_with("milim-"));

        // Later steps of the same conversation keep the key.
        req.messages
            .push(milim_core::api::openai::ChatMessage::text(
                "assistant",
                "Done.",
            ));
        req.messages
            .push(milim_core::api::openai::ChatMessage::text(
                "user", "Thanks.",
            ));
        assert_eq!(openai.build_body(&req, true).extra["prompt_cache_key"], key);

        // A thread key wins over the message hash and survives prompt changes.
        req.sampling.prompt_cache_key = Some("thread-1".into());
        let thread_key = openai.build_body(&req, true).extra["prompt_cache_key"].clone();
        assert_ne!(thread_key, key);
        assert!(!thread_key.as_str().unwrap().contains("thread-1"));
        req.messages[0] = milim_core::api::openai::ChatMessage::text("system", "Changed base.");
        assert_eq!(
            openai.build_body(&req, true).extra["prompt_cache_key"],
            thread_key
        );

        for other in [
            RemoteBackend::new("OpenRouter", "https://openrouter.ai/api/v1", None),
            RemoteBackend::new("vllm", "http://127.0.0.1:8000/v1", None),
            RemoteBackend::new("proxy", "https://api.openai.com.example.net/v1", None),
        ] {
            assert!(!other
                .build_body(&req, true)
                .extra
                .contains_key("prompt_cache_key"));
        }
    }

    #[test]
    fn leaves_max_tokens_unset_by_default() {
        let backend = RemoteBackend::new("openai", "https://api.openai.com/v1", None);
        let body = serde_json::to_value(backend.build_body(&empty_req(), true)).unwrap();
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("max_completion_tokens").is_none());
    }

    /// Serve one scripted HTTP response per connection, recording each
    /// request's JSON body.
    async fn serve(
        responses: Vec<(u16, &'static str, String)>,
    ) -> (String, Arc<std::sync::Mutex<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let bodies = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = bodies.clone();
        tokio::spawn(async move {
            for (status, content_type, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0u8; 4096];
                let request_body = loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    bytes.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    let Some((head, body)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let len = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if body.len() >= len || n == 0 {
                        break body.to_string();
                    }
                };
                recorded
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&request_body).unwrap_or(Value::Null));
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}/v1"), bodies)
    }

    fn sse(events: &[Value]) -> String {
        events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    fn tool(name: &str) -> Tool {
        Tool {
            kind: "function".into(),
            function: milim_core::api::openai::ToolFunction {
                name: name.into(),
                description: Some("List a directory.".into()),
                parameters: Some(json!({"type":"object","properties":{"path":{"type":"string"}}})),
            },
        }
    }

    #[test]
    fn recognizes_openai_reasoning_models() {
        for model in [
            "o1",
            "o3",
            "o3-mini",
            "o4-mini-2025-04-16",
            "o3-pro",
            "gpt-5",
            "gpt-5-mini",
            "gpt-5.1-codex",
            "gpt-5.5-pro",
            "gpt-6-astra",
            "codex-mini-latest",
            "openai/gpt-5",
            "GPT-5",
        ] {
            assert!(is_openai_reasoning_model(model), "{model}");
        }
        for model in [
            "gpt-4o",
            "gpt-4.1-mini",
            "gpt-5-chat-latest",
            "gpt-5.2-chat-latest",
            "gpt-4o-search-preview",
            "gpt-oss-120b",
            "gpt-realtime",
            "o1x",
            "qwen3-32b",
            "anthropic/claude-sonnet-4",
        ] {
            assert!(!is_openai_reasoning_model(model), "{model}");
        }
    }

    #[test]
    fn only_openai_reasoning_models_on_api_openai_com_use_responses() {
        let mut req = empty_req();
        let openai = RemoteBackend::new("OpenAI", "https://api.openai.com/v1", None);
        for model in ["gpt-5", "o3", "gpt-6-astra"] {
            req.model = model.into();
            assert!(openai.should_use_openai_responses(&req), "{model}");
        }
        for model in ["gpt-4o", "gpt-5-chat-latest"] {
            req.model = model.into();
            assert!(!openai.should_use_openai_responses(&req), "{model}");
        }
        // Proxies and compatible servers keep Chat Completions.
        req.model = "gpt-5".into();
        for other in [
            RemoteBackend::new("OpenRouter", "https://openrouter.ai/api/v1", None),
            RemoteBackend::new("proxy", "https://llm.example.com/v1", None),
            RemoteBackend::new("proxy", "https://api.openai.com.example.net/v1", None),
            RemoteBackend::new("vllm", "http://127.0.0.1:8000/v1", None),
        ] {
            assert!(!other.should_use_openai_responses(&req));
        }
    }

    #[test]
    fn chat_completions_sends_openai_reasoning_limits_through_proxies() {
        let mut req = empty_req();
        req.sampling.max_tokens = Some(512);
        req.sampling.temperature = Some(0.3);
        req.sampling.top_p = Some(0.9);

        req.model = "openai/gpt-5".into();
        let proxy = RemoteBackend::new("LiteLLM", "https://llm.example.com/v1", None);
        let body = serde_json::to_value(proxy.build_body(&req, true)).unwrap();
        assert_eq!(body["max_completion_tokens"], 512);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());

        // OpenRouter maps `max_tokens` itself.
        let openrouter = RemoteBackend::new("OpenRouter", "https://openrouter.ai/api/v1", None);
        let body = serde_json::to_value(openrouter.build_body(&req, true)).unwrap();
        assert_eq!(body["max_tokens"], 512);
        assert!(body.get("temperature").is_none());

        // Non-reasoning OpenAI models keep sampling, with the current field.
        req.model = "gpt-4o".into();
        let openai = RemoteBackend::new("OpenAI", "https://api.openai.com/v1", None);
        let body = serde_json::to_value(openai.build_body(&req, true)).unwrap();
        assert_eq!(body["max_completion_tokens"], 512);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("temperature").is_some());

        req.model = "llama3".into();
        let local = RemoteBackend::new("Ollama", "http://localhost:11434/v1", None);
        let body = serde_json::to_value(local.build_body(&req, true)).unwrap();
        assert_eq!(body["max_tokens"], 512);
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("temperature").is_some());
    }

    #[test]
    fn chat_completions_never_sends_provider_state() {
        let mut req = empty_req();
        let mut assistant = milim_core::api::openai::ChatMessage::text("assistant", "hi");
        assistant.provider_state = Some(json!({"anthropic": {"blocks": []}}));
        req.messages = vec![assistant];
        for backend in [
            RemoteBackend::new("OpenAI", "https://api.openai.com/v1", None),
            RemoteBackend::new("OpenRouter", "https://openrouter.ai/api/v1", None),
        ] {
            let body = backend.chat_body(&req, true).unwrap();
            assert!(body["messages"][0].get("provider_state").is_none());
            assert!(body["messages"][0].get("reasoning_details").is_none());
        }
    }

    #[test]
    fn builds_openai_responses_body() {
        let mut req = empty_req();
        req.model = "gpt-5".into();
        req.messages = vec![
            milim_core::api::openai::ChatMessage::text("system", "Be brief."),
            milim_core::api::openai::ChatMessage::text("user", "List files."),
        ];
        req.tools = vec![tool("list_dir")];
        req.tool_choice = Some(json!({"type":"function","function":{"name":"list_dir"}}));
        req.response_format = Some(json!({
            "type":"json_schema",
            "json_schema":{"name":"out","schema":{"type":"object"},"strict":true}
        }));
        req.reasoning_effort = Some(ReasoningEffort::High);
        req.sampling.max_tokens = Some(4);
        req.sampling.temperature = Some(0.2);
        req.sampling.top_p = Some(0.5);
        req.sampling.stop = vec!["END".into()];

        let body = openai_responses::build_body(&req, true, prompt_cache_key(&req)).unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["reasoning"], json!({"effort":"high","summary":"auto"}));
        assert_eq!(body["max_output_tokens"], 16, "raised to the API minimum");
        for absent in ["temperature", "top_p", "stop", "max_tokens", "messages"] {
            assert!(body.get(absent).is_none(), "{absent}");
        }
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "list_dir");
        assert_eq!(
            body["tool_choice"],
            json!({"type":"function","name":"list_dir"})
        );
        assert_eq!(
            body["text"]["format"],
            json!({"type":"json_schema","name":"out","schema":{"type":"object"},"strict":true})
        );
        assert!(body["prompt_cache_key"]
            .as_str()
            .unwrap()
            .starts_with("milim-"));
        assert_eq!(
            body["input"],
            json!([
                {"type":"message","role":"system","content":"Be brief."},
                {"type":"message","role":"user","content":"List files."}
            ])
        );

        // Auto and On leave the effort to the model; summaries can be off.
        req.reasoning_effort = Some(ReasoningEffort::On);
        let body = openai_responses::build_body(&req, false, None).unwrap();
        assert!(body.get("reasoning").is_none());
        req.reasoning_effort = Some(ReasoningEffort::None);
        let body = openai_responses::build_body(&req, false, None).unwrap();
        assert_eq!(body["reasoning"], json!({"effort":"none"}));
    }

    #[tokio::test]
    async fn streams_openai_responses_and_replays_reasoning_items() {
        let fixture = include_str!("../tests/fixtures/openai-responses-tool-call.sse");
        let (base, bodies) = serve(vec![(200, "text/event-stream", fixture.to_string())]).await;
        let backend = RemoteBackend::new("OpenAI", base, Some("sk-test".into()));
        let mut req = empty_req();
        req.model = "gpt-5".into();
        req.messages = vec![milim_core::api::openai::ChatMessage::text(
            "user",
            "List files.",
        )];
        req.tools = vec![tool("list_dir")];
        req.reasoning_effort = Some(ReasoningEffort::Medium);

        let mut stream = backend.stream_openai_responses(req.clone()).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        // The provider state arrives once, right before `Done`.
        assert!(matches!(
            &events[events.len() - 2],
            StreamEvent::Delta(delta) if delta.provider_state.is_some()
        ));
        assert!(matches!(events.last(), Some(StreamEvent::Done { .. })));

        let sent = bodies.lock().unwrap()[0].clone();
        assert_eq!(
            sent["reasoning"],
            json!({"effort":"medium","summary":"auto"})
        );

        // Assemble the same stream the way the agent loop does.
        let (base, _) = serve(vec![(200, "text/event-stream", fixture.to_string())]).await;
        let backend = RemoteBackend::new("OpenAI", base, None);
        let mut stream = backend.stream_openai_responses(req.clone()).await.unwrap();
        let mut tools = crate::ToolCallAccumulator::default();
        let (mut content, mut reasoning, mut state, mut done) =
            (String::new(), String::new(), None, None);
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                StreamEvent::Delta(delta) => {
                    content.push_str(delta.content.as_deref().unwrap_or_default());
                    reasoning.push_str(delta.reasoning.as_deref().unwrap_or_default());
                    delta
                        .tool_calls
                        .into_iter()
                        .for_each(|call| tools.push(call));
                    state = delta.provider_state.or(state);
                }
                StreamEvent::Done {
                    finish_reason,
                    usage,
                } => done = Some((finish_reason, usage)),
            }
        }
        let (finish_reason, usage) = done.unwrap();
        assert_eq!(finish_reason, "tool_calls");
        assert_eq!(usage.prompt_tokens, 1200);
        assert_eq!(usage.completion_tokens, 340);
        assert_eq!(usage.total_tokens, 1540);
        assert_eq!(usage.cache_read_tokens, Some(1024));
        assert_eq!(content, "Checking the files.");
        assert_eq!(
            reasoning,
            "**Planning** I need the file list.\n\nThen read it."
        );
        let calls = tools.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id.as_deref(), Some("call_abc"));
        assert_eq!(calls[0].function.name, "list_dir");
        assert_eq!(calls[0].function.arguments, r#"{"path":"."}"#);
        let state = state.unwrap();
        let reasoning_item = json!({
            "id": "rs_1",
            "type": "reasoning",
            "summary": [
                {"type":"summary_text","text":"**Planning** I need the file list."},
                {"type":"summary_text","text":"Then read it."}
            ],
            "encrypted_content": "gAAAAB-encrypted-reasoning-1"
        });
        assert_eq!(
            state,
            json!({"openai_responses": {
                "model": "gpt-5",
                "items": [reasoning_item.clone()],
                "phase": "commentary"
            }})
        );

        // The next request replays the reasoning ahead of the call it led to.
        let mut next = req.clone();
        next.messages.push(milim_core::api::openai::ChatMessage {
            role: "assistant".into(),
            content: Some(Content::Text(content)),
            name: None,
            tool_calls: Some(calls),
            tool_call_id: None,
            reasoning_content: Some(reasoning),
            provider_state: Some(state.clone()),
        });
        next.messages.push(milim_core::api::openai::ChatMessage {
            role: "tool".into(),
            content: Some(Content::Text("a.txt".into())),
            name: None,
            tool_calls: None,
            tool_call_id: Some("call_abc".into()),
            reasoning_content: None,
            provider_state: None,
        });
        let body = openai_responses::build_body(&next, true, None).unwrap();
        assert_eq!(
            body["input"],
            json!([
                {"type":"message","role":"user","content":"List files."},
                reasoning_item,
                {"type":"message","role":"assistant","content":"Checking the files.","phase":"commentary"},
                {"type":"function_call","call_id":"call_abc","name":"list_dir","arguments":"{\"path\":\".\"}"},
                {"type":"function_call_output","call_id":"call_abc","output":"a.txt"}
            ])
        );

        // Encrypted reasoning from another model is not replayed.
        next.model = "o3".into();
        let body = openai_responses::build_body(&next, true, None).unwrap();
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] != "reasoning"));
        assert!(body["input"][1].get("phase").is_none());
    }

    #[tokio::test]
    async fn openai_responses_drop_summaries_for_unverified_organizations() {
        let rejection = json!({"error":{
            "message":"Your organization must be verified to generate reasoning summaries. Please go to: https://platform.openai.com/settings/organization/general and click on Verify Organization.",
            "type":"invalid_request_error",
            "param":"reasoning.summary",
            "code":"unsupported_value"
        }})
        .to_string();
        let done = sse(&[
            json!({"type":"response.output_text.delta","output_index":0,"delta":"ok"}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":1,"total_tokens":6}}}),
        ]);
        let (base, bodies) = serve(vec![
            (400, "application/json", rejection),
            (200, "text/event-stream", done.clone()),
            (200, "text/event-stream", done),
        ])
        .await;
        let backend = RemoteBackend::new("OpenAI", base, None);
        let mut req = empty_req();
        req.model = "o3".into();

        for _ in 0..2 {
            let _stream = backend.stream_openai_responses(req.clone()).await.unwrap();
        }
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies[0]["reasoning"]["summary"], "auto");
        assert!(bodies[1].get("reasoning").is_none());
        assert!(bodies[2].get("reasoning").is_none(), "remembered");
    }

    #[tokio::test]
    async fn openai_responses_report_truncation_failures_and_cutoffs() {
        let incomplete = sse(&[
            json!({"type":"response.output_text.delta","output_index":0,"delta":"partial"}),
            json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":5,"output_tokens":16,"total_tokens":21}}}),
        ]);
        let failed = sse(&[json!({
            "type":"response.failed",
            "response":{"error":{"code":"server_error","message":"The model failed to generate a response."}}
        })]);
        let cut_off =
            sse(&[json!({"type":"response.output_text.delta","output_index":0,"delta":"partial"})]);
        let (base, _) = serve(vec![
            (200, "text/event-stream", incomplete),
            (200, "text/event-stream", failed),
            (200, "text/event-stream", cut_off),
        ])
        .await;
        let backend = RemoteBackend::new("OpenAI", base, None);
        let mut req = empty_req();
        req.model = "gpt-5".into();
        let drain = |req: CompletionRequest| {
            let backend = backend.clone();
            async move {
                let mut stream = backend.stream_openai_responses(req).await?;
                let mut last = None;
                while let Some(event) = stream.next().await {
                    last = Some(event?);
                }
                Ok::<_, Error>(last)
            }
        };

        let Some(StreamEvent::Done {
            finish_reason,
            usage,
        }) = drain(req.clone()).await.unwrap()
        else {
            panic!("expected done");
        };
        assert_eq!(finish_reason, "length");
        assert_eq!(usage.completion_tokens, 16);

        let error = drain(req.clone()).await.unwrap_err();
        assert!(error.to_string().contains("-> 500 server_error"), "{error}");

        let error = drain(req).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("stream ended before a completion event"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn chat_stream_needs_a_finish_reason_or_done_marker() {
        let chunk = |delta: Value, finish: Value| {
            json!({"id":"x","object":"chat.completion.chunk","created":1,"model":"m",
                "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
        };
        let finish_only = sse(&[
            chunk(json!({"content":"hi"}), Value::Null),
            chunk(json!({}), json!("stop")),
        ]);
        let done_only = format!(
            "{}data: [DONE]",
            sse(&[chunk(json!({"content":"hi"}), Value::Null)])
        );
        let neither = sse(&[chunk(json!({"content":"hi"}), Value::Null)]);
        let (base, _) = serve(vec![
            (200, "text/event-stream", finish_only),
            (200, "text/event-stream", done_only),
            (200, "text/event-stream", neither),
        ])
        .await;
        let backend = RemoteBackend::new("local", base, None);

        let out = backend.complete(empty_req()).await.unwrap();
        assert_eq!(out.message.text_content(), "hi");
        // `[DONE]` without a trailing newline still counts.
        let out = backend.complete(empty_req()).await.unwrap();
        assert_eq!(out.finish_reason, "stop");
        let error = backend.complete(empty_req()).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("stream ended before a completion event"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn chat_stream_reads_index_less_tool_calls_and_loose_chunks() {
        // Shaped like Gemini's OpenAI-compatible endpoint: no chunk `id` or
        // `created`, and whole tool calls without `index`, one per chunk.
        let events = [
            json!({"object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"Looking.","tool_calls":[
                {"id":"call_a","type":"function","function":{"name":"list_dir","arguments":"{\"path\":\"a\"}"}}
            ]}}]}),
            json!({"object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"tool_calls":[
                {"id":"call_b","type":"function","function":{"name":"list_dir","arguments":{"path":"b"}}}
            ]},"finish_reason":"tool_calls"}]}),
        ];
        let (base, _) = serve(vec![(200, "text/event-stream", sse(&events))]).await;
        let backend = RemoteBackend::new("gemini-compat", base, None);
        let out = backend.complete(empty_req()).await.unwrap();
        assert_eq!(out.message.text_content(), "Looking.");
        assert_eq!(out.finish_reason, "tool_calls");
        let calls = out.message.tool_calls.unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id.as_deref(), Some("call_a"));
        assert_eq!(calls[0].function.arguments, r#"{"path":"a"}"#);
        assert_eq!(calls[1].id.as_deref(), Some("call_b"));
        assert_eq!(calls[1].function.name, "list_dir");
        assert_eq!(calls[1].function.arguments, r#"{"path":"b"}"#);

        // Without ids, fragments continue the call at their position.
        let mut chat = ChatStream::new(None);
        let first = chat.apply(&json!({"choices":[{"delta":{"tool_calls":[
            {"function":{"name":"f","arguments":"{\"a\":"}}
        ]}}]}));
        let second = chat.apply(&json!({"choices":[{"delta":{"tool_calls":[
            {"function":{"arguments":"1}"}}
        ]}}]}));
        assert_eq!(first.tool_calls[0].index, 0);
        assert_eq!(second.tool_calls[0].index, 0);
    }

    #[test]
    fn recovers_tool_calls_that_local_models_write_as_text() {
        let chunk = |content: &str| json!({"choices":[{"delta":{"content":content}}]});
        let run = |pieces: &[&str], finish: Value| {
            let mut chat = ChatStream::new(None).with_text_tool_calls(&[tool("list_dir")]);
            let mut streamed = String::new();
            for piece in pieces {
                streamed.push_str(
                    chat.apply(&chunk(piece))
                        .content
                        .as_deref()
                        .unwrap_or_default(),
                );
            }
            chat.apply(&json!({"choices":[{"delta":{},"finish_reason":finish}]}));
            let mut tail = DeltaEvent::default();
            let mut finish_reason = String::new();
            for event in chat.finish() {
                match event {
                    StreamEvent::Delta(delta) => tail = delta,
                    StreamEvent::Done {
                        finish_reason: reason,
                        ..
                    } => finish_reason = reason,
                }
            }
            (streamed, tail, finish_reason)
        };

        let (streamed, tail, finish) = run(
            &[
                "\n<tool",
                "_call>\n{\"name\": \"list_dir\", ",
                "\"arguments\": {\"path\": \".\"}}\n</tool_call>",
            ],
            json!("stop"),
        );
        assert_eq!(streamed, "", "held back while it may be a call");
        assert_eq!(finish, "tool_calls");
        assert!(tail.content.is_none());
        assert_eq!(tail.tool_calls.len(), 1);
        assert_eq!(
            tail.tool_calls[0].function.name.as_deref(),
            Some("list_dir")
        );
        assert_eq!(
            tail.tool_calls[0].function.arguments.as_deref(),
            Some(r#"{"path":"."}"#)
        );

        // An unknown tool, extra prose, or plain text stays text.
        for pieces in [
            &["<tool_call>{\"name\":\"rm\",\"arguments\":{}}</tool_call>"][..],
            &["<tool_call>{\"name\":\"list_dir\",\"arguments\":{}}</tool_call> done"][..],
            &["<b>", "bold</b>"][..],
        ] {
            let (streamed, tail, finish) = run(pieces, json!("stop"));
            assert_eq!(finish, "stop");
            assert!(tail.tool_calls.is_empty());
            assert_eq!(
                format!("{streamed}{}", tail.content.unwrap_or_default()),
                pieces.concat()
            );
        }
        let (streamed, tail, _) = run(&["Hello ", "<tool_call>"], json!("stop"));
        assert_eq!(streamed, "Hello <tool_call>");
        assert!(tail.content.is_none());
    }

    #[tokio::test]
    async fn openrouter_keeps_reasoning_details_across_tool_steps() {
        let chunk = |delta: Value, finish: Value| {
            json!({"id":"gen","object":"chat.completion.chunk","created":1,"model":"anthropic/claude-sonnet-4",
                "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
        };
        let events = [
            chunk(
                json!({"reasoning":"Let me ","reasoning_details":[
                    {"type":"reasoning.text","text":"Let me ","format":"anthropic-claude-v1","index":0}
                ]}),
                Value::Null,
            ),
            chunk(
                json!({"reasoning":"look.","reasoning_details":[
                    {"type":"reasoning.text","text":"look.","format":"anthropic-claude-v1","index":0}
                ]}),
                Value::Null,
            ),
            chunk(
                json!({"reasoning_details":[
                    {"type":"reasoning.text","text":"","signature":"sig-1","format":"anthropic-claude-v1","index":0}
                ]}),
                Value::Null,
            ),
            chunk(
                json!({"tool_calls":[
                    {"index":0,"id":"toolu_1","type":"function","function":{"name":"list_dir","arguments":"{}"}}
                ]}),
                json!("tool_calls"),
            ),
        ];
        let body = format!("{}data: [DONE]\n\n", sse(&events));
        let (base, _) = serve(vec![(200, "text/event-stream", body)]).await;
        let backend = RemoteBackend::new("OpenRouter", base, None);
        let mut req = empty_req();
        req.model = "anthropic/claude-sonnet-4".into();
        let out = backend.complete(req.clone()).await.unwrap();
        let state = out.message.provider_state.clone().unwrap();
        assert_eq!(
            state,
            json!({"openrouter": {
                "model": "anthropic/claude-sonnet-4",
                "reasoning_details": [{
                    "type":"reasoning.text",
                    "text":"Let me look.",
                    "signature":"sig-1",
                    "format":"anthropic-claude-v1",
                    "index":0
                }]
            }})
        );

        // Replayed on the assistant turn in place of its plain reasoning.
        let openrouter = RemoteBackend::new("OpenRouter", "https://openrouter.ai/api/v1", None);
        req.messages = vec![
            milim_core::api::openai::ChatMessage::text("system", "Be brief."),
            milim_core::api::openai::ChatMessage::text("user", "List files."),
            out.message,
            milim_core::api::openai::ChatMessage {
                role: "tool".into(),
                content: Some(Content::Text("a.txt".into())),
                name: None,
                tool_calls: None,
                tool_call_id: Some("toolu_1".into()),
                reasoning_content: None,
                provider_state: None,
            },
        ];
        let body = openrouter.chat_body(&req, true).unwrap();
        let assistant = &body["messages"][2];
        assert_eq!(
            assistant["reasoning_details"],
            state["openrouter"]["reasoning_details"]
        );
        assert!(assistant.get("reasoning_content").is_none());
        assert!(assistant.get("provider_state").is_none());
        // Claude caches only at explicit breakpoints.
        assert_eq!(
            body["messages"][0]["content"],
            json!([{"type":"text","text":"Be brief.","cache_control":{"type":"ephemeral"}}])
        );
        assert_eq!(
            body["messages"][1]["content"],
            json!([{"type":"text","text":"List files.","cache_control":{"type":"ephemeral"}}])
        );
        assert_eq!(body["messages"][3]["content"], "a.txt");

        // Another model neither replays these details nor gets breakpoints.
        req.model = "openai/gpt-5".into();
        let body = openrouter.chat_body(&req, true).unwrap();
        assert!(body["messages"][2].get("reasoning_details").is_none());
        assert_eq!(body["messages"][2]["reasoning_content"], "Let me look.");
        assert_eq!(body["messages"][0]["content"], "Be brief.");
    }

    fn empty_req() -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: Default::default(),
            reasoning_effort: None,
        }
    }
}
