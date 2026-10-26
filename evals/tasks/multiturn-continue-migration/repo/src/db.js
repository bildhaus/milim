// In-memory store with a callback API (`db.*`) and the same operations
// returning promises (`db.promises.*`). Callbacks run asynchronously, like
// the real driver's.
import { promisify } from "node:util";
import { seed } from "./seed.js";

let tables = structuredClone(seed);
let nextId = 100;
const later = (callback, error, value) => setImmediate(() => callback(error, value === undefined ? value : structuredClone(value)));

export const db = {
  find(table, id, callback) {
    later(callback, null, tables[table].find((row) => row.id === id) ?? null);
  },
  list(table, predicate, callback) {
    later(callback, null, tables[table].filter(predicate));
  },
  insert(table, row, callback) {
    const created = { id: `${table[0]}${(nextId += 1)}`, ...row };
    tables[table].push(created);
    later(callback, null, created);
  },
  update(table, id, patch, callback) {
    const row = tables[table].find((candidate) => candidate.id === id);
    if (!row) return later(callback, new Error(`${table} ${id} not found`));
    Object.assign(row, patch);
    later(callback, null, row);
  },
  remove(table, id, callback) {
    tables[table] = tables[table].filter((row) => row.id !== id);
    later(callback, null, null);
  },
  /** Restore the seed data (tests). */
  reset() {
    tables = structuredClone(seed);
    nextId = 100;
  },
};

db.promises = {
  find: promisify(db.find),
  list: promisify(db.list),
  insert: promisify(db.insert),
  update: promisify(db.update),
  remove: promisify(db.remove),
};
