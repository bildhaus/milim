import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ChatAttachment } from "../src/api.js";
import { MessageAttachments } from "../src/components/MessageAttachments.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function image(id: string): ChatAttachment {
  return {
    id,
    name: "image.png",
    mime: "image/png",
    size: 24,
    dataUrl: `data:image/png;base64,${id}`,
  };
}

const gallery = renderToStaticMarkup(
  createElement(MessageAttachments, {
    attachments: [image("one"), image("two"), image("three"), image("four")],
    onOpen: () => {},
  }),
);
assert(gallery.includes(">4 images<"), "Multiple pasted images should show one concise count");
assert(gallery.includes("message-image-grid-3"), "Three or more images should use the two-column gallery layout");
assert((gallery.match(/class="message-image-button"/g) ?? []).length === 4, "Every pasted image should remain directly available");
assert(gallery.includes('aria-label="Open pasted image 4 of 4"'), "Gallery controls should expose their position");
assert(!gallery.includes("message-attachment-name"), "Image galleries should not repeat clipboard filenames visibly");
assert(!gallery.includes("message-attachment-meta"), "Image galleries should not repeat MIME types visibly");

const single = renderToStaticMarkup(
  createElement(MessageAttachments, {
    attachments: [image("single")],
    onOpen: () => {},
  }),
);
assert(single.includes("message-image-grid-1"), "A single image should use the wide-preview layout");
assert(!single.includes(">1 image<"), "A single image should not add a redundant count label");

const mixed = renderToStaticMarkup(
  createElement(MessageAttachments, {
    attachments: [
      image("visual"),
      {
        id: "notes",
        name: "notes.txt",
        mime: "text/plain",
        size: 12,
        content: "hello",
      },
    ],
    onOpen: () => {},
  }),
);
assert(mixed.includes('data-testid="message-image-visual"'), "Mixed attachments should keep images in the gallery");
assert(mixed.includes('data-testid="message-attachment-notes"'), "Mixed attachments should retain compact file rows");
assert(mixed.includes(">notes.txt<"), "Non-image files should retain their filename");
assert(mixed.includes(">text/plain<"), "Non-image files should retain their type metadata");

export {};
