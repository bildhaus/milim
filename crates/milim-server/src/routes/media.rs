use super::*;

// ----- Media generation -----

fn default_media_kind() -> String {
    "image".to_string()
}

#[derive(Deserialize)]
pub(crate) struct MediaGenerateRequest {
    #[serde(default)]
    provider_id: Option<String>,
    #[serde(default)]
    provider_kind: Option<crate::providers::ProviderKind>,
    model: String,
    #[serde(default = "default_media_kind")]
    kind: String,
    prompt: String,
    #[serde(default)]
    input: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
pub(crate) struct MediaModelsQuery {
    provider_id: String,
    #[serde(default = "default_media_kind")]
    kind: String,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    refresh: bool,
}

#[derive(Deserialize)]
pub(crate) struct MediaModelSchemaQuery {
    provider_id: String,
    model: String,
    #[serde(default = "default_media_kind")]
    kind: String,
    #[serde(default)]
    refresh: bool,
}

#[derive(Deserialize)]
pub(crate) struct MediaStatusQuery {
    provider_id: String,
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    response_url: Option<String>,
    #[serde(default)]
    status_url: Option<String>,
    #[serde(default = "default_media_kind")]
    kind: String,
}

#[derive(Deserialize)]
pub(crate) struct MediaContentQuery {
    provider_id: String,
    id: String,
    #[serde(default)]
    index: usize,
}

#[derive(Default, Deserialize)]
pub(crate) struct MediaLibraryQuery {
    #[serde(default, alias = "q")]
    query: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, alias = "provider_id")]
    provider: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MediaItem {
    url: String,
    kind: String,
    mime: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    requires_auth: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Serialize)]
pub(crate) struct MediaPrivacyInfo {
    mode: &'static str,
    redacted: bool,
    detections: usize,
    kinds: String,
}

#[derive(Debug, Serialize)]
struct MediaModelInfo {
    id: String,
    name: String,
    description: String,
    output_modalities: Vec<String>,
    supported_parameters: Vec<String>,
    default_parameters: Value,
    pricing: Value,
}

#[derive(Debug, Serialize)]
struct MediaControlOption {
    label: String,
    value: Value,
}

#[derive(Debug, Serialize)]
struct MediaSchemaControl {
    key: String,
    label: String,
    kind: String,
    path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<Vec<MediaControlOption>>,
    #[serde(rename = "default", skip_serializing_if = "Option::is_none")]
    default_value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
}

struct MediaCacheEntry {
    created_at: Instant,
    value: Value,
}

const MEDIA_METADATA_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
static MEDIA_METADATA_CACHE: OnceLock<Mutex<HashMap<String, MediaCacheEntry>>> = OnceLock::new();

fn media_metadata_cache() -> &'static Mutex<HashMap<String, MediaCacheEntry>> {
    MEDIA_METADATA_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read_media_cache(key: &str) -> Option<Value> {
    let cache = media_metadata_cache().lock().ok()?;
    let entry = cache.get(key)?;
    if entry.created_at.elapsed() <= MEDIA_METADATA_CACHE_TTL {
        Some(entry.value.clone())
    } else {
        None
    }
}

fn write_media_cache(key: String, value: &Value) {
    if let Ok(mut cache) = media_metadata_cache().lock() {
        cache.insert(
            key,
            MediaCacheEntry {
                created_at: Instant::now(),
                value: value.clone(),
            },
        );
    }
}

fn media_cache_response(mut value: Value, cached: bool) -> Value {
    if let Value::Object(map) = &mut value {
        map.insert("cached".to_string(), Value::Bool(cached));
        map.insert(
            "cache_ttl_seconds".to_string(),
            json!(MEDIA_METADATA_CACHE_TTL.as_secs()),
        );
    }
    value
}

fn media_models_cache_key(
    provider: &crate::providers::Provider,
    kind: &str,
    query: &str,
) -> String {
    format!(
        "models:{}:{:?}:{}:{}",
        provider.id,
        provider.kind,
        provider.base_url.trim_end_matches('/'),
        kind
    ) + ":"
        + query
}

fn media_schema_cache_key(
    provider: &crate::providers::Provider,
    model: &str,
    kind: &str,
) -> String {
    format!(
        "schema:{}:{}:{}:{}",
        provider.id,
        provider.base_url.trim_end_matches('/'),
        kind,
        model
    )
}

fn validate_media_kind(kind: &str) -> Result<&str, ApiError> {
    match kind.trim() {
        "image" => Ok("image"),
        "video" => Ok("video"),
        "music" => Ok("music"),
        other => Err(ApiError(Error::InvalidRequest(format!(
            "unsupported media kind '{other}'; expected image, video, or music"
        )))),
    }
}

/// `GET /media/models` - list provider models that can produce the requested
/// media kind.
pub(crate) async fn media_models(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaModelsQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    let key = media_provider_key(&provider)?;
    let kind = validate_media_kind(&query.kind)?;
    let model_query = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(kind);
    let cache_key = media_models_cache_key(&provider, kind, model_query);
    if !query.refresh {
        if let Some(cached) = read_media_cache(&cache_key) {
            return Ok(Json(media_cache_response(cached, true)).into_response());
        }
    }
    let models = match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            let upstream =
                call_replicate_media_models(&provider.base_url, &key, model_query).await?;
            media_models_from_replicate(&upstream, kind)
        }
        crate::providers::ProviderKind::Fal => {
            let upstream =
                call_fal_media_models(&provider.base_url, &key, kind, model_query).await?;
            media_models_from_fal(&upstream, kind)
        }
        crate::providers::ProviderKind::OpenAiCompatible if is_openrouter_provider(&provider) => {
            let upstream = call_openrouter_media_models(&provider.base_url, &key, kind).await?;
            filter_media_models(media_models_from_openrouter(&upstream, kind), model_query)
        }
        _ => {
            return Err(ApiError(Error::InvalidRequest(
                "selected provider does not expose media model metadata".to_string(),
            )))
        }
    };
    let response = json!({ "models": models });
    write_media_cache(cache_key, &response);
    Ok(Json(media_cache_response(response, false)).into_response())
}

/// `GET /media/model-schema` - return normalized UI controls for a selected
/// media model.
pub(crate) async fn media_model_schema(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaModelSchemaQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let model = query.model.trim();
    if model.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media model is required".to_string(),
        )));
    }
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    let kind = validate_media_kind(&query.kind)?;
    let key = media_provider_key(&provider)?;
    let cache_key = media_schema_cache_key(&provider, model, kind);
    if !query.refresh {
        if let Some(cached) = read_media_cache(&cache_key) {
            return Ok(Json(media_cache_response(cached, true)).into_response());
        }
    }
    let (supported, controls, upstream) = match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            let upstream = call_replicate_model_schema(&provider.base_url, &key, model).await?;
            let (supported, controls) = replicate_schema_controls(&upstream)?;
            (supported, controls, upstream)
        }
        crate::providers::ProviderKind::Fal => {
            let upstream = call_fal_model_schema(&provider.base_url, &key, model).await?;
            let (supported, controls) = fal_schema_controls(&upstream)?;
            (supported, controls, upstream)
        }
        crate::providers::ProviderKind::OpenAiCompatible if is_openrouter_provider(&provider) => {
            let upstream = if kind == "video" {
                call_openrouter_video_models(&provider.base_url, &key).await?
            } else {
                call_openrouter_model_endpoints(&provider.base_url, &key, model).await?
            };
            let supported = if kind == "video" {
                openrouter_video_supported_parameters(&upstream, model)
            } else {
                openrouter_schema_supported_parameters(&upstream)
            };
            let controls = match kind {
                "video" => openrouter_video_schema_controls(&upstream, model, &supported),
                "music" => openrouter_music_schema_controls(&supported),
                _ => media_schema_controls(&supported),
            };
            (supported, controls, upstream)
        }
        _ => {
            return Err(ApiError(Error::InvalidRequest(
                "selected provider does not expose media model metadata".to_string(),
            )))
        }
    };

    let response = json!({
        "model": model,
        "provider_id": provider.id,
        "provider": provider.name,
        "kind": kind,
        "supported_parameters": supported,
        "controls": controls,
        "raw": upstream,
    });
    write_media_cache(cache_key, &response);
    Ok(Json(media_cache_response(response, false)).into_response())
}

/// `GET /media/status` - fetch the latest status for an asynchronous media run.
pub(crate) async fn media_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaStatusQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let id = query.id.trim();
    if id.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media generation id is required".to_string(),
        )));
    }
    let kind = validate_media_kind(&query.kind)?;
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    let key = media_provider_key(&provider)?;
    let upstream = fetch_media_status_upstream(&provider, &key, &query, kind).await?;
    let media = if is_openrouter_provider(&provider) && kind == "video" {
        openrouter_video_media_items(&provider.id, id, &upstream)
    } else {
        media_items_from_result(&upstream, kind)
    };
    let urls = media_urls_from_result(&provider.kind, &upstream);
    let status = upstream
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("submitted")
        .to_string();
    let library_item = update_media_library(&st, None, &provider, id, &status, &urls, &media);
    Ok(Json(json!({
        "id": id,
        "object": "media.status",
        "kind": kind,
        "provider_id": provider.id,
        "provider": provider.name,
        "provider_kind": provider.kind,
        "model": query.model.unwrap_or_default(),
        "status": status,
        "output": upstream.get("output").cloned().unwrap_or(Value::Null),
        "media": media,
        "urls": urls,
        "library_id": library_item.as_ref().map(|item| item.id.as_str()),
        "save_state": library_item.as_ref().map(|item| item.save_state.as_str()),
        "raw": upstream,
    }))
    .into_response())
}

async fn fetch_media_status_upstream(
    provider: &crate::providers::Provider,
    key: &str,
    query: &MediaStatusQuery,
    kind: &str,
) -> Result<Value, ApiError> {
    let id = query.id.trim();
    match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            call_replicate_media_status(&provider.base_url, key, id).await
        }
        crate::providers::ProviderKind::Fal => {
            call_fal_media_status(
                &provider.base_url,
                key,
                id,
                query.model.as_deref(),
                query.response_url.as_deref(),
                query.status_url.as_deref(),
            )
            .await
        }
        crate::providers::ProviderKind::OpenAiCompatible
            if is_openrouter_provider(provider) && kind == "video" =>
        {
            call_openrouter_video_status(&provider.base_url, key, id).await
        }
        _ => Err(ApiError(Error::InvalidRequest(
            "selected provider does not expose media run status".to_string(),
        ))),
    }
}

/// `GET /media/content` - proxy authenticated OpenRouter video bytes without
/// exposing the provider credential to the desktop webview.
pub(crate) async fn media_content(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaContentQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    if query.index > 16 {
        return Err(ApiError(Error::InvalidRequest(
            "media content index is out of range".to_string(),
        )));
    }
    let id = query.id.trim();
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ApiError(Error::InvalidRequest(
            "media content id is invalid".to_string(),
        )));
    }
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    if !is_openrouter_provider(&provider) {
        return Err(ApiError(Error::InvalidRequest(
            "authenticated media content is only available for OpenRouter video".to_string(),
        )));
    }
    let key = media_provider_key(&provider)?;
    let url = format!(
        "{}/videos/{id}/content",
        provider.base_url.trim_end_matches('/')
    );
    let response = openrouter_request(reqwest::Client::new().get(url), &key)
        .query(&[("index", query.index)])
        .send()
        .await
        .map_err(|error| {
            ApiError(Error::Upstream(format!(
                "OpenRouter video content request failed: {error}"
            )))
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ApiError(Error::Upstream(format!(
            "OpenRouter video content returned HTTP {status}: {body}"
        ))));
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("video/mp4"));
    let content_length = response.headers().get(CONTENT_LENGTH).cloned();
    let mut proxied = Response::new(Body::from_stream(response.bytes_stream()));
    proxied.headers_mut().insert(CONTENT_TYPE, content_type);
    proxied.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    if let Some(content_length) = content_length {
        proxied.headers_mut().insert(CONTENT_LENGTH, content_length);
    }
    Ok(proxied)
}

/// `GET /media/library` - list durable locally saved media runs.
pub(crate) async fn media_library_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaLibraryQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    let limit = query.limit.unwrap_or(40).clamp(1, 100);
    Ok(Json(library.list(
        &query.query,
        query.kind.as_deref().filter(|value| !value.is_empty()),
        query.provider.as_deref().filter(|value| !value.is_empty()),
        query.status.as_deref().filter(|value| !value.is_empty()),
        query.cursor.as_deref().filter(|value| !value.is_empty()),
        limit,
    ))
    .into_response())
}

/// `GET /media/library/{id}/content/{index}` - stream one authenticated local file.
pub(crate) async fn media_library_content(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path((id, index)): Path<(String, usize)>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    let (path, mime) = library.content(&id, index).map_err(ApiError)?;
    let size = tokio::fs::metadata(&path).await.map_err(Error::from)?.len();
    let stream = async_stream::stream! {
        let mut file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(error) => {
                yield Err::<Bytes, std::io::Error>(error);
                return;
            }
        };
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = match file.read(&mut buffer).await {
                Ok(read) => read,
                Err(error) => {
                    yield Err::<Bytes, std::io::Error>(error);
                    return;
                }
            };
            if read == 0 {
                break;
            }
            yield Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer[..read]));
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    if let Ok(value) = HeaderValue::from_str(&size.to_string()) {
        response.headers_mut().insert(CONTENT_LENGTH, value);
    }
    Ok(response)
}

/// `DELETE /media/library/{id}` - permanently remove a local media run and files.
pub(crate) async fn media_library_delete(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    Ok(Json(json!({ "deleted": library.delete(&id).map_err(ApiError)? })).into_response())
}

/// `POST /media/library/{id}/refresh` - refresh a pending run or retry its local save.
pub(crate) async fn media_library_refresh(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    let item = library
        .get(&id)
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("media library item {id}"))))?;
    if matches!(item.save_state.as_str(), "saving" | "ready") {
        return Ok(Json(item).into_response());
    }
    let provider = select_media_provider(&st, Some(&item.provider_id), None)
        .await
        .map_err(ApiError)?;
    let key = media_provider_key(&provider)?;
    if item.save_state == "failed" && !item.media.is_empty() {
        let updated = library
            .update(
                &id,
                MediaLibraryUpdate {
                    provider_run_id: item.provider_run_id.clone(),
                    status: item.status.clone(),
                    urls: item.urls.clone(),
                    media: item.media.clone(),
                },
            )
            .map_err(ApiError)?;
        spawn_media_save(library.clone(), &provider, &key, &updated);
        return Ok(Json(updated).into_response());
    }
    if item.provider_run_id.is_empty() {
        return Ok(Json(item).into_response());
    }
    let query = MediaStatusQuery {
        provider_id: item.provider_id.clone(),
        id: item.provider_run_id.clone(),
        model: Some(item.model.clone()),
        response_url: item.urls.get("response").cloned(),
        status_url: item.urls.get("status").cloned(),
        kind: item.kind.clone(),
    };
    let upstream = fetch_media_status_upstream(&provider, &key, &query, &item.kind).await?;
    let media = if is_openrouter_provider(&provider) && item.kind == "video" {
        openrouter_video_media_items(&provider.id, &item.provider_run_id, &upstream)
    } else {
        media_items_from_result(&upstream, &item.kind)
    };
    let urls = media_urls_from_result(&provider.kind, &upstream);
    let status = upstream
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("submitted");
    let updated = update_media_library(
        &st,
        Some(&id),
        &provider,
        &item.provider_run_id,
        status,
        &urls,
        &media,
    )
    .ok_or_else(media_library_unavailable)?;
    Ok(Json(updated).into_response())
}

fn media_library_unavailable() -> ApiError {
    ApiError(Error::InvalidRequest(
        "media library is not enabled".to_string(),
    ))
}

/// `POST /media/generate` - submit a prompt to an enabled remote media
/// provider. The endpoint is intentionally provider-neutral at the UI boundary:
/// callers pass a model id, a prompt, and optional model-specific input fields.
pub(crate) async fn media_generate(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MediaGenerateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let model = req.model.trim();
    if model.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media model is required".to_string(),
        )));
    }
    let original_prompt = req.prompt.trim();
    if original_prompt.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media prompt is required".to_string(),
        )));
    }
    let kind = validate_media_kind(&req.kind)?.to_string();

    let provider = select_media_provider(&st, req.provider_id.as_deref(), req.provider_kind)
        .await
        .map_err(ApiError)?;
    let key = provider
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(format!(
                "{} requires an API key",
                provider.name
            )))
        })?
        .to_string();
    let (prompt, privacy) = media_prompt_for_remote(&st, original_prompt).map_err(ApiError)?;

    let original_input = Value::Object(req.input.clone());
    let mut input = req.input;
    input.insert("prompt".to_string(), Value::String(prompt));

    let library_item = st
        .media_library
        .as_ref()
        .map(|library| {
            library.create(NewMediaLibraryItem {
                provider_id: provider.id.clone(),
                provider: provider.name.clone(),
                provider_kind: provider_kind_name(provider.kind).to_string(),
                kind: kind.clone(),
                model: model.to_string(),
                prompt: original_prompt.to_string(),
                input: original_input,
                privacy: serde_json::to_value(&privacy).unwrap_or(Value::Null),
            })
        })
        .transpose()
        .map_err(ApiError)?;

    let upstream_result = match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            call_replicate_media(&provider.base_url, &key, model, input).await
        }
        crate::providers::ProviderKind::Fal => {
            call_fal_media(&provider.base_url, &key, model, input).await
        }
        crate::providers::ProviderKind::OpenAiCompatible if is_openrouter_provider(&provider) => {
            match kind.as_str() {
                "video" => {
                    call_openrouter_video_media(&provider.base_url, &key, model, input).await
                }
                "music" => {
                    call_openrouter_music_media(&provider.base_url, &key, model, input).await
                }
                _ => call_openrouter_image_media(&provider.base_url, &key, model, input).await,
            }
        }
        _ => Err(ApiError(Error::InvalidRequest(
            "selected provider is not a media provider".to_string(),
        ))),
    };
    let upstream = match upstream_result {
        Ok(upstream) => upstream,
        Err(error) => {
            if let (Some(library), Some(item)) = (&st.media_library, &library_item) {
                let _ = library.fail(&item.id, error.0.to_string());
            }
            return Err(error);
        }
    };
    let media = media_items_from_result(&upstream, &kind);
    let urls = media_urls_from_result(&provider.kind, &upstream);
    let id = upstream
        .get("id")
        .or_else(|| upstream.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let status = upstream
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if is_openrouter_provider(&provider) && kind != "video" {
            "completed"
        } else {
            "submitted"
        })
        .to_string();
    let library_item = update_media_library(
        &st,
        library_item.as_ref().map(|item| item.id.as_str()),
        &provider,
        &id,
        &status,
        &urls,
        &media,
    )
    .or(library_item);

    Ok(Json(json!({
        "id": id,
        "object": "media.generation",
        "provider_id": provider.id,
        "provider": provider.name,
        "provider_kind": provider.kind,
        "kind": kind,
        "model": model,
        "status": status,
        "output": upstream.get("output").cloned().unwrap_or(Value::Null),
        "media": media,
        "urls": urls,
        "library_id": library_item.as_ref().map(|item| item.id.as_str()),
        "save_state": library_item.as_ref().map(|item| item.save_state.as_str()),
        "privacy": privacy,
        "raw": upstream,
    }))
    .into_response())
}

fn provider_kind_name(kind: crate::providers::ProviderKind) -> &'static str {
    match kind {
        crate::providers::ProviderKind::OpenAiCompatible => "openai_compatible",
        crate::providers::ProviderKind::Anthropic => "anthropic",
        crate::providers::ProviderKind::Gemini => "gemini",
        crate::providers::ProviderKind::Replicate => "replicate",
        crate::providers::ProviderKind::Fal => "fal",
    }
}

fn update_media_library(
    st: &AppState,
    library_id: Option<&str>,
    provider: &crate::providers::Provider,
    provider_run_id: &str,
    status: &str,
    urls: &Value,
    media: &[MediaItem],
) -> Option<MediaLibraryItem> {
    let library = st.media_library.as_ref()?;
    let id = library_id.map(str::to_string).or_else(|| {
        library
            .find_by_run(&provider.id, provider_run_id)
            .map(|item| item.id)
    })?;
    let already_saving_or_ready = library
        .get(&id)
        .is_some_and(|item| matches!(item.save_state.as_str(), "saving" | "ready"));
    let item = library
        .update(
            &id,
            MediaLibraryUpdate {
                provider_run_id: provider_run_id.to_string(),
                status: status.to_string(),
                urls: media_urls_map(urls),
                media: media
                    .iter()
                    .map(|item| MediaLibraryMediaItem {
                        url: item.url.clone(),
                        source_url: item.url.clone(),
                        kind: item.kind.clone(),
                        mime: item.mime.clone(),
                        requires_auth: item.requires_auth,
                        file_name: None,
                        local_path: None,
                        size_bytes: None,
                    })
                    .collect(),
            },
        )
        .ok()?;
    if !already_saving_or_ready && !item.media.is_empty() {
        if let Ok(key) = media_provider_key(provider) {
            spawn_media_save(library.clone(), provider, &key, &item);
        }
    }
    Some(item)
}

fn media_urls_map(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn spawn_media_save(
    library: Arc<crate::media_library::MediaLibrary>,
    provider: &crate::providers::Provider,
    key: &str,
    item: &MediaLibraryItem,
) {
    let sources = item
        .media
        .iter()
        .enumerate()
        .map(|(index, media)| {
            let authenticated = media.requires_auth && is_openrouter_provider(provider);
            MediaDownloadSource {
                url: if authenticated {
                    format!(
                        "{}/videos/{}/content?index={index}",
                        provider.base_url.trim_end_matches('/'),
                        item.provider_run_id
                    )
                } else {
                    media.source_url.clone()
                },
                kind: media.kind.clone(),
                mime: media.mime.clone(),
                authorization: authenticated.then(|| format!("Bearer {key}")),
            }
        })
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return;
    }
    let id = item.id.clone();
    tokio::spawn(async move {
        if let Err(error) = library.save(&id, sources).await {
            tracing::warn!(media_library_id = %id, "failed to save generated media: {error}");
        }
    });
}

async fn select_media_provider(
    st: &AppState,
    provider_id: Option<&str>,
    provider_kind: Option<crate::providers::ProviderKind>,
) -> milim_core::Result<crate::providers::Provider> {
    let reg = st
        .providers
        .as_ref()
        .ok_or_else(|| Error::InvalidRequest("providers are not enabled".to_string()))?;
    let providers = reg.list().await;
    let selected = providers.into_iter().find(|provider| {
        if !provider.enabled {
            return false;
        }
        if provider.kind.is_chat() && !is_openrouter_provider(provider) {
            return false;
        }
        if let Some(id) = provider_id {
            return provider.id == id;
        }
        if let Some(kind) = provider_kind {
            return provider.kind == kind;
        }
        true
    });
    selected.ok_or_else(|| {
        Error::InvalidRequest(
            "no enabled Replicate, fal, or OpenRouter media provider matched the request"
                .to_string(),
        )
    })
}

fn is_openrouter_provider(provider: &crate::providers::Provider) -> bool {
    if provider.kind != crate::providers::ProviderKind::OpenAiCompatible {
        return false;
    }
    provider.name.eq_ignore_ascii_case("openrouter")
        || provider
            .base_url
            .to_ascii_lowercase()
            .contains("openrouter.ai/")
}

fn media_provider_key(provider: &crate::providers::Provider) -> Result<String, ApiError> {
    provider
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(format!(
                "{} requires an API key",
                provider.name
            )))
        })
}

fn media_prompt_for_remote(
    st: &AppState,
    prompt: &str,
) -> milim_core::Result<(String, MediaPrivacyInfo)> {
    match st.privacy.mode() {
        PrivacyMode::Off => Ok((
            prompt.to_string(),
            MediaPrivacyInfo {
                mode: "off",
                redacted: false,
                detections: 0,
                kinds: String::new(),
            },
        )),
        PrivacyMode::Block => {
            let detections = st.privacy.scan_text(prompt);
            if detections.is_empty() {
                Ok((
                    prompt.to_string(),
                    MediaPrivacyInfo {
                        mode: "block",
                        redacted: false,
                        detections: 0,
                        kinds: String::new(),
                    },
                ))
            } else {
                Err(Error::InvalidRequest(format!(
                    "blocked by the privacy gate: media prompt contains {} ({} item(s)). Switch the gate to Redact or Off to send this to a remote media provider.",
                    kinds_summary(&detections),
                    detections.len()
                )))
            }
        }
        PrivacyMode::Redact => {
            let detections = st.privacy.scan_text(prompt);
            let redaction = st.privacy.redact_text(prompt);
            Ok((
                redaction.text,
                MediaPrivacyInfo {
                    mode: "redact",
                    redacted: !redaction.map.is_empty(),
                    detections: detections.len(),
                    kinds: kinds_summary(&detections),
                },
            ))
        }
    }
}

async fn call_replicate_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let url = format!("{}/predictions", base_url.trim_end_matches('/'));
    let mut body = serde_json::Map::new();
    if is_replicate_version_id(model) {
        body.insert("version".to_string(), Value::String(model.to_string()));
    } else {
        body.insert("model".to_string(), Value::String(model.to_string()));
    }
    body.insert("input".to_string(), Value::Object(input));
    post_media_json(
        reqwest::Client::new()
            .post(url)
            .bearer_auth(api_key)
            .header("Prefer", "wait")
            .json(&Value::Object(body)),
    )
    .await
}

async fn call_replicate_media_status(
    base_url: &str,
    api_key: &str,
    id: &str,
) -> Result<Value, ApiError> {
    let url = format!(
        "{}/predictions/{}",
        base_url.trim_end_matches('/'),
        id.trim_start_matches('/')
    );
    get_media_json(reqwest::Client::new().get(url).bearer_auth(api_key)).await
}

async fn call_fal_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        model.trim_start_matches('/')
    );
    post_media_json(
        reqwest::Client::new()
            .post(url)
            .header("Authorization", format!("Key {api_key}"))
            .json(&Value::Object(input)),
    )
    .await
}

async fn call_fal_media_status(
    base_url: &str,
    api_key: &str,
    id: &str,
    model: Option<&str>,
    response_url: Option<&str>,
    status_url: Option<&str>,
) -> Result<Value, ApiError> {
    let url = response_url
        .or(status_url)
        .map(|url| validate_media_status_url(base_url, url))
        .transpose()?
        .unwrap_or_else(|| {
            let model = model.unwrap_or_default().trim_matches('/');
            format!(
                "{}/{}/requests/{}",
                base_url.trim_end_matches('/'),
                model,
                id.trim_start_matches('/')
            )
        });
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Key {api_key}")),
    )
    .await
}

async fn call_openrouter_image_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    mut input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let prompt = input
        .remove("prompt")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    // The media endpoint returns image artifacts; accepting a saved/custom
    // text co-output here breaks image-only OpenRouter endpoints such as Flux.
    let _ = input.remove("modalities");
    let modalities = json!(["image"]);
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut body = input;
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert(
        "messages".to_string(),
        json!([{ "role": "user", "content": prompt }]),
    );
    body.insert("modalities".to_string(), modalities);
    body.insert("stream".to_string(), Value::Bool(false));

    post_media_json(
        openrouter_request(reqwest::Client::new().post(url), api_key).json(&Value::Object(body)),
    )
    .await
}

async fn call_openrouter_video_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    mut input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let prompt = input
        .remove("prompt")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    input.remove("modalities");
    input.remove("stream");
    input.insert("model".to_string(), Value::String(model.to_string()));
    input.insert("prompt".to_string(), Value::String(prompt));
    let url = format!("{}/videos", base_url.trim_end_matches('/'));
    post_media_json(
        openrouter_request(reqwest::Client::new().post(url), api_key).json(&Value::Object(input)),
    )
    .await
}

async fn call_openrouter_video_status(
    base_url: &str,
    api_key: &str,
    id: &str,
) -> Result<Value, ApiError> {
    let url = format!(
        "{}/videos/{}",
        base_url.trim_end_matches('/'),
        id.trim_start_matches('/')
    );
    get_media_json(openrouter_request(reqwest::Client::new().get(url), api_key)).await
}

async fn call_openrouter_music_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    mut input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let prompt = input
        .remove("prompt")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    input.remove("modalities");
    input.insert("model".to_string(), Value::String(model.to_string()));
    input.insert(
        "messages".to_string(),
        json!([{ "role": "user", "content": prompt }]),
    );
    input.insert("modalities".to_string(), json!(["text", "audio"]));
    input.insert("stream".to_string(), Value::Bool(true));
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let response = openrouter_request(reqwest::Client::new().post(url), api_key)
        .json(&Value::Object(input))
        .send()
        .await
        .map_err(|error| {
            ApiError(Error::Upstream(format!(
                "OpenRouter music request failed: {error}"
            )))
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        ApiError(Error::Upstream(format!(
            "OpenRouter music response failed: {error}"
        )))
    })?;
    if !status.is_success() {
        return Err(ApiError(Error::Upstream(format!(
            "OpenRouter music returned HTTP {status}: {body}"
        ))));
    }
    openrouter_music_from_sse(&body)
}

fn openrouter_music_from_sse(body: &str) -> Result<Value, ApiError> {
    let mut id = String::new();
    let mut audio = String::new();
    let mut text = String::new();
    let mut transcript = String::new();
    for line in body.lines() {
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let chunk: Value = serde_json::from_str(data).map_err(|error| {
            ApiError(Error::Upstream(format!(
                "OpenRouter music stream returned invalid JSON: {error}"
            )))
        })?;
        if id.is_empty() {
            id = chunk
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
        let Some(delta) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
        else {
            continue;
        };
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            text.push_str(content);
        }
        if let Some(audio_delta) = delta.get("audio") {
            if let Some(data) = audio_delta.get("data").and_then(Value::as_str) {
                audio.push_str(data);
            }
            if let Some(value) = audio_delta.get("transcript").and_then(Value::as_str) {
                transcript.push_str(value);
            }
        }
    }
    if audio.is_empty() {
        return Err(ApiError(Error::Upstream(
            "OpenRouter music stream completed without audio data".to_string(),
        )));
    }
    Ok(json!({
        "id": id,
        "status": "completed",
        "output": {
            "text": text,
            "transcript": transcript,
        },
        "audio": {
            "url": format!("data:audio/mpeg;base64,{audio}"),
            "content_type": "audio/mpeg"
        }
    }))
}

fn openrouter_request(builder: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder {
    builder
        .bearer_auth(api_key)
        .header("HTTP-Referer", "https://milim.ai/")
        .header("X-OpenRouter-Title", "milim")
}

async fn call_openrouter_media_models(
    base_url: &str,
    api_key: &str,
    kind: &str,
) -> Result<Value, ApiError> {
    if kind == "video" {
        return call_openrouter_video_models(base_url, api_key).await;
    }
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let output_kind = if kind == "music" { "audio" } else { kind };
    get_media_json(
        openrouter_request(reqwest::Client::new().get(url), api_key)
            .query(&[("output_modalities", output_kind)]),
    )
    .await
}

async fn call_openrouter_video_models(base_url: &str, api_key: &str) -> Result<Value, ApiError> {
    let url = format!("{}/videos/models", base_url.trim_end_matches('/'));
    get_media_json(openrouter_request(reqwest::Client::new().get(url), api_key)).await
}

async fn call_openrouter_model_endpoints(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, ApiError> {
    let (author, slug) = model.split_once('/').ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "OpenRouter media model id must be in author/slug form".to_string(),
        ))
    })?;
    if author.is_empty() || slug.is_empty() || slug.contains('/') {
        return Err(ApiError(Error::InvalidRequest(
            "OpenRouter media model id must be in author/slug form".to_string(),
        )));
    }
    let url = format!(
        "{}/models/{author}/{slug}/endpoints",
        base_url.trim_end_matches('/')
    );
    get_media_json(openrouter_request(reqwest::Client::new().get(url), api_key)).await
}

async fn call_replicate_media_models(
    base_url: &str,
    api_key: &str,
    kind: &str,
) -> Result<Value, ApiError> {
    let url = format!("{}/search", base_url.trim_end_matches('/'));
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .bearer_auth(api_key)
            .query(&[("query", kind), ("limit", "50")]),
    )
    .await
}

async fn call_replicate_model_schema(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, ApiError> {
    let (owner, name) = model.split_once('/').ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "Replicate media model id must be in owner/name form".to_string(),
        ))
    })?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(ApiError(Error::InvalidRequest(
            "Replicate media model id must be in owner/name form".to_string(),
        )));
    }
    let url = format!("{}/models/{owner}/{name}", base_url.trim_end_matches('/'));
    get_media_json(reqwest::Client::new().get(url).bearer_auth(api_key)).await
}

async fn call_fal_media_models(
    base_url: &str,
    api_key: &str,
    kind: &str,
    query: &str,
) -> Result<Value, ApiError> {
    let category = match kind {
        "image" => "text-to-image",
        "music" => "text-to-audio",
        _ => kind,
    };
    let url = format!("{}/models", fal_platform_base_url(base_url));
    let mut params = vec![
        ("limit", "50"),
        ("category", category),
        ("status", "active"),
    ];
    if (query != kind || kind == "music") && !query.trim().is_empty() {
        params.push(("q", query));
    }
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Key {api_key}"))
            .query(&params),
    )
    .await
}

async fn call_fal_model_schema(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, ApiError> {
    let url = format!("{}/models", fal_platform_base_url(base_url));
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Key {api_key}"))
            .query(&[("endpoint_id", model), ("expand", "openapi-3.0")]),
    )
    .await
}

fn fal_platform_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("127.0.0.1")
        || lower.contains("localhost")
        || lower.contains("[::1]")
        || lower.contains("api.fal.ai/")
    {
        trimmed.to_string()
    } else {
        "https://api.fal.ai/v1".to_string()
    }
}

fn validate_media_status_url(base_url: &str, url: &str) -> Result<String, ApiError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        ApiError(Error::InvalidRequest(
            "media status URL is not a valid URL".to_string(),
        ))
    })?;
    let allowed = reqwest::Url::parse(base_url)
        .ok()
        .map(|base| parsed.scheme() == base.scheme() && parsed.host_str() == base.host_str())
        .unwrap_or(false)
        || matches!(
            parsed.host_str(),
            Some("queue.fal.run") | Some("fal.run") | Some("api.replicate.com")
        );
    if !allowed {
        return Err(ApiError(Error::InvalidRequest(
            "media status URL must match the selected provider".to_string(),
        )));
    }
    Ok(url.to_string())
}

async fn get_media_json(builder: reqwest::RequestBuilder) -> Result<Value, ApiError> {
    let response = builder.send().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider request failed: {e}"
        )))
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider response failed: {e}"
        )))
    })?;
    if !status.is_success() {
        return Err(ApiError(Error::Upstream(format!(
            "media provider returned HTTP {status}: {body}"
        ))));
    }
    serde_json::from_str(&body).map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider returned invalid JSON: {e}"
        )))
    })
}

fn media_models_from_openrouter(value: &Value, kind: &str) -> Vec<MediaModelInfo> {
    value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let upstream_modalities = openrouter_output_modalities(model);
            let descriptor = media_model_descriptor(model);
            let matches = match kind {
                // The dedicated /videos/models catalog is already scoped to
                // generation models and does not consistently repeat a video
                // output modality on every entry.
                "video" => true,
                "music" => {
                    upstream_modalities.iter().any(|item| item == "audio")
                        && openrouter_input_modalities(model)
                            .iter()
                            .all(|item| item != "audio")
                        && descriptor_is_music(&descriptor)
                }
                _ => upstream_modalities.iter().any(|item| item == kind),
            };
            if !matches {
                return None;
            }
            let id = model.get("id").and_then(Value::as_str)?.to_string();
            let output_modalities = vec![kind.to_string()];
            Some(MediaModelInfo {
                name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string(),
                description: model
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                output_modalities,
                supported_parameters: openrouter_video_model_parameters(model),
                default_parameters: model
                    .get("default_parameters")
                    .cloned()
                    .unwrap_or(Value::Null),
                pricing: model
                    .get("pricing")
                    .or_else(|| model.get("pricing_skus"))
                    .cloned()
                    .unwrap_or(Value::Null),
                id,
            })
        })
        .collect()
}

fn media_models_from_replicate(value: &Value, kind: &str) -> Vec<MediaModelInfo> {
    value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let model = item.get("model").unwrap_or(item);
            let openapi = replicate_model_openapi_schema(model);
            let id = replicate_model_id(model)?;
            let descriptor = format!(
                "{} {} {}",
                id,
                model
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                item.get("metadata").cloned().unwrap_or(Value::Null)
            )
            .to_ascii_lowercase();
            let output_schema =
                openapi.and_then(|schema| openapi_schema_by_name(schema, "Output"))?;
            if !schema_outputs_media_url(output_schema, openapi?, 0)
                || !descriptor_matches_media_kind(&descriptor, kind)
            {
                return None;
            }
            let output_modalities = vec![kind.to_string()];
            let controls = openapi
                .and_then(|openapi| {
                    openapi_input_schema(openapi)
                        .map(|schema| json_schema_controls(schema, openapi))
                })
                .unwrap_or_default();
            let description = item
                .get("metadata")
                .and_then(|metadata| metadata.get("generated_description"))
                .and_then(Value::as_str)
                .or_else(|| model.get("description").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            Some(MediaModelInfo {
                name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string(),
                description,
                output_modalities,
                supported_parameters: supported_parameters_from_controls(&controls),
                default_parameters: defaults_from_controls(&controls),
                pricing: Value::Null,
                id,
            })
        })
        .collect()
}

fn media_models_from_fal(value: &Value, kind: &str) -> Vec<MediaModelInfo> {
    value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let metadata = model.get("metadata").unwrap_or(&Value::Null);
            let output_modalities = fal_output_modalities(model, metadata);
            if !output_modalities.iter().any(|item| item == kind) {
                return None;
            }
            let id = model
                .get("endpoint_id")
                .and_then(Value::as_str)?
                .to_string();
            let controls = model
                .get("openapi")
                .and_then(|openapi| {
                    openapi_input_schema(openapi)
                        .map(|schema| json_schema_controls(schema, openapi))
                })
                .unwrap_or_default();
            Some(MediaModelInfo {
                name: metadata
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string(),
                description: metadata
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                output_modalities,
                supported_parameters: supported_parameters_from_controls(&controls),
                default_parameters: defaults_from_controls(&controls),
                pricing: Value::Null,
                id,
            })
        })
        .collect()
}

fn filter_media_models(models: Vec<MediaModelInfo>, query: &str) -> Vec<MediaModelInfo> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() || matches!(needle.as_str(), "image" | "video" | "music") {
        return models;
    }
    models
        .into_iter()
        .filter(|model| {
            model.id.to_ascii_lowercase().contains(&needle)
                || model.name.to_ascii_lowercase().contains(&needle)
                || model.description.to_ascii_lowercase().contains(&needle)
        })
        .collect()
}

fn replicate_schema_controls(
    value: &Value,
) -> Result<(Vec<String>, Vec<MediaSchemaControl>), ApiError> {
    let openapi = replicate_model_openapi_schema(value).ok_or_else(|| {
        ApiError(Error::Upstream(
            "Replicate model response did not include an OpenAPI schema".to_string(),
        ))
    })?;
    let input_schema = openapi_input_schema(openapi).ok_or_else(|| {
        ApiError(Error::Upstream(
            "Replicate model schema did not include an Input schema".to_string(),
        ))
    })?;
    let controls = json_schema_controls(input_schema, openapi);
    Ok((supported_parameters_from_controls(&controls), controls))
}

fn fal_schema_controls(value: &Value) -> Result<(Vec<String>, Vec<MediaSchemaControl>), ApiError> {
    let openapi = value
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| models.first())
        .and_then(|model| model.get("openapi"))
        .or_else(|| value.get("openapi"))
        .ok_or_else(|| {
            ApiError(Error::Upstream(
                "fal model response did not include an OpenAPI schema".to_string(),
            ))
        })?;
    let input_schema = openapi_input_schema(openapi).ok_or_else(|| {
        ApiError(Error::Upstream(
            "fal model schema did not include an input schema".to_string(),
        ))
    })?;
    let controls = json_schema_controls(input_schema, openapi);
    Ok((supported_parameters_from_controls(&controls), controls))
}

fn replicate_model_id(model: &Value) -> Option<String> {
    let owner = model.get("owner").and_then(Value::as_str)?;
    let name = model.get("name").and_then(Value::as_str)?;
    Some(format!("{owner}/{name}"))
}

fn replicate_model_openapi_schema(model: &Value) -> Option<&Value> {
    model
        .get("latest_version")
        .and_then(|version| version.get("openapi_schema"))
}

fn fal_output_modalities(model: &Value, metadata: &Value) -> Vec<String> {
    let category = metadata
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if category == "text-to-image" || category.ends_with("to-image") || category == "image" {
        vec!["image".to_string()]
    } else if category == "text-to-video" || category.ends_with("to-video") || category == "video" {
        vec!["video".to_string()]
    } else if category == "text-to-audio" {
        let descriptor = format!("{} {}", model, metadata).to_ascii_lowercase();
        if descriptor_is_music(&descriptor) {
            vec!["music".to_string()]
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

fn openapi_input_schema(openapi: &Value) -> Option<&Value> {
    if let Some(schema) = openapi_schema_by_name(openapi, "Input") {
        return Some(schema);
    }
    if let Some(paths) = openapi.get("paths").and_then(Value::as_object) {
        for operation in paths.values().filter_map(|path| path.get("post")) {
            if let Some(schema) = operation
                .get("requestBody")
                .and_then(|body| body.get("content"))
                .and_then(|content| content.get("application/json"))
                .and_then(|content| content.get("schema"))
                .and_then(|schema| resolve_schema_ref(openapi, schema))
            {
                return Some(schema);
            }
        }
    }
    openapi
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .and_then(|schemas| {
            schemas
                .iter()
                .find(|(name, schema)| {
                    name.to_ascii_lowercase().ends_with("input")
                        && schema
                            .get("properties")
                            .and_then(Value::as_object)
                            .is_some()
                })
                .map(|(_, schema)| schema)
        })
}

fn openapi_schema_by_name<'a>(openapi: &'a Value, suffix: &str) -> Option<&'a Value> {
    let suffix = suffix.to_ascii_lowercase();
    openapi
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .and_then(|schemas| {
            schemas
                .iter()
                .find(|(name, _)| name.to_ascii_lowercase() == suffix)
                .or_else(|| {
                    schemas
                        .iter()
                        .find(|(name, _)| name.to_ascii_lowercase().ends_with(&suffix))
                })
                .map(|(_, schema)| schema)
        })
}

fn resolve_schema_ref<'a>(root: &'a Value, schema: &'a Value) -> Option<&'a Value> {
    let reference = schema.get("$ref").and_then(Value::as_str)?;
    let path = reference.strip_prefix("#/")?;
    let mut cursor = root;
    for segment in path.split('/') {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        cursor = cursor.get(&segment)?;
    }
    Some(cursor)
}

fn json_schema_controls(input_schema: &Value, root: &Value) -> Vec<MediaSchemaControl> {
    let Some(properties) = input_schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut keys = input_schema
        .get("x-fal-order-properties")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| properties.keys().cloned().collect());
    for key in properties.keys() {
        if !keys.iter().any(|existing| existing == key) {
            keys.push(key.clone());
        }
    }
    keys.into_iter()
        .filter(|key| key != "prompt")
        .filter_map(|key| {
            properties
                .get(&key)
                .and_then(|schema| control_from_json_schema_property(&key, schema, root))
        })
        .collect()
}

fn control_from_json_schema_property(
    key: &str,
    schema: &Value,
    root: &Value,
) -> Option<MediaSchemaControl> {
    let label = schema
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| label_from_key(key));
    let description = schema_description(schema, root, 0);
    let default_value = schema.get("default").cloned();
    if let Some(options) = schema_enum_options(schema, root, 0) {
        return Some(select_control_values(
            key,
            &label,
            vec![key.to_string()],
            options,
            default_value,
            description,
        ));
    }
    let schema_type = schema_type(schema, root, 0)?;
    match schema_type.as_str() {
        "boolean" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "checkbox".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: None,
            max: None,
            step: None,
            item_kind: None,
            placeholder: None,
        }),
        "integer" | "number" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "number".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: schema_number(schema, root, "minimum", 0),
            max: schema_number(schema, root, "maximum", 0),
            step: if schema_type == "integer" {
                Some(1.0)
            } else {
                None
            },
            item_kind: None,
            placeholder: None,
        }),
        "array" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "array".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: schema_number(schema, root, "minItems", 0),
            max: schema_number(schema, root, "maxItems", 0),
            step: None,
            item_kind: schema_array_item_kind(schema, root, 0).map(str::to_string),
            placeholder: Some("one value per line".to_string()),
        }),
        "object" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "json".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: None,
            max: None,
            step: None,
            item_kind: None,
            placeholder: Some("{ }".to_string()),
        }),
        "string" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: if schema_is_url(schema, root, key, 0) {
                "url".to_string()
            } else {
                "text".to_string()
            },
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: None,
            max: None,
            step: None,
            item_kind: None,
            placeholder: None,
        }),
        _ => None,
    }
}

fn schema_enum_options(
    schema: &Value,
    root: &Value,
    depth: usize,
) -> Option<Vec<MediaControlOption>> {
    if depth > 8 {
        return None;
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let options = values
            .iter()
            .filter(|value| !value.is_null())
            .map(|value| MediaControlOption {
                label: media_option_label(value),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        if !options.is_empty() {
            return Some(options);
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(options) = schema_enum_options(item, root, depth + 1) {
                    return Some(options);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(options) = schema_enum_options(resolved, root, depth + 1) {
                        return Some(options);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema)
        .and_then(|resolved| schema_enum_options(resolved, root, depth + 1))
}

fn schema_type(schema: &Value, root: &Value, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    match schema.get("type") {
        Some(Value::String(kind)) if kind != "null" => return Some(kind.to_string()),
        Some(Value::Array(kinds)) => {
            if let Some(kind) = kinds
                .iter()
                .filter_map(Value::as_str)
                .find(|kind| *kind != "null")
            {
                return Some(kind.to_string());
            }
        }
        _ => {}
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(kind) = schema_type(item, root, depth + 1) {
                    return Some(kind);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(kind) = schema_type(resolved, root, depth + 1) {
                        return Some(kind);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema).and_then(|resolved| schema_type(resolved, root, depth + 1))
}

fn schema_number(schema: &Value, root: &Value, key: &str, depth: usize) -> Option<f64> {
    if depth > 8 {
        return None;
    }
    if let Some(value) = schema.get(key).and_then(Value::as_f64) {
        return Some(value);
    }
    for group in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(group).and_then(Value::as_array) {
            for item in items {
                if let Some(value) = schema_number(item, root, key, depth + 1) {
                    return Some(value);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(value) = schema_number(resolved, root, key, depth + 1) {
                        return Some(value);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema)
        .and_then(|resolved| schema_number(resolved, root, key, depth + 1))
}

fn schema_description(schema: &Value, root: &Value, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    if let Some(description) = schema.get("description").and_then(Value::as_str) {
        return Some(description.trim().to_string());
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(description) = schema_description(item, root, depth + 1) {
                    return Some(description);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(description) = schema_description(resolved, root, depth + 1) {
                        return Some(description);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema)
        .and_then(|resolved| schema_description(resolved, root, depth + 1))
}

fn schema_is_url(schema: &Value, root: &Value, key: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    if schema.get("format").and_then(Value::as_str) == Some("uri") {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    if lower.ends_with("_url") || lower.ends_with(" url") || lower.contains("image_url") {
        return true;
    }
    for group in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(group).and_then(Value::as_array) {
            if items
                .iter()
                .any(|item| schema_is_url(item, root, key, depth + 1))
            {
                return true;
            }
        }
    }
    resolve_schema_ref(root, schema)
        .map(|resolved| schema_is_url(resolved, root, key, depth + 1))
        .unwrap_or(false)
}

fn schema_array_item_kind(schema: &Value, root: &Value, depth: usize) -> Option<&'static str> {
    if depth > 8 {
        return None;
    }
    let items = schema.get("items")?;
    let resolved = resolve_schema_ref(root, items).unwrap_or(items);
    if schema_is_url(resolved, root, "", depth + 1) {
        return Some("url");
    }
    match schema_type(resolved, root, depth + 1).as_deref() {
        Some("integer") | Some("number") => Some("number"),
        Some("boolean") => Some("checkbox"),
        Some("object") | Some("array") => Some("json"),
        Some("string") => Some("text"),
        _ => None,
    }
}

fn supported_parameters_from_controls(controls: &[MediaSchemaControl]) -> Vec<String> {
    controls.iter().map(|control| control.key.clone()).collect()
}

fn defaults_from_controls(controls: &[MediaSchemaControl]) -> Value {
    let mut out = serde_json::Map::new();
    for control in controls {
        if let Some(value) = &control.default_value {
            out.insert(control.key.clone(), value.clone());
        }
    }
    Value::Object(out)
}

fn media_option_label(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn label_from_key(key: &str) -> String {
    key.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn openrouter_output_modalities(model: &Value) -> Vec<String> {
    string_array(
        model
            .get("architecture")
            .and_then(|architecture| architecture.get("output_modalities")),
    )
}

fn openrouter_input_modalities(model: &Value) -> Vec<String> {
    string_array(
        model
            .get("architecture")
            .and_then(|architecture| architecture.get("input_modalities")),
    )
}

fn media_model_descriptor(model: &Value) -> String {
    format!(
        "{} {} {} {}",
        model.get("id").and_then(Value::as_str).unwrap_or_default(),
        model
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        model
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        model.get("tags").cloned().unwrap_or(Value::Null)
    )
    .to_ascii_lowercase()
}

fn descriptor_is_music(descriptor: &str) -> bool {
    let excluded = [
        "text-to-speech",
        "speech-to-text",
        "transcription",
        "voice cloning",
        "voice changer",
        "dubbing",
        "tts",
    ];
    let music = [
        "music",
        "song",
        "instrumental",
        "lyrics",
        "musicgen",
        "lyria",
        "melody",
    ];
    music.iter().any(|term| descriptor.contains(term))
        && !excluded.iter().any(|term| descriptor.contains(term))
}

fn descriptor_matches_media_kind(descriptor: &str, kind: &str) -> bool {
    match kind {
        "music" => descriptor_is_music(descriptor),
        "video" => [
            "video",
            "text-to-video",
            "veo",
            "kling",
            "runway",
            "wan",
            "hailuo",
        ]
        .iter()
        .any(|term| descriptor.contains(term)),
        "image" => [
            "image",
            "text-to-image",
            "flux",
            "sdxl",
            "stable diffusion",
            "recraft",
        ]
        .iter()
        .any(|term| descriptor.contains(term)),
        _ => false,
    }
}

fn openrouter_video_model_parameters(model: &Value) -> Vec<String> {
    let mut out = string_array(model.get("supported_parameters"));
    for (field, parameter) in [
        ("supported_durations", "duration"),
        ("durations", "duration"),
        ("supported_resolutions", "resolution"),
        ("resolutions", "resolution"),
        ("supported_aspect_ratios", "aspect_ratio"),
        ("aspect_ratios", "aspect_ratio"),
        ("supported_sizes", "size"),
    ] {
        if model.get(field).is_some() && !out.iter().any(|item| item == parameter) {
            out.push(parameter.to_string());
        }
    }
    for parameter in ["generate_audio", "seed"] {
        if model.get(parameter).is_some() && !out.iter().any(|item| item == parameter) {
            out.push(parameter.to_string());
        }
    }
    out
}

fn schema_outputs_media_url(schema: &Value, root: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    if schema.get("format").and_then(Value::as_str) == Some("uri") {
        return true;
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        if properties
            .values()
            .any(|value| schema_outputs_media_url(value, root, depth + 1))
        {
            return true;
        }
    }
    if let Some(items) = schema.get("items") {
        if schema_outputs_media_url(items, root, depth + 1) {
            return true;
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if schema
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| schema_outputs_media_url(item, root, depth + 1))
            })
        {
            return true;
        }
    }
    resolve_schema_ref(root, schema)
        .is_some_and(|resolved| schema_outputs_media_url(resolved, root, depth + 1))
}

fn openrouter_schema_supported_parameters(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(endpoints) = value.get("endpoints").and_then(Value::as_array) {
        for endpoint in endpoints {
            push_unique_strings(&mut out, endpoint.get("supported_parameters"));
        }
    }
    push_unique_strings(&mut out, value.get("supported_parameters"));
    out
}

fn openrouter_video_model<'a>(value: &'a Value, model: &str) -> Option<&'a Value> {
    value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(model))
        })
}

fn openrouter_video_supported_parameters(value: &Value, model: &str) -> Vec<String> {
    openrouter_video_model(value, model)
        .map(openrouter_video_model_parameters)
        .unwrap_or_default()
}

fn openrouter_video_values(model: &Value, fields: &[&str]) -> Vec<Value> {
    for field in fields {
        if let Some(values) = model.get(field).and_then(Value::as_array) {
            return values
                .iter()
                .filter(|value| value.is_string() || value.is_number())
                .cloned()
                .collect();
        }
    }
    Vec::new()
}

fn openrouter_video_schema_controls(
    value: &Value,
    model_id: &str,
    supported: &[String],
) -> Vec<MediaSchemaControl> {
    let Some(model) = openrouter_video_model(value, model_id) else {
        return Vec::new();
    };
    let mut controls = Vec::new();
    for (key, label, fields) in [
        (
            "duration",
            "Duration",
            &["supported_durations", "durations"][..],
        ),
        (
            "resolution",
            "Resolution",
            &["supported_resolutions", "resolutions"][..],
        ),
        (
            "aspect_ratio",
            "Aspect ratio",
            &["supported_aspect_ratios", "aspect_ratios"][..],
        ),
    ] {
        let values = openrouter_video_values(model, fields);
        if !values.is_empty() {
            controls.push(select_control_values(
                key,
                label,
                vec![key.to_string()],
                values
                    .iter()
                    .map(|value| MediaControlOption {
                        label: media_option_label(value),
                        value: value.clone(),
                    })
                    .collect(),
                values.first().cloned(),
                None,
            ));
        }
    }
    let has = |key: &str| supported.iter().any(|item| item == key);
    if has("generate_audio") || model.get("generate_audio").is_some() {
        controls.push(checkbox_control(
            "generate_audio",
            "Generate audio",
            ["generate_audio"],
            Some(json!(model
                .get("generate_audio")
                .and_then(Value::as_bool)
                .unwrap_or(false))),
        ));
    }
    if has("seed") {
        controls.push(number_control(
            "seed",
            "Seed",
            ["seed"],
            None,
            Some(0.0),
            None,
            Some(1.0),
        ));
    }
    controls
}

fn openrouter_music_schema_controls(supported: &[String]) -> Vec<MediaSchemaControl> {
    let has = |key: &str| supported.iter().any(|item| item == key);
    let mut controls = Vec::new();
    if has("seed") {
        controls.push(number_control(
            "seed",
            "Seed",
            ["seed"],
            None,
            Some(0.0),
            None,
            Some(1.0),
        ));
    }
    if has("temperature") {
        controls.push(number_control(
            "temperature",
            "Temperature",
            ["temperature"],
            Some(json!(0.7)),
            Some(0.0),
            Some(2.0),
            Some(0.1),
        ));
    }
    if has("top_p") {
        controls.push(number_control(
            "top_p",
            "Top P",
            ["top_p"],
            Some(json!(0.9)),
            Some(0.0),
            Some(1.0),
            Some(0.05),
        ));
    }
    controls
}

fn push_unique_strings(out: &mut Vec<String>, value: Option<&Value>) {
    for item in string_array(value) {
        if !out.iter().any(|existing| existing == &item) {
            out.push(item);
        }
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn media_schema_controls(supported: &[String]) -> Vec<MediaSchemaControl> {
    let has = |key: &str| supported.iter().any(|item| item == key);
    let mut controls = vec![select_control(
        "aspect_ratio",
        "Aspect ratio",
        ["image_config", "aspect_ratio"],
        ["1:1", "16:9", "9:16", "4:3", "3:4"],
        Some("1:1"),
    )];

    controls.push(select_control(
        "quality",
        "Quality",
        ["image_config", "quality"],
        ["auto", "low", "medium", "high"],
        Some("auto"),
    ));

    if has("seed") {
        controls.push(number_control(
            "seed",
            "Seed",
            ["seed"],
            None,
            Some(0.0),
            None,
            Some(1.0),
        ));
    }
    if has("temperature") {
        controls.push(number_control(
            "temperature",
            "Temperature",
            ["temperature"],
            Some(json!(0.7)),
            Some(0.0),
            Some(2.0),
            Some(0.1),
        ));
    }
    if has("top_p") {
        controls.push(number_control(
            "top_p",
            "Top P",
            ["top_p"],
            Some(json!(0.9)),
            Some(0.0),
            Some(1.0),
            Some(0.05),
        ));
    }
    controls
}

fn select_control<const N: usize, const M: usize>(
    key: &str,
    label: &str,
    path: [&str; N],
    options: [&str; M],
    default_value: Option<&str>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "select".to_string(),
        path: path.into_iter().map(str::to_string).collect(),
        description: None,
        options: Some(
            options
                .into_iter()
                .map(|value| MediaControlOption {
                    label: value.to_string(),
                    value: Value::String(value.to_string()),
                })
                .collect(),
        ),
        default_value: default_value.map(|value| Value::String(value.to_string())),
        min: None,
        max: None,
        step: None,
        item_kind: None,
        placeholder: None,
    }
}

fn select_control_values(
    key: &str,
    label: &str,
    path: Vec<String>,
    options: Vec<MediaControlOption>,
    default_value: Option<Value>,
    description: Option<String>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "select".to_string(),
        path,
        description,
        options: Some(options),
        default_value,
        min: None,
        max: None,
        step: None,
        item_kind: None,
        placeholder: None,
    }
}

fn checkbox_control<const N: usize>(
    key: &str,
    label: &str,
    path: [&str; N],
    default_value: Option<Value>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "checkbox".to_string(),
        path: path.into_iter().map(str::to_string).collect(),
        description: None,
        options: None,
        default_value,
        min: None,
        max: None,
        step: None,
        item_kind: None,
        placeholder: None,
    }
}

fn number_control<const N: usize>(
    key: &str,
    label: &str,
    path: [&str; N],
    default_value: Option<Value>,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "number".to_string(),
        path: path.into_iter().map(str::to_string).collect(),
        description: None,
        options: None,
        default_value,
        min,
        max,
        step,
        item_kind: None,
        placeholder: None,
    }
}

async fn post_media_json(builder: reqwest::RequestBuilder) -> Result<Value, ApiError> {
    let response = builder.send().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider request failed: {e}"
        )))
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider response failed: {e}"
        )))
    })?;
    if !status.is_success() {
        return Err(ApiError(Error::Upstream(format!(
            "media provider returned HTTP {status}: {body}"
        ))));
    }
    serde_json::from_str(&body).map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider returned invalid JSON: {e}"
        )))
    })
}

fn is_replicate_version_id(model: &str) -> bool {
    model.len() == 64 && model.bytes().all(|b| b.is_ascii_hexdigit())
}

fn media_urls_from_result(kind: &crate::providers::ProviderKind, value: &Value) -> Value {
    let mut urls = serde_json::Map::new();
    if let Some(source) = value.get("urls").and_then(Value::as_object) {
        for (key, value) in source {
            urls.insert(key.clone(), value.clone());
        }
    }
    if matches!(kind, crate::providers::ProviderKind::Fal) {
        for (source, target) in [
            ("response_url", "response"),
            ("status_url", "status"),
            ("cancel_url", "cancel"),
        ] {
            if let Some(url) = value.get(source).and_then(Value::as_str) {
                urls.insert(target.to_string(), Value::String(url.to_string()));
            }
        }
    }
    Value::Object(urls)
}

fn media_items_from_result(value: &Value, kind_hint: &str) -> Vec<MediaItem> {
    let mut urls = Vec::new();
    if let Some(output) = value.get("output") {
        collect_media_urls(output, &mut urls);
    }
    if let Some(images) = value.get("images") {
        collect_media_urls(images, &mut urls);
    }
    if let Some(video) = value.get("video") {
        collect_media_urls(video, &mut urls);
    }
    if let Some(audio) = value.get("audio") {
        collect_media_urls(audio, &mut urls);
    }
    if let Some(unsigned_urls) = value.get("unsigned_urls") {
        collect_media_urls(unsigned_urls, &mut urls);
    }
    if let Some(choices) = value.get("choices") {
        collect_media_urls(choices, &mut urls);
    }
    urls.sort();
    urls.dedup();
    urls.into_iter()
        .map(|url| MediaItem {
            kind: media_kind_from_url(&url, kind_hint).to_string(),
            mime: media_mime_from_url(&url).map(str::to_string),
            url,
            requires_auth: false,
        })
        .collect()
}

fn openrouter_video_media_items(provider_id: &str, id: &str, value: &Value) -> Vec<MediaItem> {
    let has_output = value
        .get("unsigned_urls")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
        || value.get("output").is_some_and(|output| !output.is_null());
    if !has_output {
        return Vec::new();
    }
    vec![MediaItem {
        url: format!(
            "/media/content?provider_id={}&id={}&index=0",
            percent_encode_query(provider_id),
            percent_encode_query(id)
        ),
        kind: "video".to_string(),
        mime: Some("video/mp4".to_string()),
        requires_auth: true,
    }]
}

fn percent_encode_query(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

fn collect_media_urls(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if text.starts_with("http://")
                || text.starts_with("https://")
                || text.starts_with("data:image/")
                || text.starts_with("data:video/")
                || text.starts_with("data:audio/")
            {
                out.push(text.to_string());
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_media_urls(value, out);
            }
        }
        Value::Object(map) => {
            if let Some(url) = map.get("url").and_then(Value::as_str) {
                out.push(url.to_string());
            }
            for value in map.values() {
                collect_media_urls(value, out);
            }
        }
        _ => {}
    }
}

fn media_kind_from_url(url: &str, kind_hint: &str) -> &'static str {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("data:audio/")
        || [".mp3", ".wav", ".flac", ".ogg", ".m4a", ".aac", ".opus"]
            .iter()
            .any(|extension| lower.contains(extension))
    {
        "music"
    } else if lower.starts_with("data:video/")
        || lower.contains(".mp4")
        || lower.contains(".webm")
        || lower.contains(".mov")
    {
        "video"
    } else if matches!(kind_hint, "image" | "video" | "music") {
        match kind_hint {
            "video" => "video",
            "music" => "music",
            _ => "image",
        }
    } else {
        "image"
    }
}

fn media_mime_from_url(url: &str) -> Option<&'static str> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("data:image/png") || lower.contains(".png") {
        Some("image/png")
    } else if lower.starts_with("data:image/jpeg")
        || lower.contains(".jpg")
        || lower.contains(".jpeg")
    {
        Some("image/jpeg")
    } else if lower.starts_with("data:image/webp") || lower.contains(".webp") {
        Some("image/webp")
    } else if lower.starts_with("data:image/gif") || lower.contains(".gif") {
        Some("image/gif")
    } else if lower.starts_with("data:video/mp4") || lower.contains(".mp4") {
        Some("video/mp4")
    } else if lower.starts_with("data:video/webm") || lower.contains(".webm") {
        Some("video/webm")
    } else if lower.starts_with("data:video/quicktime") || lower.contains(".mov") {
        Some("video/quicktime")
    } else if lower.starts_with("data:audio/mpeg")
        || lower.starts_with("data:audio/mp3")
        || lower.contains(".mp3")
    {
        Some("audio/mpeg")
    } else if lower.starts_with("data:audio/wav")
        || lower.starts_with("data:audio/x-wav")
        || lower.contains(".wav")
    {
        Some("audio/wav")
    } else if lower.starts_with("data:audio/flac")
        || lower.starts_with("data:audio/x-flac")
        || lower.contains(".flac")
    {
        Some("audio/flac")
    } else if lower.starts_with("data:audio/ogg") || lower.contains(".ogg") {
        Some("audio/ogg")
    } else if lower.starts_with("data:audio/mp4")
        || lower.starts_with("data:audio/x-m4a")
        || lower.contains(".m4a")
    {
        Some("audio/mp4")
    } else if lower.starts_with("data:audio/aac") || lower.contains(".aac") {
        Some("audio/aac")
    } else if lower.starts_with("data:audio/opus") || lower.contains(".opus") {
        Some("audio/opus")
    } else {
        None
    }
}
