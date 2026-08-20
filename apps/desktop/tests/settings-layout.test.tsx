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
const surfaceSource = readFileSync(resolve(process.cwd(), "src/settings/SettingsSurface.tsx"), "utf8");
const stylesSource = readFileSync(resolve(process.cwd(), "src/settings.css"), "utf8");
assert(source.includes("export function SettingsPage({ onClose }"), "Settings should expose the full-window page contract");
assert(source.includes('testId="settings-page"'), "Settings should expose the page test identifier");
assert(source.includes('backLabel="Back to app"'), "Settings should provide dedicated back navigation");
assert(source.includes("<SettingsSurface"), "Settings should use the shared full-window surface");
assert(!source.includes("<SheetDialog"), "Settings should not render with modal sheet chrome");
assert(surfaceSource.includes("<WindowControls />"), "The settings title bar should retain native window controls");
assert(surfaceSource.includes("startWindowDrag"), "The settings title bar should use guarded native dragging");
assert(surfaceSource.includes('event.key !== "Escape"'), "The settings page should close with Escape");
assert(surfaceSource.includes('hasBackground ? " has-theme-background"'), "The shared settings surface should expose themed background styling");
assert(source.includes('hasBackground={Boolean(activeBackgroundImage)}'), "Settings should reveal the active custom theme background");
assert(source.includes('className="settings-content-inner"'), "Settings should separate the centered content column from its scroll viewport");
assert(/\.settings-content\s*\{[^}]*width: 100%;[^}]*overflow-y: auto;/s.test(stylesSource), "Settings should scroll at the full detail-pane width");
assert(/\.settings-content-inner\s*\{[^}]*width: min\(900px, 100%\);/s.test(stylesSource), "Settings should preserve the existing centered content measure");
assert(source.includes('{ label: "Preferences", sections: ["app", "chat", "appearance", "notifications"] }'), "Settings should group preferences");
assert(source.includes('{ label: "Workflows", sections: ["models", "workspace"] }'), "Settings should group workflow defaults");
assert(source.includes('{ label: "Connections", sections: ["google", "mobile"] }'), "Settings should group external connections");
assert(source.includes('{ label: "Application", sections: ["history", "about", "developer"] }'), "Settings should group application sections");
assert(source.includes('label: "General"'), "App should be labeled General");
assert(source.includes('label: "Notifications"'), "Alerts and sounds should have a dedicated section");
assert(source.includes('label: "Data & privacy"'), "History should expose its privacy scope");
assert(source.includes('label: "About & updates"'), "Update policy and application details should stay together");
assert(source.includes('label: "Google Workspace"'), "Google Workspace should have a dedicated section");
assert(source.includes('label: "Model & agent defaults"'), "The workflow label should match the defaults it contains");
assert(source.includes('data-testid="global-custom-instructions"'), "Model and agent defaults should expose app-wide custom instructions");
assert(source.includes("Applied to every chat run by this Milim desktop, including paired mobile sends."), "Custom instructions should explain their global scope");
assert(source.includes("Workspace AGENTS.md and CLAUDE.md instructions are loaded separately."), "Custom instructions should distinguish workspace rules");
assert(source.includes('title="Browser data"'), "Data settings should expose browser profile controls");
assert(source.includes('activeSection === "google"'), "Google Workspace controls should render in their dedicated section");
assert(source.includes("chooseGoogleFilesFromSettings"), "Google Workspace settings should start the system-browser Picker");
assert(source.includes('className="btn-accent google-workspace-connect-button"'), "Google Workspace connection should be a prominent primary action");
assert(source.includes('new URL("../assets/google.svg", import.meta.url).href'), "Google Workspace settings should use the bundled Google logo");
assert(source.includes('section.id === "google" ? <span className="settings-google-nav-icon"'), "Google Workspace should use the themed Google logo in the settings sidebar");
assert(source.includes('" · Managed folder"'), "Google Workspace settings should identify Milim's managed Drive folder");
assert(source.includes("GOOGLE_CONNECT_DISCLOSURE"), "Fresh Google connections should disclose remote-provider transfer");
assert(source.includes("GOOGLE_REMOVE_MESSAGE"), "Removing a file must use the local-registry-only copy");
assert(source.includes('data-setting-id="system-secret-storage"'), "Data and privacy settings should report credential storage");
assert(!source.includes('activeSection === "system"'), "Sparse System settings should be folded into their owning sections");
assert(source.includes('title="Background" data-setting-id="appearance-background"'), "Appearance should always expose background controls");
assert(source.includes("No custom background"), "Appearance should explain how to add a background when none is active");
assert(source.includes('testIdPrefix="browser-storage"'), "Browser storage should use the accessible choice control");
assert(source.includes('{ value: "open", label: "Open", detail: "Run without approval in trusted workspaces." }'), "Configured chat defaults should offer Open approval");
assert(source.includes('testIdPrefix="sidebar-rail-style"'), "Appearance settings should expose collapsed sidebar rail styles");
assert(source.includes('data-setting-id="app-thread-navigation"'), "General settings should expose thread navigation");
assert(source.includes('testIdPrefix="thread-navigation-placement"'), "Thread navigation should expose placement choices");
assert(source.includes('ariaLabel="Thread navigation placement"'), "Thread navigation placement should expose its structural label");
assert(source.includes('{ value: "sidebar", label: "Sidebar", detail: "Keep projects and threads in the left rail." }'), "Thread navigation should describe Sidebar placement");
assert(source.includes('{ value: "top", label: "Top bar", detail: "Use one horizontal rail below the title bar." }'), "Thread navigation should describe Top bar placement");
assert(source.includes('{ value: "bottom", label: "Bottom bar", detail: "Use one horizontal rail below the chat." }'), "Thread navigation should describe Bottom bar placement");
assert(source.includes('testIdPrefix="sidebar-organization"'), "Sidebar organization should use the accessible choice control");
assert(source.includes('ariaLabel="Thread organization"'), "Thread organization should expose its structural label");
assert(source.includes('{ value: "projects", label: "Projects", detail: "Group threads by project and preserve manual ordering." }'), "Sidebar organization should describe Projects mode");
assert(source.includes('{ value: "inbox", label: "Inbox", detail: "Sort active threads by recent activity and fold settled threads into the footer." }'), "Sidebar organization should describe Inbox mode");
assert(!source.includes('testId="settled-threads-toggle"'), "Appearance should no longer own Inbox organization");
assert(source.includes('data-testid="browser-data-clear"'), "Browser data should have an explicit clear action");
assert(source.includes('data-testid="developer-show-update-cards"'), "Developer settings should preview update cards");
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
  const threeChoiceMarkup = renderToStaticMarkup(createElement(SettingsChoiceGroup, {
    value: "one",
    options: [
      { value: "one", label: "One", detail: "First" },
      { value: "two", label: "Two", detail: "Second" },
      { value: "three", label: "Three", detail: "Third" },
    ],
    onChange: () => {},
    testIdPrefix: "three-choice",
  }));
  assert(threeChoiceMarkup.includes('class="settings-choice-grid three-up"'), "Three-choice groups should use a balanced three-column layout");
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
equal(matchingSettingsEntries("split rail")[0]?.id, "appearance-sidebar-colors", "Search should find collapsed sidebar rail styles");
equal(matchingSettingsEntries("inbox")[0]?.id, "app-thread-navigation", "Search should find Inbox organization under General");
equal(matchingSettingsEntries("settled")[0]?.id, "app-thread-navigation", "Search should find settled threads under General");
equal(matchingSettingsEntries("top bar")[0]?.id, "app-thread-navigation", "Search should find horizontal navigation placement");
equal(matchingSettingsEntries("sidebar organization")[0]?.id, "app-thread-navigation", "Search should find the structural sidebar choice");
equal(matchingSettingsEntries("thread colors")[0]?.id, "appearance-sidebar-colors", "Search should find automatic project thread colors");
equal(matchingSettingsEntries("custom instructions")[0]?.id, "models-defaults", "Search should find global custom instructions");
equal(matchingSettingsEntries("cookies")[0]?.id, "browser-data", "Search should find browser data controls");
equal(matchingSettingsEntries("sheets")[0]?.id, "google-workspace", "Search should find Google Workspace controls");
equal(matchingSettingsEntries("keychain")[0]?.id, "system-secret-storage", "Search should find credential storage");
equal(matchingSettingsEntries("keychain")[0]?.section, "history", "Credential storage should open Data & privacy");
equal(matchingSettingsEntries("sound")[0]?.section, "notifications", "Sound search should open Notifications");

export {};
