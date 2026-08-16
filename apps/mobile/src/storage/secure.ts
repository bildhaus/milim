import * as Keychain from 'react-native-keychain';

const SERVICE_PREFIX = 'app.milim.mobile.host.';

export async function saveDeviceCredential(hostId: string, deviceKey: string): Promise<void> {
  await Keychain.setGenericPassword(hostId, deviceKey, {
    service: `${SERVICE_PREFIX}${hostId}`,
    accessible: Keychain.ACCESSIBLE.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
  });
}

export async function readDeviceCredential(hostId: string): Promise<string | null> {
  const credential = await Keychain.getGenericPassword({
    service: `${SERVICE_PREFIX}${hostId}`,
  });
  return credential ? credential.password : null;
}

export async function removeDeviceCredential(hostId: string): Promise<void> {
  await Keychain.resetGenericPassword({service: `${SERVICE_PREFIX}${hostId}`});
}
