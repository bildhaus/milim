import assert from "node:assert/strict";
import test from "node:test";

import { googleWorkspaceFileUrl, googleWorkspaceUrl } from "../src/lib/googleWorkspace.js";

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
