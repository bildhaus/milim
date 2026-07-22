export type ComposerAutocompleteTrigger = {
  prefix: "/" | "@";
  query: string;
  start: number;
  end: number;
};

export function composerAutocompleteTriggerAt(value: string, cursor: number): ComposerAutocompleteTrigger | null {
  const end = Math.max(0, Math.min(cursor, value.length));
  let start = end;
  while (start > 0 && !/\s/.test(value[start - 1])) start -= 1;
  const token = value.slice(start, end);
  const prefix = token[0];
  if (prefix !== "/" && prefix !== "@") return null;
  return { prefix, query: token.slice(1).toLowerCase(), start, end };
}

export function replaceComposerAutocompleteTrigger(
  value: string,
  trigger: ComposerAutocompleteTrigger,
  replacement: string,
): string {
  return value.slice(0, trigger.start) + replacement + value.slice(trigger.end);
}

export function skillTagCompletion(prefix: "/" | "@", skillName: string): string {
  return `${prefix}${skillName} `;
}

export function mcpToolTagCompletion(toolName: string): string {
  return `/${toolName} `;
}

export function composerCommandRunsOnSelection(commandId: string): boolean {
  return commandId === "plan" || commandId === "goal";
}

export function composerSuggestionMatchScore(text: string, query: string): number | null {
  const normalizedText = text.trim().toLowerCase();
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) return 0;
  if (normalizedText.startsWith(normalizedQuery)) return 0;
  if (normalizedText.split(/[^a-z0-9]+/).some((token) => token.startsWith(normalizedQuery))) return 1;
  const index = normalizedText.indexOf(normalizedQuery);
  return index >= 0 ? 2 + index / Math.max(1, normalizedText.length) : null;
}
