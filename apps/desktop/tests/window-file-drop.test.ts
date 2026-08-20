import assert from "node:assert/strict";
import { dataTransferCarriesFiles } from "../src/lib/windowFileDrop.js";

assert.equal(dataTransferCarriesFiles(["Files"]), true);
assert.equal(dataTransferCarriesFiles(["text/plain", "Files"]), true);
assert.equal(dataTransferCarriesFiles(["text/plain"]), false);
assert.equal(dataTransferCarriesFiles([]), false);
