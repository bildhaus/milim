import Zeroconf from 'react-native-zeroconf';

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

export function discoverMilimHosts(timeoutMs = 5_000): Promise<DiscoveredHost[]> {
  return new Promise(resolve => {
    const zeroconf = new Zeroconf();
    const found = new Map<string, DiscoveredHost>();
    const finish = () => {
      zeroconf.stop();
      zeroconf.removeAllListeners();
      resolve([...found.values()]);
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
    zeroconf.scan('milim', 'tcp', 'local.');
    setTimeout(finish, timeoutMs);
  });
}
