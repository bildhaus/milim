import {
  cancelPairingRequest,
  claimPairingRequest,
  createPairingRequest,
  fetchPairingRequestStatus,
} from '../src/control/client';

const originalFetch = globalThis.fetch;
const mockFetch = jest.fn();

function jsonResponse(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    text: async () => JSON.stringify(body),
  } as unknown as Response;
}

describe('nearby pairing request client', () => {
  beforeEach(() => {
    mockFetch.mockReset();
    Object.assign(globalThis, {fetch: mockFetch});
  });

  afterAll(() => {
    Object.assign(globalThis, {fetch: originalFetch});
  });

  test('keeps the private request key in headers through approval polling and claim', async () => {
    mockFetch
      .mockResolvedValueOnce(jsonResponse({
        request_id: 'request-1',
        request_key: 'private-key',
        expires_at: 120,
      }))
      .mockResolvedValueOnce(jsonResponse({
        request_id: 'request-1',
        status: 'approved',
        expires_at: 120,
      }))
      .mockResolvedValueOnce(jsonResponse({
        device_id: 'device-1',
        device_key: 'device-key',
        device_name: 'Android controller',
      }))
      .mockResolvedValueOnce(jsonResponse({cancelled: true}));

    const created = await createPairingRequest(
      'http://10.0.2.2:7378',
      'Android controller',
      'android',
    );
    await fetchPairingRequestStatus(
      'http://10.0.2.2:7378',
      created.request_id,
      created.request_key,
    );
    await claimPairingRequest(
      'http://10.0.2.2:7378',
      created.request_id,
      created.request_key,
    );
    await cancelPairingRequest(
      'http://10.0.2.2:7378',
      created.request_id,
      created.request_key,
    );

    expect(mockFetch.mock.calls[0][0]).toBe('http://10.0.2.2:7378/mobile/pair-requests');
    expect(JSON.parse(mockFetch.mock.calls[0][1].body)).toEqual({
      device_name: 'Android controller',
      platform: 'android',
    });
    for (const call of mockFetch.mock.calls.slice(1)) {
      expect(call[0]).not.toContain('private-key');
      expect(call[1].headers['X-Milim-Pairing-Key']).toBe('private-key');
    }
    expect(mockFetch.mock.calls[2][1].method).toBe('POST');
    expect(mockFetch.mock.calls[3][1].method).toBe('DELETE');
  });
});
