import { readFileSync } from "node:fs";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { PaneResizeHandle } = (await server.ssrLoadModule(
    "/src/components/PaneResizeHandle.tsx",
  )) as {
    PaneResizeHandle: ComponentType<{
      orientation: "horizontal" | "vertical";
      className?: string;
      "data-testid"?: string;
    }>;
  };
  const markup = renderToStaticMarkup(createElement(PaneResizeHandle, {
    orientation: "vertical",
    className: "example-resizer",
    "data-testid": "example-resizer",
  }));
  assert(markup.includes('role="separator"'), "Shared pane handles should expose separator semantics");
  assert(markup.includes('aria-orientation="vertical"'), "Shared pane handles should expose their orientation");
  assert(markup.includes('tabindex="0"'), "Shared pane handles should be keyboard focusable by default");
  assert(markup.includes("pane-resize-handle-vertical example-resizer"), "Shared pane handles should retain surface placement classes");

  const componentFiles = [
    "src/components/Sidebar.tsx",
    "src/components/ChatView.tsx",
    "src/components/WorkspaceCodePanel.tsx",
    "src/components/GitPanel.tsx",
    "src/components/PreviewPanel.tsx",
    "src/components/PullRequestsManager.tsx",
    "src/components/MediaManager.tsx",
    "src/components/WorkersInspector.tsx",
    "src/components/GoogleWorkspacePreview.tsx",
    "src/components/AgentsManager.tsx",
    "src/components/SkillsManager.tsx",
    "src/components/SchedulesManager.tsx",
    "src/components/ProvidersManager.tsx",
    "src/components/McpManager.tsx",
    "src/components/MemoryManager.tsx",
    "src/settings/SettingsDialog.tsx",
  ];
  const paneHandleCount = componentFiles.reduce((count, path) => {
    const source = readFileSync(path, "utf8");
    assert(/from "(?:\.\/|\.\.\/components\/)PaneResizeHandle"/.test(source), `${path} should use the shared pane handle`);
    const handles = source.match(/<PaneResizeHandle\b[^>]*>/g) ?? [];
    for (const handle of handles) {
      assert(handle.includes("resize={"), `${path} should drive every pane handle through usePaneResize`);
    }
    return count + handles.length;
  }, 0);
  assert(paneHandleCount === 18, `All eighteen structural separators should use PaneResizeHandle, found ${paneHandleCount}`);
  for (const path of componentFiles) {
    const source = readFileSync(path, "utf8");
    assert(!/Resize(?:Start|Cleanup)Ref\b/.test(source), `${path} should leave drag state to the shared resize hook`);
  }

  const sheetManagers = [
    ["src/components/AgentsManager.tsx", "agents"],
    ["src/components/SkillsManager.tsx", "skills"],
    ["src/components/SchedulesManager.tsx", "schedules"],
    ["src/components/ProvidersManager.tsx", "providers"],
    ["src/components/MemoryManager.tsx", "memory"],
    ["src/components/McpManager.tsx", "mcp"],
    ["src/components/UsageManager.tsx", "usage"],
    ["src/components/MediaManager.tsx", "media"],
    ["src/components/PullRequestsManager.tsx", "pullRequests"],
  ] as const;
  for (const [path, id] of sheetManagers) {
    assert(readFileSync(path, "utf8").includes(`resizable={{ id: "${id}"`), `${path} should use the shared resizable sheet`);
  }

  const foundation = readFileSync("src/foundation.css", "utf8");
  const shell = readFileSync("src/shell.css", "utf8");
  const chat = readFileSync("src/chat.css", "utf8");
  const workspaces = readFileSync("src/workspaces.css", "utf8");
  const inspector = readFileSync("src/inspector.css", "utf8");
  const pullRequests = readFileSync("src/pull-requests.css", "utf8");
  const mediaManager = readFileSync("src/media-manager.css", "utf8");
  assert(foundation.includes("--pane-resize-target: 12px"), "Pane resize targets should share a 12px hit area");
  assert(foundation.includes("--pane-resize-indicator-thickness: 2px"), "Pane resize indicators should share one thickness");
  assert(foundation.includes("--pane-resize-indicator-length: 42px"), "Pane resize indicators should share one resting length");
  assert(foundation.includes("--pane-resize-indicator-active-length: 64px"), "Pane resize indicators should share one active length");
  assert(/body\.pane-resizing :is\(iframe, webview, embed, object\)\s*\{[^}]*pointer-events:\s*none/s.test(foundation), "Drags should stop embedded frames from swallowing the pointer");
  assert(/body\.pane-resizing \*\s*\{[^}]*cursor:\s*var\(--pane-resize-cursor/s.test(foundation), "Drags should pin one resize cursor across the window");
  assert(/\.pane-resize-handle:focus-visible\s*\{[^}]*box-shadow/s.test(foundation), "Pane handles should show a focus ring");

  assert(/\.sidebar-resize-handle\s*\{[^}]*right:\s*0/s.test(shell), "The sidebar handle should center on its inset visible edge");
  assert(/\.preview-resize-handle\s*\{[^}]*margin-inline:\s*calc\(-1 \* var\(--pane-resize-half-target\)\)/s.test(chat), "The Inspector handle should straddle its seam without adding a gap");
  assert(/\.workspace-code-rail-resizer\s*\{[^}]*left:\s*var\(--workspace-code-rail-size\)/s.test(inspector), "The Code rail handle should center on the clamped rail width");
  assert(/\.git-diff-resize-handle\s*\{[^}]*left:\s*var\(--git-diff-navigator-size/s.test(shell), "The Git handle should center on the clamped navigator width");
  assert(/\.pull-requests-divider\s*\{[^}]*left:\s*var\(--pull-requests-list-size/s.test(pullRequests), "The Pull Requests handle should center on the clamped list width");
  assert(inspector.includes("calc(100% - 220px)") && shell.includes("calc(100% - 320px)") && pullRequests.includes("calc(100% - 320px)"), "Split rails should clamp their saved width to the live container in CSS");
  assert(/\.media-library-resize-handle\s*\{[^}]*left:\s*calc\(-1 \* var\(--pane-resize-half-target\)\)/s.test(mediaManager), "The Media Library handle should straddle its seam");
  assert(mediaManager.includes("--media-output-half-gap: 6px") && mediaManager.includes("--media-output-half-gap: 4px"), "The Media composer handle should derive its center from each responsive gap");
  assert(/\.preview-log-resize-handle\s*\{[^}]*top:\s*calc\(-1 \* var\(--pane-resize-half-target\)\)/s.test(inspector), "The Logs handle should straddle the drawer seam");

  for (const selector of [
    "sidebar-resize-handle",
    "preview-resize-handle",
    "workspace-code-rail-resizer",
    "git-diff-resize-handle",
    "pull-requests-divider",
    "media-panel-resize-handle",
    "preview-log-resize-handle",
  ]) {
    const localIndicator = new RegExp(`\\.${selector}::(?:before|after)`);
    assert(!localIndicator.test(`${shell}\n${chat}\n${workspaces}\n${inspector}\n${pullRequests}\n${mediaManager}`), `${selector} should not redefine the shared indicator`);
  }

  const panes = (await server.ssrLoadModule("/src/lib/paneSizes.ts")) as typeof import("../src/lib/paneSizes");
  const { PANES, clampPaneSize, migratePaneSizes, normalizePaneSizes, paneBounds, paneDragStep, paneKeyboardTarget, withPaneSize } = panes;

  // Clamp: spec bounds for stored preferences, live bounds for rendering.
  assert(clampPaneSize(500, 220, 420) === 420, "Sizes should clamp to the max");
  assert(clampPaneSize(100.4, 220, 420) === 220, "Sizes should clamp to the min");
  assert(clampPaneSize(300.6, 220, 420) === 301, "Sizes should round to whole pixels");
  assert(clampPaneSize(500, 360, 300) === 360, "A container smaller than the min should still render the min");
  const liveSidebar = paneBounds("sidebar", { max: 300 });
  assert(liveSidebar.min === 220 && liveSidebar.max === 300, "Live bounds should narrow the spec max");
  const crampedInspector = paneBounds("inspector", { max: 200 });
  assert(crampedInspector.max === crampedInspector.min, "Live bounds should never invert");
  assert(paneBounds("sidebar", { max: 9_999 }).max === PANES.sidebar.max, "Live bounds should never exceed the spec max");

  // Sparse registry and reset.
  let sizes = withPaneSize({}, "sidebar", 9_999);
  assert(sizes.sidebar === PANES.sidebar.max, "Stored sizes should clamp to the spec");
  sizes = withPaneSize(sizes, "sidebar", PANES.sidebar.default);
  assert(!("sidebar" in sizes), "Saving the default should clear the preference");
  sizes = withPaneSize(withPaneSize({}, "context", 400), "codeRail", 200);
  sizes = withPaneSize(sizes, "context", null);
  assert(!("context" in sizes) && sizes.codeRail === 200, "Resetting one pane should keep the others");
  const normalized = normalizePaneSizes({ sidebar: 300, unknownPane: 12, inspector: "wide", context: Number.NaN });
  assert(JSON.stringify(normalized) === JSON.stringify({ sidebar: 300 }), `Malformed or unknown sizes should be dropped: ${JSON.stringify(normalized)}`);

  // Old top-level keys migrate once; the registry wins and old defaults are not user choices.
  const migrated = migratePaneSizes({
    sidebarWidth: 384,
    previewPanelWidth: 300,
    mediaStudioWidth: 1120,
    mediaStudioHeight: 700,
    mediaComposerWidth: 336,
    mediaLibraryWidth: 280,
    pullRequestsWidth: 980,
    pullRequestsHeight: 820,
    pullRequestsListWidth: 480,
  });
  assert(migrated.sidebar === 384, "A saved sidebar width should migrate");
  assert(migrated.inspector === PANES.inspector.min, "A narrow legacy inspector width should migrate at the current minimum");
  assert(migrated["mediaSheet.height"] === 700 && !("mediaSheet.width" in migrated), "Only customized media studio axes should migrate");
  assert(migrated.mediaComposer === 336 && !("mediaLibrary" in migrated), "Media panels should migrate unless they held the old default");
  assert(migrated["pullRequestsSheet.width"] === 980 && !("pullRequestsSheet.height" in migrated), "Only customized pull request sheet axes should migrate");
  assert(migrated.pullRequestsList === 480, "The pull request list width should migrate");
  const registryWins = migratePaneSizes({ sidebarWidth: 384, paneSizes: { sidebar: 260, context: 360 } });
  assert(registryWins.sidebar === 260 && registryWins.context === 360, "Registry entries should win over legacy keys");

  // Snap-collapse uses the spec overshoot below the live min.
  const sidebarBounds = paneBounds("sidebar");
  assert(paneDragStep("sidebar", 220 - 96, sidebarBounds, true).kind === "resize", "Collapse should wait for the full 96px overshoot");
  assert(paneDragStep("sidebar", 220 - 97, sidebarBounds, true).kind === "collapse", "Dragging past the overshoot should collapse");
  const pinned = paneDragStep("sidebar", 100, sidebarBounds, false);
  assert(pinned.kind === "resize" && pinned.size === 220, "Non-collapsible panes should pin at the min");
  assert(paneDragStep("previewLogs", 48 - 49, paneBounds("previewLogs"), true).kind === "collapse", "Panes may define their own overshoot");
  assert(paneDragStep("settingsNav", 0, paneBounds("settingsNav"), true).kind === "resize", "Panes without an overshoot never collapse");

  // Keyboard contract: 16px, Shift 64px, Home/End, Enter, axis and direction aware.
  const bounds = { min: 220, max: 420 };
  assert(paneKeyboardTarget("ArrowRight", false, 300, "x", 1, bounds) === 316, "ArrowRight should grow a left pane by 16px");
  assert(paneKeyboardTarget("ArrowRight", true, 300, "x", 1, bounds) === 364, "Shift+ArrowRight should grow by 64px");
  assert(paneKeyboardTarget("ArrowLeft", false, 300, "x", -1, bounds) === 316, "ArrowLeft should grow a right-anchored pane");
  assert(paneKeyboardTarget("ArrowUp", false, 142, "y", -1, { min: 48, max: 360 }) === 158, "ArrowUp should grow a bottom drawer");
  assert(paneKeyboardTarget("ArrowUp", false, 300, "x", 1, bounds) === null, "Cross-axis arrows should be ignored");
  assert(paneKeyboardTarget("ArrowRight", true, 400, "x", 1, bounds) === 420, "Keyboard steps should clamp to the max");
  assert(paneKeyboardTarget("Home", false, 300, "x", 1, bounds) === 220, "Home should jump to the min");
  assert(paneKeyboardTarget("End", false, 300, "x", 1, bounds) === 420, "End should jump to the max");
  assert(paneKeyboardTarget("Enter", false, 300, "x", 1, bounds) === "reset", "Enter should reset");
  assert(paneKeyboardTarget("a", false, 300, "x", 1, bounds) === null, "Other keys should pass through");

  const hookSource = readFileSync("src/ui/usePaneResize.ts", "utf8");
  assert(hookSource.includes("window.requestAnimationFrame(flush)"), "Pointer moves should be throttled to animation frames");
  assert(/end: \(\) => \{[\s\S]*?setPaneSize\(id, drag\.size\)/.test(hookSource), "Pane drags should persist once, on release");
  const moveHandlers = [...hookSource.matchAll(/move: \(dx, dy\) => \{/g)];
  assert(moveHandlers.length === 2, "Pane and sheet drags should each define one move handler");
  for (const { index: from = 0 } of moveHandlers) {
    const moveBody = hookSource.slice(from, hookSource.indexOf("end: () => {", from));
    assert(!/setPaneSizes?\(/.test(moveBody), "Drags should not persist per pointer move");
  }
} finally {
  await server.close();
}
