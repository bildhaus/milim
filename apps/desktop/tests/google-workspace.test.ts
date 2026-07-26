import assert from "node:assert/strict";
import test from "node:test";

import {
  GOOGLE_ACCOUNT_CONNECTIONS_URL,
  GOOGLE_CONNECT_DISCLOSURE,
  GOOGLE_REMOVE_MESSAGE,
  googleDocEditableParagraph,
  googleDocTextReplacement,
  googleDisconnectMessage,
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
