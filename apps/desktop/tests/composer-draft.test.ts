import { deepEqual, equal } from "node:assert/strict";
import { consumeComposerDraft, normalizeComposerDraft } from "../src/lib/composerDraft.js";

const attachment = { id: "file-a", name: "a.txt", mime: "text/plain", size: 1, content: "a" };
const nextAttachment = { ...attachment, id: "file-b", name: "b.txt" };
deepEqual(normalizeComposerDraft("legacy text"), { text: "legacy text", attachments: [] });
deepEqual(normalizeComposerDraft({ text: "", attachments: [attachment] }), { text: "", attachments: [attachment] });

const { getSessionComposerState, updateSessionComposerState, hydrateSessionComposerDraftsFromUserState } = await import("../src/sessions/store.js");
const { writeUserStateKey } = await import("../src/persistence/userStateStorage.js");

updateSessionComposerState("whitespace", () => ({ text: "  \n", attachments: [] }));
equal(getSessionComposerState("whitespace").text, "  \n", "the controlled composer must preserve leading whitespace while typing");

updateSessionComposerState("a", () => ({ text: "submitted", attachments: [attachment] }));
const submitted = getSessionComposerState("a");
// Switching the active UI is irrelevant: a delayed completion still addresses a.
updateSessionComposerState("b", () => ({ text: "other chat", attachments: [nextAttachment] }));
updateSessionComposerState("a", () => ({ text: "next prompt", attachments: [attachment, nextAttachment] }));
updateSessionComposerState("a", (current) => consumeComposerDraft(current, submitted));
deepEqual(getSessionComposerState("a"), { text: "next prompt", attachments: [nextAttachment] });
deepEqual(getSessionComposerState("b"), { text: "other chat", attachments: [nextAttachment] });

// File reads completing after navigation append to their captured originating id.
const originId = "a";
await Promise.resolve();
updateSessionComposerState(originId, (current) => ({ ...current, attachments: [...current.attachments, attachment] }));
equal(getSessionComposerState("a").attachments.length, 2);
equal(getSessionComposerState("b").attachments.length, 1);

await writeUserStateKey("milim.sessionDrafts", JSON.stringify({ a: "stale persisted text", legacy: "old format", saved: { text: "", attachments: [attachment] } }));
await hydrateSessionComposerDraftsFromUserState();
equal(getSessionComposerState("a").text, "next prompt", "startup hydration must preserve edits made before the read completed");
equal(getSessionComposerState("legacy").text, "old format");
deepEqual(getSessionComposerState("saved").attachments, [attachment], "attachment-only drafts restore");
deepEqual(consumeComposerDraft(submitted, submitted), { text: "", attachments: [] });
