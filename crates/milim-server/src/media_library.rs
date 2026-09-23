use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use milim_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

const INDEX_VERSION: u32 = 1;
const MAX_MEDIA_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MediaLibraryMediaItem {
    pub url: String,
    pub source_url: String,
    pub kind: String,
    pub mime: Option<String>,
    pub requires_auth: bool,
    pub file_name: Option<String>,
    pub local_path: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MediaLibraryItem {
    pub id: String,
    pub provider_run_id: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub provider_id: String,
    pub provider: String,
    pub provider_kind: String,
    pub kind: String,
    pub model: String,
    pub prompt: String,
    pub input: Value,
    pub status: String,
    pub save_state: String,
    pub error: Option<String>,
    pub privacy: Value,
    pub urls: BTreeMap<String, String>,
    pub media: Vec<MediaLibraryMediaItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MediaLibraryPage {
    pub items: Vec<MediaLibraryItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug)]
pub struct NewMediaLibraryItem {
    pub provider_id: String,
    pub provider: String,
    pub provider_kind: String,
    pub kind: String,
    pub model: String,
    pub prompt: String,
    pub input: Value,
    pub privacy: Value,
}

#[derive(Clone, Debug)]
pub struct MediaLibraryUpdate {
    pub provider_run_id: String,
    pub status: String,
    pub urls: BTreeMap<String, String>,
    pub media: Vec<MediaLibraryMediaItem>,
}

#[derive(Clone, Debug)]
pub struct MediaDownloadSource {
    pub url: String,
    pub kind: String,
    pub mime: Option<String>,
    pub authorization: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct MediaLibraryIndex {
    version: u32,
    items: Vec<MediaLibraryItem>,
}

pub struct MediaLibrary {
    root: PathBuf,
    index_path: PathBuf,
    items: Mutex<Vec<MediaLibraryItem>>,
}

impl MediaLibrary {
    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("files"))?;
        let index_path = root.join("index.json");
        let items = if index_path.exists() {
            let index: MediaLibraryIndex = serde_json::from_slice(&std::fs::read(&index_path)?)?;
            if index.version != INDEX_VERSION {
                return Err(Error::Other(format!(
                    "unsupported media library index version {}",
                    index.version
                )));
            }
            index.items
        } else {
            Vec::new()
        };
        Ok(Self {
            root,
            index_path,
            items: Mutex::new(items),
        })
    }

    pub fn create(&self, new: NewMediaLibraryItem) -> Result<MediaLibraryItem> {
        let now = now_ms();
        let item = MediaLibraryItem {
            id: uuid::Uuid::new_v4().to_string(),
            provider_run_id: String::new(),
            created_at_ms: now,
            updated_at_ms: now,
            provider_id: new.provider_id,
            provider: new.provider,
            provider_kind: new.provider_kind,
            kind: new.kind,
            model: new.model,
            prompt: new.prompt,
            input: new.input,
            status: "submitted".to_string(),
            save_state: "running".to_string(),
            error: None,
            privacy: new.privacy,
            urls: BTreeMap::new(),
            media: Vec::new(),
        };
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.insert(0, item.clone());
        self.persist(&items)?;
        Ok(item)
    }

    pub fn get(&self, id: &str) -> Option<MediaLibraryItem> {
        self.items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|item| item.id == id)
            .cloned()
    }

    pub fn find_by_run(&self, provider_id: &str, run_id: &str) -> Option<MediaLibraryItem> {
        self.items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|item| item.provider_id == provider_id && item.provider_run_id == run_id)
            .cloned()
    }

    pub fn list(
        &self,
        query: &str,
        kind: Option<&str>,
        provider_id: Option<&str>,
        status: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> MediaLibraryPage {
        let query = query.trim().to_lowercase();
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let filtered = items.iter().filter(|item| {
            (query.is_empty()
                || item.prompt.to_lowercase().contains(&query)
                || item.model.to_lowercase().contains(&query)
                || item.provider.to_lowercase().contains(&query))
                && kind.is_none_or(|value| value == item.kind)
                && provider_id.is_none_or(|value| value == item.provider_id)
                && status.is_none_or(|value| value == library_status(item))
        });
        let mut started = cursor.is_none();
        let mut page = Vec::new();
        let mut has_more = false;
        for item in filtered {
            if !started {
                started = cursor.is_some_and(|value| value == item.id);
                continue;
            }
            if page.len() == limit {
                has_more = true;
                break;
            }
            page.push(item.clone());
        }
        let next_cursor = has_more
            .then(|| page.last().map(|item| item.id.clone()))
            .flatten();
        MediaLibraryPage {
            items: page,
            next_cursor,
        }
    }

    pub fn update(&self, id: &str, update: MediaLibraryUpdate) -> Result<MediaLibraryItem> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let item = items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::ModelNotFound(format!("media library item {id}")))?;
        let same_media = !update.media.is_empty()
            && update.media.len() == item.media.len()
            && update
                .media
                .iter()
                .zip(&item.media)
                .all(|(next, current)| next.source_url == current.source_url);
        let preserve_local_media =
            same_media && matches!(item.save_state.as_str(), "saving" | "ready");
        let next_save_state = if preserve_local_media {
            item.save_state.clone()
        } else if update.media.is_empty() {
            if provider_status_failed(&update.status) {
                "failed".to_string()
            } else {
                "running".to_string()
            }
        } else {
            "saving".to_string()
        };
        let media_unchanged = if preserve_local_media {
            true
        } else {
            item.media == update.media
        };
        if item.provider_run_id == update.provider_run_id
            && item.status == update.status
            && item.urls == update.urls
            && media_unchanged
            && item.save_state == next_save_state
            && item.error.is_none()
        {
            return Ok(item.clone());
        }
        item.provider_run_id = update.provider_run_id;
        item.status = update.status;
        item.urls = update.urls;
        if !preserve_local_media {
            item.media = update.media;
        }
        item.updated_at_ms = now_ms();
        if !preserve_local_media {
            item.save_state = next_save_state;
            item.error = None;
        }
        let result = item.clone();
        self.persist(&items)?;
        Ok(result)
    }

    pub fn fail(&self, id: &str, message: String) -> Result<()> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let item = items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::ModelNotFound(format!("media library item {id}")))?;
        item.status = "failed".to_string();
        item.save_state = "failed".to_string();
        item.error = Some(message);
        item.updated_at_ms = now_ms();
        self.persist(&items)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        validate_library_id(id)?;
        if self.get(id).is_none() {
            return Ok(false);
        }
        let asset_dir = self.root.join("files").join(id);
        if asset_dir.exists() {
            std::fs::remove_dir_all(&asset_dir)?;
        }
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = items.len();
        items.retain(|item| item.id != id);
        if items.len() == before {
            return Ok(false);
        }
        self.persist(&items)?;
        Ok(true)
    }

    pub fn content(&self, id: &str, index: usize) -> Result<(PathBuf, String)> {
        validate_library_id(id)?;
        let item = self
            .get(id)
            .ok_or_else(|| Error::ModelNotFound(format!("media library item {id}")))?;
        let media = item
            .media
            .get(index)
            .ok_or_else(|| Error::ModelNotFound(format!("media item {index}")))?;
        let file_name = media
            .file_name
            .as_deref()
            .ok_or_else(|| Error::ModelNotFound("local media file".to_string()))?;
        if Path::new(file_name).components().count() != 1 {
            return Err(Error::InvalidRequest("invalid media file name".to_string()));
        }
        let path = self.root.join("files").join(id).join(file_name);
        if !path.is_file() {
            return Err(Error::ModelNotFound("local media file".to_string()));
        }
        Ok((
            path,
            media
                .mime
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string()),
        ))
    }

    pub async fn save(
        &self,
        id: &str,
        sources: Vec<MediaDownloadSource>,
    ) -> Result<MediaLibraryItem> {
        validate_library_id(id)?;
        if sources.is_empty() {
            return self
                .get(id)
                .ok_or_else(|| Error::ModelNotFound(format!("media library item {id}")));
        }
        let asset_dir = self.root.join("files").join(id);
        tokio::fs::create_dir_all(&asset_dir).await?;
        let mut saved = Vec::with_capacity(sources.len());
        for (index, source) in sources.iter().enumerate() {
            match save_source(&asset_dir, index, source).await {
                Ok((file_name, mime, size_bytes)) => saved.push(MediaLibraryMediaItem {
                    url: format!("/media/library/{id}/content/{index}"),
                    source_url: source.url.clone(),
                    kind: source.kind.clone(),
                    mime: Some(mime),
                    requires_auth: true,
                    local_path: Some(asset_dir.join(&file_name).to_string_lossy().to_string()),
                    file_name: Some(file_name),
                    size_bytes: Some(size_bytes),
                }),
                Err(error) => {
                    let message = error.to_string();
                    self.mark_save_failed(id, message.clone())?;
                    return Err(Error::Other(message));
                }
            }
        }
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let item = items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::ModelNotFound(format!("media library item {id}")))?;
        item.media = saved;
        item.save_state = "ready".to_string();
        item.error = None;
        item.updated_at_ms = now_ms();
        let result = item.clone();
        self.persist(&items)?;
        Ok(result)
    }

    fn mark_save_failed(&self, id: &str, message: String) -> Result<()> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let item = items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::ModelNotFound(format!("media library item {id}")))?;
        item.save_state = "failed".to_string();
        item.error = Some(message);
        item.updated_at_ms = now_ms();
        self.persist(&items)
    }

    fn persist(&self, items: &[MediaLibraryItem]) -> Result<()> {
        // ponytail: a single JSON index keeps v1 recoverable; move to SQLite only if measured library size makes rewrites material.
        let data = serde_json::to_vec_pretty(&MediaLibraryIndex {
            version: INDEX_VERSION,
            items: items.to_vec(),
        })?;
        milim_tools::atomic_write(&self.index_path, &data)
    }
}

pub fn library_status(item: &MediaLibraryItem) -> &'static str {
    match item.save_state.as_str() {
        "ready" => "ready",
        "saving" => "saving",
        "failed" => "failed",
        _ => "running",
    }
}

fn provider_status_failed(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "failed" | "error" | "cancelled" | "canceled"
    )
}

fn validate_library_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| Error::InvalidRequest("invalid media library id".to_string()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

async fn save_source(
    asset_dir: &Path,
    index: usize,
    source: &MediaDownloadSource,
) -> Result<(String, String, u64)> {
    if source.url.starts_with("data:") {
        return save_data_url(asset_dir, index, source).await;
    }
    save_http_url(asset_dir, index, source).await
}

async fn save_data_url(
    asset_dir: &Path,
    index: usize,
    source: &MediaDownloadSource,
) -> Result<(String, String, u64)> {
    let (header, encoded) = source
        .url
        .split_once(',')
        .ok_or_else(|| Error::InvalidRequest("invalid media data URL".to_string()))?;
    let mime = header
        .strip_prefix("data:")
        .and_then(|value| value.strip_suffix(";base64"))
        .ok_or_else(|| Error::InvalidRequest("media data URL must be base64".to_string()))?;
    let extension = media_extension(mime)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| Error::InvalidRequest(format!("invalid media data: {error}")))?;
    if bytes.len() as u64 > MAX_MEDIA_BYTES {
        return Err(Error::InvalidRequest(
            "media file exceeds 1 GiB".to_string(),
        ));
    }
    let file_name = format!("{index}.{extension}");
    let part_path = asset_dir.join(format!("{index}.part"));
    let final_path = asset_dir.join(&file_name);
    tokio::fs::write(&part_path, &bytes).await?;
    if final_path.exists() {
        tokio::fs::remove_file(&final_path).await?;
    }
    tokio::fs::rename(&part_path, &final_path).await?;
    Ok((file_name, mime.to_string(), bytes.len() as u64))
}

async fn save_http_url(
    asset_dir: &Path,
    index: usize,
    source: &MediaDownloadSource,
) -> Result<(String, String, u64)> {
    let mut url = reqwest::Url::parse(&source.url)
        .map_err(|error| Error::InvalidRequest(format!("invalid media URL: {error}")))?;
    let authorization_origin = (
        url.scheme().to_string(),
        url.host_str().map(str::to_string),
        url.port_or_known_default(),
    );
    let mut response = None;
    for redirect in 0..=MAX_REDIRECTS {
        let client = public_http_client(&url).await?;
        let mut request = client.get(url.clone());
        let same_authorization_origin = url.scheme() == authorization_origin.0
            && url.host_str() == authorization_origin.1.as_deref()
            && url.port_or_known_default() == authorization_origin.2;
        if same_authorization_origin {
            if let Some(authorization) = &source.authorization {
                request = request.header(reqwest::header::AUTHORIZATION, authorization);
            }
        }
        let next = request
            .send()
            .await
            .map_err(|error| Error::Upstream(format!("media download failed: {error}")))?;
        if next.status().is_redirection() {
            if redirect == MAX_REDIRECTS {
                return Err(Error::Upstream("too many media redirects".to_string()));
            }
            let location = next
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| Error::Upstream("media redirect is missing Location".to_string()))?;
            url = url
                .join(location)
                .map_err(|error| Error::Upstream(format!("invalid media redirect: {error}")))?;
            continue;
        }
        response = Some(next);
        break;
    }
    let mut response =
        response.ok_or_else(|| Error::Upstream("media download failed".to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Upstream(format!(
            "media download returned HTTP {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MEDIA_BYTES)
    {
        return Err(Error::InvalidRequest(
            "media file exceeds 1 GiB".to_string(),
        ));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| source.mime.clone())
        .ok_or_else(|| {
            Error::InvalidRequest("media response has no supported content type".to_string())
        })?;
    let extension = media_extension(&mime)?;
    let file_name = format!("{index}.{extension}");
    let part_path = asset_dir.join(format!("{index}.part"));
    let final_path = asset_dir.join(&file_name);
    let mut file = tokio::fs::File::create(&part_path).await?;
    let transfer = async {
        let mut size = 0_u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| Error::Upstream(format!("media download failed: {error}")))?
        {
            size += chunk.len() as u64;
            if size > MAX_MEDIA_BYTES {
                return Err(Error::InvalidRequest(
                    "media file exceeds 1 GiB".to_string(),
                ));
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(size)
    }
    .await;
    drop(file);
    let size = match transfer {
        Ok(size) => size,
        Err(error) => {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(error);
        }
    };
    if final_path.exists() {
        tokio::fs::remove_file(&final_path).await?;
    }
    tokio::fs::rename(&part_path, &final_path).await?;
    Ok((file_name, mime, size))
}

fn media_extension(mime: &str) -> Result<&'static str> {
    match mime.to_ascii_lowercase().as_str() {
        "image/png" => Ok("png"),
        "image/jpeg" => Ok("jpg"),
        "image/webp" => Ok("webp"),
        "image/gif" => Ok("gif"),
        "video/mp4" => Ok("mp4"),
        "video/webm" => Ok("webm"),
        "video/quicktime" => Ok("mov"),
        "audio/mpeg" | "audio/mp3" => Ok("mp3"),
        "audio/wav" | "audio/x-wav" => Ok("wav"),
        "audio/flac" => Ok("flac"),
        "audio/ogg" => Ok("ogg"),
        "audio/mp4" | "audio/x-m4a" => Ok("m4a"),
        other => Err(Error::InvalidRequest(format!(
            "unsupported media content type {other}"
        ))),
    }
}

async fn public_http_client(url: &reqwest::Url) -> Result<reqwest::Client> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::InvalidRequest(
            "only http(s) media URLs are allowed".to_string(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::InvalidRequest("media URL must include a host".to_string()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::InvalidRequest("media URL must include a valid port".to_string()))?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| Error::Upstream(format!("media DNS lookup failed: {error}")))?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(Error::InvalidRequest(
            "private, local, and link-local media addresses are not allowed".to_string(),
        ));
    }
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(5 * 60))
        .redirect(reqwest::redirect::Policy::none());
    for address in addresses {
        builder = builder.resolve(host, address);
    }
    builder
        .build()
        .map_err(|error| Error::Other(format!("media HTTP client: {error}")))
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 240
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])))
        }
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            if octets[..10] == [0; 10] && octets[10..12] == [0xff, 0xff] {
                return is_public_ip(IpAddr::V4(Ipv4Addr::new(
                    octets[12], octets[13], octets[14], octets[15],
                )));
            }
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (octets[0] & 0xfe) == 0xfc
                || (octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("milim-media-library-{}", uuid::Uuid::new_v4()))
    }

    fn new_item() -> NewMediaLibraryItem {
        NewMediaLibraryItem {
            provider_id: "provider".to_string(),
            provider: "Provider".to_string(),
            provider_kind: "replicate".to_string(),
            kind: "image".to_string(),
            model: "owner/model".to_string(),
            prompt: "A blue chair".to_string(),
            input: serde_json::json!({"aspect_ratio":"1:1"}),
            privacy: serde_json::json!({"mode":"off","redacted":false}),
        }
    }

    #[tokio::test]
    async fn persists_filters_saves_and_deletes_media() {
        let root = temp_root();
        let library = MediaLibrary::open(root.clone()).unwrap();
        let item = library.create(new_item()).unwrap();
        library
            .update(
                &item.id,
                MediaLibraryUpdate {
                    provider_run_id: "run-1".to_string(),
                    status: "completed".to_string(),
                    urls: BTreeMap::new(),
                    media: vec![MediaLibraryMediaItem {
                        url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                        source_url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                        kind: "image".to_string(),
                        mime: Some("image/png".to_string()),
                        requires_auth: false,
                        file_name: None,
                        local_path: None,
                        size_bytes: None,
                    }],
                },
            )
            .unwrap();
        let saved = library
            .save(
                &item.id,
                vec![MediaDownloadSource {
                    url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                    kind: "image".to_string(),
                    mime: Some("image/png".to_string()),
                    authorization: None,
                }],
            )
            .await
            .unwrap();
        assert_eq!(saved.save_state, "ready");
        assert!(library.content(&item.id, 0).unwrap().0.is_file());
        assert_eq!(
            library
                .list("chair", Some("image"), None, Some("ready"), None, 20)
                .items
                .len(),
            1
        );

        let reopened = MediaLibrary::open(root.clone()).unwrap();
        assert_eq!(reopened.get(&item.id).unwrap().save_state, "ready");
        assert!(reopened.delete(&item.id).unwrap());
        assert!(reopened.get(&item.id).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn status_updates_keep_one_record_and_do_not_replace_saved_media() {
        let root = temp_root();
        let library = MediaLibrary::open(root.clone()).unwrap();
        let item = library.create(new_item()).unwrap();
        let remote = MediaLibraryMediaItem {
            url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
            source_url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
            kind: "image".to_string(),
            mime: Some("image/png".to_string()),
            requires_auth: false,
            file_name: None,
            local_path: None,
            size_bytes: None,
        };
        library
            .update(
                &item.id,
                MediaLibraryUpdate {
                    provider_run_id: "run-1".to_string(),
                    status: "completed".to_string(),
                    urls: BTreeMap::new(),
                    media: vec![remote.clone()],
                },
            )
            .unwrap();
        let saved = library
            .save(
                &item.id,
                vec![MediaDownloadSource {
                    url: remote.url.clone(),
                    kind: remote.kind.clone(),
                    mime: remote.mime.clone(),
                    authorization: None,
                }],
            )
            .await
            .unwrap();
        let local_url = saved.media[0].url.clone();

        let updated = library
            .update(
                &item.id,
                MediaLibraryUpdate {
                    provider_run_id: "run-1".to_string(),
                    status: "succeeded".to_string(),
                    urls: BTreeMap::new(),
                    media: vec![remote],
                },
            )
            .unwrap();

        assert_eq!(updated.id, item.id);
        assert_eq!(updated.save_state, "ready");
        assert_eq!(updated.media[0].url, local_url);
        assert_eq!(
            library.find_by_run("provider", "run-1").unwrap().id,
            item.id
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn paginates_and_rejects_unsafe_content_paths() {
        let root = temp_root();
        let library = MediaLibrary::open(root.clone()).unwrap();
        let first = library.create(new_item()).unwrap();
        let second = library.create(new_item()).unwrap();
        let page = library.list("", None, None, None, None, 1);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, second.id);
        let next = library.list("", None, None, None, page.next_cursor.as_deref(), 1);
        assert_eq!(next.items[0].id, first.id);

        {
            let mut items = library.items.lock().unwrap();
            let item = items.iter_mut().find(|item| item.id == first.id).unwrap();
            item.media.push(MediaLibraryMediaItem {
                url: "/media/library/unsafe/content/0".to_string(),
                source_url: "data:image/png;base64,AAAA".to_string(),
                kind: "image".to_string(),
                mime: Some("image/png".to_string()),
                requires_auth: true,
                file_name: Some("../outside.png".to_string()),
                local_path: None,
                size_bytes: Some(3),
            });
        }
        assert!(library.content(&first.id, 0).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_private_addresses_and_unsupported_media() {
        assert!(!is_public_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("::1".parse().unwrap()));
        assert!(media_extension("text/html").is_err());
    }

    #[test]
    fn rejects_unsafe_delete_ids_before_touching_files() {
        let root = temp_root();
        let library = MediaLibrary::open(root.clone()).unwrap();
        let sentinel = root.join("sentinel.txt");
        std::fs::write(&sentinel, b"keep").unwrap();
        assert!(library.delete("..").is_err());
        assert!(sentinel.is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
