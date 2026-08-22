import { readFileSync } from "node:fs";
import { sessionRecencyLabel } from "../src/lib/sessionRecency.js";

function equal(actual: string, expected: string, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${expected}, got ${actual}`);
  }
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const now = 1_800_000_000_000;

equal(sessionRecencyLabel(now - 30_000, now), "now", "fresh sessions should show now");
equal(sessionRecencyLabel(now - 5 * 60_000, now), "5m", "minute recency should use m");
equal(sessionRecencyLabel(now - 3 * 60 * 60_000, now), "3h", "hour recency should use h");
equal(sessionRecencyLabel(now - 6 * 24 * 60 * 60_000, now), "6d", "day recency should use d");
equal(sessionRecencyLabel(now - 45 * 24 * 60 * 60_000, now), "1mo", "older recency should use mo");

const sidebarSource = readFileSync("src/components/Sidebar.tsx", "utf8");
const shellSource = readFileSync("src/shell.css", "utf8");
assert(
  !sidebarSource.includes('className="session-status"'),
  "expanded threads should not keep the old left status dot",
);
assert(
  /{generating \? \(\s*<WorkingSessionLoader[\s\S]*?className="session-side-indicator"[\s\S]*?\) : unread \? \(\s*<UnreadSessionLoader[\s\S]*?className="session-side-indicator"[\s\S]*?\) : \(\s*<span\s+className="session-side-indicator session-recency"/.test(sidebarSource),
  "working and unread threads should replace expanded-sidebar recency with the shared indicators",
);
assert(
  sidebarSource.includes('<WorkingSessionLoader phaseKey={session.id} aria-hidden="true" />'),
  "collapsed working loaders should preserve a stable per-thread animation phase",
);
assert(
  sidebarSource.includes('aria-current={session.id === activeId ? "page" : undefined}') &&
    (sidebarSource.match(/aria-current=\{s\.id === activeId \? "page" : undefined\}/g)?.length ?? 0) === 2,
  "every active thread row should expose its current-page state",
);
assert(
  sidebarSource.includes('role="img"') && sidebarSource.includes('aria-label={indicator.label}'),
  "app preview runtime markers should expose their status label",
);
assert(
  /\.inbox-session-item > \.session-runtime-marker,\s*\.session-section-title > \.session-runtime-marker\s*\{\s*left: -4px;/.test(shellSource),
  "inbox and project runtime markers should share the outer status rail",
);
assert(
  /\.session-list\s*\{[\s\S]*?margin: 3px -4px 0 -6px;[\s\S]*?padding: 0 5px 0 6px;/.test(shellSource),
  "the sidebar scrollport should expose the outer status rail without shifting row content",
);

export {};
