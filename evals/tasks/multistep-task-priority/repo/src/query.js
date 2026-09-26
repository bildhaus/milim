export function openTasks(tasks) {
  return tasks.filter((task) => !task.done);
}
