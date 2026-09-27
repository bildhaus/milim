//! How long a generation stream may stay silent before it counts as stalled.
//!
//! Reasoning models think silently before they stream anything, and at high
//! effort that can take minutes. A fixed read timeout short enough to catch a
//! hung connection cuts those requests off, and the retry bills the reasoning
//! again. Each stream instead gets an idle budget sized to the reasoning it
//! asked for; the streaming client's read timeout stays as the hang guard.

use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use milim_core::api::openai::ReasoningEffort;
use milim_core::{Error, Result};

use crate::http_error::stream_read_error;

/// No reasoning expected: the previous fixed read timeout.
#[cfg(not(test))]
pub(crate) const STANDARD_IDLE: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(crate) const STANDARD_IDLE: Duration = Duration::from_millis(250);

/// The model reasons before (or between) visible output.
#[cfg(not(test))]
pub(crate) const REASONING_IDLE: Duration = Duration::from_secs(5 * 60);
#[cfg(test)]
pub(crate) const REASONING_IDLE: Duration = Duration::from_millis(500);

/// High-effort reasoning and `-pro` / deep-research models. Also the
/// streaming client's read timeout, so no stream waits longer than this.
#[cfg(not(test))]
pub(crate) const DEEP_REASONING_IDLE: Duration = Duration::from_secs(15 * 60);
#[cfg(test)]
pub(crate) const DEEP_REASONING_IDLE: Duration = Duration::from_millis(1_000);

/// The idle budget for one generation request.
pub(crate) fn stream_idle_timeout(model: &str, effort: Option<ReasoningEffort>) -> Duration {
    let id = model.trim().to_ascii_lowercase();
    let reasons = reasons_by_default(&id);
    if id.contains("deep-research") || (reasons && id.contains("-pro")) {
        return DEEP_REASONING_IDLE;
    }
    match effort.filter(|effort| !effort.is_auto()) {
        Some(ReasoningEffort::High | ReasoningEffort::Xhigh | ReasoningEffort::Max) => {
            DEEP_REASONING_IDLE
        }
        Some(ReasoningEffort::None) => STANDARD_IDLE,
        Some(ReasoningEffort::Minimal | ReasoningEffort::Low | ReasoningEffort::Medium)
        | Some(ReasoningEffort::On) => REASONING_IDLE,
        Some(ReasoningEffort::Auto) | None if reasons => REASONING_IDLE,
        Some(ReasoningEffort::Auto) | None => STANDARD_IDLE,
    }
}

/// Model families that reason unless told not to.
fn reasons_by_default(id: &str) -> bool {
    crate::remote::looks_reasoning_model(id)
        || crate::openai_responses::is_openai_reasoning_model(id)
        || id.contains("gemini-2.5")
        || id.contains("gemini-3")
        || id.contains("thinking")
        || id.contains("qwq")
}

#[cfg(not(test))]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const CONNECT_TIMEOUT: Duration = Duration::from_millis(50);

/// The process-wide client for generation streams: the usual connect
/// timeout, with the deep-reasoning budget as the read timeout. Per-request
/// idle budgets are enforced by [`send`] and [`SseLines`].
pub(crate) fn streaming_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(DEEP_REASONING_IDLE)
                .build()
                .expect("valid reqwest client timeout configuration")
        })
        .clone()
}

/// Send a generation request, failing when no response arrives within `idle`.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    idle: Duration,
    label: &str,
    operation: &str,
) -> Result<reqwest::Response> {
    match tokio::time::timeout(idle, request.send()).await {
        Ok(result) => result.map_err(|e| Error::Upstream(e.to_string())),
        Err(_) => Err(Error::Upstream(format!(
            "{label} {operation}: no response within {idle:?} (timed out)"
        ))),
    }
}

/// The lines of an open event stream, read under an idle budget.
pub(crate) struct SseLines<S> {
    body: S,
    buf: Vec<u8>,
    idle: Duration,
    label: String,
    ended: bool,
}

impl<S> SseLines<S>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Unpin,
{
    pub(crate) fn new(body: S, idle: Duration, label: &str) -> Self {
        Self {
            body,
            buf: Vec::new(),
            idle,
            label: label.to_string(),
            ended: false,
        }
    }

    /// The next line with its line ending trimmed, or `None` once the body
    /// ends. A last line the provider did not newline-terminate still counts.
    pub(crate) async fn next(&mut self) -> Option<Result<String>> {
        loop {
            let line = match self.buf.iter().position(|&b| b == b'\n') {
                Some(pos) => self.buf.drain(..=pos).collect::<Vec<u8>>(),
                None if self.ended && !self.buf.is_empty() => std::mem::take(&mut self.buf),
                None if self.ended => return None,
                None => {
                    match next_chunk(&mut self.body, self.idle, &self.label).await {
                        Some(Ok(bytes)) => self.buf.extend_from_slice(&bytes),
                        Some(Err(error)) => return Some(Err(error)),
                        None => self.ended = true,
                    }
                    continue;
                }
            };
            return Some(Ok(String::from_utf8_lossy(&line).trim_end().to_string()));
        }
    }
}

/// The next body chunk of an open stream, or a stall error once the provider
/// has sent nothing for `idle`.
async fn next_chunk<S>(body: &mut S, idle: Duration, label: &str) -> Option<Result<Bytes>>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Unpin,
{
    match tokio::time::timeout(idle, body.next()).await {
        Ok(Some(Ok(bytes))) => Some(Ok(bytes)),
        Ok(Some(Err(error))) => Some(Err(stream_read_error(label, error))),
        Ok(None) => None,
        Err(_) => Some(Err(Error::Upstream(format!(
            "{label} stream interrupted: no data for {idle:?} (timed out)"
        )))),
    }
}

/// The error for a stream that closed without the provider's terminal event.
/// The agent loop retries it like any other transient provider failure.
pub(crate) fn ended_early() -> Error {
    Error::Other("provider stream ended before a completion event".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::provider_error::retry_hint;

    #[test]
    fn idle_budget_follows_reasoning_effort_and_model_family() {
        assert_eq!(stream_idle_timeout("gpt-4o", None), STANDARD_IDLE);
        assert_eq!(
            stream_idle_timeout("llama3", Some(ReasoningEffort::Auto)),
            STANDARD_IDLE
        );
        assert_eq!(stream_idle_timeout("gpt-5", None), REASONING_IDLE);
        assert_eq!(stream_idle_timeout("gemini-3-flash", None), REASONING_IDLE);
        assert_eq!(
            stream_idle_timeout("gpt-5", Some(ReasoningEffort::None)),
            STANDARD_IDLE
        );
        assert_eq!(
            stream_idle_timeout("llama3", Some(ReasoningEffort::Low)),
            REASONING_IDLE
        );
        for effort in [
            ReasoningEffort::High,
            ReasoningEffort::Xhigh,
            ReasoningEffort::Max,
        ] {
            assert_eq!(stream_idle_timeout("o3", Some(effort)), DEEP_REASONING_IDLE);
        }
        assert_eq!(stream_idle_timeout("o3-pro", None), DEEP_REASONING_IDLE);
        assert_eq!(
            stream_idle_timeout("o4-mini-deep-research", Some(ReasoningEffort::Low)),
            DEEP_REASONING_IDLE
        );
        assert!(STANDARD_IDLE < REASONING_IDLE && REASONING_IDLE < DEEP_REASONING_IDLE);
    }

    #[tokio::test]
    async fn silent_stream_is_a_retryable_stall() {
        let mut body = futures::stream::pending::<reqwest::Result<Bytes>>();
        let error = next_chunk(&mut body, Duration::from_millis(10), "OpenAI")
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(retry_hint(&error).unwrap().retryable);
    }
}
