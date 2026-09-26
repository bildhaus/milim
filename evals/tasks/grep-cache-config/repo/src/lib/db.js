const tables = new Map();
let nextId = 1;

function table(name) {
  if (!tables.has(name)) tables.set(name, new Map());
  return tables.get(name);
}

export const db = {
  all: (name) => [...table(name).values()],
  get: (name, id) => table(name).get(String(id)),
  insert(name, row) {
    const id = String(nextId++);
    const stored = { ...row, id };
    table(name).set(id, stored);
    return stored;
  },
  upsert(name, id, row) {
    table(name).set(String(id), { ...row, id: String(id) });
  },
  remove: (name, id) => table(name).delete(String(id)),
  reset() {
    tables.clear();
    nextId = 1;
  },
};
