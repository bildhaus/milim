//! Multi-provider LLM routing.
//!
//! A registry of OpenAI-compatible providers (OpenAI, OpenRouter, Groq, local
//! Ollama / LM Studio, …) whose API keys are stored **encrypted at rest**
//! (`milim-storage` AES-GCM), plus a [`ProviderRouter`] that dispatches each
//! request to whichever provider serves the requested model — falling back to
//! the default backend otherwise.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use milim_core::api::openai::{
    ChatMessage, Content, ContentPart, ImageUrl, Model, ModelCapabilities, ModelPricing,
    ModelReasoningMetadata, ReasoningEffort, Tool, ToolFunction,
};
use milim_core::{Error, Result};
use milim_inference::anthropic::AnthropicBackend;
use milim_inference::gemini::GeminiBackend;
use milim_inference::remote::RemoteBackend;
use milim_inference::{CompletionRequest, EventStream, ModelService, SharedService, StreamEvent};
use milim_storage::{create_private_file, EncryptedStore};
use milim_tools::atomic_write;

use crate::privacy::{self, PrivacyGate, PrivacyMode};

fn default_true() -> bool {
    true
}

/// Provider wire protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    #[serde(rename = "openai_compatible", alias = "open_ai_compatible")]
    OpenAiCompatible,
    Anthropic,
    Gemini,
    Replicate,
    Fal,
    /// Brave Search API credential for the `web_search` tool.
    BraveSearch,
    /// Tavily search API credential for the `web_search` tool.
    Tavily,
}

impl ProviderKind {
    pub fn is_chat(self) -> bool {
        matches!(
            self,
            ProviderKind::OpenAiCompatible | ProviderKind::Anthropic | ProviderKind::Gemini
        )
    }

    fn web_search_provider(self) -> Option<milim_tools::WebSearchProvider> {
        match self {
            ProviderKind::BraveSearch => Some(milim_tools::WebSearchProvider::Brave),
            ProviderKind::Tavily => Some(milim_tools::WebSearchProvider::Tavily),
            _ => None,
        }
    }
}

/// A configured provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ProviderKind,
    /// Base URL including the version segment, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Model ids the provider serves (cached on add/refresh).
    #[serde(default)]
    pub models: Vec<String>,
    /// Provider-supplied per-token pricing keyed by model id. Only trusted for
    /// OpenRouter, whose models API exposes prompt/completion prices.
    #[serde(default)]
    pub pricing: BTreeMap<String, ModelPricing>,
    /// Provider-supplied context/token limits keyed by model id.
    #[serde(default)]
    pub model_context: BTreeMap<String, ModelContextMetadata>,
    /// Provider-supplied or inferred reasoning controls keyed by model id.
    #[serde(default)]
    pub model_reasoning: BTreeMap<String, ModelReasoningMetadata>,
    /// Provider-supplied model capabilities keyed by model id.
    #[serde(default)]
    pub model_capabilities: BTreeMap<String, ModelCapabilities>,
    /// Explicit user choices layered over provider discovery. These are kept
    /// separately so a refresh cannot erase them.
    #[serde(default)]
    pub model_overrides: BTreeMap<String, ModelCapabilityOverride>,
    /// Last connection error from the model fetch (so the UI can explain an
    /// empty model list — e.g. server down, bad key, wrong URL).
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCapabilityOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_efforts: Vec<ReasoningEffort>,
    /// Context window in tokens, replacing the provider-reported or
    /// built-in value (for example a local server's configured context).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityProbeResult {
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCapabilityVerification {
    pub model: String,
    pub vision: CapabilityProbeResult,
    pub reasoning: CapabilityProbeResult,
    pub tools: CapabilityProbeResult,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelContextMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
}

#[derive(Clone)]
struct Runtime {
    cfg: Provider,
    backend: SharedService,
}

fn backend_for(cfg: &Provider) -> SharedService {
    let api_key = cfg.api_key.clone().filter(|k| !k.is_empty());
    match cfg.kind {
        ProviderKind::OpenAiCompatible => Arc::new(RemoteBackend::new(
            cfg.name.clone(),
            cfg.base_url.clone(),
            api_key,
        )),
        ProviderKind::Anthropic => Arc::new(
            AnthropicBackend::new(cfg.name.clone(), cfg.base_url.clone(), api_key)
                .with_model_output_limits(cfg.model_context.iter().filter_map(
                    |(model, context)| Some((model.clone(), context.max_completion_tokens?)),
                )),
        ),
        ProviderKind::Gemini => Arc::new(GeminiBackend::new(
            cfg.name.clone(),
            cfg.base_url.clone(),
            api_key,
        )),
        ProviderKind::Replicate
        | ProviderKind::Fal
        | ProviderKind::BraveSearch
        | ProviderKind::Tavily => Arc::new(RemoteBackend::new(
            cfg.name.clone(),
            cfg.base_url.clone(),
            api_key,
        )),
    }
}

/// Encrypted on-disk persistence for the provider list (keys included).
struct ProviderStore {
    enc: EncryptedStore,
    path: std::path::PathBuf,
}

impl ProviderStore {
    fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let key = read_or_make_key(&dir.join("providers.key"))?;
        Self::open_with_encryption(dir, EncryptedStore::from_key(&key))
    }

    fn open_with_encryption(dir: &Path, enc: EncryptedStore) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            enc,
            path: dir.join("providers.enc"),
        })
    }

    fn load(&self) -> Result<Vec<Provider>> {
        let blob = match std::fs::read(&self.path) {
            Ok(blob) => blob,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let plain = self.enc.decrypt(&blob)?;
        serde_json::from_slice(&plain).map_err(Into::into)
    }

    fn save(&self, providers: &[Provider]) -> Result<()> {
        let plain = serde_json::to_vec(providers)?;
        let blob = self.enc.encrypt(&plain)?;
        atomic_write(&self.path, &blob)
    }
}

fn read_or_make_key(path: &Path) -> Result<[u8; 32]> {
    match std::fs::read(path) {
        Ok(b) if b.len() == 32 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(&b);
            return Ok(k);
        }
        Ok(b) => {
            return Err(Error::Other(format!(
                "invalid provider encryption key length: expected 32 bytes, got {}",
                b.len()
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let k = EncryptedStore::random_key();
    create_private_file(path, &k)?;
    Ok(k)
}

/// Live provider registry: the source of truth for the router + the CRUD API.
pub struct ProviderRegistry {
    inner: Arc<RwLock<Vec<Runtime>>>,
    local: SharedService,
    store: ProviderStore,
}

impl ProviderRegistry {
    /// Load persisted providers (with their cached model lists) from `dir`.
    /// Cheap + synchronous: model lists are refreshed on upsert, not here.
    pub fn open(dir: &Path, local: SharedService) -> Result<Self> {
        let store = ProviderStore::open(dir)?;
        Self::open_with_store(store, local)
    }

    pub fn open_with_encryption(
        dir: &Path,
        local: SharedService,
        encryption: EncryptedStore,
    ) -> Result<Self> {
        let store = ProviderStore::open_with_encryption(dir, encryption)?;
        Self::open_with_store(store, local)
    }

    fn open_with_store(store: ProviderStore, local: SharedService) -> Result<Self> {
        let runtimes = store
            .load()?
            .into_iter()
            .map(|cfg| Runtime {
                backend: backend_for(&cfg),
                cfg,
            })
            .collect();
        Ok(Self {
            inner: Arc::new(RwLock::new(runtimes)),
            local,
            store,
        })
    }

    /// A `ModelService` view that routes by model → provider, applying the
    /// outbound privacy `gate` to any request bound for a remote provider.
    pub fn router(&self, gate: Arc<PrivacyGate>) -> ProviderRouter {
        ProviderRouter {
            inner: self.inner.clone(),
            local: self.local.clone(),
            privacy: gate,
        }
    }

    /// A router whose privacy mode is fixed for one request/run.
    pub fn router_with_privacy(&self, mode: PrivacyMode) -> ProviderRouter {
        let gate = Arc::new(PrivacyGate::default());
        gate.set(mode);
        self.router(gate)
    }

    /// All providers (keys included — callers redact before returning to UIs).
    pub async fn list(&self) -> Vec<Provider> {
        self.inner
            .read()
            .await
            .iter()
            .map(|r| r.cfg.clone())
            .collect()
    }

    /// Insert or update a provider, fetching its model list. Preserves the
    /// stored key when `cfg.api_key` is `None` (the UI omits it on edit).
    pub async fn upsert(&self, mut cfg: Provider) -> Result<Provider> {
        if cfg.api_key.is_none() {
            cfg.api_key = self
                .inner
                .read()
                .await
                .iter()
                .find(|r| r.cfg.id == cfg.id)
                .and_then(|r| r.cfg.api_key.clone());
        }
        let backend = backend_for(&cfg);
        if cfg.kind.is_chat() {
            match backend.list_models().await {
                Ok(ms) => {
                    let trust_pricing = is_openrouter_provider(&cfg);
                    cfg.model_context = collect_model_context(&ms);
                    cfg.model_reasoning = collect_model_reasoning(&ms, &cfg);
                    cfg.model_capabilities = collect_model_capabilities(&ms);
                    cfg.pricing = collect_pricing(&ms, trust_pricing);
                    cfg.models = ms.into_iter().map(|m| m.id).collect();
                    cfg.last_error = None;
                }
                Err(e) => {
                    cfg.models = Vec::new();
                    cfg.pricing = BTreeMap::new();
                    cfg.model_context = BTreeMap::new();
                    cfg.model_reasoning = BTreeMap::new();
                    cfg.model_capabilities = BTreeMap::new();
                    cfg.last_error = Some(e.to_string());
                }
            }
        } else {
            cfg.models = Vec::new();
            cfg.pricing = BTreeMap::new();
            cfg.model_context = BTreeMap::new();
            cfg.model_reasoning = BTreeMap::new();
            cfg.model_capabilities = BTreeMap::new();
            cfg.last_error = None;
        }
        apply_model_overrides(&mut cfg);

        let mut w = self.inner.write().await;
        let mut next = w.clone();
        if let Some(r) = next.iter_mut().find(|r| r.cfg.id == cfg.id) {
            r.cfg = cfg.clone();
            r.backend = backend;
        } else {
            next.push(Runtime {
                cfg: cfg.clone(),
                backend,
            });
        }
        self.store
            .save(&next.iter().map(|r| r.cfg.clone()).collect::<Vec<_>>())?;
        *w = next;
        Ok(cfg)
    }

    /// Re-fetch the model list for every enabled provider. Called at startup so
    /// providers populate (or surface a connection error) without a manual
    /// re-save — the T3-style "add a key, models light up" behavior.
    pub async fn refresh_all(&self) -> Result<bool> {
        let configs: Vec<Provider> = {
            let r = self.inner.read().await;
            r.iter()
                .filter(|rt| rt.cfg.enabled)
                .filter(|rt| rt.cfg.kind.is_chat())
                .map(|rt| rt.cfg.clone())
                .collect()
        };
        let refreshed = !configs.is_empty();
        for cfg in configs {
            let backend = backend_for(&cfg);
            let result = backend.list_models().await;
            let mut w = self.inner.write().await;
            if let Some(rt) = w.iter_mut().find(|rt| rt.cfg.id == cfg.id) {
                match result {
                    Ok(models) => {
                        rt.cfg.model_context = collect_model_context(&models);
                        rt.cfg.model_reasoning = collect_model_reasoning(&models, &cfg);
                        rt.cfg.model_capabilities = collect_model_capabilities(&models);
                        rt.cfg.pricing = collect_pricing(&models, is_openrouter_provider(&cfg));
                        rt.cfg.models = models.into_iter().map(|model| model.id).collect();
                        rt.cfg.last_error = None;
                    }
                    Err(error) => {
                        rt.cfg.last_error = Some(error.to_string());
                    }
                }
                apply_model_overrides(&mut rt.cfg);
            }
        }
        let snapshot = self
            .inner
            .read()
            .await
            .iter()
            .map(|rt| rt.cfg.clone())
            .collect::<Vec<_>>();
        self.store.save(&snapshot)?;
        Ok(refreshed)
    }

    /// The first enabled, keyed web-search provider, in configuration order.
    pub async fn web_search_api(&self) -> Option<milim_tools::WebSearchApi> {
        self.inner.read().await.iter().find_map(|runtime| {
            let cfg = &runtime.cfg;
            let provider = cfg.kind.web_search_provider()?;
            let api_key = cfg.api_key.as_deref().map(str::trim).unwrap_or("");
            (cfg.enabled && !api_key.is_empty()).then(|| milim_tools::WebSearchApi {
                provider,
                api_key: api_key.to_string(),
                base_url: Some(cfg.base_url.clone()).filter(|url| !url.trim().is_empty()),
            })
        })
    }

    /// Remove a provider. Returns whether one was removed.
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let mut w = self.inner.write().await;
        let mut next = w.clone();
        let n = next.len();
        next.retain(|r| r.cfg.id != id);
        let removed = next.len() != n;
        if removed {
            self.store
                .save(&next.iter().map(|r| r.cfg.clone()).collect::<Vec<_>>())?;
            *w = next;
        }
        Ok(removed)
    }

    /// Run small, explicit capability probes against one configured model.
    /// This never changes provider metadata; the UI may choose which results
    /// to save as overrides after showing them to the user.
    pub async fn verify_model_capabilities(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<ModelCapabilityVerification> {
        let backend = self
            .inner
            .read()
            .await
            .iter()
            .find(|runtime| {
                runtime.cfg.id == provider_id
                    && runtime.cfg.enabled
                    && runtime
                        .cfg
                        .models
                        .iter()
                        .any(|candidate| candidate == model)
            })
            .map(|runtime| runtime.backend.clone())
            .ok_or_else(|| {
                Error::ModelNotFound(format!("model {model} for provider {provider_id}"))
            })?;
        // Probes use request shapes current models accept: no sampling
        // parameters, no forced tool choice, and room for a model that
        // thinks by default to finish its answer.
        let base_request = |messages: Vec<ChatMessage>, max_tokens: u32| CompletionRequest {
            model: model.to_string(),
            messages,
            tools: Vec::new(),
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: milim_inference::SamplingParams {
                max_tokens: Some(max_tokens),
                ..Default::default()
            },
            reasoning_effort: None,
        };

        let vision = probe_completion(
            backend.as_ref(),
            base_request(
                vec![ChatMessage {
                    role: "user".into(),
                    content: Some(Content::Parts(vec![
                        ContentPart::Text {
                            text: "Reply with the dominant color of this one-pixel image.".into(),
                        },
                        ContentPart::ImageUrl {
                            image_url: ImageUrl {
                                url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6ZJAAAAAASUVORK5CYII=".into(),
                                detail: Some("low".into()),
                            },
                        },
                    ])),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: None,
                    provider_state: None,
                }],
                2_048,
            ),
            PROBE_TIMEOUT,
            |_| true,
        )
        .await;

        // Reasoning shows up as reasoning deltas, or as continuation data
        // (thinking blocks, reasoning items) when a provider withholds the
        // reasoning text itself.
        let mut reasoning_request = base_request(
            vec![ChatMessage::text(
                "user",
                "How many times does the letter r appear in \"strawberry raspberry\"? \
                 Reason it through, then answer with the number only.",
            )],
            8_192,
        );
        reasoning_request.reasoning_effort = Some(ReasoningEffort::High);
        let reasoning = probe_completion(
            backend.as_ref(),
            reasoning_request,
            REASONING_PROBE_TIMEOUT,
            |output| output.reasoning,
        )
        .await;

        let mut tools_request = base_request(
            vec![ChatMessage::text(
                "user",
                "Call the milim_capability_probe tool exactly once with value \"ok\".",
            )],
            2_048,
        );
        tools_request.tools = vec![Tool {
            kind: "function".into(),
            function: ToolFunction {
                name: "milim_capability_probe".into(),
                description: Some("Return a fixed capability probe value".into()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"]
                })),
            },
        }];
        let tools = probe_completion(backend.as_ref(), tools_request, PROBE_TIMEOUT, |output| {
            output
                .tool_names
                .iter()
                .any(|name| name == "milim_capability_probe")
        })
        .await;

        Ok(ModelCapabilityVerification {
            model: model.to_string(),
            vision,
            reasoning,
            tools,
        })
    }
}

const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const REASONING_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// What a capability probe saw in the model's stream.
#[derive(Debug, Default)]
struct ProbeOutput {
    /// Any non-empty reasoning delta, or provider continuation data.
    reasoning: bool,
    tool_names: Vec<String>,
    finish_reason: String,
}

async fn probe_completion(
    backend: &dyn ModelService,
    request: CompletionRequest,
    timeout: std::time::Duration,
    supported: impl FnOnce(&ProbeOutput) -> bool,
) -> CapabilityProbeResult {
    let run = async {
        let mut stream = backend.stream(request).await?;
        let mut output = ProbeOutput::default();
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::Delta(delta) => {
                    output.reasoning |= delta
                        .reasoning
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty())
                        || delta.provider_state.is_some();
                    output.tool_names.extend(
                        delta
                            .tool_calls
                            .into_iter()
                            .filter_map(|call| call.function.name),
                    );
                }
                StreamEvent::Done { finish_reason, .. } => output.finish_reason = finish_reason,
            }
        }
        Ok::<_, Error>(output)
    };
    match tokio::time::timeout(timeout, run).await {
        Ok(Ok(output)) => {
            let is_supported = supported(&output);
            CapabilityProbeResult {
                supported: is_supported,
                error: (!is_supported && output.finish_reason == "length")
                    .then(|| "capability probe reached its output-token limit".into()),
            }
        }
        Ok(Err(error)) => CapabilityProbeResult {
            supported: false,
            error: Some(error.to_string()),
        },
        Err(_) => CapabilityProbeResult {
            supported: false,
            error: Some("capability probe timed out".into()),
        },
    }
}

fn is_openrouter_provider(provider: &Provider) -> bool {
    provider.kind == ProviderKind::OpenAiCompatible
        && (provider.name.trim().eq_ignore_ascii_case("openrouter")
            || provider
                .base_url
                .to_ascii_lowercase()
                .contains("openrouter.ai/"))
}

fn collect_pricing(models: &[Model], trusted: bool) -> BTreeMap<String, ModelPricing> {
    if !trusted {
        return BTreeMap::new();
    }
    models
        .iter()
        .filter_map(|model| {
            model
                .pricing
                .clone()
                .map(|pricing| (model.id.clone(), pricing))
        })
        .collect()
}

fn collect_model_context(models: &[Model]) -> BTreeMap<String, ModelContextMetadata> {
    models
        .iter()
        .filter_map(|model| {
            if model.context_length.is_none()
                && model.max_prompt_tokens.is_none()
                && model.max_completion_tokens.is_none()
            {
                return None;
            }
            Some((
                model.id.clone(),
                ModelContextMetadata {
                    context_length: model.context_length,
                    max_prompt_tokens: model.max_prompt_tokens,
                    max_completion_tokens: model.max_completion_tokens,
                },
            ))
        })
        .collect()
}

fn collect_model_capabilities(models: &[Model]) -> BTreeMap<String, ModelCapabilities> {
    models
        .iter()
        .filter_map(|model| {
            model
                .capabilities
                .clone()
                .map(|capabilities| (model.id.clone(), capabilities))
        })
        .collect()
}

fn collect_model_reasoning(
    models: &[Model],
    provider: &Provider,
) -> BTreeMap<String, ModelReasoningMetadata> {
    models
        .iter()
        .filter_map(|model| {
            let reasoning = model
                .reasoning
                .clone()
                .or_else(|| fallback_model_reasoning(provider, &model.id))?;
            Some((model.id.clone(), reasoning))
        })
        .collect()
}

fn apply_model_overrides(provider: &mut Provider) {
    for (model, override_) in provider.model_overrides.clone() {
        if let Some(window) = override_.context_window.filter(|window| *window > 0) {
            let fallback = fallback_model_context(provider, &model);
            let context = provider
                .model_context
                .entry(model.clone())
                .or_insert(fallback);
            context.context_length = Some(window);
            context.max_prompt_tokens = None;
        }
        if override_.image_input.is_some() || override_.tool_use.is_some() {
            let capabilities = provider
                .model_capabilities
                .entry(model.clone())
                .or_default();
            if let Some(value) = override_.image_input {
                capabilities.image_input = Some(value);
            }
            if let Some(value) = override_.tool_use {
                capabilities.tool_use = Some(value);
            }
        }
        match override_.reasoning {
            Some(false) => {
                provider.model_reasoning.remove(&model);
            }
            Some(true) => {
                let efforts = if override_.supported_efforts.is_empty() {
                    vec![
                        ReasoningEffort::None,
                        ReasoningEffort::Minimal,
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::Xhigh,
                        ReasoningEffort::Max,
                    ]
                } else {
                    override_.supported_efforts.clone()
                };
                let default_effort = efforts
                    .contains(&ReasoningEffort::Medium)
                    .then_some(ReasoningEffort::Medium)
                    .or_else(|| efforts.first().copied());
                provider.model_reasoning.insert(
                    model,
                    reasoning_meta(
                        &efforts,
                        default_effort,
                        true,
                        !efforts.contains(&ReasoningEffort::None),
                    ),
                );
            }
            None if !override_.supported_efforts.is_empty() => {
                let default_effort = override_
                    .supported_efforts
                    .contains(&ReasoningEffort::Medium)
                    .then_some(ReasoningEffort::Medium)
                    .or_else(|| override_.supported_efforts.first().copied());
                provider.model_reasoning.insert(
                    model,
                    reasoning_meta(
                        &override_.supported_efforts,
                        default_effort,
                        true,
                        !override_.supported_efforts.contains(&ReasoningEffort::None),
                    ),
                );
            }
            None => {}
        }
    }
}

/// Prompt budget for one model: the provider-reported prompt or context
/// limit, else the same fallback the model catalog shows.
pub(crate) fn model_context_window(provider: &Provider, model: &str) -> Option<u32> {
    let context = provider
        .model_context
        .get(model)
        .cloned()
        .unwrap_or_else(|| fallback_model_context(provider, model));
    context.max_prompt_tokens.or(context.context_length)
}

/// Context for a model the provider did not describe: the documented limits
/// of a known hosted model family, else a conservative default. Local
/// servers (Ollama, LM Studio, loopback) run whatever context they were
/// configured with, so they always get the conservative default.
fn fallback_model_context(provider: &Provider, model: &str) -> ModelContextMetadata {
    let unknown = |context_length| ModelContextMetadata {
        context_length: Some(context_length),
        max_prompt_tokens: None,
        max_completion_tokens: None,
    };
    let known = || known_model_context(model);
    match provider.kind {
        ProviderKind::Anthropic => known().unwrap_or_else(|| unknown(200_000)),
        ProviderKind::Gemini => known().unwrap_or_else(|| unknown(32_768)),
        ProviderKind::OpenAiCompatible if is_local_provider(provider) => unknown(32_768),
        ProviderKind::OpenAiCompatible => known().unwrap_or_else(|| unknown(32_768)),
        ProviderKind::Replicate
        | ProviderKind::Fal
        | ProviderKind::BraveSearch
        | ProviderKind::Tavily => ModelContextMetadata::default(),
    }
}

/// Documented context limits by hosted model family (OpenAI, Gemini, and
/// Claude model docs), matched on the id after any `vendor/` prefix.
/// `max_prompt_tokens` is set where the vendor caps input below the window.
fn known_model_context(model: &str) -> Option<ModelContextMetadata> {
    let limits = |context: u32, prompt: Option<u32>, output: Option<u32>| {
        Some(ModelContextMetadata {
            context_length: Some(context),
            max_prompt_tokens: prompt,
            max_completion_tokens: output,
        })
    };
    let lower = model.trim().to_ascii_lowercase();
    let id = lower.rsplit('/').next().unwrap_or(&lower);
    let family = |prefix: &str| {
        id.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', ':', '.', '@']))
    };

    if let Some(context) = milim_inference::anthropic::claude_context_window(model) {
        let output = milim_inference::anthropic::claude_max_output_tokens(model);
        return limits(context, None, output);
    }
    // Embedding, image, speech, and live variants have their own, smaller
    // limits; leave them to the conservative default.
    if ["embed", "image", "tts", "audio", "live"]
        .iter()
        .any(|variant| id.contains(variant))
    {
        return None;
    }
    // OpenAI GPT-6 and the 1.05M-context GPT-5.x generations.
    if id.starts_with("gpt-6") || (id.starts_with("gpt-5.6") && !family("gpt-5.6-cyber")) {
        return limits(1_050_000, Some(922_000), Some(128_000));
    }
    if id.starts_with("gpt-5") && id.contains("-chat-latest") {
        return limits(128_000, None, Some(16_384));
    }
    if (family("gpt-5.5") || family("gpt-5.4"))
        && !family("gpt-5.4-mini")
        && !family("gpt-5.4-nano")
    {
        return limits(1_050_000, None, Some(128_000));
    }
    // The rest of GPT-5.x: 400K window, 272K input, 128K output.
    if family("gpt-5") || id.starts_with("gpt-5.") {
        return limits(400_000, Some(272_000), Some(128_000));
    }
    if family("gpt-4.1") {
        return limits(1_047_576, None, Some(32_768));
    }
    if family("gpt-4o") || family("gpt-4-turbo") || (family("gpt-4") && id.contains("-preview")) {
        return limits(128_000, None, Some(16_384));
    }
    if family("gpt-4-32k") {
        return limits(32_768, None, Some(8_192));
    }
    if family("gpt-4") {
        return limits(8_192, None, Some(8_192));
    }
    if family("gpt-3.5-turbo") {
        return limits(16_385, None, Some(4_096));
    }
    if family("o1-mini") || family("o1-preview") {
        return limits(128_000, None, Some(65_536));
    }
    if family("o1") || family("o3") || family("o4-mini") || family("codex-mini-latest") {
        return limits(200_000, None, Some(100_000));
    }
    if family("gpt-oss-120b") || family("gpt-oss-20b") {
        return limits(131_072, None, Some(131_072));
    }
    // Gemini API models take 1,048,576 input tokens; 2.0 models return up
    // to 8,192 tokens and later ones up to 65,536.
    if family("gemini-2.0") || family("gemini-1.5") {
        return limits(1_048_576, None, Some(8_192));
    }
    if id.starts_with("gemini-") {
        return limits(1_048_576, None, Some(65_536));
    }
    if family("gemma-4") {
        return limits(262_144, None, None);
    }
    None
}

fn fallback_model_reasoning(provider: &Provider, model: &str) -> Option<ModelReasoningMetadata> {
    let id = model.to_ascii_lowercase();
    match provider.kind {
        ProviderKind::Anthropic => milim_inference::anthropic::claude_reasoning_metadata(model),
        ProviderKind::Gemini => {
            if id.contains("gemini-3") {
                Some(reasoning_meta(
                    &[
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                    ],
                    Some(ReasoningEffort::High),
                    true,
                    true,
                ))
            } else if id.contains("gemini-2.5-flash") {
                Some(reasoning_meta(
                    &[
                        ReasoningEffort::None,
                        ReasoningEffort::Minimal,
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::Xhigh,
                    ],
                    None,
                    true,
                    false,
                ))
            } else if id.contains("gemini-2.5") {
                Some(reasoning_meta(
                    &[
                        ReasoningEffort::Minimal,
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::Xhigh,
                    ],
                    None,
                    true,
                    true,
                ))
            } else {
                None
            }
        }
        ProviderKind::OpenAiCompatible => {
            if is_ollama_provider(provider) {
                local_ollama_reasoning(model)
            } else if is_lm_studio_provider(provider) {
                local_lm_studio_reasoning(model)
            } else if (is_local_provider(provider) && !is_vllm_provider(provider))
                || !looks_reasoning_model(model)
            {
                None
            } else {
                Some(reasoning_meta(
                    &[
                        ReasoningEffort::None,
                        ReasoningEffort::Minimal,
                        ReasoningEffort::Low,
                        ReasoningEffort::Medium,
                        ReasoningEffort::High,
                        ReasoningEffort::Xhigh,
                    ],
                    Some(ReasoningEffort::Medium),
                    true,
                    false,
                ))
            }
        }
        ProviderKind::Replicate
        | ProviderKind::Fal
        | ProviderKind::BraveSearch
        | ProviderKind::Tavily => None,
    }
}

fn local_ollama_reasoning(model: &str) -> Option<ModelReasoningMetadata> {
    let id = model.trim().to_ascii_lowercase();
    if is_gpt_oss_model(&id) {
        return Some(reasoning_meta(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            Some(ReasoningEffort::Medium),
            true,
            true,
        ));
    }
    if looks_local_thinking_model(&id) {
        Some(reasoning_meta(
            &[
                ReasoningEffort::None,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Max,
            ],
            Some(ReasoningEffort::Medium),
            true,
            false,
        ))
    } else {
        None
    }
}

fn local_lm_studio_reasoning(model: &str) -> Option<ModelReasoningMetadata> {
    let id = model.trim().to_ascii_lowercase();
    is_gpt_oss_model(&id).then(|| {
        reasoning_meta(
            &[
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            Some(ReasoningEffort::Medium),
            true,
            true,
        )
    })
}

fn reasoning_meta(
    efforts: &[ReasoningEffort],
    default_effort: Option<ReasoningEffort>,
    default_enabled: bool,
    mandatory: bool,
) -> ModelReasoningMetadata {
    ModelReasoningMetadata {
        supported_efforts: efforts.to_vec(),
        default_effort,
        default_enabled: Some(default_enabled),
        mandatory: Some(mandatory),
    }
}

fn is_local_provider(provider: &Provider) -> bool {
    let base = provider.base_url.to_ascii_lowercase();
    provider.name.to_ascii_lowercase().contains("ollama")
        || provider.name.to_ascii_lowercase().contains("lm studio")
        || provider.name.to_ascii_lowercase().contains("lmstudio")
        || base.contains("localhost:")
        || base.contains("127.0.0.1:")
}

fn is_ollama_provider(provider: &Provider) -> bool {
    let base = provider.base_url.to_ascii_lowercase();
    provider.name.to_ascii_lowercase().contains("ollama") || base.contains(":11434/")
}

fn is_lm_studio_provider(provider: &Provider) -> bool {
    let name = provider.name.to_ascii_lowercase();
    let base = provider.base_url.to_ascii_lowercase();
    name.contains("lm studio") || name.contains("lmstudio") || base.contains(":1234/")
}

fn is_vllm_provider(provider: &Provider) -> bool {
    provider.name.to_ascii_lowercase().contains("vllm")
}

fn is_gpt_oss_model(model: &str) -> bool {
    model.contains("gpt-oss")
}

fn looks_local_thinking_model(model: &str) -> bool {
    model.contains("qwen3")
        || model.contains("deepseek-r")
        || model.contains("deepseek-v3.1")
        || model.contains("reason")
}

fn looks_reasoning_model(model: &str) -> bool {
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

/// Routes generation by model: a provider that serves the model, else local.
pub struct ProviderRouter {
    inner: Arc<RwLock<Vec<Runtime>>>,
    local: SharedService,
    privacy: Arc<PrivacyGate>,
}

const PROVIDER_MODEL_PREFIX: &str = "provider:";

pub(crate) fn provider_model_id(provider_id: &str, model_id: &str) -> String {
    format!("{PROVIDER_MODEL_PREFIX}{provider_id}:{model_id}")
}

pub(crate) fn provider_model_route(model: &str) -> Option<(String, String)> {
    let rest = model.trim().strip_prefix(PROVIDER_MODEL_PREFIX)?;
    let (provider_id, model_id) = rest.split_once(':')?;
    if provider_id.is_empty() || model_id.is_empty() {
        return None;
    }
    Some((provider_id.to_string(), model_id.to_string()))
}

#[async_trait]
impl ModelService for ProviderRouter {
    fn name(&self) -> &str {
        "router"
    }

    async fn list_models(&self) -> Result<Vec<Model>> {
        let mut out = self.local.list_models().await.unwrap_or_default();
        for r in self.inner.read().await.iter() {
            if !r.cfg.enabled || !r.cfg.kind.is_chat() {
                continue;
            }
            for m in &r.cfg.models {
                let context = r
                    .cfg
                    .model_context
                    .get(m)
                    .cloned()
                    .unwrap_or_else(|| fallback_model_context(&r.cfg, m));
                out.push(Model {
                    id: m.clone(),
                    object: "model".to_string(),
                    created: 0,
                    owned_by: r.cfg.name.clone(),
                    provider_id: Some(r.cfg.id.clone()),
                    context_length: context.context_length,
                    max_prompt_tokens: context.max_prompt_tokens,
                    max_completion_tokens: context.max_completion_tokens,
                    pricing: None,
                    reasoning: r
                        .cfg
                        .model_reasoning
                        .get(m)
                        .cloned()
                        .or_else(|| fallback_model_reasoning(&r.cfg, m)),
                    capabilities: r.cfg.model_capabilities.get(m).cloned(),
                    architecture: None,
                });
            }
        }
        Ok(out)
    }

    async fn stream(&self, mut req: CompletionRequest) -> Result<EventStream> {
        let routed = provider_model_route(&req.model);
        let provider_qualified = routed.is_some();
        let backend = {
            let guard = self.inner.read().await;
            guard
                .iter()
                .find(|r| {
                    r.cfg.enabled
                        && r.cfg.kind.is_chat()
                        && match &routed {
                            Some((provider_id, model_id)) => {
                                r.cfg.id == *provider_id && r.cfg.models.contains(model_id)
                            }
                            None => r.cfg.models.contains(&req.model),
                        }
                })
                .map(|r| r.backend.clone())
        };
        if let Some((_, model_id)) = routed {
            req.model = model_id;
        }
        let Some(remote) = backend else {
            if provider_qualified {
                return Err(Error::InvalidRequest(
                    "selected provider model is not available".to_string(),
                ));
            }
            return privacy::scoped_service(self.local.clone(), self.privacy.mode())
                .stream(req)
                .await;
        };
        // Remote: enforce the outbound privacy gate before sending.
        match self.privacy.mode() {
            PrivacyMode::Off => remote.stream(req).await,
            PrivacyMode::Block => {
                if privacy::request_has_image_parts(&req) {
                    return Err(Error::InvalidRequest(
                        "blocked by the privacy gate: outbound message contains image data, which the privacy gate cannot scan. Switch the gate to Off to send images to a remote provider.".to_string(),
                    ));
                }
                let dets = self.privacy.scan_request(&req);
                if dets.is_empty() {
                    remote.stream(req).await
                } else {
                    Err(Error::InvalidRequest(format!(
                        "blocked by the privacy gate: outbound message contains {} ({} item(s)). Switch the gate to Redact or Off to send this to a remote provider.",
                        privacy::kinds_summary(&dets),
                        dets.len()
                    )))
                }
            }
            PrivacyMode::Redact => {
                if privacy::request_has_image_parts(&req) {
                    return Err(Error::InvalidRequest(
                        "blocked by the privacy gate: outbound message contains image data, which the privacy gate cannot redact. Switch the gate to Off to send images to a remote provider.".to_string(),
                    ));
                }
                let map = self.privacy.redact_request(&mut req);
                let inner = remote.stream(req).await?;
                Ok(if map.is_empty() {
                    inner
                } else {
                    privacy::unredact_stream(inner, map)
                })
            }
        }
    }

    async fn ollama_keep_alive(
        &self,
        model: &str,
        keep_alive: Option<serde_json::Value>,
    ) -> Result<bool> {
        let routed = provider_model_route(model);
        let provider_qualified = routed.is_some();
        let model_id = routed
            .as_ref()
            .map(|(_, model_id)| model_id.as_str())
            .unwrap_or(model);
        let backend = {
            let guard = self.inner.read().await;
            guard
                .iter()
                .find(|r| {
                    r.cfg.enabled
                        && r.cfg.kind.is_chat()
                        && match &routed {
                            Some((provider_id, _)) => {
                                r.cfg.id == *provider_id
                                    && r.cfg.models.iter().any(|m| m == model_id)
                            }
                            None => r.cfg.models.iter().any(|m| m == model_id),
                        }
                })
                .map(|r| r.backend.clone())
        };
        match backend {
            Some(remote) => remote.ollama_keep_alive(model_id, keep_alive).await,
            None if provider_qualified => Err(Error::InvalidRequest(
                "selected provider model is not available".to_string(),
            )),
            None => self.local.ollama_keep_alive(model_id, keep_alive).await,
        }
    }

    async fn embed(&self, model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let routed = provider_model_route(model);
        let provider_qualified = routed.is_some();
        let model_id = routed
            .as_ref()
            .map(|(_, model_id)| model_id.as_str())
            .unwrap_or(model);
        let backend = {
            let guard = self.inner.read().await;
            guard
                .iter()
                .find(|r| {
                    r.cfg.enabled
                        && r.cfg.kind.is_chat()
                        && match &routed {
                            Some((provider_id, _)) => {
                                r.cfg.id == *provider_id
                                    && r.cfg.models.iter().any(|m| m == model_id)
                            }
                            None => r.cfg.models.iter().any(|m| m == model_id),
                        }
                })
                .map(|r| r.backend.clone())
        };
        let Some(remote) = backend else {
            if provider_qualified {
                return Err(Error::InvalidRequest(
                    "selected provider model is not available".to_string(),
                ));
            }
            return privacy::scoped_service(self.local.clone(), self.privacy.mode())
                .embed(model_id, inputs)
                .await;
        };
        match self.privacy.mode() {
            PrivacyMode::Off => remote.embed(model_id, inputs).await,
            PrivacyMode::Block => {
                let detections = inputs
                    .iter()
                    .flat_map(|input| self.privacy.scan_text(input))
                    .collect::<Vec<_>>();
                if detections.is_empty() {
                    remote.embed(model_id, inputs).await
                } else {
                    Err(Error::InvalidRequest(format!(
                        "blocked by the privacy gate: embedding input contains {} ({} item(s)). Switch the gate to Redact or Off to send this to a remote provider.",
                        privacy::kinds_summary(&detections),
                        detections.len()
                    )))
                }
            }
            PrivacyMode::Redact => {
                let inputs = inputs
                    .into_iter()
                    .map(|input| self.privacy.redact_text(&input).text)
                    .collect();
                remote.embed(model_id, inputs).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::api::openai::{ChatMessage, Content, ContentPart, ImageUrl};
    use std::sync::Mutex;

    #[derive(Clone)]
    struct RecordingBackend {
        inputs: Arc<Mutex<Vec<Vec<String>>>>,
        models: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ModelService for RecordingBackend {
        fn name(&self) -> &str {
            "recording"
        }

        fn requires_privacy_gate(&self) -> bool {
            true
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            Ok(vec![Model::local("text-embedding-3-small", 0)])
        }

        async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
            self.models.lock().unwrap().push(req.model);
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn embed(&self, _model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            self.inputs.lock().unwrap().push(inputs.clone());
            Ok(inputs
                .iter()
                .map(|input| vec![input.len() as f32])
                .collect())
        }
    }

    fn provider(name: &str, kind: ProviderKind, base_url: &str) -> Provider {
        Provider {
            id: name.to_string(),
            name: name.to_string(),
            kind,
            base_url: base_url.to_string(),
            api_key: None,
            enabled: true,
            models: Vec::new(),
            pricing: BTreeMap::new(),
            model_context: BTreeMap::new(),
            model_reasoning: BTreeMap::new(),
            model_capabilities: BTreeMap::new(),
            model_overrides: BTreeMap::new(),
            last_error: None,
        }
    }

    #[tokio::test]
    async fn refresh_all_reports_enabled_chat_provider_work() {
        let root = std::env::temp_dir().join(format!(
            "milim-provider-refresh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut cfg = provider(
            "unreachable",
            ProviderKind::OpenAiCompatible,
            "not a valid url",
        );
        cfg.enabled = false;
        let registry = ProviderRegistry {
            inner: Arc::new(RwLock::new(vec![Runtime {
                backend: backend_for(&cfg),
                cfg,
            }])),
            local: Arc::new(milim_inference::unavailable::UnavailableBackend::new()),
            store: ProviderStore::open(&root).unwrap(),
        };

        assert!(!registry.refresh_all().await.unwrap());
        registry.inner.write().await[0].cfg.enabled = true;
        assert!(registry.refresh_all().await.unwrap());
        assert!(registry.list().await[0].last_error.is_some());

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn refresh_all_keeps_cached_models_when_refresh_fails() {
        let root = std::env::temp_dir().join(format!(
            "milim-provider-refresh-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut cfg = provider(
            "unreachable",
            ProviderKind::OpenAiCompatible,
            "not a valid url",
        );
        cfg.models = vec!["cached-model".to_string()];
        let registry = ProviderRegistry {
            inner: Arc::new(RwLock::new(vec![Runtime {
                backend: backend_for(&cfg),
                cfg,
            }])),
            local: Arc::new(milim_inference::unavailable::UnavailableBackend::new()),
            store: ProviderStore::open(&root).unwrap(),
        };

        assert!(registry.refresh_all().await.unwrap());
        let refreshed = registry.list().await;
        assert_eq!(refreshed[0].models, ["cached-model"]);
        assert!(refreshed[0].last_error.is_some());

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn fallback_reasoning_metadata_for_known_direct_models() {
        let openai = provider(
            "OpenAI",
            ProviderKind::OpenAiCompatible,
            "https://api.openai.com/v1",
        );
        let meta = fallback_model_reasoning(&openai, "gpt-5").unwrap();
        assert!(meta.supported_efforts.contains(&ReasoningEffort::High));

        let anthropic = provider(
            "Anthropic",
            ProviderKind::Anthropic,
            "https://api.anthropic.com/v1",
        );
        // Claude models follow the adapter's capability table: thinking that
        // cannot be turned off is mandatory, manual thinking starts off.
        let meta = fallback_model_reasoning(&anthropic, "claude-opus-5-5").unwrap();
        assert_eq!(meta.mandatory, Some(true));
        assert!(meta.supported_efforts.contains(&ReasoningEffort::Xhigh));
        let meta = fallback_model_reasoning(&anthropic, "claude-sonnet-5").unwrap();
        assert_eq!(meta.mandatory, Some(false));
        assert!(meta.supported_efforts.contains(&ReasoningEffort::None));
        let meta = fallback_model_reasoning(&anthropic, "claude-sonnet-4-6").unwrap();
        assert!(!meta.supported_efforts.contains(&ReasoningEffort::Xhigh));
        let meta = fallback_model_reasoning(&anthropic, "claude-sonnet-4-20250514").unwrap();
        assert_eq!(meta.mandatory, Some(false));
        assert_eq!(meta.default_enabled, Some(false));
        assert!(fallback_model_reasoning(&anthropic, "claude-3-5-sonnet-20241022").is_none());

        let gemini = provider(
            "Gemini",
            ProviderKind::Gemini,
            "https://generativelanguage.googleapis.com/v1beta",
        );
        let meta = fallback_model_reasoning(&gemini, "gemini-2.5-flash").unwrap();
        assert!(meta.supported_efforts.contains(&ReasoningEffort::None));
    }

    #[test]
    fn local_provider_reasoning_metadata_is_runtime_specific() {
        let ollama = provider(
            "Ollama",
            ProviderKind::OpenAiCompatible,
            "http://localhost:11434/v1",
        );
        let meta = fallback_model_reasoning(&ollama, "deepseek-r1").unwrap();
        assert!(meta.supported_efforts.contains(&ReasoningEffort::Max));
        assert_eq!(meta.mandatory, Some(false));

        let meta = fallback_model_reasoning(&ollama, "gpt-oss:20b").unwrap();
        assert_eq!(
            meta.supported_efforts,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High
            ]
        );
        assert_eq!(meta.mandatory, Some(true));

        let lm_studio = provider(
            "LM Studio",
            ProviderKind::OpenAiCompatible,
            "http://localhost:1234/v1",
        );
        assert!(fallback_model_reasoning(&lm_studio, "openai/gpt-oss-20b").is_some());
        assert!(fallback_model_reasoning(&lm_studio, "deepseek-r1").is_none());

        let custom = provider(
            "custom",
            ProviderKind::OpenAiCompatible,
            "http://localhost:9999/v1",
        );
        assert!(fallback_model_reasoning(&custom, "deepseek-r1").is_none());
    }

    #[test]
    fn explicit_model_overrides_replace_discovery_and_survive_refresh_layers() {
        let mut vllm = provider(
            "vLLM (local)",
            ProviderKind::OpenAiCompatible,
            "http://localhost:8000/v1",
        );
        vllm.model_capabilities.insert(
            "qwen".into(),
            ModelCapabilities {
                image_input: Some(false),
                tool_use: Some(false),
                ..Default::default()
            },
        );
        vllm.model_overrides.insert(
            "qwen".into(),
            ModelCapabilityOverride {
                image_input: Some(true),
                tool_use: Some(true),
                reasoning: Some(true),
                supported_efforts: vec![ReasoningEffort::Low, ReasoningEffort::High],
                context_window: None,
            },
        );

        apply_model_overrides(&mut vllm);

        assert_eq!(vllm.model_capabilities["qwen"].image_input, Some(true));
        assert_eq!(vllm.model_capabilities["qwen"].tool_use, Some(true));
        assert_eq!(
            vllm.model_reasoning["qwen"].supported_efforts,
            [ReasoningEffort::Low, ReasoningEffort::High]
        );
    }

    #[test]
    fn context_fallback_follows_documented_model_families() {
        let openai = provider(
            "OpenAI",
            ProviderKind::OpenAiCompatible,
            "https://api.openai.com/v1",
        );
        let window = |provider: &Provider, model: &str| model_context_window(provider, model);
        assert_eq!(window(&openai, "gpt-5"), Some(272_000));
        assert_eq!(window(&openai, "gpt-5.2-codex"), Some(272_000));
        assert_eq!(window(&openai, "gpt-5.5"), Some(1_050_000));
        assert_eq!(window(&openai, "gpt-5.4-mini"), Some(272_000));
        assert_eq!(window(&openai, "gpt-6-sol"), Some(922_000));
        assert_eq!(window(&openai, "gpt-5-chat-latest"), Some(128_000));
        assert_eq!(window(&openai, "gpt-4.1-mini"), Some(1_047_576));
        assert_eq!(window(&openai, "gpt-4o"), Some(128_000));
        assert_eq!(window(&openai, "gpt-4"), Some(8_192));
        assert_eq!(window(&openai, "o3"), Some(200_000));
        assert_eq!(window(&openai, "o1-mini"), Some(128_000));
        assert_eq!(window(&openai, "gpt-3.5-turbo"), Some(16_385));
        assert_eq!(window(&openai, "text-embedding-3-small"), Some(32_768));
        assert_eq!(window(&openai, "mystery-model"), Some(32_768));

        let openrouter = provider(
            "OpenRouter",
            ProviderKind::OpenAiCompatible,
            "https://openrouter.ai/api/v1",
        );
        assert_eq!(window(&openrouter, "openai/gpt-oss-120b"), Some(131_072));
        assert_eq!(
            window(&openrouter, "google/gemini-2.5-pro"),
            Some(1_048_576)
        );
        assert_eq!(
            window(&openrouter, "anthropic/claude-sonnet-4.5"),
            Some(200_000)
        );

        // Local servers run whatever context they were started with.
        let ollama = provider(
            "Ollama",
            ProviderKind::OpenAiCompatible,
            "http://localhost:11434/v1",
        );
        assert_eq!(window(&ollama, "gpt-oss:120b"), Some(32_768));
        assert_eq!(window(&ollama, "gpt-5"), Some(32_768));

        let gemini = provider(
            "Gemini",
            ProviderKind::Gemini,
            "https://generativelanguage.googleapis.com/v1beta",
        );
        assert_eq!(window(&gemini, "gemini-3.1-pro-preview"), Some(1_048_576));
        assert_eq!(window(&gemini, "gemini-2.0-flash"), Some(1_048_576));
        assert_eq!(window(&gemini, "unknown-gemini-api-model"), Some(32_768));

        let anthropic = provider(
            "Anthropic",
            ProviderKind::Anthropic,
            "https://api.anthropic.com/v1",
        );
        assert_eq!(window(&anthropic, "claude-opus-5"), Some(1_000_000));
        assert_eq!(window(&anthropic, "claude-haiku-4-5"), Some(200_000));
        assert_eq!(window(&anthropic, "compatible-model"), Some(200_000));
        assert_eq!(
            fallback_model_context(&anthropic, "claude-opus-4-1").max_completion_tokens,
            Some(32_000)
        );
    }

    #[test]
    fn context_window_override_replaces_reported_and_built_in_limits() {
        let mut lm_studio = provider(
            "LM Studio",
            ProviderKind::OpenAiCompatible,
            "http://localhost:1234/v1",
        );
        lm_studio.model_context.insert(
            "qwen3".into(),
            ModelContextMetadata {
                context_length: Some(262_144),
                max_prompt_tokens: Some(200_000),
                max_completion_tokens: Some(8_192),
            },
        );
        for model in ["qwen3", "gpt-oss-20b"] {
            lm_studio.model_overrides.insert(
                model.into(),
                ModelCapabilityOverride {
                    context_window: Some(16_384),
                    ..Default::default()
                },
            );
        }

        apply_model_overrides(&mut lm_studio);

        assert_eq!(model_context_window(&lm_studio, "qwen3"), Some(16_384));
        assert_eq!(
            lm_studio.model_context["qwen3"].max_completion_tokens,
            Some(8_192)
        );
        assert_eq!(
            model_context_window(&lm_studio, "gpt-oss-20b"),
            Some(16_384)
        );
        assert_eq!(model_context_window(&lm_studio, "other"), Some(32_768));
    }

    /// Answers capability probes: a tool call when tools are offered,
    /// reasoning continuation data (with empty reasoning text) when an
    /// effort is set, plain text otherwise.
    #[derive(Clone, Default)]
    struct ProbeBackend {
        requests: Arc<Mutex<Vec<CompletionRequest>>>,
    }

    #[async_trait]
    impl ModelService for ProbeBackend {
        fn name(&self) -> &str {
            "probe"
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            Ok(Vec::new())
        }

        async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
            let delta = if !req.tools.is_empty() {
                milim_inference::DeltaEvent {
                    tool_calls: vec![milim_core::api::openai::DeltaToolCall {
                        index: 0,
                        id: Some("call_1".into()),
                        kind: Some("function".into()),
                        function: milim_core::api::openai::DeltaFunction {
                            name: Some("milim_capability_probe".into()),
                            arguments: Some("{}".into()),
                        },
                    }],
                    ..Default::default()
                }
            } else if req.reasoning_effort.is_some() {
                milim_inference::DeltaEvent {
                    reasoning: Some(String::new()),
                    provider_state: Some(serde_json::json!({ "anthropic": { "blocks": [] } })),
                    ..Default::default()
                }
            } else {
                milim_inference::DeltaEvent::text("red")
            };
            self.requests.lock().unwrap().push(req);
            Ok(Box::pin(futures::stream::iter([
                Ok(StreamEvent::Delta(delta)),
                Ok(StreamEvent::Done {
                    finish_reason: "stop".into(),
                    usage: Default::default(),
                }),
            ])))
        }

        async fn embed(&self, _model: &str, _inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn capability_probes_use_accepted_request_shapes_and_reasoning_deltas() {
        let root = std::env::temp_dir().join(format!(
            "milim-provider-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let backend = ProbeBackend::default();
        let mut cfg = provider(
            "anthropic",
            ProviderKind::Anthropic,
            "https://api.anthropic.com/v1",
        );
        cfg.models = vec!["claude-opus-5-5".into()];
        let registry = ProviderRegistry {
            inner: Arc::new(RwLock::new(vec![Runtime {
                backend: Arc::new(backend.clone()),
                cfg,
            }])),
            local: Arc::new(milim_inference::unavailable::UnavailableBackend::new()),
            store: ProviderStore::open(&root).unwrap(),
        };

        let verified = registry
            .verify_model_capabilities("anthropic", "claude-opus-5-5")
            .await
            .unwrap();
        assert!(verified.vision.supported);
        assert!(verified.reasoning.supported, "{:?}", verified.reasoning);
        assert!(verified.tools.supported);

        let requests = backend.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 3);
        for request in &requests {
            assert_eq!(request.sampling.temperature, None);
            assert_eq!(request.tool_choice, None);
            assert!(request.sampling.max_tokens.unwrap() >= 2_048);
        }

        // Empty reasoning text alone does not count as reasoning.
        let silent = probe_completion(
            &ScriptedDeltas(vec![milim_inference::DeltaEvent {
                reasoning: Some(" ".into()),
                ..Default::default()
            }]),
            base_probe_request(),
            PROBE_TIMEOUT,
            |output| output.reasoning,
        )
        .await;
        assert!(!silent.supported);

        std::fs::remove_dir_all(root).ok();
    }

    struct ScriptedDeltas(Vec<milim_inference::DeltaEvent>);

    #[async_trait]
    impl ModelService for ScriptedDeltas {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            Ok(Vec::new())
        }

        async fn stream(&self, _req: CompletionRequest) -> Result<EventStream> {
            let events: Vec<Result<StreamEvent>> = self
                .0
                .iter()
                .cloned()
                .map(|delta| Ok(StreamEvent::Delta(delta)))
                .collect();
            Ok(Box::pin(futures::stream::iter(events)))
        }

        async fn embed(&self, _model: &str, _inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(Vec::new())
        }
    }

    fn base_probe_request() -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text("user", "hi")],
            tools: Vec::new(),
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: Default::default(),
            reasoning_effort: None,
        }
    }

    #[test]
    fn vllm_reasoning_fallback_is_not_suppressed_as_generic_loopback() {
        let vllm = provider(
            "vLLM (local)",
            ProviderKind::OpenAiCompatible,
            "http://localhost:8000/v1",
        );
        assert!(fallback_model_reasoning(&vllm, "qwen3.8-27b").is_some());
    }

    #[tokio::test]
    async fn router_routes_embeddings_to_provider_and_redacts_inputs() {
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let models = Arc::new(Mutex::new(Vec::new()));
        let mut cfg = provider(
            "OpenAI",
            ProviderKind::OpenAiCompatible,
            "https://api.openai.com/v1",
        );
        cfg.models = vec!["text-embedding-3-small".to_string()];
        let privacy = Arc::new(PrivacyGate::default());
        privacy.set(PrivacyMode::Redact);
        let router = ProviderRouter {
            inner: Arc::new(RwLock::new(vec![Runtime {
                cfg,
                backend: Arc::new(RecordingBackend {
                    inputs: inputs.clone(),
                    models,
                }),
            }])),
            local: Arc::new(milim_inference::unavailable::UnavailableBackend::new()),
            privacy,
        };

        let vectors = router
            .embed(
                "text-embedding-3-small",
                vec!["email person@example.com".to_string()],
            )
            .await
            .unwrap();

        assert_eq!(vectors.len(), 1);
        let inputs = inputs.lock().unwrap();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0][0].contains("[EMAIL_1]"));
        assert!(!inputs[0][0].contains("person@example.com"));
    }

    #[tokio::test]
    async fn router_applies_privacy_to_remote_fallback() {
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let models = Arc::new(Mutex::new(Vec::new()));
        let privacy = Arc::new(PrivacyGate::default());
        privacy.set(PrivacyMode::Redact);
        let router = ProviderRouter {
            inner: Arc::new(RwLock::new(Vec::new())),
            local: Arc::new(RecordingBackend {
                inputs: inputs.clone(),
                models: models.clone(),
            }),
            privacy: privacy.clone(),
        };

        router
            .embed(
                "fallback-model",
                vec!["email person@example.com".to_string()],
            )
            .await
            .unwrap();
        {
            let recorded = inputs.lock().unwrap();
            assert_eq!(recorded.len(), 1);
            assert_eq!(recorded[0], ["email [EMAIL_1]"]);
        }

        privacy.set(PrivacyMode::Block);
        let error = router
            .stream(CompletionRequest {
                model: "fallback-model".to_string(),
                messages: vec![ChatMessage::text("user", "email person@example.com")],
                tools: Vec::new(),
                tool_choice: None,
                response_format: None,
                prompt: None,
                suffix: None,
                sampling: Default::default(),
                reasoning_effort: None,
            })
            .await
            .err()
            .expect("remote fallback should be blocked");
        assert!(error.to_string().contains("blocked by the privacy gate"));
        assert!(models.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn request_scoped_routers_keep_privacy_modes_independent() {
        let root = std::env::temp_dir().join(format!(
            "milim-provider-privacy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let mut cfg = provider(
            "OpenAI",
            ProviderKind::OpenAiCompatible,
            "https://api.openai.com/v1",
        );
        cfg.models = vec!["text-embedding-3-small".to_string()];
        let registry = ProviderRegistry {
            inner: Arc::new(RwLock::new(vec![Runtime {
                cfg,
                backend: Arc::new(RecordingBackend {
                    inputs: inputs.clone(),
                    models: Arc::new(Mutex::new(Vec::new())),
                }),
            }])),
            local: Arc::new(milim_inference::unavailable::UnavailableBackend::new()),
            store: ProviderStore::open(&root).unwrap(),
        };
        let off = registry.router_with_privacy(PrivacyMode::Off);
        let redact = registry.router_with_privacy(PrivacyMode::Redact);

        for index in 0..100 {
            let (off_result, redact_result) = tokio::join!(
                off.embed(
                    "text-embedding-3-small",
                    vec![format!("off-{index}@example.com")]
                ),
                redact.embed(
                    "text-embedding-3-small",
                    vec![format!("redact-{index}@example.com")]
                )
            );
            off_result.unwrap();
            redact_result.unwrap();
        }

        let inputs = inputs.lock().unwrap();
        assert_eq!(inputs.len(), 200);
        assert_eq!(
            inputs
                .iter()
                .flatten()
                .filter(|input| input.as_str() == "[EMAIL_1]")
                .count(),
            100
        );
        assert_eq!(
            inputs
                .iter()
                .flatten()
                .filter(|input| input.starts_with("off-"))
                .count(),
            100
        );
        assert!(!inputs
            .iter()
            .flatten()
            .any(|input| input.starts_with("redact-")));
        for index in 0..100 {
            assert!(inputs
                .iter()
                .flatten()
                .any(|input| input == &format!("off-{index}@example.com")));
        }
        drop(inputs);
        std::fs::remove_dir_all(root).ok();
    }

    fn image_completion_request(model: &str) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Some(Content::Parts(vec![
                    ContentPart::Text {
                        text: "what is in this image?".to_string(),
                    },
                    ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url: "data:image/png;base64,AAAA".to_string(),
                            detail: None,
                        },
                    },
                ])),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
                provider_state: None,
            }],
            tools: Vec::new(),
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: Default::default(),
            reasoning_effort: None,
        }
    }

    #[tokio::test]
    async fn router_blocks_remote_image_parts_when_privacy_gate_scans() {
        let models = Arc::new(Mutex::new(Vec::new()));
        let mut cfg = provider(
            "OpenAI",
            ProviderKind::OpenAiCompatible,
            "https://api.openai.com/v1",
        );
        cfg.models = vec!["vision-model".to_string()];
        let privacy = Arc::new(PrivacyGate::default());
        let router = ProviderRouter {
            inner: Arc::new(RwLock::new(vec![Runtime {
                cfg,
                backend: Arc::new(RecordingBackend {
                    inputs: Arc::new(Mutex::new(Vec::new())),
                    models: models.clone(),
                }),
            }])),
            local: Arc::new(milim_inference::unavailable::UnavailableBackend::new()),
            privacy: privacy.clone(),
        };

        for mode in [PrivacyMode::Redact, PrivacyMode::Block] {
            privacy.set(mode);
            let result = router
                .stream(image_completion_request("vision-model"))
                .await;
            let err = match result {
                Ok(_) => panic!("image parts should be blocked when privacy scans"),
                Err(err) => err,
            };
            assert!(err.to_string().contains("image data"));
        }

        assert!(models.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn router_uses_provider_qualified_model_id_for_duplicates() {
        let first_models = Arc::new(Mutex::new(Vec::new()));
        let second_models = Arc::new(Mutex::new(Vec::new()));
        let mut first = provider(
            "prov-first",
            ProviderKind::OpenAiCompatible,
            "https://first.test/v1",
        );
        let mut second = provider(
            "prov-second",
            ProviderKind::OpenAiCompatible,
            "https://second.test/v1",
        );
        first.models = vec!["same-model".to_string()];
        second.models = vec!["same-model".to_string()];
        let router = ProviderRouter {
            inner: Arc::new(RwLock::new(vec![
                Runtime {
                    cfg: first,
                    backend: Arc::new(RecordingBackend {
                        inputs: Arc::new(Mutex::new(Vec::new())),
                        models: first_models.clone(),
                    }),
                },
                Runtime {
                    cfg: second,
                    backend: Arc::new(RecordingBackend {
                        inputs: Arc::new(Mutex::new(Vec::new())),
                        models: second_models.clone(),
                    }),
                },
            ])),
            local: Arc::new(milim_inference::unavailable::UnavailableBackend::new()),
            privacy: Arc::new(PrivacyGate::default()),
        };

        let _stream = router
            .stream(CompletionRequest {
                model: "provider:prov-second:same-model".to_string(),
                messages: Vec::new(),
                tools: Vec::new(),
                tool_choice: None,
                response_format: None,
                prompt: None,
                suffix: None,
                sampling: Default::default(),
                reasoning_effort: None,
            })
            .await
            .unwrap();

        assert!(first_models.lock().unwrap().is_empty());
        assert_eq!(
            *second_models.lock().unwrap(),
            vec!["same-model".to_string()]
        );
    }
}
