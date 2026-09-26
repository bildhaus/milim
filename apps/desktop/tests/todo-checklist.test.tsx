import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { TodoChecklist, parseTodoArguments } = (await server.ssrLoadModule("/src/components/TodoChecklist.tsx")) as {
    TodoChecklist: ComponentType<{ argumentsText?: string }>;
    parseTodoArguments: (text?: string) => Array<{ content: string; status: string }> | null;
  };
  const args = JSON.stringify({ todos: [
    { content: "Read the code", status: "completed" },
    { content: "Write the fix", status: "in_progress" },
    { content: "Run tests", status: "pending" },
  ] });
  const markup = renderToStaticMarkup(createElement(TodoChecklist, { argumentsText: args }));
  assert(markup.includes('data-testid="todo-checklist"'), "todo_write arguments should render a checklist");
  assert(markup.includes("todo-checklist-completed") && markup.includes("todo-checklist-in-progress") && markup.includes("todo-checklist-pending"), "each status should have its own class");
  assert(markup.includes("Write the fix") && markup.includes(">In progress<"), "items should carry their text and accessible status");
  assert(parseTodoArguments('{"todos":[{"content":"x","status":"done"}]}') === null, "unknown statuses should not render");
  assert(parseTodoArguments("not json") === null, "malformed arguments should not render");
  assert(renderToStaticMarkup(createElement(TodoChecklist, { argumentsText: '{"todos":[]}' })) === "", "an empty list renders nothing");
} finally {
  await server.close();
}
