import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { ChatArtifact } from "../src/api.js";

type MarkdownProps = {
  content: string;
  previewArtifacts?: ChatArtifact[];
  onOpenPreview?: (artifact: ChatArtifact) => void;
  highlight?: boolean;
  allowHtml?: boolean;
  previewArtifactsStreaming?: boolean;
  collapseArtifacts?: boolean;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function count(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

const indexHtml: ChatArtifact = {
  id: "artifact-index",
  kind: "code",
  title: "index.html",
  filename: "index.html",
  language: "html",
  mime: "text/html",
  content: "<div>Card</div>",
  size: 15,
};

const generatedPython: ChatArtifact = {
  id: "artifact-python",
  kind: "code",
  title: "tools/report.py",
  filename: "tools/report.py",
  language: "python",
  mime: "text/plain",
  content: "def report():\n    return 'ready'",
  size: 32,
};

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { Markdown, MemoizedMarkdown, isHttpHref, parseMarkdownIntoBlocks } = await server.ssrLoadModule("/src/components/Markdown.tsx") as {
    Markdown: ComponentType<MarkdownProps>;
    MemoizedMarkdown: ComponentType<MarkdownProps>;
    isHttpHref: (href: string | undefined) => boolean;
    parseMarkdownIntoBlocks: (content: string) => string[];
  };

  function renderMarkdown(
    content: string,
    previewArtifacts?: ChatArtifact[],
    previewArtifactsStreaming = false,
    collapseArtifacts = true,
    allowHtml = false,
  ): string {
    return renderToStaticMarkup(createElement(Markdown, {
      content,
      previewArtifacts,
      onOpenPreview: () => {},
      highlight: false,
      allowHtml,
      previewArtifactsStreaming,
      collapseArtifacts,
    }));
  }

  function renderMemoizedMarkdown(
    content: string,
    previewArtifacts?: ChatArtifact[],
  ): string {
    return renderToStaticMarkup(createElement(MemoizedMarkdown, {
      content,
      previewArtifacts,
      onOpenPreview: () => {},
      highlight: false,
      collapseArtifacts: false,
    }));
  }

  assert(isHttpHref("https://milim.ai/docs"), "https links should use the native browser opener");
  assert(isHttpHref("http://localhost:5173"), "localhost http links should use the native browser opener");
  assert(!isHttpHref("#section"), "page anchors should keep default markdown behavior");
  assert(!isHttpHref("mailto:test@example.com"), "non-http links should keep default markdown behavior");

  const emptyFences = renderMarkdown([
    "```html",
    "```",
    "",
    "```css",
    "```",
  ].join("\n"));
  equal(count(emptyFences, "code-block"), 0, "empty fences should not render copy-only code blocks");
  assert(!emptyFences.includes("Copy"), "empty fences should not render copy buttons");

  const mixedFences = renderMarkdown([
    "```html",
    "```",
    "",
    "```html",
    indexHtml.content,
    "```",
  ].join("\n"), [indexHtml]);
  equal(count(mixedFences, "code-block-collapsed"), 1, "only the matching html block should collapse to an artifact card");
  equal(count(mixedFences, "code-artifact-title"), 1, "artifact title should appear once");
  assert(mixedFences.includes("index.html"), "collapsed artifact card should keep the artifact filename");

  const matchingFence = renderMarkdown([
    "```html",
    indexHtml.content,
    "```",
  ].join("\n"), [indexHtml]);
  equal(count(matchingFence, "code-block-collapsed"), 1, "matching html block should collapse to an artifact card");
  assert(!matchingFence.includes("<pre>"), "collapsed artifact should not render a raw pre block");

  const userFence = renderMarkdown([
    "```html",
    indexHtml.content,
    "```",
  ].join("\n"), [indexHtml], false, false);
  equal(
    count(userFence, "code-block-collapsed"),
    0,
    "user markdown should not collapse matching code into an artifact card",
  );
  assert(userFence.includes("<pre>"), "user markdown should render matching code as a normal code block");
  assert(!userFence.includes("Open preview"), "user markdown should not render preview actions");
  assert(!userFence.includes("Open code"), "user markdown should not render code panel actions");

  const matchingFileFence = renderMarkdown([
    "```python file=tools/report.py",
    generatedPython.content,
    "```",
  ].join("\n"), [generatedPython]);
  equal(count(matchingFileFence, "code-block-collapsed"), 1, "generated file code should collapse even when it is not previewable");
  assert(!matchingFileFence.includes("<pre>"), "collapsed generated files should not render raw source inline");
  assert(matchingFileFence.includes("Open code"), "non-previewable generated files should open the code panel");

  const streamingFence = renderMarkdown([
    "```html",
    `${indexHtml.content}   `,
    "```",
  ].join("\n"), [indexHtml], true);
  equal(count(streamingFence, "code-block-collapsed"), 1, "streaming preview artifact blocks should stay collapsed");
  assert(!streamingFence.includes("<pre>"), "streaming preview artifact should not render a raw pre block");
  assert(streamingFence.includes("Streaming..."), "streaming preview artifact should show streaming status");
  assert(!streamingFence.includes("15 B"), "streaming preview artifact should hide final byte size");

  const streamingMarkdown = renderMemoizedMarkdown([
    "**Ready** for [docs](https://milim.ai/docs)",
    "",
    "- first",
    "- second",
    "",
    "| name | value |",
    "|---|---:|",
    "| alpha | 1 |",
    "",
    "```ts",
    "const value = 1;",
    "```",
  ].join("\n"));
  assert(streamingMarkdown.includes("<strong>Ready</strong>"), "streaming markdown should render bold text");
  assert(streamingMarkdown.includes('href="https://milim.ai/docs"'), "streaming markdown should render links");
  assert(streamingMarkdown.includes("<ul>"), "streaming markdown should render lists");
  assert(streamingMarkdown.includes("<table>"), "streaming markdown should render tables");
  assert(streamingMarkdown.includes("<pre>"), "streaming markdown should render code fences");
  assert(!streamingMarkdown.includes("hljs"), "streaming markdown should skip syntax highlighting");

  const escapedHtml = renderMarkdown("<sub>Posted by a GitHub App.</sub>");
  assert(escapedHtml.includes("&lt;sub&gt;"), "raw HTML should remain escaped outside opted-in surfaces");

  const safeGitHubHtml = renderMarkdown([
    "<!-- compai-review-progress -->",
    '<sub>Posted by <a href="https://trycomp.ai">Comp AI Code Reviews</a>.</sub>',
    "",
    '<a href="https://app.tripwire.sh/runs/example"><img src="https://app.tripwire.sh/badges/view-run.png" width="185" height="40" alt="View on Tripwire" onerror="alert(1)" /></a>',
    "",
    '<img src="https://github.com/user-attachments/assets/example" width="500" height="410" alt="Screenshot" />',
    "",
    '<script>alert("unsafe")</script>',
    '<a href="javascript:alert(1)">Unsafe link</a>',
  ].join("\n"), undefined, false, true, true);
  assert(safeGitHubHtml.includes("<sub>Posted by"), "opted-in GitHub HTML should render subscript text");
  assert(safeGitHubHtml.includes('href="https://trycomp.ai"'), "safe HTML links should render");
  assert(safeGitHubHtml.includes('src="https://app.tripwire.sh/badges/view-run.png"'), "linked badge images should render");
  assert(safeGitHubHtml.includes('width="185"'), "safe image width should be preserved");
  assert(safeGitHubHtml.includes('height="410"'), "safe attachment height should be preserved");
  assert(!safeGitHubHtml.includes("compai-review-progress"), "HTML comments should not render");
  assert(!safeGitHubHtml.includes("<script"), "scripts should be removed");
  assert(!safeGitHubHtml.includes("onerror"), "event handlers should be removed");
  assert(!safeGitHubHtml.includes('href="javascript:'), "unsafe link protocols should be removed");

  const streamingGeneratedCode = renderMemoizedMarkdown([
    "```html",
    indexHtml.content,
    "```",
  ].join("\n"), [indexHtml]);
  equal(
    count(streamingGeneratedCode, "code-block-collapsed"),
    0,
    "streaming markdown should not collapse generated artifacts",
  );
  assert(streamingGeneratedCode.includes("<pre>"), "streaming generated code should render as a code block");

  const startedBlocks = parseMarkdownIntoBlocks([
    "# Title",
    "",
    "Stable paragraph.",
    "",
    "```ts",
    "const value = 1;",
  ].join("\n"));
  const grownBlocks = parseMarkdownIntoBlocks([
    "# Title",
    "",
    "Stable paragraph.",
    "",
    "```ts",
    "const value = 1;",
    "const next = 2;",
  ].join("\n"));
  equal(startedBlocks.length, 3, "streaming splitter should separate completed blocks from the live tail");
  equal(grownBlocks.length, 3, "growing the live tail should not create extra completed blocks");
  equal(grownBlocks[0], startedBlocks[0], "first completed block should stay stable");
  equal(grownBlocks[1], startedBlocks[1], "second completed block should stay stable");
  assert(grownBlocks[2] !== startedBlocks[2], "only the trailing streaming block should change");
} finally {
  await server.close();
}

export {};
