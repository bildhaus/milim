import {
  CHAT_SCROLL_BOTTOM_THRESHOLD,
  followScrollTop,
  isNearScrollBottom,
  peekEnteringMessageIds,
  scrollTopForRestoredAnchor,
  scrollTopAfterLayoutChange,
  transcriptMessageRenderId,
  transcriptSpacerHeight,
} from "../src/lib/scroll.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

assert(
  isNearScrollBottom({ scrollTop: 500, scrollHeight: 1_000, clientHeight: 500 }),
  "exact bottom should couple autoscroll",
);

assert(
  isNearScrollBottom({
    scrollTop: 500 - CHAT_SCROLL_BOTTOM_THRESHOLD,
    scrollHeight: 1_000,
    clientHeight: 500,
  }),
  "within threshold should couple autoscroll",
);

assert(
  !isNearScrollBottom({
    scrollTop: 500 - CHAT_SCROLL_BOTTOM_THRESHOLD - 1,
    scrollHeight: 1_000,
    clientHeight: 500,
  }),
  "farther than threshold should decouple autoscroll",
);

assert(
  followScrollTop({ scrollHeight: 1_000, clientHeight: 500 }) === 500,
  "follow target should pin to the latest bottom",
);

assert(
  followScrollTop({ scrollHeight: 400, clientHeight: 500 }) === 0,
  "follow target should not go negative when content is shorter than the viewport",
);

assert(
  followScrollTop({ scrollHeight: 820, clientHeight: 500 }) === 320,
  "follow target should update when the thread grows",
);

assert(
  scrollTopAfterLayoutChange(
    { scrollTop: 500, scrollHeight: 1_000, clientHeight: 420 },
    true,
  ) === 580,
  "a coupled transcript should stay pinned when the composer shrinks its viewport",
);

assert(
  scrollTopAfterLayoutChange(
    { scrollTop: 240, scrollHeight: 1_000, clientHeight: 420 },
    false,
  ) === 240,
  "a decoupled transcript should preserve the reader's position across layout changes",
);

assert(
  transcriptSpacerHeight(3, 100, 12) === 324,
  "virtual spacers should include the gaps between hidden rows",
);
assert(
  transcriptSpacerHeight(3, 100, 12, [80, 140]) === 344,
  "virtual spacers should use measured heights and estimate only unknown rows",
);
assert(
  transcriptSpacerHeight(0, 100, 12) === 0,
  "an empty virtual range should not reserve height",
);
assert(
  scrollTopForRestoredAnchor(400, 24, 57) === 433,
  "anchor restoration should compensate for the row's layout delta",
);
assert(
  transcriptMessageRenderId(
    "thread-a",
    { id: "control-stream-run-1", role: "assistant", runId: "run-1" },
    4,
  ) === transcriptMessageRenderId(
    "thread-a",
    { id: "assistant-final", role: "assistant", runId: "run-1" },
    4,
  ),
  "a canonical assistant should keep one render identity across stream finalization",
);
assert(
  transcriptMessageRenderId(
    "thread-b",
    { id: "assistant-final", role: "assistant", runId: "run-1" },
    4,
  ) !== transcriptMessageRenderId(
    "thread-a",
    { id: "assistant-final", role: "assistant", runId: "run-1" },
    4,
  ),
  "render identities should stay scoped to their thread",
);

const peekSeen = new Set(["a"]);
assert(
  JSON.stringify(peekEnteringMessageIds(peekSeen, ["a", "b"], true)) === JSON.stringify(["b"]),
  "peek should report unseen ids without mutating the seen set",
);
assert(!peekSeen.has("b"), "peek must not mark ids as seen");
assert(
  peekEnteringMessageIds(peekSeen, ["a", "b"], false).length === 0,
  "peek should stay quiet when animation is disabled",
);
