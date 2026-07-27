import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { diffRows, diffSections, diffStats, findDiffSectionIndex, gitFileTree, shouldCollapseDiffSection } from "../src/lib/gitDiffRows.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const rows = diffRows([
  " note.txt | 2 +-",
  "",
  "diff --git a/note.txt b/note.txt",
  "index 1111111..2222222 100644",
  "--- a/note.txt",
  "+++ b/note.txt",
  "@@ -1,2 +1,2 @@",
  " same",
  "-old",
  "+new",
].join("\n"));

assert(rows[0].kind === "file" && rows[0].text === "note.txt", "the duplicate stat preamble should be omitted");
assert(rows.every((row) => row.kind !== "stat"), "stat rows should not be rendered when a patch is present");
assert(rows[4].kind === "hunk", "hunk header should be classified");

const deleted = rows.find((row) => row.kind === "delete");
const added = rows.find((row) => row.kind === "add");
assert(deleted?.oldNo === "2" && deleted.text === "old", "deleted row should keep old line number");
assert(added?.newNo === "2" && added.text === "new", "added row should keep new line number");

const sections = diffSections(rows);
assert(sections.length === 1 && sections[0].path === "note.txt", "section should be created from file row");
assert(sections[0].additions === 1 && sections[0].deletions === 1, "section should count changed rows");

const stats = diffStats(rows);
assert(stats.files === 1 && stats.additions === 1 && stats.deletions === 1, "stats should summarize sections");
assert(shouldCollapseDiffSection({ ...sections[0], path: "pnpm-lock.yaml" }), "lockfiles should default collapsed");

const navigationSections = [
  { ...sections[0], id: "0:src/new.ts", path: "src/new.ts" },
  { ...sections[0], id: "1:src/other.ts", path: "src/other.ts" },
];
assert(findDiffSectionIndex(navigationSections, "src/new.ts") === 0, "exact paths should resolve");
assert(findDiffSectionIndex(navigationSections, "src/old.ts -> src/new.ts") === 0, "renamed paths should resolve to their destination");
assert(findDiffSectionIndex(navigationSections, "src/{old => new}.ts") === 0, "compact renamed paths should resolve to their destination");
assert(findDiffSectionIndex(navigationSections, "unknown.ts", 1) === 1, "the status-order fallback should resolve existing sections");
assert(findDiffSectionIndex(navigationSections, "missing.ts", 2) === -1, "missing or truncated sections should not resolve");

const fileTree = gitFileTree([
  "apps/desktop/src/components/GitPanel.tsx",
  "apps/desktop/src/lib/gitDiffRows.ts",
  "README.md",
]);
assert(fileTree[0].name === "apps/desktop/src", "single-child folders should be compacted");
assert(fileTree[0].children.map((node) => node.name).join(",") === "components,lib", "folders should preserve the file hierarchy");
assert(fileTree[1].name === "README.md" && fileTree[1].fileIndex === 2, "root files should remain navigable");

const gitPanelSource = readFileSync(resolve(process.cwd(), "src/components/GitPanel.tsx"), "utf8");
const styles = readFileSync(
  resolve(process.cwd(), "src/shell.css"),
  "utf8",
);
assert(!gitPanelSource.includes('window.prompt("Review comment")'), "diff review comments should use the Milim editor");
assert(gitPanelSource.includes('className="git-diff-comment-editor"'), "diff review comments should render inline");
assert(styles.includes(".git-diff-comment {\n  position: absolute;"), "the review button should overlay the gutter instead of creating a row");
assert(styles.includes(".git-diff-comment-editor {\n  position: sticky;"), "only the open review editor should occupy a full diff row");

export {};
