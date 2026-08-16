import {
  canUseCompactComposer,
  friendlyEndpoint,
  groupMobileThreads,
  lowercaseMilimBrand,
  nextAwayFromLatest,
  relativeConnectionTime,
} from '../src/mobileUi';
import type {ThreadSummaryV1} from '../src/control/types';

function thread(overrides: Partial<ThreadSummaryV1>): ThreadSummaryV1 {
  return {
    id: 'thread',
    title: 'Thread',
    revision: 1,
    epoch: 'epoch',
    updated_at_ms: 1,
    archived_at_ms: null,
    model: 'model',
    agent_id: null,
    workspace: null,
    busy: false,
    queued_turns: 0,
    ...overrides,
  };
}

describe('mobile UI helpers', () => {
  test('groups threads into Inbox and short project directories', () => {
    const groups = groupMobileThreads([
      thread({id: 'project', workspace: 'C:\\Users\\USER\\Documents\\DEV\\milim', updated_at_ms: 30, busy: true}),
      thread({id: 'inbox', updated_at_ms: 20, queued_turns: 1}),
      thread({id: 'older', workspace: 'C:\\Users\\USER\\Documents\\DEV\\milim', updated_at_ms: 10}),
      thread({id: 'archived', archived_at_ms: 40}),
    ], {project: 2});

    expect(groups.map(group => [group.label, group.subtitle, group.threads.length])).toEqual([
      ['Inbox', 'No Project', 1],
      ['milim', 'Project', 2],
    ]);
    expect(groups[1]).toMatchObject({busy: true, attentionCount: 2});
    expect(groups[1].threads.map(item => item.id)).toEqual(['project', 'older']);
  });

  test('uses hysteresis for reader mode', () => {
    expect(nextAwayFromLatest(false, 219)).toBe(false);
    expect(nextAwayFromLatest(false, 221)).toBe(true);
    expect(nextAwayFromLatest(true, 97)).toBe(true);
    expect(nextAwayFromLatest(true, 95)).toBe(false);
  });

  test('keeps the composer expanded for active input state', () => {
    const base = {
      awayFromLatest: false,
      draft: '',
      attachmentCount: 0,
      inputFocused: false,
      pendingApproval: false,
      forcedOpen: false,
    };
    expect(canUseCompactComposer(base)).toBe(true);
    expect(canUseCompactComposer({...base, awayFromLatest: true})).toBe(true);
    expect(canUseCompactComposer({...base, draft: 'hello'})).toBe(false);
    expect(canUseCompactComposer({...base, attachmentCount: 1})).toBe(false);
    expect(canUseCompactComposer({...base, inputFocused: true})).toBe(false);
    expect(canUseCompactComposer({...base, pendingApproval: true})).toBe(false);
  });

  test('formats connection details without exposing the raw URL', () => {
    expect(friendlyEndpoint('http://127.0.0.1:7377')).toBe('Local emulator · Port 7377');
    expect(friendlyEndpoint('https://desktop.example.ts.net:10000')).toBe('Tailscale · desktop.example.ts.net · Port 10000');
    expect(relativeConnectionTime(990_000, 1_000_000)).toBe('Connected just now');
  });

  test('renders the milim brand in lowercase without changing surrounding labels', () => {
    expect(lowercaseMilimBrand('Milim desktop')).toBe('milim desktop');
    expect(lowercaseMilimBrand('MILIM workbench')).toBe('milim workbench');
    expect(lowercaseMilimBrand('A family of millimeters')).toBe('A family of millimeters');
  });
});
