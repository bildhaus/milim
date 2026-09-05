import type {
  ControlBootstrapV1,
  ControlCommandResultV1,
  ControlCommandV1,
  ControlEventV1,
  MobileHostProbe,
  PairingRequestCreated,
  PairingRequestStatus,
  PairedCredential,
  TimelinePageV1,
} from './types';
import type {RunEventPageV1, RunInspectionV1} from './generated-v1';

export class ControlHttpError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

const CONTROL_REQUEST_TIMEOUT_MS = 8_000;

export function normalizeEndpoint(value: string): string {
  const trimmed = value.trim().replace(/\/+$/, '');
  if (!/^https?:\/\//i.test(trimmed)) {
    throw new Error('Use a complete http:// or https:// host URL.');
  }
  return trimmed;
}

async function requestJson<T>(
  endpoint: string,
  path: string,
  deviceKey?: string,
  init?: RequestInit,
): Promise<T> {
  const timeoutController = new AbortController();
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    timeoutController.abort();
  }, CONTROL_REQUEST_TIMEOUT_MS);
  const cancel = () => timeoutController.abort();
  if (init?.signal?.aborted) cancel();
  else init?.signal?.addEventListener('abort', cancel, {once: true});
  try {
    const response = await fetch(`${normalizeEndpoint(endpoint)}${path}`, {
      ...init,
      signal: timeoutController.signal,
      headers: {
        Accept: 'application/json',
        ...(init?.body ? {'Content-Type': 'application/json'} : {}),
        ...(deviceKey ? {Authorization: `Bearer ${deviceKey}`} : {}),
        ...init?.headers,
      },
    });
    const text = await response.text();
    const body = text ? JSON.parse(text) : null;
    if (!response.ok) {
      const message = body?.error?.message ?? body?.message ?? `HTTP ${response.status}`;
      throw new ControlHttpError(message, response.status);
    }
    return body as T;
  } catch (error) {
    if (timedOut) {
      throw new Error(`milim did not respond at ${normalizeEndpoint(endpoint)} within 8 seconds.`);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
    init?.signal?.removeEventListener('abort', cancel);
  }
}

export async function claimPairing(
  endpoint: string,
  pairId: string,
  secret: string,
  deviceName: string,
): Promise<PairedCredential> {
  return requestJson(endpoint, '/mobile/pair', undefined, {
    method: 'POST',
    body: JSON.stringify({pair_id: pairId, secret, device_name: deviceName}),
  });
}

export async function fetchMobileHostProbe(
  endpoint: string,
  signal?: AbortSignal,
): Promise<MobileHostProbe> {
  return requestJson(endpoint, '/mobile', undefined, signal ? {signal} : undefined);
}

export async function createPairingRequest(
  endpoint: string,
  deviceName: string,
  platform: 'android' | 'ios',
  signal?: AbortSignal,
): Promise<PairingRequestCreated> {
  return requestJson(endpoint, '/mobile/pair-requests', undefined, {
    method: 'POST',
    body: JSON.stringify({device_name: deviceName, platform}),
    signal,
  });
}

export async function fetchPairingRequestStatus(
  endpoint: string,
  requestId: string,
  requestKey: string,
  signal?: AbortSignal,
): Promise<PairingRequestStatus> {
  return requestJson(
    endpoint,
    `/mobile/pair-requests/${encodeURIComponent(requestId)}`,
    undefined,
    {headers: {'X-Milim-Pairing-Key': requestKey}, signal},
  );
}

export async function claimPairingRequest(
  endpoint: string,
  requestId: string,
  requestKey: string,
  signal?: AbortSignal,
): Promise<PairedCredential> {
  return requestJson(
    endpoint,
    `/mobile/pair-requests/${encodeURIComponent(requestId)}/claim`,
    undefined,
    {
      method: 'POST',
      headers: {'X-Milim-Pairing-Key': requestKey},
      signal,
    },
  );
}

export async function cancelPairingRequest(
  endpoint: string,
  requestId: string,
  requestKey: string,
): Promise<void> {
  await requestJson(
    endpoint,
    `/mobile/pair-requests/${encodeURIComponent(requestId)}`,
    undefined,
    {
      method: 'DELETE',
      headers: {'X-Milim-Pairing-Key': requestKey},
    },
  );
}

export async function fetchBootstrap(
  endpoint: string,
  deviceKey: string,
): Promise<ControlBootstrapV1> {
  return requestJson(endpoint, '/control/v1/bootstrap', deviceKey);
}

export async function fetchTimeline(
  endpoint: string,
  deviceKey: string,
  threadId: string,
  query: {tail?: number; afterSeq?: number; beforeSeq?: number} = {tail: 100},
): Promise<TimelinePageV1> {
  const params = new URLSearchParams();
  if (query.tail !== undefined) params.set('tail', String(query.tail));
  if (query.afterSeq !== undefined) params.set('after_seq', String(query.afterSeq));
  if (query.beforeSeq !== undefined) params.set('before_seq', String(query.beforeSeq));
  return requestJson(
    endpoint,
    `/control/v1/threads/${encodeURIComponent(threadId)}/timeline?${params}`,
    deviceKey,
  );
}

export async function fetchRunInspection(
  endpoint: string,
  deviceKey: string,
  runId: string,
): Promise<RunInspectionV1> {
  return requestJson(
    endpoint,
    `/control/v1/runs/${encodeURIComponent(runId)}`,
    deviceKey,
  );
}

export async function fetchRunEvents(
  endpoint: string,
  deviceKey: string,
  runId: string,
  afterSeq?: number,
): Promise<RunEventPageV1> {
  const params = new URLSearchParams({limit: '50'});
  if (afterSeq !== undefined) params.set('after_seq', String(afterSeq));
  return requestJson(
    endpoint,
    `/control/v1/runs/${encodeURIComponent(runId)}/events?${params}`,
    deviceKey,
  );
}

export async function sendCommand(
  endpoint: string,
  deviceKey: string,
  command: ControlCommandV1,
): Promise<ControlCommandResultV1> {
  // Callers retain the command object and command_id across an ambiguous
  // network retry. This function never invents a replacement identifier.
  return requestJson(endpoint, '/control/v1/commands', deviceKey, {
    method: 'POST',
    body: JSON.stringify(command),
  });
}

export async function connectControlSocket(
  endpoint: string,
  deviceKey: string,
  onEvent: (event: ControlEventV1) => void,
  onClose: () => void,
): Promise<WebSocket> {
  const ticket = await requestJson<{ticket: string; expires_in_seconds: number}>(
    endpoint,
    '/control/v1/socket-ticket',
    deviceKey,
    {method: 'POST'},
  );
  const base = normalizeEndpoint(endpoint);
  const socketUrl = `${base.replace(/^http:/i, 'ws:').replace(/^https:/i, 'wss:')}/control/v1/ws?ticket=${encodeURIComponent(ticket.ticket)}`;
  const socket = new WebSocket(socketUrl);
  socket.onmessage = event => {
    try {
      onEvent(JSON.parse(String(event.data)) as ControlEventV1);
    } catch {
      // A malformed live event is treated as a gap by the next foreground
      // bootstrap/timeline fetch, never as canonical state.
    }
  };
  socket.onclose = onClose;
  socket.onerror = onClose;
  return socket;
}

export function newCommandId(): string {
  const random = Math.random().toString(36).slice(2);
  return `mobile-${Date.now().toString(36)}-${random}`;
}
