/**
 * Run async jobs with at most `concurrency` in flight. `drain()` resolves
 * once every job pushed so far has settled, and results are reported in
 * push order regardless of completion order.
 */
export class JobQueue {
  constructor(concurrency = 2) {
    this.concurrency = concurrency;
    this.running = 0;
    this.pending = [];
    this.results = [];
    this.waiters = [];
  }

  push(job) {
    const index = this.results.length;
    this.results.push(undefined);
    this.pending.push({ job, index });
    this.#pump();
  }

  #pump() {
    while (this.running < this.concurrency && this.pending.length > 0) {
      const { job, index } = this.pending.shift();
      this.running += 1;
      this.#run(job, index);
    }
    if (this.running === 0 && this.pending.length === 0) {
      for (const resolve of this.waiters.splice(0)) resolve(this.results);
    }
  }

  async #run(job, index) {
    try {
      this.results[index] = { ok: true, value: job() };
    } catch (error) {
      this.results[index] = { ok: false, error: error.message };
    } finally {
      this.running -= 1;
      this.#pump();
    }
  }

  drain() {
    if (this.running === 0 && this.pending.length === 0) {
      return Promise.resolve(this.results);
    }
    return new Promise((resolve) => this.waiters.push(resolve));
  }
}
