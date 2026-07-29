import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { ChatStreamPart } from "../src/api.js";
import {
  autoApprovableToolApprovals,
  dismissToolApproval,
  pendingToolApprovals,
  toolApprovalPrompts,
} from "../src/lib/toolApproval.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { AssistantMessage } = (await server.ssrLoadModule(
    "/src/components/AssistantMessage.tsx",
  )) as { AssistantMessage: ComponentType<{ content: string; streamParts: ChatStreamPart[] }> };
  const { ToolApprovalPrompt } = (await server.ssrLoadModule(
    "/src/components/ToolApprovalPrompt.tsx",
  )) as { ToolApprovalPrompt: ComponentType<{ part: Extract<ChatStreamPart, { kind: "event" }> }> };

  const approvalPart = (
    approvalRequest: Extract<ChatStreamPart, { kind: "event" }>["approvalRequest"],
  ): Extract<ChatStreamPart, { kind: "event" }> => ({
    kind: "event",
    eventType: "status",
    label: "Approval",
    detail: "{\"command\":\"test\"}",
    status: "done",
    approvalId: "approval-1",
    approvalStatus: "pending",
    approvalRequest,
  });
  const renderApproval = (approvalRequest: Extract<ChatStreamPart, { kind: "event" }>["approvalRequest"]) =>
    renderToStaticMarkup(createElement(ToolApprovalPrompt, {
      part: approvalPart(approvalRequest),
    }));

  const googleEdit = renderToStaticMarkup(createElement(ToolApprovalPrompt, {
    part: {
      ...approvalPart({ kind: "file_change" }),
      label: "Approve google_docs_edit",
      detail: "{\"file_id\":\"document-1\",\"operations\":[{\"action\":\"replace_text\"}]}",
    },
  }));
  assert(googleEdit.includes("Approval required"), "approval prompt should explain why the run is paused");
  assert(googleEdit.includes("Google Docs edit"), "approval prompt should present tool names readably");
  assert(googleEdit.includes("<summary>Review request</summary>"), "arguments should use a compact disclosure");
  assert(googleEdit.includes("<dt>File</dt>") && googleEdit.includes("document-1"), "request fields should use readable labels");
  assert(googleEdit.includes("<dt>Changes</dt>") && googleEdit.includes("Replace text"), "nested request values should be readable");
  assert(!googleEdit.includes("<pre") && !googleEdit.includes("&quot;file_id&quot;"), "approval details should not render as JSON");
  assert(googleEdit.indexOf(">Deny<") < googleEdit.indexOf(">Approve<"), "the primary approval action should be last");

  const plainText = renderToStaticMarkup(createElement(ToolApprovalPrompt, {
    part: {
      ...approvalPart({ kind: "command" }),
      detail: "Run the formatter in the selected workspace.",
    },
  }));
  assert(plainText.includes("Run the formatter in the selected workspace."), "plain-text requests should remain readable");

  const form = renderApproval({
    kind: "mcp_form",
    server_name: "example",
    message: "Choose values",
    fields: [
      { name: "name", label: "Name", kind: "string", required: true },
      { name: "tone", label: "Tone", kind: "enum", required: true, options: [{ value: "calm", label: "Calm" }] },
    ],
  });
  assert(form.includes("Choose values"), "MCP form message should render");
  assert(form.includes("Name *"), "required MCP form fields should render");
  assert(form.includes("<select"), "enum MCP form fields should use a native select");
  assert(form.includes(">Submit<"), "supported MCP forms should submit explicitly");
  assert(form.includes(">Decline<"), "supported MCP forms should remain declineable");

  const permission = renderApproval({
    kind: "permissions",
    reason: "Needs network",
    permissions: { network: { domains: ["example.com"] } },
  });
  assert(permission.includes("Needs network"), "permission reason should render");
  assert(permission.includes("<dt>Network access</dt>") && permission.includes("<dt>Domains</dt>"), "permission labels should be readable");
  assert(permission.includes("example.com"), "exact requested permission should render");
  assert(!permission.includes("<pre"), "permissions should not render as JSON");
  assert(permission.includes("Allow once"), "permission approval should be turn-scoped in the UI");

  const unsupported = renderApproval({
    kind: "mcp_unsupported",
    server_name: "example",
    message: "Unsupported form",
    reason: "Nested objects are unsupported.",
  });
  assert(unsupported.includes("Nested objects are unsupported."), "unsupported reason should render");
  assert(unsupported.includes(">Decline<"), "unsupported MCP requests should be declineable");
  assert(!unsupported.includes(">Approve<") && !unsupported.includes(">Submit<"), "unsupported MCP requests should be decline-only");

  const transcript = renderToStaticMarkup(createElement(AssistantMessage, {
    content: "",
    streamParts: [approvalPart({ kind: "command" })],
  }));
  assert(transcript.includes("Approval"), "approval transcript should keep the request summary");
  assert(transcript.includes("command"), "approval transcript should keep the requested arguments");
  assert(!transcript.includes(">Approve<") && !transcript.includes(">Deny<"), "approval actions should render only by the composer");

  const pending = approvalPart({ kind: "command" });
  const secondPending = { ...pending, approvalId: "approval-2", label: "Approval 2" };
  const resolved = { ...pending, label: "shell approved", approvalStatus: "approved" as const };
  const resolvedTranscript = renderToStaticMarkup(createElement(AssistantMessage, {
    content: "",
    streamParts: [resolved],
  }));
  assert(resolvedTranscript.includes("shell approved"), "resolved approval should keep its readable outcome");
  assert(!resolvedTranscript.includes("&quot;command&quot;"), "resolved approval should hide its protocol payload");
  assert(
    pendingToolApprovals([{ role: "assistant", content: "", streamParts: [pending, secondPending] }]).length === 2,
    "every pending approval should attach to the composer",
  );
  assert(
    autoApprovableToolApprovals([
      pending,
      approvalPart({ kind: "permissions" }),
      approvalPart({ kind: "mcp_form", server_name: "example", message: "Input", fields: [] }),
      approvalPart({ kind: "mcp_url", server_name: "example", message: "Authorize", url: "https://example.com" }),
    ]).length === 2,
    "Open should auto-approve plain tool and permission requests but keep MCP input and authorization interactive",
  );
  const mcpForm = approvalPart({ kind: "mcp_form", server_name: "example", message: "Input", fields: [] });
  const openPrompts = toolApprovalPrompts([pending, approvalPart({ kind: "permissions" }), mcpForm], "open");
  assert(!openPrompts.includes(pending), "Open should not mount a prompt for an auto-approved request");
  assert(openPrompts.includes(mcpForm), "Open should keep requests that need user input visible");
  assert(pendingToolApprovals([{ role: "assistant", content: "", streamParts: [pending, resolved] }]).length === 0, "resolved legacy approvals should not reappear by the composer");
  const dismissed = dismissToolApproval([{ role: "assistant", content: "", streamParts: [pending] }], "approval-1", 42);
  assert(pendingToolApprovals(dismissed).length === 0, "dismissed stale approvals should leave the composer");
  assert(
    dismissed[0].streamParts?.some((part) => part.kind === "event" && part.approvalStatus === "canceled"),
    "dismissed stale approvals should remain in transcript history",
  );
} finally {
  await server.close();
}
