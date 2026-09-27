// Control-API smoke test: drives a running milim host through /control/v1
// with the built-in `mock-echo` adapter, so no model backend is needed.
//
//   MILIM_CONTROL_URL   base URL of the desktop or mobile/control listener
//   MILIM_DEVICE_KEY    paired-device bearer key (mobile/control listener), or
//   MILIM_API_TOKEN     desktop bearer token (main listener); both optional
//                       for a loopback-trusted test server
//   MILIM_E2E_CONTROL_FILE  instead of the three above: the file a debug
//                       desktop build writes `{api_url, token}` to when it is
//                       launched with the same variable; waited for until the
//                       timeout
//   MILIM_SMOKE_TIMEOUT_MS  per-run timeout (default 30000)
//
// Credentials are sent only as Authorization headers and never printed.

import { resolveControlCredentials, retryUntilReady } from "./lib/control.mjs";

const timeoutMs = Number(process.env.MILIM_SMOKE_TIMEOUT_MS || 30_000);
const nonce = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
const terminal = new Set(["completed", "failed", "cancelled", "interrupted"]);
let base = "";
let credential = "";

try {
  const resolved = await resolveControlCredentials({ timeoutMs });
  ({ base, credential } = resolved);
  if (resolved.source === "control-file") step("read the desktop control file");
  if (!base) fail("MILIM_CONTROL_URL or MILIM_E2E_CONTROL_FILE is required.");
  await main();
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}

async function main() {
  // A freshly launched host may not accept connections yet.
  const bootstrap = await retryUntilReady(() => request("GET", "/control/v1/bootstrap"), {
    timeoutMs,
    isConnectionError: (error) => / failed: /.test(String(error?.message)),
  });
  if (bootstrap?.protocol?.min == null || !Array.isArray(bootstrap.threads)) {
    throw new Error("bootstrap is missing protocol or threads");
  }
  step(`bootstrap ok: host ${bootstrap.host_name ?? bootstrap.host_id ?? "unknown"}`);

  const threadId = `smoke-${nonce}`;
  const created = await command({
    command_id: `smoke-create-${nonce}`,
    kind: "thread.create",
    payload: {
      id: threadId,
      title: "Control smoke",
      settings: { model: "mock-echo", privacy: "off", memory: false },
    },
  });
  if (created.status !== "applied") {
    throw new Error(`thread.create returned ${created.status}: ${created.message ?? ""}`);
  }
  step(`thread.create ok: ${threadId}`);

  const text = `milim smoke ${nonce}`;
  const sent = await command({
    command_id: `smoke-send-${nonce}`,
    kind: "turn.send",
    thread_id: threadId,
    payload: { text, attachments: [] },
  });
  if (sent.status !== "accepted" || !sent.run_id) {
    throw new Error(`turn.send returned ${sent.status}: ${sent.message ?? ""}`);
  }
  step(`turn.send ok: run ${sent.run_id}`);

  const run = await waitForRun(sent.run_id);
  if (run.status !== "completed") {
    throw new Error(`run ${sent.run_id} ended ${run.status}: ${JSON.stringify(run.error ?? null)}`);
  }
  step("run completed");

  const timeline = await request(
    "GET",
    `/control/v1/threads/${encodeURIComponent(threadId)}/timeline?tail=50`,
  );
  const expected = `Echo: ${text}`;
  const reply = (timeline.items ?? []).find(
    (item) => item?.data?.role === "assistant" && item.data.content === expected,
  );
  if (!reply) throw new Error(`timeline has no assistant message "${expected}"`);
  step("timeline has the echo reply");

  const events = await request(
    "GET",
    `/control/v1/runs/${encodeURIComponent(sent.run_id)}/events?limit=50`,
  );
  if (!Array.isArray(events.events)) throw new Error("run events page has no events array");
  step(`run events readable: ${events.events.length} event(s)`);

  const archived = await command({
    command_id: `smoke-archive-${nonce}`,
    kind: "thread.archive",
    thread_id: threadId,
    payload: { archived: true },
  });
  if (archived.status !== "applied") {
    step(`warning: thread.archive returned ${archived.status}`);
  }
  console.log("control smoke passed");
}

async function waitForRun(runId) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const inspection = await request("GET", `/control/v1/runs/${encodeURIComponent(runId)}`);
    const run = inspection?.run;
    if (run && terminal.has(run.status)) return run;
    if (Date.now() > deadline) {
      throw new Error(`timed out after ${timeoutMs} ms waiting for run ${runId} (last status ${run?.status})`);
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
}

function command(body) {
  return request("POST", "/control/v1/commands", body);
}

async function request(method, path, body) {
  const headers = { Accept: "application/json" };
  if (credential) headers.Authorization = `Bearer ${credential}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  let response;
  try {
    response = await fetch(`${base}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(15_000),
    });
  } catch (error) {
    throw new Error(`${method} ${path} failed: ${error instanceof Error ? error.message : error}`);
  }
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`${method} ${path} returned ${response.status}: ${text.slice(0, 300)}`);
  }
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`${method} ${path} returned non-JSON: ${text.slice(0, 200)}`);
  }
}

function step(message) {
  console.log(`[smoke] ${message}`);
}

function fail(message) {
  console.error(`control smoke failed: ${message}`);
  process.exit(1);
}
