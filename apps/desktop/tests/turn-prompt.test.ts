import { strict as assert } from "node:assert";
import type { ChatMessage, PreviewSurfaceTarget } from "../src/api.js";
import { estimateTextTokens } from "../src/lib/contextCompaction.js";
import { managedPreviewRuntimeForTurn } from "../src/lib/managedPreviewRuntime.js";
import { previewRuntimeKeyForThread } from "../src/lib/previewRuntimeKeys.js";
import { buildTurnPromptContext, contextMessagesForTurn, folderLabel, memoryScopes, prepareTurnPromptContext, resolveTurnToolApproval, workspaceRuleMessagesForRuntime, type MemoryHit } from "../src/lib/turnPrompt.js";

function user(content: string): ChatMessage {
  return { role: "user", content };
}

function promptWithMemory(memoryHits: MemoryHit[]) {
  return buildTurnPromptContext({
    sessionId: "memory-test",
    threadTitle: "Memory",
    folder: "",
    instructions: "",
    planMode: false,
    memory: true,
    conversation: [user("continue")],
    memoryHits,
    selectedSkills: [],
    turnId: "memory-turn",
    sandbox: false,
    computerUse: false,
    activeAgentId: null,
    toolApproval: "guarded",
    toolApprovalGrant: false,
    experimentalHashlinePatch: false,
  });
}

assert.equal(folderLabel("C:\\Users\\USER\\Documents\\DEV\\milim"), "milim");
assert.deepEqual(memoryScopes("thread-1", " C:\\work "), [
  { kind: "global", locator: "personal" },
  { kind: "project", locator: "C:\\work" },
  { kind: "thread", locator: "thread-1" },
]);
assert.deepEqual(memoryScopes("thread-1", ""), [
  { kind: "global", locator: "personal" },
  { kind: "thread", locator: "thread-1" },
]);
assert.deepEqual(memoryScopes("thread-1", "C:\\clone", {
  root: "C:\\clone",
  project_locator: "git:github.com/Owner/Repo",
  legacy_project_locator: "C:\\clone",
  origin: "github.com/Owner/Repo",
  instructions: [],
  warnings: [],
}), [
  { kind: "global", locator: "personal" },
  { kind: "project", locator: "git:github.com/Owner/Repo" },
  { kind: "project", locator: "C:\\clone" },
  { kind: "thread", locator: "thread-1" },
]);

const plain = buildTurnPromptContext({
  sessionId: "s1",
  threadTitle: "Thread",
  folder: "",
  globalInstructions: " Always write focused tests. ",
  instructions: " Be terse. ",
  planMode: false,
  memory: false,
  conversation: [user("hello")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-1",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.deepEqual(plain.instructionMessages, [{ role: "system", content: "Be terse." }]);
assert.deepEqual(plain.globalInstructionMessages, [{ role: "system", content: "Always write focused tests." }]);
assert.equal(plain.artifactMessages.length, 1, "no-folder artifact guidance should stay in turn context");
assert.equal(plain.useTools, false);
assert.equal(plain.accountRuntimeMayUseTools, false);
assert.deepEqual(contextMessagesForTurn(plain, "model").slice(0, 2).map((message) => message.content), ["Always write focused tests.", "Be terse."]);
assert.equal(contextMessagesForTurn(plain, "agent").some((message) => message.content === "Be terse."), false);
assert.equal(contextMessagesForTurn(plain, "agent")[0].content, "Always write focused tests.", "global instructions should remain active for named agents");

const explicitMcp = buildTurnPromptContext({
  sessionId: "s-mcp",
  threadTitle: "MCP",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("use paper-mcp for this")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-mcp",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(explicitMcp.useTools, true, "an explicit MCP request should use the Milim tool loop without a workspace");

const explicitMcpTool = buildTurnPromptContext({
  sessionId: "s-mcp-tool",
  threadTitle: "MCP tool",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("Use /paper__get_basic_info")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-mcp-tool",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  tools: [{ name: "paper__get_basic_info", description: "Inspect Paper", effect: "read_only" }],
});
assert.equal(explicitMcpTool.useTools, true, "an explicit MCP tool token should use the Milim tool loop");

const workspaceProviderTools = buildTurnPromptContext({
  sessionId: "s-workspace",
  threadTitle: "Workspace",
  folder: "C:\\repo",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("edit files")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-workspace",
  sandbox: false,
  computerUse: false,
  privacy: "block",
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(workspaceProviderTools.useTools, true, "provider model plus workspace should use the Milim tool loop");
assert.equal(workspaceProviderTools.accountRuntimeMayUseTools, false);
assert.equal(workspaceProviderTools.toolContext.workspace, "C:\\repo");
assert.equal(workspaceProviderTools.toolContext.privacy_mode, "block");

const previewTools = buildTurnPromptContext({
  sessionId: "s-preview",
  threadTitle: "Preview",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("inspect the visible preview")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-preview",
  sandbox: false,
  computerUse: false,
  previewTools: true,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(previewTools.useTools, true, "visible preview should enable scoped preview tools without computer-use");
assert.equal(previewTools.toolContext.computer_use_enabled, false);
assert.equal(previewTools.toolContext.preview_tools_enabled, true);

const blankPreviewSurface: PreviewSurfaceTarget = {
  kind: "blank",
  title: "Browser",
  native: false,
  status: "not_inspectable",
  capabilities: [],
};
const blankPreview = buildTurnPromptContext({
  sessionId: "s-preview-blank",
  threadTitle: "Preview",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("inspect the visible preview")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-preview-blank",
  sandbox: false,
  computerUse: false,
  previewSurface: blankPreviewSurface,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(blankPreview.useTools, false, "non-inspectable preview surfaces should not expose tools");
assert.equal(blankPreview.toolContext.preview_tools_enabled, false);
assert.deepEqual(blankPreview.toolContext.preview_surface, blankPreviewSurface);
assert.match(contextMessagesForTurn(blankPreview, "model").at(-1)?.content ?? "", /untrusted UI metadata/);
assert.match(contextMessagesForTurn(blankPreview, "agent").at(-1)?.content ?? "", /\"kind\":\"blank\"/);

const managedRuntime = managedPreviewRuntimeForTurn({
  kind: "app",
  status: "running",
  url: "http://127.0.0.1:5173/dashboard",
  active: true,
  ready: true,
});
assert.deepEqual(managedRuntime, {
  kind: "app",
  status: "running",
  active: true,
  ready: true,
  url: "http://127.0.0.1:5173/dashboard",
});
const hiddenManagedRuntime = buildTurnPromptContext({
  sessionId: "s-preview-runtime",
  threadTitle: "App preview",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("is the app still running?")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-preview-runtime",
  sandbox: false,
  computerUse: false,
  managedPreviewRuntime: managedRuntime,
  previewSurface: null,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(hiddenManagedRuntime.useTools, false, "runtime awareness alone must not enable preview tools");
assert.equal(hiddenManagedRuntime.toolContext.preview_tools_enabled, false);
assert.match(hiddenManagedRuntime.previewRuntimeMessages[0].content, /127\.0\.0\.1:5173/);
assert.match(hiddenManagedRuntime.previewRuntimeMessages[0].content, /independently of the inspector/);
assert.match(contextMessagesForTurn(hiddenManagedRuntime, "model").at(-1)?.content ?? "", /\"ready\":true/);
assert.match(contextMessagesForTurn(hiddenManagedRuntime, "agent").at(-1)?.content ?? "", /runtime metadata only/);

const visibleManagedRuntime = buildTurnPromptContext({
  sessionId: "s-preview-runtime-visible",
  threadTitle: "App preview",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("inspect the app")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-preview-runtime-visible",
  sandbox: false,
  computerUse: false,
  managedPreviewRuntime: managedRuntime,
  previewSurface: {
    kind: "runtime_browser",
    title: "App",
    url: "http://127.0.0.1:5173/dashboard",
    native: true,
    status: "ready",
    capabilities: ["dom_snapshot"],
  },
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(visibleManagedRuntime.useTools, true);
assert.equal(visibleManagedRuntime.toolContext.preview_tools_enabled, true);
assert.equal(visibleManagedRuntime.previewRuntimeMessages.length, 1);
assert.equal(visibleManagedRuntime.browserMessages.length, 1);

assert.equal(managedPreviewRuntimeForTurn({
  kind: "app",
  status: "stopped",
  url: "http://127.0.0.1:5173",
  active: false,
  ready: false,
}), null, "stopped runtimes must not leak stale awareness into later turns");
assert.deepEqual(managedPreviewRuntimeForTurn({
  kind: "app",
  status: "starting",
  active: true,
  ready: false,
}), {
  kind: "app",
  status: "starting",
  active: true,
  ready: false,
}, "active but unhealthy runtimes should still be visible to the agent without tools");

const googlePreview = buildTurnPromptContext({
  sessionId: "s-preview-google",
  threadTitle: "Sheet",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("read column C")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-preview-google",
  accountRuntimeKind: "opencode",
  sandbox: false,
  computerUse: false,
  previewSurface: {
    kind: "google_workspace",
    title: "Costs",
    url: "https://docs.google.com/spreadsheets/d/file-id/edit",
    native: false,
    status: "ready",
    capabilities: [],
  },
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(googlePreview.useTools, false);
assert.equal(googlePreview.accountRuntimeMayUseTools, true);
assert.equal(googlePreview.toolContext.preview_tools_enabled, false);
assert.match(googlePreview.browserMessages[0].content, /docs\.google\.com/);
assert.match(googlePreview.browserMessages[0].content, /switch the thread to Review or Open/);

const virtualProject = buildTurnPromptContext({
  sessionId: "s1",
  threadTitle: "Thread",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("why is the preview broken?")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-virtual",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  virtualProjectFiles: [{ path: "src/App.tsx", content: "export default function App() { return null; }" }],
});
assert.equal(virtualProject.artifactMessages.length, 2, "virtual project follow-ups should include file context");
assert.match(virtualProject.artifactMessages[1].content, /src\/App\.tsx/);

const scheduled = buildTurnPromptContext({
  sessionId: "s2",
  threadTitle: "Thread",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("remind me")],
  lastUserText: "Create an automation every 5 minutes",
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-2",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "review",
  toolApprovalGrant: true,
  experimentalHashlinePatch: true,
});
assert.equal(scheduled.useScheduleTools, true);
assert.equal(scheduled.useTools, true);
assert.match(scheduled.scheduleMessages[0].content, /schedule_create/);
assert.equal(scheduled.toolContext.tool_approval_grant, true);
assert.equal(scheduled.toolContext.experimental_hashline_patch, true);
assert.equal(contextMessagesForTurn(scheduled, "model").some((message) => message.content.includes("schedule_create")), false);
assert.equal(contextMessagesForTurn(scheduled, "tools").some((message) => message.content.includes("schedule_create")), true);

const memory = buildTurnPromptContext({
  sessionId: "s3",
  threadTitle: "Memory Thread",
  folder: "",
  instructions: "",
  planMode: false,
  memory: true,
  conversation: [user("continue")],
  memoryHits: [{ node: { scope_kind: "project", kind: "decision", title: "Use SQLite", body: "Local first.", source: "user", updated_at: "2026-07-20 08:30:00" } }],
  selectedSkills: [{ id: "sk1", name: "Skill", description: "Useful", instructions: "Do it.", enabled: true, source_kind: "local" }],
  explicitSkillIds: ["sk1"],
  turnId: "turn-3",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(memory.useTools, false, "retrieved memory alone should stay on the cheap model-chat path");
assert.equal(memory.runMemoryContext.memory_enabled, false);
assert.match(memory.memoryMessages[0].content, /Relevant local memories/);
assert.match(memory.memoryMessages[0].content, /scope=project \| kind=decision \| source=user \| updated=2026-07-20/);
assert.match(memory.memoryMessages[0].content, /untrusted historical context/);
assert.match(memory.memoryMessages[0].content, /workspace files take precedence/);
assert.doesNotMatch(memory.memoryMessages[0].content, /memory_register/);
assert.match(memory.skillMessages[0].content, /## 1\. Skill/);
assert.equal(memory.runMemoryContext.project_label, undefined);

const packedMemories = promptWithMemory(Array.from({ length: 8 }, (_, index) => ({
  node: {
    scope_kind: "project",
    kind: "fact",
    title: `Memory ${index + 1}`,
    body: "Small body.",
    source: "test",
    updated_at: "2026-07-20 08:30:00",
  },
})));
const packedMemoryText = packedMemories.memoryMessages[0].content;
assert.equal(packedMemoryText.match(/^\d+\. \[/gm)?.length, 5);
assert.ok(estimateTextTokens(packedMemoryText) <= 1_024);

const truncatedMemory = promptWithMemory([
  { node: { scope_kind: "personal", kind: "preference", title: "Large preference", body: "detail ".repeat(4_000), source: "user", updated_at: "2026-07-19" } },
  { node: { scope_kind: "personal", kind: "fact", title: "Should not displace first", body: "small", source: "user", updated_at: "2026-07-18" } },
]);
assert.match(truncatedMemory.memoryMessages[0].content, /\.\.\. \[truncated\]/);
assert.doesNotMatch(truncatedMemory.memoryMessages[0].content, /Should not displace first/);
assert.ok(estimateTextTokens(truncatedMemory.memoryMessages[0].content) <= 1_024);

const skippedOversizedMemory = promptWithMemory([
  { node: { scope_kind: "project", kind: "fact", title: "First small", body: "small", source: "test", updated_at: "2026-07-20" } },
  { node: { scope_kind: "project", kind: "fact", title: "Oversized later", body: "detail ".repeat(4_000), source: "test", updated_at: "2026-07-19" } },
  { node: { scope_kind: "project", kind: "fact", title: "Third small", body: "small", source: "test", updated_at: "2026-07-18" } },
]);
assert.match(skippedOversizedMemory.memoryMessages[0].content, /First small/);
assert.doesNotMatch(skippedOversizedMemory.memoryMessages[0].content, /Oversized later/);
assert.match(skippedOversizedMemory.memoryMessages[0].content, /Third small/);
assert.ok(estimateTextTokens(skippedOversizedMemory.memoryMessages[0].content) <= 1_024);

const explicitMemoryWrite = buildTurnPromptContext({
  sessionId: "s3-write",
  threadTitle: "Memory Thread",
  folder: "",
  instructions: "",
  planMode: false,
  memory: true,
  conversation: [user("remember this preference")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-3-write",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(explicitMemoryWrite.useTools, true, "explicit memory writes should enable the agent tool path");
assert.equal(explicitMemoryWrite.runMemoryContext.memory_enabled, true);
assert.match(explicitMemoryWrite.memoryMessages[0].content, /memory_register/);
assert.match(explicitMemoryWrite.memoryMessages[0].content, /scope=personal/);
assert.match(explicitMemoryWrite.memoryMessages[0].content, /Do not create new thread-scoped memories/);

const accountRuntime = buildTurnPromptContext({
  sessionId: "s4",
  threadTitle: "Runtime",
  folder: "C:\\repo",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("edit files")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-4",
  codexModel: "gpt-5",
  sandbox: true,
  computerUse: true,
  activeAgentId: null,
  toolApproval: "review",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(accountRuntime.useTools, false);
assert.equal(accountRuntime.accountRuntimeMayUseTools, true);

const instructionFamilies = buildTurnPromptContext({
  sessionId: "families",
  threadTitle: "Families",
  folder: "C:\\repo",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("continue")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "families-turn",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  workspaceContext: {
    root: "C:\\repo",
    project_locator: "git:github.com/Owner/Repo",
    legacy_project_locator: "C:\\repo",
    origin: "github.com/Owner/Repo",
    instructions: [
      { family: "agents", scope: "project", path: "AGENTS.md", content: "agents", bytes: 6, status: "loaded" },
      { family: "claude", scope: "project", path: "CLAUDE.md", content: "claude", bytes: 6, status: "loaded" },
    ],
    warnings: [],
  },
});
assert.match(workspaceRuleMessagesForRuntime(instructionFamilies, "native")[0].content, /agents/);
assert.match(workspaceRuleMessagesForRuntime(instructionFamilies, "native")[0].content, /claude/);
assert.doesNotMatch(workspaceRuleMessagesForRuntime(instructionFamilies, "codex")[0].content, /AGENTS\.md/);
assert.doesNotMatch(workspaceRuleMessagesForRuntime(instructionFamilies, "claude")[0].content, /CLAUDE\.md/);

const planModeAccountRuntime = buildTurnPromptContext({
  sessionId: "s4-plan",
  threadTitle: "Runtime",
  folder: "C:\\repo",
  instructions: "",
  planMode: true,
  memory: false,
  conversation: [user("plan edits")],
  memoryHits: [],
  selectedSkills: [],
  turnId: "turn-4-plan",
  claudeModel: "sonnet",
  sandbox: true,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "review",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
});
assert.equal(planModeAccountRuntime.useTools, false);
assert.equal(planModeAccountRuntime.accountRuntimeMayUseTools, false, "plan mode should not start account-runtime tools");

const searched: Array<{ query: string; scopes: unknown[]; limit: number; model?: string }> = [];
const selectedQueries: Array<{ query: string; limit: number; ids?: string[] }> = [];
const prepared = await prepareTurnPromptContext({
  sessionId: "s5",
  threadTitle: "Prepared",
  folder: "C:\\repo",
  instructions: "",
  planMode: false,
  memory: true,
  conversation: [user("remember this")],
  activeAgent: { skill_mode: "custom", enabled_skills: ["custom-skill"] },
  skills: [
    { id: "custom-skill", name: "Custom", description: "", instructions: "Use custom.", enabled: true, source_kind: "local" },
    { id: "disabled-skill", name: "Disabled", description: "", instructions: "Skip.", enabled: false, source_kind: "local" },
  ],
  turnId: "turn-5",
  model: "local-model",
  sandbox: false,
  computerUse: false,
  activeAgentId: "agent-1",
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => `${message.content} with attachments`,
  searchMemory: async (query, scopes, limit, model) => {
    searched.push({ query, scopes, limit, model });
    return [{ node: { scope_kind: "thread", kind: "note", title: "Saved", body: "Memory body" } }];
  },
  selectSkills: async (query, limit, ids) => {
    selectedQueries.push({ query, limit, ids });
    return [{ id: "custom-skill", name: "Custom", description: "", instructions: "Use custom.", enabled: true, source_kind: "local" }];
  },
});
assert.equal(searched[0].query, "remember this with attachments");
assert.equal(searched[0].limit, 20);
assert.equal(searched[0].model, "local-model");
assert.deepEqual(searched[0].scopes, [
  { kind: "global", locator: "personal" },
  { kind: "project", locator: "C:\\repo" },
  { kind: "thread", locator: "s5" },
]);
assert.equal(selectedQueries.length, 1, "custom agent skills should select within their allowlist");
assert.deepEqual(selectedQueries[0].ids, ["custom-skill"]);
assert.match(prepared.memoryMessages[0].content, /Memory body/);
assert.match(prepared.skillMessages[0].content, /Custom/);
assert.doesNotMatch(prepared.skillMessages[0].content, /Use custom/);
assert.equal(prepared.useTools, true);

const skillOnlyPrepared = await prepareTurnPromptContext({
  sessionId: "s-skill-only",
  threadTitle: "Skill only",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("review this source change")],
  activeAgent: { tool_mode: "none", skill_mode: "custom", enabled_skills: ["review"] },
  skills: [],
  turnId: "turn-skill-only",
  model: "local-model",
  sandbox: false,
  computerUse: false,
  activeAgentId: "agent-skill-only",
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => message.content,
  searchMemory: async () => [],
  selectSkills: async () => [{
    id: "review",
    name: "Review",
    description: "Review source changes",
    instructions: "List findings first.",
    enabled: true,
    source_kind: "local",
  }],
  tools: [
    { name: "shell", description: "Run commands", effect: "command" },
    { name: "milim_skill_search", description: "Find skills", effect: "read_only" },
    { name: "milim_skill_read", description: "Read a skill", effect: "read_only" },
  ],
});
assert.equal(skillOnlyPrepared.useTools, true, "lazy skill loading should work when regular Agent tools are disabled");
assert.deepEqual(
  skillOnlyPrepared.enabledTools,
  ["milim_skill_search", "milim_skill_read"],
  "skill-only turns should expose only the lazy skill tools",
);
assert.doesNotMatch(
  skillOnlyPrepared.toolDefinitionMessages?.[0]?.content ?? "",
  /"shell"/,
  "skill-only turns should not re-enable host tools",
);

let planMemorySearches = 0;
const plannedWithMemoryEnabled = await prepareTurnPromptContext({
  sessionId: "s-plan-memory",
  threadTitle: "Plan",
  folder: "",
  instructions: "",
  planMode: true,
  memory: true,
  conversation: [user("plan this and remember it")],
  activeAgent: null,
  skills: [],
  turnId: "turn-plan-memory",
  model: "local-model",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => message.content,
  searchMemory: async () => {
    planMemorySearches += 1;
    return [];
  },
  selectSkills: async () => [],
});
assert.equal(planMemorySearches, 0, "plan mode should not search memory");
assert.equal(plannedWithMemoryEnabled.memoryMessages.length, 0, "plan mode should not expose memory writes");
assert.equal(plannedWithMemoryEnabled.runMemoryContext.memory_enabled, false);

const tagSkills = [
  { id: "code-review", name: "Code Review", description: "", instructions: "Review carefully.", enabled: true, source_kind: "local" },
  { id: "code", name: "Code", description: "", instructions: "Shorter match.", enabled: true, source_kind: "local" },
  { id: "design", name: "Design-Polish", description: "", instructions: "Polish UI.", enabled: true, source_kind: "local" },
  { id: "disabled-skill", name: "Disabled Skill", description: "", instructions: "Skip disabled.", enabled: false, source_kind: "local" },
];
const noneTagged = await prepareTurnPromptContext({
  sessionId: "s-tags-none",
  threadTitle: "Tags",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("Use @Code Review and /Design-Polish, not @Disabled Skill or @Missing Skill.")],
  activeAgent: { skill_mode: "none", enabled_skills: [] },
  skills: tagSkills,
  turnId: "turn-tags-none",
  model: "local-model",
  sandbox: false,
  computerUse: false,
  activeAgentId: "agent-tags",
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => message.content,
  searchMemory: async () => [],
  selectSkills: async () => {
    throw new Error("none mode should not auto-select skills");
  },
});
assert.match(noneTagged.skillMessages[0].content, /Code Review/);
assert.match(noneTagged.skillMessages[0].content, /Design-Polish/);
assert.doesNotMatch(noneTagged.skillMessages[0].content, /Shorter match/);
assert.doesNotMatch(noneTagged.skillMessages[0].content, /Disabled Skill/);
assert.doesNotMatch(noneTagged.skillMessages[0].content, /Missing Skill/);

const customTagged = await prepareTurnPromptContext({
  sessionId: "s-tags-custom",
  threadTitle: "Tags",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("Use @Code Review.")],
  activeAgent: { skill_mode: "custom", enabled_skills: ["code-review"] },
  skills: tagSkills,
  turnId: "turn-tags-custom",
  model: "local-model",
  sandbox: false,
  computerUse: false,
  activeAgentId: "agent-tags",
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => message.content,
  searchMemory: async () => [],
  selectSkills: async () => [],
});
assert.equal(customTagged.skillMessages[0].content.match(/Code Review/g)?.length, 1, "tagged custom skill should dedupe");

const autoTagged = await prepareTurnPromptContext({
  sessionId: "s-tags-auto",
  threadTitle: "Tags",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("Use /Code Review.")],
  activeAgent: null,
  skills: tagSkills,
  turnId: "turn-tags-auto",
  model: "local-model",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => message.content,
  searchMemory: async () => [],
  selectSkills: async () => [tagSkills[0], tagSkills[2]],
});
assert.equal(autoTagged.skillMessages[0].content.match(/Code Review/g)?.length, 1, "tagged auto skill should dedupe");
assert.match(autoTagged.skillMessages[0].content, /Design-Polish/);

const staleCatalogTagged = await prepareTurnPromptContext({
  sessionId: "s-tags-stale-catalog",
  threadTitle: "Tags",
  folder: "",
  instructions: "",
  planMode: false,
  memory: false,
  conversation: [user("Use @Code Review.")],
  activeAgent: { skill_mode: "none", enabled_skills: [] },
  skills: [],
  turnId: "turn-tags-stale-catalog",
  model: "local-model",
  sandbox: false,
  computerUse: false,
  activeAgentId: "agent-tags",
  toolApproval: "guarded",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => message.content,
  searchMemory: async () => [],
  selectSkills: async (query, limit, ids) => {
    assert.equal(query, "Use @Code Review.");
    assert.equal(limit, 10);
    assert.equal(ids, undefined);
    return [tagSkills[0]];
  },
});
assert.match(staleCatalogTagged.skillMessages[0].content, /Review carefully\./);

const accountPrepared = await prepareTurnPromptContext({
  sessionId: "s6",
  threadTitle: "Account",
  folder: "C:\\repo",
  instructions: "",
  planMode: false,
  memory: true,
  conversation: [user("use account runtime")],
  activeAgent: null,
  skills: [],
  turnId: "turn-6",
  codexModel: "gpt-5",
  model: "codex:gpt-5",
  sandbox: false,
  computerUse: false,
  activeAgentId: null,
  toolApproval: "review",
  toolApprovalGrant: false,
  experimentalHashlinePatch: false,
  messageContent: (message) => message.content,
  searchMemory: async (_query, _scopes, _limit, model) => {
    searched.push({ query: "account", scopes: [], limit: 0, model });
    return [];
  },
  selectSkills: async (query, limit) => {
    selectedQueries.push({ query, limit });
    return [];
  },
});
assert.equal(searched.at(-1)?.model, undefined, "account-runtime memory search should use default memory model");
assert.equal(selectedQueries.at(-1)?.query, "use account runtime");
assert.equal(accountPrepared.accountRuntimeMayUseTools, true);
assert.equal(
  accountPrepared.toolContext.preview_runtime_key,
  previewRuntimeKeyForThread("s6", "C:\\repo"),
);

assert.deepEqual(resolveTurnToolApproval({
  useTools: false,
  accountRuntimeMayUseTools: false,
  toolApproval: "review",
  planMode: false,
}), { status: "not_required", grant: false });
assert.deepEqual(resolveTurnToolApproval({
  useTools: true,
  accountRuntimeMayUseTools: false,
  toolApproval: "review",
  planMode: false,
  requestedGrant: true,
}), { status: "granted", grant: true });
assert.deepEqual(resolveTurnToolApproval({
  useTools: false,
  accountRuntimeMayUseTools: true,
  toolApproval: "review",
  planMode: false,
  requestedGrant: false,
}), { status: "denied", grant: false, error: "Tool run canceled." });
assert.deepEqual(resolveTurnToolApproval({
  useTools: true,
  accountRuntimeMayUseTools: false,
  toolApproval: "review",
  planMode: false,
}), { status: "not_required", grant: false });
assert.equal(resolveTurnToolApproval({
  useTools: true,
  accountRuntimeMayUseTools: false,
  toolApproval: "review",
  planMode: true,
}).status, "not_required");
