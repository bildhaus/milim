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
    /// Time excluded from the run time limit: waiting for a person to
    /// decide tool approvals.
    paused: Duration,
    paused_since: Option<Instant>,
    limits: AgentRunLimits,
    cost: f64,
    cost_unknown: bool,
}

impl RunBudget {
    pub(crate) fn new(limits: AgentRunLimits) -> Self {
        Self {
            started: Instant::now(),
            paused: Duration::ZERO,
            paused_since: None,
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
                if usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
                    return None;
                }
                self.limits.pricing.as_ref()?.estimate_cost_usd(&usage)
            });
        if let Some(cost) = cost {
            self.cost += cost;
        } else {
            self.cost_unknown = true;
        }
    }

    pub(crate) fn reason(&self) -> Option<String> {
        self.reason_at(self.elapsed())
    }

    /// Time left before the run time limit, when one is set.
    pub(crate) fn remaining_time(&self) -> Option<Duration> {
        self.limits
            .max_duration
            .map(|limit| limit.saturating_sub(self.elapsed()))
    }

    /// Stop the run clock until [`Self::resume_clock`].
    pub(crate) fn pause_clock(&mut self) {
        self.paused_since.get_or_insert_with(Instant::now);
    }

    pub(crate) fn resume_clock(&mut self) {
        if let Some(since) = self.paused_since.take() {
            self.paused += since.elapsed();
        }
    }

    /// Run time counted against the limit.
    fn elapsed(&self) -> Duration {
        let paused = self.paused
            + self
                .paused_since
                .map_or(Duration::ZERO, |since| since.elapsed());
        self.started.elapsed().saturating_sub(paused)
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
    fn spend_budget_prices_cached_input_at_the_cache_rate() {
        let mut budget = RunBudget::new(AgentRunLimits {
            max_cost_usd: Some(1.0),
            pricing: Some(ModelPricing {
                prompt: Some("0.001".into()),
                completion: Some("0.002".into()),
                input_cache_read: Some("0.0001".into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        budget.record(Usage {
            cache_read_tokens: Some(900),
            ..Usage::new(1_000, 10)
        });
        // 100 uncached + 900 cached input tokens, 10 output tokens.
        assert!((budget.cost - (0.1 + 0.09 + 0.02)).abs() < 1e-9);
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

    #[test]
    fn paused_time_does_not_count_against_the_time_limit() {
        let mut budget = RunBudget::new(AgentRunLimits {
            max_duration: Some(Duration::from_millis(40)),
            ..Default::default()
        });
        budget.pause_clock();
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            budget.reason().is_none(),
            "the clock is stopped while paused"
        );
        budget.resume_clock();
        assert!(budget.reason().is_none());
        assert!(budget.remaining_time().unwrap() > Duration::from_millis(20));
        std::thread::sleep(Duration::from_millis(50));
        assert!(budget.reason().unwrap().contains("time limit"));
    }
}
