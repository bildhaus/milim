import type {
  UserDecisionResponseV1,
  UserDecisionV1,
  UserQuestionV1,
} from './generated-v1';
import type {JsonValue} from './types';

// A run that ends on `ask_user` or `exit_plan_mode` hands the thread a
// structured decision. The host closes it with `user_decision_resolved`; a
// later user turn supersedes it even when that item has not synced yet.
export type UserDecisionState = 'pending' | 'answered' | 'superseded';

export const USER_DECISION_REQUESTED = 'user_decision_requested';
export const USER_DECISION_RESOLVED = 'user_decision_resolved';
export const PLAN_APPROVED_TEXT = 'Plan approved. Implement it now.';

export interface QuestionAnswerDraft {
  selected: string[];
  other: string;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function parseQuestion(value: unknown): UserQuestionV1 | null {
  const record = asRecord(value);
  if (!record || typeof record.question !== 'string') return null;
  const options = Array.isArray(record.options)
    ? record.options.flatMap(option => {
      const entry = asRecord(option);
      return entry && typeof entry.label === 'string'
        ? [{label: entry.label, description: typeof entry.description === 'string' ? entry.description : ''}]
        : [];
    })
    : [];
  return {
    question: record.question,
    header: typeof record.header === 'string' && record.header.trim() ? record.header : 'Question',
    options,
    multi_select: record.multi_select === true,
  };
}

// The decision carried by a `user_decision_requested` item, or null when the
// payload is not one this client understands.
export function parseUserDecision(data: unknown): UserDecisionV1 | null {
  const decision = asRecord(asRecord(data)?.decision);
  if (!decision || typeof decision.id !== 'string' || !decision.id) return null;
  if (decision.kind !== 'ask_user' && decision.kind !== 'exit_plan_mode') return null;
  const questions = Array.isArray(decision.questions)
    ? decision.questions.flatMap(question => {
      const parsed = parseQuestion(question);
      return parsed ? [parsed] : [];
    })
    : [];
  if (decision.kind === 'ask_user' && !questions.length) return null;
  return {
    id: decision.id,
    kind: decision.kind,
    questions,
    ...(typeof decision.plan === 'string' ? {plan: decision.plan} : {}),
    fallback_text: typeof decision.fallback_text === 'string' ? decision.fallback_text : '',
  };
}

export function parseUserDecisionResolution(data: unknown): {
  decisionId: string;
  status: Exclude<UserDecisionState, 'pending'>;
  response?: UserDecisionResponseV1;
} | null {
  const record = asRecord(data);
  if (!record || typeof record.decision_id !== 'string') return null;
  if (record.status !== 'answered' && record.status !== 'superseded') return null;
  const response = record.response;
  return {
    decisionId: record.decision_id,
    status: record.status,
    ...(response === 'answer' || response === 'approve_plan' || response === 'revise_plan'
      ? {response}
      : {}),
  };
}

// The deciding run's final message ends with the decision's Markdown fallback
// for older clients; the card already shows it.
export function stripDecisionFallback(content: string, fallbackText: string): string {
  const fallback = fallbackText.trim();
  if (!fallback) return content;
  const trimmed = content.trimEnd();
  if (!trimmed.endsWith(fallback)) return content;
  return trimmed.slice(0, trimmed.length - fallback.length).trimEnd();
}

export function emptyAnswerDrafts(decision: UserDecisionV1): QuestionAnswerDraft[] {
  return decision.questions.map(() => ({selected: [], other: ''}));
}

// Single-select questions behave like radios; multi-select ones toggle.
export function toggleOption(
  draft: QuestionAnswerDraft,
  question: UserQuestionV1,
  label: string,
): QuestionAnswerDraft {
  const selected = draft.selected.includes(label);
  if (question.multi_select) {
    return {
      ...draft,
      selected: selected
        ? draft.selected.filter(entry => entry !== label)
        : question.options.map(option => option.label).filter(
          entry => entry === label || draft.selected.includes(entry),
        ),
    };
  }
  return {...draft, selected: selected ? [] : [label]};
}

function questionAnswered(draft: QuestionAnswerDraft | undefined): boolean {
  return Boolean(draft && (draft.selected.length || draft.other.trim()));
}

export function askUserAnswerReady(drafts: QuestionAnswerDraft[]): boolean {
  return drafts.some(questionAnswered);
}

export function askUserAnswerText(
  questions: UserQuestionV1[],
  drafts: QuestionAnswerDraft[],
): string {
  const lines = questions.map((question, index) => {
    const draft = drafts[index];
    const other = draft?.other.trim() ?? '';
    const picked = draft?.selected.join(', ') ?? '';
    const answer = picked && other
      ? `${picked}; ${other}`
      : picked || other || '(no answer)';
    return `- ${question.header}: ${answer}`;
  });
  return ['Answers:', ...lines].join('\n');
}

// The `turn.send` payload that answers a decision. The host validates the
// response against the decision kind and replies `conflict` when it is stale.
export function decisionAnswerPayload(
  decision: UserDecisionV1,
  response: UserDecisionResponseV1,
  text: string,
): JsonValue {
  return {
    text,
    display_text: text,
    answers_decision: {decision_id: decision.id, response},
  };
}

export function decisionStatusLabel(
  state: UserDecisionState,
  response?: UserDecisionResponseV1,
): string {
  if (state === 'superseded') return 'Superseded by a later message';
  if (state === 'answered') {
    if (response === 'approve_plan') return 'Plan approved';
    if (response === 'revise_plan') return 'Sent feedback on the plan';
    return 'Answered';
  }
  return 'Waiting for your answer';
}
