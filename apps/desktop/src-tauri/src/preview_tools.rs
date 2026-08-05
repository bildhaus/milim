use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use milim_core::{Error, Result};
use milim_tools::{Tool, ToolEffect};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, Wry};

const PREVIEW_EVAL_TIMEOUT: Duration = Duration::from_secs(3);
pub const PREVIEW_OPEN_URL_EVENT: &str = "milim://preview-open-url";

const PREVIEW_ACTION_JS: &str = r##"
try {
  const rootWindow = window;
  const rootDocument = document;
  const frame = __milimNative ? null : rootDocument.querySelector(".preview-frame");
  const W = __milimNative ? rootWindow : frame?.contentWindow;
  const D = __milimNative ? rootDocument : frame?.contentDocument;
  if (!W || !D) return { ok: false, error: "No active preview document is available." };

  const textOf = (el) => (el?.innerText || el?.textContent || el?.getAttribute?.("aria-label") || el?.value || "").trim();
  const describe = (el) => {
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    return {
      tag: el.tagName?.toLowerCase?.() || "",
      text: textOf(el).slice(0, 160),
      id: el.id || null,
      testid: el.getAttribute?.("data-testid") || null,
      role: el.getAttribute?.("role") || null,
      aria: el.getAttribute?.("aria-label") || null,
      rect: { x: Math.round(rect.x), y: Math.round(rect.y), width: Math.round(rect.width), height: Math.round(rect.height) }
    };
  };
  const point = () => {
    let x = Number(__milimArgs.x ?? __milimArgs.client_x ?? 0);
    let y = Number(__milimArgs.y ?? __milimArgs.client_y ?? 0);
    if (x > 0 && x <= 1) x *= W.innerWidth;
    if (y > 0 && y <= 1) y *= W.innerHeight;
    return { x, y };
  };
  const byText = (text) => {
    if (!text) return null;
    const needle = String(text).toLowerCase();
    return Array.from(D.querySelectorAll("button,a,input,textarea,select,label,[role],[aria-label],[data-testid],[onclick]"))
      .find((el) => textOf(el).toLowerCase().includes(needle)) || null;
  };
  const pick = () => {
    if (__milimArgs.selector) return D.querySelector(String(__milimArgs.selector));
    if (__milimArgs.text) return byText(__milimArgs.text);
    const p = point();
    return D.elementFromPoint(p.x, p.y);
  };
  const mouse = (el, kind) => {
    const rect = el.getBoundingClientRect();
    const p = __milimArgs.x != null && __milimArgs.y != null ? point() : { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    el.dispatchEvent(new MouseEvent(kind, { bubbles: true, cancelable: true, clientX: p.x, clientY: p.y, button: 0 }));
  };

  const cleanupPicker = () => {
    const picker = W.__milimPreviewPicker;
    if (!picker) return;
    D.removeEventListener("pointerover", picker.hover, true);
    D.removeEventListener("click", picker.click, true);
    D.removeEventListener("keydown", picker.key, true);
    picker.active?.style?.removeProperty("outline");
    W.clearTimeout(picker.timer);
  };
  const stableSelector = (el) => {
    if (el.id) return "#" + W.CSS.escape(el.id);
    const testId = el.getAttribute?.("data-testid");
    if (testId) return '[data-testid="' + W.CSS.escape(testId) + '"]';
    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && parts.length < 6) {
      let part = node.tagName.toLowerCase();
      const peers = node.parentElement ? Array.from(node.parentElement.children).filter((item) => item.tagName === node.tagName) : [];
      if (peers.length > 1) part += ":nth-of-type(" + (peers.indexOf(node) + 1) + ")";
      parts.unshift(part); node = node.parentElement;
    }
    return parts.join(" > ");
  };

  if (__milimAction === "annotate_start") {
    cleanupPicker();
    const picker = { active: null, selection: null, timer: 0 };
    picker.hover = (event) => {
      picker.active?.style?.removeProperty("outline");
      picker.active = event.target instanceof W.Element ? event.target : null;
      picker.active?.style?.setProperty("outline", "2px solid #7c5cff", "important");
    };
    picker.click = (event) => {
      if (!(event.target instanceof W.Element)) return;
      event.preventDefault(); event.stopImmediatePropagation();
      const el = event.target; const rect = el.getBoundingClientRect();
      picker.selection = {
        url: W.location.href, title: D.title || "", selector: stableSelector(el),
        tag: el.tagName.toLowerCase(), id: el.id || null,
        testId: el.getAttribute("data-testid"), role: el.getAttribute("role"),
        visibleText: textOf(el).slice(0, 500), outerHtml: el.outerHTML.slice(0, 2048),
        rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
      };
      cleanupPicker();
    };
    picker.key = (event) => { if (event.key === "Escape") { cleanupPicker(); W.__milimPreviewPicker = null; } };
    picker.timer = W.setTimeout(() => { cleanupPicker(); W.__milimPreviewPicker = null; }, 30000);
    W.__milimPreviewPicker = picker;
    D.addEventListener("pointerover", picker.hover, true);
    D.addEventListener("click", picker.click, true);
    D.addEventListener("keydown", picker.key, true);
    return { ok: true, started: true };
  }
  if (__milimAction === "annotate_take") {
    const selection = W.__milimPreviewPicker?.selection || null;
    if (selection) W.__milimPreviewPicker = null;
    return { ok: true, selection };
  }
  if (__milimAction === "annotate_cancel") {
    cleanupPicker(); W.__milimPreviewPicker = null;
    return { ok: true, canceled: true };
  }

  if (__milimAction === "snapshot") {
    const max = Math.min(Math.max(Number(__milimArgs.max_chars) || 12000, 1000), 40000);
    const elements = Array.from(D.querySelectorAll("button,a,input,textarea,select,[role],[aria-label],[data-testid]"))
      .slice(0, 100)
      .map(describe);
    return {
      ok: true,
      url: W.location.href,
      title: D.title || "",
      text: (D.body?.innerText || "").trim().slice(0, max),
      elements
    };
  }

  if (__milimAction === "click") {
    const el = pick();
    if (!el) return { ok: false, error: "No preview element matched the click target." };
    mouse(el, "mousemove");
    mouse(el, "mousedown");
    mouse(el, "mouseup");
    mouse(el, "click");
    return { ok: true, clicked: describe(el) };
  }

  if (__milimAction === "type_text") {
    const text = String(__milimArgs.text ?? "");
    const hasTarget = Boolean(__milimArgs.selector) || (__milimArgs.x != null && __milimArgs.y != null);
    const el = (hasTarget ? pick() : null) || D.activeElement;
    if (!el) return { ok: false, error: "No preview element matched the typing target." };
    el.focus?.();
    if (el instanceof W.HTMLInputElement || el instanceof W.HTMLTextAreaElement) {
      const value = `${el.value ?? ""}${text}`;
      const proto = el instanceof W.HTMLTextAreaElement ? W.HTMLTextAreaElement.prototype : W.HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
      if (setter) setter.call(el, value); else el.value = value;
      el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (el.isContentEditable) {
      el.textContent = `${el.textContent ?? ""}${text}`;
      el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
    } else {
      return { ok: false, error: "Matched element is not text-editable.", target: describe(el) };
    }
    return { ok: true, typed: text.length, target: describe(el) };
  }

  if (__milimAction === "key_press") {
    const key = String(__milimArgs.key ?? "");
    const el = pick() || D.activeElement || D.body;
    el.focus?.();
    el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    el.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true }));
    return { ok: true, pressed: key, target: describe(el) };
  }

  if (__milimAction === "scroll") {
    const el = __milimArgs.selector || __milimArgs.text ? pick() : null;
    const x = Number(__milimArgs.x ?? __milimArgs.scroll_x ?? 0);
    const y = Number(__milimArgs.y ?? __milimArgs.scroll_y ?? __milimArgs.amount ?? 0);
    if (el?.scrollBy) el.scrollBy({ left: x, top: y, behavior: "auto" });
    else W.scrollBy({ left: x, top: y, behavior: "auto" });
    return { ok: true, scrolled: { x, y }, target: el ? describe(el) : "window" };
  }

  return { ok: false, error: `Unknown preview action: ${__milimAction}` };
} catch (error) {
  return { ok: false, error: String(error?.message || error), stack: String(error?.stack || "") };
}
"##;

#[derive(Clone, Debug)]
struct PreviewTarget {
    label: Option<String>,
    title: Option<String>,
    url: Option<String>,
    native: bool,
    kind: String,
    status: String,
    capabilities: Vec<String>,
}

#[derive(Default)]
pub struct PreviewToolState {
    app: RwLock<Option<AppHandle<Wry>>>,
    target: RwLock<Option<PreviewTarget>>,
}

pub type SharedPreviewToolState = Arc<PreviewToolState>;

impl PreviewToolState {
    pub fn set_app(&self, app: AppHandle<Wry>) {
        if let Ok(mut current) = self.app.write() {
            *current = Some(app);
        }
    }

    fn snapshot(&self) -> SharedPreviewToolState {
        Arc::new(Self {
            app: RwLock::new(self.app.read().ok().and_then(|value| value.clone())),
            target: RwLock::new(self.target.read().ok().and_then(|value| value.clone())),
        })
    }

    // ponytail: Tauri target payload is flat; wrap it only if this grows.
    #[allow(clippy::too_many_arguments)]
    pub fn set_target(
        &self,
        label: Option<String>,
        title: Option<String>,
        url: Option<String>,
        native: bool,
        kind: Option<String>,
        status: Option<String>,
        capabilities: Option<Vec<String>>,
    ) {
        if let Ok(mut current) = self.target.write() {
            *current = if label.is_none() && kind.is_none() {
                None
            } else {
                Some(PreviewTarget {
                    label,
                    title,
                    url,
                    native,
                    kind: kind.unwrap_or_else(|| {
                        if native {
                            "native_browser".to_string()
                        } else {
                            "artifact_iframe".to_string()
                        }
                    }),
                    status: status.unwrap_or_else(|| "ready".to_string()),
                    capabilities: capabilities.unwrap_or_else(default_preview_capabilities),
                })
            };
        }
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn set_active_preview_target(
    state: tauri::State<'_, SharedPreviewToolState>,
    label: Option<String>,
    title: Option<String>,
    url: Option<String>,
    native: bool,
    kind: Option<String>,
    status: Option<String>,
    capabilities: Option<Vec<String>>,
) {
    state.set_target(label, title, url, native, kind, status, capabilities);
}

pub fn preview_tools(state: SharedPreviewToolState) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(PreviewOpenUrlTool {
            state: state.clone(),
        }),
        Arc::new(PreviewDomSnapshotTool {
            state: state.clone(),
        }),
        Arc::new(PreviewClickTool {
            state: state.clone(),
        }),
        Arc::new(PreviewTypeTextTool {
            state: state.clone(),
        }),
        Arc::new(PreviewKeyPressTool {
            state: state.clone(),
        }),
        Arc::new(PreviewScrollTool { state }),
    ]
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewOpenUrlArgs {
    url: String,
}

#[derive(Clone, Serialize)]
struct PreviewOpenUrlPayload {
    url: String,
}

pub struct PreviewOpenUrlTool {
    state: SharedPreviewToolState,
}

#[async_trait]
impl Tool for PreviewOpenUrlTool {
    fn name(&self) -> &str {
        "preview_open_url"
    }

    fn description(&self) -> &str {
        "Open an HTTPS or loopback HTTP URL in Milim's in-app Preview inspector. Use this instead of the OpenAI Browser plugin when the user asks for Milim's in-app browser."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "HTTPS URL or loopback HTTP URL to open." }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: PreviewOpenUrlArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid preview_open_url arguments: {error}"))
        })?;
        let url = crate::preview_webview::allowed_preview_url(&args.url)
            .map_err(Error::InvalidRequest)?;
        let app = self
            .state
            .app
            .read()
            .ok()
            .and_then(|value| value.clone())
            .ok_or_else(|| Error::Other("Milim desktop preview is unavailable".to_string()))?;
        let url = url.to_string();
        // ponytail: Open the visible task; add run-id routing if hidden background turns need this.
        app.emit(
            PREVIEW_OPEN_URL_EVENT,
            PreviewOpenUrlPayload { url: url.clone() },
        )
        .map_err(|error| Error::Other(format!("failed to open Milim preview: {error}")))?;
        Ok(json!({ "status": "requested", "url": url }))
    }
}

async fn run_preview_action(
    state: &SharedPreviewToolState,
    action: &str,
    args: Value,
) -> Result<Value> {
    let target = state
        .target
        .read()
        .ok()
        .and_then(|target| target.clone())
        .ok_or_else(|| {
            Error::InvalidRequest("No active preview panel is available.".to_string())
        })?;
    let capability = capability_for_action(action);
    if target.status != "ready" || !target.capabilities.iter().any(|item| item == capability) {
        return Err(Error::InvalidRequest(format!(
            "Active preview surface is not inspectable: {} ({})",
            target.kind, target.status
        )));
    }
    let app = state
        .app
        .read()
        .ok()
        .and_then(|app| app.clone())
        .ok_or_else(|| Error::InvalidRequest("Preview tools are not ready yet.".to_string()))?;
    let label = target.label.as_deref().ok_or_else(|| {
        Error::InvalidRequest("Active preview surface has no webview target.".to_string())
    })?;
    let webview = app.get_webview(label).ok_or_else(|| {
        Error::InvalidRequest("The active preview webview is no longer available.".to_string())
    })?;
    let action_json = serde_json::to_string(action).map_err(|e| Error::Other(e.to_string()))?;
    let args_json = serde_json::to_string(&args).map_err(|e| Error::Other(e.to_string()))?;
    let script = format!(
        "(() => {{ const __milimAction = {action_json}; const __milimArgs = {args_json}; const __milimNative = {}; {PREVIEW_ACTION_JS} }})()",
        if target.native { "true" } else { "false" },
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    webview
        .eval_with_callback(script, move |raw| {
            if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                let _ = tx.send(raw);
            }
        })
        .map_err(|e| Error::Other(format!("preview eval failed: {e}")))?;
    let raw = tokio::time::timeout(PREVIEW_EVAL_TIMEOUT, rx)
        .await
        .map_err(|_| Error::Other("preview tool timed out".to_string()))?
        .map_err(|_| Error::Other("preview callback closed".to_string()))?;
    let mut result = serde_json::from_str::<Value>(&raw)
        .map_err(|error| Error::Other(format!("invalid preview response: {error}")))?;
    if let Value::Object(map) = &mut result {
        map.insert("preview_url".to_string(), json!(target.url));
        map.insert("preview_surface".to_string(), target_metadata_json(&target));
    }
    Ok(result)
}

fn default_preview_capabilities() -> Vec<String> {
    [
        "dom_snapshot",
        "click",
        "type",
        "key",
        "scroll",
        "logs",
        "annotate",
        "source",
    ]
    .iter()
    .map(|capability| (*capability).to_string())
    .collect()
}

#[tauri::command]
pub async fn preview_picker_start(
    state: tauri::State<'_, SharedPreviewToolState>,
) -> std::result::Result<Value, String> {
    run_preview_action(&state, "annotate_start", json!({}))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_picker_take(
    state: tauri::State<'_, SharedPreviewToolState>,
) -> std::result::Result<Value, String> {
    run_preview_action(&state, "annotate_take", json!({}))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_picker_cancel(
    state: tauri::State<'_, SharedPreviewToolState>,
) -> std::result::Result<Value, String> {
    run_preview_action(&state, "annotate_cancel", json!({}))
        .await
        .map_err(|error| error.to_string())
}

fn capability_for_action(action: &str) -> &str {
    match action {
        "snapshot" => "dom_snapshot",
        "type_text" => "type",
        "key_press" => "key",
        other => other,
    }
}

fn target_metadata_json(target: &PreviewTarget) -> Value {
    json!({
        "label": target.label.clone(),
        "title": target.title.clone(),
        "url": target.url.clone(),
        "native": target.native,
        "kind": target.kind.clone(),
        "status": target.status.clone(),
        "capabilities": target.capabilities.clone(),
    })
}

#[derive(Debug, Deserialize)]
struct DomSnapshotArgs {
    #[serde(default)]
    max_chars: Option<u32>,
}

pub struct PreviewDomSnapshotTool {
    state: SharedPreviewToolState,
}

#[async_trait]
impl Tool for PreviewDomSnapshotTool {
    fn name(&self) -> &str {
        "preview_dom_snapshot"
    }

    fn description(&self) -> &str {
        "Inspect the active Milim preview side panel. Returns visible text, URL/title, and up to 100 clickable/form elements. Always scoped to the preview; it does not read the OS screen."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "max_chars": { "type": "integer", "description": "Maximum visible text characters to return, default 12000." }
            }
        })
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            state: self.state.snapshot(),
        }))
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: DomSnapshotArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid preview_dom_snapshot arguments: {error}"))
        })?;
        run_preview_action(
            &self.state,
            "snapshot",
            json!({ "max_chars": args.max_chars }),
        )
        .await
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PreviewTargetArgs {
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
}

pub struct PreviewClickTool {
    state: SharedPreviewToolState,
}

#[async_trait]
impl Tool for PreviewClickTool {
    fn name(&self) -> &str {
        "preview_click"
    }

    fn description(&self) -> &str {
        "Click inside the active Milim preview side panel by CSS selector, visible text, or x/y preview coordinates. Does not move the OS mouse."
    }

    fn input_schema(&self) -> Value {
        target_schema()
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            state: self.state.snapshot(),
        }))
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args = serde_json::to_value(normalize_target_args(args)?)
            .map_err(|e| Error::Other(e.to_string()))?;
        run_preview_action(&self.state, "click", args).await
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct TypeTextArgs {
    text: String,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
}

pub struct PreviewTypeTextTool {
    state: SharedPreviewToolState,
}

#[async_trait]
impl Tool for PreviewTypeTextTool {
    fn name(&self) -> &str {
        "preview_type_text"
    }

    fn description(&self) -> &str {
        "Type text into a focused or targeted field inside the active Milim preview side panel. Does not use the OS keyboard."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" },
                "selector": { "type": "string", "description": "Optional CSS selector for the input/textarea/contenteditable target." },
                "x": { "type": "number", "description": "Optional x coordinate in preview pixels or 0-1 relative position." },
                "y": { "type": "number", "description": "Optional y coordinate in preview pixels or 0-1 relative position." }
            },
            "required": ["text"]
        })
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            state: self.state.snapshot(),
        }))
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: TypeTextArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid preview_type_text arguments: {e}"))
        })?;
        validate_coordinate_pair(args.x, args.y)?;
        let args = serde_json::to_value(args).map_err(|e| Error::Other(e.to_string()))?;
        run_preview_action(&self.state, "type_text", args).await
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct KeyPressArgs {
    key: String,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

pub struct PreviewKeyPressTool {
    state: SharedPreviewToolState,
}

#[async_trait]
impl Tool for PreviewKeyPressTool {
    fn name(&self) -> &str {
        "preview_key_press"
    }

    fn description(&self) -> &str {
        "Dispatch a key press inside the active Milim preview side panel. Does not use the OS keyboard."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Key value such as Enter, Escape, Tab, ArrowDown, or a character." },
                "selector": { "type": "string" },
                "text": { "type": "string", "description": "Optional visible text target." }
            },
            "required": ["key"]
        })
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            state: self.state.snapshot(),
        }))
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: KeyPressArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid preview_key_press arguments: {e}"))
        })?;
        let args = serde_json::to_value(args).map_err(|e| Error::Other(e.to_string()))?;
        run_preview_action(&self.state, "key_press", args).await
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ScrollArgs {
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    amount: Option<f64>,
}

pub struct PreviewScrollTool {
    state: SharedPreviewToolState,
}

#[async_trait]
impl Tool for PreviewScrollTool {
    fn name(&self) -> &str {
        "preview_scroll"
    }

    fn description(&self) -> &str {
        "Scroll the active Milim preview side panel or a matched scrollable element. Does not use the OS mouse wheel."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string" },
                "text": { "type": "string" },
                "x": { "type": "number", "description": "Horizontal pixels to scroll." },
                "y": { "type": "number", "description": "Vertical pixels to scroll." },
                "amount": { "type": "number", "description": "Vertical pixels to scroll when y is omitted." }
            }
        })
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            state: self.state.snapshot(),
        }))
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: ScrollArgs = serde_json::from_value(args)
            .map_err(|e| Error::InvalidRequest(format!("invalid preview_scroll arguments: {e}")))?;
        let args = serde_json::to_value(args).map_err(|e| Error::Other(e.to_string()))?;
        run_preview_action(&self.state, "scroll", args).await
    }
}

fn normalize_target_args(args: Value) -> Result<PreviewTargetArgs> {
    let args: PreviewTargetArgs = serde_json::from_value(args)
        .map_err(|e| Error::InvalidRequest(format!("invalid preview target arguments: {e}")))?;
    validate_coordinate_pair(args.x, args.y)?;
    Ok(args)
}

fn validate_coordinate_pair(x: Option<f64>, y: Option<f64>) -> Result<()> {
    if x.is_some() != y.is_some() {
        return Err(Error::InvalidRequest(
            "preview coordinates require both x and y".into(),
        ));
    }
    if x.is_some_and(|value| !value.is_finite()) || y.is_some_and(|value| !value.is_finite()) {
        return Err(Error::InvalidRequest(
            "preview coordinates must be finite numbers".into(),
        ));
    }
    Ok(())
}

fn target_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "selector": { "type": "string", "description": "CSS selector inside the preview document." },
            "text": { "type": "string", "description": "Visible text to match when selector is omitted." },
            "x": { "type": "number", "description": "Preview x coordinate in pixels, or 0-1 for relative position." },
            "y": { "type": "number", "description": "Preview y coordinate in pixels, or 0-1 for relative position." }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_preview_tools() {
        let state = Arc::new(PreviewToolState::default());
        let mut names: Vec<String> = preview_tools(state)
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "preview_click".to_string(),
                "preview_dom_snapshot".to_string(),
                "preview_key_press".to_string(),
                "preview_open_url".to_string(),
                "preview_scroll".to_string(),
                "preview_type_text".to_string()
            ]
        );
    }

    #[test]
    fn target_schema_supports_selector_text_and_coordinates() {
        let schema = target_schema();
        assert!(schema["properties"]["selector"].is_object());
        assert!(schema["properties"]["text"].is_object());
        assert!(schema["properties"]["x"].is_object());
        assert!(schema["properties"]["y"].is_object());
    }

    #[test]
    fn preview_state_stores_surface_metadata_and_clears() {
        let state = PreviewToolState::default();
        state.set_target(
            Some("main".to_string()),
            Some("index.html".to_string()),
            Some("index.html".to_string()),
            false,
            Some("artifact_iframe".to_string()),
            Some("ready".to_string()),
            Some(vec!["dom_snapshot".to_string(), "source".to_string()]),
        );
        let target = state.target.read().unwrap().clone().unwrap();
        assert_eq!(target.label.as_deref(), Some("main"));
        assert_eq!(target.title.as_deref(), Some("index.html"));
        assert_eq!(target.kind, "artifact_iframe");
        assert_eq!(target.status, "ready");
        assert!(target.capabilities.contains(&"dom_snapshot".to_string()));

        state.set_target(None, None, None, false, None, None, None);
        assert!(state.target.read().unwrap().is_none());
    }

    #[test]
    fn preview_action_rejects_non_inspectable_surface() {
        let state = Arc::new(PreviewToolState::default());
        state.set_target(
            None,
            Some("Browser".to_string()),
            None,
            false,
            Some("blank".to_string()),
            Some("not_inspectable".to_string()),
            Some(vec![]),
        );
        let err = tauri::async_runtime::block_on(run_preview_action(&state, "snapshot", json!({})))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not inspectable"), "unexpected error: {err}");
    }
}
