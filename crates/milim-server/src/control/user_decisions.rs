//! Decisions a run handed the user (`ask_user`, `exit_plan_mode`).
//!
//! The run records a `user_decision_requested` timeline item and ends. The
//! decision stays pending until one `user_decision_resolved` item closes it:
//! `answered` by the `turn.send` that names it in `answers_decision`, or
//! `superseded` by any other turn that starts first. Both happen under the
//! thread lock, so of two clients answering at once one wins and the other
//! gets `conflict`; a retried `command_id` replays its stored receipt.

use milim_control_contract::{
    UserDecisionAnswerV1, UserDecisionResolvedV1, UserDecisionResponseV1, UserDecisionStatusV1,
    UserDecisionV1, USER_DECISION_REQUESTED_ITEM, USER_DECISION_RESOLVED_ITEM,
};
use milim_core::{Error, Result};
use serde_json::Value;

use super::RunManager;

/// Timeline items read back to find a thread's pending decision. Only the
/// deciding run's closing items and thread setting changes follow it, so a
/// pending decision is always well inside this window.
const PENDING_DECISION_SCAN: usize = 200;

pub(super) struct PendingUserDecision {
    pub(super) decision: UserDecisionV1,
    pub(super) run_id: Option<String>,
}

impl RunManager {
    /// The thread's latest decision, if nothing has resolved it yet.
    pub(super) fn pending_user_decision(
        &self,
        thread_id: &str,
    ) -> Result<Option<PendingUserDecision>> {
        let Some(page) =
            self.store
                .control_timeline_page(thread_id, None, None, true, PENDING_DECISION_SCAN)?
        else {
            return Ok(None);
        };
        let mut resolved = Vec::new();
        for item in page.items.iter().rev() {
            let parse = || serde_json::from_str::<Value>(&item.data_json).ok();
            if item.item_type == USER_DECISION_RESOLVED_ITEM {
                if let Some(id) =
                    parse().and_then(|data| data.get("decision_id")?.as_str().map(str::to_string))
                {
                    resolved.push(id);
                }
            } else if item.item_type == USER_DECISION_REQUESTED_ITEM {
                let Some(decision) = parse()
                    .and_then(|data| data.get("decision").cloned())
                    .and_then(|decision| serde_json::from_value::<UserDecisionV1>(decision).ok())
                else {
                    continue;
                };
                // Only the latest decision can be pending: any turn after an
                // older one resolved it.
                return Ok(
                    (!resolved.contains(&decision.id)).then(|| PendingUserDecision {
                        decision,
                        run_id: item.run_id.clone(),
                    }),
                );
            }
        }
        Ok(None)
    }

    /// Check that `answer` names the thread's pending decision and fits its
    /// kind. `Ok(None)` means the decision is already closed or unknown.
    pub(super) fn answerable_user_decision(
        &self,
        thread_id: &str,
        answer: &UserDecisionAnswerV1,
    ) -> Result<Option<PendingUserDecision>> {
        let Some(pending) = self
            .pending_user_decision(thread_id)?
            .filter(|pending| pending.decision.id == answer.decision_id)
        else {
            return Ok(None);
        };
        if !answer.response.fits(pending.decision.kind) {
            return Err(Error::InvalidRequest(format!(
                "answers_decision.response {:?} does not fit a {:?} decision",
                answer.response, pending.decision.kind
            )));
        }
        Ok(Some(pending))
    }

    pub(super) fn resolve_user_decision(
        &self,
        thread_id: &str,
        pending: &PendingUserDecision,
        status: UserDecisionStatusV1,
        response: Option<UserDecisionResponseV1>,
        command_id: Option<&str>,
    ) -> Result<()> {
        let resolved = UserDecisionResolvedV1 {
            decision_id: pending.decision.id.clone(),
            status,
            response,
            command_id: command_id.map(str::to_string),
        };
        let data = serde_json::to_value(&resolved)
            .map_err(|error| Error::Other(format!("serialize decision resolution: {error}")))?;
        self.persist_and_emit(
            thread_id,
            pending.run_id.as_deref(),
            USER_DECISION_RESOLVED_ITEM,
            data,
        )?;
        Ok(())
    }

    /// A turn is starting without answering the pending decision, so the
    /// decision can no longer be answered.
    pub(super) fn supersede_pending_user_decision(&self, thread_id: &str) -> Result<()> {
        if let Some(pending) = self.pending_user_decision(thread_id)? {
            self.resolve_user_decision(
                thread_id,
                &pending,
                UserDecisionStatusV1::Superseded,
                None,
                None,
            )?;
        }
        Ok(())
    }
}
