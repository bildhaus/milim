import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, load, read, runNodeTests } from "../../lib/check.mjs";

expect(read("test/queue.test.js") === headFile("test/queue.test.js"), "tests were modified");
expectOnlyChanged((path) => path.startsWith("src/"));
expect(!/setTimeout|retry/i.test(read("src/queue.js")), "src/queue.js must not add sleeps or retries");
runNodeTests("test");

await attempt("hidden queue cases", async () => {
  const { JobQueue } = await load("src/queue.js");
  const delay = (ms, value) => new Promise((resolve) => setTimeout(() => resolve(value), ms));
  const queue = new JobQueue(1);
  const order = [];
  queue.push(async () => {
    await delay(10);
    order.push("a");
    return "a";
  });
  queue.push(() => {
    order.push("b");
    return "b";
  });
  queue.push(() => {
    throw new Error("sync");
  });
  const results = await queue.drain();
  expectEqual(order, ["a", "b"], "concurrency 1 runs jobs sequentially");
  expectEqual(results, [{ ok: true, value: "a" }, { ok: true, value: "b" }, { ok: false, error: "sync" }], "results");
});

finish();
