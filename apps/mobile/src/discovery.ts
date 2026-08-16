import Zeroconf from 'react-native-zeroconf';
import {Platform} from 'react-native';
import {fetchMobileHostProbe} from './control/client';

export interface DiscoveredHost {
  name: string;
  hostId: string | null;
  endpoint: string;
}

interface ZeroconfService {
  name?: string;
  port?: number;
  addresses?: string[];
  txt?: Record<string, string>;
}

let activeDiscovery: Promise<DiscoveredHost[]> | null = null;

const SIMULATOR_DISCOVERY_PORT = 7378;
const SIMULATOR_PROBE_TIMEOUT_MS = 800;

export function simulatorHostEndpoint(platform: string): string | null {
  if (platform === 'android') {
    return `http://10.0.2.2:${SIMULATOR_DISCOVERY_PORT}`;
  }
  if (platform === 'ios') {
    return `http://127.0.0.1:${SIMULATOR_DISCOVERY_PORT}`;
  }
  return null;
}

async function discoverSimulatorHost(): Promise<DiscoveredHost[]> {
  if (!__DEV__) return [];
  const endpoint = simulatorHostEndpoint(Platform.OS);
  if (!endpoint) return [];

  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), SIMULATOR_PROBE_TIMEOUT_MS);
  try {
    const host = await fetchMobileHostProbe(endpoint, controller.signal);
    if (host.service !== 'milim-mobile-control' || !host.host_id || !host.host_name) {
      return [];
    }
    return [{name: host.host_name, hostId: host.host_id, endpoint}];
  } catch {
    return [];
  } finally {
    clearTimeout(timeout);
  }
}

function startNativeMilimHostDiscovery(timeoutMs: number): Promise<DiscoveredHost[]> {
  return new Promise((resolve, reject) => {
    const zeroconf = new Zeroconf();
    const found = new Map<string, DiscoveredHost>();
    const implementation = Platform.OS === 'android' ? 'DNSSD' : undefined;
    let finished = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const finish = (error?: unknown) => {
      if (finished) return;
      finished = true;
      if (timer) clearTimeout(timer);
      let cleanupError: unknown = null;
      try {
        zeroconf.stop(implementation);
      } catch (reason) {
        cleanupError = reason;
      }
      zeroconf.removeAllListeners();
      zeroconf.removeDeviceListeners();
      const failure = error ?? cleanupError;
      if (failure) {
        reject(failure instanceof Error ? failure : new Error(String(failure)));
        return;
      }
      resolve([...found.values()].sort((left, right) => left.name.localeCompare(right.name)));
    };
    zeroconf.on('resolved', (service: ZeroconfService) => {
      const address = service.addresses?.find(value => value.includes('.'));
      if (!address || !service.port) return;
      const endpoint = `http://${address}:${service.port}`;
      found.set(service.txt?.host_id ?? endpoint, {
        name: service.name ?? 'milim desktop',
        hostId: service.txt?.host_id ?? null,
        endpoint,
      });
    });
    zeroconf.on('error', finish);
    try {
      zeroconf.scan('milim', 'tcp', 'local.', implementation);
      timer = setTimeout(finish, timeoutMs);
    } catch (error) {
      finish(error);
    }
  });
}

function mergeDiscoveredHosts(...groups: DiscoveredHost[][]): DiscoveredHost[] {
  const hosts = new Map<string, DiscoveredHost>();
  for (const group of groups) {
    for (const host of group) {
      hosts.set(host.hostId ?? host.endpoint, host);
    }
  }
  return [...hosts.values()].sort((left, right) => left.name.localeCompare(right.name));
}

async function startMilimHostDiscovery(timeoutMs: number): Promise<DiscoveredHost[]> {
  if (!__DEV__) return startNativeMilimHostDiscovery(timeoutMs);
  const nativeDiscovery = startNativeMilimHostDiscovery(timeoutMs).then(
    hosts => ({hosts, error: null}),
    error => ({hosts: [] as DiscoveredHost[], error}),
  );
  const [native, simulatorHosts] = await Promise.all([
    nativeDiscovery,
    discoverSimulatorHost(),
  ]);
  if (native.error && simulatorHosts.length === 0) throw native.error;
  return mergeDiscoveredHosts(native.hosts, simulatorHosts);
}

export function discoverMilimHosts(timeoutMs = 5_000): Promise<DiscoveredHost[]> {
  if (activeDiscovery) return activeDiscovery;
  const discovery = startMilimHostDiscovery(timeoutMs);
  activeDiscovery = discovery;
  const clear = () => {
    if (activeDiscovery === discovery) activeDiscovery = null;
  };
  void discovery.then(clear, clear);
  return discovery;
}
