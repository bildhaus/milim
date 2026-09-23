import type { ChatMessage, PreviewAppFile } from "../api";
import { hasPreviewPackageJson, previewRuntimeFiles } from "./artifacts.js";

const RUNTIME_CONTEXT_MAX_FILES = 8;
const RUNTIME_CONTEXT_MAX_CHARS = 32 * 1024;
const RUNTIME_CONTEXT_MAX_FILE_CHARS = 16 * 1024;

export const NO_FOLDER_ARTIFACT_INSTRUCTIONS = [
  "milim captures named fenced code blocks into the current chat's artifact panel.",
  "When no working folder is selected and the user asks you to create a file, web app, document, dataset, or other generated artifact, return the artifact inline instead of asking for a folder.",
  "Use fenced blocks with filename metadata, for example ```html file=index.html ... ```; a standalone filename line immediately before a fence or a file=path first line inside a fence is also treated as that file path. For multi-file artifacts, return one named fenced block per relative file path.",
  "Runnable Node/Vite apps must be returned as named files, including package.json, index.html, src/App.tsx, and any sibling CSS/JS/TS/TSX files; do not return runnable apps as anonymous tsx blocks.",
  "Markdown tables should stay as markdown tables unless the user explicitly asks for a CSV file.",
  "For browser apps, use index.html plus sibling CSS/JS/TS/TSX files when that is clearer; the preview resolves relative links and imports across those artifacts.",
  "Ask for a folder only when the user wants you to read, modify, run, or save directly against existing local project files.",
].join(" ");

export function threadArtifactInstructionMessages(
  folder: string,
  conversation: readonly ChatMessage[] = [],
  lastUserText = latestUserContent(conversation),
  virtualProjectFiles: readonly PreviewAppFile[] = [],
): ChatMessage[] {
  if (folder.trim()) return [];
  const messages: ChatMessage[] = [{ role: "system", content: NO_FOLDER_ARTIFACT_INSTRUCTIONS }];
  const runtimeContext = runtimePreviewFileContext(conversation, lastUserText, virtualProjectFiles);
  if (runtimeContext) messages.push({ role: "system", content: runtimeContext });
  return messages;
}

export const PLAN_MODE_INSTRUCTIONS = [
  "milim Plan Mode is active for this turn.",
  "Use only read-only inspection tools that are available to understand the request and relevant files.",
  "Do not edit files, write files, run commands, control the computer, create schedules, register memories, or perform other mutations.",
  "After inspecting enough context, return an inline implementation plan that is concrete enough to execute.",
  "Do not implement the plan until the user approves execution.",
].join("\n");

export function planModeInstructionMessages(active: boolean): ChatMessage[] {
  return active ? [{ role: "system", content: PLAN_MODE_INSTRUCTIONS }] : [];
}

function runtimePreviewFileContext(conversation: readonly ChatMessage[], lastUserText: string, virtualProjectFiles: readonly PreviewAppFile[]): string {
  const files = virtualProjectFiles.length ? [...virtualProjectFiles] : latestRunnablePreviewFiles(conversation);
  if (!files.length || !shouldIncludeRuntimePreviewContext(lastUserText, files)) return "";
  const selected = prioritizePreviewFiles(files, lastUserText).slice(0, RUNTIME_CONTEXT_MAX_FILES);
  let remaining = RUNTIME_CONTEXT_MAX_CHARS;
  const parts = [
    "Current no-folder virtual project files are available as read-only context below. These are generated artifact files managed by milim for this thread's preview app, not arbitrary local disk access. Use them when the user asks about the runtime preview or one of these files.",
    `Files: ${files.map((file) => file.path).join(", ")}`,
  ];
  for (const file of selected) {
    if (remaining <= 0) break;
    const block = previewFileBlock(file, remaining);
    if (!block) break;
    remaining -= block.length;
    parts.push(block);
  }
  return parts.join("\n\n");
}

function latestRunnablePreviewFiles(conversation: readonly ChatMessage[]): PreviewAppFile[] {
  for (let i = conversation.length - 1; i >= 0; i -= 1) {
    const message = conversation[i];
    if (message.role !== "assistant" || !message.artifacts?.length) continue;
    const files = previewRuntimeFiles(message.artifacts);
    if (files.length && hasPreviewPackageJson(files)) return files;
  }
  return [];
}

function shouldIncludeRuntimePreviewContext(lastUserText: string, files: readonly PreviewAppFile[]): boolean {
  const text = lastUserText.toLowerCase();
  if (!text.trim()) return false;
  if (files.some((file) => text.includes(file.path.toLowerCase()))) return true;
  return /\b(runtime|preview|vite|error|logs?|read|file|code|fix|broken|unstyled|tsx|jsx|css|html|package\.json)\b/.test(text);
}

function prioritizePreviewFiles(files: readonly PreviewAppFile[], lastUserText: string): PreviewAppFile[] {
  const text = lastUserText.toLowerCase();
  return [...files].sort((left, right) => previewFilePriority(right, text) - previewFilePriority(left, text));
}

function previewFilePriority(file: PreviewAppFile, text: string): number {
  const path = file.path.toLowerCase();
  if (text.includes(path)) return 100;
  if (path === "package.json") return 80;
  if (path === "index.html") return 70;
  if (/src\/(main|app|index)\.(tsx|ts|jsx|js|css)$/.test(path)) return 60;
  return 0;
}

function previewFileBlock(file: PreviewAppFile, remaining: number): string {
  const budget = Math.min(RUNTIME_CONTEXT_MAX_FILE_CHARS, remaining);
  if (budget <= file.path.length + 96) return "";
  const contentBudget = budget - file.path.length - 96;
  const truncated = file.content.length > contentBudget;
  const content = escapeFenceContent(file.content.slice(0, contentBudget));
  return [
    `File: ${file.path}${truncated ? " (truncated)" : ""}`,
    `\`\`\`${extensionOf(file.path)}`,
    content,
    "```",
  ].join("\n");
}

function escapeFenceContent(content: string): string {
  return content.replace(/```/g, "``\\`");
}

function extensionOf(path: string): string {
  return path.match(/\.([A-Za-z0-9]+)$/)?.[1]?.toLowerCase() ?? "";
}

function latestUserContent(conversation: readonly ChatMessage[]): string {
  return conversation
    .slice()
    .reverse()
    .find((message) => message.role === "user")?.content ?? "";
}
