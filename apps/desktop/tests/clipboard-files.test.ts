import assert from "node:assert/strict";
import { clipboardFiles, isDuplicateClipboardPaste } from "../src/lib/clipboardFiles.js";

function file(name: string, type: string, lastModified: number, bytes = "png"): File {
  return new File([bytes], name, { type, lastModified });
}

function item(source: File) {
  return { kind: "file" as const, getAsFile: () => source };
}

const screenshot = file("image.png", "image/png", 1);
const screenshotTwin = file("image.png", "image/png", 99);
const screenshotBmp = file("image.bmp", "image/bmp", 2, "bmp");
const notes = file("notes.md", "text/markdown", 3, "notes");
const readme = file("README.md", "text/markdown", 4, "readme");

const sameImageTwice = clipboardFiles({
  files: [screenshot],
  items: [item(screenshotTwin)],
});
assert.equal(sameImageTwice.length, 1, "files and items should not attach the same screenshot twice");
assert.equal(sameImageTwice[0]?.name, "image.png");

const pngAndBmp = clipboardFiles({
  files: [screenshotBmp],
  items: [item(screenshot)],
});
assert.equal(pngAndBmp.length, 1, "Windows PNG/BMP clipboard twins should collapse to one image");
assert.equal(pngAndBmp[0]?.type, "image/png", "PNG should win over BMP");

const namedFiles = clipboardFiles({
  files: [notes, readme],
  items: [item(notes)],
});
assert.equal(namedFiles.length, 2, "distinct named files should still attach together");
assert.deepEqual(namedFiles.map((entry) => entry.name).sort(), ["README.md", "notes.md"]);

const sameSizeNamedFiles = clipboardFiles({
  files: [file("alpha.txt", "text/plain", 5, "one"), file("beta.txt", "text/plain", 6, "two")],
});
assert.equal(sameSizeNamedFiles.length, 2, "same-size files with distinct names should not be treated as clipboard twins");

const twoFolderImages = clipboardFiles({
  files: [file("image.png", "image/png", 1, "one"), file("image.jpg", "image/jpeg", 2, "two-bytes")],
});
assert.equal(twoFolderImages.length, 2, "two real files should not collapse just because their names look generic");

const twoAnonymousPngs = clipboardFiles({
  files: [file("image.png", "image/png", 1, "first-image"), file("", "image/png", 2, "second-image")],
});
assert.equal(twoAnonymousPngs.length, 2, "two different anonymous PNGs should stay separate");

const first = isDuplicateClipboardPaste(sameImageTwice, null, 1_000);
assert.equal(first.duplicate, false, "the first screenshot paste should attach");
const second = isDuplicateClipboardPaste(sameImageTwice, first.stamp, 1_200);
assert.equal(second.duplicate, true, "a second paste event in the same gesture should be ignored");
const later = isDuplicateClipboardPaste(sameImageTwice, first.stamp, 1_500);
assert.equal(later.duplicate, false, "a later intentional paste should attach again");
