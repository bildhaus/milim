import { strict as assert } from "node:assert";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const originalFetch = globalThis.fetch;
const requests = [];
const server = await createServer({
  root,
  logLevel: "silent",
  server: { middlewareMode: true },
  appType: "custom",
});

globalThis.fetch = async (input) => {
  const url = String(input);
  requests.push(url);
  if (url.endsWith("/v1/models")) throw new Error("provider list unavailable");
  if (url.endsWith("/codex/account")) {
    return Response.json({
      requiresOpenaiAuth: false,
      account: { type: "chatgpt" },
    });
  }
  if (url.endsWith("/codex/models")) {
    return Response.json({
      data: [{ model: "gpt-test", inputModalities: ["text"] }],
    });
  }
  if (url.endsWith("/claude/status")) {
    return Response.json({
      available: true,
      authenticated: true,
      models: ["sonnet"],
    });
  }
  if (url.endsWith("/pi/status")) {
    return Response.json({
      available: true,
      authenticated: true,
      provider_count: 1,
      models: [{
        id: "openai-codex/gpt-pi-test",
        provider: "openai-codex",
        model_id: "gpt-pi-test",
        name: "GPT Pi Test",
        reasoning: true,
        image_input: true,
      }],
    });
  }
  return new Response("not found", { status: 404 });
};

try {
  const { listModelsDetailed } = await server.ssrLoadModule("/src/api.ts");
  const models = await listModelsDetailed();

  assert.deepEqual(
    models.map((model) => model.id),
    ["codex:gpt-test", "claude:sonnet", "pi:openai-codex/gpt-pi-test"],
  );
  assert(requests.some((url) => url.endsWith("/v1/models")));
  assert(requests.some((url) => url.endsWith("/codex/models")));
  assert(requests.some((url) => url.endsWith("/claude/status")));
  assert(requests.some((url) => url.endsWith("/pi/status")));

  requests.length = 0;
  const withoutAccountRuntimes = await listModelsDetailed({
    codex: false,
    claude: false,
    opencode: false,
    pi: false,
  });
  assert.deepEqual(withoutAccountRuntimes, []);
  assert(requests.some((url) => url.endsWith("/v1/models")));
  assert(!requests.some((url) => /\/(codex|claude|opencode|pi)\//.test(url)));
} finally {
  globalThis.fetch = originalFetch;
  await server.close();
}
