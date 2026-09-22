import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import type { WorkspaceGitStatus } from "../src/api.js";
import { diffHunks, diffRows, diffSections, diffStats, findDiffSectionIndex, gitFileTree, shouldCollapseDiffSection } from "../src/lib/gitDiffRows.js";
import {
  defaultCommitStageAll,
  gitCommitScope,
  gitDiscardConfirmation,
  gitFileStaging,
  gitFileStagingActions,
  gitHunkStagingActions,
} from "../src/lib/gitStaging.js";

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

const hunkDiff = [
  "diff --git a/notes.txt b/notes.txt",
  "index 1111111..2222222 100644",
  "--- a/notes.txt",
  "+++ b/notes.txt",
  "@@ -1,3 +1,3 @@",
  " one",
  "-two",
  "+TWO",
  "@@ -20,2 +20,3 @@ fn tail",
  " twenty",
  "+twenty-one",
  "\\ No newline at end of file",
  "",
  "diff --git a/fresh.txt b/fresh.txt",
  "new file mode 100644",
  "--- /dev/null",
  "+++ b/fresh.txt",
  "@@ -0,0 +1 @@",
  "+new",
  "",
].join("\n");
const hunks = diffHunks(hunkDiff);
const hunkRows = diffRows(hunkDiff).filter((row) => row.kind === "hunk");
assert(hunks.length === 3 && hunkRows.length === 3, "every hunk row should map to one raw hunk");
assert(hunks[0].path === "notes.txt" && hunks[0].text === "@@ -1,3 +1,3 @@\n one\n-two\n+TWO", "hunk text should keep header and body exactly");
assert(hunks[1].text.endsWith("+twenty-one\n\\ No newline at end of file"), "no-newline markers belong to their hunk and trailing blanks are trimmed");
assert(hunks[2].path === "fresh.txt" && hunks[2].text === "@@ -0,0 +1 @@\n+new", "file metadata should never leak into the previous hunk");

const staged = gitFileStaging({ status: "M", path: "a.ts", staged: true, unstaged: false });
const both = gitFileStaging({ status: "MM", path: "b.ts", staged: true, unstaged: true });
const untracked = gitFileStaging({ status: "??", path: "c.ts", staged: false, unstaged: true });
const conflicted = gitFileStaging({ status: "UU", path: "d.ts", staged: true, unstaged: true });
const legacy = gitFileStaging({ status: "M", path: "e.ts" });
assert(gitFileStagingActions(staged).join() === "unstage_file", "staged-only files can only be unstaged");
assert(gitFileStagingActions(both).join() === "stage_file,unstage_file,discard_file", "partially staged files expose every action");
assert(gitFileStagingActions(untracked).join() === "stage_file,discard_file", "untracked files can be staged or deleted");
assert(gitFileStagingActions(conflicted).join() === "stage_file", "conflicts can only be staged to mark them resolved");
assert(gitFileStagingActions(legacy).join() === "stage_file,discard_file", "ambiguous legacy statuses never offer unstage");
assert(gitHunkStagingActions("unstaged", both).join() === "stage_hunk,discard_hunk", "unstaged diffs stage or discard hunks");
assert(gitHunkStagingActions("staged", both).join() === "unstage_hunk", "staged diffs unstage hunks");
assert(gitHunkStagingActions("all", both).length === 0, "combined HEAD diffs cannot map hunks to one side of the index");
assert(gitHunkStagingActions("unstaged", untracked).length === 0, "untracked files are staged as a whole");
assert(gitDiscardConfirmation("c.ts", untracked, "file").confirmLabel === "Delete file", "untracked discard must say it deletes the file");
assert(gitDiscardConfirmation("b.ts", both, "file").message.includes("Staged changes are kept"), "tracked discard should explain what survives");

const commitStatus = {
  staged: 1,
  changed_files: [
    { status: "M", path: "a.ts", staged: true, unstaged: false },
    { status: "MM", path: "b.ts", staged: true, unstaged: true },
    { status: "??", path: "c.ts", staged: false, unstaged: true },
  ],
} as unknown as WorkspaceGitStatus;
const commitScope = gitCommitScope(commitStatus);
assert(commitScope.stagedFiles.length === 2 && commitScope.unstagedFiles.length === 2, "commit scope should split staged and unstaged files");
assert(!defaultCommitStageAll(commitStatus), "a staged index should default to committing only staged changes");
assert(defaultCommitStageAll({ ...commitStatus, staged: 0 }), "an empty index should default to staging everything");

const gitPanelSource = readFileSync(resolve(process.cwd(), "src/components/GitPanel.tsx"), "utf8");
const styles = readFileSync(
  resolve(process.cwd(), "src/shell.css"),
  "utf8",
).replace(/\r\n/g, "\n");
assert(!gitPanelSource.includes('window.prompt("Review comment")'), "diff review comments should use the Milim editor");
assert(gitPanelSource.includes('className="git-diff-comment-editor"'), "diff review comments should render inline");
assert(styles.includes(".git-diff-comment {\n  position: absolute;"), "the review button should overlay the gutter instead of creating a row");
assert(styles.includes(".git-diff-comment-editor {\n  position: sticky;"), "only the open review editor should occupy a full diff row");

export {};
