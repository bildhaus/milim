import { strict as assert } from "node:assert";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ControlTimelineItemV1, UserDecisionV1 } from "../src/api.js";
import {
  projectControlRunMessages,
  shouldReconcileControlRunProjection,
} from "../src/lib/canonicalControl.js";
import {
  askUserAnswersComplete,
  askUserAnswerText,
  stripUserDecisionFallback,
  userDecisionForRun,
} from "../src/lib/userDecisions.js";
import { UserDecisionCard } from "../src/components/UserDecisionCard.js";

const item = (
  seq: number,
  type: string,
  data: Record<string, unknown>,
  runId = "run-1",
): ControlTimelineItemV1 => ({
  id: `event-${seq}`,
  thread_id: "thread-1",
  epoch: "epoch-1",
  seq,
  run_id: runId,
  type,
  data,
  created_at_ms: seq,
});

const fallback = "**Questions for you**\n\n1. **Store**: Which store?\n   - SQLite: Embedded\n   - Postgres: Server";
const decision: UserDecisionV1 = {
  id: "decision-1",
  kind: "ask_user",
  questions: [
    {
      question: "Which store?",
      header: "Store",
      options: [
        { label: "SQLite", description: "Embedded" },
        { label: "Postgres", description: "Server" },
      ],
      multi_select: false,
    },
    {
      question: "Which extras?",
      header: "Extras",
      options: [
        { label: "Cache", description: "" },
        { label: "Search", description: "" },
      ],
      multi_select: true,
    },
  ],
  fallback_text: fallback,
};

const deciding = [
  item(1, "message", { id: "user-1", role: "user", content: "pick a store" }),
  item(2, "assistant_delta", { text: "One moment." }),
  item(3, "user_decision_requested", {
    type: "user_decision_requested",
    call_id: "call-1",
    name: "ask_user",
    decision,
  }),
  item(4, "message", {
    id: "assistant-1",
    role: "assistant",
    content: `One moment.\n\n${fallback}`,
  }),
  item(5, "run_status", { status: "completed" }),
];

{
  const pending = userDecisionForRun(deciding, "run-1");
  assert.equal(pending?.status, "pending", "an unresolved decision is pending");
  assert.equal(userDecisionForRun(deciding, "run-2"), undefined);

  const projected = projectControlRunMessages(deciding, "run-1");
  const assistant = projected.find((message) => message.role === "assistant");
  assert.equal(assistant?.content, "One moment.", "the card replaces the text fallback");
  assert.equal(assistant?.userDecision?.id, "decision-1");
  assert.equal(assistant?.userDecision?.status, "pending");
  assert.deepEqual(
    assistant?.streamParts,
    [{ kind: "text", content: "One moment." }],
    "stripping the fallback keeps the streamed parts",
  );
}

{
  const answered = [
    ...deciding,
    item(6, "user_decision_resolved", {
      decision_id: "decision-1",
      status: "answered",
      response: "answer",
      command_id: "cmd-1",
    }),
    item(7, "message", {
      id: "user-2",
      role: "user",
      content: "Answers:\n- Store: SQLite",
      answersDecision: { decision_id: "decision-1", response: "answer" },
    }, "run-2"),
  ];
  const state = userDecisionForRun(answered, "run-1");
  assert.equal(state?.status, "answered");
  assert.equal(state?.response, "answer");
  assert.ok(
    shouldReconcileControlRunProjection(answered, "run-1", true),
    "a resolution from another client re-projects the deciding run",
  );
  const answer = projectControlRunMessages(answered, "run-2")[0];
  assert.deepEqual(answer.answersDecision, { decision_id: "decision-1", response: "answer" });

  const superseded = [
    ...deciding,
    item(6, "message", { id: "user-2", role: "user", content: "never mind" }, "run-2"),
  ];
  assert.equal(
    userDecisionForRun(superseded, "run-1")?.status,
    "superseded",
    "a later user turn supersedes an unresolved decision",
  );
  const steered = [
    ...deciding,
    item(6, "message", { id: "steer", role: "user", content: "also", steering: true }),
  ];
  assert.equal(userDecisionForRun(steered, "run-1")?.status, "pending");
}

{
  assert.equal(stripUserDecisionFallback(`Intro\n\n${fallback}\n`, fallback), "Intro");
  assert.equal(stripUserDecisionFallback(fallback, fallback), "");
  assert.equal(stripUserDecisionFallback("Edited text", fallback), "Edited text");

  const empty = [
    { selected: [], other: "" },
    { selected: [], other: "" },
  ];
  assert.ok(!askUserAnswersComplete(decision, empty));
  const answers = [
    { selected: ["SQLite"], other: "" },
    { selected: ["Cache", "Search"], other: " keep it small " },
  ];
  assert.ok(askUserAnswersComplete(decision, answers));
  assert.equal(
    askUserAnswerText(decision, answers),
    "Answers:\n- Store: SQLite\n- Extras: Cache, Search; keep it small",
  );
  assert.equal(
    askUserAnswerText(decision, [{ selected: [], other: "MySQL" }]),
    "Answers:\n- Store: MySQL\n- Extras: (no answer)",
  );
}

{
  const pendingCard = renderToStaticMarkup(
    createElement(UserDecisionCard, {
      decision: { ...decision, status: "pending" },
      disabled: false,
      onAnswer: () => true,
    }),
  );
  assert.ok(pendingCard.includes("Questions for you"));
  assert.ok(pendingCard.includes('role="radio"'), "single-select options act as radios");
  assert.ok(pendingCard.includes('role="checkbox"'), "multi-select options act as checkboxes");
  assert.ok(pendingCard.includes("Other..."), "every question takes free text");
  assert.ok(pendingCard.includes('data-testid="user-decision-submit"'));

  const answeredPlan = renderToStaticMarkup(
    createElement(UserDecisionCard, {
      decision: {
        id: "plan-1",
        kind: "exit_plan_mode",
        questions: [],
        plan: "1. Edit a.rs",
        fallback_text: "",
        status: "answered",
        response: "approve_plan",
      },
      disabled: false,
      onAnswer: () => true,
    }),
  );
  assert.ok(answeredPlan.includes("Plan approved"));
  assert.ok(!answeredPlan.includes("Approve and implement"), "a closed plan takes no input");

  const pendingPlan = renderToStaticMarkup(
    createElement(UserDecisionCard, {
      decision: {
        id: "plan-2",
        kind: "exit_plan_mode",
        questions: [],
        plan: "1. Edit a.rs",
        fallback_text: "",
        status: "pending",
      },
      disabled: false,
      onAnswer: () => true,
    }),
  );
  assert.ok(pendingPlan.includes("Approve and implement"));
  assert.ok(pendingPlan.includes("Keep planning"));
}
