import { createElement, type ComponentType, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { ChatArtifact, GoogleFilePreview, GoogleFileSummary, PreviewAppPreflight, PreviewAppStatus, PreviewSurfaceTarget } from "../src/api.js";
import type { ArtifactRevision } from "../src/lib/artifactRevisions.js";
import type { PreviewControlActivity } from "../src/lib/previewActivity.js";
import type { PreviewBrowserSession, PreviewSource, PreviewTab } from "../src/components/PreviewPanel.js";

type PreviewPanelProps = {
  artifact: ChatArtifact;
  artifacts?: readonly ChatArtifact[];
  fixArtifact?: ChatArtifact;
  fixArtifacts?: readonly ChatArtifact[];
  fixRevision?: ArtifactRevision;
  onClose: () => void;
  onOpenBrowser?: () => void;
  onPrepareArtifactFix?: (prompt: string) => void;
  activeTab?: PreviewTab;
  onActiveTabChange?: (tab: PreviewTab) => void;
  previewSource?: PreviewSource;
  availablePreviewSources?: readonly PreviewSource[];
  onPreviewSourceChange?: (source: PreviewSource) => void;
  browserSession?: PreviewBrowserSession;
  onBrowserSessionChange?: (session: PreviewBrowserSession) => void;
  runtimeStatus?: PreviewAppStatus | null;
  runtimePreflight?: PreviewAppPreflight | null;
  runtimeStale?: boolean;
  onRuntimePreflight?: () => void;
  onRuntimeStart?: () => void;
  onRuntimeStop?: () => void;
  onRuntimeRestart?: () => void;
  modeSwitcher?: ReactNode;
  controlActivity?: PreviewControlActivity | null;
  onSurfaceChange?: (surface: PreviewSurfaceTarget | null) => void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const urlArtifact: ChatArtifact = {
  id: "url-preview",
  kind: "text",
  title: "http://localhost:5173/",
  mime: "text/uri-list",
  content: "http://localhost:5173/",
  size: 22,
  language: "url",
};

const blankUrlArtifact: ChatArtifact = {
  id: "blank-url-preview",
  kind: "text",
  title: "Browser",
  mime: "text/uri-list",
  content: "",
  size: 0,
  language: "url",
};

const googleSheetArtifact: ChatArtifact = {
  ...urlArtifact,
  id: "google-sheet-preview",
  title: "Google Sheet",
  content: "https://docs.google.com/spreadsheets/d/sheet_123/edit",
};

const googleFolderArtifact: ChatArtifact = {
  ...urlArtifact,
  id: "google-folder-preview",
  title: "Google Drive folder",
  content: "https://drive.google.com/drive/folders/folder_123",
};

const htmlArtifact: ChatArtifact = {
  id: "html-preview",
  kind: "code",
  title: "index.html",
  filename: "index.html",
  language: "html",
  mime: "text/html",
  content: "<!doctype html><html><body>Preview</body></html>",
  size: 48,
};

const textArtifact: ChatArtifact = {
  id: "notes-text",
  kind: "code",
  title: "notes.txt",
  filename: "notes.txt",
  language: "text",
  mime: "text/plain",
  content: "first line\nsecond line",
  size: 22,
};

const runtimePreflight: PreviewAppPreflight = {
  thread_id: "thread-1",
  cwd: "C:\\workspace\\generated-app",
  managed: false,
  scope: "selected_folder",
  package_manager: "pnpm",
  install_required: true,
  install_command: "pnpm install --ignore-scripts",
  dev_command: "pnpm dev -- --host 127.0.0.1 --port 4173",
  source_fingerprint: "0123456789abcdef0123456789abcdef",
  port: 4173,
  url: "http://127.0.0.1:4173/",
};

const runtimeStatus: PreviewAppStatus = {
  thread_id: "thread-1",
  status: "error",
  cwd: runtimePreflight.cwd,
  active: false,
  ready: false,
  managed: false,
  error: { code: "runtime_failed", message: "Compilation failed" },
  message: "The app could not compile.",
  preflight: runtimePreflight,
  logs: [{ seq: 1, ts: 1, stream: "stderr", line: "Unexpected token" }],
};

const readyRuntimeStatus: PreviewAppStatus = {
  ...runtimeStatus,
  status: "running",
  active: true,
  ready: true,
  error: null,
  message: "Running.",
  url: runtimePreflight.url,
  logs: [],
};

const startingRuntimeStatus: PreviewAppStatus = {
  ...readyRuntimeStatus,
  status: "starting",
  ready: false,
  message: "Starting preview.",
};

const controlActivity: PreviewControlActivity = {
  id: "activity-1",
  gesture: "click",
  label: "Used computer",
  status: "done",
};

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { PreviewPanel, buildFixPrompt, nativePreviewBlockedByAppUi, previewDocumentReadyForSurface, previewSurfaceIsInspectable, nextPreviewTab } = await server.ssrLoadModule("/src/components/PreviewPanel.tsx") as {
    PreviewPanel: ComponentType<PreviewPanelProps>;
    buildFixPrompt: (
      artifact: ChatArtifact,
      files: string[],
      revisionNumber: number | undefined,
      previewError: string | null,
      runtimeError: string | null,
      errors: Array<{ id: number; level: "error"; message: string; timestamp: number }>,
    ) => string;
    nativePreviewBlockedByAppUi: (root: Pick<ParentNode, "querySelector">) => boolean;
    previewDocumentReadyForSurface: (isUrlPreview: boolean, builtKey: string, requestedKey: string) => boolean;
    previewSurfaceIsInspectable: (surface: PreviewSurfaceTarget | null) => boolean;
    nextPreviewTab: (current: PreviewTab, key: string, tabs: readonly PreviewTab[]) => PreviewTab | null;
  };
  const { ContextMenuProvider } = await server.ssrLoadModule("/src/components/ContextMenu.tsx") as {
    ContextMenuProvider: ComponentType<{ children: ReactNode }>;
  };
  const { DocumentPreview, FolderPreview, SheetPreview, SlidesPreview, browserLinkOpensNewTab, googleWorkspacePreviewNeedsLoad } = await server.ssrLoadModule("/src/components/GoogleWorkspacePreview.tsx") as {
    DocumentPreview: ComponentType<{
      document: Record<string, unknown>;
      fallbackText: string;
    }>;
    FolderPreview: ComponentType<{
      children: GoogleFileSummary[];
      choosing: boolean;
      onChoose: () => void;
      onOpenFile: (url: string) => void;
      onOpenFileInNewTab: (url: string) => void;
    }>;
    SheetPreview: ComponentType<{
      preview: Extract<GoogleFilePreview, { kind: "sheet" }>;
      range: string;
      setRange: (value: string) => void;
      loadRange: (value: string) => void;
      submitRange: () => void;
    }>;
    SlidesPreview: ComponentType<{
      fileId: string;
      slides: Array<{ objectId?: string | null; text: string; notes?: string | null }>;
      active: boolean;
    }>;
    browserLinkOpensNewTab: (event: { button: number; ctrlKey: boolean; metaKey: boolean }) => boolean;
    googleWorkspacePreviewNeedsLoad: (active: boolean, loadedRequest: string | null, request: string) => boolean;
  };
  const renderPreviewPanel = (props: PreviewPanelProps) => renderToStaticMarkup(
    createElement(ContextMenuProvider, null, createElement(PreviewPanel, props)),
  );
  const sheetPreview: Extract<GoogleFilePreview, { kind: "sheet" }> = {
    kind: "sheet",
    file: {
      id: "sheet_123",
      name: "Pipeline",
      mime_type: "application/vnd.google-apps.spreadsheet",
      trashed: false,
      parents: [],
      capabilities: {
        can_edit: true,
        can_download: true,
        can_move: true,
        can_rename: true,
        can_share: true,
        can_trash: true,
      },
      created_by_milim: false,
    },
    range: "'Venues'!B2:C3",
    sheets: [{ properties: { title: "Venues" } }, { properties: { title: "Archive" } }],
    values: [["Venue", "Status"], ["Noor", "Completed"]],
    formulas: [["Venue", "Status"], ["Noor", "=UPPER(\"completed\")"]],
  };
  const sheetMarkup = renderToStaticMarkup(createElement(SheetPreview, {
    preview: sheetPreview,
    range: sheetPreview.range,
    setRange: () => {},
    loadRange: () => {},
    submitRange: () => {},
  }));
  assert(sheetMarkup.includes('aria-current="page"'), "Sheets preview should mark the active worksheet");
  assert(sheetMarkup.includes('aria-label="Active cell">B2'), "Sheets preview should expose the active cell address");
  assert(sheetMarkup.includes('aria-label="Search this sheet range"'), "Sheets preview should expose range search");
  assert(sheetMarkup.includes('aria-label="Sheet zoom"'), "Sheets preview should expose zoom");
  assert(sheetMarkup.includes('aria-label="Resize column B"'), "Sheets preview should expose keyboard-accessible column resizing");
  const documentMarkup = renderToStaticMarkup(createElement(DocumentPreview, {
    document: {
      body: { content: [
        { paragraph: {
          paragraphStyle: { namedStyleType: "TITLE", alignment: "CENTER" },
          elements: [{ textRun: { content: "Project brief", textStyle: { bold: true } } }],
        } },
        { paragraph: {
          paragraphStyle: { namedStyleType: "HEADING_1" },
          elements: [{ textRun: { content: "Overview" } }],
        } },
      ] },
    },
    fallbackText: "Project brief Overview",
  }));
  assert(documentMarkup.includes('aria-label="Search document"'), "Docs preview should expose document search");
  assert(documentMarkup.includes('aria-label="Document zoom"'), "Docs preview should expose zoom");
  assert(documentMarkup.includes('aria-label="Show document outline"'), "Docs preview should expose a heading outline control");
  assert(documentMarkup.includes("text-align:center"), "Docs preview should preserve paragraph alignment");
  const slidesMarkup = renderToStaticMarkup(createElement(SlidesPreview, {
    fileId: "slides_123",
    active: true,
    slides: [
      { objectId: "slide_1", text: "Opening", notes: "Welcome everyone" },
      { objectId: "slide_2", text: "Details", notes: "" },
    ],
  }));
  assert(slidesMarkup.includes('aria-label="Previous slide"'), "Slides preview should expose previous-slide navigation");
  assert(slidesMarkup.includes('aria-label="Next slide"'), "Slides preview should expose next-slide navigation");
  assert(slidesMarkup.includes('aria-label="Search slides"'), "Slides preview should expose presentation search");
  assert(slidesMarkup.includes('aria-label="Slide zoom"'), "Slides preview should expose zoom");
  assert(slidesMarkup.includes(">Notes<"), "Slides preview should expose available speaker notes");
  assert(slidesMarkup.includes('aria-current="page"'), "Slides preview should mark the active slide");
  const folderMarkup = renderToStaticMarkup(createElement(
    ContextMenuProvider,
    null,
    createElement(FolderPreview, {
      children: [sheetPreview.file],
      choosing: false,
      onChoose: () => {},
      onOpenFile: () => {},
      onOpenFileInNewTab: () => {},
    }),
  ));
  assert(folderMarkup.includes('class="google-folder-entry-main"'), "Folder entry bodies should open files in Milim");
  assert(folderMarkup.includes('aria-label="Open Pipeline in Google"'), "Folder entries should keep a separate external-browser action");
  assert(browserLinkOpensNewTab({ button: 1, ctrlKey: false, metaKey: false }), "Middle-click should open browser links in a new tab");
  assert(browserLinkOpensNewTab({ button: 0, ctrlKey: true, metaKey: false }), "Ctrl-click should open browser links in a new tab");
  assert(!browserLinkOpensNewTab({ button: 0, ctrlKey: false, metaKey: false }), "Plain click should reuse the active browser tab");
  assert(!googleWorkspacePreviewNeedsLoad(true, "same", "same"), "Reactivating a loaded Google tab should not refetch it");
  assert(googleWorkspacePreviewNeedsLoad(true, "same", "refreshed"), "Explicit Google refreshes should still fetch");
  let blockerSelector = "";
  nativePreviewBlockedByAppUi({
    querySelector: ((selector: string) => {
      blockerSelector = selector;
      return null;
    }) as ParentNode["querySelector"],
  });
  assert(blockerSelector.includes('[data-native-preview-blocker="true"]'), "Native preview blockers should use an explicit mounted-overlay marker");
  assert(blockerSelector.includes('[data-native-preview-blocker="open"][open]'), "Native preview blockers should support always-mounted disclosure controls only while open");
  assert(!blockerSelector.includes('[role="menu"]') && !blockerSelector.includes("aria-modal"), "Native preview blocking should not infer visibility from semantic roles");
  assert(nativePreviewBlockedByAppUi({ querySelector: () => ({}) as Element }), "Native preview should hide behind app modal/menu UI");
  assert(!nativePreviewBlockedByAppUi({ querySelector: () => null }), "Native preview should stay visible without blocking app UI");
  assert(previewSurfaceIsInspectable({
    label: "main",
    kind: "artifact_iframe",
    title: "index.html",
    native: false,
    status: "ready",
    capabilities: ["dom_snapshot", "click"],
  }), "Ready DOM-capable artifact surfaces should expose preview tools");
  assert(!previewSurfaceIsInspectable({
    kind: "blank",
    title: "Browser",
    native: false,
    status: "not_inspectable",
    capabilities: [],
  }), "Blank browser surfaces should not expose preview tools");
  assert(!previewSurfaceIsInspectable({
    kind: "markdown",
    title: "notes.md",
    native: false,
    status: "not_inspectable",
    capabilities: ["source"],
  }), "Markdown/code-only surfaces should not expose preview tools");
  assert(nextPreviewTab("preview", "ArrowRight", ["preview", "code"]) === "code", "Right Arrow should advance inspector tabs");
  assert(nextPreviewTab("preview", "ArrowLeft", ["preview", "code"]) === "code", "Left Arrow should wrap inspector tabs");
  assert(nextPreviewTab("code", "Home", ["preview", "code"]) === "preview", "Home should focus the first inspector tab");
  assert(nextPreviewTab("preview", "End", ["preview", "code"]) === "code", "End should focus the last inspector tab");
  assert(previewDocumentReadyForSurface(true, "previous-url", "active-url"), "URL tabs should bypass artifact document rebuilding");
  assert(!previewDocumentReadyForSurface(false, "old-artifact", "new-artifact"), "Artifact previews should still wait for their built document");
  const fixPrompt = buildFixPrompt(
    htmlArtifact,
    ["index.html"],
    3,
    null,
    "Compilation failed\nThe app could not compile.",
    [{ id: 1, level: "error", message: "Unexpected token", timestamp: 1 }],
  );
  assert(fixPrompt.includes("Revision: v3"), "Prepare fix should identify the selected revision");
  assert(fixPrompt.includes("Compilation failed"), "Prepare fix should include structured runtime failure details");
  assert(fixPrompt.includes("The app could not compile."), "Prepare fix should include the runtime message");
  assert(fixPrompt.includes("Unexpected token"), "Prepare fix should include recent error logs");

  const urlMarkup = renderPreviewPanel({ artifact: urlArtifact, onClose: () => {} });
  assert(urlMarkup.includes('data-testid="preview-browser-bar"'), "URL artifacts should render browser chrome");
  assert(urlMarkup.includes('data-testid="preview-browser-tabs"'), "URL artifacts should render browser tabs");
  assert(urlMarkup.includes('data-testid="preview-browser-url"'), "URL artifacts should render an address input");
  assert(!urlMarkup.includes('data-testid="preview-context-title"'), "URL artifacts should not duplicate the browser address in the inspector header");
  assert(urlMarkup.includes('data-testid="preview-browser-setup"'), "First URL-browser use should disclose persistent sign-ins");
  assert(urlMarkup.includes('data-testid="preview-native-browser"'), "URL artifacts should render the native browser host");
  assert(urlMarkup.includes('src="http://localhost:5173/"'), "URL artifacts should render a non-Tauri iframe fallback");
  const tabbedUrlMarkup = renderPreviewPanel({
    artifact: urlArtifact,
    onClose: () => {},
    browserSession: {
      profileId: "test-profile",
      activeTabId: "second",
      tabs: [
        { id: "first", url: "https://example.com/", input: "https://example.com/", history: ["https://example.com/"], historyIndex: 0 },
        { id: "second", url: "https://sheets.google.com/", input: "https://sheets.google.com/", history: ["https://sheets.google.com/"], historyIndex: 0 },
      ],
    },
  });
  assert(tabbedUrlMarkup.includes("example.com") && tabbedUrlMarkup.includes("sheets.google.com"), "Browser sessions should render every tab");
  assert(tabbedUrlMarkup.includes('aria-selected="true"'), "Browser sessions should identify the active tab");

  const googleMarkup = renderPreviewPanel({ artifact: googleSheetArtifact, onClose: () => {} });
  assert(googleMarkup.includes("Choose this file with Google"), "Unauthorized Google files should offer the Picker flow");
  assert(!googleMarkup.includes('data-testid="preview-native-browser"'), "Recognized Google files should use a native Milim viewer instead of embedding Google");
  const googleFolderMarkup = renderPreviewPanel({ artifact: googleFolderArtifact, onClose: () => {} });
  assert(googleFolderMarkup.includes("Choose files from this folder"), "Google Drive folders should offer one multi-file Picker flow");
  assert(googleFolderMarkup.includes("select its files"), "Folder selection should explain the explicit per-file authorization boundary");

  const blankUrlMarkup = renderPreviewPanel({ artifact: blankUrlArtifact, onClose: () => {} });
  assert(blankUrlMarkup.includes('data-testid="preview-browser-bar"'), "Blank URL artifacts should still render browser chrome");
  assert(blankUrlMarkup.includes('data-testid="preview-browser-empty"'), "Blank URL artifacts should render the empty browser state");
  assert(!blankUrlMarkup.includes('data-testid="preview-native-browser"'), "Blank URL artifacts should not render a native browser host");

  const htmlMarkup = renderPreviewPanel({ artifact: htmlArtifact, onClose: () => {}, onOpenBrowser: () => {} });
  assert(!htmlMarkup.includes('data-testid="preview-browser-bar"'), "HTML artifacts should not render browser chrome");
  assert(htmlMarkup.includes('data-testid="preview-open-browser"'), "HTML artifacts should let users switch to the browser");
  assert(htmlMarkup.includes("srcDoc="), "HTML artifacts should keep srcDoc preview rendering");
  assert(!htmlMarkup.includes('data-testid="preview-control-overlay"'), "Preview overlay should not render without activity");
  assert(htmlMarkup.includes('aria-label="Inspector"'), "The side inspector should have an accessible name");
  assert(htmlMarkup.includes('id="inspector-tab-preview"'), "Fallback Preview tab should expose a stable id");
  assert(htmlMarkup.includes('aria-controls="inspector-panel-preview"'), "Preview tab should control its linked panel");
  assert(htmlMarkup.includes('id="inspector-panel-preview"'), "Preview panel should expose the linked id");
  assert(htmlMarkup.includes('aria-labelledby="inspector-tab-preview"'), "Preview panel should be labelled by its tab");
  assert(htmlMarkup.includes('data-testid="preview-context-title"'), "Inspector should render a contextual title row");
  assert(htmlMarkup.includes('data-testid="preview-header"'), "Inspector navigation and context should share one responsive header");
  assert(htmlMarkup.includes('data-native-preview-blocker="open"'), "Inspector overflow should block its native preview only while expanded");
  assert(htmlMarkup.includes("index.html"), "Contextual title should include the artifact name");

  const codeMarkup = renderPreviewPanel({ artifact: textArtifact, artifacts: [textArtifact, htmlArtifact], activeTab: "code", onClose: () => {} });
  assert(!codeMarkup.includes('data-testid="preview-tab-preview"'), "Non-renderable artifacts should be code-only");
  assert(codeMarkup.includes('data-testid="preview-tab-code"'), "Code-only artifacts should keep the Code tab");
  assert(codeMarkup.includes('id="inspector-panel-code"'), "Code panel should expose the linked id");
  assert(codeMarkup.includes('aria-labelledby="inspector-tab-code"'), "Code panel should be labelled by its tab");
  assert(codeMarkup.includes('aria-label="Artifact file"'), "Multi-file code should provide a narrow-layout file selector");
  assert(codeMarkup.includes('data-testid="preview-code-line-number" aria-hidden="true"'), "Visual line numbers should be hidden from assistive technology");

  const unifiedTabs = createElement("div", { className: "side-panel-switcher", role: "tablist", "aria-label": "Inspector views" },
    createElement("button", { id: "inspector-tab-preview", role: "tab", "aria-selected": true, "aria-controls": "inspector-panel-preview" }, "Preview"),
    createElement("button", { id: "inspector-tab-code", role: "tab", "aria-selected": false, "aria-controls": "inspector-panel-code" }, "Code"),
  );
  const unifiedMarkup = renderPreviewPanel({ artifact: htmlArtifact, onClose: () => {}, modeSwitcher: unifiedTabs });
  assert((unifiedMarkup.match(/id="inspector-tab-preview"/g) ?? []).length === 1, "Unified mode switcher should replace nested Preview/Code tabs");
  const appCodeMarkup = renderPreviewPanel({
    artifact: htmlArtifact,
    activeTab: "code",
    previewSource: "app",
    availablePreviewSources: ["artifact", "app", "url"],
    runtimeStatus,
    onClose: () => {},
    modeSwitcher: unifiedTabs,
  });
  assert(appCodeMarkup.includes("index.html"), "Code context should keep the selected artifact title");
  assert(!appCodeMarkup.includes('data-testid="preview-source-selector"'), "Preview source controls should not appear in Code");

  const runtimeMarkup = renderPreviewPanel({
    artifact: urlArtifact,
    fixArtifact: htmlArtifact,
    fixArtifacts: [htmlArtifact],
    previewSource: "app",
    availablePreviewSources: ["artifact", "app", "url"],
    runtimeStatus,
    runtimePreflight,
    onRuntimePreflight: () => {},
    onRuntimeStart: () => {},
    onPrepareArtifactFix: () => {},
    onClose: () => {},
  });
  assert(runtimeMarkup.includes('data-testid="preview-source-selector"'), "Multiple preview sources should render a compact selector");
  assert(runtimeMarkup.includes("generated-app"), "App context should use the project folder title");
  assert(runtimeMarkup.includes('data-testid="preview-runtime-preflight-details"'), "Runtime review should show preflight details");
  assert(runtimeMarkup.includes("pnpm install --ignore-scripts"), "Runtime review should show the exact install command");
  assert(runtimeMarkup.includes("may modify the selected folder"), "Selected-folder installs should show a mutation warning");
  assert(runtimeMarkup.includes('aria-label="Run app preview"'), "Runtime should expose an explicit accessible Run action");
  assert(runtimeMarkup.includes('data-testid="preview-prepare-fix"'), "App runtime errors should offer Prepare fix");
  assert(!runtimeMarkup.includes("Quick Fix"), "Legacy Quick Fix copy should be removed");

  const readyRuntimeMarkup = renderPreviewPanel({
    artifact: urlArtifact,
    previewSource: "app",
    runtimeStatus: readyRuntimeStatus,
    runtimePreflight,
    onRuntimePreflight: () => {},
    onRuntimeStart: () => {},
    onRuntimeStop: () => {},
    onRuntimeRestart: () => {},
    onClose: () => {},
  });
  assert(!readyRuntimeMarkup.includes('data-testid="preview-managed-runtime"'), "Healthy runtime should give the preview the full content area");
  assert(!readyRuntimeMarkup.includes('data-testid="preview-browser-setup"'), "Generated App previews should not show persistent browser setup");
  assert(readyRuntimeMarkup.includes('data-testid="preview-runtime-status"'), "Healthy runtime should keep a status disclosure");
  assert(readyRuntimeMarkup.includes('aria-expanded="false"'), "Healthy runtime details should start collapsed");
  assert(readyRuntimeMarkup.includes('data-testid="preview-runtime-quick-stop"'), "Healthy runtime should keep a one-click Stop action");
  assert(!readyRuntimeMarkup.includes("Running."), "Healthy runtime should suppress redundant running copy");

  const startingRuntimeMarkup = renderPreviewPanel({
    artifact: urlArtifact,
    previewSource: "app",
    runtimeStatus: startingRuntimeStatus,
    runtimePreflight,
    onRuntimeStop: () => {},
    onRuntimeRestart: () => {},
    onClose: () => {},
  });
  assert(startingRuntimeMarkup.includes('data-testid="preview-managed-runtime"'), "Starting runtime should keep a compact progress surface");
  assert(!startingRuntimeMarkup.includes('data-testid="preview-runtime-preflight-details"'), "Starting runtime should hide already-reviewed commands");
  assert(startingRuntimeMarkup.includes("Starting preview."), "Starting runtime should preserve actionable progress copy");
  assert(startingRuntimeMarkup.includes('data-testid="preview-runtime-stop"'), "Starting runtime should remain stoppable");
  assert(!startingRuntimeMarkup.includes('data-testid="preview-runtime-restart"'), "Starting runtime should not expose a redundant restart action");

  const staleRuntimeMarkup = renderPreviewPanel({
    artifact: urlArtifact,
    previewSource: "app",
    runtimeStatus: readyRuntimeStatus,
    runtimePreflight,
    runtimeStale: true,
    onRuntimeStop: () => {},
    onRuntimeRestart: () => {},
    onClose: () => {},
  });
  assert(staleRuntimeMarkup.includes('data-testid="preview-managed-runtime"'), "Disconnected runtime should restore the full status panel");
  assert(staleRuntimeMarkup.includes('data-testid="preview-runtime-preflight-details"'), "Disconnected runtime should expose its reviewed commands");

  const activeMarkup = renderPreviewPanel({ artifact: htmlArtifact, onClose: () => {}, controlActivity });
  assert(activeMarkup.includes('data-testid="preview-control-overlay"'), "Preview overlay should render when activity is supplied");
  assert(activeMarkup.includes('aria-hidden="true"'), "Preview overlay should be hidden from assistive tech");

  const activeUrlMarkup = renderPreviewPanel({ artifact: urlArtifact, onClose: () => {}, controlActivity });
  assert(!activeUrlMarkup.includes('data-testid="preview-control-overlay"'), "URL previews should leave preview control cues to the native overlay window");
} finally {
  await server.close();
}

export {};
