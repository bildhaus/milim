use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::webview::{NewWindowResponse, PageLoadEvent, PageLoadPayload, WebviewBuilder};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, Runtime, Url, Webview, WebviewUrl};

pub const PREVIEW_WEBVIEW_NAVIGATION_EVENT: &str = "milim://preview-webview-navigation";
pub const PREVIEW_WEBVIEW_NEW_TAB_EVENT: &str = "milim://preview-webview-new-tab";
pub const PREVIEW_WEBVIEW_SHORTCUT_EVENT: &str = "milim://preview-webview-shortcut";
pub const PREVIEW_WEBVIEW_TITLE_EVENT: &str = "milim://preview-webview-title";
const PREVIEW_APP_PRIVATE_LABEL: &str = "artifact-browser-app-private-slot";
const PREVIEW_URL_PRIVATE_LABEL: &str = "artifact-browser-url-private-slot";
const PREVIEW_URL_PERSISTENT_LABEL: &str = "artifact-browser-url-persistent-slot";
const PREVIEW_CLEAR_LABEL: &str = "artifact-browser-clear-maintenance";
const PREVIEW_SHORTCUT_SCHEME: &str = "milim-browser-shortcut";
const BROWSER_PROFILE_DIRECTORY: &str = "browser-profile";
const PRIVATE_BROWSER_PROFILE_DIRECTORY: &str = "milim-private-browser";
const PRIVATE_BROWSER_PROFILE_LOCK: &str = "owner.lock";
const BROWSER_DATA_STORE_IDENTIFIER: [u8; 16] = *b"milim-browser-v1";
#[cfg(windows)]
const PREVIEW_NATIVE_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(windows)]
const PREVIEW_SUSPEND_TIMEOUT: Duration = Duration::from_secs(2);
static NEXT_BROWSER_WINDOW_ID: AtomicU64 = AtomicU64::new(1);
static PREVIEW_SURFACES: OnceLock<Mutex<HashMap<String, PreviewSurfaceState>>> = OnceLock::new();
static PREVIEW_LIFECYCLE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
static PRIVATE_PROFILE_LEASE: OnceLock<Mutex<Option<PrivateProfileLease>>> = OnceLock::new();

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewBrowserStorageMode {
    Persistent,
    Private,
}

#[derive(Deserialize)]
pub struct PreviewWebviewBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWebviewCreateResult {
    reused: bool,
    url: String,
    claim_token: u64,
    navigated: bool,
}

#[derive(Clone)]
struct PreviewSurfaceState {
    claim_token: u64,
    profile_id: String,
    parked: bool,
}

struct PrivateProfileLease {
    directory: PathBuf,
    _lock: File,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWebviewDiagnostics {
    live_surfaces: usize,
    active_surfaces: usize,
    parked_surfaces: usize,
    maximum_surfaces: usize,
    maintenance_surface: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewWebviewNavigationPayload {
    label: String,
    claim_token: u64,
    url: String,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewWebviewNewTabPayload {
    request_id: u64,
    opener_label: String,
    claim_token: u64,
    url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PreviewWebviewShortcutAction {
    NewTab,
    CloseTab,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewWebviewShortcutPayload {
    label: String,
    claim_token: u64,
    action: PreviewWebviewShortcutAction,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewWebviewTitlePayload {
    label: String,
    claim_token: u64,
    title: String,
}

pub fn handle_page_load(webview: &Webview, payload: &PageLoadPayload<'_>) {
    if !is_preview_pool_label(webview.label()) {
        return;
    }
    let Some(claim_token) = current_claim_token(webview.label()) else {
        return;
    };
    if webview
        .url()
        .is_ok_and(|current_url| current_url.as_str() != payload.url().as_str())
    {
        return;
    }
    let state = match payload.event() {
        PageLoadEvent::Started => "loading",
        PageLoadEvent::Finished => "ready",
    };
    let event = PreviewWebviewNavigationPayload {
        label: webview.label().to_string(),
        claim_token,
        url: payload.url().to_string(),
        state,
        message: None,
    };
    emit_navigation(webview, event);
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    scavenge_private_browser_profiles();
    Builder::new("preview-webview-navigation")
        .on_navigation(|webview, url| {
            if !is_preview_pool_label(webview.label()) {
                return true;
            }
            let Some(claim_token) = current_claim_token(webview.label()) else {
                return true;
            };
            let allowed = preview_url_allowed_for_label(webview.label(), url);
            emit_navigation(
                webview,
                PreviewWebviewNavigationPayload {
                    label: webview.label().to_string(),
                    claim_token,
                    url: url.to_string(),
                    state: if allowed { "navigated" } else { "error" },
                    message: (!allowed).then(|| {
                        "Blocked navigation: preview URLs must use HTTPS or loopback HTTP."
                            .to_string()
                    }),
                },
            );
            allowed
        })
        .build()
}

#[tauri::command]
pub async fn preview_webview_create(
    app: tauri::AppHandle,
    label: String,
    url: String,
    bounds: PreviewWebviewBounds,
    storage_mode: PreviewBrowserStorageMode,
    profile_id: String,
) -> Result<PreviewWebviewCreateResult, String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    if !is_preview_pool_label(&label) {
        return Err("invalid preview webview label".to_string());
    }
    if !preview_label_accepts_storage_mode(&label, storage_mode) {
        return Err("preview webview label does not match its storage mode".to_string());
    }
    if !is_profile_id(&profile_id) {
        return Err("invalid preview browser profile id".to_string());
    }
    validate_preview_bounds(&bounds)?;
    let url = allowed_preview_url(&url)?;
    let claim_token = NEXT_BROWSER_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    if let Some(webview) = app.get_webview(&label) {
        let previous = preview_surfaces()
            .lock()
            .map_err(|_| "preview surface lock poisoned".to_string())?
            .get(&label)
            .cloned();
        let private_owner_changed = storage_mode == PreviewBrowserStorageMode::Private
            && previous
                .as_ref()
                .is_some_and(|surface| surface.profile_id != profile_id);
        preview_surfaces()
            .lock()
            .map_err(|_| "preview surface lock poisoned".to_string())?
            .insert(
                label.clone(),
                PreviewSurfaceState {
                    claim_token,
                    profile_id,
                    parked: true,
                },
            );
        set_preview_webview_muted(&webview, true)?;
        webview.hide().map_err(|error| error.to_string())?;
        if private_owner_changed {
            clear_preview_browsing_data(&webview).await?;
        }
        resume_preview_webview(&webview)?;
        let current_url = webview.url().map_err(|error| error.to_string())?;
        let navigated = true;
        if current_url != url {
            webview
                .navigate(url.clone())
                .map_err(|error| error.to_string())?;
        } else {
            webview.reload().map_err(|error| error.to_string())?;
        }
        return Ok(PreviewWebviewCreateResult {
            reused: true,
            url: url.to_string(),
            claim_token,
            navigated,
        });
    }
    let initial_url = url.to_string();
    let window = app
        .get_window("main")
        .ok_or_else(|| "main window is no longer available".to_string())?;
    let profile_directory = browser_profile_directory_for_mode(storage_mode, &label)?;
    let popup_app = app.clone();
    let opener_label = label.clone();
    let shortcut_token = NEXT_BROWSER_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    let builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
        .focused(false)
        .incognito(native_incognito_for_mode(storage_mode))
        .data_directory(profile_directory.clone())
        .data_store_identifier(BROWSER_DATA_STORE_IDENTIFIER)
        // Route zoom shortcuts through the main UI so their value can be persisted
        // and restored when this child webview is recreated.
        .zoom_hotkeys_enabled(false)
        .initialization_script(preview_shortcut_script(shortcut_token))
        .on_document_title_changed(|webview, title| {
            if let (Some(main), Some(claim_token)) = (
                webview.get_webview("main"),
                current_claim_token(webview.label()),
            ) {
                let _ = main.emit(
                    PREVIEW_WEBVIEW_TITLE_EVENT,
                    PreviewWebviewTitlePayload {
                        label: webview.label().to_string(),
                        claim_token,
                        title,
                    },
                );
            }
        })
        .on_new_window(move |url, _features| {
            if let Some(action) = preview_shortcut_action(&url, shortcut_token) {
                if let (Some(main), Some(claim_token)) = (
                    popup_app.get_webview("main"),
                    current_claim_token(&opener_label),
                ) {
                    let _ = main.emit(
                        PREVIEW_WEBVIEW_SHORTCUT_EVENT,
                        PreviewWebviewShortcutPayload {
                            label: opener_label.clone(),
                            claim_token,
                            action,
                        },
                    );
                }
                return NewWindowResponse::Deny;
            }
            if !preview_new_tab_url_allowed(&url) {
                return NewWindowResponse::Deny;
            }
            if let (Some(main), Some(claim_token)) = (
                popup_app.get_webview("main"),
                current_claim_token(&opener_label),
            ) {
                let _ = main.emit(
                    PREVIEW_WEBVIEW_NEW_TAB_EVENT,
                    PreviewWebviewNewTabPayload {
                        request_id: NEXT_BROWSER_WINDOW_ID.fetch_add(1, Ordering::Relaxed),
                        opener_label: opener_label.clone(),
                        claim_token,
                        url: url.to_string(),
                    },
                );
            }
            NewWindowResponse::Deny
        });
    preview_surfaces()
        .lock()
        .map_err(|_| "preview surface lock poisoned".to_string())?
        .insert(
            label.clone(),
            PreviewSurfaceState {
                claim_token,
                profile_id,
                parked: true,
            },
        );
    let webview = match window.add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width, bounds.height),
        ) {
        Ok(webview) => webview,
        Err(error) => {
            if let Ok(mut surfaces) = preview_surfaces().lock() {
                if surfaces
                    .get(&label)
                    .is_some_and(|surface| surface.claim_token == claim_token)
                {
                    surfaces.remove(&label);
                }
            }
            return Err(error.to_string());
        }
    };
    webview.hide().map_err(|error| error.to_string())?;
    Ok(PreviewWebviewCreateResult {
        reused: false,
        url: initial_url,
        claim_token,
        navigated: false,
    })
}

#[tauri::command]
pub async fn preview_webview_clear_data(app: tauri::AppHandle) -> Result<(), String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    let window = app
        .get_window("main")
        .ok_or_else(|| "main window is no longer available".to_string())?;
    let webview = if let Some(webview) = app.get_webview(PREVIEW_CLEAR_LABEL) {
        webview
    } else {
        let builder = WebviewBuilder::new(
            PREVIEW_CLEAR_LABEL,
            WebviewUrl::External(Url::parse("about:blank").expect("valid internal URL")),
        )
        .focused(false)
        .data_directory(browser_profile_directory())
        .data_store_identifier(BROWSER_DATA_STORE_IDENTIFIER);
        window
            .add_child(
                builder,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1.0, 1.0),
            )
            .map_err(|error| error.to_string())?
    };
    let _ = webview.hide();
    clear_preview_browsing_data(&webview).await?;
    if let Some(persistent) = app.get_webview(PREVIEW_URL_PERSISTENT_LABEL) {
        let parked = {
            let surfaces = preview_surfaces()
                .lock()
                .map_err(|_| "preview surface lock poisoned".to_string())?;
            match surfaces.get(PREVIEW_URL_PERSISTENT_LABEL) {
                Some(surface) => surface.parked,
                None => true,
            }
        };
        if !parked {
            persistent.reload().map_err(|error| error.to_string())?;
        }
    }
    let _ = suspend_preview_webview(&webview).await;
    Ok(())
}

#[tauri::command]
pub async fn preview_webview_set_visibility(
    app: tauri::AppHandle,
    label: String,
    claim_token: u64,
    visible: bool,
    muted: bool,
) -> Result<bool, String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    if !claim_is_current(&label, claim_token)? {
        return Ok(false);
    }
    let webview = preview_webview(&app, &label)?;
    if visible {
        resume_preview_webview(&webview)?;
        set_preview_webview_muted(&webview, muted)?;
        webview.show().map_err(|error| error.to_string())?;
    } else {
        set_preview_webview_muted(&webview, true)?;
        webview.hide().map_err(|error| error.to_string())?;
        let _ = suspend_preview_webview(&webview).await;
    }
    if let Some(surface) = preview_surfaces()
        .lock()
        .map_err(|_| "preview surface lock poisoned".to_string())?
        .get_mut(&label)
    {
        surface.parked = !visible;
    }
    Ok(true)
}

fn emit_navigation<R: Runtime>(webview: &Webview<R>, event: PreviewWebviewNavigationPayload) {
    if let Some(main) = webview.get_webview("main") {
        let _ = main.emit(PREVIEW_WEBVIEW_NAVIGATION_EVENT, event);
    }
}

#[tauri::command]
pub async fn preview_webview_navigate(
    app: tauri::AppHandle,
    label: String,
    claim_token: u64,
    url: String,
) -> Result<(), String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    require_current_claim(&label, claim_token)?;
    let webview = preview_webview(&app, &label)?;
    let url = allowed_preview_url(&url)?;
    webview.navigate(url).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_webview_reload(
    app: tauri::AppHandle,
    label: String,
    claim_token: u64,
) -> Result<(), String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    require_current_claim(&label, claim_token)?;
    let webview = preview_webview(&app, &label)?;
    if webview
        .url()
        .map(|url| preview_reload_bypasses_cache(&url))
        .unwrap_or(false)
    {
        reload_without_cache(&webview)
    } else {
        webview.reload().map_err(|error| error.to_string())
    }
}

#[tauri::command]
pub async fn preview_webview_set_muted(
    app: tauri::AppHandle,
    label: String,
    claim_token: u64,
    muted: bool,
) -> Result<(), String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    require_current_claim(&label, claim_token)?;
    set_preview_webview_muted(&preview_webview(&app, &label)?, muted)
}

#[tauri::command]
pub async fn preview_webview_set_zoom(
    app: tauri::AppHandle,
    label: String,
    claim_token: u64,
    scale_factor: f64,
) -> Result<(), String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    require_current_claim(&label, claim_token)?;
    if !scale_factor.is_finite() || !(0.25..=5.0).contains(&scale_factor) {
        return Err("preview zoom must be between 25% and 500%".to_string());
    }
    preview_webview(&app, &label)?
        .set_zoom(scale_factor)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_webview_set_bounds(
    app: tauri::AppHandle,
    label: String,
    claim_token: u64,
    bounds: PreviewWebviewBounds,
) -> Result<(), String> {
    let _lifecycle = preview_lifecycle_lock().lock().await;
    require_current_claim(&label, claim_token)?;
    validate_preview_bounds(&bounds)?;
    let webview = preview_webview(&app, &label)?;
    webview
        .set_position(LogicalPosition::new(bounds.x, bounds.y))
        .map_err(|error| error.to_string())?;
    webview
        .set_size(LogicalSize::new(bounds.width, bounds.height))
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn set_preview_webview_muted(webview: &Webview, muted: bool) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows_core::Interface;

    webview
        .with_webview(move |platform_webview| {
            let controller = platform_webview.controller();
            let result = unsafe {
                (|| -> windows_core::Result<()> {
                    let core = controller.CoreWebView2()?;
                    core.cast::<ICoreWebView2_8>()?.SetIsMuted(muted)
                })()
            };
            if let Err(error) = result {
                tracing::warn!("preview audio mute update failed: {error}");
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn set_preview_webview_muted(webview: &Webview, muted: bool) -> Result<(), String> {
    webview
        .eval(preview_audio_mute_script(muted))
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
async fn clear_preview_browsing_data(webview: &Webview) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Profile2, ICoreWebView2_13,
    };
    use webview2_com::ClearBrowsingDataCompletedHandler;
    use windows_core::Interface;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let callback_tx = tx.clone();
    webview
        .with_webview(move |platform_webview| {
            let result = unsafe {
                (|| -> windows_core::Result<()> {
                    let core = platform_webview.controller().CoreWebView2()?;
                    let profile = core.cast::<ICoreWebView2_13>()?.Profile()?;
                    let handler = ClearBrowsingDataCompletedHandler::create(Box::new(
                        move |status| {
                            if let Some(tx) = callback_tx.lock().ok().and_then(|mut tx| tx.take()) {
                                let _ = tx.send(status.map_err(|error| error.to_string()));
                            }
                            Ok(())
                        },
                    ));
                    profile
                        .cast::<ICoreWebView2Profile2>()?
                        .ClearBrowsingDataAll(&handler)
                })()
            };
            if let Err(error) = result {
                if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                    let _ = tx.send(Err(error.to_string()));
                }
            }
        })
        .map_err(|error| error.to_string())?;
    tokio::time::timeout(PREVIEW_NATIVE_OPERATION_TIMEOUT, rx)
        .await
        .map_err(|_| "timed out while clearing preview browser data".to_string())?
        .map_err(|_| "preview browser data callback closed".to_string())?
}

#[cfg(not(windows))]
async fn clear_preview_browsing_data(webview: &Webview) -> Result<(), String> {
    webview
        .clear_all_browsing_data()
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
async fn suspend_preview_webview(webview: &Webview) -> Result<bool, String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
    use webview2_com::TrySuspendCompletedHandler;
    use windows_core::Interface;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<bool, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let callback_tx = tx.clone();
    webview
        .with_webview(move |platform_webview| {
            let result = unsafe {
                (|| -> windows_core::Result<()> {
                    let core = platform_webview.controller().CoreWebView2()?;
                    let handler = TrySuspendCompletedHandler::create(Box::new(
                        move |status, suspended| {
                            if let Some(tx) = callback_tx.lock().ok().and_then(|mut tx| tx.take()) {
                                let _ = tx.send(
                                    status
                                        .map(|_| suspended)
                                        .map_err(|error| error.to_string()),
                                );
                            }
                            Ok(())
                        },
                    ));
                    core.cast::<ICoreWebView2_3>()?.TrySuspend(&handler)
                })()
            };
            if let Err(error) = result {
                if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                    let _ = tx.send(Err(error.to_string()));
                }
            }
        })
        .map_err(|error| error.to_string())?;
    tokio::time::timeout(PREVIEW_SUSPEND_TIMEOUT, rx)
        .await
        .map_err(|_| "timed out while suspending preview webview".to_string())?
        .map_err(|_| "preview suspend callback closed".to_string())?
}

#[cfg(not(windows))]
async fn suspend_preview_webview(_webview: &Webview) -> Result<bool, String> {
    Ok(false)
}

#[cfg(windows)]
fn resume_preview_webview(webview: &Webview) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
    use windows_core::Interface;

    webview
        .with_webview(|platform_webview| {
            let result = unsafe {
                (|| -> windows_core::Result<()> {
                    platform_webview
                        .controller()
                        .CoreWebView2()?
                        .cast::<ICoreWebView2_3>()?
                        .Resume()
                })()
            };
            if let Err(error) = result {
                tracing::warn!("preview resume failed: {error}");
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn resume_preview_webview(_webview: &Webview) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn reload_without_cache(webview: &Webview) -> Result<(), String> {
    use webview2_com::{CallDevToolsProtocolMethodCompletedHandler, CoTaskMemPWSTR};

    webview
        .with_webview(|platform_webview| {
            let controller = platform_webview.controller();
            let result = unsafe {
                (|| {
                    let core = controller.CoreWebView2()?;
                    let fallback_core = core.clone();
                    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                        move |status, _| status.or_else(|_| fallback_core.Reload()),
                    ));
                    let method = CoTaskMemPWSTR::from("Page.reload");
                    let parameters = CoTaskMemPWSTR::from(r#"{"ignoreCache":true}"#);
                    let method = method.as_ref();
                    let parameters = parameters.as_ref();
                    core.CallDevToolsProtocolMethod(
                        *method.as_pcwstr(),
                        *parameters.as_pcwstr(),
                        &handler,
                    )
                })()
            };
            if let Err(error) = result {
                tracing::warn!(
                    "cache-bypassing preview reload failed, using normal reload: {error}"
                );
                if let Ok(core) = unsafe { controller.CoreWebView2() } {
                    let _ = unsafe { core.Reload() };
                }
            }
        })
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn reload_without_cache(webview: &Webview) -> Result<(), String> {
    webview
        .with_webview(|platform_webview| unsafe {
            let webview: &objc2_web_kit::WKWebView = &*platform_webview.inner().cast();
            let _ = webview.reloadFromOrigin();
        })
        .map_err(|error| error.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn reload_without_cache(webview: &Webview) -> Result<(), String> {
    webview.reload().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_webview_diagnostics(app: tauri::AppHandle) -> PreviewWebviewDiagnostics {
    let surfaces = preview_surfaces().lock().unwrap_or_else(|error| error.into_inner());
    let active_surfaces = surfaces.values().filter(|surface| !surface.parked).count();
    PreviewWebviewDiagnostics {
        live_surfaces: surfaces.len(),
        active_surfaces,
        parked_surfaces: surfaces.len().saturating_sub(active_surfaces),
        maximum_surfaces: preview_pool_labels().len(),
        maintenance_surface: app.get_webview(PREVIEW_CLEAR_LABEL).is_some(),
    }
}

fn preview_webview(app: &tauri::AppHandle, label: &str) -> Result<Webview, String> {
    if !is_preview_pool_label(label) {
        return Err("invalid preview webview label".to_string());
    }
    app.get_webview(label)
        .ok_or_else(|| "preview webview is no longer available".to_string())
}

fn current_claim_token(label: &str) -> Option<u64> {
    preview_surfaces()
        .lock()
        .ok()
        .and_then(|surfaces| surfaces.get(label).map(|surface| surface.claim_token))
}

pub(crate) fn allowed_preview_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|_| "invalid preview URL".to_string())?;
    if preview_url_allowed(&url) {
        Ok(url)
    } else {
        Err("preview URL must use HTTPS or loopback HTTP".to_string())
    }
}

fn browser_profile_directory() -> PathBuf {
    milim_core::paths::Paths::resolve()
        .root()
        .join(BROWSER_PROFILE_DIRECTORY)
}

fn private_browser_profile_directory() -> Result<PathBuf, String> {
    let lease = PRIVATE_PROFILE_LEASE.get_or_init(|| Mutex::new(None));
    let mut lease = lease
        .lock()
        .map_err(|_| "private preview profile lock poisoned".to_string())?;
    if let Some(current) = lease.as_ref() {
        return Ok(current.directory.clone());
    }
    let root = std::env::temp_dir().join(PRIVATE_BROWSER_PROFILE_DIRECTORY);
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = root.join(format!("{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let lock_path = directory.join(PRIVATE_BROWSER_PROFILE_LOCK);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let lock = options.open(&lock_path).map_err(|error| error.to_string())?;
    *lease = Some(PrivateProfileLease {
        directory: directory.clone(),
        _lock: lock,
    });
    Ok(directory)
}

#[cfg(windows)]
fn scavenge_private_browser_profiles() {
    use std::os::windows::fs::OpenOptionsExt;

    let root = std::env::temp_dir().join(PRIVATE_BROWSER_PROFILE_DIRECTORY);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let lock_path = path.join(PRIVATE_BROWSER_PROFILE_LOCK);
        let removable = if lock_path.is_file() {
            OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&lock_path)
                .is_ok()
        } else {
            entry
                .file_name()
                .to_string_lossy()
                .split('-')
                .next()
                .and_then(|value| value.parse::<u32>().ok())
                .is_some_and(|pid| !windows_process_is_running(pid))
        };
        if removable {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[cfg(not(windows))]
fn scavenge_private_browser_profiles() {}

#[cfg(windows)]
fn windows_process_is_running(pid: u32) -> bool {
    use std::ffi::c_void;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return std::io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER);
    }
    let _ = unsafe { CloseHandle(handle) };
    true
}

fn browser_profile_directory_for_mode(
    storage_mode: PreviewBrowserStorageMode,
    label: &str,
) -> Result<PathBuf, String> {
    match storage_mode {
        PreviewBrowserStorageMode::Persistent => Ok(browser_profile_directory()),
        PreviewBrowserStorageMode::Private => {
            let directory = private_browser_profile_directory()?.join(label);
            std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            Ok(directory)
        }
    }
}

fn native_incognito_for_mode(storage_mode: PreviewBrowserStorageMode) -> bool {
    matches!(storage_mode, PreviewBrowserStorageMode::Private)
}

fn preview_url_allowed_for_label(label: &str, url: &Url) -> bool {
    preview_url_allowed(url)
        || (label == PREVIEW_CLEAR_LABEL && url.as_str() == "about:blank")
}

fn preview_new_tab_url_allowed(url: &Url) -> bool {
    preview_url_allowed(url) || url.as_str() == "about:blank"
}

fn preview_shortcut_action(url: &Url, token: u64) -> Option<PreviewWebviewShortcutAction> {
    if url.scheme() != PREVIEW_SHORTCUT_SCHEME || url.host_str()? != token.to_string() {
        return None;
    }
    match url.path() {
        "/new-tab" => Some(PreviewWebviewShortcutAction::NewTab),
        "/close-tab" => Some(PreviewWebviewShortcutAction::CloseTab),
        "/zoom-in" => Some(PreviewWebviewShortcutAction::ZoomIn),
        "/zoom-out" => Some(PreviewWebviewShortcutAction::ZoomOut),
        "/zoom-reset" => Some(PreviewWebviewShortcutAction::ZoomReset),
        _ => None,
    }
}

fn preview_shortcut_script(token: u64) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "event.metaKey && !event.ctrlKey"
    } else {
        "event.ctrlKey && !event.metaKey"
    };
    let wheel_modifier = if cfg!(target_os = "macos") {
        "event.metaKey || event.ctrlKey"
    } else {
        "event.ctrlKey && !event.metaKey"
    };
    format!(
        r#"
(() => {{
  const openShortcut = window.open.bind(window);
  const sendShortcut = (action) => openShortcut("{PREVIEW_SHORTCUT_SCHEME}://{token}/" + action, "_blank");
  window.addEventListener("keydown", (event) => {{
    if (!event.isTrusted || event.repeat || event.altKey || !({modifier})) return;
    const key = event.key.toLowerCase();
    const action = !event.shiftKey && key === "t" ? "new-tab"
      : !event.shiftKey && key === "w" ? "close-tab"
      : key === "+" || key === "=" ? "zoom-in"
      : key === "-" || key === "_" ? "zoom-out"
      : !event.shiftKey && key === "0" ? "zoom-reset"
      : null;
    if (!action) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    sendShortcut(action);
  }}, true);
  let wheelDelta = 0;
  window.addEventListener("wheel", (event) => {{
    if (!event.isTrusted || event.altKey || event.shiftKey || !({wheel_modifier})) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    wheelDelta += event.deltaY;
    if (Math.abs(wheelDelta) < 50) return;
    sendShortcut(wheelDelta < 0 ? "zoom-in" : "zoom-out");
    wheelDelta = 0;
  }}, {{ capture: true, passive: false }});
}})();
"#
    )
}

#[cfg(any(not(windows), test))]
fn preview_audio_mute_script(muted: bool) -> String {
    format!(
        r#"
(() => {{
  const key = "__milimPreviewAudioMute";
  const state = window[key] ||= {{ previous: new WeakMap(), observer: null }};
  const muted = {muted};
  const apply = (root) => {{
    const media = [];
    if (root instanceof HTMLMediaElement) media.push(root);
    if (root?.querySelectorAll) media.push(...root.querySelectorAll("audio,video"));
    for (const element of media) {{
      if (muted) {{
        if (!state.previous.has(element)) state.previous.set(element, element.muted);
        if (!element.muted) element.muted = true;
      }} else if (state.previous.has(element)) {{
        element.muted = state.previous.get(element);
        state.previous.delete(element);
      }}
    }}
  }};
  state.observer?.disconnect();
  state.observer = null;
  apply(document);
  if (muted) {{
    state.observer = new MutationObserver((records) => {{
      for (const record of records) {{
        apply(record.target);
        for (const node of record.addedNodes || []) apply(node);
      }}
    }});
    state.observer.observe(document, {{ childList: true, subtree: true, attributes: true, attributeFilter: ["muted"] }});
  }}
}})();
"#
    )
}

fn preview_url_allowed(url: &Url) -> bool {
    url.scheme() == "https"
        || (url.scheme() == "http" && is_loopback_host(url.host_str().unwrap_or_default()))
}

fn preview_reload_bypasses_cache(url: &Url) -> bool {
    is_loopback_host(url.host_str().unwrap_or_default())
}

fn preview_pool_labels() -> [&'static str; 3] {
    [
        PREVIEW_APP_PRIVATE_LABEL,
        PREVIEW_URL_PRIVATE_LABEL,
        PREVIEW_URL_PERSISTENT_LABEL,
    ]
}

fn is_preview_pool_label(label: &str) -> bool {
    preview_pool_labels().contains(&label)
}

fn preview_label_accepts_storage_mode(
    label: &str,
    storage_mode: PreviewBrowserStorageMode,
) -> bool {
    matches!(
        (label, storage_mode),
        (PREVIEW_URL_PERSISTENT_LABEL, PreviewBrowserStorageMode::Persistent)
            | (PREVIEW_APP_PRIVATE_LABEL, PreviewBrowserStorageMode::Private)
            | (PREVIEW_URL_PRIVATE_LABEL, PreviewBrowserStorageMode::Private)
    )
}

fn validate_preview_bounds(bounds: &PreviewWebviewBounds) -> Result<(), String> {
    if bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width > 0.0
        && bounds.height > 0.0
    {
        Ok(())
    } else {
        Err("invalid preview webview bounds".to_string())
    }
}

fn is_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn preview_surfaces() -> &'static Mutex<HashMap<String, PreviewSurfaceState>> {
    PREVIEW_SURFACES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn preview_lifecycle_lock() -> &'static tokio::sync::Mutex<()> {
    PREVIEW_LIFECYCLE_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn claim_is_current(label: &str, claim_token: u64) -> Result<bool, String> {
    Ok(preview_surfaces()
        .lock()
        .map_err(|_| "preview surface lock poisoned".to_string())?
        .get(label)
        .is_some_and(|surface| surface.claim_token == claim_token))
}

fn require_current_claim(label: &str, claim_token: u64) -> Result<(), String> {
    if claim_is_current(label, claim_token)? {
        Ok(())
    } else {
        Err("preview webview claim is no longer current".to_string())
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1" | "[::1]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_event_claim_tokens_use_the_frontend_contract() {
        let navigation = serde_json::to_value(PreviewWebviewNavigationPayload {
            label: PREVIEW_URL_PERSISTENT_LABEL.to_string(),
            claim_token: 42,
            url: "https://example.com/".to_string(),
            state: "ready",
            message: None,
        })
        .unwrap();
        let title = serde_json::to_value(PreviewWebviewTitlePayload {
            label: PREVIEW_URL_PERSISTENT_LABEL.to_string(),
            claim_token: 42,
            title: "Example".to_string(),
        })
        .unwrap();

        assert_eq!(navigation["claimToken"], 42);
        assert!(navigation.get("claim_token").is_none());
        assert_eq!(title["claimToken"], 42);
        assert!(title.get("claim_token").is_none());
    }

    #[test]
    fn artifact_preview_navigation_restricts_labels_and_urls() {
        assert!(is_preview_pool_label(PREVIEW_APP_PRIVATE_LABEL));
        assert!(!is_preview_pool_label("artifact-browser-session-1"));
        assert!(!is_preview_pool_label("main"));
        assert!(allowed_preview_url("https://example.com/path").is_ok());
        assert!(allowed_preview_url("http://127.0.0.1:4173/").is_ok());
        assert!(allowed_preview_url("http://[::1]:4173/").is_ok());
        assert!(allowed_preview_url("http://example.com/").is_err());
        assert!(allowed_preview_url("javascript:alert(1)").is_err());
        assert!(preview_reload_bypasses_cache(
            &Url::parse("http://localhost:4173/").unwrap()
        ));
        assert!(!preview_reload_bypasses_cache(
            &Url::parse("https://example.com/").unwrap()
        ));
        assert!(preview_new_tab_url_allowed(
            &Url::parse("about:blank").unwrap()
        ));
        assert_eq!(
            preview_shortcut_action(
                &Url::parse("milim-browser-shortcut://42/new-tab").unwrap(),
                42
            ),
            Some(PreviewWebviewShortcutAction::NewTab)
        );
        assert_eq!(
            preview_shortcut_action(
                &Url::parse("milim-browser-shortcut://42/close-tab").unwrap(),
                42
            ),
            Some(PreviewWebviewShortcutAction::CloseTab)
        );
        assert_eq!(
            preview_shortcut_action(
                &Url::parse("milim-browser-shortcut://42/zoom-in").unwrap(),
                42
            ),
            Some(PreviewWebviewShortcutAction::ZoomIn)
        );
        assert_eq!(
            preview_shortcut_action(
                &Url::parse("milim-browser-shortcut://42/zoom-out").unwrap(),
                42
            ),
            Some(PreviewWebviewShortcutAction::ZoomOut)
        );
        assert_eq!(
            preview_shortcut_action(
                &Url::parse("milim-browser-shortcut://42/zoom-reset").unwrap(),
                42
            ),
            Some(PreviewWebviewShortcutAction::ZoomReset)
        );
        let shortcut_script = preview_shortcut_script(42);
        assert!(shortcut_script.contains("milim-browser-shortcut://42/"));
        assert!(shortcut_script.contains("zoom-reset"));
        assert!(shortcut_script.contains("addEventListener(\"wheel\""));
        let mute_script = preview_audio_mute_script(true);
        assert!(mute_script.contains("const muted = true"));
        assert!(mute_script.contains("audio,video"));
        assert!(mute_script.contains("MutationObserver"));
        assert!(preview_shortcut_action(
            &Url::parse("milim-browser-shortcut://41/close-tab").unwrap(),
            42
        )
        .is_none());
        assert!(!preview_url_allowed_for_label(
            "artifact-browser-session-1",
            &Url::parse("about:blank").unwrap()
        ));
        assert!(is_profile_id("browser_session-1"));
        assert!(!is_profile_id("../browser-session"));
        assert!(!preview_url_allowed(
            &Url::parse("http://example.com/redirected").unwrap()
        ));
        assert!(is_preview_pool_label(PREVIEW_APP_PRIVATE_LABEL));
        assert!(is_preview_pool_label(PREVIEW_URL_PRIVATE_LABEL));
        assert!(is_preview_pool_label(PREVIEW_URL_PERSISTENT_LABEL));
        assert!(!is_preview_pool_label("artifact-browser-unbounded-tab"));
        assert!(!is_preview_pool_label(PREVIEW_CLEAR_LABEL));
        assert!(preview_label_accepts_storage_mode(
            PREVIEW_URL_PERSISTENT_LABEL,
            PreviewBrowserStorageMode::Persistent
        ));
        assert!(!preview_label_accepts_storage_mode(
            PREVIEW_URL_PERSISTENT_LABEL,
            PreviewBrowserStorageMode::Private
        ));
        assert!(validate_preview_bounds(&PreviewWebviewBounds {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        })
        .is_ok());

        let persistent_a = browser_profile_directory_for_mode(
            PreviewBrowserStorageMode::Persistent,
            PREVIEW_URL_PERSISTENT_LABEL,
        )
        .unwrap();
        let persistent_b = browser_profile_directory_for_mode(
            PreviewBrowserStorageMode::Persistent,
            PREVIEW_URL_PERSISTENT_LABEL,
        )
        .unwrap();
        assert_eq!(persistent_a, persistent_b);
        assert!(!native_incognito_for_mode(
            PreviewBrowserStorageMode::Persistent
        ));
        assert!(native_incognito_for_mode(
            PreviewBrowserStorageMode::Private
        ));
    }
}
