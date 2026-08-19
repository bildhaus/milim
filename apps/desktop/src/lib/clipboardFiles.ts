const IMAGE_RANK: Record<string, number> = {
  "image/png": 5,
  "image/jpeg": 4,
  "image/jpg": 4,
  "image/webp": 3,
  "image/gif": 2,
  "image/bmp": 1,
  "image/x-ms-bmp": 1,
};

const ANONYMOUS_IMAGE_NAME = /^(image|untitled|screenshot|clipboard|paste)(\.\w+)?$/i;
const DUPLICATE_PASTE_WINDOW_MS = 400;

export type ClipboardFileSource = {
  files?: ArrayLike<File> | null;
  items?: ArrayLike<{
    kind: string;
    getAsFile: () => File | null;
  }> | null;
};

export type ClipboardPasteStamp = {
  key: string;
  at: number;
};

export function clipboardFiles(data: ClipboardFileSource): File[] {
  const collected: File[] = [];
  for (const file of Array.from(data.files ?? [])) collected.push(file);
  for (const item of Array.from(data.items ?? [])) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (file) collected.push(file);
  }
  return collapseAnonymousClipboardImages(uniqueClipboardFiles(collected));
}

export function isDuplicateClipboardPaste(
  files: File[],
  previous: ClipboardPasteStamp | null,
  now = Date.now(),
  windowMs = DUPLICATE_PASTE_WINDOW_MS,
): { duplicate: boolean; stamp: ClipboardPasteStamp } {
  const stamp = {
    key: files.map((file) => `${normalizeName(file.name)}:${file.size}:${normalizeType(file)}`).join("\0"),
    at: now,
  };
  if (
    previous
    && previous.key
    && previous.key === stamp.key
    && now - previous.at < windowMs
  ) {
    return { duplicate: true, stamp: previous };
  }
  return { duplicate: false, stamp };
}

function uniqueClipboardFiles(files: File[]): File[] {
  const byKey = new Map<string, File>();
  for (const file of files) {
    const key = `${normalizeName(file.name)}:${file.size}:${normalizeType(file)}`;
    const existing = byKey.get(key);
    byKey.set(key, existing ? preferClipboardFile(existing, file) : file);
  }
  return Array.from(byKey.values());
}

function collapseAnonymousClipboardImages(files: File[]): File[] {
  const named = files.filter((file) => !isAnonymousClipboardImage(file));
  const anonymous = files.filter(isAnonymousClipboardImage);
  if (anonymous.length < 2) return files;

  const bitmaps: File[] = [];
  const bySize = new Map<number, File[]>();
  for (const file of anonymous) {
    if (isBitmapType(normalizeType(file))) {
      bitmaps.push(file);
      continue;
    }
    const group = bySize.get(file.size) ?? [];
    group.push(file);
    bySize.set(file.size, group);
  }

  const others = Array.from(bySize.values()).map((group) =>
    group.reduce((best, file) => preferClipboardFile(best, file)),
  );
  // Windows screenshot pastes often expose both PNG and a larger BMP copy.
  if (bitmaps.length && others.length === 1) {
    return [...named, preferClipboardFile(others[0], bitmaps[0])];
  }
  return [...named, ...others, ...bitmaps];
}

function isBitmapType(type: string): boolean {
  return type === "image/bmp" || type === "image/x-ms-bmp";
}

function isAnonymousClipboardImage(file: File): boolean {
  if (!normalizeType(file).startsWith("image/")) return false;
  const name = normalizeName(file.name);
  return !name || ANONYMOUS_IMAGE_NAME.test(name);
}

function preferClipboardFile(left: File, right: File): File {
  const rankLeft = IMAGE_RANK[normalizeType(left)] ?? 0;
  const rankRight = IMAGE_RANK[normalizeType(right)] ?? 0;
  if (rankLeft !== rankRight) return rankLeft >= rankRight ? left : right;
  if (left.name && !right.name) return left;
  if (right.name && !left.name) return right;
  return left;
}

function normalizeType(file: File): string {
  const type = file.type.trim().toLowerCase();
  if (type) return type;
  const name = normalizeName(file.name);
  if (name.endsWith(".png")) return "image/png";
  if (name.endsWith(".jpg") || name.endsWith(".jpeg")) return "image/jpeg";
  if (name.endsWith(".webp")) return "image/webp";
  if (name.endsWith(".gif")) return "image/gif";
  if (name.endsWith(".bmp")) return "image/bmp";
  return "";
}

function normalizeName(name: string): string {
  return name.trim().toLowerCase();
}
