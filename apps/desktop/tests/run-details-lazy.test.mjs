import { strict as assert } from "node:assert";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

const originalFetch = globalThis.fetch;
const originalWindow = globalThis.window;
const requests = [];

globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke: async (command) => {
      if (command === "api_base_url") return "http://127.0.0.1:7377";
      if (command === "api_token") return "fixture-token";
      throw new Error(`Unexpected Tauri command: ${command}`);
    },
  },
};
globalThis.fetch = async (input, init) => {
  const url = String(input);
  requests.push({ url, authorization: new Headers(init?.headers).get("Authorization") });
  if (url.endsWith("/control/v1/runs/run%2Ffixture")) {
    return Response.json({
      run: {
        id: "run/fixture",
        thread_id: "thread-1",
        status: "completed",
        adapter: "provider",
        config: { model: "fixture" },
        capabilities: { ledger: true, inspectable: true, steering: true, visibility: "model_visible" },
        created_at_ms: 1,
        updated_at_ms: 2,
        completed_at_ms: 2,
        error: null,
      },
      composition: null,
    });
  }
  if (url.endsWith("/control/v1/runs/run%2Ffixture/events?limit=100")) {
    return Response.json({
      run_id: "run/fixture",
      after_seq: null,
      next_seq: null,
      has_more: false,
      events: [],
    });
  }
  return new Response("not found", { status: 404 });
};

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { AssistantMessage } = await server.ssrLoadModule("/src/components/AssistantMessage.tsx");
  const { getControlRunDetails } = await server.ssrLoadModule("/src/api.ts");
  const parts = [{ kind: "event", eventType: "tool", label: "read_file", status: "done" }];
  renderToStaticMarkup(createElement(AssistantMessage, { content: "", streamParts: parts }));
  renderToStaticMarkup(createElement(AssistantMessage, {
    content: "",
    streamParts: parts,
    runDetailsRunId: "run/fixture",
  }));
  assert.equal(requests.length, 0, "ordinary and closed work rendering must not request ledger data");

  const details = await getControlRunDetails("run/fixture");
  assert.equal(details.inspection.run.id, "run/fixture");
  assert.equal(requests.length, 2, "one explicit details load should make only its two lazy requests");
  assert.equal(
    requests.filter(({ url }) => url.includes("/events?")).length,
    1,
    "opening run details should request exactly one bounded event page",
  );
  assert(requests.every(({ authorization }) => authorization === "Bearer fixture-token"));
} finally {
  globalThis.fetch = originalFetch;
  globalThis.window = originalWindow;
  await server.close();
}
