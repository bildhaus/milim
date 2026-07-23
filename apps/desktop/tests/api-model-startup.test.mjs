import { strict as assert } from "node:assert";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const originalFetch = globalThis.fetch;
const originalWindow = globalThis.window;
const seen = [];
let modelReads = 0;
let refreshCalls = 0;

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

globalThis.fetch = async (input) => {
  const url = String(input);
  if (url.endsWith("/v1/models")) {
    modelReads += 1;
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
    return Response.json({ requiresOpenaiAuth: true, account: null });
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
  await loadStartupModels((models) => {
    seen.push(models.map((model) => model.id));
  });

  assert.equal(refreshCalls, 1);
  assert.equal(modelReads, 2);
  assert.deepEqual(seen, [["cached-model"], ["refreshed-model"]]);
} finally {
  globalThis.fetch = originalFetch;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
  await server.close();
}
