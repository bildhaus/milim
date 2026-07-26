import assert from "node:assert/strict";
import test from "node:test";

import {
  GOOGLE_ACCOUNT_CONNECTIONS_URL,
  GOOGLE_CONNECT_DISCLOSURE,
  GOOGLE_REMOVE_MESSAGE,
  applyGoogleSheetDimension,
  applyGoogleSheetValues,
  createGoogleSaveQueue,
  googleDocEditableParagraph,
  googleDocSelectionRange,
  googleDocTextReplacement,
  googleDisconnectMessage,
  googleFloatingToolbarPosition,
  googleSheetCellRange,
  googleWorkspaceFileUrl,
  googleWorkspaceUrl,
  parseGoogleSheetClipboard,
} from "../src/lib/googleWorkspace.js";

test("uses honest connection and revocation copy", () => {
  assert.match(GOOGLE_CONNECT_DISCLOSURE, /remote model/);
  assert.match(GOOGLE_REMOVE_MESSAGE, /authorization were not changed/);
  assert.equal(GOOGLE_ACCOUNT_CONNECTIONS_URL, "https://myaccount.google.com/connections");
  assert.match(googleDisconnectMessage("confirmed"), /confirmed revocation/);
  assert.match(googleDisconnectMessage("unconfirmed"), /did not confirm revocation/);
  assert.match(googleDisconnectMessage("not_needed"), /local authorization was removed/);
});

test("recognizes supported Google Workspace URLs", () => {
  assert.deepEqual(
    googleWorkspaceUrl("https://docs.google.com/spreadsheets/d/sheet_123/edit#gid=0"),
    { fileId: "sheet_123", kind: "sheet" },
  );
  assert.deepEqual(
    googleWorkspaceUrl("https://docs.google.com/document/d/doc-123/edit"),
    { fileId: "doc-123", kind: "document" },
  );
  assert.deepEqual(
    googleWorkspaceUrl("https://drive.google.com/drive/folders/folder_123"),
    { fileId: "folder_123", kind: "folder" },
  );
  assert.equal(googleWorkspaceUrl("https://example.com/file/d/nope"), null);
  assert.equal(googleWorkspaceUrl("https://drive.google.com/file/d/../view"), null);
});

test("builds stable fallback URLs by MIME type", () => {
  assert.equal(
    googleWorkspaceFileUrl({
      id: "sheet",
      mime_type: "application/vnd.google-apps.spreadsheet",
    }),
    "https://docs.google.com/spreadsheets/d/sheet/edit",
  );
  assert.equal(
    googleWorkspaceFileUrl({
      id: "blob",
      mime_type: "application/pdf",
      web_view_link: "https://drive.google.com/file/d/blob/view",
    }),
    "https://drive.google.com/file/d/blob/view",
  );
});

test("builds safe edit ranges and parses pasted cells", () => {
  assert.equal(googleSheetCellRange("Omer's sheet", "B4"), "'Omer''s sheet'!B4");
  assert.deepEqual(
    parseGoogleSheetClipboard("one\ttwo\r\nthree\tfour\r\n"),
    [["one", "two"], ["three", "four"]],
  );
});

test("applies optimistic Sheets edits without mutating the confirmed grid", () => {
  const original = {
    values: [["A", "B"], ["C", "D"]],
    formulas: [["A", "B"], ["C", "D"]],
  };
  const edited = applyGoogleSheetValues(original, 0, 1, [["changed", "=SUM(A1:A2)"]]);
  assert.deepEqual(edited.values, [["A", "changed", ""], ["C", "D"]]);
  assert.deepEqual(edited.formulas, [["A", "changed", "=SUM(A1:A2)"], ["C", "D"]]);
  assert.deepEqual(original.values, [["A", "B"], ["C", "D"]]);

  const inserted = applyGoogleSheetDimension(edited, "insert_rows", 1);
  assert.deepEqual(inserted.values[1], ["", "", ""]);
  assert.deepEqual(
    applyGoogleSheetDimension(inserted, "delete_columns", 0).values,
    [["changed", ""], ["", ""], ["D"]],
  );
});

test("serializes background saves and retries the failed task before later edits", async () => {
  const calls: string[] = [];
  const states: string[] = [];
  let drained = 0;
  let releaseFirst!: () => void;
  const first = new Promise<void>((resolve) => {
    releaseFirst = resolve;
  });
  let secondAttempts = 0;
  const queue = createGoogleSaveQueue(
    (state) => states.push(`${state.status}:${state.pending}`),
    () => { drained += 1; },
  );

  queue.enqueue(async () => {
    calls.push("first");
    await first;
  });
  queue.enqueue(async () => {
    secondAttempts += 1;
    calls.push(`second:${secondAttempts}`);
    if (secondAttempts === 1) throw new Error("offline");
  });
  assert.deepEqual(calls, ["first"]);

  releaseFirst();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["first", "second:1"]);
  assert.equal(queue.getState().status, "error");

  queue.enqueue(async () => {
    calls.push("third");
  });
  assert.equal(queue.getState().pending, 2);
  assert(!calls.includes("third"));

  queue.retry();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["first", "second:1", "second:2", "third"]);
  assert.equal(queue.getState().status, "saved");
  assert.equal(drained, 1);
  assert(states.includes("error:1"));
});

test("extracts only safe top-level Docs paragraphs for editing", () => {
  assert.deepEqual(
    googleDocEditableParagraph({
      startIndex: 4,
      endIndex: 11,
      paragraph: {
        elements: [
          { textRun: { content: "Hello " } },
          { textRun: { content: "world\n" } },
        ],
      },
    }),
    { start: 4, end: 10, text: "Hello world" },
  );
  assert.equal(
    googleDocEditableParagraph({
      startIndex: 4,
      endIndex: 6,
      paragraph: { elements: [{ inlineObjectElement: { inlineObjectId: "image" } }] },
    }),
    null,
  );
});

test("replaces only the changed Docs text range", () => {
  assert.deepEqual(
    googleDocTextReplacement(10, "Hello styled world", "Hello edited world"),
    { start: 16, end: 20, text: "edit" },
  );
  assert.deepEqual(
    googleDocTextReplacement(10, "A😀B", "A🙂B"),
    { start: 11, end: 13, text: "🙂" },
  );
  assert.deepEqual(
    googleDocTextReplacement(10, "same", "same"),
    null,
  );
});

test("maps Docs selections with Google UTF-16 indices", () => {
  assert.deepEqual(
    googleDocSelectionRange(10, "A😀", "selected"),
    { start: 13, end: 21 },
  );
});

test("positions the Docs selection toolbar within the viewer", () => {
  assert.deepEqual(
    googleFloatingToolbarPosition(
      { left: 180, top: 120, right: 220, bottom: 140 },
      { left: 100, top: 50, right: 500, bottom: 400 },
      { width: 200, height: 40 },
    ),
    { left: 108, top: 72, placement: "above" },
  );
  assert.deepEqual(
    googleFloatingToolbarPosition(
      { left: 460, top: 60, right: 490, bottom: 80 },
      { left: 100, top: 50, right: 500, bottom: 400 },
      { width: 200, height: 40 },
    ),
    { left: 292, top: 88, placement: "below" },
  );
});
