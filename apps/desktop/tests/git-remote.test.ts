import assert from "node:assert/strict";
import { gitRemoteWebUrl } from "../src/lib/gitRemote.js";

assert.equal(
  gitRemoteWebUrl("https://github.com/bildhaus/milim.git"),
  "https://github.com/bildhaus/milim",
);
assert.equal(
  gitRemoteWebUrl("ssh://git@github.com/bildhaus/milim.git"),
  "https://github.com/bildhaus/milim",
);
assert.equal(
  gitRemoteWebUrl("git@github.com:bildhaus/milim.git"),
  "https://github.com/bildhaus/milim",
);
assert.equal(gitRemoteWebUrl("C:\\work\\milim"), null);
assert.equal(gitRemoteWebUrl(`${"!.".repeat(20_000)}:bildhaus/milim`), null);
