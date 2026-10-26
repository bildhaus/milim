// Shared by the solution scripts: each turn's edits on the repository in
// the current directory.
import { readFileSync, writeFileSync } from "node:fs";

const FILES = ["src/config.js", "src/client.js", "test/client.test.js", "docs/config.md"];

export function renameEverywhere() {
  for (const path of FILES) {
    writeFileSync(path, readFileSync(path, "utf8").replace(/\btimeoutMs\b/g, "requestTimeoutMs"));
  }
}

export function bumpRetries() {
  const config = readFileSync("src/config.js", "utf8");
  writeFileSync("src/config.js", config.replace("  retries: 3,", "  retries: 5,"));
  const docs = readFileSync("docs/config.md", "utf8");
  writeFileSync("docs/config.md", docs.replace("| `retries` | 3 |", "| `retries` | 5 |"));
}
