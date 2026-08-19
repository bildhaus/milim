import {
  CHAT_SCROLL_BOTTOM_THRESHOLD,
  followScrollTop,
  isNearScrollBottom,
  nextEnteringMessageIds,
  peekEnteringMessageIds,
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

const seeded = new Set<string>();
assert(
  nextEnteringMessageIds(seeded, ["a", "b"], false).length === 0,
  "restored thread ids should be remembered without animating",
);
assert(seeded.has("a") && seeded.has("b"), "restored thread ids should be marked seen");

assert(
  JSON.stringify(nextEnteringMessageIds(seeded, ["a", "b", "c"], true)) === JSON.stringify(["c"]),
  "only newly appended ids should enter while following",
);
assert(
  nextEnteringMessageIds(seeded, ["a", "b", "c"], true).length === 0,
  "already seen ids should not re-enter",
);
assert(
  nextEnteringMessageIds(seeded, ["c", undefined, "d"], false).length === 0,
  "uncoupled or restored updates should still record ids without animating",
);
assert(seeded.has("d"), "uncoupled ids should still be remembered");

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
