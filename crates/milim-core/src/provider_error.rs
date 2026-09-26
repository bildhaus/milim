//! Structured classification for provider and runtime failures.
//!
//! Upstream failures still travel as text inside [`crate::Error`], so the
//! classification is derived from that text at the boundary where an error
//! enters canonical run state. Messages built by [`upstream_http_error`] carry
//! the HTTP status and any `Retry-After` value in a stable shape; account
//! runtimes and older providers fall back to well-known phrases.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::Error;

/// What kind of provider failure a message describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    /// The key or sign-in was rejected (HTTP 401/403).
    Auth,
    /// The provider throttled the request (HTTP 429).
    RateLimited,
    /// The prompt exceeded the model's context window.
    ContextLength,
    /// The selected model does not exist for this provider or account.
    ModelNotFound,
    /// The provider or the network path to it is down (HTTP 5xx, timeouts).
    ProviderUnavailable,
    /// The account is out of credit, quota, or needs billing attention.
    Quota,
    Unknown,
}

/// Machine-readable classification attached to canonical run errors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderErrorInfo {
    pub kind: ProviderErrorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

/// Build the upstream error for a non-success provider HTTP response.
///
/// The shape is `"{label} {operation} -> {status}: {body}"`, followed by
/// `" (retry after {n}s)"` when the provider sent a usable `Retry-After`.
pub fn upstream_http_error(
    label: &str,
    operation: &str,
    status: impl std::fmt::Display,
    retry_after: Option<&str>,
    body: &str,
) -> Error {
    let mut message = format!("{label} {operation} -> {status}: {}", body.trim());
    if let Some(seconds) = retry_after.and_then(parse_retry_after_header) {
        message.push_str(&format!(" (retry after {seconds}s)"));
    }
    Error::Upstream(message)
}

/// Build the upstream error for a failure the provider reported inside an
/// already-open stream (an SSE `error` event or an error object in a chunk).
///
/// `status` is the HTTP-equivalent code when the provider sent one; otherwise
/// it is derived from `error_type` (e.g. `overloaded_error` -> 529). The shape
/// is `"{label} {operation} stream -> {status} {error_type}: {message}"`, so
/// [`classify_provider_error`] and [`retry_hint`] treat it exactly like the
/// matching HTTP failure.
pub fn upstream_stream_error(
    label: &str,
    operation: &str,
    status: Option<u16>,
    error_type: Option<&str>,
    message: &str,
) -> Error {
    let error_type = error_type.map(str::trim).filter(|kind| !kind.is_empty());
    let status = status
        .filter(|status| (400..600).contains(status))
        .or_else(|| error_type.and_then(stream_error_status));
    let message = message.trim();
    let detail = match (error_type, message.is_empty()) {
        (Some(kind), false) => format!("{kind}: {message}"),
        (Some(kind), true) => kind.to_string(),
        (None, false) => message.to_string(),
        (None, true) => "provider stream error".to_string(),
    };
    Error::Upstream(match status {
        Some(status) => format!("{label} {operation} stream -> {status} {detail}"),
        None => format!("{label} {operation} stream error: {detail}"),
    })
}

/// HTTP-equivalent status for an error type or status name that Anthropic,
/// OpenAI-compatible, or Gemini APIs send inside a stream.
pub fn stream_error_status(error_type: &str) -> Option<u16> {
    let status = match error_type.trim().to_ascii_lowercase().as_str() {
        "invalid_request_error" | "invalid_argument" | "failed_precondition" => 400,
        "authentication_error" | "unauthenticated" | "invalid_api_key" => 401,
        "billing_error" => 402,
        "permission_error" | "permission_denied" => 403,
        "not_found_error" | "not_found" | "model_not_found" => 404,
        "request_too_large" => 413,
        "rate_limit_error"
        | "rate_limit_exceeded"
        | "resource_exhausted"
        | "insufficient_quota"
        | "tokens"
        | "requests" => 429,
        "api_error" | "server_error" | "internal" | "internal_error" | "internal_server_error" => {
            500
        }
        "unavailable" | "service_unavailable" => 503,
        "timeout_error" | "deadline_exceeded" => 504,
        "overloaded_error" | "overloaded" => 529,
        _ => return None,
    };
    Some(status)
}

/// Whether a failed provider call is worth retrying, and how long to wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryHint {
    /// True for throttling and transient provider or network failures.
    pub retryable: bool,
    /// The provider's `Retry-After` (or `retry-after-ms`) hint, when sent.
    pub retry_after: Option<Duration>,
}

/// Retry guidance for an error returned by a model backend, covering both
/// HTTP status failures and errors reported mid-stream. Returns `None` for
/// errors Milim raised itself (validation, missing models) and for provider
/// failures it cannot recognize.
pub fn retry_hint(error: &Error) -> Option<RetryHint> {
    match error {
        Error::Upstream(_) | Error::Inference(_) | Error::Io(_) | Error::Other(_) => {}
        Error::InvalidRequest(_)
        | Error::ModelNotFound(_)
        | Error::NotFound(_)
        | Error::Unauthorized(_)
        | Error::Json(_) => return None,
    }
    let info = classify_provider_error(&error.to_string());
    let retryable = match info.kind {
        ProviderErrorKind::RateLimited | ProviderErrorKind::ProviderUnavailable => true,
        ProviderErrorKind::Auth
        | ProviderErrorKind::ContextLength
        | ProviderErrorKind::ModelNotFound
        | ProviderErrorKind::Quota => false,
        ProviderErrorKind::Unknown => return None,
    };
    Some(RetryHint {
        retryable,
        retry_after: info.retry_after_secs.map(Duration::from_secs),
    })
}

/// Parse a `Retry-After` header given in delta seconds. HTTP-date values are
/// ignored rather than guessed.
pub fn parse_retry_after_header(value: &str) -> Option<u64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map(|seconds| seconds.ceil() as u64)
}

impl ProviderErrorInfo {
    fn new(kind: ProviderErrorKind, status: Option<u16>, retry_after_secs: Option<u64>) -> Self {
        Self {
            kind,
            status,
            retry_after_secs,
        }
    }
}

/// Classify a provider or runtime error message.
pub fn classify_provider_error(message: &str) -> ProviderErrorInfo {
    let text = message.to_ascii_lowercase();
    let status = http_status(&text);
    let retry_after = retry_after_secs(&text);
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    use ProviderErrorKind::*;

    // Quota is checked before 429 because OpenAI reports exhausted credit as
    // `429 insufficient_quota`, which retrying will not fix.
    if status == Some(402)
        || has(&[
            "insufficient_quota",
            "exceeded your current quota",
            "quota exceeded",
            "billing",
            "credit balance",
            "insufficient credit",
            "insufficient balance",
            "payment required",
            "out of credits",
        ])
    {
        return ProviderErrorInfo::new(Quota, status, None);
    }
    if status == Some(413)
        || has(&[
            "context_length_exceeded",
            "context length",
            "context window",
            "maximum context",
            "prompt is too long",
            "too many tokens",
            "reduce the length",
            "input is too long",
            "exceeds the model's maximum",
        ])
    {
        return ProviderErrorInfo::new(ContextLength, status, None);
    }
    if matches!(status, Some(401 | 403))
        || has(&[
            "invalid api key",
            "invalid_api_key",
            "incorrect api key",
            "invalid x-api-key",
            "api key not valid",
            "authentication_error",
            "unauthorized",
            "permission_denied",
        ])
    {
        return ProviderErrorInfo::new(Auth, status, None);
    }
    if has(&[
        "model_not_found",
        "model not found",
        "unknown model",
        "no such model",
        "invalid model",
    ]) || (status == Some(404) && text.contains("model"))
    {
        return ProviderErrorInfo::new(ModelNotFound, status, None);
    }
    if status == Some(429)
        || has(&[
            "rate limit",
            "rate_limit",
            "ratelimit",
            "too many requests",
            "usage limit",
        ])
    {
        return ProviderErrorInfo::new(RateLimited, status, retry_after);
    }
    if status.is_some_and(|status| status >= 500)
        || has(&[
            "overloaded",
            "service unavailable",
            "bad gateway",
            "gateway timeout",
            "timed out",
            "connection refused",
            "connection reset",
            "error sending request",
            "stream interrupted",
            "dns error",
            "failed to lookup address",
        ])
    {
        return ProviderErrorInfo::new(ProviderUnavailable, status, retry_after);
    }
    ProviderErrorInfo::new(Unknown, status, None)
}

/// Canonical run error payload: the stable `code` and raw `message`, plus a
/// `provider_error` classification when the failure is recognizable.
pub fn run_error_value(error: &Error) -> serde_json::Value {
    let message = error.to_string();
    let mut value = serde_json::json!({ "code": error.code(), "message": message });
    let info = classify_provider_error(&message);
    if info.kind != ProviderErrorKind::Unknown {
        value["provider_error"] = serde_json::to_value(info).unwrap_or_default();
    }
    value
}

fn http_status(text: &str) -> Option<u16> {
    for marker in ["-> ", "http ", "status code ", "status: ", "status "] {
        let mut rest = text;
        while let Some(index) = rest.find(marker) {
            rest = &rest[index + marker.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if digits.len() == 3 {
                if let Ok(status) = digits.parse::<u16>() {
                    if (400..600).contains(&status) {
                        return Some(status);
                    }
                }
            }
        }
    }
    None
}

fn retry_after_secs(text: &str) -> Option<u64> {
    for marker in [
        "retry after ",
        "retry-after: ",
        "retry-after ",
        "try again in ",
    ] {
        let Some(index) = text.find(marker) else {
            continue;
        };
        let rest = &text[index + marker.len()..];
        let number: String = rest
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
            .collect();
        let Ok(value) = number.parse::<f64>() else {
            continue;
        };
        if !value.is_finite() || value < 0.0 {
            continue;
        }
        let unit = rest[number.len()..].trim_start();
        let seconds = if unit.starts_with("ms") {
            value / 1000.0
        } else if unit.starts_with("min") || unit.starts_with('m') {
            value * 60.0
        } else {
            value
        };
        return Some(seconds.ceil().max(1.0) as u64);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(message: &str) -> ProviderErrorKind {
        classify_provider_error(message).kind
    }

    #[test]
    fn classifies_http_status_messages() {
        let auth = upstream_http_error(
            "OpenAI",
            "chat/completions",
            "401 Unauthorized",
            None,
            r#"{"error":{"message":"Incorrect API key provided"}}"#,
        );
        assert_eq!(kind(&auth.to_string()), ProviderErrorKind::Auth);
        assert_eq!(classify_provider_error(&auth.to_string()).status, Some(401));
        assert_eq!(
            kind("upstream error: Groq chat/completions -> 403 Forbidden: denied"),
            ProviderErrorKind::Auth
        );
        assert_eq!(
            kind("upstream error: x chat/completions -> 503 Service Unavailable: "),
            ProviderErrorKind::ProviderUnavailable
        );
        assert_eq!(
            kind(
                "upstream error: x chat/completions -> 404 Not Found: model `gpt-9` does not exist"
            ),
            ProviderErrorKind::ModelNotFound
        );
    }

    #[test]
    fn rate_limits_keep_retry_after() {
        let error = upstream_http_error(
            "OpenAI",
            "chat/completions",
            "429 Too Many Requests",
            Some("12"),
            "slow down",
        );
        let info = classify_provider_error(&error.to_string());
        assert_eq!(info.kind, ProviderErrorKind::RateLimited);
        assert_eq!(info.status, Some(429));
        assert_eq!(info.retry_after_secs, Some(12));

        let info = classify_provider_error("Rate limit reached. Please try again in 1.5s.");
        assert_eq!(info.kind, ProviderErrorKind::RateLimited);
        assert_eq!(info.retry_after_secs, Some(2));
        assert_eq!(
            classify_provider_error("rate_limit_exceeded: try again in 250ms").retry_after_secs,
            Some(1)
        );
        assert_eq!(
            parse_retry_after_header("Wed, 21 Oct 2015 07:28:00 GMT"),
            None
        );
        assert_eq!(parse_retry_after_header(" 3 "), Some(3));
    }

    #[test]
    fn quota_and_context_win_over_generic_status() {
        assert_eq!(
            kind("x chat/completions -> 429 Too Many Requests: {\"error\":{\"code\":\"insufficient_quota\"}}"),
            ProviderErrorKind::Quota
        );
        assert_eq!(
            kind("Your credit balance is too low to access the Anthropic API."),
            ProviderErrorKind::Quota
        );
        assert_eq!(
            kind("x chat/completions -> 400 Bad Request: context_length_exceeded"),
            ProviderErrorKind::ContextLength
        );
        assert_eq!(
            kind("prompt is too long: 210000 tokens > 200000 maximum"),
            ProviderErrorKind::ContextLength
        );
    }

    #[test]
    fn network_and_unknown_messages() {
        assert_eq!(
            kind("upstream error: error sending request for url (https://api.example.com)"),
            ProviderErrorKind::ProviderUnavailable
        );
        assert_eq!(
            kind("Anthropic API overloaded"),
            ProviderErrorKind::ProviderUnavailable
        );
        assert_eq!(
            kind("upstream error: anthropic stream interrupted: error decoding response body"),
            ProviderErrorKind::ProviderUnavailable
        );
        assert_eq!(kind("something odd happened"), ProviderErrorKind::Unknown);
        assert_eq!(
            kind("tool call 42 failed at step 500"),
            ProviderErrorKind::Unknown
        );
    }

    #[test]
    fn stream_errors_classify_like_http_errors() {
        let overloaded = upstream_stream_error(
            "Anthropic",
            "messages",
            None,
            Some("overloaded_error"),
            "Overloaded",
        );
        assert_eq!(
            overloaded.to_string(),
            "upstream error: Anthropic messages stream -> 529 overloaded_error: Overloaded"
        );
        let info = classify_provider_error(&overloaded.to_string());
        assert_eq!(info.kind, ProviderErrorKind::ProviderUnavailable);
        assert_eq!(info.status, Some(529));

        let limited =
            upstream_stream_error("Gemini", "streamGenerateContent", Some(429), None, "slow");
        assert_eq!(kind(&limited.to_string()), ProviderErrorKind::RateLimited);

        let quota = upstream_stream_error(
            "OpenAI",
            "chat/completions",
            None,
            Some("insufficient_quota"),
            "You exceeded your current quota",
        );
        assert_eq!(kind(&quota.to_string()), ProviderErrorKind::Quota);

        let unknown = upstream_stream_error("x", "chat", None, Some("weird"), "");
        assert_eq!(
            unknown.to_string(),
            "upstream error: x chat stream error: weird"
        );
    }

    #[test]
    fn retry_hint_marks_transient_failures() {
        let hint = retry_hint(&upstream_http_error(
            "OpenAI",
            "chat/completions",
            "429 Too Many Requests",
            Some("3"),
            "slow down",
        ))
        .unwrap();
        assert!(hint.retryable);
        assert_eq!(hint.retry_after, Some(Duration::from_secs(3)));

        let hint = retry_hint(&upstream_stream_error(
            "Anthropic",
            "messages",
            None,
            Some("api_error"),
            "Internal server error",
        ))
        .unwrap();
        assert!(hint.retryable);
        assert_eq!(hint.retry_after, None);

        let auth = retry_hint(&Error::Upstream(
            "x chat/completions -> 401 Unauthorized: bad key".into(),
        ))
        .unwrap();
        assert!(!auth.retryable);
        assert_eq!(
            retry_hint(&Error::InvalidRequest("rate limit wording".into())),
            None
        );
        assert_eq!(retry_hint(&Error::Other("odd".into())), None);
    }

    #[test]
    fn run_error_value_adds_classification_only_when_known() {
        let value = run_error_value(&Error::Upstream(
            "x chat/completions -> 429 Too Many Requests: busy (retry after 7s)".into(),
        ));
        assert_eq!(value["code"], "upstream_error");
        assert_eq!(value["provider_error"]["kind"], "rate_limited");
        assert_eq!(value["provider_error"]["status"], 429);
        assert_eq!(value["provider_error"]["retry_after_secs"], 7);
        let plain = run_error_value(&Error::Other("account runtime failed".into()));
        assert!(plain.get("provider_error").is_none());
    }
}
