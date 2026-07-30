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
assert(
  !sidebarSource.includes('className="session-status"'),
  "expanded threads should not keep the old left status dot",
);
assert(
  /{generating \? \(\s*<WorkingSessionLoader[\s\S]*?className="session-side-indicator"[\s\S]*?\) : unread \? \(\s*<UnreadSessionLoader[\s\S]*?className="session-side-indicator"[\s\S]*?\) : \(\s*<span\s+className="session-side-indicator session-recency"/.test(sidebarSource),
  "working and unread threads should replace expanded-sidebar recency with the shared indicators",
);

export {};
