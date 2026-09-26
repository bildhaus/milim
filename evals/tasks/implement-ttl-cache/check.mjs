import { attempt, expect, expectEqual, finish, load } from "../../lib/check.mjs";

function throwsRange(fn) {
  try {
    fn();
  } catch (error) {
    return error instanceof RangeError;
  }
  return false;
}

await attempt("TtlCache", async () => {
  const { TtlCache } = await load("src/cache.js");
  let clock = 1_000;
  const now = () => clock;

  const lru = new TtlCache({ capacity: 3, now });
  lru.set("a", 1).set("b", 2).set("c", 3);
  expectEqual(lru.get("a"), 1, "get a");
  lru.set("d", 4);
  expectEqual(lru.has("b"), false, "b evicted as least recently used");
  expectEqual(lru.keys(), ["c", "a", "d"], "recency order after eviction");
  lru.has("c");
  lru.set("e", 5);
  expectEqual(lru.keys(), ["a", "d", "e"], "has() must not refresh recency");
  lru.set("a", 10);
  expectEqual(lru.keys(), ["d", "e", "a"], "set existing refreshes recency");
  expectEqual(lru.size, 3, "size at capacity");
  expectEqual(lru.delete("d"), true, "delete live");
  expectEqual(lru.delete("d"), false, "delete missing");

  clock = 0;
  const ttl = new TtlCache({ capacity: 2, ttlMs: 100, now });
  ttl.set("x", "X");
  ttl.set("y", "Y", 500);
  clock = 99;
  expectEqual(ttl.get("x"), "X", "alive before ttl");
  clock = 100;
  expectEqual(ttl.get("x"), undefined, "expired exactly at ttl");
  expectEqual(ttl.size, 1, "expired not counted");
  expectEqual(ttl.keys(), ["y"], "expired not listed");
  expectEqual(ttl.delete("x"), false, "deleting expired returns false");
  ttl.set("z", "Z");
  expectEqual(ttl.keys(), ["y", "z"], "expired dropped before evicting live");
  clock = 150;
  ttl.get("y");
  clock = 199;
  expectEqual(ttl.has("z"), true, "hit does not extend ttl (still alive)");
  clock = 200;
  expectEqual(ttl.has("z"), false, "z expires at 200");
  ttl.set("y", "Y2");
  clock = 299;
  expectEqual(ttl.get("y"), "Y2", "replace restarts ttl");
  clock = 300;
  expectEqual(ttl.get("y"), undefined, "replaced entry uses default ttl");

  const forever = new TtlCache({ capacity: 1, now });
  forever.set("k", "v");
  clock = Number.MAX_SAFE_INTEGER;
  expectEqual(forever.get("k"), "v", "Infinity ttl never expires");

  expect(throwsRange(() => new TtlCache({ capacity: 0 })), "capacity 0 throws RangeError");
  expect(throwsRange(() => new TtlCache({ capacity: 2, ttlMs: -1 })), "negative ttl throws RangeError");
  expect(throwsRange(() => new TtlCache({ capacity: 2 }).set("a", 1, 0)), "zero per-entry ttl throws RangeError");
  expect(!throwsRange(() => new TtlCache({ capacity: 2 })), "valid cache constructs");
});

finish();
