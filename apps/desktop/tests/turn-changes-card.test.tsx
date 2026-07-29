import assert from "node:assert/strict";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type {
  WorkspaceCheckpoint,
  WorkspaceGitActionResult,
} from "../src/api.js";
import type { DiffSection, DiffStats } from "../src/lib/gitDiffRows.js";

type TurnChanges = {
  key: string;
  checkpoint: WorkspaceCheckpoint;
  result: WorkspaceGitActionResult;
  sections: DiffSection[];
  stats: DiffStats;
};

type TurnReviewState =
  | ({ status: "ready" } & TurnChanges)
  | {
      key: string;
      status: "checking" | "no_changes";
      checkpoint: WorkspaceCheckpoint;
    }
  | {
      key: string;
      status: "unavailable";
      checkpoint: WorkspaceCheckpoint;
      message: string;
    };

type TurnChangesCardProps = {
  review: TurnReviewState;
  onUndo: () => void;
  onReview: () => void;
  onRetry: () => void;
  onOpenGit: () => void;
};

const checkpoint: WorkspaceCheckpoint = {
  ref: "refs/milim/checkpoints/turn-1",
  createdAt: 1,
  folder: "C:\\work",
};

function result(stdout: string, ok = true): WorkspaceGitActionResult {
  return {
    ok,
    action: "diff",
    command: "git diff",
    stdout,
    stderr: "",
    exit_code: ok ? 0 : 1,
    message: ok ? "Diff ready." : "Diff failed.",
    truncated: false,
  };
}

const patch = Array.from({ length: 5 }, (_, index) => {
  const path = `src/file-${index + 1}.ts`;
  return [
    `diff --git a/${path} b/${path}`,
    `--- a/${path}`,
    `+++ b/${path}`,
    "@@ -1 +1 @@",
    "-before",
    "+after",
  ].join("\n");
}).join("\n");

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { TurnChangesCard, turnReviewFromDiff } = await server.ssrLoadModule(
    "/src/components/TurnChangesCard.tsx",
  ) as {
    TurnChangesCard: ComponentType<TurnChangesCardProps>;
    turnReviewFromDiff: (
      key: string,
      checkpoint: WorkspaceCheckpoint,
      result: WorkspaceGitActionResult,
    ) => TurnReviewState;
  };
  const changes = turnReviewFromDiff("turn-1", checkpoint, result(patch));
  assert.equal(changes.status, "ready");
  if (changes.status !== "ready") throw new Error("Expected ready review.");
  assert.deepEqual(changes.stats, { files: 5, additions: 5, deletions: 5 });
  assert.equal(changes.sections[0].path, "src/file-1.ts");
  assert.equal(turnReviewFromDiff("empty", checkpoint, result("")).status, "no_changes");
  assert.equal(turnReviewFromDiff("error", checkpoint, result("", false)).status, "unavailable");

  const markup = renderToStaticMarkup(
    createElement(TurnChangesCard, {
      review: changes,
      onUndo: () => {},
      onReview: () => {},
      onRetry: () => {},
      onOpenGit: () => {},
    }),
  );
  assert.match(markup, /aria-label="Turn changes"/);
  assert.match(markup, />Changed 5 files</);
  assert.match(markup, />\+5</);
  assert.match(markup, />-5</);
  assert.match(markup, />Undo</);
  assert.match(markup, />Review changes</);
  assert.match(markup, /src\/file-1\.ts/);
  assert.match(markup, /src\/file-3\.ts/);
  assert.doesNotMatch(markup, /src\/file-4\.ts/);
  assert.match(markup, /aria-expanded="false"/);
  assert.match(markup, />Show 2 more</);

  const single = { ...changes, sections: changes.sections.slice(0, 1), stats: { files: 1, additions: 1, deletions: 1 } };
  const singleMarkup = renderToStaticMarkup(
    createElement(TurnChangesCard, {
      review: single,
      onUndo: () => {},
      onReview: () => {},
      onRetry: () => {},
      onOpenGit: () => {},
    }),
  );
  assert.match(singleMarkup, />Changed 1 file</);
  assert.doesNotMatch(singleMarkup, /Show .* more/);

  const unavailable = turnReviewFromDiff(
    "unavailable",
    checkpoint,
    result("", false),
  );
  const unavailableMarkup = renderToStaticMarkup(
    createElement(TurnChangesCard, {
      review: unavailable,
      onUndo: () => {},
      onReview: () => {},
      onRetry: () => {},
      onOpenGit: () => {},
    }),
  );
  assert.match(unavailableMarkup, /data-review-state="unavailable"/);
  assert.match(unavailableMarkup, />Retry</);
  assert.match(unavailableMarkup, />Open Git</);
} finally {
  await server.close();
}
