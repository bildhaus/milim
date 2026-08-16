import {assertHostIdentity, parsePairingClaim} from '../src/pairing';

test('rejects host-less legacy pairing links', () => {
  expect(() =>
    parsePairingClaim('https://desktop.example:10000/mobile?pair_id=p1&secret=s1'),
  ).toThrow(/valid milim pairing claim/);
});

test('parses the native deep-link payload', () => {
  expect(parsePairingClaim('milim://pair?endpoint=https%3A%2F%2Fdesktop&pair_id=p&secret=s&host_id=h').hostId).toBe('h');
});

test('rejects incomplete claims', () => {
  expect(() => parsePairingClaim('milim://pair?pair_id=x')).toThrow(/valid milim pairing claim/);
});

test('rejects a probe or bootstrap from a different host', () => {
  expect(() => assertHostIdentity('host-a', 'host-b')).toThrow(/different milim desktop/);
  expect(() => assertHostIdentity('host-a', 'host-a')).not.toThrow();
});
