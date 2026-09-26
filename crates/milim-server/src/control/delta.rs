//! Streaming assistant deltas: text and reasoning are buffered and flushed
//! as `assistant_delta` timeline events.

use std::time::Duration;

use milim_core::Result;
use serde_json::json;

use super::RunManager;

pub(super) const DELTA_FLUSH_BYTES: usize = 512;
pub(super) const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(40);

pub(super) fn flush_deltas(
    manager: &RunManager,
    thread_id: &str,
    run_id: &str,
    text: &mut String,
    reasoning: &mut String,
) -> Result<()> {
    if text.is_empty() && reasoning.is_empty() {
        return Ok(());
    }
    manager.persist_and_emit(
        thread_id,
        Some(run_id),
        "assistant_delta",
        json!({ "text": std::mem::take(text), "reasoning": std::mem::take(reasoning) }),
    )?;
    Ok(())
}
