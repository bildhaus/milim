//! Timeline persistence and live event emission.

use milim_control_contract::ControlEventV1;
use milim_core::{Error, Result};
use milim_storage::{ControlThreadRecord, ControlTimelineRecord};
use serde_json::{json, Value};
use uuid::Uuid;

use super::views::timeline_item;
use super::RunManager;

impl RunManager {
    pub(super) fn persist_and_emit(
        &self,
        thread_id: &str,
        run_id: Option<&str>,
        item_type: &str,
        data: Value,
    ) -> Result<ControlTimelineRecord> {
        // Streaming loops append deltas synchronously; keep the SQLite write
        // from stalling the other tasks scheduled on this worker.
        let record = crate::blocking::in_place(|| {
            self.store.control_append_timeline(
                thread_id,
                &Uuid::new_v4().to_string(),
                run_id,
                item_type,
                &data.to_string(),
            )
        })?;
        self.emit(
            "timeline.appended",
            Some(thread_id),
            Some(&record.epoch),
            Some(record.seq),
            json!({ "item": timeline_item(record.clone())? }),
        );
        Ok(record)
    }

    pub fn record_schedule_error(&self, thread_id: &str, message: &str) -> Result<()> {
        let _admission = self.mutation_guard()?;
        self.persist_and_emit(
            thread_id,
            None,
            "runtime_notice",
            json!({
                "message": message,
                "tone": "error",
                "source": "schedule",
            }),
        )?;
        Ok(())
    }

    pub(super) fn persist_message_and_event(
        &self,
        thread_id: &str,
        run_id: &str,
        message: Value,
        step_id: Option<&str>,
        event_type: &str,
        event_data: Value,
    ) -> Result<ControlTimelineRecord> {
        let item_id = message
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("message projection is missing id".into()))?;
        let (record, _) = self.store.control_commit_message_projection_and_event(
            thread_id,
            run_id,
            item_id,
            &message.to_string(),
            &Uuid::new_v4().to_string(),
            step_id,
            event_type,
            &event_data.to_string(),
        )?;
        self.emit(
            "timeline.appended",
            Some(thread_id),
            Some(&record.epoch),
            Some(record.seq),
            json!({ "item": timeline_item(record.clone())? }),
        );
        Ok(record)
    }

    pub(super) fn persist_adopted_message_and_event(
        &self,
        thread_id: &str,
        run_id: &str,
        message: Value,
        event_type: &str,
        event_data: Value,
    ) -> Result<ControlTimelineRecord> {
        let item_id = message
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("message projection is missing id".into()))?;
        let (record, _) = self.store.control_adopt_message_projection_and_event(
            thread_id,
            run_id,
            item_id,
            &message.to_string(),
            &Uuid::new_v4().to_string(),
            None,
            event_type,
            &event_data.to_string(),
        )?;
        self.emit(
            "timeline.appended",
            Some(thread_id),
            Some(&record.epoch),
            Some(record.seq),
            json!({ "item": timeline_item(record.clone())? }),
        );
        Ok(record)
    }

    pub(super) fn emit_thread_changed(&self, thread: &ControlThreadRecord, event_type: &str) {
        self.emit(
            event_type,
            Some(&thread.id),
            Some(&thread.epoch),
            None,
            json!({ "revision": thread.revision }),
        );
    }

    pub(super) fn emit(
        &self,
        event_type: &str,
        thread_id: Option<&str>,
        epoch: Option<&str>,
        seq: Option<u64>,
        data: Value,
    ) {
        let _ = self.events.send(ControlEventV1 {
            event_id: Uuid::new_v4().to_string(),
            host_id: self.host().host_id,
            thread_id: thread_id.map(str::to_string),
            epoch: epoch.map(str::to_string),
            seq,
            event_type: event_type.to_string(),
            data,
        });
    }
}
