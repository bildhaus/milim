import { JobQueue } from "./queue.js";

/** Fetch every id through `load`, two at a time, keeping input order. */
export async function fetchAll(ids, load) {
  const queue = new JobQueue(2);
  for (const id of ids) queue.push(() => load(id));
  const results = await queue.drain();
  return results.map((result) => (result.ok ? result.value : null));
}
