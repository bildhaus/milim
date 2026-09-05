use std::time::{Duration, Instant};

use milim_core::api::openai::{ModelPricing, Usage};

/// Optional bounds checked at safe model/tool boundaries, never by aborting a
/// filesystem mutation or pretending that a provider's bill is known in advance.
#[derive(Clone, Debug, Default)]
pub struct AgentRunLimits {
    pub max_duration: Option<Duration>,
    pub max_cost_usd: Option<f64>,
    pub pricing: Option<ModelPricing>,
}

pub(crate) struct RunBudget {
    started: Instant,
    limits: AgentRunLimits,
    cost: f64,
    cost_unknown: bool,
}

impl RunBudget {
    pub(crate) fn new(limits: AgentRunLimits) -> Self {
        Self {
            started: Instant::now(),
            limits,
            cost: 0.0,
            cost_unknown: false,
        }
    }

    pub(crate) fn record(&mut self, usage: Usage) {
        let cost = usage
            .cost_usd
            .filter(|value| value.is_finite() && *value >= 0.0)
            .or_else(|| {
                self.limits.pricing.as_ref().and_then(|pricing| {
                    if usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
                        return None;
                    }
                    let prompt = pricing.prompt.as_deref()?.parse::<f64>().ok()?;
                    let completion = pricing.completion.as_deref()?.parse::<f64>().ok()?;
                    if !prompt.is_finite()
                        || prompt < 0.0
                        || !completion.is_finite()
                        || completion < 0.0
                    {
                        return None;
                    }
                    Some(
                        f64::from(usage.prompt_tokens) * prompt
                            + f64::from(usage.completion_tokens) * completion,
                    )
                })
            });
        if let Some(cost) = cost {
            self.cost += cost;
        } else {
            self.cost_unknown = true;
        }
    }

    pub(crate) fn reason(&self) -> Option<String> {
        self.reason_at(self.started.elapsed())
    }

    fn reason_at(&self, elapsed: Duration) -> Option<String> {
        if self
            .limits
            .max_duration
            .is_some_and(|limit| elapsed >= limit)
        {
            return Some("Paused at the run time limit.".into());
        }
        if let Some(limit) = self.limits.max_cost_usd {
            if self.cost_unknown {
                return Some("Paused because this model did not report cost and no usable price estimate is available for the spend limit.".into());
            }
            if self.cost >= limit {
                return Some(format!("Paused at the ${limit:.2} run spend threshold (reported or estimated cost: ${:.4}).", self.cost));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_budget_accumulates_reported_cost_and_never_treats_unknown_as_free() {
        let mut budget = RunBudget::new(AgentRunLimits {
            max_cost_usd: Some(0.1),
            ..Default::default()
        });
        budget.record(Usage {
            cost_usd: Some(0.06),
            ..Usage::new(10, 10)
        });
        assert!(budget.reason().is_none());
        budget.record(Usage {
            cost_usd: Some(0.05),
            ..Usage::new(10, 10)
        });
        assert!(budget.reason().unwrap().contains("spend threshold"));
        let mut unknown = RunBudget::new(AgentRunLimits {
            max_cost_usd: Some(1.0),
            ..Default::default()
        });
        unknown.record(Usage::new(10, 10));
        assert!(unknown.reason().unwrap().contains("no usable price"));
    }

    #[test]
    fn time_budget_stops_only_when_the_deadline_is_reached() {
        let budget = RunBudget::new(AgentRunLimits {
            max_duration: Some(Duration::from_secs(10)),
            ..Default::default()
        });
        assert!(budget.reason_at(Duration::from_secs(9)).is_none());
        assert!(budget
            .reason_at(Duration::from_secs(10))
            .unwrap()
            .contains("time limit"));
        assert!(RunBudget::new(AgentRunLimits::default())
            .reason_at(Duration::from_secs(100_000))
            .is_none());
    }
}
