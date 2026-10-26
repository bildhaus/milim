import type {
  UserDecisionAnswerV1,
  UserDecisionResponseV1,
  UserDecisionStatusV1,
  UserDecisionV1,
} from "../generated/control-v1.js";

/** Timeline item a run records when it ends on `ask_user` or `exit_plan_mode`. */
export const USER_DECISION_REQUESTED_ITEM = "user_decision_requested";
/** Timeline item that closes a decision (answered or superseded). */
export const USER_DECISION_RESOLVED_ITEM = "user_decision_resolved";
/** The answering turn's text when the user approves a plan. */
export const PLAN_APPROVED_TEXT = "Plan approved. Implement it now.";

export type ChatUserDecisionStatus = "pending" | UserDecisionStatusV1;

/** A decision as the transcript renders it: the request plus its state. */
export type ChatUserDecision = UserDecisionV1 & {
  status: ChatUserDecisionStatus;
  response?: UserDecisionResponseV1;
};

/** The timeline fields decision state is derived from. */
type DecisionTimelineItem = {
  seq: number;
  run_id: string | null;
  type: string;
  data: Record<string, unknown>;
};

/** One question's answer while the user fills the card in. */
export type UserQuestionAnswer = {
  selected: string[];
  other: string;
};

function isDecision(value: unknown): value is UserDecisionV1 {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const decision = value as Partial<UserDecisionV1>;
  return (
    typeof decision.id === "string"
    && (decision.kind === "ask_user" || decision.kind === "exit_plan_mode")
  );
}

/**
 * The decision `runId` ended on, with its state: closed by a
 * `user_decision_resolved` item, or, failing that, superseded by any later
 * user turn; otherwise still pending.
 */
export function userDecisionForRun(
  items: readonly DecisionTimelineItem[],
  runId: string,
): ChatUserDecision | undefined {
  let request: { seq: number; decision: UserDecisionV1 } | undefined;
  for (const item of items) {
    if (item.run_id !== runId || item.type !== USER_DECISION_REQUESTED_ITEM) continue;
    const decision = item.data.decision;
    if (isDecision(decision)) {
      request = { seq: item.seq, decision };
      break;
    }
  }
  if (!request) return undefined;
  const { seq, decision } = request;
  let status: ChatUserDecisionStatus = "pending";
  let response: UserDecisionResponseV1 | undefined;
  for (const item of items) {
    if (
      item.type === USER_DECISION_RESOLVED_ITEM
      && item.data.decision_id === decision.id
      && (item.data.status === "answered" || item.data.status === "superseded")
    ) {
      status = item.data.status;
      const resolvedResponse = item.data.response;
      if (
        resolvedResponse === "answer"
        || resolvedResponse === "approve_plan"
        || resolvedResponse === "revise_plan"
      ) {
        response = resolvedResponse;
      }
      break;
    }
    if (
      status === "pending"
      && item.seq > seq
      && item.type === "message"
      && item.data.role === "user"
      && item.data.steering !== true
    ) {
      status = "superseded";
    }
  }
  return {
    ...decision,
    questions: Array.isArray(decision.questions) ? decision.questions : [],
    status,
    ...(response ? { response } : {}),
  };
}

/** Whether a decision of `runId` was closed, so its card needs re-projecting. */
export function hasUserDecisionResolution(
  items: readonly DecisionTimelineItem[],
  runId: string,
): boolean {
  return items.some(
    (item) => item.run_id === runId && item.type === USER_DECISION_RESOLVED_ITEM,
  );
}

/**
 * The assistant text without the decision's Markdown fallback, which the
 * host appends for clients without decision cards; the card shows it.
 */
export function stripUserDecisionFallback(content: string, fallback: string): string {
  const suffix = fallback.trim();
  if (!suffix) return content;
  const trimmed = content.trimEnd();
  if (!trimmed.endsWith(suffix)) return content;
  return trimmed.slice(0, trimmed.length - suffix.length).trimEnd();
}

export function answerForQuestion(answer: UserQuestionAnswer | undefined): string {
  const selected = answer?.selected.filter((label) => label.trim()) ?? [];
  const other = answer?.other.trim() ?? "";
  if (selected.length && other) return `${selected.join(", ")}; ${other}`;
  if (selected.length) return selected.join(", ");
  return other;
}

/** Whether every question has an option or text, so the card can submit. */
export function askUserAnswersComplete(
  decision: Pick<UserDecisionV1, "questions">,
  answers: readonly (UserQuestionAnswer | undefined)[],
): boolean {
  return decision.questions.length > 0
    && decision.questions.some((_, index) => Boolean(answerForQuestion(answers[index])));
}

/** The answering turn's text: one readable line per question. */
export function askUserAnswerText(
  decision: Pick<UserDecisionV1, "questions">,
  answers: readonly (UserQuestionAnswer | undefined)[],
): string {
  const lines = decision.questions.map((question, index) =>
    `- ${question.header}: ${answerForQuestion(answers[index]) || "(no answer)"}`,
  );
  return ["Answers:", ...lines].join("\n");
}

export function userDecisionAnswer(
  decision: Pick<UserDecisionV1, "id">,
  response: UserDecisionResponseV1,
): UserDecisionAnswerV1 {
  return { decision_id: decision.id, response };
}
