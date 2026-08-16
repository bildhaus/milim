declare module 'react-native-zeroconf' {
  type Listener = (...args: any[]) => void;

  export default class Zeroconf {
    scan(type?: string, protocol?: string, domain?: string, implType?: string): void;
    stop(implType?: string): void;
    on(event: string, listener: Listener): this;
    removeAllListeners(event?: string): this;
    removeDeviceListeners(): void;
  }
}
