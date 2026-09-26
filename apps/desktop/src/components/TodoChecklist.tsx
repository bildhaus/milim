import { Check } from "./icons";

export type TodoChecklistStatus = "pending" | "in_progress" | "completed";

export type TodoChecklistItem = {
  content: string;
  status: TodoChecklistStatus;
};

const STATUS_LABEL: Record<TodoChecklistStatus, string> = {
  pending: "Pending",
  in_progress: "In progress",
  completed: "Completed",
};

/** The checklist a `todo_write` call sent; the call replaces the whole list. */
export function parseTodoArguments(argumentsText?: string): TodoChecklistItem[] | null {
  if (!argumentsText?.trim()) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(argumentsText);
  } catch {
    return null;
  }
  const todos = parsed && typeof parsed === "object" ? (parsed as { todos?: unknown }).todos : undefined;
  if (!Array.isArray(todos)) return null;
  const items: TodoChecklistItem[] = [];
  for (const todo of todos) {
    const record = todo && typeof todo === "object" ? (todo as Record<string, unknown>) : null;
    const content = typeof record?.content === "string" ? record.content.trim() : "";
    const status = record?.status;
    if (!content || (status !== "pending" && status !== "in_progress" && status !== "completed")) return null;
    items.push({ content, status });
  }
  return items;
}

export function TodoChecklist({ argumentsText }: { argumentsText?: string }) {
  const todos = parseTodoArguments(argumentsText);
  if (!todos?.length) return null;
  return (
    <ul className="todo-checklist" aria-label="Todo list" data-testid="todo-checklist">
      {todos.map((todo, index) => (
        <li key={index} className={`todo-checklist-item todo-checklist-${todo.status.replace("_", "-")}`}>
          <span className="todo-checklist-mark" aria-hidden="true">
            {todo.status === "completed" ? <Check size={10} /> : null}
          </span>
          <span className="todo-checklist-text">{todo.content}</span>
          <span className="todo-checklist-sr-only">{STATUS_LABEL[todo.status]}</span>
        </li>
      ))}
    </ul>
  );
}
