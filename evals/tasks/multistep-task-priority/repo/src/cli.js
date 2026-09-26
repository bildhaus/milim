#!/usr/bin/env node
import { addTask, completeTask, createStore } from "./store.js";
import { openTasks } from "./query.js";

function format(task) {
  return `${task.id}. ${task.done ? "(done) " : ""}${task.text}`;
}

/** Run one command against `store` and return the output lines. */
export function run(store, argv) {
  const [command, ...args] = argv;
  switch (command) {
    case "add": {
      const task = addTask(store, args.join(" "));
      return [`Added ${format(task)}`];
    }
    case "done":
      return [`Completed ${format(completeTask(store, Number(args[0])))}`];
    case "list": {
      const tasks = args.includes("--all") ? store.tasks : openTasks(store.tasks);
      return tasks.map(format);
    }
    default:
      return ["usage: todo add|done|list"];
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  for (const line of run(createStore(), process.argv.slice(2))) console.log(line);
}
