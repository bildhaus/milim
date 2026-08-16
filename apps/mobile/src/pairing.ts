export interface PairingClaim {
  endpoint: string;
  pairId: string;
  secret: string;
  hostId: string;
}

export function parsePairingClaim(value: string): PairingClaim {
  const url = new URL(value.trim());
  const pairId = url.searchParams.get('pair_id')?.trim();
  const secret = url.searchParams.get('secret')?.trim();
  const hostId = url.searchParams.get('host_id')?.trim();
  if (!pairId || !secret || !hostId) {
    throw new Error('This link does not contain a valid milim pairing claim.');
  }
  let endpoint = url.searchParams.get('endpoint')?.trim();
  if (!endpoint && (url.protocol === 'http:' || url.protocol === 'https:')) {
    endpoint = url.origin;
  }
  if (!endpoint) {
    throw new Error('This pairing claim does not include the desktop endpoint.');
  }
  return {
    endpoint: endpoint.replace(/\/+$/, ''),
    pairId,
    secret,
    hostId,
  };
}

export function assertHostIdentity(expected: string, actual: string): void {
  if (expected !== actual) {
    throw new Error('The pairing link belongs to a different milim desktop.');
  }
}
