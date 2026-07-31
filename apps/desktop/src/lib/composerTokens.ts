import type { SkillInfo, ToolInfo, WorkspaceFileSuggestion } from "../api";

export type ComposerTokenKind = "skill" | "mcp" | "file" | "link";

export type ComposerToken = {
  kind: ComposerTokenKind;
  start: number;
  end: number;
  label: string;
  value: string;
};

export type ComposerTokenPart =
  | { kind: "text"; text: string }
  | { kind: "token"; text: string; token: ComposerToken };

export type ComposerLinkClickAction = "external" | "sidepanel";
export type ComposerOffsetEdge = "nearest" | "start" | "end";

export type ComposerDisplayPart = (
  | { kind: "text" }
  | { kind: "token"; token: ComposerToken }
) & {
  text: string;
  rawStart: number;
  rawEnd: number;
  displayStart: number;
  displayEnd: number;
};

export type ComposerDisplay = {
  text: string;
  parts: ComposerDisplayPart[];
  rawOffset: (displayOffset: number, edge?: ComposerOffsetEdge) => number;
  displayOffset: (rawOffset: number, edge?: ComposerOffsetEdge) => number;
  applyEdit: (nextDisplayText: string) => { value: string; cursor: number };
};

type ComposerTokenCandidate = ComposerToken & { priority: number };

type ComposerTokenOptions = {
  skills?: SkillInfo[];
  tools?: ToolInfo[];
  workspaceFiles?: WorkspaceFileSuggestion[];
};

const TOKEN_PRIORITIES: Record<ComposerTokenKind, number> = {
  skill: 0,
  mcp: 1,
  file: 2,
  link: 3,
};

const URL_PATTERN = /https?:\/\/[^\s<>"'`]+/gi;
const OBJECT_REPLACEMENT_CHARACTER = "\uFFFC";

export function composerTokensForText(text: string, options: ComposerTokenOptions = {}): ComposerToken[] {
  const candidates = [
    ...skillTokenCandidates(text, options.skills ?? []),
    ...mcpTokenCandidates(text, options.tools ?? []),
    ...fileTokenCandidates(text, options.workspaceFiles ?? []),
    ...linkTokenCandidates(text),
  ];
  return selectTokenCandidates(candidates);
}

export function composerTokenParts(text: string, tokens: ComposerToken[]): ComposerTokenPart[] {
  const parts: ComposerTokenPart[] = [];
  let cursor = 0;
  for (const token of tokens) {
    if (token.start > cursor) {
      parts.push({ kind: "text", text: text.slice(cursor, token.start) });
    }
    parts.push({ kind: "token", text: text.slice(token.start, token.end), token });
    cursor = token.end;
  }
  if (cursor < text.length) {
    parts.push({ kind: "text", text: text.slice(cursor) });
  }
  return parts;
}

export function composerLinkClickAction(modifiers: {
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): ComposerLinkClickAction | null {
  if (!modifiers.ctrlKey && !modifiers.metaKey) return null;
  return modifiers.shiftKey ? "sidepanel" : "external";
}

export function pasteComposerUrl(
  value: string,
  start: number,
  end: number,
  clipboardText: string,
): { value: string; cursor: number } | null {
  if (!clipboardText || clipboardText.trim() !== clipboardText) return null;
  try {
    const url = new URL(clipboardText);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  } catch {
    return null;
  }
  const insertion = clipboardText + (/^\s/.test(value.slice(end)) ? "" : " ");
  return {
    value: value.slice(0, start) + insertion + value.slice(end),
    cursor: start + insertion.length,
  };
}

export function composerDisplayForText(text: string, tokens: ComposerToken[]): ComposerDisplay {
  const rawParts = composerTokenParts(text, tokens);
  const parts: ComposerDisplayPart[] = [];
  let rawOffset = 0;
  let displayOffset = 0;
  for (const part of rawParts) {
    const rawStart = rawOffset;
    rawOffset += part.text.length;
    const displayText = part.kind === "token" && isGithubLinkToken(part.token)
      ? `${OBJECT_REPLACEMENT_CHARACTER} ${[...part.token.label].join("\u200B")}`
      : part.text;
    const displayStart = displayOffset;
    displayOffset += displayText.length;
    parts.push({
      ...part,
      text: displayText,
      rawStart,
      rawEnd: rawOffset,
      displayStart,
      displayEnd: displayOffset,
    });
  }

  const displayText = parts.map((part) => part.text).join("");
  const display: ComposerDisplay = {
    text: displayText,
    parts,
    rawOffset(offset, edge = "nearest") {
      return mapOffset(parts, clamp(offset, 0, displayText.length), "display", edge);
    },
    displayOffset(offset, edge = "nearest") {
      return mapOffset(parts, clamp(offset, 0, text.length), "raw", edge);
    },
    applyEdit(nextDisplayText) {
      let start = 0;
      while (
        start < displayText.length
        && start < nextDisplayText.length
        && displayText[start] === nextDisplayText[start]
      ) start += 1;
      let oldEnd = displayText.length;
      let nextEnd = nextDisplayText.length;
      while (
        oldEnd > start
        && nextEnd > start
        && displayText[oldEnd - 1] === nextDisplayText[nextEnd - 1]
      ) {
        oldEnd -= 1;
        nextEnd -= 1;
      }
      const insertion = nextDisplayText.slice(start, nextEnd);
      const rawStart = display.rawOffset(start, oldEnd === start ? "nearest" : "start");
      const rawEnd = display.rawOffset(oldEnd, oldEnd === start ? "nearest" : "end");
      return {
        value: text.slice(0, rawStart) + insertion + text.slice(rawEnd),
        cursor: rawStart + insertion.length,
      };
    },
  };
  return display;
}

function isGithubLinkToken(token: ComposerToken): boolean {
  return token.kind === "link" && token.label !== token.value;
}

function mapOffset(
  parts: ComposerDisplayPart[],
  offset: number,
  source: "raw" | "display",
  edge: ComposerOffsetEdge,
): number {
  for (const part of parts) {
    const start = source === "raw" ? part.rawStart : part.displayStart;
    const end = source === "raw" ? part.rawEnd : part.displayEnd;
    const targetStart = source === "raw" ? part.displayStart : part.rawStart;
    const targetEnd = source === "raw" ? part.displayEnd : part.rawEnd;
    if (offset < start) return targetStart;
    if (offset > end) continue;
    if (part.kind !== "token" || !isGithubLinkToken(part.token)) {
      return targetStart + offset - start;
    }
    if (offset === start) return targetStart;
    if (offset === end) return targetEnd;
    if (edge === "start") return targetStart;
    if (edge === "end") return targetEnd;
    return offset - start < (end - start) / 2 ? targetStart : targetEnd;
  }
  const last = parts[parts.length - 1];
  return last
    ? source === "raw" ? last.displayEnd : last.rawEnd
    : 0;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function skillTokenCandidates(text: string, skills: SkillInfo[]): ComposerTokenCandidate[] {
  const enabledSkills = skills
    .filter((skill) => skill.enabled && skill.name.trim())
    .slice()
    .sort((a, b) => b.name.length - a.name.length || a.name.localeCompare(b.name));
  const candidates: ComposerTokenCandidate[] = [];
  for (let index = 0; index < text.length; index += 1) {
    const prefix = text[index];
    if ((prefix !== "@" && prefix !== "/") || !isTagStartBoundary(text, index)) continue;
    for (const skill of enabledSkills) {
      const end = matchNameAt(text, index + 1, skill.name);
      if (end === null || !isTagEndBoundary(text, end)) continue;
      candidates.push({
        kind: "skill",
        start: index,
        end,
        label: skill.name,
        value: skill.id,
        priority: TOKEN_PRIORITIES.skill,
      });
      break;
    }
  }
  return candidates;
}

function mcpTokenCandidates(text: string, tools: ToolInfo[]): ComposerTokenCandidate[] {
  const toolsByName = tools
    .filter((tool) => tool.name.includes("__"))
    .slice()
    .sort((a, b) => b.name.length - a.name.length || a.name.localeCompare(b.name));
  const candidates: ComposerTokenCandidate[] = [];
  for (let index = 0; index < text.length; index += 1) {
    if (text[index] !== "/" || !isTagStartBoundary(text, index)) continue;
    for (const tool of toolsByName) {
      const end = index + 1 + tool.name.length;
      if (text.slice(index + 1, end).toLowerCase() !== tool.name.toLowerCase()) continue;
      if (!isTagEndBoundary(text, end)) continue;
      candidates.push({
        kind: "mcp",
        start: index,
        end,
        label: tool.name,
        value: tool.name,
        priority: TOKEN_PRIORITIES.mcp,
      });
      break;
    }
  }
  return candidates;
}

function fileTokenCandidates(text: string, workspaceFiles: WorkspaceFileSuggestion[]): ComposerTokenCandidate[] {
  const knownPaths = new Set(workspaceFiles.map((file) => normalizePathToken(file.path)));
  const candidates: ComposerTokenCandidate[] = [];
  for (let index = 0; index < text.length; index += 1) {
    if (text[index] !== "@" || !isTagStartBoundary(text, index)) continue;
    const parsed = parseFileToken(text, index + 1);
    if (!parsed) continue;
    const normalized = normalizePathToken(parsed.value);
    if (!knownPaths.has(normalized) && !looksLikeWorkspaceFilePath(parsed.value)) continue;
    candidates.push({
      kind: "file",
      start: index,
      end: parsed.end,
      label: parsed.value,
      value: parsed.value,
      priority: TOKEN_PRIORITIES.file,
    });
  }
  return candidates;
}

function linkTokenCandidates(text: string): ComposerTokenCandidate[] {
  const candidates: ComposerTokenCandidate[] = [];
  for (const match of text.matchAll(URL_PATTERN)) {
    const start = match.index ?? 0;
    let end = start + match[0].length;
    while (end > start && /[.,;:!?)}\]]/.test(text[end - 1])) end -= 1;
    if (end <= start) continue;
    const value = text.slice(start, end);
    const githubLabel = githubLinkLabel(value);
    candidates.push({
      kind: "link",
      start,
      end,
      label: githubLabel ?? value,
      value,
      priority: TOKEN_PRIORITIES.link,
    });
  }
  return candidates;
}

function githubLinkLabel(value: string): string | null {
  try {
    const url = new URL(value);
    if (url.hostname !== "github.com" && url.hostname !== "www.github.com") return null;
    return url.pathname.replace(/^\/|\/$/g, "") || "github.com";
  } catch {
    return null;
  }
}

function selectTokenCandidates(candidates: ComposerTokenCandidate[]): ComposerToken[] {
  const selected: ComposerToken[] = [];
  const sorted = candidates.slice().sort((a, b) =>
    a.start - b.start ||
    a.priority - b.priority ||
    (b.end - b.start) - (a.end - a.start) ||
    a.label.localeCompare(b.label),
  );
  let cursor = 0;
  for (const candidate of sorted) {
    if (candidate.start < cursor) continue;
    const { priority: _priority, ...token } = candidate;
    selected.push(token);
    cursor = token.end;
  }
  return selected;
}

function parseFileToken(text: string, start: number): { value: string; end: number } | null {
  if (text[start] === "\"") {
    const endQuote = text.indexOf("\"", start + 1);
    if (endQuote <= start + 1) return null;
    return { value: text.slice(start + 1, endQuote), end: endQuote + 1 };
  }
  let end = start;
  while (end < text.length && !/\s/.test(text[end])) end += 1;
  if (end === start) return null;
  return { value: text.slice(start, end), end };
}

function looksLikeWorkspaceFilePath(value: string): boolean {
  return /[\\/]/.test(value) || /\.[A-Za-z0-9]{1,12}$/.test(value);
}

function normalizePathToken(value: string): string {
  return value.replace(/\\/g, "/").toLowerCase();
}

function matchNameAt(text: string, start: number, name: string): number | null {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  let cursor = start;
  for (let partIndex = 0; partIndex < parts.length; partIndex += 1) {
    const part = parts[partIndex];
    if (text.slice(cursor, cursor + part.length).toLowerCase() !== part.toLowerCase()) {
      return null;
    }
    cursor += part.length;
    if (partIndex < parts.length - 1) {
      const next = cursor;
      while (cursor < text.length && /\s/.test(text[cursor])) cursor += 1;
      if (cursor === next) return null;
    }
  }
  return cursor;
}

function isTagStartBoundary(text: string, index: number): boolean {
  if (index === 0) return true;
  return /\s|[([{]/.test(text[index - 1]);
}

function isTagEndBoundary(text: string, index: number): boolean {
  if (index >= text.length) return true;
  return /\s|[,.;:!?()[\]{}"'`]/.test(text[index]);
}
