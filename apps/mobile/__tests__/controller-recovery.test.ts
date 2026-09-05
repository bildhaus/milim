import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { useMilimController } from '../src/controller/useMilimController';
import * as client from '../src/control/client';
import * as cache from '../src/storage/cache';
import { AppState } from 'react-native';

jest.mock('react-native', () => ({
  AppState: {
    currentState: 'active',
    addEventListener: jest.fn(() => ({ remove: jest.fn() })),
  },
  Platform: { OS: 'ios' },
}));
jest.mock('../src/appearance', () => ({
  cleanupAppearanceBackgrounds: jest.fn(),
  fetchAppearanceBackground: jest.fn(),
}));
jest.mock('../src/attachments', () => ({ prepareWireAttachments: jest.fn() }));
jest.mock('../src/discovery', () => ({
  discoverMilimHosts: jest.fn(async () => []),
}));
jest.mock('../src/storage/secure', () => ({
  readDeviceCredential: jest.fn(async (host: string) => `key-${host}`),
}));
jest.mock('../src/storage/cache', () => ({
  listHosts: jest.fn(),
  readDraft: jest.fn(),
  saveDraft: jest.fn(),
  saveHost: jest.fn(),
  readTimelineTail: jest.fn(),
  saveTimelineTail: jest.fn(),
}));
jest.mock('../src/control/client', () => ({
  ...jest.requireActual('../src/control/client'),
  fetchBootstrap: jest.fn(),
  fetchTimeline: jest.fn(),
  sendCommand: jest.fn(),
  connectControlSocket: jest.fn(),
}));

let controller: ReturnType<typeof useMilimController>;
let renderer: ReactTestRenderer;
let consoleError: jest.SpyInstance;
const drafts = new Map<string, string>();
const closed: Array<() => void> = [];
function Harness() {
  controller = useMilimController();
  return null;
}
const hosts = ['a', 'b'].map(hostId => ({
  hostId,
  displayName: hostId,
  protocol: { min: 1, max: 1 },
  candidates: [`http://${hostId}`],
  lastSuccessfulUrl: `http://${hostId}`,
  lastConnectedAt: 0,
}));
const bootstrap = (host: string) => ({
  host_id: host,
  host_name: host,
  protocol: { min: 1, max: 1 },
  capabilities: {},
  threads: [{ id: `thread-${host}`, archived_at_ms: null }],
  active_runs: [],
});
async function flush() {
  await act(async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
  });
}
beforeEach(async () => {
  const originalError = console.error;
  consoleError = jest
    .spyOn(console, 'error')
    .mockImplementation((message, ...args) => {
      if (String(message).startsWith('react-test-renderer is deprecated.'))
        return;
      originalError(message, ...args);
    });
  jest.useFakeTimers();
  jest.clearAllMocks();
  drafts.clear();
  closed.length = 0;
  Object.assign(globalThis, {
    IS_REACT_ACT_ENVIRONMENT: true,
    requestAnimationFrame: (cb: () => void) => setTimeout(cb, 16),
    cancelAnimationFrame: clearTimeout,
  });
  AppState.currentState = 'active';
  jest.mocked(cache.listHosts).mockResolvedValue(hosts);
  jest
    .mocked(cache.readDraft)
    .mockImplementation(
      async (host, thread) => drafts.get(`${host}:${thread}`) ?? '',
    );
  jest
    .mocked(cache.saveDraft)
    .mockImplementation(async (host, thread, text) => {
      drafts.set(`${host}:${thread}`, text);
    });
  jest.mocked(cache.readTimelineTail).mockResolvedValue(null);
  jest
    .mocked(client.fetchBootstrap)
    .mockImplementation(async url => bootstrap(url.slice(-1)) as never);
  jest
    .mocked(client.fetchTimeline)
    .mockImplementation(
      async (_url, _key, thread) =>
        ({
          thread_id: thread,
          epoch: 'epoch',
          items: [],
          has_older: false,
          has_newer: false,
          first_seq: null,
          last_seq: null,
        } as never),
    );
  jest
    .mocked(client.connectControlSocket)
    .mockImplementation(async (_url, _key, _event, close) => {
      closed.push(close);
      return { close: jest.fn() } as never;
    });
  jest
    .mocked(client.sendCommand)
    .mockResolvedValue({ status: 'accepted' } as never);
  await act(async () => {
    renderer = create(React.createElement(Harness));
  });
  await flush();
});
afterEach(async () => {
  await act(async () => renderer.unmount());
  consoleError.mockRestore();
  jest.useRealTimers();
});

test('uncertain acceptance blocks a fresh command and explicit retry preserves ID and clears only the sent draft', async () => {
  await act(async () => controller.setDraft('hello'));
  jest
    .mocked(client.sendCommand)
    .mockRejectedValueOnce(new Error('response lost'));
  await act(async () => {
    await expect(
      controller.command(
        'turn.send',
        { display_text: 'hello', text: 'hello' },
        'thread-a',
      ),
    ).rejects.toThrow('response lost');
  });
  const original = jest.mocked(client.sendCommand).mock.calls[0][2];
  expect(controller.pendingRetry?.command).toEqual(original);
  expect(controller.lastError).toBeNull();
  await act(async () => {
    await expect(
      controller.command('turn.send', { text: 'hello' }, 'thread-a'),
    ).rejects.toThrow('previous command');
  });
  expect(client.sendCommand).toHaveBeenCalledTimes(1);
  await act(async () => {
    await controller.retryPendingCommand();
  });
  await flush();
  expect(jest.mocked(client.sendCommand).mock.calls[1][2]).toEqual(original);
  expect(controller.pendingRetry).toBeNull();
  expect(controller.draft).toBe('');
  expect(controller.acceptedRetry?.command).toEqual(original);
});

test('retry remains bound to its host and preserves later composer edits', async () => {
  await act(async () => controller.setDraft('first'));
  jest.mocked(client.sendCommand).mockRejectedValueOnce(new Error('lost'));
  await act(async () => {
    await controller
      .command('turn.send', { display_text: 'first' }, 'thread-a')
      .catch(() => {});
  });
  await act(async () => controller.setActiveHost('b'));
  await flush();
  expect(controller.pendingRetry).toBeNull();
  await act(async () => {
    expect(await controller.retryPendingCommand()).toBeNull();
  });
  expect(client.sendCommand).toHaveBeenCalledTimes(1);
  await act(async () => controller.setActiveHost('a'));
  await flush();
  await act(async () => controller.setDraft('later edit'));
  await act(async () => {
    await controller.retryPendingCommand();
  });
  await flush();
  expect(controller.draft).toBe('later edit');
  expect(jest.mocked(client.sendCommand).mock.calls[1].slice(0, 2)).toEqual([
    'http://a',
    'key-a',
  ]);
});

test('foreground socket loss reboots canonical state with no automatic command replay', async () => {
  jest.mocked(client.sendCommand).mockRejectedValueOnce(new Error('lost'));
  await act(async () => {
    await controller.command('thread.create', {}, null).catch(() => {});
    closed[0]();
  });
  expect(controller.status).toBe('offline');
  await act(async () => {
    jest.advanceTimersByTime(1000);
  });
  await flush();
  expect(controller.status).toBe('online');
  expect(client.fetchBootstrap).toHaveBeenCalledTimes(2);
  expect(client.sendCommand).toHaveBeenCalledTimes(1);
  expect(controller.pendingRetry).not.toBeNull();
});

test('explicit retry can recover while offline and sends only after bootstrap succeeds', async () => {
  jest.mocked(client.sendCommand).mockRejectedValueOnce(new Error('lost'));
  await act(async () => {
    await controller.command('thread.create', {}, null).catch(() => {});
    closed[0]();
  });
  await act(async () => {
    await controller.retryPendingCommand();
  });
  await flush();
  expect(client.sendCommand).toHaveBeenCalledTimes(2);
  expect(controller.pendingRetry).toBeNull();
});

test('failed reconnect retains the original command without sending it', async () => {
  jest.mocked(client.sendCommand).mockRejectedValueOnce(new Error('lost'));
  await act(async () => {
    await controller.command('thread.create', {}, null).catch(() => {});
    closed[0]();
  });
  jest
    .mocked(client.fetchBootstrap)
    .mockRejectedValue(new Error('unreachable'));
  await act(async () => {
    await expect(controller.retryPendingCommand()).rejects.toThrow(
      'unreachable',
    );
  });
  expect(client.sendCommand).toHaveBeenCalledTimes(1);
  expect(controller.pendingRetry).not.toBeNull();
});

test('foreground retries are bounded and backgrounding cancels a scheduled reconnect', async () => {
  jest
    .mocked(client.fetchBootstrap)
    .mockRejectedValue(new Error('unreachable'));
  await act(async () => closed[0]());
  for (const delay of [1000, 2000, 4000, 8000, 16000, 30000]) {
    await act(async () => {
      jest.advanceTimersByTime(delay);
    });
    await flush();
  }
  const attempts = jest.mocked(client.fetchBootstrap).mock.calls.length;
  await act(async () => {
    jest.advanceTimersByTime(120000);
  });
  await flush();
  expect(client.fetchBootstrap).toHaveBeenCalledTimes(attempts);
  expect(attempts).toBe(7);
  await act(async () => controller.reconnect());
  await flush();
  const listener = jest.mocked(AppState.addEventListener).mock.calls[0][1];
  await act(async () => {
    AppState.currentState = 'background';
    listener('background');
  });
  const beforeBackgroundWait = jest.mocked(client.fetchBootstrap).mock.calls
    .length;
  await act(async () => {
    jest.advanceTimersByTime(120000);
  });
  await flush();
  expect(client.fetchBootstrap).toHaveBeenCalledTimes(beforeBackgroundWait);
});

test('reconnect does not replace an unsaved composer edit with the cached draft', async () => {
  await act(async () => {
    controller.setDraft('still typing');
    closed[0]();
  });
  expect(controller.draft).toBe('still typing');
  await act(async () => {
    jest.advanceTimersByTime(1000);
  });
  await flush();
  expect(controller.draft).toBe('still typing');
});
