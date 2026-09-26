use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

const AGENTS_MAX_BYTES: usize = 32 * 1024;
const GIT_STATUS_MAX_LINES: usize = 20;
const GIT_RECENT_COMMITS: &str = "5";
const GIT_ENVIRONMENT_TIMEOUT: Duration = Duration::from_secs(3);

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

/// Machine and workspace facts a native agent run starts from. Captured once
/// per run so the rendered block stays byte-identical across the run's steps.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RunEnvironment {
    pub os: String,
    pub arch: String,
    pub shell: &'static str,
    pub date: String,
    pub timezone: String,
    pub workspace: Option<String>,
    pub git: Option<GitSnapshot>,
    pub model: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct GitSnapshot {
    pub root: String,
    pub branch: Option<String>,
    pub status: Vec<String>,
    pub status_total: usize,
    pub commits: Vec<String>,
}

impl RunEnvironment {
    pub(crate) fn capture(workspace: Option<&Path>, model: &str) -> Self {
        let now = chrono::Local::now();
        let offset = now.format("UTC%:z").to_string();
        Self {
            os: os_label().to_string(),
            arch: std::env::consts::ARCH.to_string(),
            shell: match milim_tools::shell_command::ShellDialect::host() {
                milim_tools::shell_command::ShellDialect::PowerShell => "PowerShell",
                milim_tools::shell_command::ShellDialect::Posix => "sh (POSIX)",
            },
            date: now.format("%Y-%m-%d").to_string(),
            timezone: match timezone_name() {
                Some(name) => format!("{name}, {offset}"),
                None => offset,
            },
            workspace: workspace.map(|path| path.display().to_string()),
            git: workspace.and_then(git_snapshot),
            model: model.to_string(),
        }
    }

    pub(crate) fn render(&self) -> String {
        let mut lines = vec![
            "<environment>".to_string(),
            format!("OS: {} ({}, {})", self.os, std::env::consts::OS, self.arch),
            format!("Shell: {}", self.shell),
            format!("Today's date: {} ({})", self.date, self.timezone),
            match &self.workspace {
                Some(path) => format!("Workspace root: {path}"),
                None => "Workspace root: none (no working folder is selected)".to_string(),
            },
        ];
        match (&self.workspace, &self.git) {
            (None, _) => {}
            (Some(_), None) => lines.push("Git repository: no".to_string()),
            (Some(workspace), Some(git)) => {
                let mut repo = String::from("Git repository: yes");
                if &git.root != workspace {
                    repo.push_str(&format!(" (root {})", git.root));
                }
                lines.push(repo);
                lines.push(format!(
                    "Current branch: {}",
                    git.branch.as_deref().unwrap_or("(detached HEAD)")
                ));
                if git.status_total == 0 {
                    lines.push("Git status: clean".to_string());
                } else {
                    let shown = if git.status.len() < git.status_total {
                        format!(", first {}", git.status.len())
                    } else {
                        String::new()
                    };
                    lines.push(format!(
                        "Git status ({} changed paths{shown}):",
                        git.status_total
                    ));
                    lines.extend(git.status.iter().map(|line| format!("  {line}")));
                }
                if !git.commits.is_empty() {
                    lines.push("Recent commits:".to_string());
                    lines.extend(git.commits.iter().map(|line| format!("  {line}")));
                }
            }
        }
        lines.push(format!("Model: {}", self.model));
        lines.push(
            "This snapshot was taken when the run started; re-check git state before relying on it."
                .to_string(),
        );
        lines.push("</environment>".to_string());
        lines.join("\n")
    }
}

fn os_label() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn timezone_name() -> Option<String> {
    if let Some(tz) = std::env::var("TZ")
        .ok()
        .map(|value| value.trim().trim_start_matches(':').to_string())
        .filter(|value| !value.is_empty() && !value.starts_with('/'))
    {
        return Some(tz);
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_string_lossy();
    let (_, name) = target.split_once("zoneinfo/")?;
    (!name.is_empty()).then(|| name.to_string())
}

fn git_snapshot(workspace: &Path) -> Option<GitSnapshot> {
    let root = git_bounded(workspace, &["rev-parse", "--show-toplevel"])?;
    let root = canonical(Path::new(root.trim())).display().to_string();
    let branch = git_bounded(workspace, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let status_text = git_bounded(
        workspace,
        &["status", "--porcelain", "--untracked-files=normal"],
    )
    .unwrap_or_default();
    let status: Vec<String> = status_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    let commits = git_bounded(
        workspace,
        &[
            "log",
            "--oneline",
            "--no-decorate",
            "-n",
            GIT_RECENT_COMMITS,
        ],
    )
    .unwrap_or_default()
    .lines()
    .map(str::to_string)
    .collect();
    Some(GitSnapshot {
        root,
        branch,
        status_total: status.len(),
        status: status.into_iter().take(GIT_STATUS_MAX_LINES).collect(),
        commits,
    })
}

/// Run a read-only git command with a hard timeout. Optional locks are
/// disabled so the snapshot never contends with the user's own git commands.
fn git_bounded(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = milim_core::proc::hide_console(&mut command).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut buffer).map(|_| buffer)
    });
    let deadline = Instant::now() + GIT_ENVIRONMENT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let output = reader.join().ok()?.ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&output).trim_end().to_string())
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
    fn environment_block_reports_git_state_and_is_stable() {
        let dir = std::env::temp_dir().join(format!("milim-env-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = canonical(&dir);
        let git_ok = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .is_ok_and(|output| output.status.success())
        };
        let plain = RunEnvironment::capture(Some(&dir), "model-a");
        assert!(plain.git.is_none());
        assert!(plain.render().contains("Git repository: no"));

        if git_ok(&["init", "-q", "-b", "main"])
            && git_ok(&["config", "user.email", "test@example.com"])
            && git_ok(&["config", "user.name", "Test"])
        {
            std::fs::write(dir.join("a.txt"), "a").unwrap();
            assert!(git_ok(&["add", "a.txt"]));
            assert!(git_ok(&["commit", "-q", "-m", "first commit"]));
            for index in 0..25 {
                std::fs::write(dir.join(format!("new-{index:02}.txt")), "x").unwrap();
            }
            let env = RunEnvironment::capture(Some(&dir), "model-a");
            let git = env.git.as_ref().expect("git snapshot");
            assert_eq!(git.branch.as_deref(), Some("main"));
            assert_eq!(git.status_total, 25);
            assert_eq!(git.status.len(), GIT_STATUS_MAX_LINES);
            assert!(git.commits[0].ends_with("first commit"));
            let rendered = env.render();
            assert!(rendered.contains(&format!("Workspace root: {}", dir.display())));
            assert!(rendered.contains("Current branch: main"));
            assert!(rendered.contains("Git status (25 changed paths, first 20):"));
            assert!(rendered.contains("Model: model-a"));
            assert_eq!(rendered, env.clone().render());
        }

        let none = RunEnvironment::capture(None, "model-b").render();
        assert!(none.contains("Workspace root: none"));
        assert!(!none.contains("Git repository"));
        let _ = std::fs::remove_dir_all(dir);
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
