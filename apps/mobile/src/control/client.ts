import type {
  ControlBootstrapV1,
  ControlCommandResultV1,
  ControlCommandV1,
  ControlEventV1,
  PairedCredential,
  TimelinePageV1,
} from './types';

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
  const timeout = setTimeout(() => timeoutController.abort(), CONTROL_REQUEST_TIMEOUT_MS);
  let response: Response;
  try {
    response = await fetch(`${normalizeEndpoint(endpoint)}${path}`, {
      ...init,
      signal: init?.signal ?? timeoutController.signal,
      headers: {
        Accept: 'application/json',
        ...(init?.body ? {'Content-Type': 'application/json'} : {}),
        ...(deviceKey ? {Authorization: `Bearer ${deviceKey}`} : {}),
        ...init?.headers,
      },
    });
  } catch (error) {
    if (timeoutController.signal.aborted) {
      throw new Error(`milim did not respond at ${normalizeEndpoint(endpoint)} within 8 seconds.`);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }
  const text = await response.text();
  const body = text ? JSON.parse(text) : null;
  if (!response.ok) {
    const message = body?.error?.message ?? body?.message ?? `HTTP ${response.status}`;
    throw new ControlHttpError(message, response.status);
  }
  return body as T;
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
