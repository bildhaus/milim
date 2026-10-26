import {
  projectTranscript,
  projectTranscriptIncrementally,
  type ProjectedMessage,
  type ProjectedUserDecision,
} from '../src/control/replica';
import type {UserDecisionV1} from '../src/control/generated-v1';
import type {TimelineItemV1} from '../src/control/types';
import {
  askUserAnswerReady,
  askUserAnswerText,
  decisionAnswerPayload,
  emptyAnswerDrafts,
  PLAN_APPROVED_TEXT,
  stripDecisionFallback,
  toggleOption,
} from '../src/control/userDecisions';

const FALLBACK = '**Scope** — Which part first?\n- API\n- UI';

const askDecision: UserDecisionV1 = {
  id: 'd1',
  kind: 'ask_user',
  questions: [
    {
      question: 'Which part first?',
      header: 'Scope',
      options: [
        {label: 'API', description: 'Server routes'},
        {label: 'UI', description: 'Screens'},
      ],
      multi_select: false,
    },
    {
      question: 'Which platforms?',
      header: 'Platforms',
      options: [
        {label: 'iOS', description: ''},
        {label: 'Android', description: ''},
        {label: 'Web', description: ''},
      ],
      multi_select: true,
    },
  ],
  fallback_text: FALLBACK,
};

function timelineItem(seq: number, type: string, data: unknown, runId: string | null = 'r1'): TimelineItemV1 {
  return {id: `i-${seq}`, thread_id: 't', epoch: 'e1', seq, run_id: runId, type, data: data as never, created_at_ms: seq};
}

function decisionRun(decision: UserDecisionV1 = askDecision): TimelineItemV1[] {
  return [
    timelineItem(1, 'message', {id: 'u1', role: 'user', content: 'Build it'}, 'r1'),
    timelineItem(2, 'user_decision_requested', {type: 'user_decision_requested', call_id: 'c1', name: decision.kind, decision}),
    timelineItem(3, 'message', {id: 'a1', role: 'assistant', content: `Before I start:\n\n${decision.fallback_text}\n`}),
    timelineItem(4, 'run_status', {status: 'completed'}),
  ];
}

const decisionOf = (items: ReturnType<typeof projectTranscript>) =>
  items.find((entry): entry is ProjectedUserDecision => entry.kind === 'user-decision');
const messagesOf = (items: ReturnType<typeof projectTranscript>) =>
  items.filter((entry): entry is ProjectedMessage => entry.kind === 'message');

test('strips the decision fallback only when the message ends with it', () => {
  expect(stripDecisionFallback(`Intro\n\n${FALLBACK}\n\n`, FALLBACK)).toBe('Intro');
  expect(stripDecisionFallback(FALLBACK, `\n${FALLBACK}\n`)).toBe('');
  expect(stripDecisionFallback('Something else entirely', FALLBACK)).toBe('Something else entirely');
  expect(stripDecisionFallback('Intro', '')).toBe('Intro');
});

test('single-select questions behave like radios and multi-select ones toggle in option order', () => {
  const [scope, platforms] = askDecision.questions;
  let draft = toggleOption({selected: [], other: ''}, scope, 'API');
  draft = toggleOption(draft, scope, 'UI');
  expect(draft.selected).toEqual(['UI']);
  expect(toggleOption(draft, scope, 'UI').selected).toEqual([]);
  let multi = toggleOption({selected: [], other: ''}, platforms, 'Web');
  multi = toggleOption(multi, platforms, 'iOS');
  expect(multi.selected).toEqual(['iOS', 'Web']);
  expect(toggleOption(multi, platforms, 'Web').selected).toEqual(['iOS']);
});

test('builds the readable answer text and requires one answered question', () => {
  const drafts = emptyAnswerDrafts(askDecision);
  expect(askUserAnswerReady(drafts)).toBe(false);
  expect(askUserAnswerText(askDecision.questions, drafts)).toBe(
    'Answers:\n- Scope: (no answer)\n- Platforms: (no answer)',
  );
  const answered = [
    {selected: ['API'], other: ' start with auth '},
    {selected: [], other: 'Desktop only'},
  ];
  expect(askUserAnswerReady(answered)).toBe(true);
  expect(askUserAnswerText(askDecision.questions, answered)).toBe(
    'Answers:\n- Scope: API; start with auth\n- Platforms: Desktop only',
  );
  expect(askUserAnswerText(askDecision.questions, [{selected: ['UI'], other: ''}, {selected: ['iOS', 'Android'], other: ''}]))
    .toBe('Answers:\n- Scope: UI\n- Platforms: iOS, Android');
  expect(askUserAnswerReady([{selected: [], other: '   '}])).toBe(false);
});

test('answer payload is a turn.send naming the decision', () => {
  expect(decisionAnswerPayload(askDecision, 'approve_plan', PLAN_APPROVED_TEXT)).toEqual({
    text: 'Plan approved. Implement it now.',
    display_text: 'Plan approved. Implement it now.',
    answers_decision: {decision_id: 'd1', response: 'approve_plan'},
  });
});

test('projects a pending decision after the stripped final message', () => {
  const projected = projectTranscript(decisionRun());
  const decision = decisionOf(projected)!;
  expect(decision.state).toBe('pending');
  expect(decision.decision.id).toBe('d1');
  expect(projected.at(-1)).toBe(decision);
  expect(messagesOf(projected).map(message => message.content)).toEqual(['Build it', 'Before I start:']);
  expect(projected.some(entry => entry.kind === 'activity')).toBe(false);
});

test('hides a final message that held only the fallback and keeps unmatched text', () => {
  const onlyFallback = decisionRun().map(entry =>
    entry.seq === 3 ? timelineItem(3, 'message', {id: 'a1', role: 'assistant', content: FALLBACK}) : entry,
  );
  expect(messagesOf(projectTranscript(onlyFallback)).map(message => message.id)).toEqual(['u1']);
  const unmatched = decisionRun().map(entry =>
    entry.seq === 3 ? timelineItem(3, 'message', {id: 'a1', role: 'assistant', content: 'Edited answer'}) : entry,
  );
  expect(messagesOf(projectTranscript(unmatched)).at(-1)?.content).toBe('Edited answer');
});

test('derives answered and superseded states', () => {
  const answered = projectTranscript([
    ...decisionRun(),
    timelineItem(5, 'user_decision_resolved', {decision_id: 'd1', status: 'answered', response: 'answer', command_id: 'cmd'}),
    timelineItem(6, 'message', {id: 'u2', role: 'user', content: 'Answers:', answersDecision: {decision_id: 'd1', response: 'answer'}}, 'r2'),
  ]);
  expect(decisionOf(answered)).toMatchObject({state: 'answered', response: 'answer'});

  const supersededItem = projectTranscript([
    ...decisionRun(),
    timelineItem(5, 'user_decision_resolved', {decision_id: 'd1', status: 'superseded'}),
  ]);
  expect(decisionOf(supersededItem)?.state).toBe('superseded');

  // Without a resolution item, a later non-steering user message supersedes it.
  const steered = projectTranscript([
    ...decisionRun(),
    timelineItem(5, 'message', {id: 's1', role: 'user', content: 'nudge', steering: true}, 'r1'),
  ]);
  expect(decisionOf(steered)?.state).toBe('pending');
  const later = projectTranscript([
    ...decisionRun(),
    timelineItem(5, 'message', {id: 'u2', role: 'user', content: 'Never mind'}, 'r2'),
  ]);
  expect(decisionOf(later)?.state).toBe('superseded');
});

test('renders an exit_plan_mode decision with its plan', () => {
  const plan: UserDecisionV1 = {
    id: 'p1',
    kind: 'exit_plan_mode',
    questions: [],
    plan: '1. Do the thing',
    fallback_text: '**Proposed plan**\n\n1. Do the thing',
  };
  const decision = decisionOf(projectTranscript(decisionRun(plan)))!;
  expect(decision.decision.kind).toBe('exit_plan_mode');
  expect(decision.decision.plan).toBe('1. Do the thing');
  expect(decision.state).toBe('pending');
});

test('ignores malformed decisions', () => {
  const projected = projectTranscript([
    timelineItem(1, 'user_decision_requested', {decision: {id: 'x', kind: 'ask_user', questions: []}}),
    timelineItem(2, 'user_decision_requested', {decision: {id: 'y', kind: 'unknown'}}),
  ]);
  expect(decisionOf(projected)).toBeUndefined();
});

test('incremental projection settles an earlier decision like a full rebuild', () => {
  const initial = decisionRun();
  const first = projectTranscriptIncrementally(null, initial);
  expect(decisionOf(first.projected)?.state).toBe('pending');

  const resolved = [
    ...initial,
    timelineItem(5, 'user_decision_resolved', {decision_id: 'd1', status: 'answered', response: 'answer'}),
  ];
  const second = projectTranscriptIncrementally(first, resolved);
  expect(second.projected).toEqual(projectTranscript(resolved));
  expect(decisionOf(second.projected)?.state).toBe('answered');

  const superseding = [
    ...initial,
    timelineItem(5, 'message', {id: 'u2', role: 'user', content: 'Different idea'}, 'r2'),
  ];
  const third = projectTranscriptIncrementally(first, superseding);
  expect(third.projected).toEqual(projectTranscript(superseding));
  expect(decisionOf(third.projected)?.state).toBe('superseded');

  // The final message arriving after the request still strips the fallback.
  const partial = initial.slice(0, 2);
  const early = projectTranscriptIncrementally(null, partial);
  const late = projectTranscriptIncrementally(early, initial);
  expect(late.projected).toEqual(projectTranscript(initial));
});
