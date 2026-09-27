//! Usage, cost, and context-window metrics attached to completed turns.

use milim_core::api::openai::{ModelPricing, Usage};
use milim_core::Result;
use milim_storage::UserDataStore;
use serde_json::{json, Map, Value};

use super::now_ms;
use crate::AppState;

pub(super) async fn provider_context_window(state: &AppState, model: &str) -> Option<u32> {
    let providers = state.providers.as_ref()?.list().await;
    if let Some((id, raw_model)) = crate::providers::provider_model_route(model) {
        let provider = providers.iter().find(|provider| provider.id == id)?;
        crate::providers::model_context_window(provider, &raw_model)
    } else {
        let provider = providers
            .iter()
            .find(|provider| provider.models.iter().any(|candidate| candidate == model))?;
        crate::providers::model_context_window(provider, model)
    }
}

pub(super) async fn provider_pricing(state: &AppState, model: &str) -> Option<ModelPricing> {
    let providers = state.providers.as_ref()?.list().await;
    if let Some((id, raw_model)) = crate::providers::provider_model_route(model) {
        providers
            .iter()
            .find(|provider| provider.id == id)?
            .pricing
            .get(&raw_model)
            .cloned()
    } else {
        providers
            .iter()
            .find(|provider| provider.models.iter().any(|candidate| candidate == model))?
            .pricing
            .get(model)
            .cloned()
    }
}

pub(super) async fn response_metrics_value(
    state: &AppState,
    store: &UserDataStore,
    run_id: &str,
    model: &str,
    usage: Option<Usage>,
    reported_cost_usd: Option<f64>,
) -> Result<Value> {
    let ended_at = now_ms();
    let started_at = store
        .control_run(run_id)?
        .map(|run| run.created_at_ms)
        .unwrap_or(ended_at);
    let reported_cost_usd = reported_cost_usd
        .or_else(|| usage.and_then(|usage| usage.cost_usd))
        .filter(|cost| cost.is_finite() && *cost >= 0.0);
    let mut provider_name = None;
    let mut estimated_cost_usd = None;

    if let Some(registry) = state.providers.as_ref() {
        let providers = registry.list().await;
        let routed = crate::providers::provider_model_route(model);
        let provider = routed
            .as_ref()
            .and_then(|(provider_id, _)| {
                providers
                    .iter()
                    .find(|provider| provider.id == *provider_id)
            })
            .or_else(|| {
                providers
                    .iter()
                    .find(|provider| provider.models.iter().any(|candidate| candidate == model))
            });
        if let Some(provider) = provider {
            provider_name = Some(provider.name.clone());
            if reported_cost_usd.is_none() {
                let raw_model = routed
                    .as_ref()
                    .map(|(_, raw_model)| raw_model.as_str())
                    .unwrap_or(model);
                estimated_cost_usd = usage.and_then(|usage| {
                    provider
                        .pricing
                        .get(raw_model)
                        .and_then(|pricing| estimate_usage_cost_usd(pricing, usage))
                });
            }
        }
    }

    let (cost_usd, cost_source) = if let Some(cost) = reported_cost_usd {
        (Some(cost), Some("provider"))
    } else if let Some(cost) = estimated_cost_usd {
        (Some(cost), Some("estimate"))
    } else {
        (None, None)
    };
    let mut metrics = Map::new();
    metrics.insert("startedAt".into(), json!(started_at));
    metrics.insert("endedAt".into(), json!(ended_at));
    metrics.insert(
        "durationMs".into(),
        json!(ended_at.saturating_sub(started_at)),
    );
    metrics.insert("model".into(), json!(model));
    if let Some(provider) = provider_name {
        metrics.insert("provider".into(), json!(provider));
    }
    if let Some(usage) = usage {
        metrics.insert("usage".into(), json!(usage));
    }
    if let Some(cost) = cost_usd {
        metrics.insert("costUsd".into(), json!(cost));
    }
    if let Some(source) = cost_source {
        metrics.insert("costSource".into(), json!(source));
    }
    Ok(Value::Object(metrics))
}

/// Catalog-price estimate for a response the provider did not price, with
/// cached prompt tokens at the provider's cache prices where published.
pub(super) fn estimate_usage_cost_usd(pricing: &ModelPricing, usage: Usage) -> Option<f64> {
    pricing.estimate_cost_usd(&usage)
}
