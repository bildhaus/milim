#!/usr/bin/env node
// Regenerate the large and medium task fixtures under evals/tasks/*/repo.
// The generated repositories are checked in so every task stays
// self-contained; `--check` verifies they still match this generator.
//
//   node evals/fixtures-gen/generate.mjs          # write fixtures
//   node evals/fixtures-gen/generate.mjs --check  # exit 1 on drift

import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { catalogFiles, formattersFiles } from "./large-files.mjs";
import { mediumServiceFiles } from "./medium-service.mjs";
import { meetingNotesFiles } from "./meeting-notes.mjs";

const TASKS = join(dirname(fileURLToPath(import.meta.url)), "..", "tasks");

const GENERATED = {
  "grep-quota-status": () => mediumServiceFiles("quota"),
  "grep-cache-config": () => mediumServiceFiles("cache"),
  "large-file-catalog-edit": catalogFiles,
  "large-file-function-fix": formattersFiles,
  "large-context-late-fees": meetingNotesFiles,
};

function listFiles(root) {
  if (!existsSync(root)) return [];
  const out = [];
  for (const name of readdirSync(root)) {
    const full = join(root, name);
    if (statSync(full).isDirectory()) out.push(...listFiles(full));
    else out.push(full);
  }
  return out;
}

const check = process.argv.includes("--check");
let drift = 0;
for (const [task, build] of Object.entries(GENERATED)) {
  const root = join(TASKS, task, "repo");
  const files = build();
  const existing = listFiles(root).map((path) => relative(root, path).split(sep).join("/"));
  if (check) {
    for (const [path, content] of Object.entries(files)) {
      const full = join(root, path);
      if (!existsSync(full) || readFileSync(full, "utf8") !== content) {
        console.log(`drift: ${task}/repo/${path}`);
        drift += 1;
      }
    }
    for (const path of existing.filter((path) => !(path in files))) {
      console.log(`extra: ${task}/repo/${path}`);
      drift += 1;
    }
    continue;
  }
  rmSync(root, { recursive: true, force: true });
  for (const [path, content] of Object.entries(files)) {
    const full = join(root, path);
    mkdirSync(dirname(full), { recursive: true });
    writeFileSync(full, content);
  }
  console.log(`wrote ${task}: ${Object.keys(files).length} files`);
}
if (check) {
  if (drift > 0) process.exit(1);
  console.log("fixtures match the generator");
}
