//! Tool approvals: resolution, thread allowances, and auto-resolution of
//! requests an allowance already covers.

use milim_control_contract::{ControlCommandResultV1, ControlCommandStatusV1, ControlCommandV1};
use milim_core::{Error, Result};
use milim_storage::ControlApprovalRecord;
use serde_json::{json, Value};

use super::commands::required_payload_string;
use super::{now_ms, RunManager};
use crate::AppState;

impl RunManager {
    pub fn owns_approval(&self, approval_id: &str) -> bool {
        self.store
            .control_approval(approval_id)
            .ok()
            .flatten()
            .is_some()
    }

    pub(super) async fn resolve_approval(
        &self,
        state: &AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let approval_id = required_payload_string(&command.payload, "approval_id")?;
        let decision = required_payload_string(&command.payload, "decision")?;
        let approved = match decision.as_str() {
            "approve" => true,
            "deny" => false,
            _ => {
                return Err(Error::InvalidRequest(
                    "payload.decision must be approve or deny".into(),
                ))
            }
        };
        let response = command.payload.get("response").cloned();
        let Some(mut durable) = self.store.control_approval(&approval_id)? else {
            return Err(Error::NotFound(format!("approval {approval_id}")));
        };
        // Optional `scope: "thread"` ("Allow for this chat"). Older clients
        // omit it and keep one-shot semantics. `allowance_match: "prefix"`
        // asks for a command-prefix rule and falls back to the exact command.
        let prefix = match command
            .payload
            .get("allowance_match")
            .and_then(Value::as_str)
        {
            None | Some("exact") => false,
            Some("prefix") => true,
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "payload.allowance_match must be exact or prefix".into(),
                ))
            }
        };
        let allowance = match command.payload.get("scope").and_then(Value::as_str) {
            None | Some("once") => None,
            Some("thread") if approved => Some(thread_allowance_for_approval(&durable, prefix)?),
            Some("thread") => None,
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "payload.scope must be once or thread".into(),
                ))
            }
        };
        let scope = if allowance.is_some() {
            milim_agents::ApprovalScope::Thread
        } else {
            milim_agents::ApprovalScope::Once
        };
        let resolved =
            state
                .tool_approvals
                .resolve_with_scope(&approval_id, approved, response, scope);
        if resolved == milim_agents::ApprovalResolve::Conflict {
            return Ok(ControlCommandResultV1 {
                command_id: command.command_id.clone(),
                status: ControlCommandStatusV1::Conflict,
                thread_id: Some(durable.thread_id),
                revision: None,
                run_id: Some(durable.run_id),
                queue_id: None,
                confirmation_token: None,
                message: Some("approval was already resolved with a different decision".into()),
                data: Value::Null,
            });
        }
        if matches!(
            resolved,
            milim_agents::ApprovalResolve::Missing | milim_agents::ApprovalResolve::Failed
        ) {
            return Err(Error::InvalidRequest(
                "approval is no longer deliverable to its runtime".into(),
            ));
        }
        let snapshot = state
            .tool_approvals
            .wait_for_delivery(&approval_id, milim_agents::APPROVAL_DELIVERY_TIMEOUT)
            .await
            .ok_or_else(|| Error::NotFound(format!("approval {approval_id}")))?;
        if !matches!(
            snapshot.state,
            milim_agents::ApprovalState::Delivered | milim_agents::ApprovalState::Acknowledged
        ) {
            return Err(Error::Upstream(
                snapshot
                    .error
                    .unwrap_or_else(|| "approval delivery failed".into()),
            ));
        }
        let scope_name = if allowance.is_some() {
            "thread"
        } else {
            "once"
        };
        durable.status = if approved { "approved" } else { "denied" }.into();
        durable.decision_json = Some(
            json!({
                "decision": decision,
                "scope": scope_name,
                "allowance": allowance.as_ref().map(|allowance| allowance.key.as_str()),
            })
            .to_string(),
        );
        durable.resolved_at_ms = Some(now_ms());
        self.store.control_put_approval(&durable)?;
        if let Some(allowance) = allowance.clone() {
            self.record_approval_allowance(&durable.thread_id, allowance)?;
        }
        self.persist_and_emit(
            &durable.thread_id,
            Some(&durable.run_id),
            "approval_resolved",
            json!({
                "approval_id": approval_id,
                "decision": decision,
                "status": snapshot.state,
                "scope": scope_name,
            }),
        )?;
        let mut data = serde_json::to_value(snapshot)
            .map_err(|error| Error::Other(format!("serialize approval: {error}")))?;
        if let Some(object) = data.as_object_mut() {
            object.insert("scope".into(), Value::from(scope_name));
            if let Some(allowance) = &allowance {
                object.insert(
                    "allowance".into(),
                    serde_json::to_value(allowance).unwrap_or_default(),
                );
            }
        }
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(durable.thread_id),
            revision: None,
            run_id: Some(durable.run_id),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data,
        })
    }

    /// Current "Allow for this chat" rules for one thread.
    pub(crate) fn approval_allowances(
        &self,
        thread_id: &str,
    ) -> Result<Vec<crate::approval_allowances::ApprovalAllowance>> {
        crate::approval_allowances::load(&self.store, thread_id)
    }

    /// Revoke the listed chat allowances, or all of them when `keys` is None.
    pub(crate) fn revoke_approval_allowances(
        &self,
        thread_id: &str,
        keys: Option<&[String]>,
    ) -> Result<Vec<crate::approval_allowances::ApprovalAllowance>> {
        let remaining = crate::approval_allowances::revoke(&self.store, thread_id, keys)?;
        self.emit_approval_allowances(thread_id, &remaining);
        Ok(remaining)
    }

    pub(super) fn record_approval_allowance(
        &self,
        thread_id: &str,
        allowance: crate::approval_allowances::ApprovalAllowance,
    ) -> Result<()> {
        if let Some(allowances) =
            crate::approval_allowances::record(&self.store, thread_id, allowance, now_ms())?
        {
            self.emit_approval_allowances(thread_id, &allowances);
        }
        Ok(())
    }

    fn emit_approval_allowances(
        &self,
        thread_id: &str,
        allowances: &[crate::approval_allowances::ApprovalAllowance],
    ) {
        self.emit(
            crate::approval_allowances::ALLOWANCES_EVENT_TYPE,
            Some(thread_id),
            None,
            None,
            json!({ "thread_id": thread_id, "allowances": allowances }),
        );
    }

    /// Resolve a just-requested approval that a chat allowance already
    /// covers. Returns the matching rule key when it was auto-approved.
    pub(super) fn auto_resolve_allowed_approval(
        &self,
        state: &AppState,
        thread_id: &str,
        approval_id: &str,
        kind: &str,
        name: &str,
        arguments: &str,
    ) -> Result<Option<String>> {
        let allowances = crate::approval_allowances::load(&self.store, thread_id)?;
        let Some(allowance) =
            crate::approval_allowances::matching(&allowances, kind, name, arguments)
        else {
            return Ok(None);
        };
        if state.tool_approvals.resolve_with_scope(
            approval_id,
            true,
            None,
            milim_agents::ApprovalScope::Thread,
        ) != milim_agents::ApprovalResolve::Resolved
        {
            return Ok(None);
        }
        if let Some(mut durable) = self.store.control_approval(approval_id)? {
            durable.status = "approved".into();
            durable.decision_json = Some(
                json!({
                    "decision": "approve",
                    "scope": "thread",
                    "allowance": allowance.key,
                    "automatic": true,
                })
                .to_string(),
            );
            durable.resolved_at_ms = Some(now_ms());
            self.store.control_put_approval(&durable)?;
        }
        Ok(Some(allowance.key))
    }
}

/// The chat allowance an approved `scope: "thread"` decision would create.
fn thread_allowance_for_approval(
    durable: &ControlApprovalRecord,
    prefix: bool,
) -> Result<crate::approval_allowances::ApprovalAllowance> {
    let request: Value = serde_json::from_str(&durable.request_json).unwrap_or(Value::Null);
    let name = request
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = request
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or_default();
    prefix
        .then(|| crate::approval_allowances::prefix_allowance_for(&durable.kind, name, arguments))
        .flatten()
        .or_else(|| crate::approval_allowances::allowance_for(&durable.kind, name, arguments))
        .ok_or_else(|| {
            Error::InvalidRequest(
                "this approval cannot be allowed for the whole chat; approve it once instead"
                    .into(),
            )
        })
}

pub(super) fn normalized_approval_kind(kind: &str) -> &str {
    match kind {
        "command" => "command",
        "file_change" => "file_change",
        "permissions" | "permission" => "permission_elevation",
        "mcp_form" => "mcp_form",
        "mcp_url" => "mcp_url",
        _ => "unsupported",
    }
}
