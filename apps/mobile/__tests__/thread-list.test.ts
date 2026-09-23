import {filterThreadsByQuery, unreadThreadIds} from '../src/mobileUi';

const thread = (id: string, updated: number, extra: {archived_at_ms?: number | null; title?: string; workspace?: string | null} = {}) => ({
  id,
  updated_at_ms: updated,
  archived_at_ms: extra.archived_at_ms ?? null,
  title: extra.title ?? id,
  workspace: extra.workspace ?? null,
});

test('threads changed after they were last open are unread', () => {
  const unread = unreadThreadIds(
    [thread('seen', 100), thread('changed', 300), thread('open', 900)],
    {baselineMs: 50, seen: {seen: 100, changed: 200, open: 100}},
    'open',
  );
  expect([...unread]).toEqual(['changed']);
});

test('pairing does not mark existing history unread, but later desktop work is', () => {
  const unread = unreadThreadIds(
    [thread('old', 100), thread('new-on-desktop', 600), thread('archived', 700, {archived_at_ms: 800})],
    {baselineMs: 500, seen: {}},
    null,
  );
  expect([...unread]).toEqual(['new-on-desktop']);
});

test('nothing is unread before read tracking has loaded', () => {
  expect(unreadThreadIds([thread('a', 999)], null, null).size).toBe(0);
});

test('thread search matches titles and project paths case-insensitively', () => {
  const threads = [
    thread('1', 0, {title: 'Fix login bug', workspace: '/Users/me/dev/milim'}),
    thread('2', 0, {title: 'Release notes', workspace: '/Users/me/dev/site'}),
    thread('3', 0, {title: 'Scratch'}),
  ];
  expect(filterThreadsByQuery(threads, '  LOGIN ').map(item => item.id)).toEqual(['1']);
  expect(filterThreadsByQuery(threads, 'dev/site').map(item => item.id)).toEqual(['2']);
  expect(filterThreadsByQuery(threads, '').map(item => item.id)).toEqual(['1', '2', '3']);
});
