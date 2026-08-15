import {parsePairingClaim} from '../src/pairing';

test('parses legacy https pairing links for native claim', () => {
  expect(parsePairingClaim('https://desktop.example:10000/mobile?pair_id=p1&secret=s1')).toEqual({
    endpoint: 'https://desktop.example:10000',
    pairId: 'p1',
    secret: 's1',
    hostId: undefined,
  });
});

test('parses the native deep-link payload', () => {
  expect(parsePairingClaim('milim://pair?endpoint=https%3A%2F%2Fdesktop&pair_id=p&secret=s&host_id=h').hostId).toBe('h');
});

test('rejects incomplete claims', () => {
  expect(() => parsePairingClaim('milim://pair?pair_id=x')).toThrow(/valid milim pairing claim/);
});
