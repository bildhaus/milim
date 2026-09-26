import { test } from "node:test";
import assert from "node:assert/strict";
import { JobQueue } from "../src/queue.js";
import { fetchAll } from "../src/fetcher.js";

const delay = (ms, value) => new Promise((resolve) => setTimeout(() => resolve(value), ms));

test("results keep push order", async () => {
  const values = await fetchAll([3, 1, 2], (id) => delay(id * 5, `item-${id}`));
  assert.deepEqual(values, ["item-3", "item-1", "item-2"]);
});

test("never exceeds the concurrency limit", async () => {
  const queue = new JobQueue(2);
  let active = 0;
  let peak = 0;
  for (let index = 0; index < 6; index += 1) {
    queue.push(async () => {
      active += 1;
      peak = Math.max(peak, active);
      await delay(5);
      active -= 1;
    });
  }
  await queue.drain();
  assert.equal(peak, 2);
});

test("a rejected job is reported without stopping the others", async () => {
  const values = await fetchAll([1, 2], async (id) => {
    if (id === 1) throw new Error("boom");
    return id;
  });
  assert.deepEqual(values, [null, 2]);
});
