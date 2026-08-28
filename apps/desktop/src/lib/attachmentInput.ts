export const MAX_ATTACHMENT_IMAGE_BYTES = 2 * 1024 * 1024;
export const MAX_ATTACHMENT_TEXT_BYTES = 128 * 1024;
export const MAX_DESKTOP_ATTACHMENTS = 12;
export const MAX_OUTBOUND_IMAGE_DATA_URL_BYTES = 20 * 1024 * 1024;
export const OMITTED_IMAGE_NOTE =
  "[Earlier image attachments were omitted from this request to stay within Milim's size limit.]";

const SUPPORTED_IMAGE_MIMES = new Set([
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
]);

type ImageAttachment = {
  name?: string;
  mime: string;
  size: number;
  dataUrl?: string;
  truncated?: boolean;
};

type ImageMessage<T extends ImageAttachment> = {
  role: string;
  attachments?: readonly T[];
};

export type AccountRuntimeImage = {
  media_type: string;
  data: string;
};

export function isSupportedImageMime(mime: string): boolean {
  return SUPPORTED_IMAGE_MIMES.has(mime.toLowerCase());
}

export function assertValidImageAttachment(attachment: ImageAttachment): void {
  if (!attachment.mime.toLowerCase().startsWith("image/")) return;
  const label = attachment.name || "Image";
  if (!isSupportedImageMime(attachment.mime)) {
    throw new Error(`${label} must be PNG, JPEG, WebP, or GIF.`);
  }
  if (
    attachment.size <= 0 ||
    attachment.size > MAX_ATTACHMENT_IMAGE_BYTES ||
    attachment.truncated
  ) {
    throw new Error(`${label} must be no larger than 2 MB.`);
  }
  const prefix = `data:${attachment.mime.toLowerCase()};base64,`;
  if (!attachment.dataUrl?.toLowerCase().startsWith(prefix)) {
    throw new Error(`${label} could not be read as image data.`);
  }
}

export function selectOutboundImageAttachments<T extends ImageAttachment>(
  messages: readonly ImageMessage<T>[],
): Set<T> {
  const groups = messages
    .map((message, index) => ({
      index,
      images:
        message.role === "user"
          ? (message.attachments ?? []).filter((attachment) => {
              if (!attachment.mime.toLowerCase().startsWith("image/"))
                return false;
              assertValidImageAttachment(attachment);
              return true;
            })
          : [],
    }))
    .filter((group) => group.images.length > 0);
  if (!groups.length) return new Set();

  let latestUserIndex = -1;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].role === "user") {
      latestUserIndex = index;
      break;
    }
  }
  const selected = new Set<T>();
  let used = 0;

  for (let index = groups.length - 1; index >= 0; index -= 1) {
    const group = groups[index];
    const bytes = group.images.reduce(
      (total, image) => total + image.dataUrl!.length,
      0,
    );
    if (group.index === latestUserIndex) {
      if (bytes > MAX_OUTBOUND_IMAGE_DATA_URL_BYTES) {
        throw new Error(
          "This message contains too much image data. Remove some images and retry.",
        );
      }
    } else if (used + bytes > MAX_OUTBOUND_IMAGE_DATA_URL_BYTES) {
      break;
    }
    for (const image of group.images) selected.add(image);
    used += bytes;
  }
  return selected;
}

export function accountRuntimeImage(
  attachment: ImageAttachment,
): AccountRuntimeImage | null {
  if (!attachment.mime.toLowerCase().startsWith("image/")) return null;
  assertValidImageAttachment(attachment);
  const separator = attachment.dataUrl!.indexOf(",");
  return {
    media_type: attachment.mime.toLowerCase(),
    data: attachment.dataUrl!.slice(separator + 1),
  };
}

export function readBrowserAttachmentDataUrl(
  file: File,
  mime: string,
): Promise<string | undefined> {
  if (!mime.toLowerCase().startsWith("image/")) return Promise.resolve(undefined);
  assertValidImageAttachment({
    name: file.name,
    mime,
    size: file.size,
    dataUrl: `data:${mime};base64,placeholder`,
  });
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result !== "string") {
        reject(new Error(`${file.name || "Image"} could not be read as image data.`));
        return;
      }
      const separator = reader.result.indexOf(",");
      if (separator < 0) {
        reject(new Error(`${file.name || "Image"} could not be read as image data.`));
        return;
      }
      resolve(`data:${mime.toLowerCase()};base64,${reader.result.slice(separator + 1)}`);
    };
    reader.onerror = () =>
      reject(new Error(`${file.name || "Image"} could not be read as image data.`));
    reader.readAsDataURL(file);
  });
}

export function isTextLikeAttachmentMime(mime: string): boolean {
  const normalized = mime.toLowerCase();
  return (
    normalized.startsWith("text/") ||
    normalized === "application/json" ||
    normalized === "application/xml" ||
    normalized === "application/javascript"
  );
}

export async function browserAttachment(
  file: File,
  mime: string,
  id: string,
): Promise<{
  id: string;
  name: string;
  mime: string;
  size: number;
  content?: string;
  dataUrl?: string;
  truncated: boolean;
}> {
  const textLike = isTextLikeAttachmentMime(mime);
  const image = mime.toLowerCase().startsWith("image/");
  if (!textLike && !image) {
    throw new Error(
      `${file.name || "Attachment"} is not a supported text or image file.`,
    );
  }
  const [content, dataUrl] = await Promise.all([
    textLike
      ? file.slice(0, MAX_ATTACHMENT_TEXT_BYTES).text()
      : Promise.resolve(undefined),
    readBrowserAttachmentDataUrl(file, mime),
  ]);
  return {
    id,
    name: file.name || "attachment",
    mime,
    size: file.size,
    content,
    dataUrl,
    truncated: textLike && file.size > MAX_ATTACHMENT_TEXT_BYTES,
  };
}
