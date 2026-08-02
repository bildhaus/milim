import { strict as assert } from "node:assert";
import {
  wireMessageContent,
  wireMessages,
} from "../src/lib/attachmentWire.js";
import { MAX_OUTBOUND_IMAGE_DATA_URL_BYTES } from "../src/lib/attachmentInput.js";
import { accountRuntimeInputFromMessages } from "../src/lib/turnRuntime.js";
import type { ChatMessage } from "../src/api.js";

const message: ChatMessage = {
  role: "user",
  content: "Read these.",
  attachments: [
    {
      id: "text",
      name: "note.txt",
      mime: "text/plain",
      size: 5,
      content: "hello",
    },
    {
      id: "image",
      name: "screen.png",
      mime: "image/png",
      size: 4,
      dataUrl: "data:image/png;base64,AAAA",
    },
  ],
};

const textOnly = wireMessageContent(message);
assert.match(textOnly, /Read these\./);
assert.match(textOnly, /hello/);
assert.match(textOnly, /text-only view cannot receive image pixels/);

const wired = wireMessages([message]);
assert.equal(wired.length, 1);
assert.equal(wired[0].role, "user");

const content = wired[0].content;
assert(Array.isArray(content));
const parts = content;
assert.equal(parts.length, 2);
assert.equal(parts[0].type, "text");
assert.match(parts[0].text, /Read these\./);
assert.match(parts[0].text, /hello/);
assert.doesNotMatch(parts[0].text, /screen\.png/);
assert.doesNotMatch(parts[0].text, /OCR/);
assert.equal(parts[1].type, "image_url");
assert.equal(parts[1].image_url.url, "data:image/png;base64,AAAA");

const textAttachmentOnly = wireMessages([
  {
    role: "user",
    content: "Read this note.",
    attachments: [
      {
        id: "text",
        name: "note.txt",
        mime: "text/plain",
        size: 5,
        content: "hello",
      },
    ],
  },
]);
assert.equal(typeof textAttachmentOnly[0].content, "string");
assert.match(textAttachmentOnly[0].content as string, /Read this note\./);
assert.match(textAttachmentOnly[0].content as string, /hello/);

const imageDataUrl = (id: string, bytes: number) => {
  const prefix = "data:image/png;base64,";
  return {
    id,
    name: `${id}.png`,
    mime: "image/png",
    size: 1,
    dataUrl: `${prefix}${"A".repeat(Math.max(0, bytes - prefix.length))}`,
  };
};
const mib = 1024 * 1024;
const oldImages = [
  imageDataUrl("old-a", Math.floor(4.5 * mib)),
  imageDataUrl("old-b", Math.floor(4.5 * mib)),
];
const recentImage = imageDataUrl("recent", 12 * mib);
const currentImage = imageDataUrl("current", 32);
const boundedMessages: ChatMessage[] = [
  { role: "user", content: "Old images", attachments: oldImages },
  { role: "user", content: "Recent image", attachments: [recentImage] },
  { role: "user", content: "Current image", attachments: [currentImage] },
];
for (const item of boundedMessages) {
  for (const attachment of item.attachments ?? []) Object.freeze(attachment);
  if (item.attachments) Object.freeze(item.attachments);
  Object.freeze(item);
}
Object.freeze(boundedMessages);

const boundedWire = wireMessages(boundedMessages);
assert.equal(typeof boundedWire[0].content, "string");
assert.match(
  boundedWire[0].content as string,
  /Earlier image attachments were omitted/,
);
assert(Array.isArray(boundedWire[1].content));
assert(Array.isArray(boundedWire[2].content));
const accountInput = accountRuntimeInputFromMessages(boundedMessages);
assert.equal(accountInput.images.length, 2);
assert.equal(accountInput.images[0].data, recentImage.dataUrl.slice(recentImage.dataUrl.indexOf(",") + 1));
assert.equal(accountInput.images[1].data, currentImage.dataUrl.slice(currentImage.dataUrl.indexOf(",") + 1));
assert.match(accountInput.prompt, /Earlier image attachments were omitted/);

const oversizedCurrent: ChatMessage[] = [
  {
    role: "user",
    content: "Too many current pixels",
    attachments: [
      imageDataUrl("oversized-current", MAX_OUTBOUND_IMAGE_DATA_URL_BYTES + 1),
    ],
  },
];
assert.throws(
  () => wireMessages(oversizedCurrent),
  /This message contains too much image data/,
);
assert.throws(
  () => accountRuntimeInputFromMessages(oversizedCurrent),
  /This message contains too much image data/,
);

const reviewMessage: ChatMessage = {
  role: "user",
  content: "Please address the review.",
  reviewComments: Array.from({ length: 24 }, (_, index) => ({
    id: `review-${index}`,
    surface: "diff" as const,
    filePath: "src/app.ts",
    side: "new" as const,
    startLine: index + 1,
    endLine: index + 1,
    selectedText: `line ${index + 1}`,
    body: `Comment ${index + 1}`,
    timestamp: index + 1,
  })),
};
const reviewWire = wireMessageContent(reviewMessage);
assert.match(reviewWire, /<milim_review_context>/);
const reviewJson = reviewWire.match(/<milim_review_context>(.*)<\/milim_review_context>/)?.[1];
assert(reviewJson);
assert.equal(JSON.parse(reviewJson).length, 20);
assert.equal(reviewMessage.reviewComments?.length, 24);
const accountReviewPrompt = accountRuntimeInputFromMessages([reviewMessage]).prompt;
assert.match(accountReviewPrompt, /<milim_review_context>/);
assert.match(accountReviewPrompt, /Comment 1/);
