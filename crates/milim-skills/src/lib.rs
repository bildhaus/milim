//! `milim-skills` — reusable AI capabilities (milim "skills").
//!
//! A skill is a `SKILL.md`: YAML-ish frontmatter (`name`, `description`) plus
//! markdown instructions. Skills are persisted and selected by keyword
//! relevance so the agent loop can inject the most relevant ones into its
//! prompt.

use std::cmp::Reverse;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use milim_core::{Error, Result};
use milim_storage::{Database, Migration};

/// A persisted skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_source_kind")]
    pub source_kind: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub updated_at: String,
}

/// Schema for the skills store.
pub const SKILL_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "skills",
        sql: "CREATE TABLE skills (
            id           TEXT PRIMARY KEY,
            name         TEXT NOT NULL,
            description  TEXT NOT NULL DEFAULT '',
            instructions TEXT NOT NULL DEFAULT '',
            created_at   TEXT NOT NULL DEFAULT (datetime('now'))
          );",
    },
    Migration {
        version: 2,
        name: "skill_metadata",
        sql: "ALTER TABLE skills ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
              ALTER TABLE skills ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'manual';
              ALTER TABLE skills ADD COLUMN source_url TEXT;
              ALTER TABLE skills ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';",
    },
];

fn default_true() -> bool {
    true
}

fn default_source_kind() -> String {
    "manual".to_string()
}

fn skill_name_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Parse a `SKILL.md` into `(name, description, instructions)`.
pub fn parse_skill_md(md: &str) -> (String, String, String) {
    let lines: Vec<&str> = md.lines().collect();
    if lines.first().map(|l| l.trim()) == Some("---") {
        if let Some(rel) = lines.iter().skip(1).position(|l| l.trim() == "---") {
            let close = rel + 1; // index of the closing `---` in `lines`
            let mut name = String::new();
            let mut description = String::new();
            for line in &lines[1..close] {
                if let Some((k, v)) = line.split_once(':') {
                    let val = v.trim().trim_matches('"').to_string();
                    match k.trim() {
                        "name" => name = val,
                        "description" => description = val,
                        _ => {}
                    }
                }
            }
            let body = lines[close + 1..].join("\n").trim().to_string();
            return (name, description, body);
        }
    }
    let name = lines
        .first()
        .map(|l| l.trim_start_matches('#').trim())
        .unwrap_or("")
        .to_string();
    (name, String::new(), md.trim().to_string())
}

/// CRUD + selection over skills. `Mutex<Database>` keeps it `Sync`.
pub struct SkillStore {
    db: Mutex<Database>,
}

impl SkillStore {
    pub fn new(db: Database) -> Result<Self> {
        db.migrate(SKILL_MIGRATIONS)?;
        Ok(Self { db: Mutex::new(db) })
    }

    pub fn create(&self, name: &str, description: &str, instructions: &str) -> Result<SkillDef> {
        self.create_with_source(name, description, instructions, true, "manual", None)
    }

    pub fn create_with_source(
        &self,
        name: &str,
        description: &str,
        instructions: &str,
        enabled: bool,
        source_kind: &str,
        source_url: Option<String>,
    ) -> Result<SkillDef> {
        let skill = SkillDef {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            instructions: instructions.to_string(),
            enabled,
            source_kind: source_kind.to_string(),
            source_url,
            updated_at: String::new(),
        };
        self.upsert(&skill)?;
        self.get(&skill.id)?
            .ok_or_else(|| Error::Other("created skill missing".to_string()))
    }

    /// Create a skill by parsing a `SKILL.md` document.
    pub fn create_from_md(&self, md: &str) -> Result<SkillDef> {
        self.create_from_md_with_source(md, true, "pasted", None)
    }

    pub fn create_from_md_with_source(
        &self,
        md: &str,
        enabled: bool,
        source_kind: &str,
        source_url: Option<String>,
    ) -> Result<SkillDef> {
        let (name, description, instructions) = parse_skill_md(md);
        if name.is_empty() {
            return Err(Error::InvalidRequest("skill is missing a name".to_string()));
        }
        self.create_with_source(
            &name,
            &description,
            &instructions,
            enabled,
            source_kind,
            source_url,
        )
    }

    pub fn import_global_skills(&self) -> Result<usize> {
        self.import_skill_dirs(&default_global_skill_dirs())
    }

    pub fn import_skill_dirs(&self, dirs: &[PathBuf]) -> Result<usize> {
        let mut files = Vec::new();
        for dir in dirs {
            collect_skill_files(dir, &mut files);
        }
        let mut count = 0;
        for path in files {
            let Ok(md) = fs::read_to_string(&path) else {
                continue;
            };
            let (name, description, instructions) = parse_skill_md(&md);
            if name.is_empty() {
                continue;
            }
            let source = fs::canonicalize(&path)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            let name_key = skill_name_key(&name);
            // ponytail: O(n) lookup is fine for local skill counts; add an index if this grows.
            let existing = self.list()?.into_iter().find(|s| {
                s.source_url.as_deref() == Some(source.as_str())
                    || (s.source_kind == "global" && skill_name_key(&s.name) == name_key)
            });
            let kept_id = if let Some(existing) = existing {
                let mut updated = existing;
                updated.name = name;
                updated.description = description;
                updated.instructions = instructions;
                updated.source_kind = "global".to_string();
                updated.source_url = Some(source);
                let kept_id = updated.id.clone();
                self.update(&updated)?;
                kept_id
            } else {
                self.create_with_source(
                    &name,
                    &description,
                    &instructions,
                    true,
                    "global",
                    Some(source),
                )?
                .id
            };
            for dupe in self.list()?.into_iter().filter(|s| {
                s.id != kept_id && s.source_kind == "global" && skill_name_key(&s.name) == name_key
            }) {
                self.delete(&dupe.id)?;
            }
            count += 1;
        }
        Ok(count)
    }

    pub fn upsert(&self, skill: &SkillDef) -> Result<()> {
        let db = self.db.lock().expect("skills db poisoned");
        db.conn()
            .execute(
                "INSERT INTO skills (id, name, description, instructions, enabled, source_kind, source_url, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
                 ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name, description=excluded.description,
                   instructions=excluded.instructions, enabled=excluded.enabled,
                   source_kind=excluded.source_kind, source_url=excluded.source_url,
                   updated_at=datetime('now')",
                params![
                    skill.id,
                    skill.name,
                    skill.description,
                    skill.instructions,
                    skill.enabled,
                    skill.source_kind,
                    skill.source_url
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn update(&self, skill: &SkillDef) -> Result<Option<SkillDef>> {
        let db = self.db.lock().expect("skills db poisoned");
        let changed = db
            .conn()
            .execute(
                "UPDATE skills
                 SET name = ?2, description = ?3, instructions = ?4,
                     enabled = ?5, source_kind = ?6, source_url = ?7,
                     updated_at = datetime('now')
                 WHERE id = ?1",
                params![
                    skill.id,
                    skill.name,
                    skill.description,
                    skill.instructions,
                    skill.enabled,
                    skill.source_kind,
                    skill.source_url
                ],
            )
            .map_err(sqlite)?;
        drop(db);
        if changed == 0 {
            return Ok(None);
        }
        self.get(&skill.id)
    }

    pub fn get(&self, id: &str) -> Result<Option<SkillDef>> {
        let db = self.db.lock().expect("skills db poisoned");
        db.conn()
            .query_row(
                "SELECT id, name, description, instructions, enabled, source_kind, source_url, updated_at
                 FROM skills WHERE id = ?1",
                params![id],
                row_to_skill,
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(sqlite(other)),
            })
    }

    pub fn list(&self) -> Result<Vec<SkillDef>> {
        let db = self.db.lock().expect("skills db poisoned");
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, instructions, enabled, source_kind, source_url, updated_at
                 FROM skills ORDER BY name",
            )
            .map_err(sqlite)?;
        let rows = stmt.query_map([], row_to_skill).map_err(sqlite)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(sqlite)?);
        }
        Ok(out)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().expect("skills db poisoned");
        let n = db
            .conn()
            .execute("DELETE FROM skills WHERE id = ?1", params![id])
            .map_err(sqlite)?;
        Ok(n > 0)
    }

    /// Select up to `limit` skills most relevant to `query` (keyword scoring).
    pub fn select(&self, query: &str, limit: usize) -> Result<Vec<SkillDef>> {
        self.select_filtered(query, limit, None)
    }

    /// Select enabled skills, optionally restricted to an Agent allowlist.
    pub fn select_filtered(
        &self,
        query: &str,
        limit: usize,
        allowed_ids: Option<&[String]>,
    ) -> Result<Vec<SkillDef>> {
        let terms: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .filter(|t| t.len() >= 3) // drop trivial words ("i", "do", "to", …)
            .map(str::to_string)
            .collect();
        let mut scored: Vec<(usize, SkillDef)> = self
            .list()?
            .into_iter()
            .filter(|s| s.enabled && allowed_ids.is_none_or(|ids| ids.iter().any(|id| id == &s.id)))
            .map(|s| {
                let hay = format!("{} {} {}", s.name, s.description, s.instructions).to_lowercase();
                let explicit_tag = contains_explicit_skill_tag(query, &s.name);
                let score = terms.iter().filter(|t| hay.contains(t.as_str())).count()
                    + usize::from(explicit_tag) * 1_000;
                (score, s)
            })
            .filter(|(score, _)| *score > 0)
            .collect();
        scored.sort_by_key(|(score, _)| Reverse(*score));
        Ok(scored.into_iter().take(limit).map(|(_, s)| s).collect())
    }
}

fn contains_explicit_skill_tag(query: &str, name: &str) -> bool {
    let query = query.to_lowercase();
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return false;
    }
    [format!("@{name}"), format!("/{name}")]
        .into_iter()
        .any(|needle| {
            query.match_indices(&needle).any(|(start, _)| {
                let start_ok = start == 0
                    || query[..start]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{'));
                let end = start + needle.len();
                let end_ok = end == query.len()
                    || query[end..].chars().next().is_some_and(|c| {
                        c.is_whitespace()
                            || matches!(
                                c,
                                ',' | '.'
                                    | ';'
                                    | ':'
                                    | '!'
                                    | '?'
                                    | '('
                                    | ')'
                                    | '['
                                    | ']'
                                    | '{'
                                    | '}'
                                    | '"'
                                    | '\''
                                    | '`'
                            )
                    });
                start_ok && end_ok
            })
        })
}

pub fn default_global_skill_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(codex_home) = env::var_os("CODEX_HOME") {
        dirs.push(PathBuf::from(codex_home).join("skills"));
    }
    if let Some(home) = home_dir() {
        let codex = home.join(".codex").join("skills");
        if !dirs.contains(&codex) {
            dirs.push(codex);
        }
        dirs.push(home.join(".agents").join("skills"));
        dirs.push(home.join(".claude").join("skills"));
    }
    dirs
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
}

fn collect_skill_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files(&path, out);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
            out.push(path);
        }
    }
}

fn row_to_skill(r: &rusqlite::Row) -> rusqlite::Result<SkillDef> {
    Ok(SkillDef {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        instructions: r.get(3)?,
        enabled: r.get::<_, i64>(4)? != 0,
        source_kind: r.get(5)?,
        source_url: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn sqlite(e: rusqlite::Error) -> Error {
    Error::Other(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SkillStore {
        SkillStore::new(Database::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn parses_skill_md_frontmatter() {
        let md = "---\nname: Git Helper\ndescription: \"Work with git\"\n---\n# Git\nRun git commands carefully.";
        let (name, desc, body) = parse_skill_md(md);
        assert_eq!(name, "Git Helper");
        assert_eq!(desc, "Work with git");
        assert!(body.contains("Run git commands"));
    }

    #[test]
    fn create_from_md_and_select() {
        let s = store();
        s.create_from_md("---\nname: Git Helper\ndescription: version control\n---\nUse git.")
            .unwrap();
        s.create_from_md("---\nname: Mailer\ndescription: send email\n---\nUse SMTP.")
            .unwrap();
        assert_eq!(s.list().unwrap().len(), 2);

        // "Git Helper" scores highest (matches "git" + "version control"); the
        // top result is what the agent loop would inject.
        let hits = s.select("how do I use git for version control", 5).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].name, "Git Helper");
    }

    #[test]
    fn explicit_tag_selects_the_named_skill() {
        let s = store();
        s.create_from_md(
            "---\nname: kata-workbench-creator\ndescription: Build workbenches\n---\nCreate a KATA workbench.",
        )
        .unwrap();
        s.create_from_md(
            "---\nname: Agent Helper\ndescription: Use agent profiles\n---\nManage agents.",
        )
        .unwrap();

        let hits = s
            .select("Use @kata-workbench-creator for this task.", 10)
            .unwrap();
        assert_eq!(hits[0].name, "kata-workbench-creator");
    }

    #[test]
    fn update_delete_and_enabled_selection() {
        let s = store();
        let skill = s
            .create_with_source(
                "Git Helper",
                "version control",
                "Use git.",
                false,
                "github",
                Some("https://github.com/example/skills/tree/main/git".to_string()),
            )
            .unwrap();

        assert!(s.select("git version control", 5).unwrap().is_empty());
        let mut update = skill.clone();
        update.instructions = "Use git carefully.".to_string();
        update.enabled = true;
        let updated = s.update(&update).unwrap().unwrap();
        assert!(updated.enabled);
        assert_eq!(updated.source_kind, "github");
        assert!(updated.instructions.contains("carefully"));
        assert_eq!(s.select("git version control", 5).unwrap().len(), 1);
        assert!(s
            .select_filtered("git version control", 5, Some(&["other".to_string()]))
            .unwrap()
            .is_empty());
        assert_eq!(
            s.select_filtered(
                "git version control",
                5,
                Some(std::slice::from_ref(&skill.id))
            )
            .unwrap()
            .len(),
            1
        );
        assert!(s.delete(&skill.id).unwrap());
        assert!(s.get(&skill.id).unwrap().is_none());
        assert!(s
            .update(&SkillDef {
                id: "missing".to_string(),
                name: "Nope".to_string(),
                description: String::new(),
                instructions: String::new(),
                enabled: true,
                source_kind: "manual".to_string(),
                source_url: None,
                updated_at: String::new(),
            })
            .unwrap()
            .is_none());
    }

    #[test]
    fn imports_global_skill_dirs_idempotently_and_preserves_enabled() {
        let root = env::temp_dir().join(format!("milim-skills-{}", uuid::Uuid::new_v4()));
        let skill_dir = root.join("codex").join("skills").join("review");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        fs::write(
            &skill_file,
            "---\nname: Review\ndescription: code review\n---\nCheck diffs.",
        )
        .unwrap();

        let s = store();
        let dirs = [root.join("codex").join("skills")];
        assert_eq!(s.import_skill_dirs(&dirs).unwrap(), 1);
        let imported = s.list().unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].source_kind, "global");
        assert!(imported[0].enabled);

        let mut disabled = imported[0].clone();
        disabled.enabled = false;
        s.update(&disabled).unwrap();
        fs::write(
            &skill_file,
            "---\nname: Review\ndescription: code review\n---\nCheck diffs carefully.",
        )
        .unwrap();

        assert_eq!(s.import_skill_dirs(&dirs).unwrap(), 1);
        let refreshed = s.list().unwrap();
        assert_eq!(refreshed.len(), 1);
        assert!(!refreshed[0].enabled);
        assert!(refreshed[0].instructions.contains("carefully"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_global_skill_dirs_dedupes_same_name_across_dirs() {
        let root = env::temp_dir().join(format!("milim-skills-{}", uuid::Uuid::new_v4()));
        let codex_skill = root
            .join("codex")
            .join("skills")
            .join("caption")
            .join("SKILL.md");
        let agents_skill = root
            .join("agents")
            .join("skills")
            .join("caption")
            .join("SKILL.md");
        fs::create_dir_all(codex_skill.parent().unwrap()).unwrap();
        fs::create_dir_all(agents_skill.parent().unwrap()).unwrap();
        fs::write(
            &codex_skill,
            "---\nname: Caption Helper\ndescription: captions\n---\nUse concise captions.",
        )
        .unwrap();
        fs::write(
            &agents_skill,
            "---\nname:  caption helper \ndescription: duplicate captions\n---\nUse duplicate captions.",
        )
        .unwrap();

        let s = store();
        let dirs = [
            root.join("codex").join("skills"),
            root.join("agents").join("skills"),
        ];
        assert_eq!(s.import_skill_dirs(&dirs).unwrap(), 2);
        let imported = s.list().unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(skill_name_key(&imported[0].name), "caption helper");
        assert_eq!(imported[0].source_kind, "global");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unnamed_skill() {
        let s = store();
        assert!(s
            .create_from_md("just some text with no frontmatter and no heading line")
            .is_ok());
        // empty doc → no name → error
        assert!(s.create_from_md("").is_err());
    }
}
