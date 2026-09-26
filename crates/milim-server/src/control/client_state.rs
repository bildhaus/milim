//! Desktop-owned state mirrored to control clients: appearance, the model
//! catalog, and model favorites.

use base64::Engine as _;
use milim_control_contract::{
    AppearanceSnapshotV1, ControlCommandResultV1, ControlCommandStatusV1, ControlCommandV1,
};
use milim_core::{Error, Result};
use serde_json::{json, Value};

use super::{
    AppearanceBackgroundAsset, RunManager, MODEL_CATALOG_STATE_KEY, MODEL_FAVORITES_EVENT_TYPE,
    MODEL_FAVORITES_SETTINGS_KEY,
};

pub(super) const APPEARANCE_STATE_KEY: &str = "milim.appearanceSnapshot";
pub(super) const CUSTOM_THEMES_STATE_KEY: &str = "milim.customThemes";
const MAX_APPEARANCE_BACKGROUND_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODEL_FAVORITES: usize = 256;
const MAX_MODEL_FAVORITE_ID_CHARS: usize = 512;

impl RunManager {
    pub fn appearance_snapshot(&self) -> AppearanceSnapshotV1 {
        self.store
            .get_json(APPEARANCE_STATE_KEY)
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default()
    }

    pub fn publish_appearance(&self) {
        self.emit(
            "appearance.updated",
            None,
            None,
            None,
            json!({ "appearance": self.appearance_snapshot() }),
        );
    }

    pub fn publish_model_catalog(&self) {
        self.emit("models.updated", None, None, None, json!({}));
    }

    pub(super) fn published_model_catalog(&self) -> Option<Vec<Value>> {
        self.store
            .get_json(MODEL_CATALOG_STATE_KEY)
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<Vec<Value>>(&value).ok())
            .filter(|models| {
                models.iter().all(|model| {
                    model
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| !id.trim().is_empty())
                })
            })
    }

    pub fn model_favorites(&self) -> Vec<String> {
        self.store
            .get_json(MODEL_FAVORITES_SETTINGS_KEY)
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<Value>(&value).ok())
            .and_then(|value| value.get("state")?.get("favorites").cloned())
            .and_then(|value| value.as_array().cloned())
            .map(|values| normalized_model_favorite_ids(&values, false).unwrap_or_default())
            .unwrap_or_default()
    }

    pub fn publish_model_favorites(&self) {
        self.emit(
            MODEL_FAVORITES_EVENT_TYPE,
            None,
            None,
            None,
            json!({ "favorite_model_ids": self.model_favorites() }),
        );
    }

    pub(crate) fn appearance_background_asset(&self) -> Option<AppearanceBackgroundAsset> {
        let appearance = self.appearance_snapshot();
        if !appearance.background.has_image {
            return None;
        }
        let themes = self
            .store
            .get_json(CUSTOM_THEMES_STATE_KEY)
            .ok()
            .flatten()?;
        let themes: Value = serde_json::from_str(&themes).ok()?;
        let source = themes
            .as_array()?
            .iter()
            .find(|theme| theme.get("id").and_then(Value::as_str) == Some(&appearance.theme_id))?
            .get("background")?
            .get("image")?
            .as_str()?;
        let (mime, bytes) = decode_appearance_background(source)?;
        Some(AppearanceBackgroundAsset {
            revision: appearance.revision,
            mime,
            bytes,
        })
    }

    pub(super) fn set_model_favorites(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let values = command
            .payload
            .get("favorite_model_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::InvalidRequest("payload.favorite_model_ids must be an array".into())
            })?;
        let favorite_model_ids = normalized_model_favorite_ids(values, true)?;
        let mut root = self
            .store
            .get_json(MODEL_FAVORITES_SETTINGS_KEY)?
            .map(|value| {
                serde_json::from_str::<Value>(&value)
                    .map_err(|error| Error::Other(format!("invalid stored settings JSON: {error}")))
            })
            .transpose()?
            .unwrap_or_else(|| json!({ "state": {}, "version": 0 }));
        let state = root
            .as_object_mut()
            .ok_or_else(|| Error::Other("stored settings are not an object".into()))?
            .entry("state")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| Error::Other("stored settings state is not an object".into()))?;
        state.insert("favorites".into(), json!(favorite_model_ids));
        self.store
            .set_json(MODEL_FAVORITES_SETTINGS_KEY, &root.to_string())?;
        self.publish_model_favorites();
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: None,
            revision: None,
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({ "favorite_model_ids": favorite_model_ids }),
        })
    }
}

fn normalized_model_favorite_ids(values: &[Value], strict: bool) -> Result<Vec<String>> {
    if values.len() > MAX_MODEL_FAVORITES {
        return Err(Error::InvalidRequest(format!(
            "favorite_model_ids supports at most {MAX_MODEL_FAVORITES} models"
        )));
    }
    let mut normalized = Vec::new();
    for value in values {
        let Some(id) = value.as_str() else {
            if strict {
                return Err(Error::InvalidRequest(
                    "favorite_model_ids must contain only strings".into(),
                ));
            }
            continue;
        };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if id.chars().count() > MAX_MODEL_FAVORITE_ID_CHARS {
            if strict {
                return Err(Error::InvalidRequest(format!(
                    "favorite model ids must contain at most {MAX_MODEL_FAVORITE_ID_CHARS} characters"
                )));
            }
            continue;
        }
        if !normalized.iter().any(|existing| existing == id) {
            normalized.push(id.to_string());
        }
    }
    Ok(normalized)
}

pub(super) fn decode_appearance_background(source: &str) -> Option<(&'static str, Vec<u8>)> {
    let source = source.trim();
    let source = source.strip_prefix("url(")?.strip_suffix(')')?.trim();
    let source = match source.as_bytes() {
        [b'\'', .., b'\''] | [b'"', .., b'"'] if source.len() >= 2 => &source[1..source.len() - 1],
        _ => source,
    };
    let data = source.strip_prefix("data:")?;
    let (metadata, payload) = data.split_once(',')?;
    let mut metadata = metadata.split(';');
    let source_mime = metadata.next()?.trim().to_ascii_lowercase();
    if !metadata.any(|part| part.eq_ignore_ascii_case("base64")) {
        return None;
    }
    let (mime, signature_matches): (&'static str, fn(&[u8]) -> bool) = match source_mime.as_str() {
        "image/jpeg" | "image/jpg" => {
            ("image/jpeg", |bytes| bytes.starts_with(&[0xff, 0xd8, 0xff]))
        }
        "image/png" => ("image/png", |bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n")),
        "image/gif" => ("image/gif", |bytes| {
            bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
        }),
        "image/webp" => ("image/webp", |bytes| {
            bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
        }),
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    if bytes.is_empty()
        || bytes.len() > MAX_APPEARANCE_BACKGROUND_BYTES
        || !signature_matches(&bytes)
    {
        return None;
    }
    Some((mime, bytes))
}
