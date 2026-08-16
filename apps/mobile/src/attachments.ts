import ReactNativeBlobUtil from 'react-native-blob-util';
import {pick, keepLocalCopy} from '@react-native-documents/picker';
import {launchCamera, launchImageLibrary} from 'react-native-image-picker';
import type {ControlAttachmentV1} from './control/types';
import {
  forgetTemporaryFile,
  staleTemporaryFiles,
  trackTemporaryFile,
} from './storage/cache';

export const MAX_ATTACHMENTS = 6;
export const MAX_ATTACHMENT_BYTES = 2 * 1024 * 1024;
const MAX_TEXT_ATTACHMENT_CHARS = 128 * 1024;

function id(): string {
  return `attachment-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

async function fromLocalFile(
  uri: string,
  name: string,
  mime: string,
  size: number,
): Promise<ControlAttachmentV1> {
  const path = uri.replace(/^file:\/\//, '');
  const stat = await ReactNativeBlobUtil.fs.stat(path);
  const actualSize = Math.max(size, Number(stat.size ?? 0));
  if (actualSize <= 0) {
    throw new Error(`${name} is empty or its size could not be read.`);
  }
  if (actualSize > MAX_ATTACHMENT_BYTES) {
    throw new Error(`${name} is larger than the 2 MiB attachment limit.`);
  }
  await trackTemporaryFile(uri);
  if (
    mime.startsWith('text/') ||
    /(?:json|xml|yaml|javascript|typescript|csv|markdown)/i.test(mime)
  ) {
    const content = await ReactNativeBlobUtil.fs.readFile(path, 'utf8');
    if (content.length > MAX_TEXT_ATTACHMENT_CHARS) {
      throw new Error(`${name} exceeds the 128 KiB text attachment limit.`);
    }
    return {
      id: id(),
      name,
      mime,
      size: actualSize,
      content,
      local_uri: uri,
    };
  }
  const base64 = await ReactNativeBlobUtil.fs.readFile(path, 'base64');
  return {
    id: id(),
    name,
    mime,
    size: actualSize,
    data_url: `data:${mime};base64,${base64}`,
    local_uri: uri,
  };
}

export async function pickFiles(): Promise<ControlAttachmentV1[]> {
  const selected = await pick({allowMultiSelection: true});
  const files = selected.slice(0, MAX_ATTACHMENTS).map(file => ({
    uri: file.uri,
    fileName: file.name ?? id(),
  }));
  if (!files.length) return [];
  const copies = await keepLocalCopy({
    files: files as [typeof files[number], ...typeof files],
    destination: 'cachesDirectory',
  });
  return Promise.all(
    copies.map((copy, index) => {
      if (copy.status !== 'success') throw new Error(copy.copyError);
      const source = selected[index];
      return fromLocalFile(
        copy.localUri,
        source.name ?? `file-${index + 1}`,
        source.type ?? 'application/octet-stream',
        source.size ?? 0,
      );
    }),
  );
}

export async function pickPhoto(source: 'camera' | 'library'): Promise<ControlAttachmentV1[]> {
  const result = await (source === 'camera'
    ? launchCamera({mediaType: 'photo', quality: 0.8})
    : launchImageLibrary({mediaType: 'photo', quality: 0.8, selectionLimit: MAX_ATTACHMENTS}));
  if (result.didCancel) return [];
  if (result.errorCode) throw new Error(result.errorMessage ?? result.errorCode);
  return Promise.all(
    (result.assets ?? []).slice(0, MAX_ATTACHMENTS).map((asset, index) => {
      if (!asset.uri) throw new Error('The selected photo has no readable local URI.');
      return fromLocalFile(
        asset.uri,
        asset.fileName ?? `photo-${index + 1}.jpg`,
        asset.type ?? 'image/jpeg',
        asset.fileSize ?? 0,
      );
    }),
  );
}

export async function cleanupAttachments(attachments: ControlAttachmentV1[]): Promise<void> {
  await Promise.all(
    attachments.map(async attachment => {
      const path = attachment.local_uri?.replace(/^file:\/\//, '');
      if (!path) return;
      try {
        if (await ReactNativeBlobUtil.fs.exists(path)) await ReactNativeBlobUtil.fs.unlink(path);
        await forgetTemporaryFile(attachment.local_uri!);
      } catch {
        // Bounded cache cleanup on next launch is the fallback for OS-owned or
        // already-removed temporary files.
      }
    }),
  );
}

export async function cleanupStaleAttachments(): Promise<void> {
  const uris = await staleTemporaryFiles();
  for (const uri of uris) {
    await cleanupAttachments([{id: '', name: '', mime: '', size: 0, local_uri: uri}]);
  }
}

export function wireAttachments(attachments: ControlAttachmentV1[]): ControlAttachmentV1[] {
  return attachments.map(({local_uri: _localUri, ...attachment}) => attachment);
}

export function promptWithAttachments(text: string, attachments: ControlAttachmentV1[]): string {
  const blocks = attachments
    .filter(attachment => attachment.content !== undefined)
    .map(attachment => [
      `--- attachment name=${attachment.name} mime=${attachment.mime} size=${attachment.size} ---`,
      attachment.content || '[Empty text attachment]',
      '--- end attachment ---',
    ].join('\n'));
  return blocks.length
    ? [text, '[Attached files]', ...blocks, '[/Attached files]'].filter(Boolean).join('\n\n')
    : text;
}
