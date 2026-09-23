import type { ClaudeImportedThread, CodexRecoveredThread, ToolApprovalRequest } from "../src/api.js";
import { recoveredCodexSession, recoveredCodexSessionId } from "../src/lib/codexRecovery.js";
import { importedClaudeSession, importedClaudeSessionId } from "../src/lib/claudeImport.js";
import {
  approvalChatAllowance,
  approvalResponse,
  approvalShortcut,
  approvalShortcutTarget,
  initialApprovalValues,
  updateApprovalField,
} from "../src/lib/toolApproval.js";
import type { Session } from "../src/sessions/store.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
}

const form: ToolApprovalRequest = {
  kind: "mcp_form",
  server_name: "example",
  message: "Choose values",
  fields: [
    { name: "name", label: "Name", kind: "string", required: true },
    { name: "count", label: "Count", kind: "integer", required: false, minimum: 1 },
    { name: "enabled", label: "Enabled", kind: "boolean", required: false, default: true },
    { name: "tone", label: "Tone", kind: "enum", value_type: "string", required: true, options: [
      { value: "calm", label: "Calm" },
      { value: "direct", label: "Direct" },
    ] },
  ],
};
const values = initialApprovalValues(form);
equal(values.enabled, true, "form defaults should initialize locally");
values.name = "";
values.count = "2";
values.tone = updateApprovalField(form.fields[3], "1");
const approved = approvalResponse(form, values);
assert(approved.response, "valid bounded form should produce a response");
equal(approved.response.name, "", "required empty strings should remain valid");
equal(approved.response.count, 2, "integer input should be parsed");
equal(approved.response.tone, "direct", "enum input should preserve its scalar value");
assert(approvalResponse(form, { ...values, count: "1.5" }).error, "fractional integers should fail client validation");
assert(approvalResponse(form, { ...values, tone: "unknown" }).error, "unknown enum values should fail client validation");

const recovered: CodexRecoveredThread = {
  id: "codex-thread-1",
  title: "Recovered work",
  cwd: "C:\\repo",
  created_at_ms: 1,
  updated_at_ms: 2,
  messages: [{ role: "user", content: "Hello" }, { role: "assistant", content: "Hi" }],
};
const input = recoveredCodexSession(recovered);
equal(input.settings.model, "", "recovery should require an explicit Codex model choice");
equal(input.settings.folder, "C:\\repo", "recovery should preserve the validated folder");
equal(input.messages.length, 2, "recovery should preserve visible transcript messages");

const sessions = [{ id: "milim-1", accountRuntime: { codexThreadId: recovered.id } }] as Session[];
equal(recoveredCodexSessionId(sessions, recovered.id), "milim-1", "existing recovered threads should open instead of duplicating");
equal(recoveredCodexSessionId(sessions, "missing"), null, "unknown threads should remain recoverable");

const claudeImported: ClaudeImportedThread = {
  id: "11111111-1111-4111-8111-111111111111",
  title: "Imported Claude work",
  cwd: "C:\\repo",
  created_at_ms: 1,
  updated_at_ms: 2,
  resumable: true,
  messages: [{ role: "user", content: "Hello" }, { role: "assistant", content: "Hi" }],
};
const claudeInput = importedClaudeSession(claudeImported);
equal(claudeInput.settings.model, "", "Claude import should require an explicit model choice");
equal(claudeInput.settings.folder, "C:\\repo", "resumable Claude imports should preserve their project");
equal(
  importedClaudeSession({ ...claudeImported, resumable: false }).settings.folder,
  "",
  "non-resumable Claude imports should not preserve a missing project",
);
const claudeSessions = [{
  id: "milim-claude",
  accountRuntime: { claudeSessionId: claudeImported.id },
}] as Session[];
equal(
  importedClaudeSessionId(claudeSessions, claudeImported.id),
  "milim-claude",
  "existing Claude imports should open instead of duplicating",
);

const approvalPart = (label: string, detail?: string, request?: ToolApprovalRequest) => ({
  kind: "event" as const,
  eventType: "status" as const,
  label,
  detail,
  status: "running" as const,
  approvalId: "approval-1",
  approvalStatus: "pending" as const,
  approvalRequest: request,
});
const shellAllowance = approvalChatAllowance(approvalPart("Approve shell", '{"command":"cargo test"}'));
equal(shellAllowance?.kind, "command", "shell approvals allow only their exact command");
equal(shellAllowance?.kind === "command" ? shellAllowance.command : "", "cargo test", "the exact command is kept");
equal(approvalChatAllowance(approvalPart("Approve shell", "{}")), null, "shell without a command is never allowed for a chat");
equal(approvalChatAllowance(approvalPart("Approval required: Bash")), null, "Bash without a command stays one-shot");
const toolAllowance = approvalChatAllowance(approvalPart("Approve write_file", '{"path":"a.txt"}'));
equal(toolAllowance?.kind === "tool" ? toolAllowance.tool : "", "write_file", "other tools are allowed by name");
equal(
  approvalChatAllowance(approvalPart("Approve additional permissions", undefined, { kind: "permissions" })),
  null,
  "permission elevations stay one-shot",
);
equal(
  approvalChatAllowance(approvalPart("Provide MCP input", undefined, { kind: "mcp_form", server_name: "s", message: "m", fields: [] })),
  null,
  "MCP forms stay one-shot",
);

equal(approvalShortcut("Enter", "other", true), "approve", "Enter approves once when nothing is focused");
equal(approvalShortcut("y", "other", true), "approve", "Y approves once");
equal(approvalShortcut("N", "other", true), "deny", "N denies");
equal(approvalShortcut("Escape", "other", true), "deny", "Escape denies");
equal(approvalShortcut("Enter", "composer", true), "approve", "Enter in an empty composer approves");
equal(approvalShortcut("y", "composer", true), null, "typing Y into the composer is never hijacked");
equal(approvalShortcut("Enter", "composer", false), null, "a composer draft keeps Enter for sending");
equal(approvalShortcut("Escape", "other", false), null, "shortcuts pause while the composer has text");
equal(approvalShortcut("Enter", "button", true), null, "a focused button keeps its native Enter");
equal(approvalShortcut("n", "editable", true), null, "form fields keep their keys");
equal(approvalShortcutTarget(null), "other", "no target is treated as the page");
