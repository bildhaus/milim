import { strict as assert } from "node:assert";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { ChatStreamPart } from "../src/api.js";
import { liveWorkGroupSummary } from "../src/lib/streamParts.js";
import { formatDuration } from "../src/lib/usageMetrics.js";

function tool(
  label: string,
  status: "done" | "running" | "error",
  detail?: string,
): ChatStreamPart {
  return {
    kind: "event",
    eventType: "tool",
    label,
    name: "shell",
    icon: "command",
    status,
    detail,
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
assert.equal(formatDuration(4_774_000), "1h 19m 34s");

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
      workDurationMs?: number;
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

  const runningMarkup = renderToStaticMarkup(
    createElement(AssistantMessage, {
      content: "",
      streamParts: [
        { kind: "thinking", content: "checking the command" },
        tool("Running command", "running", "pnpm test"),
      ],
      streaming: true,
    }),
  );
  const runningTag = runningMarkup.match(
    /<details[^>]+data-testid="assistant-stream-work-group"[^>]*>/,
  )?.[0] ?? "";
  assert.match(runningTag, /\sopen=""/, "live work should stay expanded");
  assert.match(runningMarkup, /Running command/);
  assert.match(runningMarkup.replace(/<[^>]+>/g, ""), /pnpm test/);

  const successfulMarkup = renderToStaticMarkup(
    createElement(AssistantMessage, {
      content: "",
      streamParts: [
        tool("Ran command", "done", "pnpm test"),
        { kind: "thinking", content: "verified the result" },
      ],
      streaming: false,
      workDurationMs: 65_000,
    }),
  );
  const successfulTag = successfulMarkup.match(
    /<details[^>]+data-testid="assistant-stream-work-group"[^>]*>/,
  )?.[0] ?? "";
  assert.doesNotMatch(
    successfulTag,
    /\sopen=""/,
    "completed work should collapse",
  );
  assert.match(successfulMarkup, /Worked for 1m 5s/);
  assert.match(successfulMarkup, /1 command, 1 reasoning note/);

  const failedMarkup = renderToStaticMarkup(
    createElement(AssistantMessage, {
      content: "",
      streamParts: [
        tool("Command failed", "error", "exit 1"),
        { kind: "thinking", content: "captured the failure" },
      ],
      streaming: false,
    }),
  );
  const failedTag = failedMarkup.match(
    /<details[^>]+data-testid="assistant-stream-work-group"[^>]*>/,
  )?.[0] ?? "";
  assert.match(failedTag, /\sopen=""/, "failed work should stay expanded");
  assert.match(failedMarkup, /Work stopped/);
  assert.match(failedMarkup, /Command failed/);
  assert.match(failedMarkup, /exit 1/);
  assert.match(
    failedMarkup,
    /role="alert"/,
    "failure detail should be announced",
  );

  const completedToolsMarkup = renderToStaticMarkup(
    createElement(AssistantMessage, {
      content: "",
      streamParts: [
        tool("Read file", "done", "src/App.tsx"),
        tool("Ran command", "done", "pnpm test"),
      ],
      streaming: false,
    }),
  );
  const completedToolsTag = completedToolsMarkup.match(
    /<details[^>]+data-testid="assistant-stream-tool-group"[^>]*>/,
  )?.[0] ?? "";
  assert.doesNotMatch(
    completedToolsTag,
    /\sopen=""/,
    "completed tool-only work should collapse",
  );

  const failedToolsMarkup = renderToStaticMarkup(
    createElement(AssistantMessage, {
      content: "",
      streamParts: [
        tool("Command failed", "error", "exit 1"),
        tool("Read file", "done", "src/App.tsx"),
      ],
      streaming: false,
    }),
  );
  const failedToolsTag = failedToolsMarkup.match(
    /<details[^>]+data-testid="assistant-stream-tool-group"[^>]*>/,
  )?.[0] ?? "";
  assert.match(
    failedToolsTag,
    /\sopen=""/,
    "failed tool-only work should stay expanded",
  );
  assert.match(failedToolsMarkup, /Work stopped/);
  assert.match(failedToolsMarkup, /Command failed · exit 1/);

  const streamingMarkup = render(true);
  assert.equal(
    (streamingMarkup.match(/assistant-stream-work-group/g) ?? []).length,
    1,
    "streaming should keep failed and successful activity in one work group",
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
  assert.match(
    completedMarkup,
    /<summary class="stream-event stream-event-tool stream-event-error">/,
    "completed drawers containing failures should keep a visible error state",
  );
  assert.equal(
    (completedMarkup.match(/stream-event-running/g) ?? []).length,
    0,
    "completed work groups should not render as running",
  );
} finally {
  await server.close();
}

export {};
