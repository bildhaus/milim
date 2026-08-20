import { strict as assert } from "node:assert";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const originalFetch = globalThis.fetch;
const originalWindow = globalThis.window;
const seen = [];
let modelReads = 0;
let refreshCalls = 0;
let codexReads = 0;
let phase = "slow-account";

globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke: async (command) => {
      if (command === "api_base_url") return "http://127.0.0.1:7377";
      if (command === "api_token") return "";
      if (command === "refresh_provider_models") {
        refreshCalls += 1;
        return true;
      }
      throw new Error(`Unexpected Tauri command: ${command}`);
    },
  },
};

globalThis.fetch = async (input, init) => {
  const url = String(input);
  if (url.endsWith("/v1/models")) {
    modelReads += 1;
    if (phase === "failed-refresh") {
      return new Response("provider unavailable", { status: 503 });
    }
    return Response.json({
      data: [
        {
          id: modelReads === 1 ? "cached-model" : "refreshed-model",
          owned_by: "Test Provider",
        },
      ],
    });
  }
  if (url.endsWith("/codex/account")) {
    codexReads += 1;
    if (codexReads > 1) {
      return Response.json({ requiresOpenaiAuth: true, account: null });
    }
    return await new Promise((_, reject) => {
      init?.signal?.addEventListener(
        "abort",
        () => reject(new DOMException("Aborted", "AbortError")),
        { once: true },
      );
    });
  }
  if (url.endsWith("/claude/status")) {
    return Response.json({ available: false, authenticated: false, models: [] });
  }
  if (url.endsWith("/pi/status")) {
    return Response.json({ available: false, authenticated: false, models: [] });
  }
  return new Response("not found", { status: 404 });
};

const server = await createServer({
  root,
  logLevel: "silent",
  server: { middlewareMode: true },
  appType: "custom",
});

try {
  const { loadStartupModels } = await server.ssrLoadModule("/src/api.ts");
  const startedAt = performance.now();
  let cachedAt;
  let resolveCached;
  const cachedVisible = new Promise((resolve) => {
    resolveCached = resolve;
  });
  const loading = loadStartupModels(
    (models) => {
      const ids = models.map((model) => model.id);
      seen.push(ids);
      if (ids.includes("cached-model") && cachedAt === undefined) {
        cachedAt = performance.now() - startedAt;
        resolveCached();
      }
    },
    { codex: true, claude: false, opencode: false, pi: false },
    [{ id: "codex:existing", owned_by: "Codex" }],
  );

  await Promise.race([
    cachedVisible,
    new Promise((_, reject) =>
      setTimeout(
        () =>
          reject(
            new Error(
              "Cached provider models were not visible within one second.",
            ),
          ),
        1_000,
      ),
    ),
  ]);
  assert.ok(cachedAt < 1_000, `cached models took ${cachedAt}ms`);
  await loading;

  assert.equal(refreshCalls, 1);
  assert.equal(modelReads, 2);
  assert.equal(codexReads, 1);
  assert.ok(
    performance.now() - startedAt >= 8_000,
    "The account probe should remain isolated while its full configured timeout elapses.",
  );
  assert.ok(
    seen.every((ids) => ids.includes("codex:existing")),
    "A slow or failed account lane must not clear an existing usable model.",
  );
  assert.ok(
    seen.some((ids) => ids.includes("cached-model")),
    "Cached provider models should render before account discovery finishes.",
  );
  assert.deepEqual(seen.at(-1), ["refreshed-model", "codex:existing"]);

  phase = "failed-refresh";
  const retained = [];
  await loadStartupModels(
    (models) => retained.push(models.map((model) => model.id)),
    { codex: false, claude: false, opencode: false, pi: false },
    [{ id: "provider-seeded", owned_by: "Test Provider" }],
  );
  assert.ok(
    retained.length > 0 &&
      retained.every((ids) => ids.includes("provider-seeded")),
    "Failed provider reads must preserve the last usable seeded catalog.",
  );
} finally {
  globalThis.fetch = originalFetch;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
  await server.close();
}
