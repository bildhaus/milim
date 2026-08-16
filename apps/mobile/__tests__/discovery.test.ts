interface MockZeroconfInstance {
  emit(event: string, value?: unknown): void;
  scan: jest.Mock;
  stop: jest.Mock;
  removeAllListeners: jest.Mock;
  removeDeviceListeners: jest.Mock;
}

const mockZeroconfInstances: MockZeroconfInstance[] = [];
const mockFetchMobileHostProbe = jest.fn();

class MockZeroconf implements MockZeroconfInstance {
  listeners = new Map<string, (value?: unknown) => void>();
  scan = jest.fn();
  stop = jest.fn();
  removeAllListeners = jest.fn();
  removeDeviceListeners = jest.fn();

  constructor() {
    mockZeroconfInstances.push(this);
  }

  on(event: string, listener: (value?: unknown) => void) {
    this.listeners.set(event, listener);
    return this;
  }

  emit(event: string, value?: unknown) {
    this.listeners.get(event)?.(value);
  }
}

jest.mock('react-native', () => ({Platform: {OS: 'android'}}));
jest.doMock('react-native-zeroconf', () => ({__esModule: true, default: MockZeroconf}));
jest.mock('../src/control/client', () => ({
  fetchMobileHostProbe: mockFetchMobileHostProbe,
}));

function discoveryModule(): typeof import('../src/discovery') {
  return require('../src/discovery');
}

describe('milim desktop discovery', () => {
  beforeEach(() => {
    jest.useFakeTimers();
    mockZeroconfInstances.length = 0;
    mockFetchMobileHostProbe.mockReset();
    Object.assign(globalThis, {__DEV__: false});
  });

  afterEach(() => {
    jest.useRealTimers();
  });

  test('returns resolved LAN desktops and cleans up Android DNSSD discovery', async () => {
    const {discoverMilimHosts} = discoveryModule();
    const result = discoverMilimHosts(100);
    const zeroconf = mockZeroconfInstances[0];

    expect(zeroconf.scan).toHaveBeenCalledWith('milim', 'tcp', 'local.', 'DNSSD');
    zeroconf.emit('resolved', {
      name: 'milim desktop',
      port: 60959,
      addresses: ['fe80::1', '192.168.1.20'],
      txt: {host_id: 'host-1'},
    });
    jest.advanceTimersByTime(100);

    await expect(result).resolves.toEqual([{
      name: 'milim desktop',
      hostId: 'host-1',
      endpoint: 'http://192.168.1.20:60959',
    }]);
    expect(zeroconf.stop).toHaveBeenCalledWith('DNSSD');
    expect(zeroconf.removeDeviceListeners).toHaveBeenCalledTimes(1);
  });

  test('rejects native discovery errors after cleaning up listeners', async () => {
    const {discoverMilimHosts} = discoveryModule();
    const result = discoverMilimHosts(100);
    const zeroconf = mockZeroconfInstances[0];
    zeroconf.emit('error', new Error('network unavailable'));

    await expect(result).rejects.toThrow('network unavailable');
    expect(zeroconf.removeAllListeners).toHaveBeenCalledTimes(1);
    expect(zeroconf.removeDeviceListeners).toHaveBeenCalledTimes(1);
  });

  test('shares one native scan across concurrent callers', async () => {
    const {discoverMilimHosts} = discoveryModule();
    const first = discoverMilimHosts(100);
    const second = discoverMilimHosts(50);

    expect(second).toBe(first);
    expect(mockZeroconfInstances).toHaveLength(1);
    jest.advanceTimersByTime(100);
    await expect(first).resolves.toEqual([]);
  });

  test('uses the Android host bridge in development without waiting for mDNS', async () => {
    Object.assign(globalThis, {__DEV__: true});
    mockFetchMobileHostProbe.mockResolvedValue({
      service: 'milim-mobile-control',
      host_id: 'host-local',
      host_name: 'Development desktop',
      protocol: {min: 1, max: 1},
    });
    const {discoverMilimHosts} = discoveryModule();

    await expect(discoverMilimHosts(100)).resolves.toEqual([{
      name: 'Development desktop',
      hostId: 'host-local',
      endpoint: 'http://10.0.2.2:7378',
    }]);
    expect(mockFetchMobileHostProbe).toHaveBeenCalledWith(
      'http://10.0.2.2:7378',
      expect.any(AbortSignal),
    );
    expect(mockZeroconfInstances).toHaveLength(0);
  });

  test('maps iOS simulator and Android emulator to their host loopbacks', () => {
    const {simulatorHostEndpoint} = discoveryModule();

    expect(simulatorHostEndpoint('android')).toBe('http://10.0.2.2:7378');
    expect(simulatorHostEndpoint('ios')).toBe('http://127.0.0.1:7378');
    expect(simulatorHostEndpoint('windows')).toBeNull();
  });
});
