//! Shared conversion of provider transport failures into errors.

use std::error::Error as _;

use milim_core::provider_error::upstream_http_error;
use milim_core::Error;

/// A non-success provider response, read to completion.
pub(crate) struct HttpFailure {
    pub(crate) status: reqwest::StatusCode,
    retry_after: Option<String>,
    pub(crate) body: String,
}

impl HttpFailure {
    /// Consume a failed response, keeping the status, the provider's body, and
    /// any retry hint for classification.
    pub(crate) async fn read(resp: reqwest::Response) -> Self {
        let status = resp.status();
        let retry_after = retry_after_header(resp.headers());
        let body = resp.text().await.unwrap_or_default();
        Self {
            status,
            retry_after,
            body,
        }
    }

    pub(crate) fn into_error(self, label: &str, operation: &str) -> Error {
        upstream_http_error(
            label,
            operation,
            self.status,
            self.retry_after.as_deref(),
            &self.body,
        )
    }
}

/// Consume a failed response into an upstream error that keeps the status,
/// the provider's body, and any `Retry-After` hint for classification.
pub(crate) async fn http_status_error(
    label: &str,
    operation: &str,
    resp: reqwest::Response,
) -> Error {
    HttpFailure::read(resp).await.into_error(label, operation)
}

/// The retry delay in seconds. OpenAI's millisecond `retry-after-ms` wins over
/// the standard `Retry-After` because it is more precise.
fn retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
    };
    if let Some(ms) = header("retry-after-ms")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|ms| ms.is_finite() && *ms >= 0.0)
    {
        return Some((ms / 1000.0).to_string());
    }
    header(reqwest::header::RETRY_AFTER.as_str()).map(str::to_string)
}

/// A failure while reading an already-open response stream. The source chain
/// is kept so dropped connections and read timeouts classify as transient.
pub(crate) fn stream_read_error(label: &str, error: reqwest::Error) -> Error {
    let mut message = format!("{label} stream interrupted: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    if error.is_timeout() && !message.to_ascii_lowercase().contains("timed out") {
        message.push_str(" (timed out)");
    }
    Error::Upstream(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    #[test]
    fn prefers_millisecond_retry_hint() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", HeaderValue::from_static("20"));
        assert_eq!(retry_after_header(&headers).as_deref(), Some("20"));
        headers.insert("retry-after-ms", HeaderValue::from_static("1500"));
        assert_eq!(retry_after_header(&headers).as_deref(), Some("1.5"));
        assert!(retry_after_header(&HeaderMap::new()).is_none());
    }
}
