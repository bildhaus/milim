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
  renderMermaid?: boolean;
  sourceLinks?: boolean;
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
  const { Markdown, MemoizedMarkdown, hasClosedMermaidFence, isHttpHref, parseMarkdownIntoBlocks, sourceLinkDetails } = await server.ssrLoadModule("/src/components/Markdown.tsx") as {
    Markdown: ComponentType<MarkdownProps>;
    MemoizedMarkdown: ComponentType<MarkdownProps>;
    hasClosedMermaidFence: (content: string) => boolean;
    isHttpHref: (href: string | undefined) => boolean;
    parseMarkdownIntoBlocks: (content: string) => string[];
    sourceLinkDetails: (href: string | undefined) => { host: string; path: string } | null;
  };
  const { MermaidDiagram, boundedRasterDimensions, mermaidSvgDimensions, standaloneMermaidSvg } = await server.ssrLoadModule("/src/components/MermaidDiagram.tsx") as {
    MermaidDiagram: ComponentType<{ source: string }>;
    boundedRasterDimensions: (width: number, height: number) => { width: number; height: number };
    mermaidSvgDimensions: (svg: string) => { width: number; height: number };
    standaloneMermaidSvg: (svg: string) => string;
  };

  function renderMarkdown(
    content: string,
    previewArtifacts?: ChatArtifact[],
    previewArtifactsStreaming = false,
    collapseArtifacts = true,
    allowHtml = false,
    renderMermaid = false,
    sourceLinks = false,
  ): string {
    return renderToStaticMarkup(createElement(Markdown, {
      content,
      previewArtifacts,
      onOpenPreview: () => {},
      highlight: false,
      allowHtml,
      previewArtifactsStreaming,
      collapseArtifacts,
      renderMermaid,
      sourceLinks,
    }));
  }

  function renderMemoizedMarkdown(
    content: string,
    previewArtifacts?: ChatArtifact[],
    renderMermaid = false,
  ): string {
    return renderToStaticMarkup(createElement(MemoizedMarkdown, {
      content,
      previewArtifacts,
      onOpenPreview: () => {},
      highlight: false,
      collapseArtifacts: false,
      renderMermaid,
    }));
  }

  assert(isHttpHref("https://milim.ai/docs"), "https links should use the native browser opener");
  assert(isHttpHref("http://localhost:5173"), "localhost http links should use the native browser opener");
  assert(!isHttpHref("#section"), "page anchors should keep default markdown behavior");
  assert(!isHttpHref("mailto:test@example.com"), "non-http links should keep default markdown behavior");
  equal(sourceLinkDetails("https://www.prompt-kit.com/docs?ref=chat")?.host, "prompt-kit.com", "source links should show a compact host");
  equal(sourceLinkDetails("https://www.prompt-kit.com/docs?ref=chat")?.path, "prompt-kit.com/docs", "source previews should omit query parameters");
  equal(sourceLinkDetails("#section"), null, "page anchors should not become source links");

  const ordinaryLink = renderMarkdown("[Prompt Kit](https://www.prompt-kit.com/docs)");
  assert(!ordinaryLink.includes("md-source-link"), "ordinary Markdown surfaces should keep regular links");
  const assistantSourceLink = renderMarkdown("[Prompt Kit](https://www.prompt-kit.com/docs?ref=chat)", undefined, false, true, false, false, true);
  assert(assistantSourceLink.includes("md-source-link"), "assistant sources should render as source chips");
  assert(assistantSourceLink.includes("prompt-kit.com/docs"), "source chips should expose hover path details");
  assert(assistantSourceLink.includes("opens in browser"), "source chips should announce external navigation");

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

  const longCodexStream = [
    "**Codex stream**",
    ...Array.from({ length: 1_000 }, (_, index) => `soft-wrapped-token-${index}`),
  ].join("\n");
  assert(longCodexStream.length > 12_000, "Codex streaming fixture should cover the former plain-text cutoff");
  const longStreamingMarkdown = renderMemoizedMarkdown(longCodexStream);
  assert(longStreamingMarkdown.includes("<strong>Codex stream</strong>"), "long streaming answers should retain Markdown formatting");
  equal(count(longStreamingMarkdown, "<p>"), 1, "soft newlines in long streaming answers should remain one Markdown paragraph");
  assert(!longStreamingMarkdown.includes("<br"), "soft newlines in long streaming answers should not become forced line breaks");

  const mermaidSource = ["```mermaid", "flowchart LR", "A --> B", "```"].join("\n");
  assert(hasClosedMermaidFence(mermaidSource), "closed Mermaid fences should be detected");
  assert(!hasClosedMermaidFence(["```mermaid", "flowchart LR", "A --> B"].join("\n")), "open Mermaid fences should remain code while streaming");
  assert(hasClosedMermaidFence(["~~~MERMAID", "flowchart LR", "A --> B", "~~~"].join("\n")), "tilde Mermaid fences should be detected case-insensitively");

  const transcriptMermaid = renderMarkdown(mermaidSource, undefined, false, true, false, true);
  assert(transcriptMermaid.includes('data-testid="mermaid-diagram"'), "transcript Mermaid fences should render a diagram card");
  assert(transcriptMermaid.includes("Rendering diagram..."), "diagram cards should expose a rendering status before the lazy renderer completes");
  assert(transcriptMermaid.includes("Image clipboard is unavailable"), "unsupported image clipboard environments should disable that action clearly");

  const nonTranscriptMermaid = renderMarkdown(mermaidSource);
  assert(nonTranscriptMermaid.includes("<pre>"), "Markdown surfaces without transcript opt-in should keep Mermaid as code");
  assert(!nonTranscriptMermaid.includes('data-testid="mermaid-diagram"'), "non-transcript Markdown should not render Mermaid diagrams");

  const streamingOpenMermaid = renderMemoizedMarkdown(["```mermaid", "flowchart LR", "A --> B"].join("\n"), undefined, true);
  assert(streamingOpenMermaid.includes("<pre>"), "incomplete streaming Mermaid fences should remain code");
  assert(!streamingOpenMermaid.includes('data-testid="mermaid-diagram"'), "incomplete streaming Mermaid fences should not invoke the renderer");
  const streamingClosedMermaid = renderMemoizedMarkdown(mermaidSource, undefined, true);
  assert(streamingClosedMermaid.includes('data-testid="mermaid-diagram"'), "completed streaming Mermaid fences should promote to diagrams");

  const regularFenceWithMermaidEnabled = renderMarkdown("```ts\nconst value = 1;\n```", undefined, false, true, false, true);
  assert(regularFenceWithMermaidEnabled.includes("<pre>"), "ordinary code fences should remain code when Mermaid rendering is enabled");
  assert(!regularFenceWithMermaidEnabled.includes('data-testid="mermaid-diagram"'), "ordinary code fences should not become diagrams");

  const standaloneSvg = standaloneMermaidSvg('<svg viewBox="0 0 900 300"></svg>');
  assert(standaloneSvg.startsWith('<?xml version="1.0"'), "SVG downloads should include an XML declaration");
  assert(standaloneSvg.includes('xmlns="http://www.w3.org/2000/svg"'), "SVG downloads should include the SVG namespace");
  equal(mermaidSvgDimensions(standaloneSvg).width, 900, "SVG export should read viewBox width");
  equal(mermaidSvgDimensions(standaloneSvg).height, 300, "SVG export should read viewBox height");
  equal(boundedRasterDimensions(900, 300).width, 1800, "PNG export should rasterize at 2x when within bounds");
  equal(boundedRasterDimensions(3000, 1500).width, 4096, "PNG export should cap its longest dimension");
  equal(boundedRasterDimensions(3000, 1500).height, 2048, "PNG export should preserve aspect ratio at the cap");

  const directMermaidCard = renderToStaticMarkup(createElement(MermaidDiagram, { source: "flowchart LR\nA --> B" }));
  assert(directMermaidCard.includes("Copy Mermaid code"), "diagram cards should keep source-copy access in diagram view");
  assert(directMermaidCard.includes("Download SVG"), "diagram cards should expose SVG export");
  assert(directMermaidCard.includes("Download PNG"), "diagram cards should expose PNG export");

  const { default: mermaid } = await import("mermaid");
  mermaid.initialize({ startOnLoad: false, securityLevel: "strict", suppressErrorRendering: true });
  const parsedMermaid = await mermaid.parse("flowchart LR\nA --> B");
  assert(Boolean(parsedMermaid), "valid Mermaid source should parse");
  let invalidMermaidRejected = false;
  try {
    await mermaid.parse("flowchart ???");
  } catch {
    invalidMermaidRejected = true;
  }
  assert(invalidMermaidRejected, "invalid Mermaid source should reach the renderer error path");

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
