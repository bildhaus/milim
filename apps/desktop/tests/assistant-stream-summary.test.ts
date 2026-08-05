import { strict as assert } from "node:assert";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { ChatStreamPart } from "../src/api.js";
import { MilimUsageRidgeline } from "../src/components/MilimUsageRidgeline.js";
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

const ridgelineMarkup = renderToStaticMarkup(createElement(MilimUsageRidgeline, {
  usage: {
    months: Array.from({ length: 12 }, (_, index) => ({
      key: `2026-${String(index + 1).padStart(2, "0")}`,
      label: `Month ${index + 1}`,
      days: [1],
    })),
    metrics: [],
    threadCount: 12,
    projectCount: 1,
    activeDayCount: 12,
    hasUsage: true,
  },
}));
assert.equal(
  (ridgelineMarkup.match(/class="usage-ridge-line"/g) ?? []).length,
  3,
  "the empty-chat ridgeline should show only the latest three months",
);

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

  const { AssistantMessage, formatCommandDisplay } = (await server.ssrLoadModule(
    "/src/components/AssistantMessage.tsx",
  )) as {
    AssistantMessage: ComponentType<{
      content: string;
      streamParts: ChatStreamPart[];
      streaming: boolean;
      workDurationMs?: number;
      workspaceFolder?: string;
    }>;
    formatCommandDisplay: (command: string, workspaceFolder?: string) => string;
  };
  const windowsCommand = '"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe" -NoLogo -NoProfile -Command Get-Content "C:\\repo\\src\\App.tsx"';
  assert.equal(
    formatCommandDisplay(windowsCommand, "C:\\repo"),
    'Get-Content ".\\src\\App.tsx"',
    "Windows shell wrappers and workspace paths should be shortened",
  );
  assert.equal(
    formatCommandDisplay("/bin/bash -lc cat file:///Users/me/app/src/App.tsx", "/Users/me/app"),
    "cat ./src/App.tsx",
    "POSIX shell wrappers and workspace file URLs should be shortened",
  );
  assert.equal(
    formatCommandDisplay("cmd.exe /d /c type file:///C:/repo/src/App.tsx", "C:\\repo"),
    "type ./src/App.tsx",
    "cmd wrappers and Windows workspace file URLs should be shortened",
  );
  assert.equal(
    formatCommandDisplay("zsh -c cat /Users/me/app/src/App.tsx", "/Users/me/app"),
    "cat ./src/App.tsx",
    "zsh wrappers should be shortened",
  );
  assert.equal(
    formatCommandDisplay("type C:\\repo2\\src\\App.tsx", "C:\\repo"),
    "type C:\\repo2\\src\\App.tsx",
    "workspace-like path prefixes should remain unchanged",
  );
  assert.equal(
    formatCommandDisplay("fish -c echo unchanged"),
    "fish -c echo unchanged",
    "unrecognized shell wrappers should remain unchanged",
  );
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
  assert.doesNotMatch(runningTag, /\sopen=""/, "live work should start collapsed");
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
  assert.doesNotMatch(failedTag, /\sopen=""/, "completed failed work should collapse");
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
  assert.doesNotMatch(
    failedToolsTag,
    /\sopen=""/,
    "completed failed tool-only work should collapse",
  );
  assert.match(failedToolsMarkup, /Work stopped/);
  assert.match(failedToolsMarkup.replace(/<[^>]+>/g, ""), /Command failed · exit 1/);

  const liveFailureMarkup = renderToStaticMarkup(
    createElement(AssistantMessage, {
      content: "",
      streamParts: [
        { kind: "thinking", content: "running the command" },
        tool("Command failed", "error", windowsCommand),
      ],
      streaming: true,
      workspaceFolder: "C:\\repo",
    }),
  );
  const liveFailureTag = liveFailureMarkup.match(
    /<details[^>]+data-testid="assistant-stream-work-group"[^>]*>/,
  )?.[0] ?? "";
  assert.match(liveFailureTag, /\sopen=""/, "the newest live failure should open its drawer");
  assert.match(liveFailureMarkup, /Get-Content/);
  assert.doesNotMatch(liveFailureMarkup.replace(/<[^>]+>/g, ""), /WindowsPowerShell/);
  assert.match(liveFailureMarkup, /title="&quot;C:\\Windows/);
  assert.match(liveFailureMarkup, /aria-label="Copy full command"/);

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
  const recoveredTag = streamingMarkup.match(
    /<details[^>]+data-testid="assistant-stream-work-group"[^>]*>/,
  )?.[0] ?? "";
  assert.doesNotMatch(recoveredTag, /\sopen=""/, "later activity should collapse an earlier failure");
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
