import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const packageJson = JSON.parse(
  readFileSync(join(root, "package.json"), "utf8"),
);
const config = JSON.parse(
  readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const cargoToml = readFileSync(join(root, "src-tauri", "Cargo.toml"), "utf8");
const tauriLib = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");
const repoVersion = readFileSync(
  join(root, "..", "..", "VERSION"),
  "utf8",
).trim();
const capabilities = JSON.parse(
  readFileSync(join(root, "src-tauri", "capabilities", "default.json"), "utf8"),
);
const topBar = readFileSync(
  join(root, "src", "components", "TopBar.tsx"),
  "utf8",
);
const windowControls = readFileSync(
  join(root, "src", "components", "WindowControls.tsx"),
  "utf8",
);
const resizeHandles = readFileSync(
  join(root, "src", "components", "ResizeHandles.tsx"),
  "utf8",
);
const previewPanel = readFileSync(
  join(root, "src", "components", "PreviewPanel.tsx"),
  "utf8",
);
const chatView = readFileSync(
  join(root, "src", "components", "ChatView.tsx"),
  "utf8",
);
const sidebar = readFileSync(
  join(root, "src", "components", "Sidebar.tsx"),
  "utf8",
);
const styles = readFileSync(join(root, "src", "styles.css"), "utf8");
const nativePreviewBlockerFiles = [
  ["App.tsx", 2],
  [join("components", "SheetDialog.tsx"), 1],
  [join("components", "ChatSearchPopover.tsx"), 1],
  [join("components", "ContextMenu.tsx"), 1],
  [join("components", "ChatView.tsx"), 1],
  [join("components", "ModelPicker.tsx"), 1],
  [join("components", "GitPanel.tsx"), 3],
  [join("components", "TopBar.tsx"), 1],
];
const cargoVersion = cargoToml.match(/^version = "([^"]+)"$/m)?.[1];
const tauriFeatures =
  cargoToml.match(
    /^tauri = \{ version = "2", features = \[([^\]]*)\] \}$/m,
  )?.[1] ?? "";

if (config.productName !== "milim") {
  throw new Error(`Tauri productName must be milim, got ${config.productName}`);
}

if (config.identifier !== "com.omershatz.milim") {
  throw new Error(
    `Tauri bundle identifier must be com.omershatz.milim, got ${config.identifier}`,
  );
}

if (config.app?.windows?.[0]?.title !== "milim") {
  throw new Error(
    `Tauri main window title must be milim, got ${config.app?.windows?.[0]?.title}`,
  );
}

if (String(config.identifier).endsWith(".app")) {
  throw new Error("Tauri bundle identifier must not end with .app");
}

if (config.version !== packageJson.version) {
  throw new Error(
    `Tauri version ${config.version} must match package.json ${packageJson.version}`,
  );
}

if (cargoVersion !== packageJson.version) {
  throw new Error(
    `Tauri Cargo version ${cargoVersion ?? "(missing)"} must match package.json ${packageJson.version}`,
  );
}

if (!tauriFeatures.includes('"unstable"')) {
  throw new Error(
    "Native artifact-panel URL webviews require Tauri's unstable Cargo feature",
  );
}

if (!tauriFeatures.includes('"tray-icon"')) {
  throw new Error(
    "Desktop background tray support requires Tauri's tray-icon Cargo feature",
  );
}

if (config.app?.macOSPrivateApi !== true) {
  throw new Error(
    "macOS transparent preview overlays require app.macOSPrivateApi",
  );
}

if (!tauriFeatures.includes('"macos-private-api"')) {
  throw new Error(
    "macOS transparent preview overlays require Tauri's macos-private-api Cargo feature",
  );
}

if (repoVersion !== packageJson.version) {
  throw new Error(
    `Root VERSION ${repoVersion} must match package.json ${packageJson.version}`,
  );
}

const csp = config.app?.security?.csp ?? "";
if (!csp.includes("frame-src blob: data:")) {
  throw new Error(
    "Tauri CSP must allow sandboxed blob/data artifact preview frames",
  );
}

if (
  !csp.includes(
    "frame-src blob: data: https: http://127.0.0.1:* http://localhost:* http://[::1]:*",
  )
) {
  throw new Error(
    "Tauri CSP must allow HTTPS and localhost artifact-panel URL previews",
  );
}

if (csp.includes("frame-src blob: data: http:")) {
  throw new Error("Tauri CSP must not allow arbitrary public HTTP frames");
}

if (!previewPanel.includes("srcDoc={previewDocument.source}")) {
  throw new Error("Artifact previews must render HTML through iframe srcDoc");
}

if (!previewPanel.includes('setPreviewDocument({ key: "", source: "" })')) {
  throw new Error("Artifact previews must clear stale HTML before rebuilding");
}

if (!previewPanel.includes('src="about:blank"')) {
  throw new Error("Artifact preview iframes must keep an inert src fallback");
}

if (previewPanel.includes("URL.createObjectURL(new Blob([previewDocument.source]")) {
  throw new Error(
    "Artifact previews must not use blob object URLs for iframe HTML",
  );
}

for (const [file, expectedCount] of nativePreviewBlockerFiles) {
  const source = readFileSync(join(root, "src", file), "utf8");
  const count = source.match(/data-native-preview-blocker="true"/g)?.length ?? 0;
  if (count < expectedCount) {
    throw new Error(`${file} must explicitly mark ${expectedCount} native preview blocker(s), found ${count}`);
  }
}

if (!previewPanel.includes('data-native-preview-blocker="open"')) {
  throw new Error("Inspector overflow must block native preview composition only while open");
}

for (const needle of [
  ".chat-body.inspector-stacked",
  "min-width: 360px",
  "@container preview-panel (max-width: 439px)",
  ".preview-file-select",
  ".preview-overflow",
]) {
  if (!styles.includes(needle)) {
    throw new Error(`Inspector responsive layout is missing: ${needle}`);
  }
}

for (const needle of [
  'data-testid="preview-browser-bar"',
  'data-testid="preview-browser-url"',
  'data-testid="preview-browser-empty"',
  'data-testid="preview-native-browser"',
  "new Webview",
  "new WebviewWindow",
  "@tauri-apps/api/webview",
  "@tauri-apps/api/webviewWindow",
  "incognito: true",
  "setIgnoreCursorEvents(true)",
  "normalizeArtifactBrowserUrl",
]) {
  if (!previewPanel.includes(needle)) {
    throw new Error(`Artifact URL previews must include ${needle}`);
  }
}

for (const needle of [
  ".preview-native-browser",
  ".preview-native-browser-error",
]) {
  if (!styles.includes(needle)) {
    throw new Error(
      `styles.css must include ${needle} for native URL preview webviews`,
    );
  }
}

for (const needle of [
  "blankBrowserArtifact",
  'data-testid="open-artifact-browser"',
]) {
  if (!chatView.includes(needle)) {
    throw new Error(
      `ChatView must include manual artifact browser opener ${needle}`,
    );
  }
}

if (
  chatView.includes(
    "setPreviewSelection(latestPreviewSelection);\n    if (!artifactPanelOpen) setArtifactPanelOpen(activeId, true);",
  )
) {
  throw new Error(
    "Preview auto-open must not override collapsed artifact-panel state",
  );
}

if (
  !chatView.includes(
    "setDismissedPreviewKey(latestPreviewSelection.autoOpenKey ?? null)",
  )
) {
  throw new Error(
    "Collapsed artifact panel should dismiss new auto-preview selections instead of reopening",
  );
}

for (const needle of [".preview-open-btn", ".preview-open-btn svg"]) {
  if (!styles.includes(needle)) {
    throw new Error(
      `styles.css must include ${needle} for the manual artifact browser button`,
    );
  }
}

for (const needle of [
  "SIDEBAR_SECTION_PREVIEW_LIMIT",
  "session-more-btn",
  "MoreHorizontal",
]) {
  if (!sidebar.includes(needle)) {
    throw new Error(
      `Sidebar must include ${needle} for per-section ellipsis reveal`,
    );
  }
}

for (const needle of [
  "aria-expanded={sectionManuallyExpanded}",
  "next.delete(group.id)",
  "focusComposerSoon",
  'aria-haspopup="menu"',
  "Current checkout",
  "Isolated worktree",
]) {
  if (!sidebar.includes(needle)) {
    throw new Error(
      `Sidebar must include ${needle} for section overflow and new-chat behavior`,
    );
  }
}

if (sidebar.includes("new-chat-isolated") || sidebar.includes("<span>Isolated</span>")) {
  throw new Error("Isolated worktrees should be a New chat menu option, not a peer sidebar button");
}

if (
  sidebar.includes("section-count") ||
  styles.includes(".section-count") ||
  sidebar.includes("Show more")
) {
  throw new Error(
    "Sidebar sections should use a centered ellipsis reveal instead of count badges or Show more text",
  );
}

if (
  styles.includes(".session-section-title:focus-within .section-actions-inline") ||
  !styles.includes(".section-actions-inline:focus-within")
) {
  throw new Error("Project actions should stay visible for their own keyboard focus, not a clicked section toggle");
}

if (
  !chatView.includes("messages.length === 0) focusComposer();") ||
  !chatView.includes('case "clear":')
) {
  throw new Error("New empty chats should focus the composer after creation");
}

for (const needle of [
  "struct DesktopProviders",
  "async fn refresh_provider_models",
  ".manage(DesktopProviders(providers))",
  "providers.refresh_all().await",
  "TrayIconBuilder::with_id",
  ".show_menu_on_left_click(false)",
  "TRAY_OPEN_ID => show_main_window(app)",
  "TRAY_QUIT_ID => request_user_state_flush_then_exit(app)",
  "TrayIconEvent::Click",
  "WindowEvent::CloseRequested",
  "api.prevent_close()",
  "window.hide()",
]) {
  if (!tauriLib.includes(needle)) {
    throw new Error(`Desktop background tray support must include ${needle}`);
  }
}

for (const needle of [
  "setup_native_menu",
  'SubmenuBuilder::new(app, "Milim")',
  'SubmenuBuilder::new(app, "File")',
  'SubmenuBuilder::new(app, "Edit")',
  'SubmenuBuilder::new(app, "View")',
  'SubmenuBuilder::new(app, "Window")',
  'SubmenuBuilder::new(app, "Help")',
  ".on_menu_event(handle_app_menu_event)",
  'const APP_MENU_EVENT: &str = "milim://menu-action"',
  "APP_MENU_QUIT_ID =>",
  "request_user_state_flush_then_exit(app)",
  "request_desktop_quit",
]) {
  if (!tauriLib.includes(needle)) {
    throw new Error(`Native app menu support must include ${needle}`);
  }
}

for (const permission of [
  "core:window:allow-close",
  "core:window:allow-create",
  "core:window:allow-hide",
  "core:window:allow-is-always-on-top",
  "core:window:allow-is-maximized",
  "core:window:allow-minimize",
  "core:window:allow-outer-position",
  "core:window:allow-scale-factor",
  "core:window:allow-set-always-on-top",
  "core:window:allow-set-ignore-cursor-events",
  "core:window:allow-set-position",
  "core:window:allow-set-size",
  "core:window:allow-show",
  "core:window:allow-start-dragging",
  "core:window:allow-start-resize-dragging",
  "core:window:allow-toggle-maximize",
  "core:webview:allow-create-webview",
  "core:webview:allow-create-webview-window",
  "core:webview:allow-set-webview-position",
  "core:webview:allow-set-webview-size",
  "core:webview:allow-set-webview-zoom",
  "core:webview:allow-webview-close",
]) {
  if (!capabilities.permissions.includes(permission)) {
    throw new Error(`Default Tauri capabilities must include ${permission}`);
  }
}

for (const needle of [
  "getCurrentWindow",
  "isAlwaysOnTop",
  "setAlwaysOnTop",
  "milim.window.alwaysOnTop",
  'data-testid="pin-window"',
  "startWindowDrag",
  "startDragging",
  "INTERACTIVE_TITLEBAR_SELECTOR",
  "data-tauri-drag-region",
  "onMouseDown={startWindowDrag}",
  'data-testid="app-menu-trigger"',
  'aria-haspopup="menu"',
  "aria-pressed={pinned}",
  "toggleAlwaysOnTop",
]) {
  if (!topBar.includes(needle)) {
    throw new Error(`TopBar must include ${needle}`);
  }
}

for (const needle of [
  "minimize",
  "toggleMaximize",
  "close",
  "isMaximized",
  "onResized",
]) {
  if (!windowControls.includes(needle)) {
    throw new Error(`WindowControls must include ${needle}`);
  }
}

for (const needle of [
  "startResizeDragging",
  '"North"',
  '"South"',
  '"East"',
  '"West"',
  '"NorthEast"',
  '"NorthWest"',
  '"SouthEast"',
  '"SouthWest"',
]) {
  if (!resizeHandles.includes(needle)) {
    throw new Error(`ResizeHandles must include ${needle}`);
  }
}

if (!styles.includes(".icon-btn.active")) {
  throw new Error(
    "styles.css must include active icon button styling for the pinned title-bar control",
  );
}
