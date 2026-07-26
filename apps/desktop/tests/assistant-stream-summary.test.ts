import { strict as assert } from "node:assert";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { ChatStreamPart } from "../src/api.js";
import { liveWorkGroupSummary } from "../src/lib/streamParts.js";

function tool(
  label: string,
  status: "done" | "running" | "error",
): ChatStreamPart {
  return {
    kind: "event",
    eventType: "tool",
    label,
    name: "shell",
    icon: "command",
    status,
  };
}

const completedOnly = liveWorkGroupSummary({
  kind: "workGroup",
  parts: [tool("Ran command", "done")],
});
assert.equal(completedOnly?.label, "Ran command");
assert.equal(
  completedOnly?.status,
  "done",
  "completed tools should not render as still running while the answer streams",
);

const activeTool = liveWorkGroupSummary({
  kind: "workGroup",
  parts: [tool("Ran command", "done"), tool("Using Edit", "running")],
});
assert.equal(activeTool?.label, "Using Edit");
assert.equal(activeTool?.status, "running");

const reasoningAfterTool = liveWorkGroupSummary({
  kind: "workGroup",
  parts: [
    tool("Ran command", "done"),
    { kind: "thinking", content: "checking next step" },
  ],
});
assert.equal(reasoningAfterTool?.label, "reasoning...");
assert.equal(reasoningAfterTool?.status, "running");

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { formatRunDuration } = (await server.ssrLoadModule(
    "/src/components/RunTimeline.tsx",
  )) as {
    formatRunDuration: (ms: number) => string;
  };
  assert.equal(formatRunDuration(5_500), "5s");
  assert.equal(formatRunDuration(65_500), "1m 5s");
  assert.equal(formatRunDuration(3_665_500), "1h 1m 5s");

  const { AssistantMessage } = (await server.ssrLoadModule(
    "/src/components/AssistantMessage.tsx",
  )) as {
    AssistantMessage: ComponentType<{
      content: string;
      streamParts: ChatStreamPart[];
      streaming: boolean;
    }>;
  };
  const streamParts: ChatStreamPart[] = [
    tool("Ran first command", "done"),
    { kind: "thinking", content: "first reasoning" },
    tool("Separator failed", "error"),
    tool("Ran second command", "done"),
    { kind: "thinking", content: "latest reasoning" },
  ];
  const render = (streaming: boolean) =>
    renderToStaticMarkup(
      createElement(AssistantMessage, {
        content: "",
        streamParts,
        streaming,
      }),
    );

  const streamingMarkup = render(true);
  assert.equal(
    (streamingMarkup.match(/assistant-stream-work-group/g) ?? []).length,
    2,
    "streaming should preserve both compacted work groups",
  );
  assert.equal(
    (streamingMarkup.match(/reasoning\.\.\./g) ?? []).length,
    1,
    "only the latest work group should render as reasoning",
  );
  assert.equal(
    (streamingMarkup.match(/stream-event-thinking stream-event-running/g) ?? [])
      .length,
    1,
    "only the latest work group should render as running",
  );

  const completedMarkup = render(false);
  assert.equal(
    (completedMarkup.match(/stream-event-running/g) ?? []).length,
    0,
    "completed work groups should not render as running",
  );
} finally {
  await server.close();
}

export {};
