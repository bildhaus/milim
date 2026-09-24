import { createElement, type ComponentType, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type {
  AccountProfileList,
  AccountRuntimeKind,
  AccountRuntimeUpdateStatus,
  ProviderInfo,
} from "../src/api.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function count(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

/** Visible text only, so attribute values such as titles do not count. */
function text(markup: string): string {
  return markup.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ");
}

const provider = (overrides: Partial<ProviderInfo> & Pick<ProviderInfo, "id" | "name" | "base_url">): ProviderInfo => ({
  kind: "openai_compatible",
  enabled: true,
  has_key: true,
  models: [],
  ...overrides,
});

const providers: ProviderInfo[] = [
  provider({ id: "gemini", name: "Gemini", kind: "gemini", base_url: "https://generativelanguage.googleapis.com/v1beta", models: ["gemini-2.5-pro"] }),
  provider({ id: "openrouter", name: "OpenRouter", base_url: "https://openrouter.ai/api/v1", models: ["openai/gpt-5"] }),
  provider({ id: "ollama", name: "Ollama (local)", base_url: "http://localhost:11434/v1", has_key: false, models: ["qwen3:8b"] }),
  provider({ id: "replicate", name: "Replicate", kind: "replicate", base_url: "https://api.replicate.com/v1" }),
  provider({ id: "openai", name: "OpenAI", base_url: "https://api.openai.com/v1", has_key: false, enabled: false }),
];

// ContextMenuProvider positions its menu in a layout effect, which React
// reports as a no-op during server rendering.
const consoleError = console.error;
console.error = (...args: unknown[]) => {
  if (String(args[0]).includes("useLayoutEffect does nothing on the server")) return;
  consoleError(...args);
};

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const connections = (await server.ssrLoadModule("/src/lib/providerConnections.ts")) as typeof import("../src/lib/providerConnections.js");
  const { ContextMenuProvider } = (await server.ssrLoadModule("/src/components/ContextMenu.tsx")) as {
    ContextMenuProvider: ComponentType<{ children: ReactNode }>;
  };
  const { ProvidersRail } = (await server.ssrLoadModule("/src/components/ProvidersRail.tsx")) as typeof import("../src/components/ProvidersRail.js");
  const { ProviderOverview } = (await server.ssrLoadModule("/src/components/ProviderOverview.tsx")) as typeof import("../src/components/ProviderOverview.js");
  const { RuntimePage } = (await server.ssrLoadModule("/src/components/RuntimePage.tsx")) as typeof import("../src/components/RuntimePage.js");
  const { RuntimeAccounts } = (await server.ssrLoadModule("/src/components/RuntimeAccounts.tsx")) as typeof import("../src/components/RuntimeAccounts.js");
  type State = import("../src/components/useAccountRuntimes.js").AccountRuntimesState;

  const render = (node: ReactNode) =>
    renderToStaticMarkup(createElement(ContextMenuProvider, null, node));

  const claudeAccounts: AccountProfileList = {
    runtime: "claude",
    auto_selection: "lw",
    profiles: [
      { id: "default", runtime: "claude", label: "Default", is_default: true, enabled: true, priority: 0 },
      {
        id: "lw",
        runtime: "claude",
        label: "LW",
        config_dir: "/Users/you/.milim/account-profiles/claude/lw",
        is_default: false,
        enabled: true,
        priority: 1,
        short_window_percent: 42,
      },
    ],
  };
  const codexAccounts: AccountProfileList = {
    runtime: "codex",
    auto_selection: "default",
    profiles: [{ id: "default", runtime: "codex", label: "Default", is_default: true, enabled: true, priority: 0 }],
  };

  function makeState(options: {
    installed?: Partial<Record<AccountRuntimeKind, boolean>>;
    ready?: Partial<Record<AccountRuntimeKind, boolean>>;
    updates?: Partial<Record<AccountRuntimeKind, AccountRuntimeUpdateStatus>>;
    enabled?: Partial<Record<AccountRuntimeKind, boolean>>;
  }): State {
    const enabled = { codex: true, claude: true, opencode: true, pi: true, ...options.enabled };
    const ready = { codex: true, claude: true, opencode: true, pi: false, ...options.ready };
    const installed = { codex: true, claude: true, opencode: true, pi: true, ...options.installed };
    const updates = options.updates ?? {};
    const profiles = { codex: codexAccounts, claude: claudeAccounts };
    const summaries = Object.fromEntries(
      connections.ACCOUNT_RUNTIME_KINDS.map((runtime) => [
        runtime,
        connections.summarizeRuntime({
          runtime,
          enabled: enabled[runtime],
          checked: true,
          installed: installed[runtime],
          ready: enabled[runtime] && ready[runtime],
          update: updates[runtime],
          accountCount: runtime === "codex" || runtime === "claude" ? profiles[runtime].profiles.length : undefined,
        }),
      ]),
    );
    const noop = async () => {};
    return {
      enabled,
      codexAccount: { requiresOpenaiAuth: true, account: { type: "chatgpt", email: "you@example.com", planType: "plus" } },
      claudeStatus: {
        available: installed.claude,
        authenticated: ready.claude,
        auth: { email: "you@example.com", subscriptionType: "max" },
        models: ["claude-opus", "claude-sonnet"],
      },
      openCodeStatus: { available: installed.opencode, authenticated: ready.opencode, models: ["a", "b"] },
      piStatus: { available: installed.pi, authenticated: ready.pi, provider_count: 0, models: [] },
      busy: {},
      notes: {},
      binaries: {},
      updates,
      updatesLoaded: true,
      updateTargets: connections.runtimeUpdateTargets(enabled, updates),
      confirmUpdate: null,
      updating: null,
      updateProgress: null,
      profiles,
      profileErrors: {},
      ready,
      summaries,
      missing: (runtime: AccountRuntimeKind) => !installed[runtime],
      refreshStatus: noop,
      refreshRuntime: noop,
      refreshProfiles: noop,
      binaryChanged: () => {},
      setEnabled: () => {},
      runUpdate: noop,
      runAllUpdates: noop,
      connectCodex: noop,
      disconnectCodex: noop,
    } as unknown as State;
  }

  const behind = (version: string, latest: string): AccountRuntimeUpdateStatus => ({
    available: true,
    version,
    latest_version: latest,
    update_available: true,
  });
  const current = (version: string): AccountRuntimeUpdateStatus => ({
    available: true,
    version,
    latest_version: version,
    update_available: false,
  });

  // ----- Rail grouping -----
  const railState = makeState({ installed: { pi: false }, updates: { codex: behind("0.150.0", "0.156.1") } });
  const rail = render(createElement(ProvidersRail, {
    target: { view: "runtime", runtime: "claude" },
    runtimes: railState.summaries,
    providers,
    onSelect: () => {},
  }));
  const railText = text(rail);
  const order = ["Overview", "Coding CLIs", "Codex", "Claude", "OpenCode", "Pi", "Hosted", "Gemini", "OpenRouter", "OpenAI", "Local", "Ollama (local)", "Media", "Replicate"];
  let cursor = -1;
  for (const label of order) {
    const index = railText.indexOf(` ${label} `, cursor + 1);
    assert(index > cursor, `rail should list ${label} after the previous item (Overview, Coding CLIs, Hosted, Local, Media)`);
    cursor = index;
  }
  assert(rail.includes('data-testid="provider-rail-pi"') && railText.includes("Not installed"), "rail should show every coding CLI, muting ones that are not installed");
  assert(/providers-rail-row muted"[^>]*data-testid="provider-rail-pi"/.test(rail), "a missing CLI should render muted");
  assert(railText.includes("+ media"), "a chat provider that also generates media should sit under Hosted with a media hint");
  assert(railText.includes(" Update "), "a runtime with an update should carry a short Update hint");
  assert(railText.includes("2 accounts"), "a runtime with several accounts should say so");
  assert(railText.includes("Disabled"), "a disabled provider should say Disabled");
  const railVisibleText = text(rail.replace(/<span class="providers-sr-only">[^<]*<\/span>/g, ""));
  assert(!railVisibleText.includes("Key saved") && !railVisibleText.includes("Connected"), "rail rows should not carry key or connection chips");
  assert(count(rail, 'aria-current="page"') === 1 && /aria-current="page"[^>]*data-testid="provider-rail-claude"/.test(rail), "only the selected rail row should be aria-current");
  assert(rail.includes('class="providers-sr-only">Ready<'), "status dots should carry accessible text");
  assert(connections.providerRailGroup(providers[1]) === "hosted", "OpenRouter should group under Hosted");
  assert(connections.shortenHomePath("C:\\Users\\you\\.milim\\x") === "~\\.milim\\x", "Windows home paths should shorten too");

  // ----- One primary action per view -----
  const overviewState = makeState({
    updates: {
      codex: behind("0.150.0", "0.156.1"),
      opencode: behind("1.18.0", "1.18.32"),
      pi: behind("0.80.0", "0.87.1"),
      claude: current("2.1.3"),
    },
    ready: { pi: true },
  });
  const overview = render(createElement(ProviderOverview, {
    state: overviewState,
    providers,
    detecting: false,
    discoveries: [],
    busy: false,
    note: null,
    onSelect: () => {},
    onStartPreset: () => {},
    onDetectLocal: () => {},
    onAddDiscovery: () => {},
  }));
  assert(count(overview, "btn-accent") === 1, "Overview should show exactly one primary button");
  assert(overview.includes('data-testid="account-runtimes-update-all"') && text(overview).includes("3 coding CLI updates available"), "the attention bar should hold Update all");
  assert((overview.match(/class="providers-attention[ "]/g) ?? []).length === 1, "Overview should show at most one attention bar");
  for (const id of ["codex", "claude", "opencode", "pi", "provider-gemini", "provider-replicate"]) {
    assert(overview.includes(`data-testid="provider-overview-${id}"`), `Overview should list ${id}`);
  }
  assert(overview.includes('data-testid="detect-local-providers"'), "Overview should keep local detection");

  const updatePage = render(createElement(RuntimePage, {
    runtime: "codex",
    state: overviewState,
    onImportChats: () => {},
  }));
  assert(!updatePage.includes("btn-accent"), "a runtime page should not use the solid primary button");
  assert(count(updatePage, "providers-emphasis-button") === 1, "a runtime page should emphasize at most one action");
  assert(/data-testid="codex-update"[^>]*>Update to v0\.156\.1</.test(updatePage), "an available update should be the one emphasized action");
  assert(updatePage.includes('data-testid="codex-actions"') && updatePage.includes('aria-haspopup="menu"'), "remaining actions should live in the overflow menu");
  assert(!text(updatePage).includes("Locate binary"), "a found CLI should not show Locate binary inline");
  assert(updatePage.includes('data-testid="codex-enabled-toggle"'), "the Enabled switch should stay in the header");

  const currentPage = render(createElement(RuntimePage, {
    runtime: "claude",
    state: overviewState,
    onImportChats: () => {},
  }));
  assert(!currentPage.includes("providers-emphasis-button"), "a ready, current runtime needs no emphasized action");
  assert(text(currentPage).includes("Up to date"), "a current runtime should say Up to date quietly");

  const missingState = makeState({ installed: { pi: false } });
  const missingPage = render(createElement(RuntimePage, {
    runtime: "pi",
    state: missingState,
    onImportChats: () => {},
  }));
  assert(text(missingPage).includes("Not installed"), "a missing CLI should say Not installed");
  assert(missingPage.includes('data-testid="pi-locate-binary"') && missingPage.includes("npm install -g"), "a missing CLI should show the install hint and Locate binary inline");
  assert(!missingPage.includes("providers-emphasis-button"), "a missing CLI should not add another emphasized action");

  // ----- Accounts -----
  const accounts = render(createElement(RuntimeAccounts, {
    runtime: "claude",
    list: claudeAccounts,
    identities: { default: "you@example.com" },
    onReload: async () => {},
  }));
  const accountsText = text(accounts);
  assert(count(accounts, 'class="runtime-account-row') === 2, "every Claude account should render as one row");
  assert(accountsText.includes("Default") && accountsText.includes("you@example.com"), "the default account should show its name and identity");
  assert(count(accounts, ">Default<") === 1, "an account already named Default should not repeat a Default badge");
  assert(/data-testid="claude-account-lw"[\s\S]*Auto picks/.test(accounts), "Auto picks should mark the account Auto would choose");
  assert(count(accounts, "Auto picks") === 1, "only one account should be marked Auto picks");
  assert(accountsText.includes("~/.milim/account-profiles/claude/lw") && !accountsText.includes("/Users/you"), "account paths should be shortened in visible text");
  assert(accounts.includes('title="/Users/you/.milim/account-profiles/claude/lw"'), "the full path should stay in the tooltip");
  assert(count(accounts, ">Use in Auto<") === 2, "every account should have a labelled Use in Auto switch");
  assert(/data-testid="claude-account-default-auto"[^>]*disabled/.test(accounts) || /disabled=""[^>]*data-testid="claude-account-default-auto"/.test(accounts), "the default account should always stay in Auto");
  assert(accounts.includes('data-testid="claude-account-lw-menu"') && !accounts.includes('data-testid="claude-account-default-menu"'), "only removable accounts get a row menu");
  assert(accounts.includes('data-testid="claude-add-account"') && accountsText.includes("Add account"), "Add account should close the list");
  assert(!accounts.includes("btn-accent"), "the accounts list should not add a primary button");

  const codexAccountsMarkup = render(createElement(RuntimeAccounts, {
    runtime: "codex",
    list: codexAccounts,
    onReload: async () => {},
  }));
  assert(count(codexAccountsMarkup, 'class="runtime-account-row') === 1 && text(codexAccountsMarkup).includes("Default"), "Codex should list its default account with the same layout");
  assert(!codexAccountsMarkup.includes("Auto picks"), "a single account needs no Auto picks marker");
} finally {
  await server.close();
}
