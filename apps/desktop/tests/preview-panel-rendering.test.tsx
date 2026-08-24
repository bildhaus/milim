import { createElement, type ComponentType, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
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
  workspaceFolder?: string;
  onPreviewWorkspaceFile?: (path: string) => void;
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

const workspaceArtifact: ChatArtifact = {
  ...textArtifact,
  id: "workspace-review",
  title: "Workspace",
  content: "Select a workspace file above to review it.",
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
  kind: "app",
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
  const { PreviewPanel, buildFixPrompt, nativePreviewBlockedByAppUi, previewDocumentReadyForSurface, previewNativeWebviewLabel, previewSurfaceIsInspectable, nextPreviewTab } = await server.ssrLoadModule("/src/components/PreviewPanel.tsx") as {
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
    previewNativeWebviewLabel: (surfaceKind: "runtime_browser" | "native_browser", storageMode: "persistent" | "private") => string;
    previewDocumentReadyForSurface: (isUrlPreview: boolean, builtKey: string, requestedKey: string) => boolean;
    previewSurfaceIsInspectable: (surface: PreviewSurfaceTarget | null) => boolean;
    nextPreviewTab: (current: PreviewTab, key: string, tabs: readonly PreviewTab[]) => PreviewTab | null;
  };
  const { ContextMenuProvider } = await server.ssrLoadModule("/src/components/ContextMenu.tsx") as {
    ContextMenuProvider: ComponentType<{ children: ReactNode }>;
  };
  const { SourceCodeView } = await server.ssrLoadModule("/src/components/SourceCodeView.tsx") as {
    SourceCodeView: ComponentType<{
      source: string;
      language?: string;
      ariaLabel: string;
      selectedLine?: number;
      onLineClick?: () => void;
    }>;
  };
  const { registerWorkspaceEditorGuard, requestWorkspaceEditorLeave } = await server.ssrLoadModule("/src/lib/workspaceEditorGuard.ts") as {
    registerWorkspaceEditorGuard: (guard: (reason: "navigate" | "hide" | "quit") => Promise<boolean>) => () => void;
    requestWorkspaceEditorLeave: (reason?: "navigate" | "hide" | "quit") => Promise<boolean>;
  };
  const { DocumentPreview, FolderPreview, GoogleTextFormatToolbar, SheetPreview, SlidesPreview, browserLinkOpensNewTab, googleDocFitScale, googleFileKindDetail, googleSlideGestureRect, googleSlideGroupBounds, googleSlideMarqueeRect, googleSlideRectsIntersect, googleSlideSnapRect, googleSlideTextFormat, googleSlideTextSegments, googleSlideThumbnailRequestKey, googleSlideTransformGroup, googleSlidesNavigationAction, googleWorkspacePreviewNeedsLoad } = await server.ssrLoadModule("/src/components/GoogleWorkspacePreview.tsx") as {
    DocumentPreview: ComponentType<{
      fileId: string;
      canEdit: boolean;
      document: Record<string, unknown>;
      fallbackText: string;
      onSaved: () => void;
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
      onSaved: () => void;
    }>;
    SlidesPreview: ComponentType<{
      fileId: string;
      slides: Extract<GoogleFilePreview, { kind: "presentation" }>["slides"];
      pageAspectRatio: number;
      pageWidth: number;
      pageHeight: number;
      active: boolean;
      canEdit: boolean;
      onSaved: () => void;
    }>;
    GoogleTextFormatToolbar: ComponentType<{
      format: { bold: boolean; italic: boolean; underline: boolean; fontSize: number; color: string; alignment: "START" | "CENTER" | "END" | "JUSTIFIED" };
      onChange: () => void;
    }>;
    browserLinkOpensNewTab: (event: { button: number; ctrlKey: boolean; metaKey: boolean }) => boolean;
    googleDocFitScale: (width: number, paddingLeft: number, paddingRight: number) => number;
    googleFileKindDetail: (file: GoogleFileSummary) => string;
    googleSlideGestureRect: (rect: { x: number; y: number; width: number; height: number }, deltaX: number, deltaY: number, handle: "move" | "ne" | "se" | "sw" | "nw") => { x: number; y: number; width: number; height: number };
    googleSlideGroupBounds: (rects: Array<{ x: number; y: number; width: number; height: number }>) => { x: number; y: number; width: number; height: number } | null;
    googleSlideMarqueeRect: (startX: number, startY: number, endX: number, endY: number) => { x: number; y: number; width: number; height: number };
    googleSlideRectsIntersect: (a: { x: number; y: number; width: number; height: number }, b: { x: number; y: number; width: number; height: number }) => boolean;
    googleSlideSnapRect: (rect: { x: number; y: number; width: number; height: number }, others: Array<{ x: number; y: number; width: number; height: number }>, handle: "move" | "ne" | "se" | "sw" | "nw", thresholdX: number, thresholdY: number) => { rect: { x: number; y: number; width: number; height: number }; guideX?: number; guideY?: number };
    googleSlideThumbnailRequestKey: (fileId: string, slideId: string, generation: number) => string;
    googleSlideTransformGroup: (rect: { x: number; y: number; width: number; height: number }, from: { x: number; y: number; width: number; height: number }, to: { x: number; y: number; width: number; height: number }) => { x: number; y: number; width: number; height: number };
    googleSlideTextFormat: (field: {
      id: string;
      label: string;
      text: string;
      kind: "shape";
      styleRuns: Array<{ start: number; end: number; style: Record<string, unknown> }>;
      paragraphRuns: Array<{ start: number; end: number; style: Record<string, unknown> }>;
    }, index: number) => { bold: boolean; fontSize: number; color: string; alignment: string };
    googleSlideTextSegments: (field: {
      id: string;
      label: string;
      text: string;
      kind: "shape";
      styleRuns: Array<{ start: number; end: number; style: Record<string, unknown> }>;
      paragraphRuns: Array<{ start: number; end: number; style: Record<string, unknown> }>;
    }) => Array<{ start: number; end: number; text: string; format: { bold: boolean; italic: boolean; color: string } }>;
    googleSlidesNavigationAction: (key: string) => "previous" | "next" | "first" | "last" | "exit" | null;
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
    sheets: [{ properties: { title: "Venues", sheetId: 7 } }, { properties: { title: "Archive", sheetId: 8 } }],
    values: [["Venue", "Status"], ["Noor", "Completed"]],
    formulas: [["Venue", "Status"], ["Noor", "=UPPER(\"completed\")"]],
  };
  const renderSheet = (preview: typeof sheetPreview) => renderToStaticMarkup(
    createElement(ContextMenuProvider, null, createElement(SheetPreview, {
      preview,
      range: preview.range,
      setRange: () => {},
      loadRange: () => {},
      submitRange: () => {},
      onSaved: () => {},
    })),
  );
  const sheetMarkup = renderSheet(sheetPreview);
  assert(sheetMarkup.includes('aria-current="page"'), "Sheets preview should mark the active worksheet");
  assert(sheetMarkup.includes('aria-label="Active cell">B2'), "Sheets preview should expose the active cell address");
  assert(sheetMarkup.includes('aria-label="Edit active cell B2"'), "Sheets preview should expose formula-bar editing");
  assert(sheetMarkup.includes('aria-label="Search this sheet range"'), "Sheets preview should expose range search");
  assert(sheetMarkup.includes('aria-label="Sheet zoom"'), "Sheets preview should expose zoom");
  assert(sheetMarkup.includes('aria-label="Resize column B"'), "Sheets preview should expose keyboard-accessible column resizing");
  assert(sheetMarkup.includes('aria-haspopup="menu"'), "Editable Sheets previews should expose dimension actions in a menu");
  assert(
    sheetMarkup.indexOf('class="google-sheet-grid-wrap"') < sheetMarkup.indexOf('class="google-sheet-tabs"'),
    "Sheets preview should place worksheet tabs below the grid",
  );
  const viewOnlySheetMarkup = renderSheet({
    ...sheetPreview,
    file: {
      ...sheetPreview.file,
      capabilities: { ...sheetPreview.file.capabilities, can_edit: false },
    },
  });
  assert(viewOnlySheetMarkup.includes('readonly=""'), "View-only Sheets previews should keep the formula bar read-only");
  const documentMarkup = renderToStaticMarkup(createElement(DocumentPreview, {
    fileId: "doc_123",
    canEdit: true,
    document: {
      namedStyles: { styles: [{
        namedStyleType: "NORMAL_TEXT",
        textStyle: {
          weightedFontFamily: { fontFamily: "Roboto", weight: 500 },
          smallCaps: true,
        },
      }] },
      body: { content: [
        { startIndex: 1, endIndex: 15, paragraph: {
          paragraphStyle: { namedStyleType: "TITLE", alignment: "CENTER" },
          elements: [{ textRun: { content: "Project brief", textStyle: { bold: true } } }],
        } },
        { startIndex: 15, endIndex: 24, paragraph: {
          paragraphStyle: { namedStyleType: "HEADING_1" },
          elements: [{ textRun: { content: "Overview" } }],
        } },
      ] },
    },
    fallbackText: "Project brief Overview",
    onSaved: () => {},
  }));
  assert(documentMarkup.includes('aria-label="Search document"'), "Docs preview should expose document search");
  assert(documentMarkup.includes('aria-label="Document zoom"'), "Docs preview should expose zoom");
  assert(documentMarkup.includes('<option value="fit" selected="">Fit width</option>'), "Docs preview should default to one unambiguous fit-width zoom mode");
  assert(documentMarkup.includes('aria-label="Show document outline"'), "Docs preview should expose a heading outline control");
  assert(documentMarkup.includes("text-align:center"), "Docs preview should preserve paragraph alignment");
  assert(documentMarkup.includes("font-family:Roboto"), "Docs preview should inherit named-style fonts");
  assert(documentMarkup.includes("font-weight:500"), "Docs preview should preserve named-style font weights");
  assert(documentMarkup.includes("font-variant-caps:small-caps"), "Docs preview should preserve small caps");
  assert(documentMarkup.includes('contenteditable="true"'), "Editable Docs previews should use a native document editing surface");
  assert(documentMarkup.includes('aria-label="Edit document text"'), "Inline Docs editors should expose document textbox semantics");
  assert(
    documentMarkup.match(/contenteditable="true"/g)?.length === 1,
    "Adjacent Docs paragraphs should share one editing surface",
  );
  assert(documentMarkup.includes('aria-label="Paragraph formatting"'), "Docs should keep paragraph and list controls in a fixed toolbar");
  assert(documentMarkup.includes("google-doc-unified-toolbar"), "Docs should consolidate search and formatting into one command bar");
  assert(!documentMarkup.includes("google-doc-selection-toolbar"), "Docs should not render a separate floating selection toolbar");
  assert(documentMarkup.includes("Click text to edit"), "Editable Docs previews should expose paragraph editing");
  const formatToolbarMarkup = renderToStaticMarkup(createElement(GoogleTextFormatToolbar, {
    format: { bold: true, italic: false, underline: false, fontSize: 18, color: "#336699", alignment: "CENTER" },
    onChange: () => {},
  }));
  assert(formatToolbarMarkup.includes('role="toolbar"'), "Docs and Slides should share an accessible formatting toolbar");
  assert(formatToolbarMarkup.includes('aria-label="Font size"'), "Formatting toolbar should expose font size");
  assert(formatToolbarMarkup.includes('aria-label="Text alignment"'), "Formatting toolbar should expose alignment");
  assert(formatToolbarMarkup.includes('aria-label="Text color"'), "Formatting toolbar should expose text color");
  assert(googleDocFitScale(860, 22, 22) === 1, "Docs fit width should preserve a full-size page");
  assert(googleDocFitScale(452, 22, 22) === 0.5, "Docs fit width should scale to half width");
  assert(googleDocFitScale(0, 22, 22) === 0.1, "Docs fit width should retain a visible minimum scale");
  const documentFile = {
    ...sheetPreview.file,
    mime_type: "application/vnd.google-apps.document",
  };
  assert(googleFileKindDetail(documentFile).includes("Click text to edit"), "Editable Docs should explain inline editing");
  assert(
    googleFileKindDetail({
      ...documentFile,
      capabilities: { ...documentFile.capabilities, can_edit: false },
    }) === "Google Docs",
    "View-only Docs should omit the editing hint",
  );
  const slidesMarkup = renderToStaticMarkup(createElement(SlidesPreview, {
    fileId: "slides_123",
    active: true,
    canEdit: true,
    pageAspectRatio: 16 / 9,
    pageWidth: 720,
    pageHeight: 405,
    onSaved: () => {},
    slides: [
      {
        objectId: "slide_1",
        text: "Opening",
        notes: "Welcome everyone",
        notesObjectId: "notes_1",
        textElements: [{
          objectId: "title_1",
          text: "Opening",
          styleRuns: [{ start: 0, end: 7, style: {
            fontFamily: "Aptos",
            fontSize: { magnitude: 32, unit: "PT" },
          } }],
          paragraphRuns: [{ start: 0, end: 7, style: { alignment: "CENTER" } }],
          contentAlignment: "MIDDLE",
          fontScale: 0.9,
          x: 0.1,
          y: 0.1,
          width: 0.8,
          height: 0.2,
        }],
        elements: [{
          objectId: "title_1",
          kind: "shape",
          order: 0,
          x: 0.1,
          y: 0.1,
          width: 0.8,
          height: 0.2,
          baseWidth: 576,
          baseHeight: 81,
        }],
      },
      {
        objectId: "slide_2",
        text: "Details",
        notes: "",
        textElements: [{ objectId: "title_2", text: "Details", x: 0.1, y: 0.1, width: 0.8, height: 0.2 }],
      },
    ],
  }));
  assert(slidesMarkup.includes('aria-label="Previous slide"'), "Slides preview should expose previous-slide navigation");
  assert(slidesMarkup.includes('aria-label="Next slide"'), "Slides preview should expose next-slide navigation");
  assert(slidesMarkup.includes('aria-label="Search slides"'), "Slides preview should expose presentation search");
  assert(slidesMarkup.includes('aria-label="Slide zoom"'), "Slides preview should expose zoom");
  assert(slidesMarkup.includes('<option value="fit" selected="">Fit</option>'), "Slides preview should default to one unambiguous fit zoom mode");
  assert(slidesMarkup.includes('aria-label="Hide slide thumbnails"'), "Slides preview should expose a thumbnail-rail toggle");
  assert(slidesMarkup.includes('aria-label="New slide"'), "Editable Slides previews should create slides inline");
  assert(slidesMarkup.includes('aria-label="Duplicate slide"'), "Editable Slides previews should duplicate slides inline");
  assert(slidesMarkup.includes('aria-label="Delete slide"'), "Editable Slides previews should delete slides inline");
  assert(slidesMarkup.includes('aria-label="Slide 1"'), "Slide thumbnails should expose number-only navigation labels");
  assert(slidesMarkup.includes('aria-label="Duplicate slide 1"'), "Slide thumbnail actions should expose duplication on hover and focus");
  assert(slidesMarkup.includes('aria-label="Delete slide 1"'), "Slide thumbnail actions should expose deletion on hover and focus");
  assert(!slidesMarkup.includes("<small>Opening</small>"), "Slide thumbnails should not repeat slide text beneath the preview");
  assert(slidesMarkup.includes('aria-haspopup="dialog"'), "Slides preview should expose presentation mode");
  assert(slidesMarkup.includes(">Present<"), "Slides preview should label the presentation action");
  assert(slidesMarkup.includes(">Notes<"), "Slides preview should expose available speaker notes");
  assert(!slidesMarkup.includes(">Edit slide text<"), "Slides text should not require a separate edit-mode toggle");
  assert(slidesMarkup.includes('class="google-slide-inline-editor"'), "Editable slide text boxes should always be directly interactive");
  assert(slidesMarkup.includes('contenteditable="true"'), "Slides should edit text in place without textarea overlays");
  assert(slidesMarkup.includes('data-content-alignment="MIDDLE"'), "Slides editors should preserve vertical text alignment");
  assert(slidesMarkup.includes("text-align:center"), "Slides editors should preserve paragraph alignment");
  assert(!slidesMarkup.includes("google-slide-format-toolbar"), "Slides should not render a floating selection toolbar");
  assert(slidesMarkup.includes("google-slides-unified-toolbar"), "Slides should keep contextual formatting in the top command bar");
  assert(slidesMarkup.includes('aria-label="Loading slide preview"'), "Slides should use a neutral thumbnail loading state");
  assert(!slidesMarkup.includes("<strong>Slide "), "Slides should not render flattened text as a fake slide preview");
  assert(slidesMarkup.includes('aria-current="page"'), "Slides preview should mark the active slide");
  assert(googleSlidesNavigationAction("ArrowLeft") === "previous", "Slides should move backward with ArrowLeft");
  assert(googleSlidesNavigationAction("PageDown") === "next", "Slides should move forward with PageDown");
  assert(googleSlidesNavigationAction(" ") === "next", "Slides should move forward with Space");
  assert(googleSlidesNavigationAction("Home") === "first", "Slides should jump to the first slide with Home");
  assert(googleSlidesNavigationAction("End") === "last", "Slides should jump to the last slide with End");
  assert(googleSlidesNavigationAction("Escape") === "exit", "Slides should exit presentation mode with Escape");
  assert(googleSlidesNavigationAction("Enter") === null, "Slides should ignore unrelated keys");
  assert(
    googleSlideThumbnailRequestKey("deck", "slide", 1) !== googleSlideThumbnailRequestKey("deck", "slide", 2),
    "Slides should refresh thumbnails in the background after each reconciled save",
  );
  const movedRect = googleSlideGestureRect({ x: 0.1, y: 0.1, width: 0.2, height: 0.2 }, 0.9, 0.9, "move");
  assert(movedRect.x === 0.8 && movedRect.y === 0.8, "Slide movement should remain inside the canvas");
  const resizedRect = googleSlideGestureRect({ x: 0.2, y: 0.2, width: 0.3, height: 0.3 }, -0.1, -0.1, "nw");
  assert(resizedRect.x === 0.1 && resizedRect.y === 0.1 && resizedRect.width === 0.4 && resizedRect.height === 0.4, "Slide corner handles should resize from the selected edge");
  const marqueeRect = googleSlideMarqueeRect(0.8, 0.7, 0.2, 0.1);
  assert(marqueeRect.x === 0.2 && marqueeRect.y === 0.1 && Math.abs(marqueeRect.width - 0.6) < 1e-9 && Math.abs(marqueeRect.height - 0.6) < 1e-9, "Slide marquee selection should work in every drag direction");
  assert(googleSlideRectsIntersect(marqueeRect, { x: 0.4, y: 0.3, width: 0.1, height: 0.1 }), "Slide marquee selection should include intersecting elements");
  assert(!googleSlideRectsIntersect(marqueeRect, { x: 0.85, y: 0.8, width: 0.1, height: 0.1 }), "Slide marquee selection should exclude outside elements");
  const groupBounds = googleSlideGroupBounds([
    { x: 0.1, y: 0.2, width: 0.2, height: 0.2 },
    { x: 0.5, y: 0.4, width: 0.1, height: 0.3 },
  ]);
  assert(groupBounds?.x === 0.1 && groupBounds.y === 0.2 && groupBounds.width === 0.5 && Math.abs(groupBounds.height - 0.5) < 1e-9, "Slide group bounds should contain every selected element");
  const groupedRect = googleSlideTransformGroup(
    { x: 0.5, y: 0.4, width: 0.1, height: 0.3 },
    groupBounds!,
    { x: 0.2, y: 0.1, width: 0.25, height: 0.25 },
  );
  assert(Math.abs(groupedRect.x - 0.4) < 1e-9 && Math.abs(groupedRect.y - 0.2) < 1e-9 && Math.abs(groupedRect.width - 0.05) < 1e-9 && Math.abs(groupedRect.height - 0.15) < 1e-9, "Slide group resizing should preserve each element's relative position and size");
  const snappedRect = googleSlideSnapRect(
    { x: 0.29, y: 0.19, width: 0.2, height: 0.2 },
    [{ x: 0.5, y: 0.4, width: 0.2, height: 0.2 }],
    "move",
    0.02,
    0.02,
  );
  assert(snappedRect.rect.x === 0.3 && snappedRect.rect.y === 0.2 && snappedRect.guideX === 0.5 && snappedRect.guideY === 0.4, "Slide elements should snap matching edges and expose alignment guides");
  const slideFormat = googleSlideTextFormat({
    id: "title_1",
    label: "Title",
    text: "Opening",
    kind: "shape",
    styleRuns: [{ start: 0, end: 7, style: {
      bold: true,
      fontSize: { magnitude: 24, unit: "PT" },
      foregroundColor: { opaqueColor: { rgbColor: { red: 0.2, green: 0.4, blue: 0.6 } } },
    } }],
    paragraphRuns: [{ start: 0, end: 7, style: { alignment: "CENTER" } }],
  }, 2);
  assert(slideFormat.bold && slideFormat.fontSize === 24, "Slides toolbar should read the selected text style");
  assert(slideFormat.color === "#336699" && slideFormat.alignment === "CENTER", "Slides toolbar should read color and alignment");
  const slideSegments = googleSlideTextSegments({
    id: "title_2",
    label: "Title",
    text: "A😀BC",
    kind: "shape",
    styleRuns: [
      { start: 0, end: 3, style: { bold: true } },
      { start: 3, end: 5, style: { italic: true } },
    ],
    paragraphRuns: [{ start: 0, end: 5, style: { alignment: "CENTER" } }],
  });
  assert(
    slideSegments.length === 2
      && slideSegments[0].text === "A😀"
      && slideSegments[0].end === 3
      && slideSegments[0].format.bold
      && slideSegments[1].text === "BC"
      && slideSegments[1].start === 3
      && slideSegments[1].format.italic,
    "Slides editors should preserve mixed UTF-16 character runs while editing",
  );
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
  const appNativeLabel = previewNativeWebviewLabel("runtime_browser", "private");
  assert(appNativeLabel === "artifact-browser-app-private-slot", "App previews should use the bounded private native slot");
  assert(
    previewNativeWebviewLabel("native_browser", "persistent") === "artifact-browser-url-persistent-slot",
    "Persistent URL previews should use their bounded native slot",
  );
  assert(previewNativeWebviewLabel("native_browser", "private") === "artifact-browser-url-private-slot", "Private URL previews should stay isolated from persistent previews");
  let leaveReason = "";
  const unregisterEditorGuard = registerWorkspaceEditorGuard(async (reason) => {
    leaveReason = reason;
    return false;
  });
  assert(!(await requestWorkspaceEditorLeave("quit")) && leaveReason === "quit", "Dirty workspace navigation should route through the active editor guard");
  unregisterEditorGuard();
  assert(await requestWorkspaceEditorLeave("navigate"), "Workspace navigation should continue after the editor guard unmounts");
  const previewPanelSource = readFileSync("src/components/PreviewPanel.tsx", "utf8");
  const previewWebviewSource = readFileSync("src-tauri/src/preview_webview.rs", "utf8");
  const workspaceCodePanelSource = readFileSync("src/components/WorkspaceCodePanel.tsx", "utf8");
  const workspaceCodeEditorSource = readFileSync("src/components/WorkspaceCodeEditor.tsx", "utf8");
  assert(!previewPanelSource.includes("window.prompt"), "Preview review comments should use the themed dialog instead of a native prompt");
  assert(previewPanelSource.includes('data-testid="review-comment-dialog"'), "Preview review comments should keep a testable themed dialog");
  assert(previewPanelSource.includes("setPreviewWebviewZoom(label, claim.claimToken, zoomPercentRef.current / 100)"), "Native previews should restore saved zoom through the current Rust claim");
  assert(previewPanelSource.includes("setPreviewWebviewZoom(claim.label, claim.claimToken, zoomPercent / 100)"), "Native previews should apply zoom changes without recreating the webview");
  assert(previewPanelSource.includes("setPreviewWebviewMuted(label, claim.claimToken, mutedRef.current)"), "Native previews should restore persisted tab mute through the current Rust claim");
  assert(previewPanelSource.includes("setPreviewWebviewBounds(claim.label, claim.claimToken, rect)"), "Native preview geometry should reject stale slot owners");
  assert(previewPanelSource.includes("navigation.claimToken !== claim.claimToken"), "Pooled navigation events should reject superseded logical tab claims");
  assert(previewPanelSource.includes('PREVIEW_CONTROL_OVERLAY_LABEL = "artifact-overlay-preview"'), "Preview activity should reuse one native overlay window");
  assert(previewPanelSource.includes("active ? tab.url ? googleTarget"), "Inactive tabs should not mount browser preview surfaces");
  assert(previewPanelSource.includes("MAX_PREVIEW_BROWSER_TABS = 24"), "Live preview tabs should have the same bound as restored sessions");
  assert(!previewPanelSource.includes("closePreviewWebview"), "Routine preview lifecycle should not expose controller close calls");
  assert(previewWebviewSource.includes("current_url.as_str() != payload.url().as_str()"), "Superseded page-load completions should not cross pooled claims");
  assert(!previewWebviewSource.includes("webview.close()"), "Native preview maintenance should avoid synchronous controller close calls");
  assert(previewPanelSource.includes('label: tab.muted ? "Unmute tab" : "Mute tab"'), "Preview tab menus should toggle audio mute");
  assert(previewPanelSource.includes('shortcut.action === "zoom_in"'), "Native preview shortcuts should update the persisted zoom preference");
  assert(workspaceCodePanelSource.includes('data-testid="workspace-code-rail-resizer"'), "Workspace Code should expose a keyboard-resizable file rail");
  assert(workspaceCodePanelSource.includes('aria-label="Collapse file rail"'), "Workspace Code should expose a collapsible file rail");
  assert(workspaceCodePanelSource.includes("listWorkspaceFiles(workspace, query, 50)"), "Workspace Code search should use recursive native search");
  assert(workspaceCodePanelSource.includes("writeWorkspaceTextFile"), "Workspace Code should save through the conflict-safe native API");
  assert(workspaceCodePanelSource.includes("Reload") && workspaceCodePanelSource.includes("Overwrite"), "Workspace conflicts should keep both explicit recovery actions");
  assert(workspaceCodeEditorSource.includes('key: "Mod-s"'), "Workspace Code should expose the platform save shortcut");
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
        { id: "second", url: "https://sheets.google.com/", input: "https://sheets.google.com/", history: ["https://sheets.google.com/"], historyIndex: 0, muted: true },
      ],
    },
  });
  assert(tabbedUrlMarkup.includes("example.com") && tabbedUrlMarkup.includes("sheets.google.com"), "Browser sessions should render every tab");
  assert(tabbedUrlMarkup.includes('aria-selected="true"'), "Browser sessions should identify the active tab");
  assert(tabbedUrlMarkup.includes('data-testid="preview-browser-tab-muted"'), "Muted browser tabs should expose a visible muted indicator");
  assert(tabbedUrlMarkup.includes('aria-label="sheets.google.com, muted"'), "Muted browser tabs should expose their state accessibly");

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

  const htmlMarkup = renderPreviewPanel({ artifact: htmlArtifact, onClose: () => {} });
  assert(!htmlMarkup.includes('data-testid="preview-browser-bar"'), "HTML artifacts should not render browser chrome");
  assert(!htmlMarkup.includes('data-testid="preview-open-browser"'), "Preview source switching should not use a separate globe shortcut");
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
  assert(codeMarkup.includes('aria-label="Code sources"'), "Code should render one persistent source rail");
  assert(codeMarkup.includes("Generated") && codeMarkup.includes("read-only"), "Generated artifacts should be clearly labelled read-only");
  assert(codeMarkup.includes('data-testid="preview-code-line-number" aria-hidden="true"'), "Visual line numbers should be hidden from assistive technology");
  const highlightedCodeMarkup = renderToStaticMarkup(createElement(SourceCodeView, {
    source: "/* first\nsecond */\nconst answer: number = 42;",
    language: "typescript",
    ariaLabel: "Highlighted source",
  }));
  assert((highlightedCodeMarkup.match(/hljs-comment/g) ?? []).length === 2, "Source code should preserve multiline syntax highlighting");
  assert(highlightedCodeMarkup.includes("hljs-keyword"), "Source code should reuse the existing syntax theme classes");
  const reviewCodeMarkup = renderToStaticMarkup(createElement(SourceCodeView, {
    source: "first\nsecond",
    ariaLabel: "Review source",
    selectedLine: 2,
    onLineClick: () => {},
  }));
  assert(reviewCodeMarkup.includes('<button type="button"'), "Review source should keep line-level comment controls");
  assert(reviewCodeMarkup.includes("preview-code-line selected"), "Review source should expose its selected line");
  const workspaceMarkup = renderPreviewPanel({ artifact: workspaceArtifact, activeTab: "code", workspaceFolder: "C:\\workspace", onClose: () => {} });
  assert(workspaceMarkup.includes('aria-label="Search workspace files"'), "Workspace Code should lead with recursive workspace search");
  assert(workspaceMarkup.includes("Select a workspace file"), "Opening Code directly should show the workspace editor empty state");
  assert(!workspaceMarkup.includes('data-testid="preview-code-source"'), "Workspace-only Code should omit the generated placeholder source");

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
  assert(runtimeMarkup.includes("Refresh commands"), "Runtime review should name the read-only command check");
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

  const staticRuntimeStatus = {
    ...readyRuntimeStatus,
    kind: "static",
    command: null,
    preflight: null,
    message: "Serving index.html.",
  };
  const staticRuntimeMarkup = renderPreviewPanel({
    artifact: urlArtifact,
    previewSource: "app",
    runtimeStatus: staticRuntimeStatus,
    runtimeStale: true,
    onRuntimeStop: () => {},
    onClose: () => {},
  });
  assert(staticRuntimeMarkup.includes("Static preview"), "Static runtime should identify itself without app command controls");
  assert(staticRuntimeMarkup.includes("preview-managed-runtime stale compact"), "Ready static runtime details should stay compact");
  assert(!staticRuntimeMarkup.includes("Serving index.html."), "Ready static runtime should suppress redundant serving copy");
  assert(!staticRuntimeMarkup.includes('<span role="status" aria-live="polite">Ready · disconnected</span>'), "Ready static runtime should not repeat its status inside the compact panel");
  assert(!staticRuntimeMarkup.includes('data-testid="preview-runtime-restart"'), "Static runtime should omit Restart");
  assert(!staticRuntimeMarkup.includes('data-testid="preview-runtime-preflight"'), "Static runtime should omit command preflight");

  const healthyStaticRuntimeMarkup = renderPreviewPanel({
    artifact: urlArtifact,
    previewSource: "app",
    runtimeStatus: staticRuntimeStatus,
    onRuntimeStop: () => {},
    onClose: () => {},
  });
  assert(!healthyStaticRuntimeMarkup.includes('data-testid="preview-managed-runtime"'), "Healthy static runtime should use only the compact toolbar");
  assert(healthyStaticRuntimeMarkup.includes('preview-runtime-badge-label" role="status" aria-live="polite">Static preview</span>'), "Healthy static runtime should name the preview in its compact badge");
  assert(healthyStaticRuntimeMarkup.includes("preview-runtime-quick-stop labeled"), "Healthy static runtime should keep a compact labeled Stop action");
  assert(!healthyStaticRuntimeMarkup.includes("Serving index.html."), "Healthy static runtime should omit serving copy from the compact toolbar");

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
