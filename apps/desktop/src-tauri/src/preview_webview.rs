use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::webview::{NewWindowResponse, PageLoadEvent, PageLoadPayload, WebviewBuilder};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, Runtime, Url, Webview, WebviewUrl};

pub const PREVIEW_WEBVIEW_NAVIGATION_EVENT: &str = "milim://preview-webview-navigation";
pub const PREVIEW_WEBVIEW_NEW_TAB_EVENT: &str = "milim://preview-webview-new-tab";
pub const PREVIEW_WEBVIEW_SHORTCUT_EVENT: &str = "milim://preview-webview-shortcut";
pub const PREVIEW_WEBVIEW_TITLE_EVENT: &str = "milim://preview-webview-title";
const PREVIEW_WEBVIEW_LABEL_PREFIX: &str = "artifact-browser-";
const PREVIEW_CLEAR_LABEL_PREFIX: &str = "artifact-browser-clear-";
const PREVIEW_SHORTCUT_SCHEME: &str = "milim-browser-shortcut";
const BROWSER_PROFILE_DIRECTORY: &str = "browser-profile";
const PRIVATE_BROWSER_PROFILE_DIRECTORY: &str = "milim-private-browser";
const BROWSER_DATA_STORE_IDENTIFIER: [u8; 16] = *b"milim-browser-v1";
static NEXT_BROWSER_WINDOW_ID: AtomicU64 = AtomicU64::new(1);
static PRIVATE_WEBVIEW_PROFILES: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

#[derive(Clone, Copy, Deserialize)]
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct PreviewWebviewNavigationPayload {
    label: String,
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
    action: PreviewWebviewShortcutAction,
}

#[derive(Clone, Serialize)]
struct PreviewWebviewTitlePayload {
    label: String,
    title: String,
}

pub fn handle_page_load(webview: &Webview, payload: &PageLoadPayload<'_>) {
    if !is_preview_label(webview.label()) {
        return;
    }
    let state = match payload.event() {
        PageLoadEvent::Started => "loading",
        PageLoadEvent::Finished => "ready",
    };
    let event = PreviewWebviewNavigationPayload {
        label: webview.label().to_string(),
        url: payload.url().to_string(),
        state,
        message: None,
    };
    emit_navigation(webview, event);
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("preview-webview-navigation")
        .on_navigation(|webview, url| {
            if !is_preview_label(webview.label()) {
                return true;
            }
            let allowed = preview_url_allowed_for_label(webview.label(), url);
            emit_navigation(
                webview,
                PreviewWebviewNavigationPayload {
                    label: webview.label().to_string(),
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
) -> Result<(), String> {
    if !is_preview_label(&label) || label.starts_with(PREVIEW_CLEAR_LABEL_PREFIX) {
        return Err("invalid preview webview label".to_string());
    }
    if !is_profile_id(&profile_id) {
        return Err("invalid preview browser profile id".to_string());
    }
    let url = allowed_preview_url(&url)?;
    let window = app
        .get_window("main")
        .ok_or_else(|| "main window is no longer available".to_string())?;
    let private = matches!(storage_mode, PreviewBrowserStorageMode::Private);
    let profile_directory = browser_profile_directory_for_mode(storage_mode, &profile_id);
    let popup_app = app.clone();
    let opener_label = label.clone();
    let shortcut_token = NEXT_BROWSER_WINDOW_ID.fetch_add(1, Ordering::Relaxed);
    let builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(url))
        .focused(true)
        .incognito(native_incognito_for_mode(storage_mode))
        .data_directory(profile_directory.clone())
        .data_store_identifier(BROWSER_DATA_STORE_IDENTIFIER)
        // Route zoom shortcuts through the main UI so their value can be persisted
        // and restored when this child webview is recreated.
        .zoom_hotkeys_enabled(false)
        .initialization_script(preview_shortcut_script(shortcut_token))
        .on_document_title_changed(|webview, title| {
            if let Some(main) = webview.get_webview("main") {
                let _ = main.emit(
                    PREVIEW_WEBVIEW_TITLE_EVENT,
                    PreviewWebviewTitlePayload {
                        label: webview.label().to_string(),
                        title,
                    },
                );
            }
        })
        .on_new_window(move |url, _features| {
            if let Some(action) = preview_shortcut_action(&url, shortcut_token) {
                if let Some(main) = popup_app.get_webview("main") {
                    let _ = main.emit(
                        PREVIEW_WEBVIEW_SHORTCUT_EVENT,
                        PreviewWebviewShortcutPayload {
                            label: opener_label.clone(),
                            action,
                        },
                    );
                }
                return NewWindowResponse::Deny;
            }
            if !preview_new_tab_url_allowed(&url) {
                return NewWindowResponse::Deny;
            }
            if let Some(main) = popup_app.get_webview("main") {
                let _ = main.emit(
                    PREVIEW_WEBVIEW_NEW_TAB_EVENT,
                    PreviewWebviewNewTabPayload {
                        request_id: NEXT_BROWSER_WINDOW_ID.fetch_add(1, Ordering::Relaxed),
                        opener_label: opener_label.clone(),
                        url: url.to_string(),
                    },
                );
            }
            NewWindowResponse::Deny
        });
    let result = window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width.max(1.0), bounds.height.max(1.0)),
        )
        .map(|_| ())
        .map_err(|error| error.to_string());
    if result.is_ok() && private {
        private_webview_profiles()
            .lock()
            .expect("private preview profile lock poisoned")
            .insert(label, profile_directory);
    }
    result
}

#[tauri::command]
pub async fn preview_webview_clear_data(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_window("main")
        .ok_or_else(|| "main window is no longer available".to_string())?;
    let label = format!(
        "{PREVIEW_CLEAR_LABEL_PREFIX}{}",
        NEXT_BROWSER_WINDOW_ID.fetch_add(1, Ordering::Relaxed)
    );
    let builder = WebviewBuilder::new(
        label,
        WebviewUrl::External(Url::parse("about:blank").expect("valid internal URL")),
    )
    .focused(false)
    .data_directory(browser_profile_directory())
    .data_store_identifier(BROWSER_DATA_STORE_IDENTIFIER);
    let webview = window
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| error.to_string())?;
    let _ = webview.hide();
    let result = webview
        .clear_all_browsing_data()
        .map_err(|error| error.to_string());
    let _ = webview.close();
    result
}

#[tauri::command]
pub fn preview_webview_close(app: tauri::AppHandle, label: String) -> Result<(), String> {
    if !is_preview_label(&label) || label.starts_with(PREVIEW_CLEAR_LABEL_PREFIX) {
        return Err("invalid preview webview label".to_string());
    }
    if let Some(webview) = app.get_webview(&label) {
        webview.close().map_err(|error| error.to_string())?;
    }
    release_private_webview_profile(&label);
    Ok(())
}

fn emit_navigation<R: Runtime>(webview: &Webview<R>, event: PreviewWebviewNavigationPayload) {
    if let Some(main) = webview.get_webview("main") {
        let _ = main.emit(PREVIEW_WEBVIEW_NAVIGATION_EVENT, event);
    }
}

#[tauri::command]
pub fn preview_webview_navigate(
    app: tauri::AppHandle,
    label: String,
    url: String,
) -> Result<(), String> {
    let webview = preview_webview(&app, &label)?;
    let url = allowed_preview_url(&url)?;
    webview.navigate(url).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_webview_reload(app: tauri::AppHandle, label: String) -> Result<(), String> {
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
pub fn preview_webview_set_muted(
    app: tauri::AppHandle,
    label: String,
    muted: bool,
) -> Result<(), String> {
    set_preview_webview_muted(&preview_webview(&app, &label)?, muted)
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
pub fn preview_webview_history(
    app: tauri::AppHandle,
    label: String,
    delta: i32,
) -> Result<(), String> {
    if delta != -1 && delta != 1 {
        return Err("preview history delta must be -1 or 1".to_string());
    }
    preview_webview(&app, &label)?
        .eval(format!("history.go({delta})"))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_webview_url(app: tauri::AppHandle, label: String) -> Result<String, String> {
    preview_webview(&app, &label)?
        .url()
        .map(|url| url.to_string())
        .map_err(|error| error.to_string())
}

fn preview_webview(app: &tauri::AppHandle, label: &str) -> Result<Webview, String> {
    if !is_preview_label(label) {
        return Err("invalid preview webview label".to_string());
    }
    app.get_webview(label)
        .ok_or_else(|| "preview webview is no longer available".to_string())
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

fn browser_profile_directory_for_mode(
    storage_mode: PreviewBrowserStorageMode,
    profile_id: &str,
) -> PathBuf {
    match storage_mode {
        PreviewBrowserStorageMode::Persistent => browser_profile_directory(),
        PreviewBrowserStorageMode::Private => std::env::temp_dir()
            .join(PRIVATE_BROWSER_PROFILE_DIRECTORY)
            .join(format!("{}-{profile_id}", std::process::id())),
    }
}

fn native_incognito_for_mode(storage_mode: PreviewBrowserStorageMode) -> bool {
    matches!(storage_mode, PreviewBrowserStorageMode::Private)
}

fn preview_url_allowed_for_label(label: &str, url: &Url) -> bool {
    preview_url_allowed(url)
        || (label.starts_with(PREVIEW_CLEAR_LABEL_PREFIX) && url.as_str() == "about:blank")
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

fn is_preview_label(label: &str) -> bool {
    label.starts_with(PREVIEW_WEBVIEW_LABEL_PREFIX)
        && label[PREVIEW_WEBVIEW_LABEL_PREFIX.len()..]
            .chars()
            .all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':')
            })
}

fn is_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn private_webview_profiles() -> &'static Mutex<HashMap<String, PathBuf>> {
    PRIVATE_WEBVIEW_PROFILES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn release_private_webview_profile(label: &str) {
    let directory = {
        let mut profiles = private_webview_profiles()
            .lock()
            .expect("private preview profile lock poisoned");
        let Some(directory) = profiles.remove(label) else {
            return;
        };
        (!profiles.values().any(|profile| profile == &directory)).then_some(directory)
    };
    let Some(directory) = directory else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let still_unused = private_webview_profiles()
            .lock()
            .map(|profiles| !profiles.values().any(|profile| profile == &directory))
            .unwrap_or(false);
        if still_unused {
            let _ = std::fs::remove_dir_all(directory);
        }
    });
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
    fn artifact_preview_navigation_restricts_labels_and_urls() {
        assert!(is_preview_label("artifact-browser-session-1"));
        assert!(!is_preview_label("main"));
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

        let persistent_a =
            browser_profile_directory_for_mode(PreviewBrowserStorageMode::Persistent, "profile-a");
        let persistent_b =
            browser_profile_directory_for_mode(PreviewBrowserStorageMode::Persistent, "profile-b");
        let private_a =
            browser_profile_directory_for_mode(PreviewBrowserStorageMode::Private, "profile-a");
        let private_a_tab =
            browser_profile_directory_for_mode(PreviewBrowserStorageMode::Private, "profile-a");
        let private_b =
            browser_profile_directory_for_mode(PreviewBrowserStorageMode::Private, "profile-b");
        assert_eq!(persistent_a, persistent_b);
        assert_eq!(private_a, private_a_tab);
        assert_ne!(private_a, private_b);
        assert_ne!(persistent_a, private_a);
        assert!(!native_incognito_for_mode(
            PreviewBrowserStorageMode::Persistent
        ));
        assert!(native_incognito_for_mode(
            PreviewBrowserStorageMode::Private
        ));
    }
}
