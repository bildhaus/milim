//! Named agent definitions and their persistence.

use std::sync::Mutex;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use milim_core::{Error, Result};
use milim_storage::{Database, Migration};

/// A named, persisted agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub system_prompt: String,
    /// Deprecated compatibility field used only as a fallback by legacy schedules.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tool_mode: String,
    /// Tool names this agent may use; empty ⇒ all registered tools.
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default = "default_skill_mode")]
    pub skill_mode: String,
    #[serde(default)]
    pub enabled_skills: Vec<String>,
    /// Profile-picture filename under the app's `images/` (e.g.
    /// `milim-char-3.png`); empty ⇒ the UI auto-picks one from the agent id.
    #[serde(default)]
    pub avatar: String,
}

/// Schema for the agent store.
pub const AGENT_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "agents",
        sql: "CREATE TABLE agents (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            system_prompt TEXT NOT NULL DEFAULT '',
            model         TEXT NOT NULL DEFAULT '',
            enabled_tools TEXT NOT NULL DEFAULT '[]',
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
          );",
    },
    Migration {
        version: 2,
        name: "agent_avatar",
        sql: "ALTER TABLE agents ADD COLUMN avatar TEXT NOT NULL DEFAULT '';",
    },
    Migration {
        version: 3,
        name: "agent_tool_mode",
        sql: "ALTER TABLE agents ADD COLUMN tool_mode TEXT NOT NULL DEFAULT '';",
    },
    Migration {
        version: 4,
        name: "agent_skill_mode",
        sql: "ALTER TABLE agents ADD COLUMN skill_mode TEXT NOT NULL DEFAULT '';
              ALTER TABLE agents ADD COLUMN enabled_skills TEXT NOT NULL DEFAULT '[]';",
    },
    Migration {
        version: 5,
        name: "agent_description",
        sql: "ALTER TABLE agents ADD COLUMN description TEXT NOT NULL DEFAULT '';",
    },
];

fn default_skill_mode() -> String {
    "auto".to_string()
}

pub fn normalize_tool_mode(tool_mode: &str, enabled_tools: &[String]) -> String {
    match tool_mode {
        "all" | "custom" | "none" => tool_mode.to_string(),
        _ if enabled_tools.is_empty() => "all".to_string(),
        _ => "custom".to_string(),
    }
}

pub fn normalize_skill_mode(skill_mode: &str, enabled_skills: &[String]) -> String {
    match skill_mode {
        "auto" | "custom" | "none" => skill_mode.to_string(),
        _ if enabled_skills.is_empty() => "auto".to_string(),
        _ => "custom".to_string(),
    }
}

/// CRUD over [`AgentDef`] rows. `Mutex<Database>` keeps it `Sync` for sharing
/// across async request handlers (rusqlite's `Connection` is not `Sync`).
pub struct AgentStore {
    db: Mutex<Database>,
}

impl AgentStore {
    pub fn new(db: Database) -> Result<Self> {
        db.migrate(AGENT_MIGRATIONS)?;
        Ok(Self { db: Mutex::new(db) })
    }

    /// Create a new agent with a generated id.
    // ponytail: mirrors the flat agents table; use an input struct if this grows again.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        name: &str,
        description: &str,
        model: &str,
        system_prompt: &str,
        tool_mode: &str,
        enabled_tools: Vec<String>,
        skill_mode: &str,
        enabled_skills: Vec<String>,
        avatar: &str,
    ) -> Result<AgentDef> {
        let tool_mode = normalize_tool_mode(tool_mode, &enabled_tools);
        let skill_mode = normalize_skill_mode(skill_mode, &enabled_skills);
        let agent = AgentDef {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            system_prompt: system_prompt.to_string(),
            model: model.to_string(),
            tool_mode,
            enabled_tools,
            skill_mode,
            enabled_skills,
            avatar: avatar.to_string(),
        };
        self.upsert(&agent)?;
        Ok(agent)
    }

    /// Insert or replace an agent.
    pub fn upsert(&self, agent: &AgentDef) -> Result<()> {
        let tools = serde_json::to_string(&agent.enabled_tools)?;
        let skills = serde_json::to_string(&agent.enabled_skills)?;
        let tool_mode = normalize_tool_mode(&agent.tool_mode, &agent.enabled_tools);
        let skill_mode = normalize_skill_mode(&agent.skill_mode, &agent.enabled_skills);
        let db = self.db.lock().expect("agents db poisoned");
        db.conn()
            .execute(
                "INSERT INTO agents (id, name, description, system_prompt, model, tool_mode, enabled_tools, skill_mode, enabled_skills, avatar)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name, description=excluded.description,
                   system_prompt=excluded.system_prompt,
                   model=excluded.model, tool_mode=excluded.tool_mode,
                   enabled_tools=excluded.enabled_tools,
                   skill_mode=excluded.skill_mode,
                   enabled_skills=excluded.enabled_skills,
                   avatar=excluded.avatar",
                params![
                    agent.id,
                    agent.name,
                    agent.description,
                    agent.system_prompt,
                    agent.model,
                    tool_mode,
                    tools,
                    skill_mode,
                    skills,
                    agent.avatar
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    /// Fetch one agent by id.
    pub fn get(&self, id: &str) -> Result<Option<AgentDef>> {
        let db = self.db.lock().expect("agents db poisoned");
        let agent = db
            .conn()
            .query_row(
                "SELECT id, name, description, system_prompt, model, tool_mode, enabled_tools, skill_mode, enabled_skills, avatar FROM agents WHERE id = ?1",
                params![id],
                row_to_agent,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Error::Other("no rows".to_string()),
                other => sqlite(other),
            });
        match agent {
            Ok(a) => Ok(Some(a)),
            Err(Error::Other(ref m)) if m == "no rows" => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// List all agents, newest first.
    pub fn list(&self) -> Result<Vec<AgentDef>> {
        let db = self.db.lock().expect("agents db poisoned");
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, system_prompt, model, tool_mode, enabled_tools, skill_mode, enabled_skills, avatar
                 FROM agents ORDER BY created_at DESC",
            )
            .map_err(sqlite)?;
        let rows = stmt.query_map([], row_to_agent).map_err(sqlite)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(sqlite)?);
        }
        Ok(out)
    }

    /// Delete an agent. Returns whether a row was removed.
    pub fn delete(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().expect("agents db poisoned");
        let n = db
            .conn()
            .execute("DELETE FROM agents WHERE id = ?1", params![id])
            .map_err(sqlite)?;
        Ok(n > 0)
    }
}

fn row_to_agent(r: &rusqlite::Row) -> rusqlite::Result<AgentDef> {
    let tool_mode: String = r.get(5)?;
    let tools_json: String = r.get(6)?;
    let enabled_tools: Vec<String> = serde_json::from_str(&tools_json).unwrap_or_default();
    let tool_mode = normalize_tool_mode(&tool_mode, &enabled_tools);
    let skill_mode: String = r.get(7)?;
    let skills_json: String = r.get(8)?;
    let enabled_skills: Vec<String> = serde_json::from_str(&skills_json).unwrap_or_default();
    let skill_mode = normalize_skill_mode(&skill_mode, &enabled_skills);
    Ok(AgentDef {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        system_prompt: r.get(3)?,
        model: r.get(4)?,
        tool_mode,
        enabled_tools,
        skill_mode,
        enabled_skills,
        avatar: r.get(9)?,
    })
}

fn sqlite(e: rusqlite::Error) -> Error {
    Error::Other(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> AgentStore {
        AgentStore::new(Database::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn create_get_list_delete() {
        let s = store();
        let a = s
            .create(
                "Researcher",
                "Finds and verifies primary sources.",
                "test-echo",
                "You research.",
                "custom",
                vec!["echo".into()],
                "custom",
                vec!["review".into()],
                "milim-char-2.png",
            )
            .unwrap();
        let fetched = s.get(&a.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Researcher");
        assert_eq!(fetched.description, "Finds and verifies primary sources.");
        assert_eq!(fetched.model, "test-echo");
        assert_eq!(fetched.tool_mode, "custom");
        assert_eq!(fetched.enabled_tools, vec!["echo".to_string()]);
        assert_eq!(fetched.skill_mode, "custom");
        assert_eq!(fetched.enabled_skills, vec!["review".to_string()]);
        assert_eq!(fetched.avatar, "milim-char-2.png");

        assert_eq!(s.list().unwrap().len(), 1);
        assert!(s.get("missing").unwrap().is_none());

        assert!(s.delete(&a.id).unwrap());
        assert!(s.list().unwrap().is_empty());
    }

    #[test]
    fn upsert_updates_in_place() {
        let s = store();
        let mut a = s
            .create("A", "", "m", "", "all", vec![], "auto", vec![], "")
            .unwrap();
        a.name = "B".to_string();
        a.skill_mode = "".to_string();
        a.enabled_skills = vec!["skill-1".to_string()];
        s.upsert(&a).unwrap();
        assert_eq!(s.list().unwrap().len(), 1);
        let fetched = s.get(&a.id).unwrap().unwrap();
        assert_eq!(fetched.name, "B");
        assert_eq!(fetched.skill_mode, "custom");
    }
}
