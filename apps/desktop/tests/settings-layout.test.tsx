import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
}

const source = readFileSync(resolve(process.cwd(), "src/settings/SettingsDialog.tsx"), "utf8");
assert(source.includes('{ label: "Preferences", sections: ["app", "chat", "appearance"] }'), "Settings should group preferences");
assert(source.includes('{ label: "Workflows", sections: ["models", "workspace"] }'), "Settings should group workflow defaults");
assert(source.includes('{ label: "Data & devices", sections: ["history", "mobile"] }'), "Settings should group data and devices");
assert(source.includes('{ label: "Application", sections: ["system", "about", "developer"] }'), "Settings should group application sections");
assert(source.includes('label: "General"'), "App should be labeled General");
assert(source.includes('label: "Shortcuts"'), "System should be labeled Shortcuts");
assert(source.includes('label: "Data"'), "History should be labeled Data");
assert(source.includes('label: "Models & agents"'), "Models and agents should have a workflow section");
assert(!source.includes("filteredSettingsSections"), "Search should not switch the active section while typing");
assert(!source.includes("settings-status-pill"), "Decorative section status pills should stay removed");

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { SettingsBlock, SettingsChoiceGroup, settingsChoiceNextIndex } = await server.ssrLoadModule("/src/settings/SettingsPrimitives.tsx") as {
    SettingsBlock: ComponentType<{ title?: string; children: string }>;
    SettingsChoiceGroup: ComponentType<{
      value: string;
      options: Array<{ value: string; label: string; detail: string }>;
      onChange: (value: string) => void;
      testIdPrefix: string;
      ariaLabel?: string;
    }>;
    settingsChoiceNextIndex: (key: string, index: number, length: number) => number | null;
  };

  const untitledBlock = renderToStaticMarkup(createElement(SettingsBlock, { children: "Content" }));
  assert(!untitledBlock.includes("settings-block-title"), "Single-group sections should omit a duplicate block title");

  const choiceMarkup = renderToStaticMarkup(createElement(SettingsChoiceGroup, {
    value: "one",
    options: [
      { value: "one", label: "One", detail: "First" },
      { value: "two", label: "Two", detail: "Second" },
    ],
    onChange: () => {},
    testIdPrefix: "test-choice",
    ariaLabel: "Test choice",
  }));
  assert(choiceMarkup.includes('role="radiogroup" aria-label="Test choice"'), "Choice groups should expose a useful accessible name");
  equal(settingsChoiceNextIndex("ArrowRight", 1, 2), 0, "Right arrow should wrap choices");
  equal(settingsChoiceNextIndex("ArrowUp", 0, 2), 1, "Up arrow should wrap choices");
  equal(settingsChoiceNextIndex("Home", 1, 2), 0, "Home should select the first choice");
  equal(settingsChoiceNextIndex("End", 0, 2), 1, "End should select the last choice");
  equal(settingsChoiceNextIndex("Enter", 0, 2), null, "Unrelated keys should not change choices");
} finally {
  await server.close();
}

const { matchingSettingsEntries } = await import("../src/settings/search.js");
equal(matchingSettingsEntries("general")[0]?.id, "app-window-layout", "General search should find window and layout");
equal(matchingSettingsEntries("sound")[0]?.id, "appearance-interface-sounds", "Search should return direct setting results");
equal(matchingSettingsEntries("worktree")[0]?.id, "workspace-new-chat", "Search should find workspace policies");
equal(matchingSettingsEntries("ghost text")[0]?.id, "chat-ai-completion", "Search should find composer completion");
equal(matchingSettingsEntries("ridgeline")[0]?.id, "appearance-empty-chat-ridgeline", "Search should find the empty-chat ridgeline preference");

export {};
