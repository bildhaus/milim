const mockCalls: string[] = [];
const drafts = new Map<string, string>();
const timelines = new Map<string, string>();
const modelPickerPreferences = new Map<string, string>();
const key = (values: unknown[] = []) => `${values[0]}\u0000${values[1]}`;
const mockExecute = jest.fn(async (sql: string, values: unknown[] = []) => {
  mockCalls.push(`execute:${sql}`);
  if (sql.startsWith('INSERT INTO drafts')) drafts.set(key(values), String(values[2]));
  if (sql.startsWith('DELETE FROM drafts')) drafts.delete(key(values));
  if (sql.startsWith('SELECT text FROM drafts')) {
    const text = drafts.get(key(values));
    return {rows: text === undefined ? [] : [{text}]};
  }
  if (sql.startsWith('INSERT INTO timeline_tails')) timelines.set(key(values), String(values[4]));
  if (sql.startsWith('SELECT page_json FROM timeline_tails')) {
    const page_json = timelines.get(key(values));
    return {rows: page_json === undefined ? [] : [{page_json}]};
  }
  if (sql.startsWith('INSERT INTO model_picker_preferences')) {
    modelPickerPreferences.set(String(values[0]), String(values[1]));
  }
  if (sql.startsWith('SELECT preferences_json FROM model_picker_preferences')) {
    const preferences_json = modelPickerPreferences.get(String(values[0]));
    return {rows: preferences_json === undefined ? [] : [{preferences_json}]};
  }
  return {rows: []};
});
const mockExecuteBatch = jest.fn(async () => {
  mockCalls.push('batch');
});

jest.mock('@op-engineering/op-sqlite', () => ({
  open: () => ({execute: mockExecute, executeBatch: mockExecuteBatch}),
}));

const {listHosts, readDraft, readModelPickerPreferences, readTimelineTail, saveDraft, saveModelPickerPreferences, saveTimelineTail} = require('../src/storage/cache') as typeof import('../src/storage/cache');

test('enables WAL before the schema batch opens its transaction', async () => {
  await listHosts();

  expect(mockCalls[0]).toBe('execute:PRAGMA journal_mode = WAL');
  expect(mockCalls[1]).toBe('batch');
  expect(mockCalls[2]).toContain('SELECT metadata_json FROM hosts');
});

test('persists model picker preferences per host', async () => {
  await saveModelPickerPreferences('host-a', {
    favorites: ['codex:gpt-5'],
    favoritesOnly: true,
    collapsedGroups: ['OpenRouter'],
  });

  expect(await readModelPickerPreferences('host-a')).toEqual({
    favorites: ['codex:gpt-5'],
    favoritesOnly: true,
    collapsedGroups: ['OpenRouter'],
  });
  expect(await readModelPickerPreferences('host-b')).toEqual({
    favorites: [],
    favoritesOnly: false,
    collapsedGroups: [],
  });
});

test('partitions drafts and timeline tails by host id', async () => {
  await saveDraft('host-a', 'shared-thread', 'draft a');
  await saveDraft('host-b', 'shared-thread', 'draft b');
  await saveTimelineTail('host-a', {
    thread_id: 'shared-thread', epoch: 'a', first_seq: 1, last_seq: 1,
    has_older: false, has_newer: false, before_seq: null, after_seq: null, items: [],
  });
  await saveTimelineTail('host-b', {
    thread_id: 'shared-thread', epoch: 'b', first_seq: 2, last_seq: 2,
    has_older: false, has_newer: false, before_seq: null, after_seq: null, items: [],
  });

  expect(await readDraft('host-a', 'shared-thread')).toBe('draft a');
  expect(await readDraft('host-b', 'shared-thread')).toBe('draft b');
  expect((await readTimelineTail('host-a', 'shared-thread'))?.epoch).toBe('a');
  expect((await readTimelineTail('host-b', 'shared-thread'))?.epoch).toBe('b');
});
