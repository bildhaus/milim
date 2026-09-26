//! Turn attachments: paired-device uploads and validation before acceptance.

use std::time::{Duration, Instant};

use base64::Engine as _;
use milim_control_contract::{
    ControlAttachmentUploadV1, ControlAttachmentV1, ControlCommandV1, CONTROL_MAX_ATTACHMENTS,
    CONTROL_MAX_ATTACHMENT_BYTES, CONTROL_MAX_ATTACHMENT_CONTENT_CHARS,
};
use milim_core::{Error, Result};
use serde_json::Value;
use uuid::Uuid;

use super::{now_ms, PendingAttachmentUpload, RunManager};

const MAX_CONTROL_ATTACHMENT_NAME_CHARS: usize = 140;
const MAX_CONTROL_ATTACHMENT_MIME_CHARS: usize = 120;
const MAX_CONTROL_ATTACHMENT_DATA_URL_CHARS: usize = 3 * 1024 * 1024;
const CONTROL_ATTACHMENT_UPLOAD_TTL: Duration = Duration::from_secs(15 * 60);
pub(super) const CONTROL_MAX_PENDING_UPLOADS_PER_DEVICE: usize = 12;

impl RunManager {
    pub(crate) fn put_attachment_upload(
        &self,
        device_id: &str,
        client_attachment_id: &str,
        name: &str,
        mime: &str,
        declared_size: u64,
        bytes: Vec<u8>,
    ) -> Result<ControlAttachmentUploadV1> {
        if client_attachment_id.trim().is_empty() || client_attachment_id.chars().count() > 200 {
            return Err(Error::InvalidRequest(
                "attachment upload IDs must contain 1 to 200 characters".into(),
            ));
        }
        if name.trim().is_empty()
            || name.chars().count() > MAX_CONTROL_ATTACHMENT_NAME_CHARS
            || mime.trim().is_empty()
            || mime.chars().count() > MAX_CONTROL_ATTACHMENT_MIME_CHARS
        {
            return Err(Error::InvalidRequest(
                "attachment upload metadata is missing or too long".into(),
            ));
        }
        if bytes.is_empty() || bytes.len() as u64 != declared_size {
            return Err(Error::InvalidRequest(
                "attachment upload size does not match its body".into(),
            ));
        }
        if declared_size > CONTROL_MAX_ATTACHMENT_BYTES {
            return Err(Error::InvalidRequest(format!(
                "attachment {name} exceeds the 2 MiB limit"
            )));
        }

        let now = Instant::now();
        let mut uploads = self
            .attachment_uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        uploads.retain(|_, upload| upload.expires_at > now);
        let existing = uploads.values().find(|upload| {
            upload.device_id == device_id && upload.client_attachment_id == client_attachment_id
        });
        if let Some(existing) = existing {
            if existing.name != name
                || existing.mime != mime
                || existing.bytes.as_slice() != bytes.as_slice()
            {
                return Err(Error::InvalidRequest(
                    "an attachment upload ID cannot be reused for different content".into(),
                ));
            }
            return Ok(ControlAttachmentUploadV1 {
                upload_id: existing.upload_id.clone(),
                expires_at_ms: existing.expires_at_ms,
            });
        }
        if uploads
            .values()
            .filter(|upload| upload.device_id == device_id)
            .count()
            >= CONTROL_MAX_PENDING_UPLOADS_PER_DEVICE
        {
            return Err(Error::InvalidRequest(
                "this device already has 12 pending attachment uploads".into(),
            ));
        }
        let upload_id = Uuid::new_v4().to_string();
        let expires_at_ms = now_ms() + CONTROL_ATTACHMENT_UPLOAD_TTL.as_millis() as i64;
        uploads.insert(
            upload_id.clone(),
            PendingAttachmentUpload {
                upload_id: upload_id.clone(),
                client_attachment_id: client_attachment_id.to_string(),
                device_id: device_id.to_string(),
                name: name.to_string(),
                mime: mime.to_string(),
                bytes,
                expires_at: now + CONTROL_ATTACHMENT_UPLOAD_TTL,
                expires_at_ms,
            },
        );
        Ok(ControlAttachmentUploadV1 {
            upload_id,
            expires_at_ms,
        })
    }

    pub(super) fn resolve_command_attachment_uploads(
        &self,
        device_id: Option<&str>,
        command: &mut ControlCommandV1,
    ) -> Result<Vec<String>> {
        let Some(attachments) = command
            .payload
            .get_mut("attachments")
            .and_then(Value::as_array_mut)
        else {
            return Ok(Vec::new());
        };
        let requested = attachments
            .iter()
            .filter_map(|attachment| {
                attachment
                    .get("upload_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        if requested.is_empty() {
            return Ok(Vec::new());
        }
        let device_id = device_id.ok_or_else(|| {
            Error::Unauthorized("attachment uploads require a paired-device credential".into())
        })?;
        let uploads = {
            let now = Instant::now();
            let mut pending = self
                .attachment_uploads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pending.retain(|_, upload| upload.expires_at > now);
            requested
                .iter()
                .map(|upload_id| {
                    pending.get(upload_id).cloned().ok_or_else(|| {
                        Error::InvalidRequest(
                            "an attachment upload expired; attach the file again".into(),
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?
        };
        for (attachment, upload) in attachments
            .iter_mut()
            .filter(|attachment| attachment.get("upload_id").is_some())
            .zip(uploads.iter())
        {
            if upload.device_id != device_id {
                return Err(Error::Unauthorized(
                    "attachment upload belongs to another paired device".into(),
                ));
            }
            let object = attachment
                .as_object_mut()
                .ok_or_else(|| Error::InvalidRequest("attachments must be JSON objects".into()))?;
            let id = object.get("id").and_then(Value::as_str).unwrap_or_default();
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mime = object
                .get("mime")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let size = object
                .get("size")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            if id != upload.client_attachment_id
                || name != upload.name
                || mime != upload.mime
                || size != upload.bytes.len() as u64
            {
                return Err(Error::InvalidRequest(
                    "attachment upload metadata does not match the command".into(),
                ));
            }
            let encoded = base64::engine::general_purpose::STANDARD.encode(&upload.bytes);
            object.insert(
                "data_url".into(),
                Value::String(format!("data:{};base64,{encoded}", upload.mime)),
            );
            object.remove("upload_id");
        }
        Ok(requested)
    }

    pub(super) fn consume_attachment_uploads(&self, upload_ids: &[String]) {
        if upload_ids.is_empty() {
            return;
        }
        let mut pending = self
            .attachment_uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for upload_id in upload_ids {
            pending.remove(upload_id);
        }
    }
}

pub(super) fn validate_control_attachments(attachments: &[ControlAttachmentV1]) -> Result<()> {
    if attachments.len() > CONTROL_MAX_ATTACHMENTS {
        return Err(Error::InvalidRequest(format!(
            "a turn may contain at most {CONTROL_MAX_ATTACHMENTS} attachments"
        )));
    }
    for attachment in attachments {
        if attachment.id.trim().is_empty() || attachment.name.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "attachments require stable IDs and names".into(),
            ));
        }
        if attachment.name.chars().count() > MAX_CONTROL_ATTACHMENT_NAME_CHARS
            || attachment.mime.chars().count() > MAX_CONTROL_ATTACHMENT_MIME_CHARS
        {
            return Err(Error::InvalidRequest(format!(
                "attachment {} metadata is too long",
                attachment.name
            )));
        }
        if attachment.size > CONTROL_MAX_ATTACHMENT_BYTES
            && !(attachment.truncated && attachment.content.is_some())
        {
            return Err(Error::InvalidRequest(format!(
                "attachment {} exceeds the 2 MiB limit",
                attachment.name
            )));
        }
        if attachment
            .content
            .as_ref()
            .is_some_and(|value| value.chars().count() > CONTROL_MAX_ATTACHMENT_CONTENT_CHARS)
        {
            return Err(Error::InvalidRequest(format!(
                "attachment {} text exceeds the 128 KiB limit",
                attachment.name
            )));
        }
        if attachment
            .data_url
            .as_ref()
            .is_some_and(|value| value.len() > MAX_CONTROL_ATTACHMENT_DATA_URL_CHARS)
        {
            return Err(Error::InvalidRequest(format!(
                "attachment {} image payload exceeds the wire limit",
                attachment.name
            )));
        }
        if attachment.content.is_none() && attachment.data_url.is_none() {
            return Err(Error::InvalidRequest(format!(
                "attachment {} has no content",
                attachment.name
            )));
        }
    }
    Ok(())
}

pub(super) fn control_account_images(
    attachments: &[ControlAttachmentV1],
) -> Vec<crate::codex_bridge::AccountImage> {
    attachments
        .iter()
        .filter_map(|attachment| {
            let data_url = attachment.data_url.as_deref()?;
            if !attachment.mime.starts_with("image/") {
                return None;
            }
            let (_, data) = data_url.split_once(',')?;
            Some(crate::codex_bridge::AccountImage {
                media_type: attachment.mime.clone(),
                data: data.to_string(),
            })
        })
        .collect()
}
