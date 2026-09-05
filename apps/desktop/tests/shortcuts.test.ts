import {
  APP_SHORTCUT_LABELS,
  composerEnterAction,
  isComposingKeyEvent,
  DEFAULT_APP_SHORTCUTS,
  globalAcceleratorToShortcut,
  normalizeAppShortcuts,
  normalizeShortcut,
  shortcutConflict,
  shortcutFromKeyboardEvent,
  shortcutLabel,
  shortcutMatchesEvent,
  shortcutToGlobalAccelerator,
  shortcutValidationIssue,
  uiSizeShortcutDelta,
} from "../src/ui/shortcuts.js";

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function withNavigator(platform: string, userAgent: string, run: () => void): void {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { platform, userAgent },
  });
  try {
    run();
  } finally {
    if (previous) Object.defineProperty(globalThis, "navigator", previous);
    else delete (globalThis as { navigator?: Navigator }).navigator;
  }
}

for (const composing of [{ isComposing: true }, { keyCode: 229 }, { nativeEvent: { isComposing: true } }, { nativeEvent: { keyCode: 229 } }]) {
  equal(isComposingKeyEvent({ key: "Enter", ...composing }), true, "native and React IME events must be recognized before autocomplete");
  for (const busy of [false, true]) {
    equal(composerEnterAction({ key: "Enter", ctrlKey: true, ...composing }, { sendShortcut: "enter", busy, canSteer: true, mac: false }), "none", "IME confirmation must never send or steer");
  }
}

equal(normalizeShortcut("ctrl+k"), "Ctrl+K", "ctrl should normalize to literal Ctrl");
equal(normalizeShortcut("ctrl+tab"), "Ctrl+Tab", "ctrl tab should normalize to literal Ctrl+Tab");
equal(normalizeShortcut("CommandOrControl+Shift+Space"), "Mod+Shift+Space", "CommandOrControl should normalize");
equal(shortcutToGlobalAccelerator("Mod+Shift+Space"), "CommandOrControl+Shift+Space", "Mod shortcut should convert to Tauri accelerator");
equal(shortcutToGlobalAccelerator("Ctrl+Tab"), "Ctrl+Tab", "literal Ctrl shortcut should stay literal for Tauri accelerator");
equal(globalAcceleratorToShortcut("CommandOrControl+Shift+Space"), "Mod+Shift+Space", "Tauri accelerator should convert back to normalized shortcut");
equal(shortcutLabel("Mod+K", false), "Ctrl+K", "Windows labels should show Ctrl");
equal(shortcutLabel("Mod+K", true), "Cmd+K", "Mac labels should show Cmd");
equal(shortcutLabel("Ctrl+K", true), "Ctrl+K", "Mac labels should keep literal Ctrl");
withNavigator("MacIntel", "", () => {
  equal(shortcutLabel("Mod+K"), "Cmd+K", "MacIntel should default labels to Cmd");
  equal(shortcutMatchesEvent("Mod+K", { key: "k", metaKey: true }), true, "MacIntel should match Cmd");
  equal(shortcutMatchesEvent("Mod+K", { key: "k", ctrlKey: true }), false, "MacIntel should reject Ctrl");
  equal(shortcutMatchesEvent("Ctrl+Tab", { key: "Tab", ctrlKey: true }), true, "MacIntel should match literal Ctrl+Tab");
  equal(shortcutMatchesEvent("Ctrl+Tab", { key: "Tab", metaKey: true }), false, "MacIntel should not match Cmd+Tab as Ctrl+Tab");
});

equal(shortcutFromKeyboardEvent({ key: "k", ctrlKey: true }, false), "Mod+K", "Windows recording should capture Ctrl");
equal(shortcutFromKeyboardEvent({ key: "k", metaKey: true }, true), "Mod+K", "Mac recording should capture Cmd");
equal(shortcutFromKeyboardEvent({ key: "k", ctrlKey: true }, true), "Ctrl+K", "Mac recording should capture literal Ctrl");
equal(shortcutFromKeyboardEvent({ key: "Escape" }), "Escape", "recording should allow bare Escape");
equal(shortcutFromKeyboardEvent({ key: "F2" }), "F2", "recording should allow F-keys");
equal(shortcutFromKeyboardEvent({ key: "k" }), null, "recording should reject bare letters");
equal(shortcutFromKeyboardEvent({ key: "Shift", shiftKey: true }), null, "recording should reject modifier-only events");
equal(shortcutFromKeyboardEvent({ key: "k", ctrlKey: true, repeat: true }), null, "recording should ignore repeat events");

equal(shortcutValidationIssue("N"), "Use a modifier, Escape, or an F-key.", "bare letters should be invalid");
equal(shortcutValidationIssue("Mod+N"), null, "modified letters should be valid");
equal(shortcutMatchesEvent("Mod+K", { key: "k", ctrlKey: true }, false), true, "Mod should match Ctrl on Windows");
equal(shortcutMatchesEvent("Ctrl+Tab", { key: "Tab", ctrlKey: true }, false), true, "Ctrl+Tab should match Ctrl+Tab on Windows");
equal(shortcutMatchesEvent("Mod+K", { key: "k", metaKey: true }, true), true, "Mod should match Cmd on Mac");
equal(shortcutMatchesEvent("Mod+K", { key: "k", metaKey: true }, false), false, "Meta should not match Mod on Windows");
equal(uiSizeShortcutDelta({ key: "=", ctrlKey: true }, false), 1, "Ctrl+= should zoom UI in on Windows");
equal(uiSizeShortcutDelta({ key: "+", ctrlKey: true, shiftKey: true }, false), 1, "Ctrl++ should zoom UI in on Windows");
equal(uiSizeShortcutDelta({ key: "-", ctrlKey: true }, false), -1, "Ctrl+- should zoom UI out on Windows");
equal(uiSizeShortcutDelta({ key: "=", metaKey: true }, true), 1, "Cmd+= should zoom UI in on Mac");
equal(uiSizeShortcutDelta({ key: "-", metaKey: true }, true), -1, "Cmd+- should zoom UI out on Mac");
equal(uiSizeShortcutDelta({ key: "=", ctrlKey: true }, true), 0, "Ctrl+= should not act as Cmd+= on Mac");
equal(uiSizeShortcutDelta({ key: "=", ctrlKey: true, altKey: true }, false), 0, "Alt should not be part of UI size shortcuts");

equal(
  composerEnterAction(
    { key: "Enter" },
    { sendShortcut: "enter", busy: true, canSteer: true, mac: false },
  ),
  "send",
  "plain Enter should keep queueing while a steer-capable run is busy",
);
equal(
  composerEnterAction(
    { key: "Enter", ctrlKey: true },
    { sendShortcut: "enter", busy: true, canSteer: true, mac: false },
  ),
  "steer",
  "Ctrl+Enter should steer a busy steer-capable run on Windows",
);
equal(
  composerEnterAction(
    { key: "Enter", metaKey: true },
    { sendShortcut: "enter", busy: true, canSteer: true, mac: true },
  ),
  "steer",
  "Cmd+Enter should steer a busy steer-capable run on macOS",
);
equal(
  composerEnterAction(
    { key: "Enter", ctrlKey: true },
    { sendShortcut: "enter", busy: true, canSteer: false, mac: false },
  ),
  "send",
  "modifier Enter should keep queueing when the active runtime cannot steer",
);
equal(
  composerEnterAction(
    { key: "Enter", ctrlKey: true },
    { sendShortcut: "enter", busy: false, canSteer: true, mac: false },
  ),
  "send",
  "modifier Enter should send normally while idle",
);
equal(
  composerEnterAction(
    { key: "Enter", ctrlKey: true },
    { sendShortcut: "modEnter", busy: true, canSteer: true, mac: false },
  ),
  "steer",
  "the modifier-send preference should still steer during a capable active run",
);
equal(
  composerEnterAction(
    { key: "Enter" },
    { sendShortcut: "modEnter", busy: true, canSteer: true, mac: false },
  ),
  "none",
  "plain Enter should remain a newline with the modifier-send preference",
);
equal(
  composerEnterAction(
    { key: "Enter", ctrlKey: true, shiftKey: true },
    { sendShortcut: "enter", busy: true, canSteer: true, mac: false },
  ),
  "none",
  "Shift+modifier+Enter should remain a newline",
);

const normalized = normalizeAppShortcuts({
  newChat: "Mod+Shift+N",
  focusSearch: "Mod+Shift+N",
  focusComposer: "x",
  stopGeneration: "F2",
  previousThread: "Mod+Tab",
});
equal(normalized.newChat, "Mod+Shift+N", "first custom shortcut should survive");
equal(normalized.focusSearch, DEFAULT_APP_SHORTCUTS.focusSearch, "duplicate persisted shortcut should fall back");
equal(normalized.focusComposer, DEFAULT_APP_SHORTCUTS.focusComposer, "invalid persisted shortcut should fall back");
equal(normalized.stopGeneration, "F2", "valid bare F-key should survive");
equal(normalized.previousThread, "Ctrl+Tab", "old previous thread default should migrate to Ctrl+Tab");
equal(shortcutConflict(DEFAULT_APP_SHORTCUTS, "newChat", "Mod+K"), "focusSearch", "conflict lookup should name the existing action");
equal(shortcutConflict(DEFAULT_APP_SHORTCUTS, "newChat", "Ctrl+Tab"), "previousThread", "conflict lookup should find thread shortcut");
equal(APP_SHORTCUT_LABELS.focusSearch, "Command palette", "persisted focusSearch action should label the palette");

export {};
