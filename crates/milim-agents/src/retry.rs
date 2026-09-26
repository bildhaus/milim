//! Provider retry policy for the agent loop: which failures are worth another
//! attempt, and how long to wait before it.

use std::time::Duration;

use milim_core::provider_error::{classify_provider_error, ProviderErrorKind};

/// Retries after the first attempt of one model step.
pub(crate) const MAX_PROVIDER_RETRIES: u32 = 4;
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// A retryable provider failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Retryable {
    /// Short label for the UI, e.g. `rate limited (429)`.
    pub reason: String,
    pub retry_after: Option<Duration>,
}

/// Classify a provider error. Only throttling, overload, server errors,
/// request timeouts/conflicts, and connection failures are retryable; every
/// other 4xx (auth, quota, context length, bad request) is final.
pub(crate) fn retryable(message: &str) -> Option<Retryable> {
    let info = classify_provider_error(message);
    let retry_after = info
        .retry_after_secs
        .map(|seconds| Duration::from_secs(seconds).min(MAX_RETRY_AFTER));
    let with_status = |label: &str| match info.status {
        Some(status) => format!("{label} ({status})"),
        None => label.to_string(),
    };
    match info.kind {
        ProviderErrorKind::Auth
        | ProviderErrorKind::Quota
        | ProviderErrorKind::ContextLength
        | ProviderErrorKind::ModelNotFound => return None,
        ProviderErrorKind::RateLimited => {
            return Some(Retryable {
                reason: with_status("rate limited"),
                retry_after,
            })
        }
        ProviderErrorKind::ProviderUnavailable => {
            let label = if info.status == Some(529)
                || message.to_ascii_lowercase().contains("overloaded")
            {
                "provider overloaded"
            } else if info.status.is_some() {
                "provider error"
            } else {
                "connection error"
            };
            return Some(Retryable {
                reason: with_status(label),
                retry_after,
            });
        }
        ProviderErrorKind::Unknown => {}
    }
    match info.status {
        Some(408) => Some(Retryable {
            reason: with_status("request timeout"),
            retry_after,
        }),
        Some(409) => Some(Retryable {
            reason: with_status("request conflict"),
            retry_after,
        }),
        Some(_) => None,
        None => {
            let text = message.to_ascii_lowercase();
            [
                "connection closed",
                "connection aborted",
                "broken pipe",
                "unexpected eof",
                "error decoding response body",
                "incomplete message",
                "timeout",
            ]
            .iter()
            .any(|needle| text.contains(needle))
            .then(|| Retryable {
                reason: "connection error".into(),
                retry_after: None,
            })
        }
    }
}

/// Wait before retry `attempt` (1-based): exponential backoff from `base`
/// with equal jitter, capped at 30s, or the provider's Retry-After (capped at
/// 60s) when it asked for one.
pub(crate) fn backoff_delay(
    base: Duration,
    attempt: u32,
    retry_after: Option<Duration>,
) -> Duration {
    if let Some(retry_after) = retry_after {
        return retry_after.min(MAX_RETRY_AFTER);
    }
    let exponential = base
        .saturating_mul(1u32 << attempt.saturating_sub(1).min(16))
        .min(MAX_BACKOFF);
    let half = exponential / 2;
    let jitter_ms = u64::try_from(half.as_millis()).unwrap_or(u64::MAX);
    let jitter = if jitter_ms == 0 {
        Duration::ZERO
    } else {
        Duration::from_millis((uuid::Uuid::new_v4().as_u128() as u64) % (jitter_ms + 1))
    };
    half + jitter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_retryable_provider_failures() {
        let rate = retryable(
            "upstream error: x chat/completions -> 429 Too Many Requests: slow down (retry after 12s)",
        )
        .unwrap();
        assert_eq!(rate.reason, "rate limited (429)");
        assert_eq!(rate.retry_after, Some(Duration::from_secs(12)));
        let capped =
            retryable("x chat/completions -> 429 Too Many Requests: busy (retry after 600s)")
                .unwrap();
        assert_eq!(capped.retry_after, Some(Duration::from_secs(60)));
        assert_eq!(
            retryable("x messages -> 529 : overloaded_error")
                .unwrap()
                .reason,
            "provider overloaded (529)"
        );
        assert_eq!(
            retryable("x chat/completions -> 503 Service Unavailable: ")
                .unwrap()
                .reason,
            "provider error (503)"
        );
        assert!(retryable("x chat/completions -> 408 Request Timeout: ").is_some());
        assert!(retryable("x chat/completions -> 409 Conflict: ").is_some());
        assert!(retryable("upstream error: error sending request for url (https://x)").is_some());
        assert!(retryable("error decoding response body: connection closed").is_some());
    }

    #[test]
    fn never_retries_other_client_errors() {
        for message in [
            "x chat/completions -> 400 Bad Request: invalid tool schema",
            "x chat/completions -> 401 Unauthorized: bad key",
            "x chat/completions -> 404 Not Found: model gpt-9",
            "x chat/completions -> 422 Unprocessable Entity: nope",
            "x chat/completions -> 429 Too Many Requests: insufficient_quota",
            "prompt is too long: 210000 tokens > 200000 maximum",
            "something odd happened",
        ] {
            assert_eq!(retryable(message), None, "{message}");
        }
    }

    #[test]
    fn backoff_grows_exponentially_with_bounded_jitter_and_honors_retry_after() {
        let base = Duration::from_millis(1000);
        for attempt in 1..=4 {
            let full = Duration::from_millis(1000 * (1 << (attempt - 1)));
            let delay = backoff_delay(base, attempt, None);
            assert!(delay >= full / 2 && delay <= full, "{attempt}: {delay:?}");
        }
        assert!(backoff_delay(base, 10, None) <= Duration::from_secs(30));
        assert_eq!(backoff_delay(Duration::ZERO, 3, None), Duration::ZERO);
        assert_eq!(
            backoff_delay(base, 1, Some(Duration::from_secs(7))),
            Duration::from_secs(7)
        );
        assert_eq!(
            backoff_delay(base, 1, Some(Duration::from_secs(700))),
            Duration::from_secs(60)
        );
    }
}
