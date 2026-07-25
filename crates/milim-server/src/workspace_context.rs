use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

const AGENTS_MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WorkspaceContext {
    pub root: Option<String>,
    pub project_locator: Option<String>,
    pub legacy_project_locator: Option<String>,
    pub origin: Option<String>,
    pub instructions: Vec<WorkspaceInstruction>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WorkspaceInstruction {
    pub family: &'static str,
    pub scope: &'static str,
    pub path: String,
    pub content: String,
    pub bytes: usize,
    pub status: &'static str,
}

pub(crate) fn resolve(folder: Option<&Path>) -> WorkspaceContext {
    let Some(folder) = folder else {
        return WorkspaceContext {
            root: None,
            project_locator: None,
            legacy_project_locator: None,
            origin: None,
            instructions: Vec::new(),
            warnings: Vec::new(),
        };
    };
    let legacy_folder = folder.to_path_buf();
    let folder = canonical(folder);
    let git_root = git(&folder, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .map(|path| canonical(&path));
    let root = git_root.as_deref().unwrap_or(&folder);
    let origin = git(root, &["config", "--get", "remote.origin.url"])
        .and_then(|value| normalize_origin(&value));
    let project_locator = origin
        .as_deref()
        .map(|value| format!("git:{value}"))
        .or_else(|| Some(format!("path:{}", canonical(root).display())));
    let mut context = WorkspaceContext {
        root: Some(root.display().to_string()),
        project_locator,
        legacy_project_locator: Some(legacy_folder.display().to_string()),
        origin,
        instructions: Vec::new(),
        warnings: Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut agents_bytes = 0;

    if let Some(home) = home_dir() {
        let codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        add_first_agents(&mut context, &mut seen, &mut agents_bytes, &codex, "global");
        add_file(
            &mut context,
            &mut seen,
            home.join(".claude").join("CLAUDE.md"),
            "claude",
            "global",
            false,
            None,
        );
        add_rules(
            &mut context,
            &mut seen,
            &home.join(".claude").join("rules"),
            "global",
        );
    }

    for dir in project_chain(root, &folder) {
        add_first_agents(&mut context, &mut seen, &mut agents_bytes, &dir, "project");
        for relative in ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"] {
            add_file(
                &mut context,
                &mut seen,
                dir.join(relative),
                "claude",
                "project",
                false,
                None,
            );
        }
        add_rules(
            &mut context,
            &mut seen,
            &dir.join(".claude").join("rules"),
            "project",
        );
    }
    context
}

pub(crate) fn formatted(context: &WorkspaceContext, family: Option<&str>) -> Option<String> {
    let loaded: Vec<_> = context
        .instructions
        .iter()
        .filter(|item| item.status == "loaded" && family.is_none_or(|f| item.family == f))
        .collect();
    if loaded.is_empty() {
        return None;
    }
    let mut text = String::from(
        "Repository and user instructions, ordered from broadest to most specific. Later instructions take precedence on conflicts.\n",
    );
    for item in loaded {
        text.push_str("\n## From: ");
        text.push_str(&item.path);
        text.push('\n');
        text.push_str(&item.content);
        text.push('\n');
    }
    Some(text)
}

fn add_first_agents(
    context: &mut WorkspaceContext,
    seen: &mut HashSet<PathBuf>,
    bytes: &mut usize,
    dir: &Path,
    scope: &'static str,
) {
    for name in ["AGENTS.override.md", "AGENTS.md"] {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            context
                .warnings
                .push(format!("Could not read {}", path.display()));
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        add_file(context, seen, path, "agents", scope, false, Some(bytes));
        return;
    }
}

fn add_rules(
    context: &mut WorkspaceContext,
    seen: &mut HashSet<PathBuf>,
    dir: &Path,
    scope: &'static str,
) {
    let mut files = Vec::new();
    collect_markdown(dir, &mut files, &mut context.warnings);
    files.sort();
    for path in files {
        add_file(context, seen, path, "claude", scope, true, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn add_file(
    context: &mut WorkspaceContext,
    seen: &mut HashSet<PathBuf>,
    path: PathBuf,
    family: &'static str,
    scope: &'static str,
    rule: bool,
    agents_bytes: Option<&mut usize>,
) {
    if !path.is_file() {
        return;
    }
    let canonical_path = canonical(&path);
    if !seen.insert(canonical_path) {
        return;
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(content) if !content.trim().is_empty() => content,
        Ok(_) => return,
        Err(error) => {
            context
                .warnings
                .push(format!("Could not read {}: {error}", path.display()));
            return;
        }
    };
    let bytes = content.len();
    let mut status = "loaded";
    if rule && has_paths_frontmatter(&content) {
        status = "conditional";
        context.warnings.push(format!(
            "Skipped path-conditional Claude rule {} outside the Claude runtime",
            path.display()
        ));
    }
    if let Some(total) = agents_bytes {
        if *total + bytes > AGENTS_MAX_BYTES {
            status = "limit_exceeded";
            context.warnings.push(format!(
                "Skipped {} because AGENTS instructions exceed 32 KiB",
                path.display()
            ));
        } else {
            *total += bytes;
        }
    }
    context.instructions.push(WorkspaceInstruction {
        family,
        scope,
        path: path.display().to_string(),
        content: if status == "loaded" {
            content
        } else {
            String::new()
        },
        bytes,
        status,
    });
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>, warnings: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            warnings.push(format!(
                "Could not read Claude rules directory {}: {error}",
                dir.display()
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!(
                    "Could not inspect Claude rule in {}: {error}",
                    dir.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out, warnings);
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("md"))
        {
            out.push(path);
        }
    }
}

fn has_paths_frontmatter(content: &str) -> bool {
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            return false;
        }
        if line
            .split_once(':')
            .is_some_and(|(key, _)| key.trim() == "paths")
        {
            return true;
        }
    }
    false
}

fn project_chain(root: &Path, folder: &Path) -> Vec<PathBuf> {
    if !folder.starts_with(root) {
        return vec![folder.to_path_buf()];
    }
    let mut chain = Vec::new();
    let mut current = Some(folder);
    while let Some(dir) = current {
        chain.push(dir.to_path_buf());
        if dir == root {
            break;
        }
        current = dir.parent();
    }
    chain.reverse();
    chain
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(cwd).args(args);
    let output = milim_core::proc::hide_console(&mut command).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_origin(value: &str) -> Option<String> {
    let mut value = value.trim().trim_end_matches('/').to_string();
    if value.is_empty() || value.starts_with("file:") || Path::new(&value).is_absolute() {
        return None;
    }
    if let Some((_, rest)) = value.split_once("://") {
        let rest = rest.split(['?', '#']).next().unwrap_or(rest);
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?.to_ascii_lowercase();
        value = format!("{host}/{path}");
    } else if let Some((authority, path)) = value.split_once(':') {
        if authority.len() == 1 || path.contains('\\') {
            return None;
        }
        let host = authority.rsplit('@').next()?.to_ascii_lowercase();
        let path = path.split(['?', '#']).next().unwrap_or(path);
        value = format!("{host}/{path}");
    } else {
        return None;
    }
    let mut value = value.trim_end_matches('/').to_string();
    if value
        .get(value.len().saturating_sub(4)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".git"))
    {
        value.truncate(value.len() - 4);
    }
    (!value.is_empty()).then_some(value)
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_context() -> WorkspaceContext {
        WorkspaceContext {
            root: None,
            project_locator: None,
            legacy_project_locator: None,
            origin: None,
            instructions: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn origins_share_identity_across_transport() {
        assert_eq!(
            normalize_origin("git@GitHub.com:Owner/Repo.git"),
            Some("github.com/Owner/Repo".to_string())
        );
        assert_eq!(
            normalize_origin("https://token@github.com/Owner/Repo.git?x=1"),
            Some("github.com/Owner/Repo".to_string())
        );
        assert_eq!(normalize_origin("C:\\repo"), None);
        assert_eq!(
            normalize_origin("ssh://user:secret@GitHub.com/Owner/Repo.git#branch"),
            Some("github.com/Owner/Repo".to_string())
        );
        assert_eq!(
            normalize_origin("git@GitHub.com:Owner/Repo.git?token=secret"),
            Some("github.com/Owner/Repo".to_string())
        );
    }

    #[test]
    fn detects_only_frontmatter_paths() {
        assert!(has_paths_frontmatter("---\npaths:\n - src/**\n---\nrule"));
        assert!(!has_paths_frontmatter("# paths:\nrule"));
        assert!(!has_paths_frontmatter("---\ntags: [x]\n---\npaths: later"));
    }

    #[test]
    fn agents_override_wins_and_claude_conditional_rules_are_visible() {
        let dir = std::env::temp_dir().join(format!("milim-context-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("rules")).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "base").unwrap();
        std::fs::write(dir.join("AGENTS.override.md"), "override").unwrap();
        std::fs::write(dir.join("rules").join("always.md"), "always").unwrap();
        std::fs::write(
            dir.join("rules").join("conditional.md"),
            "---\npaths:\n  - src/**\n---\nconditional",
        )
        .unwrap();

        let mut context = empty_context();
        let mut seen = HashSet::new();
        let mut bytes = 0;
        add_first_agents(&mut context, &mut seen, &mut bytes, &dir, "project");
        add_rules(&mut context, &mut seen, &dir.join("rules"), "project");

        assert!(context.instructions.iter().any(|file| {
            file.path.ends_with("AGENTS.override.md") && file.content == "override"
        }));
        assert!(!context
            .instructions
            .iter()
            .any(|file| file.path.ends_with("AGENTS.md")));
        assert!(context.instructions.iter().any(|file| {
            file.path.ends_with("conditional.md")
                && file.status == "conditional"
                && file.content.is_empty()
        }));
        assert_eq!(context.warnings.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn agents_aggregate_limit_never_loads_more_than_32_kib() {
        let dir =
            std::env::temp_dir().join(format!("milim-context-limit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "x".repeat(AGENTS_MAX_BYTES + 1)).unwrap();
        let mut context = empty_context();
        let mut seen = HashSet::new();
        let mut bytes = 0;
        add_first_agents(&mut context, &mut seen, &mut bytes, &dir, "project");
        assert_eq!(bytes, 0);
        assert_eq!(context.instructions[0].status, "limit_exceeded");
        assert!(context.instructions[0].content.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
