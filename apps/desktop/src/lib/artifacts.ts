import type { ChatArtifact, ChatArtifactDisposition, ChatArtifactKind, PreviewAppFile, RunStep, RunTrace, SavedArtifactFile } from "../api";

const MAX_INLINE_ARTIFACTS = 12;
const MAX_NAMED_ARTIFACT_BYTES = 8 * 1024 * 1024;
const MIN_ANON_CODE_CHARS = 400;
const FENCE_RE = /```([^\n`]*)\n([\s\S]*?)```/g;
const ANY_FENCE_RE = /```([\s\S]*?)```/g;
const OPEN_FENCE_RE = /```([^\n`]*)\n/g;
const LOCALHOST_URL_RE = /\bhttps?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])(?::\d{1,5})?(?:[/?#][^\s"'<>]*)?/gi;
const WRITE_FILE_TOOL = "write_file";

const CODE_EXTENSIONS = new Set([
  "c",
  "cpp",
  "cs",
  "css",
  "go",
  "h",
  "hpp",
  "html",
  "java",
  "js",
  "jsx",
  "kt",
  "md",
  "mjs",
  "py",
  "rb",
  "rs",
  "sh",
  "sql",
  "swift",
  "toml",
  "ts",
  "tsx",
  "xml",
  "yaml",
  "yml",
]);

const PREVIEWABLE_SCRIPT_EXTENSIONS = new Set(["js", "jsx", "mjs", "ts", "tsx"]);

interface FenceInfo {
  language?: string;
  filename?: string;
  title?: string;
}

export function extractArtifactsFromContent(content: string): ChatArtifact[] {
  const trimmed = content.trim();
  if (!trimmed) return [];

  const artifacts: ChatArtifact[] = [];
  collectFencedArtifacts(content, artifacts);
  collectCollapsedNamedFencedArtifacts(content, artifacts);
  if (artifacts.length === 0) {
    collectStandaloneJson(trimmed, artifacts);
  }
  if (artifacts.length === 0) {
    collectCsvArtifact(trimmed, artifacts);
  }
  return applyArtifactLimits(artifacts);
}

export function extractArtifactsFromMessage(content: string, run?: RunTrace): ChatArtifact[] {
  const artifacts = extractArtifactsFromContent(content);
  for (const artifact of extractArtifactsFromRunTrace(run, artifacts.length)) {
    if (artifacts.some((item) => artifactIdentity(item) === artifactIdentity(artifact))) continue;
    artifacts.push(artifact);
  }
  return applyArtifactLimits(artifacts);
}

export function extractArtifactsFromRunTrace(run: RunTrace | undefined, startIndex = 0): ChatArtifact[] {
  if (!run?.steps.length) return [];
  const artifacts: ChatArtifact[] = [];
  for (const step of run.steps) {
    if (step.name !== WRITE_FILE_TOOL || step.error || step.result === undefined || toolResultError(step.result)) continue;
    const written = parseWriteFileArguments(step.arguments);
    if (!written) continue;
    const language = extensionOf(written.path);
    if (!isGeneratedArtifactSource(written.path, language)) continue;
    const artifact = makeArtifact(startIndex + artifacts.length, {
      content: written.content,
      filename: written.path,
      language,
      title: written.path,
      kind: kindForSource(written.path, language, "code"),
      disposition: "file",
    });
    const saved = savedFileForToolWrite(run, step, written.path, artifact.size);
    artifacts.push(saved ? { ...artifact, saved } : artifact);
  }
  return applyArtifactLimits(artifacts);
}

export function extractLocalhostUrlFromRunTrace(run: RunTrace | undefined): string | null {
  if (!run?.steps.length) return null;
  for (let i = run.steps.length - 1; i >= 0; i--) {
    const step = run.steps[i];
    if (step.endedAt == null || step.error || (step.name !== "shell" && step.name !== "run_command")) continue;
    const url = extractLocalhostUrl(toolResultText(step.result));
    if (url) return url;
  }
  return null;
}

export function extractLocalhostUrl(text: string): string | null {
  LOCALHOST_URL_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = LOCALHOST_URL_RE.exec(text))) {
    const url = normalizeLocalhostUrl(match[0]);
    if (url) return url;
  }
  return null;
}

export function isLocalhostPreviewUrl(value: string): boolean {
  return normalizeLocalhostUrl(value) !== null;
}

export function normalizeArtifactBrowserUrl(input: string): string | null {
  const text = input.trim();
  if (!text) return null;
  const candidate = artifactBrowserUrlCandidate(text);
  if (!candidate) return null;
  try {
    const url = new URL(candidate);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (url.protocol === "http:" && !isLoopbackHost(url.hostname)) return null;
    return url.toString();
  } catch {
    return null;
  }
}

export function resolveArtifactBrowserInput(input: string): string | null {
  const text = input.trim();
  if (!text) return null;
  if (/^(?:https?:\/\/|localhost(?::|\/|[?#]|$)|127\.0\.0\.1(?::|\/|[?#]|$)|\[::1\](?::|\/|[?#]|$)|[^\s/]+\.[^\s]+)/i.test(text)) {
    return normalizeArtifactBrowserUrl(text);
  }
  if (/^[a-z][a-z0-9+.-]*:/i.test(text)) return null;
  return `https://www.google.com/search?q=${encodeURIComponent(text)}`;
}

export function isArtifactBrowserUrl(value: string): boolean {
  return normalizeArtifactBrowserUrl(value) !== null;
}

function toolResultText(result: unknown): string {
  if (typeof result === "string") return result;
  if (!result || typeof result !== "object" || Array.isArray(result)) return "";
  const record = result as Record<string, unknown>;
  return ["stdout", "stderr", "output", "message"]
    .map((key) => record[key])
    .filter((value): value is string => typeof value === "string")
    .join("\n");
}

function normalizeLocalhostUrl(raw: string): string | null {
  const text = raw.trim().replace(/[),.;]+$/g, "");
  try {
    const url = new URL(text);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (!isLoopbackHost(url.hostname)) return null;
    return url.toString();
  } catch {
    return null;
  }
}

function artifactBrowserUrlCandidate(text: string): string | null {
  if (/^(?:localhost|127\.0\.0\.1|\[::1\])(?::|\/|[?#]|$)/i.test(text)) return `http://${text}`;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(text)) return text;
  if (/^[a-z][a-z0-9+.-]*:/i.test(text)) return null;
  return `https://${text}`;
}

function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase();
  return host === "localhost" || host === "127.0.0.1" || host === "::1" || host === "[::1]";
}

function toolResultError(result: unknown): string | undefined {
  if (!result || typeof result !== "object" || Array.isArray(result)) return undefined;
  const error = (result as { error?: unknown }).error;
  return typeof error === "string" && error.trim() ? error.trim() : undefined;
}

function savedFileForToolWrite(run: RunTrace, step: RunStep, path: string, bytes: number): SavedArtifactFile | null {
  const workspacePath = workspaceArtifactPath(run.workspace, path);
  if (!workspacePath) return null;
  return {
    path: workspacePath,
    bytes: toolResultBytes(step.result) ?? bytes,
    overwritten: false,
    savedAt: step.endedAt,
    sourceSessionId: run.sourceSessionId,
    source: "tool_write",
  };
}

function toolResultBytes(result: unknown): number | undefined {
  if (!result || typeof result !== "object" || Array.isArray(result)) return undefined;
  const record = result as { written?: unknown; bytes?: unknown };
  if (typeof record.written === "number" && Number.isFinite(record.written)) return record.written;
  if (typeof record.bytes === "number" && Number.isFinite(record.bytes)) return record.bytes;
  return undefined;
}

function workspaceArtifactPath(workspace: string | undefined, path: string): string | null {
  const root = workspace?.trim().replace(/[\\/]+$/, "");
  if (!root) return null;
  const normalized = path.trim().replace(/\\/g, "/");
  if (!normalized || normalized.startsWith("/") || /^[A-Za-z]:/.test(normalized)) return null;
  const parts = normalized.split("/").filter((part) => part && part !== ".");
  if (!parts.length || parts.some((part) => part === ".." || part.includes(":"))) return null;
  const sep = root.includes("\\") ? "\\" : "/";
  return `${root}${sep}${parts.join(sep)}`;
}

export function extractLivePreviewArtifactFromContent(content: string): ChatArtifact | null {
  let match: RegExpExecArray | null;
  let latest: { info: FenceInfo; content: string; start: number } | null = null;
  OPEN_FENCE_RE.lastIndex = 0;
  while ((match = OPEN_FENCE_RE.exec(content))) {
    const start = match.index;
    const bodyStart = OPEN_FENCE_RE.lastIndex;
    const close = content.indexOf("```", bodyStart);
    const info = parseFenceInfo(match[1]);
    if (isPreviewableSource(info.filename, info.language)) {
      const block = cleanFenceContent(content.slice(bodyStart, close >= 0 ? close : undefined));
      if (block.trim()) latest = { info, content: block, start };
    }
    if (close >= 0) {
      OPEN_FENCE_RE.lastIndex = close + 3;
    }
  }
  if (!latest) return null;
  const language = latest.info.language ?? extensionOf(latest.info.filename ?? "");
  const artifact = makeArtifact(0, {
    content: latest.content,
    filename: latest.info.filename,
    language: language || "html",
    title: latest.info.title ?? latest.info.filename ?? "HTML preview",
    kind: "code",
    disposition: "preview",
  });
  return { ...artifact, id: `live-preview-${latest.start}-${artifact.language ?? "html"}` };
}

export function isPreviewableArtifact(artifact: ChatArtifact): boolean {
  return isPreviewableSource(artifact.filename, artifact.language) || artifact.mime === "text/html" || artifact.mime === "text/markdown";
}

export function artifactPreviewAutoOpenKey(artifact: ChatArtifact): string {
  const source = (artifact.language || extensionOf(artifact.filename ?? artifact.title) || artifact.mime).toLowerCase();
  return `${artifact.filename?.toLowerCase() ?? ""}\0${source}\0${artifact.mime.toLowerCase()}`;
}

export function artifactDisposition(artifact: Pick<ChatArtifact, "disposition" | "filename" | "kind">): ChatArtifactDisposition {
  if (artifact.disposition === "file" || artifact.disposition === "inline" || artifact.disposition === "preview") {
    return artifact.disposition;
  }
  if (artifact.kind === "table") return "inline";
  return artifact.filename ? "file" : "inline";
}

export function isFileArtifact(artifact: Pick<ChatArtifact, "disposition" | "filename" | "kind">): boolean {
  return artifactDisposition(artifact) === "file";
}

export function normalizeArtifactDisposition<T extends ChatArtifact>(artifact: T): T {
  return artifact.disposition ? artifact : { ...artifact, disposition: artifactDisposition(artifact) };
}

export function defaultArtifactTargetPath(artifact: Pick<ChatArtifact, "disposition" | "filename" | "kind">): string {
  return isFileArtifact(artifact) ? artifact.filename ?? "" : "";
}

export function previewRuntimeFiles(artifacts?: readonly ChatArtifact[]): PreviewAppFile[] {
  const seen = new Set<string>();
  const files: PreviewAppFile[] = [];
  for (const artifact of artifacts ?? []) {
    if (!isFileArtifact(artifact) || !artifact.filename?.trim()) continue;
    const path = normalizePreviewRuntimePath(artifact.filename);
    if (!path || seen.has(path)) continue;
    seen.add(path);
    files.push({ path, content: artifact.content });
  }

  const anonymousCss = (artifacts ?? []).filter((artifact) => !isFileArtifact(artifact) && sourceForArtifact(artifact) === "css");
  if (anonymousCss.length === 1) {
    for (const path of importedCssPaths(files)) {
      if (seen.has(path)) continue;
      seen.add(path);
      files.push({ path, content: anonymousCss[0].content });
    }
  }

  return files;
}

export function hasPreviewPackageJson(files: readonly Pick<PreviewAppFile, "path">[]): boolean {
  return files.some((file) => normalizePreviewRuntimePath(file.path).toLowerCase() === "package.json");
}

export function previewRuntimeBrowserUrl(status?: { status?: string | null; url?: string | null } | null): string | null {
  return status?.status === "running" && status.url?.trim() ? status.url.trim() : null;
}

function importedCssPaths(files: readonly PreviewAppFile[]): string[] {
  const paths: string[] = [];
  for (const file of files) {
    if (!PREVIEWABLE_SCRIPT_EXTENSIONS.has(extensionOf(file.path))) continue;
    const dir = file.path.includes("/") ? file.path.slice(0, file.path.lastIndexOf("/") + 1) : "";
    for (const match of file.content.matchAll(/\bimport\s+(?:[^'"]+\s+from\s+)?["'](\.\/[^"']+\.css)["']/g)) {
      const path = normalizePreviewRuntimePath(dir + match[1].replace(/^\.\//, ""));
      if (path) paths.push(path);
    }
  }
  return paths;
}

function normalizePreviewRuntimePath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/^(\.\/)+/, "");
}

function sourceForArtifact(artifact: Pick<ChatArtifact, "filename" | "language" | "title">): string {
  return (artifact.language || extensionOf(artifact.filename ?? artifact.title)).toLowerCase();
}

function collectFencedArtifacts(content: string, artifacts: ChatArtifact[]): void {
  let match: RegExpExecArray | null;
  while ((match = FENCE_RE.exec(content))) {
    const parsedInfo = parseFenceInfo(match[1]);
    const cleanedBlock = cleanFenceContent(match[2]);
    const leadingFile = leadingFileMetadata(cleanedBlock);
    const inferredFilename = parsedInfo.filename ?? leadingFile?.filename ?? inferFenceFilename(content, match.index);
    const info = inferredFilename
      ? {
          ...parsedInfo,
          filename: inferredFilename,
          language: parsedInfo.language ?? extensionOf(inferredFilename),
        }
      : parsedInfo;
    const block = leadingFile ? leadingFile.content : cleanedBlock;
    if (!block.trim()) continue;
    if (!info.filename && info.language === "mermaid") continue;
    if (!info.filename && block.trim().length < MIN_ANON_CODE_CHARS) continue;
    artifacts.push(makeArtifact(artifacts.length, {
      content: block,
      filename: info.filename,
      language: info.language,
      title: info.title ?? info.filename ?? `Code block ${artifacts.length + 1}`,
      kind: kindForSource(info.filename, info.language, "code"),
      disposition: info.filename ? "file" : "inline",
    }));
  }
}

function collectCollapsedNamedFencedArtifacts(content: string, artifacts: ChatArtifact[]): void {
  let match: RegExpExecArray | null;
  while ((match = ANY_FENCE_RE.exec(content))) {
    const parsed = parseCollapsedNamedFence(match[1]);
    if (!parsed) continue;
    artifacts.push(makeArtifact(artifacts.length, {
      content: parsed.content,
      filename: parsed.info.filename,
      language: parsed.info.language,
      title: parsed.info.title ?? parsed.info.filename ?? `Code block ${artifacts.length + 1}`,
      kind: kindForSource(parsed.info.filename, parsed.info.language, "code"),
      disposition: "file",
    }));
  }
}

function parseCollapsedNamedFence(raw: string): { info: FenceInfo; content: string } | null {
  if (raw.includes("\n")) return null;
  const tokens = raw.trim().split(/\s+/).filter(Boolean);
  if (tokens.length < 3) return null;

  let filenameIndex = -1;
  const infoTokens: string[] = [];
  for (let i = 0; i < tokens.length; i++) {
    infoTokens.push(tokens[i]);
    const [key, ...rest] = tokens[i].split("=");
    const value = stripQuotes(rest.join("="));
    if ((key === "file" || key === "filename" || key === "path") && value) {
      filenameIndex = i;
      break;
    }
    if (looksLikeFilename(tokens[i])) {
      filenameIndex = i;
      break;
    }
  }
  if (filenameIndex < 0 || filenameIndex >= tokens.length - 1) return null;

  const info = parseFenceInfo(infoTokens.join(" "));
  if (!info.filename) return null;
  const body = tokens.slice(filenameIndex + 1).join(" ").trim();
  return body ? { info, content: body } : null;
}

function inferFenceFilename(content: string, fenceStart: number): string | undefined {
  const lines = content.slice(0, fenceStart).replace(/\r\n/g, "\n").split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    const filename = filenameFromLabelLine(lines[i]);
    if (filename) return filename;
    if (lines[i].trim()) return undefined;
  }
  return undefined;
}

function filenameFromLabelLine(line: string): string | undefined {
  const cleaned = line
    .trim()
    .replace(/^[#>*\-\d.)\s]+/, "")
    .replace(/[:：]\s*$/, "")
    .replace(/^["'`*]+|["'`*]+$/g, "");
  if (looksLikeFilename(cleaned)) return cleaned;
  const matches = [...line.matchAll(/`([^`]+)`/g)].map((match) => match[1]).filter(looksLikeFilename);
  return matches[matches.length - 1];
}

function collectStandaloneJson(trimmed: string, artifacts: ChatArtifact[]): void {
  if (!/^[{[]/.test(trimmed) || trimmed.length < 20) return;
  try {
    const parsed: unknown = JSON.parse(trimmed);
    const content = JSON.stringify(parsed, null, 2);
    artifacts.push(makeArtifact(artifacts.length, {
      content,
      filename: "response.json",
      language: "json",
      title: "response.json",
      kind: "json",
      disposition: "inline",
    }));
  } catch {
    /* not JSON */
  }
}

function collectCsvArtifact(trimmed: string, artifacts: ChatArtifact[]): void {
  if (!looksLikeCsv(trimmed)) return;
  artifacts.push(makeArtifact(artifacts.length, {
    content: trimmed,
    filename: "table.csv",
    language: "csv",
    title: "table.csv",
    kind: "csv",
    disposition: "inline",
  }));
}

function applyArtifactLimits(artifacts: ChatArtifact[]): ChatArtifact[] {
  let inlineCount = 0;
  let namedBytes = 0;
  let keptNamed = 0;
  let omittedInline = 0;
  let omittedNamed = 0;
  let omittedNamedBytes = 0;
  const kept: ChatArtifact[] = [];

  for (const artifact of artifacts) {
    if (isFileArtifact(artifact)) {
      if (namedBytes + artifact.size <= MAX_NAMED_ARTIFACT_BYTES) {
        kept.push(artifact);
        namedBytes += artifact.size;
        keptNamed++;
      } else {
        omittedNamed++;
        omittedNamedBytes += artifact.size;
      }
      continue;
    }
    if (inlineCount < MAX_INLINE_ARTIFACTS) {
      kept.push(artifact);
      inlineCount++;
    } else {
      omittedInline++;
    }
  }

  if (omittedNamed > 0) {
    const hiddenNamed = keptNamed + omittedNamed;
    const hiddenNamedBytes = namedBytes + omittedNamedBytes;
    return [
      limitWarningArtifact(0, `Milim hid all ${hiddenNamed} named file artifact(s) (${formatBytes(hiddenNamedBytes)}) because the generated file set exceeded the ${formatBytes(MAX_NAMED_ARTIFACT_BYTES)} preview budget. Runtime preview was disabled instead of staging a partial app.`),
      ...kept.filter((artifact) => !isFileArtifact(artifact)),
    ];
  }
  if (omittedInline > 0) {
    kept.push(limitWarningArtifact(kept.length, `Milim hid ${omittedInline} extra inline artifact(s). Named file artifacts were kept for preview/runtime staging.`));
  }
  return kept;
}

function limitWarningArtifact(index: number, content: string): ChatArtifact {
  return makeArtifact(index, {
    content,
    language: "txt",
    title: "Artifact extraction notice",
    kind: "text",
    disposition: "inline",
  });
}

function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${Math.round(size / 1024)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function makeArtifact(index: number, input: {
  content: string;
  filename?: string;
  language?: string;
  title: string;
  kind: ChatArtifactKind;
  disposition?: ChatArtifactDisposition;
}): ChatArtifact {
  const content = input.content.replace(/\s+$/g, "");
  return {
    id: `artifact-${index + 1}-${hashArtifact(input.title, content)}`,
    kind: input.kind,
    title: input.title,
    mime: mimeForSource(input.filename, input.language, input.kind),
    content,
    size: new Blob([content]).size,
    language: input.language,
    filename: input.filename,
    disposition: input.disposition ?? (input.filename ? "file" : "inline"),
  };
}

function artifactIdentity(artifact: ChatArtifact): string {
  return `${artifact.filename ?? artifact.title}\0${artifact.content}`;
}

function parseWriteFileArguments(raw: string | undefined): { path: string; content: string } | null {
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const args = parsed as { path?: unknown; content?: unknown };
    if (typeof args.path !== "string" || typeof args.content !== "string") return null;
    if (!args.path.trim()) return null;
    return { path: args.path, content: args.content };
  } catch {
    return null;
  }
}

function isGeneratedArtifactSource(filename: string, language: string | undefined): boolean {
  const source = (language || extensionOf(filename)).toLowerCase();
  return CODE_EXTENSIONS.has(source) || source === "json" || source === "csv" || source === "txt";
}

function parseFenceInfo(raw: string): FenceInfo {
  const info = raw.trim();
  if (!info) return {};
  const parts = info.match(/(?:[^\s"']+|"[^"]*"|'[^']*')+/g) ?? [];
  const result: FenceInfo = {};
  for (const part of parts) {
    const [key, ...rest] = part.split("=");
    const value = stripQuotes(rest.join("="));
    if (rest.length > 0) {
      if (key === "file" || key === "filename" || key === "path") result.filename = value;
      if (key === "title") result.title = value;
      continue;
    }
    const token = stripQuotes(part);
    if (!result.language && !looksLikeFilename(token)) {
      result.language = token.toLowerCase();
    } else if (!result.filename && looksLikeFilename(token)) {
      result.filename = token;
    }
  }
  if (!result.language && result.filename) {
    result.language = extensionOf(result.filename);
  }
  return result;
}

function cleanFenceContent(content: string): string {
  return content.replace(/\n$/g, "");
}

function leadingFileMetadata(content: string): { filename: string; content: string } | null {
  const normalized = content.replace(/\r\n/g, "\n");
  const newline = normalized.indexOf("\n");
  const firstLine = (newline >= 0 ? normalized.slice(0, newline) : normalized).trim();
  const match = firstLine.match(/^(?:file|filename|path)\s*=\s*["']?([^"']+)["']?$/i);
  if (!match || !looksLikeFilename(match[1])) return null;
  return {
    filename: match[1],
    content: newline >= 0 ? normalized.slice(newline + 1) : "",
  };
}

function stripQuotes(value: string): string {
  return value.replace(/^["']|["']$/g, "");
}

function looksLikeFilename(value: string): boolean {
  if (!value || /\s/.test(value)) return false;
  if (value.includes("/") || value.includes("\\")) return true;
  const ext = extensionOf(value);
  return Boolean(ext && (CODE_EXTENSIONS.has(ext) || ext === "json" || ext === "csv" || ext === "txt"));
}

function extensionOf(filename: string): string {
  const base = filename.split(/[\\/]/).pop() ?? filename;
  const dot = base.lastIndexOf(".");
  return dot >= 0 ? base.slice(dot + 1).toLowerCase() : "";
}

function kindForSource(filename: string | undefined, language: string | undefined, fallback: ChatArtifactKind): ChatArtifactKind {
  const source = (language || extensionOf(filename ?? "")).toLowerCase();
  if (source === "json") return "json";
  if (source === "csv") return "csv";
  if (source === "txt" || source === "text") return "text";
  return fallback;
}

function isPreviewableSource(filename: string | undefined, language: string | undefined): boolean {
  const source = (language || extensionOf(filename ?? "")).toLowerCase();
  return source === "html" || source === "htm" || source === "md" || source === "markdown" || PREVIEWABLE_SCRIPT_EXTENSIONS.has(source);
}

function mimeForSource(filename: string | undefined, language: string | undefined, kind: ChatArtifactKind): string {
  const source = (language || extensionOf(filename ?? "")).toLowerCase();
  if (kind === "json" || source === "json") return "application/json";
  if (kind === "csv" || kind === "table" || source === "csv") return "text/csv";
  if (source === "md" || source === "markdown") return "text/markdown";
  if (source === "html" || source === "htm") return "text/html";
  if (source === "css") return "text/css";
  if (source === "js" || source === "mjs" || source === "jsx" || source === "ts" || source === "tsx") return "text/javascript";
  if (source === "xml") return "application/xml";
  return "text/plain";
}

function hashArtifact(title: string, content: string): string {
  let hash = 2166136261;
  const input = `${title}\n${content}`;
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

function isMarkdownSeparator(line: string): boolean {
  const cells = markdownCells(line);
  return cells.length > 1 && cells.every((cell) => /^:?-{3,}:?$/.test(cell.replace(/\s+/g, "")));
}

function markdownCells(line: string): string[] {
  return line.trim().replace(/^\|/, "").replace(/\|$/, "").split("|").map((cell) => cell.trim());
}

export function markdownTableToCsv(lines: string[]): string {
  return lines
    .filter((line) => !isMarkdownSeparator(line))
    .map((line) => markdownCells(line).map(csvCell).join(","))
    .join("\n");
}

function csvCell(value: string): string {
  if (!/[",\n]/.test(value)) return value;
  return `"${value.replace(/"/g, '""')}"`;
}

function looksLikeCsv(content: string): boolean {
  const lines = content.split(/\r?\n/).filter((line) => line.trim());
  if (lines.length < 3 || lines.some((line) => !line.includes(","))) return false;
  const counts = lines.map(countCsvColumns);
  const expected = counts[0];
  return expected >= 2 && counts.every((count) => count === expected);
}

function countCsvColumns(line: string): number {
  let columns = 1;
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const char = line[i];
    if (char === '"' && line[i + 1] === '"') {
      i++;
    } else if (char === '"') {
      quoted = !quoted;
    } else if (char === "," && !quoted) {
      columns++;
    }
  }
  return columns;
}
