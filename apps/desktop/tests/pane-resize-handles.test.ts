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
  ];
  const paneHandleCount = componentFiles.reduce((count, path) => {
    const source = readFileSync(path, "utf8");
    assert(source.includes('from "./PaneResizeHandle"'), `${path} should use the shared pane handle`);
    return count + (source.match(/<PaneResizeHandle\b/g)?.length ?? 0);
  }, 0);
  assert(paneHandleCount === 8, `All eight structural separators should use PaneResizeHandle, found ${paneHandleCount}`);

  const foundation = readFileSync("src/foundation.css", "utf8");
  const shell = readFileSync("src/shell.css", "utf8");
  const chat = readFileSync("src/chat.css", "utf8");
  const workspaces = readFileSync("src/workspaces.css", "utf8");
  assert(foundation.includes("--pane-resize-target: 12px"), "Pane resize targets should share a 12px hit area");
  assert(foundation.includes("--pane-resize-indicator-thickness: 2px"), "Pane resize indicators should share one thickness");
  assert(foundation.includes("--pane-resize-indicator-length: 42px"), "Pane resize indicators should share one resting length");
  assert(foundation.includes("--pane-resize-indicator-active-length: 64px"), "Pane resize indicators should share one active length");

  assert(/\.sidebar-resize-handle\s*\{[^}]*right:\s*0/s.test(shell), "The sidebar handle should center on its inset visible edge");
  assert(/\.preview-resize-handle\s*\{[^}]*margin-inline:\s*calc\(-1 \* var\(--pane-resize-half-target\)\)/s.test(chat), "The Inspector handle should straddle its seam without adding a gap");
  assert(/\.workspace-code-rail-resizer\s*\{[^}]*left:\s*var\(--workspace-code-rail-width\)/s.test(chat), "The Code rail handle should center on the rail width");
  assert(/\.git-diff-resize-handle\s*\{[^}]*left:\s*var\(--git-diff-navigator-width/s.test(shell), "The Git handle should center on the navigator width");
  assert(/\.pull-requests-divider\s*\{[^}]*left:\s*var\(--pull-requests-list-width/s.test(workspaces), "The Pull Requests handle should center on the list width");
  assert(/\.media-library-resize-handle\s*\{[^}]*left:\s*calc\(-1 \* var\(--pane-resize-half-target\)\)/s.test(workspaces), "The Media Library handle should straddle its seam");
  assert(workspaces.includes("--media-output-half-gap: 6px") && workspaces.includes("--media-output-half-gap: 4px"), "The Media composer handle should derive its center from each responsive gap");
  assert(/\.preview-log-resize-handle\s*\{[^}]*top:\s*calc\(-1 \* var\(--pane-resize-half-target\)\)/s.test(chat), "The Logs handle should straddle the drawer seam");

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
    assert(!localIndicator.test(`${shell}\n${chat}\n${workspaces}`), `${selector} should not redefine the shared indicator`);
  }
} finally {
  await server.close();
}
