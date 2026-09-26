//! Streaming assistant deltas: text and reasoning are buffered and flushed
//! as `assistant_delta` timeline events.

use std::time::{Duration, Instant};

use milim_core::Result;
use serde_json::json;

use super::RunManager;

const DELTA_FLUSH_BYTES: usize = 512;
pub(super) const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(40);

/// Accumulates a run's assistant text and reasoning and buffers the part not
/// yet emitted as `assistant_delta` events.
///
/// The first delta is emitted as soon as it arrives; after that, pending
/// output is emitted once it reaches `DELTA_FLUSH_BYTES`. Runs that poll their
/// stream with a `DELTA_FLUSH_INTERVAL` timeout flush idle output themselves;
/// a [`DeltaBuffer::timed`] buffer instead also flushes once that interval has
/// passed since its last due flush.
pub(super) struct DeltaBuffer<'a> {
    manager: &'a RunManager,
    thread_id: &'a str,
    run_id: &'a str,
    content: String,
    reasoning: String,
    pending_text: String,
    pending_reasoning: String,
    emitted_first_delta: bool,
    last_flush: Option<Instant>,
}

impl<'a> DeltaBuffer<'a> {
    /// A buffer whose caller flushes idle output on its own timeout.
    pub(super) fn new(manager: &'a RunManager, thread_id: &'a str, run_id: &'a str) -> Self {
        Self {
            manager,
            thread_id,
            run_id,
            content: String::new(),
            reasoning: String::new(),
            pending_text: String::new(),
            pending_reasoning: String::new(),
            emitted_first_delta: false,
            last_flush: None,
        }
    }

    /// A buffer that also flushes when `DELTA_FLUSH_INTERVAL` has passed since
    /// it was created or last flushed by [`DeltaBuffer::flush_if_due`].
    pub(super) fn timed(manager: &'a RunManager, thread_id: &'a str, run_id: &'a str) -> Self {
        Self {
            last_flush: Some(Instant::now()),
            ..Self::new(manager, thread_id, run_id)
        }
    }

    pub(super) fn push_text(&mut self, text: &str) {
        self.content.push_str(text);
        self.pending_text.push_str(text);
    }

    pub(super) fn push_reasoning(&mut self, text: &str) {
        self.reasoning.push_str(text);
        self.pending_reasoning.push_str(text);
    }

    pub(super) fn has_pending_text(&self) -> bool {
        !self.pending_text.is_empty()
    }

    pub(super) fn has_pending_reasoning(&self) -> bool {
        !self.pending_reasoning.is_empty()
    }

    /// Emits pending output if the first delta is still unsent, if pending
    /// output reached `DELTA_FLUSH_BYTES`, or, for a timed buffer, if the
    /// flush interval has passed.
    pub(super) fn flush_if_due(&mut self) -> Result<()> {
        if !self.emitted_first_delta
            || self.pending_text.len() + self.pending_reasoning.len() >= DELTA_FLUSH_BYTES
            || self
                .last_flush
                .is_some_and(|last_flush| last_flush.elapsed() >= DELTA_FLUSH_INTERVAL)
        {
            self.flush()?;
            self.emitted_first_delta = true;
            if self.last_flush.is_some() {
                self.last_flush = Some(Instant::now());
            }
        }
        Ok(())
    }

    /// Emits all pending output as one `assistant_delta` event.
    pub(super) fn flush(&mut self) -> Result<()> {
        if self.pending_text.is_empty() && self.pending_reasoning.is_empty() {
            return Ok(());
        }
        self.manager.persist_and_emit(
            self.thread_id,
            Some(self.run_id),
            "assistant_delta",
            json!({
                "text": std::mem::take(&mut self.pending_text),
                "reasoning": std::mem::take(&mut self.pending_reasoning),
            }),
        )?;
        Ok(())
    }

    /// Drops the tail a failed provider attempt contributed. The failed
    /// attempt's partial text is not part of the answer; the retried step
    /// streams it again.
    pub(super) fn truncate_for_retry(
        &mut self,
        discarded_content_bytes: usize,
        discarded_reasoning_bytes: usize,
    ) {
        self.content
            .truncate(self.content.len().saturating_sub(discarded_content_bytes));
        self.reasoning.truncate(
            self.reasoning
                .len()
                .saturating_sub(discarded_reasoning_bytes),
        );
    }

    pub(super) fn content(&self) -> &str {
        &self.content
    }

    pub(super) fn reasoning(&self) -> &str {
        &self.reasoning
    }

    /// The accumulated assistant text and reasoning.
    pub(super) fn into_output(self) -> (String, String) {
        (self.content, self.reasoning)
    }
}
