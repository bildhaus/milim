import {parseMilimLink, threadLink} from '../src/deepLinks';

test('thread links round-trip with their desktop', () => {
  const link = threadLink('host a', 'thread/1');
  expect(parseMilimLink(link)).toEqual({kind: 'thread', threadId: 'thread/1', hostId: 'host a'});
});

test('a thread link without a desktop targets the active one', () => {
  expect(parseMilimLink('milim://thread/abc')).toEqual({kind: 'thread', threadId: 'abc', hostId: null});
});

test('pairing links and unknown links stay with the pairing flow', () => {
  const claim = 'milim://pair?pair_id=1&secret=2&host_id=3&endpoint=http%3A%2F%2F10.0.0.2%3A7377';
  expect(parseMilimLink(claim)).toEqual({kind: 'pair', claim});
  expect(parseMilimLink('https://desk.ts.net/mobile?pair_id=1')).toEqual({kind: 'pair', claim: 'https://desk.ts.net/mobile?pair_id=1'});
});

test('empty and malformed thread links are ignored', () => {
  expect(parseMilimLink('   ')).toBeNull();
  expect(parseMilimLink('milim://thread/%E0%A4%A')).toBeNull();
});
