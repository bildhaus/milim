export function createStore() {
  return { nextId: 1, tasks: [] };
}

export function addTask(store, text) {
  if (typeof text !== "string" || text.trim() === "") {
    throw new TypeError("task text is required");
  }
  const task = { id: store.nextId++, text: text.trim(), done: false };
  store.tasks.push(task);
  return task;
}

export function completeTask(store, id) {
  const task = store.tasks.find((candidate) => candidate.id === id);
  if (!task) throw new Error(`no task ${id}`);
  task.done = true;
  return task;
}
