import { artifactDisposition, artifactPreviewAutoOpenKey, defaultArtifactTargetPath, extractArtifactsFromContent, extractArtifactsFromRunTrace, extractLivePreviewArtifactFromContent, extractLocalhostUrlFromRunTrace, hasPreviewPackageJson, isArtifactBrowserUrl, isFileArtifact, isLocalhostPreviewUrl, isPreviewableArtifact, markdownTableToCsv, normalizeArtifactBrowserUrl, previewRuntimeBrowserUrl, previewRuntimeFiles, resolveArtifactBrowserInput } from "../src/lib/artifacts.js";
import { artifactOccurrenceKey, artifactRevisionChoiceByOccurrence, artifactRevisionGroups } from "../src/lib/artifactRevisions.js";
import { buildArtifactPreviewDocument } from "../src/lib/artifactPreview.js";
import { planModeInstructionMessages, threadArtifactInstructionMessages } from "../src/lib/chatInstructions.js";
import { skillInstructionMessage } from "../src/lib/skills.js";
import type { ChatMessage } from "../src/api.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function extensionForTestFile(file: string): string {
  const ext = file.slice(file.lastIndexOf(".") + 1);
  return ext === "json" ? "json" : ext === "css" ? "css" : ext === "html" ? "html" : ext;
}

function sourceForTestFile(file: string): string {
  if (file === "package.json") return '{"scripts":{"dev":"vite"}}';
  if (file.endsWith(".json")) return "{}";
  if (file.endsWith(".html")) return '<div id="root"></div>';
  if (file.endsWith(".css")) return "body { color: white; }";
  if (file.endsWith(".js")) return "export default {};";
  return "export const generated = true;";
}

const code = [
  "Here is the file:",
  "",
  "```ts file=src/generated/greeting.ts",
  "export function greeting(name: string) {",
  "  return `Hello ${name}`;",
  "}",
  "```",
].join("\n");

const codeArtifacts = extractArtifactsFromContent(code);
equal(codeArtifacts.length, 1, "filename fenced block should create one artifact");
equal(codeArtifacts[0].kind, "code", "filename fenced block should be code");
equal(codeArtifacts[0].language, "ts", "fenced info should keep the language");
equal(codeArtifacts[0].filename, "src/generated/greeting.ts", "fenced info should keep the filename");
equal(codeArtifacts[0].disposition, "file", "named fenced blocks should be file artifacts");
assert(isFileArtifact(codeArtifacts[0]), "named fenced blocks should be eligible for workspace file actions");
assert(codeArtifacts[0].content.includes("export function greeting"), "code artifact should keep block content");

const jsonArtifacts = extractArtifactsFromContent('{"name":"milim","features":["chat","artifacts"]}');
equal(jsonArtifacts.length, 1, "standalone JSON should create one artifact");
equal(jsonArtifacts[0].kind, "json", "standalone JSON artifact should be json");
equal(jsonArtifacts[0].filename, "response.json", "standalone JSON should use response.json");
equal(jsonArtifacts[0].disposition, "inline", "standalone JSON should stay display/export-only");

const tableArtifacts = extractArtifactsFromContent([
  "| Name | Status |",
  "| --- | --- |",
  "| Artifacts | Ready |",
].join("\n"));
equal(tableArtifacts.length, 0, "markdown tables should stay inline instead of creating table.csv file artifacts");
equal(markdownTableToCsv(["| Name | Status |", "| --- | --- |", "| Artifacts | Ready |"]), "Name,Status\nArtifacts,Ready", "markdown tables should still be convertible for explicit export flows");

const pipeProseAfterTable = extractArtifactsFromContent([
  "| JS piece | SwiftUI equivalent |",
  "| --- | --- |",
  "| Math.imul | UInt32 overflow ops |",
  "semantics using UInt32 overflow arithmetic || generators returning {pts,c,w} || pure Swift functions",
].join("\n"));
equal(pipeProseAfterTable.length, 0, "markdown tables followed by pipe-heavy prose should stay inline");

const partialStreamingTable = extractArtifactsFromContent([
  "| JS piece | SwiftUI equivalent |",
  "| --- | --- |",
  "| `mul",
].join("\n"));
equal(partialStreamingTable.length, 0, "partial streamed table rows should stay inline until complete");

const shortSnippet = extractArtifactsFromContent("```ts\nconst x = 1;\n```");
equal(shortSnippet.length, 0, "short anonymous code snippets should stay inline only");
const manyAnonymousBlocks = extractArtifactsFromContent(Array.from({ length: 15 }, (_, index) => [
  "```ts",
  `export const anonymous${index} = "${"anonymous preview ".repeat(30)}";`,
  "```",
].join("\n")).join("\n"));
equal(manyAnonymousBlocks.length, 13, "anonymous inline artifacts should stay capped with a notice");
assert(manyAnonymousBlocks[12].content.includes("hid 3 extra inline artifact"), "inline cap should explain omitted anonymous artifacts");
equal(defaultArtifactTargetPath({ filename: "src/generated.ts", kind: "code" }), "src/generated.ts", "named artifacts should default to their filename when saving");
equal(defaultArtifactTargetPath({ kind: "code" }), "", "anonymous artifact display titles should not become workspace file paths");
equal(defaultArtifactTargetPath({ filename: "response.json", kind: "json", disposition: "inline" }), "", "inline data exports should not become workspace file paths");
equal(artifactDisposition({ filename: "src/generated.ts", kind: "code" }), "file", "persisted named artifacts without disposition should default to file");
equal(artifactDisposition({ kind: "table" }), "inline", "persisted table artifacts without disposition should default to inline");

const collapsedNamedFence = extractArtifactsFromContent("Echo: ```ts file=src/e2e-artifact.ts export const e2eArtifact = true; ```");
equal(collapsedNamedFence.length, 1, "single-line named fences should create an artifact");
equal(collapsedNamedFence[0].filename, "src/e2e-artifact.ts", "single-line named fence should keep the filename");
assert(collapsedNamedFence[0].content.includes("e2eArtifact"), "single-line named fence should keep body content");

const proseLabeledFiles = extractArtifactsFromContent([
  "package.json",
  "```json",
  '{"scripts":{"dev":"vite"}}',
  "```",
  "",
  "src/App.tsx",
  "```tsx",
  "export default function App() { return <main>Preview</main>; }",
  "```",
].join("\n"));
equal(proseLabeledFiles.length, 2, "filename lines before fences should create file artifacts");
equal(proseLabeledFiles[0].filename, "package.json", "prose-labeled package.json should be captured");
equal(proseLabeledFiles[0].disposition, "file", "prose-labeled package.json should be a file artifact");
equal(proseLabeledFiles[1].filename, "src/App.tsx", "prose-labeled TSX should keep the path");

const inlineMetadataFile = extractArtifactsFromContent([
  "```tsx",
  "file=src/main.tsx",
  "import App from './App';",
  "console.log(App);",
  "```",
].join("\n"));
equal(inlineMetadataFile.length, 1, "file= metadata inside a fence should create a file artifact");
equal(inlineMetadataFile[0].filename, "src/main.tsx", "inline file= metadata should become the artifact path");
assert(!inlineMetadataFile[0].content.includes("file=src/main.tsx"), "inline file= metadata should not be executed as source");

const runtimeCssArtifacts = extractArtifactsFromContent([
  "package.json",
  "```json",
  '{"scripts":{"dev":"vite"},"dependencies":{"@vitejs/plugin-react":"latest","vite":"latest","typescript":"latest","react":"latest","react-dom":"latest","tailwindcss":"latest","postcss":"latest","autoprefixer":"latest"}}',
  "```",
  "",
  "src/main.tsx",
  "```tsx",
  "import './index.css';",
  "import App from './App';",
  "console.log(App);",
  "```",
  "",
  "src/App.tsx",
  "```tsx",
  "export default function App() { return <main className=\"bg-slate-950 text-white\">Preview</main>; }",
  "```",
  "",
  "```css",
  ["@tailwind base;", "@tailwind components;", "@tailwind utilities;", ".glass { @apply bg-white/5; }", "/* filler */"].join("\n") + "x".repeat(420),
  "```",
].join("\n"));
const runtimeCssFiles = previewRuntimeFiles(runtimeCssArtifacts);
assert(hasPreviewPackageJson(runtimeCssFiles), "runtime files should keep package.json");
assert(runtimeCssFiles.some((file) => file.path === "src/index.css" && file.content.includes("@tailwind")), "runtime staging should map the anonymous CSS block to the imported CSS path");
const runtimeContextMessages = threadArtifactInstructionMessages("", [
  { role: "assistant", content: "", artifacts: runtimeCssArtifacts },
  { role: "user", content: "you can read src/index.css no?" },
], "you can read src/index.css no?");
equal(runtimeContextMessages.length, 2, "runtime troubleshooting turns should include staged preview files");
equal(runtimeContextMessages[1].role, "system", "runtime preview context should be a system message");
assert(runtimeContextMessages[1].content.includes("read-only context"), "runtime preview context should describe its scope");
assert(runtimeContextMessages[1].content.includes("src/index.css"), "runtime preview context should include staged CSS path");
assert(runtimeContextMessages[1].content.includes("@tailwind"), "runtime preview context should include staged CSS content");
equal(previewRuntimeBrowserUrl({ status: "installing", url: "http://127.0.0.1:5173/" }), null, "runtime browser URL should wait for running status");
equal(previewRuntimeBrowserUrl({ status: "running", url: " http://127.0.0.1:5173/ " }), "http://127.0.0.1:5173/", "runtime browser URL should open once running");

const liveHtml = extractLivePreviewArtifactFromContent([
  "Here is the preview:",
  "",
  "```html",
  "<!DOCTYPE html>",
  "<html>",
  "<body><button>Toggle</button></body>",
].join("\n"));
assert(liveHtml, "open html fence should create a live preview artifact before the response finishes");
equal(liveHtml.language, "html", "live preview should keep the html language");
equal(liveHtml.mime, "text/html", "live preview should be rendered as html");
equal(liveHtml.disposition, "preview", "live preview artifacts should be transient previews");
assert(liveHtml.content.includes("<button>Toggle</button>"), "live preview should keep streamed html content");
assert(isPreviewableArtifact(liveHtml), "live html artifact should be previewable");

const finalHtml = extractArtifactsFromContent([
  "```html",
  "<!DOCTYPE html>",
  "<html>",
  "<body><button>Toggle</button></body>",
  `<p>${"x".repeat(420)}</p>`,
  "</html>",
  "```",
].join("\n"))[0];
assert(finalHtml, "long anonymous html should create a finished artifact");
equal(
  artifactPreviewAutoOpenKey(liveHtml),
  artifactPreviewAutoOpenKey(finalHtml),
  "live and final anonymous html previews should share an auto-open dismissal key",
);

const jsOnly = extractArtifactsFromContent([
  "```js",
  "console.log('not a standalone html document');",
  "```",
].join("\n"));
equal(jsOnly.length, 0, "short non-preview snippets should stay inline");

const generatedHtml = extractArtifactsFromRunTrace({
  model: "test",
  startedAt: 1,
  endedAt: 2,
  status: "done",
  steps: [{
    name: "write_file",
    arguments: JSON.stringify({ path: "tarot.html", content: "<!DOCTYPE html><html><body>Tarot</body></html>" }),
    result: { written: 47 },
    startedAt: 1,
    endedAt: 2,
  }],
});
equal(generatedHtml.length, 1, "completed write_file html calls should create an artifact");
equal(generatedHtml[0].filename, "tarot.html", "write_file artifact should keep the target path");
equal(generatedHtml[0].disposition, "file", "write_file artifacts should be file artifacts");
equal(generatedHtml[0].mime, "text/html", "write_file html artifact should use html mime");
assert(isPreviewableArtifact(generatedHtml[0]), "write_file html artifact should be previewable");

const generatedSavedHtml = extractArtifactsFromRunTrace({
  model: "test",
  startedAt: 1,
  endedAt: 2,
  status: "done",
  workspace: "C:\\workspace",
  sourceSessionId: "session-test",
  steps: [{
    name: "write_file",
    arguments: JSON.stringify({ path: "web/tarot.html", content: "<!DOCTYPE html><html><body>Tarot</body></html>" }),
    result: { written: 47 },
    startedAt: 1,
    endedAt: 2,
  }],
});
equal(generatedSavedHtml[0].saved?.path, "C:\\workspace\\web\\tarot.html", "write_file artifacts should point at the created workspace file");
equal(generatedSavedHtml[0].saved?.source, "tool_write", "write_file artifacts should be marked as direct tool writes");
equal(generatedSavedHtml[0].saved?.sourceSessionId, "session-test", "write_file saved metadata should keep the source session");

const pendingWrite = extractArtifactsFromRunTrace({
  model: "test",
  startedAt: 1,
  status: "running",
  steps: [{
    name: "write_file",
    arguments: JSON.stringify({ path: "draft.html", content: "<html></html>" }),
    startedAt: 1,
  }],
});
equal(pendingWrite.length, 0, "pending write_file calls should not create generated artifacts yet");

const failedWrite = extractArtifactsFromRunTrace({
  model: "test",
  startedAt: 1,
  endedAt: 2,
  status: "done",
  steps: [{
    name: "write_file",
    arguments: JSON.stringify({ path: "failed.html", content: "<!DOCTYPE html><html><body>Failed</body></html>" }),
    result: { error: "no working folder selected - pick one with the Folder chip first" },
    startedAt: 1,
    endedAt: 2,
  }],
});
equal(failedWrite.length, 0, "failed write_file calls should not create generated artifacts");

const generatedMarkdown = extractArtifactsFromRunTrace({
  model: "test",
  startedAt: 1,
  endedAt: 2,
  status: "done",
  steps: [{
    name: "write_file",
    arguments: JSON.stringify({ path: "notes/reading.md", content: "# Reading\n\n- Card" }),
    result: { written: 17 },
    startedAt: 1,
    endedAt: 2,
  }],
});
equal(generatedMarkdown.length, 1, "completed write_file markdown calls should create an artifact");
equal(generatedMarkdown[0].mime, "text/markdown", "write_file markdown artifact should use markdown mime");
assert(isPreviewableArtifact(generatedMarkdown[0]), "write_file markdown artifact should be previewable");

const localhostPreviewUrl = extractLocalhostUrlFromRunTrace({
  model: "test",
  startedAt: 1,
  endedAt: 2,
  status: "done",
  steps: [{
    name: "shell",
    arguments: JSON.stringify({ command: "pnpm dev" }),
    result: { stdout: "Local: http://localhost:5173/\nNetwork: http://192.168.1.12:5173/" },
    startedAt: 1,
    endedAt: 2,
  }],
});
equal(localhostPreviewUrl, "http://localhost:5173/", "shell localhost URLs should be preview candidates");
assert(isLocalhostPreviewUrl("http://127.0.0.1:4173"), "127.0.0.1 URLs should be valid preview URLs");
assert(!isLocalhostPreviewUrl("https://example.com"), "external URLs should not auto-open in the artifact panel");
equal(normalizeArtifactBrowserUrl("localhost:5173"), "http://localhost:5173/", "bare localhost should normalize to local http");
equal(normalizeArtifactBrowserUrl("127.0.0.1:4173"), "http://127.0.0.1:4173/", "bare 127.0.0.1 should normalize to local http");
equal(normalizeArtifactBrowserUrl("[::1]:3000"), "http://[::1]:3000/", "bare IPv6 loopback should normalize to local http");
equal(normalizeArtifactBrowserUrl("example.com/path"), "https://example.com/path", "bare public domains should normalize to https");
equal(normalizeArtifactBrowserUrl("https://example.com/path"), "https://example.com/path", "public https should stay valid");
assert(isArtifactBrowserUrl("https://example.com"), "public https should be valid in the manual artifact browser");
equal(normalizeArtifactBrowserUrl("http://example.com"), null, "public http should be rejected");
equal(normalizeArtifactBrowserUrl("file:///tmp/a.html"), null, "file URLs should be rejected");
equal(normalizeArtifactBrowserUrl("javascript:alert(1)"), null, "javascript URLs should be rejected");
equal(normalizeArtifactBrowserUrl(""), null, "blank URLs should be rejected");
equal(resolveArtifactBrowserInput("example.com/path"), "https://example.com/path", "address-like input should remain a URL");
equal(resolveArtifactBrowserInput("google sheets tips"), "https://www.google.com/search?q=google%20sheets%20tips", "plain words should become a Google search");
equal(resolveArtifactBrowserInput("javascript:alert(1)"), null, "invalid schemes should not become searches");

const multiFileArtifacts = extractArtifactsFromContent([
  "```html file=index.html",
  '<!doctype html><html><head><link rel="stylesheet" href="./styles.css"></head><body><div id="app"></div><script type="module" src="./src/main.js"></script></body></html>',
  "```",
  "```css file=styles.css",
  "#app { color: red; }",
  "```",
  "```js file=src/main.js",
  'import { label } from "./label.js";',
  'document.getElementById("app").textContent = label;',
  "```",
  "```js file=src/label.js",
  'export const label = "Loaded";',
  "```",
].join("\n"));
equal(multiFileArtifacts.length, 4, "multi-file named fences should create one artifact per file");
const multiFileIndex = multiFileArtifacts.find((artifact) => artifact.filename === "index.html");
assert(multiFileIndex, "multi-file artifact set should include index.html");
const multiFilePreview = await buildArtifactPreviewDocument(multiFileIndex, multiFileArtifacts);
assert(multiFilePreview.source.includes('data-artifact-file="styles.css"'), "html preview should inline sibling CSS artifacts");
assert(!multiFilePreview.source.includes('href="./styles.css"'), "html preview should remove sibling CSS hrefs");
assert(multiFilePreview.source.includes("data-milim-artifact-scroll"), "html preview should inject the scroll guard");
assert(multiFilePreview.source.includes("overflow: auto !important"), "html preview should allow iframe document scrolling");
assert(
  multiFilePreview.source.indexOf('data-artifact-file="styles.css"') < multiFilePreview.source.indexOf("data-milim-artifact-scroll"),
  "artifact scroll guard should override generated page-level overflow styles",
);
assert(!multiFilePreview.source.includes('src="./src/main.js"'), "html preview should remove sibling script srcs");
assert(multiFilePreview.source.includes("document.getElementById"), "html preview should inline sibling script entrypoints");
assert(multiFilePreview.source.includes("data:text/javascript"), "html preview should rewrite relative JS imports to data URL modules");
assert(multiFilePreview.source.includes("data-milim-artifact-log-bridge"), "html preview should inject the artifact log bridge");
assert(
  multiFilePreview.source.indexOf("data-milim-artifact-log-bridge") < multiFilePreview.source.indexOf("document.getElementById"),
  "artifact log bridge should run before generated scripts",
);

const generatedMultiFile = extractArtifactsFromRunTrace({
  model: "test",
  startedAt: 1,
  endedAt: 2,
  status: "done",
  steps: [
    {
      name: "write_file",
      arguments: JSON.stringify({ path: "styles.css", content: "body { color: red; }" }),
      result: { written: 20 },
      startedAt: 1,
      endedAt: 2,
    },
    {
      name: "write_file",
      arguments: JSON.stringify({ path: "src/main.js", content: "document.body.dataset.ready = 'true';" }),
      result: { written: 35 },
      startedAt: 1,
      endedAt: 2,
    },
    {
      name: "write_file",
      arguments: JSON.stringify({ path: "index.html", content: '<link rel="stylesheet" href="./styles.css"><script type="module" src="./src/main.js"></script>' }),
      result: { written: 92 },
      startedAt: 1,
      endedAt: 2,
    },
  ],
});
equal(generatedMultiFile.length, 3, "write_file runs should keep sibling source files for multi-file previews");
assert(generatedMultiFile.some((artifact) => artifact.filename === "styles.css"), "write_file CSS should stay available as preview context");
assert(generatedMultiFile.some((artifact) => artifact.filename === "src/main.js"), "write_file JS should stay available as preview context");
assert(generatedMultiFile.some((artifact) => artifact.filename === "index.html"), "write_file HTML should stay available as preview entry");

const generatedViteDashboardFiles = [
  "package.json",
  "index.html",
  "vite.config.ts",
  "tsconfig.json",
  "postcss.config.js",
  "tailwind.config.js",
  "src/main.tsx",
  "src/index.css",
  "src/App.tsx",
  "src/components/ParticleField.tsx",
  "src/components/Sidebar.tsx",
  "src/components/StatCard.tsx",
  "src/components/Charts.tsx",
  "src/components/DonutWidget.tsx",
  "src/components/ActivityFeed.tsx",
];
const generatedViteDashboard = extractArtifactsFromContent(generatedViteDashboardFiles.flatMap((file) => [
  "```" + extensionForTestFile(file) + " file=" + file,
  sourceForTestFile(file),
  "```",
]).join("\n"));
equal(generatedViteDashboard.length, generatedViteDashboardFiles.length, "generated Vite apps should keep all named files");
assert(generatedViteDashboard.some((artifact) => artifact.filename === "src/components/Charts.tsx"), "component files after the old artifact cap should be captured");
assert(generatedViteDashboard.some((artifact) => artifact.filename === "src/components/DonutWidget.tsx"), "later imported component files should be captured");
assert(generatedViteDashboard.some((artifact) => artifact.filename === "src/components/ActivityFeed.tsx"), "last imported component file should be captured");

const manyNamedFiles = extractArtifactsFromContent(Array.from({ length: 40 }, (_, index) => [
  "```ts file=src/generated/file-" + index + ".ts",
  "export const generated" + index + " = true;",
  "```",
]).flat().join("\n"));
equal(manyNamedFiles.length, 40, "named file artifacts should not be capped by count");
assert(manyNamedFiles.every(isFileArtifact), "many named artifacts should remain file artifacts");

const overBudgetNamedFiles = extractArtifactsFromContent([
  "```ts file=src/small.ts",
  "export const small = true;",
  "```",
  "```txt file=src/huge.txt",
  "x".repeat(8 * 1024 * 1024),
  "```",
  "```ts",
  `export const inline = "${"inline artifact ".repeat(40)}";`,
  "```",
].join("\n"));
equal(overBudgetNamedFiles.length, 2, "over-budget named sets should keep the notice and inline artifacts only");
assert(overBudgetNamedFiles.every((artifact) => !isFileArtifact(artifact)), "over-budget named artifacts should all be hidden");
assert(overBudgetNamedFiles[0].content.includes("hid all 2 named file artifact"), "over-budget notice should describe the full hidden named set");
assert(overBudgetNamedFiles[1].content.includes("inline artifact"), "inline artifacts should survive named file budget overflow");

const revisionOne = extractArtifactsFromContent([
  "```ts file=src/revised.ts",
  "export const revised = 1;",
  "```",
].join("\n"));
const revisionTwo = extractArtifactsFromContent([
  "```ts file=src/revised.ts",
  "export const revised = 2;",
  "```",
].join("\n"));
const revisionMessages: ChatMessage[] = [
  { role: "assistant", content: "", artifacts: revisionOne },
  { role: "assistant", content: "", artifacts: revisionTwo },
];
const revisionGroups = artifactRevisionGroups(revisionMessages);
const revisedGroup = revisionGroups.find((group) => group.label === "src/revised.ts");
assert(revisedGroup, "same artifact path should create a revision group");
equal(revisedGroup.revisions.length, 2, "same artifact path should keep both revisions");
equal(revisedGroup.latest.revisionNumber, 2, "latest revision should be the last occurrence");
assert(revisedGroup.latest.artifact.content.includes("revised = 2"), "latest revision should keep newest content");
const revisionChoice = artifactRevisionChoiceByOccurrence(revisionGroups).get(artifactOccurrenceKey(0, 0));
equal(revisionChoice?.revision.revisionNumber, 1, "occurrence index should point to the original revision");

const anonymousRevisionMessages: ChatMessage[] = [
  { role: "assistant", content: "", artifacts: extractArtifactsFromContent(`\`\`\`ts\n${"const anonymousA = 1;\n".repeat(24)}\`\`\``) },
  { role: "assistant", content: "", artifacts: extractArtifactsFromContent(`\`\`\`ts\n${"const anonymousB = 2;\n".repeat(24)}\`\`\``) },
];
const anonymousGroups = artifactRevisionGroups(anonymousRevisionMessages);
equal(anonymousGroups.length, 2, "generic unnamed artifacts should stay distinct");
assert(anonymousGroups.every((group) => group.revisions.length === 1), "generic unnamed artifacts should not become revisions");

const multiFileArtifactsV2 = extractArtifactsFromContent([
  "```html file=index.html",
  '<!doctype html><html><head><link rel="stylesheet" href="./styles.css"></head><body><div id="app"></div><script type="module" src="./src/main.js"></script></body></html>',
  "```",
  "```css file=styles.css",
  "#app { color: blue; }",
  "```",
  "```js file=src/main.js",
  "document.getElementById('app').textContent = 'v2';",
  "```",
].join("\n"));
const multiRevisionGroups = artifactRevisionGroups([
  { role: "assistant", content: "", artifacts: multiFileArtifacts },
  { role: "assistant", content: "", artifacts: multiFileArtifactsV2 },
]);
const indexRevisionGroup = multiRevisionGroups.find((group) => group.label === "index.html");
assert(indexRevisionGroup, "multi-file artifact entry should have revisions");
equal(indexRevisionGroup.latest.totalRevisions, 2, "multi-file entry should count both revisions");
assert(
  indexRevisionGroup.latest.artifacts.some((artifact) => artifact.filename === "styles.css" && artifact.content.includes("blue")),
  "latest revision should carry sibling artifacts from the same message",
);

const reactArtifacts = extractArtifactsFromContent([
  "```html file=index.html",
  '<div id="root"></div><script type="module" src="./src/main.tsx"></script>',
  "```",
  "```tsx file=src/main.tsx",
  'import { createRoot } from "react-dom/client";',
  'import { App } from "./App";',
  'createRoot(document.getElementById("root")!).render(<App />);',
  "```",
  "```tsx file=src/App.tsx",
  "export function App() {",
  "  return <button>React preview</button>;",
  "}",
  "```",
].join("\n"));
const reactIndex = reactArtifacts.find((artifact) => artifact.filename === "index.html");
assert(reactIndex, "react artifact set should include index.html");
const reactPreview = await buildArtifactPreviewDocument(reactIndex, reactArtifacts);
assert(!reactPreview.source.includes('src="./src/main.tsx"'), "tsx script references should be inlined");
assert(reactPreview.source.includes("https://esm.sh/react-dom@18.3.1/client"), "bare React DOM imports should resolve to browser ESM");
assert(reactPreview.source.includes("https://esm.sh/react@18.3.1/jsx-runtime"), "TSX should compile to the React JSX runtime import");
assert(reactPreview.source.includes("connect-src https: wss:"), "artifact previews should allow internet fetches inside the preview frame");
assert(reactPreview.source.includes("script-src 'unsafe-inline' https: data: blob:"), "artifact previews should allow remote ESM and generated module data URLs");
assert(
  reactPreview.source.indexOf("data-milim-artifact-log-bridge") < reactPreview.source.indexOf("react-dom@18.3.1/client"),
  "artifact log bridge should run before standalone script modules",
);

const reactDependencyArtifacts = extractArtifactsFromContent([
  "```html file=index.html",
  '<div id="root"></div><script type="module" src="./src/main.tsx"></script>',
  "```",
  "```tsx file=src/main.tsx",
  'import { motion } from "framer-motion";',
  'import { Activity } from "lucide-react";',
  'import { AreaChart } from "recharts";',
  "export default function Dashboard() {",
  "  return <motion.main><Activity /><AreaChart data={[]} /></motion.main>;",
  "}",
  "```",
].join("\n"));
const reactDependencyIndex = reactDependencyArtifacts.find((artifact) => artifact.filename === "index.html");
assert(reactDependencyIndex, "react dependency artifact set should include index.html");
const reactDependencyPreview = await buildArtifactPreviewDocument(reactDependencyIndex, reactDependencyArtifacts);
assert(
  reactDependencyPreview.source.includes("https://esm.sh/framer-motion?deps=react@18.3.1,react-dom@18.3.1"),
  "React ecosystem bare imports should pin esm.sh React peers",
);
assert(
  reactDependencyPreview.source.includes("https://esm.sh/lucide-react?deps=react@18.3.1,react-dom@18.3.1"),
  "React icon imports should pin esm.sh React peers",
);
assert(
  reactDependencyPreview.source.includes("https://esm.sh/recharts?deps=react@18.3.1,react-dom@18.3.1"),
  "React chart imports should pin esm.sh React peers",
);

const anonymousTsxArtifacts = extractArtifactsFromContent([
  "```tsx",
  `// ${"standalone anonymous TSX preview ".repeat(16)}`,
  'const previewNotes = "anonymous standalone TSX preview ".repeat(20);',
  "export default function Dashboard() {",
  '  return <main data-notes={previewNotes}><h1>Anonymous TSX</h1></main>;',
  "}",
  "```",
].join("\n"));
equal(anonymousTsxArtifacts.length, 1, "long anonymous TSX fences should create an artifact");
equal(anonymousTsxArtifacts[0].disposition, "inline", "long anonymous TSX fences should stay inline/display-only");
assert(!isFileArtifact(anonymousTsxArtifacts[0]), "long anonymous TSX fences should not be workspace file artifacts");
assert(isPreviewableArtifact(anonymousTsxArtifacts[0]), "anonymous TSX artifacts should be previewable");
const anonymousTsxPreview = await buildArtifactPreviewDocument(anonymousTsxArtifacts[0], anonymousTsxArtifacts);
assert(anonymousTsxPreview.source.includes('await import("data:text/javascript'), "anonymous TSX should run as a compiled module");
assert(anonymousTsxPreview.source.includes("React.createElement(previewModule.default)"), "anonymous default TSX components should auto-mount");
assert(anonymousTsxPreview.source.includes("https://esm.sh/react-dom@18.3.1/client"), "anonymous TSX auto-mount should use browser React DOM");
assert(!anonymousTsxPreview.source.includes("export default function Dashboard()"), "anonymous TSX should not render raw source as HTML");

const webglArtifacts = extractArtifactsFromContent([
  "```html file=index.html",
  '<canvas id="scene"></canvas><script type="module" src="./scene.js"></script>',
  "```",
  "```js file=scene.js",
  'import * as THREE from "three";',
  'import { Renderer } from "ogl";',
  "window.previewImports = [THREE, Renderer];",
  "```",
].join("\n"));
const webglIndex = webglArtifacts.find((artifact) => artifact.filename === "index.html");
assert(webglIndex, "webgl artifact set should include index.html");
const webglPreview = await buildArtifactPreviewDocument(webglIndex, webglArtifacts);
assert(webglPreview.source.includes("https://esm.sh/three"), "Three imports should resolve to browser ESM");
assert(webglPreview.source.includes("https://esm.sh/ogl"), "OGL imports should resolve to browser ESM");

const earthPrompt = [
  "build a fully interactive 3D digital twin of Earth that has the following features",
  "allow users to zoom seamlessly from outer space down to individual city streets",
  "when I hover over a city the country's outline should be highlighted and there should be a pop-up displaying stats like area population GDP",
  "show a realistic planet Earth with toggles for atmospheric cloud cover flight traffic day and night and then night mode to show city streets",
  "make sure it loads efficiently on a regular web browser",
].join(" ");
const regularNoFolderTurn = [
  ...threadArtifactInstructionMessages(""),
  { role: "user", content: earthPrompt },
];
equal(regularNoFolderTurn.length, 2, "regular no-folder artifact requests should receive one system instruction");
equal(regularNoFolderTurn[0].role, "system", "no-folder artifact instruction should be a system message");
assert(regularNoFolderTurn[0].content.includes("current chat's artifact panel"), "no-folder instruction should route generated files to chat artifacts");
assert(regularNoFolderTurn[0].content.includes("```html file=index.html"), "no-folder instruction should teach named fenced HTML artifacts");
assert(regularNoFolderTurn[0].content.includes("package.json"), "no-folder instruction should require named package.json for runnable apps");
assert(regularNoFolderTurn[0].content.includes("anonymous tsx blocks"), "no-folder instruction should reject anonymous TSX app output");
assert(regularNoFolderTurn[0].content.includes("Markdown tables should stay as markdown tables"), "no-folder instruction should keep markdown tables inline by default");
assert(regularNoFolderTurn[0].content.includes("instead of asking for a folder"), "no-folder instruction should avoid blocking on folder selection for new artifacts");
equal(regularNoFolderTurn[1].content, earthPrompt, "regression prompt should remain the user turn");
equal(threadArtifactInstructionMessages("C:\\project").length, 0, "project chats should not receive no-folder artifact instructions");

equal(planModeInstructionMessages(false).length, 0, "inactive plan mode should not inject instructions");
const planInstruction = planModeInstructionMessages(true)[0];
equal(planInstruction.role, "system", "plan mode should inject a system instruction");
assert(planInstruction.content.includes("read-only inspection"), "plan mode should require read-only inspection");
assert(planInstruction.content.includes("Do not implement"), "plan mode should defer execution");

const skillMessage = skillInstructionMessage([
  {
    id: "skill-1",
    name: "code-review",
    description: "Use when reviewing diffs",
    instructions: "List findings first.",
    enabled: true,
    source_kind: "manual",
  },
  {
    id: "skill-2",
    name: "disabled",
    description: "",
    instructions: "Do not include me.",
    enabled: false,
    source_kind: "manual",
  },
]);
assert(skillMessage, "enabled skills should create a system message");
equal(skillMessage.role, "system", "skill instructions should be injected as system context");
assert(skillMessage.content.includes("code-review"), "skill name should be included");
assert(skillMessage.content.includes("List findings first."), "skill instructions should be included");
assert(!skillMessage.content.includes("Do not include me."), "disabled skills should not be included");
equal(skillInstructionMessage([]), null, "empty skill selection should not create a system message");

export {};
