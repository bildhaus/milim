import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  // Finish lazy Markdown compilation before rendering can race Vite teardown.
  await server.ssrLoadModule("/src/components/Markdown.tsx");
  const api = (await server.ssrLoadModule("/src/api.ts")) as {
    agentAvatarSeed: (agent: { id?: string; name?: string; avatar?: string }) => string;
    parseAgentDraftResponse: (text: string) => { avatar: string };
  };
  const { AgentAvatar } = (await server.ssrLoadModule("/src/components/AgentAvatar.tsx")) as {
    AgentAvatar: ComponentType<{ id?: string; name?: string; avatar?: string; className?: string }>;
  };
  const { WorkersInspector, WorkersSummary } = (await server.ssrLoadModule("/src/components/WorkersInspector.tsx")) as {
    WorkersInspector: ComponentType<any>;
    WorkersSummary: ComponentType<any>;
  };
  const { Select } = (await server.ssrLoadModule("/src/components/ui.tsx")) as {
    Select: ComponentType<any>;
  };

  equal(api.agentAvatarSeed({ name: "Researcher", avatar: "field-notes" }), "field-notes", "Explicit seeds should win");
  equal(api.agentAvatarSeed({ name: "Security", avatar: "🛡️" }), "🛡️", "Existing emoji should remain a compatible seed");
  equal(api.agentAvatarSeed({ name: "Researcher", avatar: "" }), "Researcher", "Blank seeds should follow the name");
  equal(api.agentAvatarSeed({ name: "Researcher", avatar: "/images/legacy.png" }), "Researcher", "Legacy images should follow the name");
  equal(api.agentAvatarSeed({ id: "agent-123" }), "agent-123", "Nameless records should fall back to their persisted ID");
  equal(api.agentAvatarSeed({}), "", "Seed resolution should never use a shared generic fallback");

  const draft = api.parseAgentDraftResponse(JSON.stringify({
    name: "Research Scout",
    avatar: "current-primary-sources",
    system_prompt: "Find current primary sources.",
  }));
  equal(draft.avatar, "current-primary-sources", "Generated seeds should not be truncated to initials");

  const props = { name: "Unsafe", avatar: '\"><script>alert(1)</script>' };
  const first = renderToStaticMarkup(createElement(AgentAvatar, props));
  const second = renderToStaticMarkup(createElement(AgentAvatar, props));
  equal(first, second, "Avatar element markup should be deterministic");
  assert(first.startsWith("<shatz-avatar"), "Agent avatars should use the shared custom element");
  assert(first.includes('shape="circle"'), "Agent avatars should use the circle recipe");
  assert(first.includes('secondary-color="'), "Agent avatars should set the seed-derived secondary color");
  assert(first.includes('background="'), "Agent avatars should set the seed-derived background color");
  assert(first.includes('aria-hidden="true"'), "Agent avatars paired with visible names should be decorative");
  assert(!first.includes("<script>"), "Seed text must remain escaped in markup");

  const coral = renderToStaticMarkup(createElement(AgentAvatar, { name: "Coral", avatar: "CR" }));
  const violet = renderToStaticMarkup(createElement(AgentAvatar, { name: "Violet", avatar: "WR" }));
  assert(coral !== violet, "Different seeds should produce varied avatar markup");
  assert(
    coral.match(/ color="([^"]+)"/)?.[1] !== violet.match(/ color="([^"]+)"/)?.[1],
    "Different seeds should select visibly different palettes",
  );

  const agent = {
    id: "agent-security",
    name: "Security Review",
    avatar: "security-seed",
    system_prompt: "Review security.",
    model: "",
    tool_mode: "none",
    enabled_tools: [],
    skill_mode: "none",
    enabled_skills: [],
  };
  const selectMarkup = renderToStaticMarkup(createElement(Select, {
    value: agent.id,
    onChange: () => {},
    options: [{ value: agent.id, label: agent.name, leading: createElement(AgentAvatar, agent) }],
  }));
  assert(selectMarkup.includes('data-avatar-seed="security-seed"'), "Agent selectors should render the shared avatar");

  const inspectorProps = {
    policy: "ask",
    workerModel: "",
    models: [],
    providers: [],
    agents: [agent],
    settingsOpen: false,
    onSettingsOpenChange: () => {},
    onPolicyChange: () => {},
    onWorkerModelChange: () => {},
    onStart: () => {},
    onStop: () => {},
    onContinueSolo: () => {},
    onStopWorker: async () => {},
    onLoadDiff: async () => ({ worker_id: "task-1", status: {}, diff: "" }),
    onApplyDiff: async () => ({ message: "" }),
    onClose: () => {},
  };
  const tasks = [
    { id: "task-a", title: "Inspect API", prompt: "Inspect the API.", model: "test", access: "read_only" },
    { id: "task-b", title: "Inspect UI", prompt: "Inspect the UI.", model: "test", access: "read_only" },
  ];
  const proposedRun = {
    id: "run-proposed",
    parent_thread_id: "thread-1",
    policy: "ask",
    runtime: "managed",
    status: "proposed",
    tasks,
    created_at: "2026-07-13T00:00:00Z",
    updated_at: "2026-07-13T00:00:00Z",
  };
  const proposedMarkup = renderToStaticMarkup(createElement(WorkersInspector, {
    ...inspectorProps,
    agents: [],
    records: [{ run: proposedRun, workers: [] }],
  }));
  assert(proposedMarkup.includes('data-avatar-seed="worker:run-proposed:task-a"'), "Unassigned proposed tasks should receive deterministic avatars");
  assert(proposedMarkup.includes('data-avatar-seed="worker:run-proposed:task-b"'), "Different proposed tasks should receive different avatars");

  const assignedMarkup = renderToStaticMarkup(createElement(WorkersInspector, {
    ...inspectorProps,
    records: [{
      run: { ...proposedRun, id: "run-assigned", tasks: [{ ...tasks[0], agent_id: agent.id }] },
      workers: [],
    }],
  }));
  assert(assignedMarkup.includes('data-avatar-seed="security-seed"'), "Assigned Worker tasks should render their Agent avatar");

  for (const policy of ["off", "ask", "auto"] as const) {
    const emptyMarkup = renderToStaticMarkup(createElement(WorkersSummary, {
      records: [],
      policy,
      workerModel: "",
      models: [],
      agents: [],
      onOpen: () => {},
      onOpenSettings: () => {},
    }));
    const policyLabel = policy[0].toUpperCase() + policy.slice(1);
    assert(emptyMarkup.includes(`${policyLabel} · Inherit parent`), `${policy} should appear in the compact summary`);
    assert(emptyMarkup.includes("No runs yet"), `${policy} should render the compact idle state`);
    assert(emptyMarkup.includes('data-testid="workers-settings-toggle"'), "Worker summary should open settings");
  }

  const runningRecord = {
    run: { ...proposedRun, status: "running" },
    workers: [{
      id: "worker-live",
      parent_id: "thread-1",
      root_id: "thread-1",
      title: "Inspect UI",
      status: "running",
      model: "test",
      prompt: "Inspect the UI.",
      created_at: "2026-07-13T00:00:00Z",
      updated_at: "2026-07-13T00:01:00Z",
      runtime: "managed",
      access: "read_only",
    }],
  };
  const doneRecord = {
    run: { ...proposedRun, id: "run-done", status: "done" },
    workers: [{
      id: "worker-done",
      parent_id: "thread-1",
      root_id: "thread-1",
      title: "Inspect API",
      status: "done",
      model: "test",
      prompt: "Inspect the API.",
      summary: "API inspection complete.",
      created_at: "2026-07-13T00:00:00Z",
      updated_at: "2026-07-13T00:01:00Z",
      runtime: "managed",
      access: "read_only",
    }],
  };
  const historyMarkup = renderToStaticMarkup(createElement(WorkersInspector, {
    ...inspectorProps,
    agents: [],
    records: [runningRecord, doneRecord],
    focusRunId: "run-done",
  }));
  assert(historyMarkup.includes("Active <span>1</span>"), "Workers inspector should group active Workers");
  assert(historyMarkup.includes("Done <span>1</span>"), "Workers inspector should group completed Workers");
  assert(historyMarkup.includes("API inspection complete."), "Workers inspector should render historical result previews");
  assert(historyMarkup.includes('aria-labelledby="inspector-tab-workers"'), "Workers inspector should link to its tab");
  assert(historyMarkup.includes('data-avatar-seed="worker:run-proposed:task-b"'), "Live Workers should keep their proposed task avatar");
} finally {
  await server.close();
}

export {};
