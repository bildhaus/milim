//! `todo_write`: a harness-held checklist the model keeps for multi-step work.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use milim_core::{Error, Result};

use crate::{Tool, ToolConcurrency, ToolEffect};

const MAX_TODOS: usize = 100;
const MAX_TODO_CHARS: usize = 500;
const MAX_TRACKED_LISTS: usize = 512;

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TodoWriteArgs {
    todos: Vec<TodoItem>,
}

/// Latest list per scope key, in insertion order for bounded eviction.
#[derive(Default)]
struct TodoLists {
    lists: HashMap<String, Vec<TodoItem>>,
    order: Vec<String>,
}

fn todo_lists() -> &'static Mutex<TodoLists> {
    static LISTS: OnceLock<Mutex<TodoLists>> = OnceLock::new();
    LISTS.get_or_init(Default::default)
}

fn next_run_key() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("run:{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
fn thread_todos(thread_id: &str) -> Option<Vec<TodoItem>> {
    let lists = todo_lists().lock().ok()?;
    lists.lists.get(&format!("thread:{thread_id}")).cloned()
}

/// Replace the run's (or, when thread-scoped, the thread's) checklist.
#[derive(Clone)]
pub struct TodoWriteTool {
    key: Arc<str>,
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self {
            key: Arc::from("unscoped"),
        }
    }
}

impl TodoWriteTool {
    fn store(&self, todos: Vec<TodoItem>) -> Option<Vec<TodoItem>> {
        let mut lists = todo_lists().lock().ok()?;
        let key = self.key.to_string();
        let previous = lists.lists.insert(key.clone(), todos);
        if previous.is_none() {
            lists.order.push(key);
            while lists.order.len() > MAX_TRACKED_LISTS {
                let evicted = lists.order.remove(0);
                lists.lists.remove(&evicted);
            }
        }
        previous
    }
}

fn render_checklist(todos: &[TodoItem]) -> String {
    if todos.is_empty() {
        return "Todo list cleared.".to_string();
    }
    let completed = todos
        .iter()
        .filter(|todo| todo.status == TodoStatus::Completed)
        .count();
    let mut text = format!("Todo list ({completed}/{} completed):", todos.len());
    for todo in todos {
        let mark = match todo.status {
            TodoStatus::Pending => "[ ]",
            TodoStatus::InProgress => "[~]",
            TodoStatus::Completed => "[x]",
        };
        text.push_str(&format!("\n{mark} {}", todo.content));
    }
    text
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todo_write"
    }

    fn description(&self) -> &str {
        "Replace your task checklist for multi-step work. Send the whole list each time; keep at most one item in_progress and mark items completed as soon as they are done."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "maxItems": MAX_TODOS,
                    "description": "The complete checklist, replacing any previous list. Send an empty array to clear it.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string", "maxLength": MAX_TODO_CHARS },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                        },
                        "required": ["content", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["todos"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Exclusive
    }

    fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            key: Arc::from(next_run_key()),
        }))
    }

    fn scoped_to_thread(&self, thread_id: &str) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            key: Arc::from(format!("thread:{thread_id}")),
        }))
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let TodoWriteArgs { mut todos } = serde_json::from_value(args)
            .map_err(|error| Error::InvalidRequest(format!("invalid todos: {error}")))?;
        if todos.len() > MAX_TODOS {
            return Err(Error::InvalidRequest(format!(
                "todos may contain at most {MAX_TODOS} items"
            )));
        }
        for todo in &mut todos {
            todo.content = todo.content.trim().to_string();
            if todo.content.is_empty() || todo.content.chars().count() > MAX_TODO_CHARS {
                return Err(Error::InvalidRequest(format!(
                    "each todo content must be non-empty and at most {MAX_TODO_CHARS} characters"
                )));
            }
        }
        let in_progress = todos
            .iter()
            .filter(|todo| todo.status == TodoStatus::InProgress)
            .count();
        if in_progress > 1 {
            return Err(Error::InvalidRequest(format!(
                "at most one todo may be in_progress; got {in_progress}"
            )));
        }
        let previous = self.store(todos.clone());
        Ok(json!({
            "todos": todos,
            "previous_count": previous.map(|list| list.len()).unwrap_or(0),
        }))
    }

    fn model_text(&self, result: &Value) -> Option<String> {
        let todos: Vec<TodoItem> = serde_json::from_value(result.get("todos")?.clone()).ok()?;
        Some(render_checklist(&todos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn todo_write_replaces_the_thread_list_and_renders_a_checklist() {
        let base = TodoWriteTool::default();
        let tool = base.scoped_to_thread("thread-todo-test").unwrap();
        let first = tool
            .invoke(json!({ "todos": [
                { "content": "Read the code", "status": "completed" },
                { "content": "Write the fix", "status": "in_progress" },
                { "content": "Run tests", "status": "pending" }
            ]}))
            .await
            .unwrap();
        assert_eq!(first["previous_count"], 0);
        assert_eq!(
            tool.model_text(&first).unwrap(),
            "Todo list (1/3 completed):\n[x] Read the code\n[~] Write the fix\n[ ] Run tests"
        );

        // A later run in the same thread sees and replaces the same list.
        let later = base
            .scoped_for_run()
            .unwrap()
            .scoped_to_thread("thread-todo-test")
            .unwrap();
        let second = later
            .invoke(json!({ "todos": [{ "content": "Run tests", "status": "completed" }] }))
            .await
            .unwrap();
        assert_eq!(second["previous_count"], 3);
        assert_eq!(
            thread_todos("thread-todo-test").unwrap(),
            vec![TodoItem {
                content: "Run tests".into(),
                status: TodoStatus::Completed
            }]
        );

        let cleared = later.invoke(json!({ "todos": [] })).await.unwrap();
        assert_eq!(later.model_text(&cleared).unwrap(), "Todo list cleared.");
    }

    #[tokio::test]
    async fn todo_write_rejects_invalid_lists() {
        let tool = TodoWriteTool::default().scoped_for_run().unwrap();
        for invalid in [
            json!({}),
            json!({ "todos": [{ "content": "x", "status": "done" }] }),
            json!({ "todos": [{ "content": "  ", "status": "pending" }] }),
            json!({ "todos": [
                { "content": "a", "status": "in_progress" },
                { "content": "b", "status": "in_progress" }
            ]}),
        ] {
            assert!(tool.invoke(invalid).await.is_err());
        }
        assert_eq!(tool.effect(), ToolEffect::ReadOnly);
        assert_eq!(tool.concurrency(), ToolConcurrency::Exclusive);
    }
}
