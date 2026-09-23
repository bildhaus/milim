//! Shared conversion of non-success provider HTTP responses into errors.

use milim_core::provider_error::upstream_http_error;
use milim_core::Error;

/// Consume a failed response into an upstream error that keeps the status,
/// the provider's body, and any `Retry-After` hint for classification.
pub(crate) async fn http_status_error(
    label: &str,
    operation: &str,
    resp: reqwest::Response,
) -> Error {
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let text = resp.text().await.unwrap_or_default();
    upstream_http_error(label, operation, status, retry_after.as_deref(), &text)
}
