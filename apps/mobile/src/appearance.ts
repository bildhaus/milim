import ReactNativeBlobUtil from 'react-native-blob-util';
import {normalizeEndpoint} from './control/client';

const BACKGROUND_PREFIX = 'milim-appearance-background-';
const MAX_BACKGROUND_BYTES = 8 * 1024 * 1024;

function safePart(value: string): string {
  return value.replace(/[^a-z0-9._-]/gi, '_').slice(0, 120);
}

function backgroundPath(hostId: string, revision: string): string {
  return `${ReactNativeBlobUtil.fs.dirs.CacheDir}/${BACKGROUND_PREFIX}${safePart(hostId)}-${safePart(revision)}.image`;
}

function base64ByteLength(value: string): number {
  const encoded = value.replace(/\s/g, '');
  const padding = encoded.endsWith('==') ? 2 : encoded.endsWith('=') ? 1 : 0;
  return Math.floor((encoded.length * 3) / 4) - padding;
}

function contentLength(headers: unknown): number | null {
  if (!headers || typeof headers !== 'object') return null;
  const entry = Object.entries(headers).find(([key]) => key.toLowerCase() === 'content-length');
  const value = Number(entry?.[1]);
  return Number.isFinite(value) && value > 0 ? value : null;
}

async function hasCompleteCache(path: string): Promise<boolean> {
  const marker = `${path}.complete`;
  if (!(await ReactNativeBlobUtil.fs.exists(path)) || !(await ReactNativeBlobUtil.fs.exists(marker))) {
    return false;
  }
  const [stat, expected] = await Promise.all([
    ReactNativeBlobUtil.fs.stat(path),
    ReactNativeBlobUtil.fs.readFile(marker, 'utf8'),
  ]);
  const size = Number(stat.size ?? 0);
  return size > 0 && size === Number(expected);
}

export async function fetchAppearanceBackground(
  endpoint: string,
  deviceKey: string,
  hostId: string,
  revision: string,
): Promise<string> {
  const path = backgroundPath(hostId, revision);
  if (await hasCompleteCache(path)) return `file://${path}`;
  const partialPath = `${path}.part`;
  const markerPath = `${path}.complete`;
  await ReactNativeBlobUtil.fs.unlink(partialPath).catch(() => undefined);
  const response = await ReactNativeBlobUtil.fetch(
    'GET',
    `${normalizeEndpoint(endpoint)}/control/v1/appearance/background?revision=${encodeURIComponent(revision)}`,
    {Accept: 'image/*', Authorization: `Bearer ${deviceKey}`},
  );
  const status = response.info().status;
  if (status < 200 || status >= 300) {
    if (await ReactNativeBlobUtil.fs.exists(path)) await ReactNativeBlobUtil.fs.unlink(path);
    throw new Error(
      status === 409
        ? 'Desktop appearance changed while its background was loading.'
        : `Could not load the desktop background (HTTP ${status}).`,
    );
  }
  const payload = String(await Promise.resolve(response.base64()));
  const payloadSize = base64ByteLength(payload);
  const expectedSize = contentLength(response.info().headers);
  if (payloadSize <= 0) {
    throw new Error('The desktop background was empty.');
  }
  if (payloadSize > MAX_BACKGROUND_BYTES) {
    throw new Error('The desktop background exceeds the mobile size limit.');
  }
  if (expectedSize !== null && payloadSize !== expectedSize) {
    throw new Error('The desktop background download was incomplete.');
  }
  await ReactNativeBlobUtil.fs.writeFile(partialPath, payload, 'base64');
  const stat = await ReactNativeBlobUtil.fs.stat(partialPath);
  if (Number(stat.size ?? 0) !== payloadSize) {
    await ReactNativeBlobUtil.fs.unlink(partialPath).catch(() => undefined);
    throw new Error('The desktop background download was incomplete.');
  }
  await ReactNativeBlobUtil.fs.unlink(path).catch(() => undefined);
  await ReactNativeBlobUtil.fs.unlink(markerPath).catch(() => undefined);
  await ReactNativeBlobUtil.fs.mv(partialPath, path);
  await ReactNativeBlobUtil.fs.writeFile(markerPath, String(payloadSize), 'utf8');
  return `file://${path}`;
}

export async function cleanupAppearanceBackgrounds(
  hostId: string,
  keepRevision?: string,
): Promise<void> {
  const prefix = `${BACKGROUND_PREFIX}${safePart(hostId)}-`;
  const keep = keepRevision ? `${prefix}${safePart(keepRevision)}.image` : null;
  const keepFiles = new Set(keep ? [keep, `${keep}.complete`] : []);
  const entries = await ReactNativeBlobUtil.fs.ls(ReactNativeBlobUtil.fs.dirs.CacheDir).catch(() => []);
  await Promise.all(
    entries
      .filter(name => name.startsWith(prefix) && !keepFiles.has(name))
      .map(name =>
        ReactNativeBlobUtil.fs
          .unlink(`${ReactNativeBlobUtil.fs.dirs.CacheDir}/${name}`)
          .catch(() => undefined),
      ),
  );
}
