import {
  applyControlEvent,
  applyTimelinePage,
  controlEventInvalidatesBootstrap,
  emptyReplica,
  projectMessages,
  projectTranscript,
} from '../src/control/replica';
import type {
  ControlEventV1,
  JsonValue,
  PendingApprovalV1,
  TimelineItemV1,
  TimelinePageV1,
} from '../src/control/types';

test('model catalog events invalidate bootstrap without entering the timeline', () => {
  expect(controlEventInvalidatesBootstrap('models.updated')).toBe(true);
  expect(controlEventInvalidatesBootstrap('appearance.updated')).toBe(true);
  expect(controlEventInvalidatesBootstrap('unrelated.event')).toBe(false);
});

function item(seq: number, type = 'message', data: any = {id: `m-${seq}`, role: 'user', content: String(seq)}): TimelineItemV1 {
  return {id: `i-${seq}`, thread_id: 't', epoch: 'e1', seq, run_id: null, type, data, created_at_ms: seq};
}

function page(epoch: string, items: TimelineItemV1[], hasOlder = false, hasNewer = false): TimelinePageV1 {
  return {thread_id: 't', epoch, first_seq: items[0]?.seq ?? null, last_seq: items.at(-1)?.seq ?? null, has_older: hasOlder, has_newer: hasNewer, before_seq: null, after_seq: null, items};
}

test('epoch mismatch replaces the canonical replica', () => {
  const first = applyTimelinePage(emptyReplica('t'), page('e1', [item(1)]), 'tail');
  const replacement = applyTimelinePage(first, page('e2', [{...item(1), epoch: 'e2'}]), 'after');
  expect(replacement.epoch).toBe('e2');
  expect(replacement.items).toHaveLength(1);
});

test('a true middle gap requests an authoritative tail', () => {
  const first = applyTimelinePage(emptyReplica('t'), page('e1', [item(1)]), 'tail');
  const gap = applyTimelinePage(first, page('e1', [item(4)]), 'after');
  expect(gap.needsTailRefresh).toBe(true);
});

test('live duplicates are idempotent and assistant deltas project in order', () => {
  let replica = applyTimelinePage(emptyReplica('t'), page('e1', [item(1)]), 'tail');
  const delta = {...item(2, 'assistant_delta', {text: 'hel'}), run_id: 'r1'};
  const event: ControlEventV1 = {
    event_id: 'ev',
    host_id: 'h',
    thread_id: 't',
    epoch: 'e1',
    seq: 2,
    type: 'timeline.appended',
    data: {item: delta} as unknown as JsonValue,
  };
  replica = applyControlEvent(replica, event, 'h');
  replica = applyControlEvent(replica, event, 'h');
  expect(replica.items).toHaveLength(2);
  expect(projectMessages(replica.items).at(-1)?.content).toBe('hel');
});

test('ignores live events emitted by a different desktop host', () => {
  const replica = applyTimelinePage(emptyReplica('t'), page('e1', [item(1)]), 'tail');
  const foreign: ControlEventV1 = {
    event_id: 'foreign',
    host_id: 'host-b',
    thread_id: 't',
    epoch: 'e1',
    seq: 2,
    type: 'timeline.appended',
    data: {item: item(2)} as unknown as JsonValue,
  };
  expect(applyControlEvent(replica, foreign, 'host-a')).toBe(replica);
});

test('older pages prepend in order and preserve whether more history exists', () => {
  const tail = applyTimelinePage(emptyReplica('t'), page('e1', [item(4), item(5)], true), 'tail');
  expect(tail.hasOlder).toBe(true);
  const older = applyTimelinePage(tail, page('e1', [item(2), item(3)], true, true), 'before');
  expect(older.items.map(entry => entry.seq)).toEqual([2, 3, 4, 5]);
  expect(older.hasOlder).toBe(true);
  const first = applyTimelinePage(older, page('e1', [item(1)], false, true), 'before');
  expect(first.items.map(entry => entry.seq)).toEqual([1, 2, 3, 4, 5]);
  expect(first.hasOlder).toBe(false);
});

test('projects native and account activity into one ordered transcript grammar', () => {
  const runItem = (seq: number, type: string, data: any): TimelineItemV1 => ({
    ...item(seq, type, data),
    run_id: 'run-1',
    created_at_ms: seq * 1_000,
  });
  const longRtlPath = `C:\\workspace\\${'nested\\'.repeat(24)}מסמך.ts`;
  const transcript = projectTranscript([
    item(1, 'message', {id: 'user-1', role: 'user', content: 'Please update it'}),
    runItem(2, 'tool_started', {
      id: 'edit-1',
      name: 'edit_file',
      status: 'running',
      detail: longRtlPath,
    }),
    runItem(3, 'tool_updated', {id: 'edit-1', name: 'edit_file', status: 'running'}),
    runItem(4, 'approval_requested', {
      approval_id: 'approval-1',
      request_kind: 'command',
      request: {name: 'Run tests', command: 'pnpm test'},
    }),
    runItem(5, 'approval_resolved', {approval_id: 'approval-1', decision: 'approve'}),
    runItem(6, 'tool_finished', {
      id: 'edit-1',
      name: 'edit_file',
      status: 'completed',
      result: {additions: 3, deletions: 1},
    }),
    runItem(7, 'future_runtime_event', {label: 'בדיקת מצב', detail: 'נשמר בהצלחה'}),
    runItem(8, 'turn_completed', {status: 'completed'}),
    runItem(9, 'message', {id: 'assistant-1', role: 'assistant', content: 'Done'}),
  ]);

  expect(transcript.map(entry => entry.kind)).toEqual([
    'message',
    'activity',
    'approval',
    'message',
  ]);
  const activity = transcript.find(entry => entry.kind === 'activity');
  expect(activity).toMatchObject({status: 'completed', label: 'Worked for 6s'});
  if (!activity || activity.kind !== 'activity') throw new Error('missing activity group');
  expect(activity.rows).toHaveLength(2);
  expect(activity.rows[0]).toMatchObject({
    kind: 'change',
    label: 'Edited file',
    additions: 3,
    deletions: 1,
  });
  expect(activity.rows[0].detail).toContain(longRtlPath);
  expect(activity.rows[0].detail).toContain('+3 -1');
  expect(activity.rows[1]).toMatchObject({label: 'בדיקת מצב', detail: 'נשמר בהצלחה'});
  const approval = transcript.find(entry => entry.kind === 'approval');
  expect(approval).toMatchObject({status: 'completed', label: 'Approved', approval: null});
});

test('deduplicates tool streaming updates and keeps pending approvals actionable inline', () => {
  const pending: PendingApprovalV1 = {
    id: 'approval-2',
    run_id: 'run-2',
    thread_id: 't',
    kind: 'file_change',
    request: {name: 'Edit settings', message: 'Allow this file change'},
    status: 'pending',
    created_at_ms: 4,
  };
  const transcript = projectTranscript([
    {...item(1, 'assistant_delta', {reasoning: 'Checking', text: ''}), run_id: 'run-2'},
    {...item(2, 'tool_started', {id: 'tool-2', name: 'read_file', status: 'running'}), run_id: 'run-2'},
    {...item(3, 'tool_updated', {id: 'tool-2', name: 'read_file', status: 'running', detail: 'README.md'}), run_id: 'run-2'},
  ], [pending]);

  const activity = transcript.find(entry => entry.kind === 'activity');
  expect(activity && activity.kind === 'activity' ? activity.rows : []).toHaveLength(1);
  expect(transcript.find(entry => entry.kind === 'approval')).toMatchObject({
    status: 'approval',
    approval: pending,
  });
  expect(transcript.find(entry => entry.kind === 'message')).toMatchObject({reasoning: 'Checking'});
});

test('projects failures as failed work and does not expose protocol-only events', () => {
  const transcript = projectTranscript([
    {...item(1, 'session_established', {native_session_id: 'secret-session'}), run_id: 'run-3'},
    {...item(2, 'tool_started', {id: 'cmd', name: 'command', status: 'running', detail: 'pnpm test'}), run_id: 'run-3'},
    {...item(3, 'turn_failed', {message: 'Tests failed'}), run_id: 'run-3'},
  ]);
  const activity = transcript.find(entry => entry.kind === 'activity');
  expect(activity).toMatchObject({status: 'failed', label: 'Work stopped'});
  if (!activity || activity.kind !== 'activity') throw new Error('missing failed activity');
  expect(activity.rows.some(row => row.label === 'Tests failed')).toBe(true);
  expect(JSON.stringify(transcript)).not.toContain('secret-session');
});

test('a later tool update recovers the live summary from an earlier tool failure', () => {
  const transcript = projectTranscript([
    {...item(1, 'tool_finished', {id: 'cmd', name: 'command', status: 'failed', error: 'exit 1'}), run_id: 'run-4'},
    {...item(2, 'tool_started', {id: 'read', name: 'read_file', status: 'running', detail: 'README.md'}), run_id: 'run-4'},
  ]);
  expect(transcript.find(entry => entry.kind === 'activity')).toMatchObject({
    status: 'running',
    label: 'Reading file',
    detail: 'README.md',
  });
});

test('marks only ledger-backed assistant messages as inspectable without projecting diagnostics', () => {
  const transcript = projectTranscript([
    {...item(1, 'tool_started', {id: 'read', name: 'read_file', status: 'running'}), run_id: 'run-5'},
    {...item(2, 'turn_completed', {status: 'completed'}), run_id: 'run-5'},
    {...item(3, 'message', {id: 'assistant-5', role: 'assistant', content: 'Done', ledgerVersion: 1}), run_id: 'run-5'},
    {...item(4, 'message', {id: 'assistant-legacy', role: 'assistant', content: 'Old'}), run_id: 'run-old'},
  ]);
  const messages = transcript.filter(entry => entry.kind === 'message');
  expect(messages[0]).toMatchObject({runId: 'run-5', ledgerVersion: 1});
  expect(messages[1].ledgerVersion).toBeUndefined();
  expect(JSON.stringify(transcript)).not.toContain('model_request_resolved');
});
