export type AppShortcutAction = "newChat" | "focusSearch" | "focusComposer" | "openComposerSuggestions" | "stopGeneration" | "toggleSidebar" | "previousThread";
export type AppShortcuts = Record<AppShortcutAction, string>;

export const APP_SHORTCUT_ACTIONS: AppShortcutAction[] = [
  "newChat",
  "focusSearch",
  "focusComposer",
  "openComposerSuggestions",
  "stopGeneration",
  "toggleSidebar",
  "previousThread",
];

export const APP_SHORTCUT_LABELS: Record<AppShortcutAction, string> = {
  newChat: "New chat",
  focusSearch: "Command palette",
  focusComposer: "Focus composer",
  openComposerSuggestions: "Open composer suggestions",
  stopGeneration: "Stop generation",
  toggleSidebar: "Toggle sidebar",
  previousThread: "Previous thread",
};

export const DEFAULT_APP_SHORTCUTS: AppShortcuts = {
  newChat: "Mod+N",
  focusSearch: "Mod+K",
  focusComposer: "Mod+L",
  openComposerSuggestions: "Mod+/",
  stopGeneration: "Escape",
  toggleSidebar: "Mod+B",
  previousThread: "Ctrl+Tab",
};

type ShortcutParts = {
  mod: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
};

type KeyboardLike = {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
  repeat?: boolean;
};

const KEY_ALIASES: Record<string, string> = {
  esc: "Escape",
  escape: "Escape",
  space: "Space",
  " ": "Space",
  spacebar: "Space",
  return: "Enter",
  enter: "Enter",
  tab: "Tab",
  backspace: "Backspace",
  delete: "Delete",
  del: "Delete",
  arrowup: "ArrowUp",
  up: "ArrowUp",
  arrowdown: "ArrowDown",
  down: "ArrowDown",
  arrowleft: "ArrowLeft",
  left: "ArrowLeft",
  arrowright: "ArrowRight",
  right: "ArrowRight",
};

function isMacPlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  return /(Mac|iPhone|iPad|iPod)/i.test(`${navigator.platform} ${navigator.userAgent}`);
}

function normalizeKey(key: unknown): string | null {
  if (typeof key !== "string") return null;
  const raw = key.trim();
  if (!raw) return null;
  const alias = KEY_ALIASES[raw.toLowerCase()];
  if (alias) return alias;
  if (/^[a-z]$/i.test(raw)) return raw.toUpperCase();
  if (/^[0-9]$/.test(raw)) return raw;
  const fKey = raw.match(/^f([1-9]|1\d|2[0-4])$/i);
  if (fKey) return `F${Number(fKey[1])}`;
  if (raw.length > 1) return raw[0].toUpperCase() + raw.slice(1);
  return null;
}

function parseShortcut(shortcut: unknown): ShortcutParts | null {
  if (typeof shortcut !== "string") return null;
  const parts = shortcut.split("+").map((part) => part.trim()).filter(Boolean);
  if (!parts.length) return null;
  const result: ShortcutParts = { mod: false, ctrl: false, alt: false, shift: false, key: "" };
  for (const part of parts) {
    const token = part.toLowerCase().replace(/\s+/g, "");
    if (token === "mod" || token === "cmd" || token === "command" || token === "meta" || token === "commandorcontrol" || token === "cmdorctrl") {
      result.mod = true;
    } else if (token === "ctrl" || token === "control") {
      result.ctrl = true;
    } else if (token === "alt" || token === "option") {
      result.alt = true;
    } else if (token === "shift") {
      result.shift = true;
    } else if (!result.key) {
      const key = normalizeKey(part);
      if (!key) return null;
      result.key = key;
    } else {
      return null;
    }
  }
  return result.key ? result : null;
}

function serializeShortcut(parts: ShortcutParts): string {
  return [
    parts.mod ? "Mod" : "",
    parts.ctrl ? "Ctrl" : "",
    parts.alt ? "Alt" : "",
    parts.shift ? "Shift" : "",
    parts.key,
  ].filter(Boolean).join("+");
}

export function normalizeShortcut(shortcut: unknown): string | null {
  const parsed = parseShortcut(shortcut);
  return parsed ? serializeShortcut(parsed) : null;
}

export function shortcutToGlobalAccelerator(shortcut: unknown): string | null {
  const normalized = normalizeShortcut(shortcut);
  return normalized ? normalized.replace(/^Mod(?=\+)/, "CommandOrControl") : null;
}

export function globalAcceleratorToShortcut(shortcut: unknown): string | null {
  if (typeof shortcut !== "string") return null;
  return normalizeShortcut(shortcut.replace(/^CommandOrControl(?=\+)/i, "Mod"));
}

export function shortcutValidationIssue(shortcut: unknown): string | null {
  const parsed = parseShortcut(shortcut);
  if (!parsed) return "Press a key combination.";
  const hasModifier = parsed.mod || parsed.ctrl || parsed.alt || parsed.shift;
  if (hasModifier) return null;
  if (parsed.key === "Escape" || /^F([1-9]|1\d|2[0-4])$/.test(parsed.key)) return null;
  return "Use a modifier, Escape, or an F-key.";
}

export function shortcutFromKeyboardEvent(event: KeyboardLike, mac = isMacPlatform()): string | null {
  if (event.repeat) return null;
  const key = normalizeKey(event.key);
  if (!key || key === "Control" || key === "Meta" || key === "Alt" || key === "Shift") return null;
  if (!mac && event.metaKey) return null;
  const shortcut = serializeShortcut({
    mod: mac ? Boolean(event.metaKey) : Boolean(event.ctrlKey),
    ctrl: mac ? Boolean(event.ctrlKey) : false,
    alt: Boolean(event.altKey),
    shift: Boolean(event.shiftKey),
    key,
  });
  return shortcutValidationIssue(shortcut) ? null : shortcut;
}

export function shortcutMatchesEvent(shortcut: string, event: KeyboardLike, mac = isMacPlatform()): boolean {
  if (event.repeat) return false;
  const parsed = parseShortcut(shortcut);
  const key = normalizeKey(event.key);
  if (!parsed || !key || parsed.key !== key) return false;
  const controlPressed = Boolean(event.ctrlKey);
  const modifierMatches = mac
    ? parsed.mod === Boolean(event.metaKey) && parsed.ctrl === controlPressed
    : !event.metaKey && (parsed.mod || parsed.ctrl) === controlPressed;
  return modifierMatches &&
    parsed.alt === Boolean(event.altKey) &&
    parsed.shift === Boolean(event.shiftKey);
}

export type ComposerEnterAction = "none" | "send" | "steer";

export function composerEnterAction(
  event: KeyboardLike,
  options: {
    sendShortcut: "enter" | "modEnter";
    busy: boolean;
    canSteer: boolean;
    mac?: boolean;
  },
): ComposerEnterAction {
  if (normalizeKey(event.key) !== "Enter" || event.shiftKey || event.repeat) return "none";
  const modifierEnter = shortcutMatchesEvent("Mod+Enter", event, options.mac ?? isMacPlatform());
  if (options.busy && options.canSteer && modifierEnter) return "steer";
  if (options.sendShortcut === "enter") return "send";
  return modifierEnter ? "send" : "none";
}

export function uiSizeShortcutDelta(event: KeyboardLike, mac = isMacPlatform()): -1 | 0 | 1 {
  const modPressed = mac ? Boolean(event.metaKey) : Boolean(event.ctrlKey);
  const extraModPressed = mac ? Boolean(event.ctrlKey) : Boolean(event.metaKey);
  if (!modPressed || extraModPressed || event.altKey) return 0;
  if (event.key === "-") return -1;
  if (event.key === "=" || event.key === "+") return 1;
  return 0;
}

export function shortcutLabel(shortcut: string, mac = isMacPlatform()): string {
  const normalized = normalizeShortcut(shortcut) ?? shortcut;
  return normalized.split("+").map((part) => part === "Mod" ? (mac ? "Cmd" : "Ctrl") : part).join("+");
}

export function normalizeAppShortcuts(value: unknown): AppShortcuts {
  const source = value && typeof value === "object" && !Array.isArray(value)
    ? value as Partial<Record<AppShortcutAction, unknown>>
    : {};
  const used = new Set<string>();
  const next = {} as AppShortcuts;
  for (const action of APP_SHORTCUT_ACTIONS) {
    const normalized = normalizeShortcut(source[action]);
    const candidate = action === "previousThread" && normalized === "Mod+Tab" ? "Ctrl+Tab" : normalized;
    const valid = candidate && !shortcutValidationIssue(candidate) && !used.has(candidate)
      ? candidate
      : DEFAULT_APP_SHORTCUTS[action];
    next[action] = used.has(valid) ? DEFAULT_APP_SHORTCUTS[action] : valid;
    used.add(next[action]);
  }
  return next;
}

export function shortcutConflict(shortcuts: AppShortcuts, action: AppShortcutAction, shortcut: string): AppShortcutAction | null {
  const normalized = normalizeShortcut(shortcut);
  if (!normalized) return null;
  return APP_SHORTCUT_ACTIONS.find((other) => other !== action && shortcuts[other] === normalized) ?? null;
}
