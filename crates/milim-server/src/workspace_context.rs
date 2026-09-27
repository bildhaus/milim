use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Each instruction family (AGENTS files; Claude files and their imports)
/// loads at most this many bytes.
const INSTRUCTIONS_MAX_BYTES: usize = 32 * 1024;
/// How many `@path` hops a Claude instruction file may import through.
const MAX_IMPORT_DEPTH: usize = 4;
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
    let mut loader = InstructionLoader::new(&mut context);

    if let Some(home) = home_dir() {
        let codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        loader.add_first_agents(&codex, "global");
        // Files the user wrote for every project may import from anywhere in
        // their home folder.
        let user_roots = [canonical(&home)];
        loader.add_claude(
            home.join(".claude").join("CLAUDE.md"),
            "global",
            &user_roots,
        );
        loader.add_rules(&home.join(".claude").join("rules"), "global", &user_roots);
    }

    // Repository files may only import files inside the repository, so a
    // cloned project cannot pull the user's private files into the prompt.
    let project_roots = [canonical(root), folder.clone()];
    for dir in project_chain(root, &folder) {
        loader.add_first_agents(&dir, "project");
        for relative in ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"] {
            loader.add_claude(dir.join(relative), "project", &project_roots);
        }
        loader.add_rules(
            &dir.join(".claude").join("rules"),
            "project",
            &project_roots,
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

/// Instructions a native run carries besides its instruction files.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct InstructionLayers {
    /// App-wide Custom instructions from Milim settings.
    pub milim: String,
    /// The active Agent's instructions.
    pub agent: String,
    /// Instructions for this thread only.
    pub thread: String,
}

/// Every instruction source of a native run in one block, ordered from the
/// broadest to the most specific so that "the later one wins" follows how
/// widely each source applies: Milim's custom instructions, the user's own
/// instruction files, the Agent, the repository's files from its root down to
/// the working folder, and last the thread's instructions.
pub(crate) fn instruction_block(
    context: &WorkspaceContext,
    layers: &InstructionLayers,
) -> Option<String> {
    let mut sections = Vec::new();
    let mut push = |heading: String, content: &str| {
        let content = content.trim();
        if !content.is_empty() {
            sections.push(format!("## {heading}\n{content}"));
        }
    };
    let files = |scope: &'static str| {
        context
            .instructions
            .iter()
            .filter(move |item| item.status == "loaded" && item.scope == scope)
    };
    push("Custom instructions (all chats)".to_string(), &layers.milim);
    for item in files("global") {
        push(
            format!("User instructions from {}", item.path),
            &item.content,
        );
    }
    push("Agent instructions".to_string(), &layers.agent);
    for item in files("project") {
        push(
            format!("Repository instructions from {}", item.path),
            &item.content,
        );
    }
    push("Thread instructions".to_string(), &layers.thread);
    (!sections.is_empty()).then(|| {
        format!(
            "# Instructions\nFollow these instructions. They are ordered from broadest to most specific; when two conflict, the later one takes precedence.\n\n{}",
            sections.join("\n\n")
        )
    })
}

/// Machine and workspace facts a native agent run starts from, captured once
/// per run. The stable part leads the prompt and must not change between
/// turns, or every turn would rewrite the provider's prompt cache for the
/// whole conversation; the date and git state go with each turn instead.
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

    /// The facts that stay fixed for a thread: machine, shell, time zone,
    /// workspace, and model.
    pub(crate) fn render_stable(&self) -> String {
        let mut lines = vec![
            "<environment>".to_string(),
            format!("OS: {} ({}, {})", self.os, std::env::consts::OS, self.arch),
            format!("Shell: {}", self.shell),
            format!("Time zone: {}", self.timezone),
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
            }
        }
        lines.push(format!("Model: {}", self.model));
        lines.push(
            if self.git.is_some() {
                "Today's date and the current git branch, status, and recent commits come with each user message."
            } else {
                "Today's date comes with each user message."
            }
            .to_string(),
        );
        lines.push("</environment>".to_string());
        lines.join("\n")
    }

    /// The facts that change between turns: today's date and, in a git
    /// repository, the branch, status, and recent commits.
    pub(crate) fn render_turn(&self) -> String {
        let mut lines = vec![format!("Today's date: {}", self.date)];
        if let Some(git) = &self.git {
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

/// Collects instruction files in precedence order. Each file loads at most
/// once, and each family (AGENTS files; Claude files with their imports) is
/// held to its own 32 KiB budget.
struct InstructionLoader<'a> {
    context: &'a mut WorkspaceContext,
    seen: HashSet<PathBuf>,
    agents_bytes: usize,
    claude_bytes: usize,
}

impl<'a> InstructionLoader<'a> {
    fn new(context: &'a mut WorkspaceContext) -> Self {
        Self {
            context,
            seen: HashSet::new(),
            agents_bytes: 0,
            claude_bytes: 0,
        }
    }

    /// The first non-empty `AGENTS.override.md` or `AGENTS.md` in `dir`.
    fn add_first_agents(&mut self, dir: &Path, scope: &'static str) {
        for name in ["AGENTS.override.md", "AGENTS.md"] {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                self.context
                    .warnings
                    .push(format!("Could not read {}", path.display()));
                continue;
            };
            if content.trim().is_empty() {
                continue;
            }
            self.add_file(path, "agents", scope, false, None);
            return;
        }
    }

    /// A CLAUDE.md file, preceded by the files it imports with `@path`.
    /// Imports must resolve inside one of `roots`.
    fn add_claude(&mut self, path: PathBuf, scope: &'static str, roots: &[PathBuf]) {
        self.add_file(path, "claude", scope, false, Some((roots, 0)));
    }

    fn add_rules(&mut self, dir: &Path, scope: &'static str, roots: &[PathBuf]) {
        let mut files = Vec::new();
        collect_markdown(dir, &mut files, &mut self.context.warnings);
        files.sort();
        for path in files {
            self.add_file(path, "claude", scope, true, Some((roots, 0)));
        }
    }

    /// Load one file. With `imports`, the files it imports load first, up to
    /// [`MAX_IMPORT_DEPTH`] hops, so the importing file's own text follows
    /// them and takes precedence, as in Claude Code.
    fn add_file(
        &mut self,
        path: PathBuf,
        family: &'static str,
        scope: &'static str,
        rule: bool,
        imports: Option<(&[PathBuf], usize)>,
    ) {
        if !path.is_file() || !self.seen.insert(canonical(&path)) {
            return;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) if !content.trim().is_empty() => content,
            Ok(_) => return,
            Err(error) => {
                self.context
                    .warnings
                    .push(format!("Could not read {}: {error}", path.display()));
                return;
            }
        };
        let bytes = content.len();
        let mut status = "loaded";
        if rule && has_paths_frontmatter(&content) {
            status = "conditional";
            self.context.warnings.push(format!(
                "Skipped path-conditional Claude rule {} outside the Claude runtime",
                path.display()
            ));
        }
        if status == "loaded" {
            if let Some((roots, depth)) = imports.filter(|(_, depth)| *depth < MAX_IMPORT_DEPTH) {
                for target in import_targets(&content, &path) {
                    if !target.is_file() {
                        continue;
                    }
                    let resolved = canonical(&target);
                    if roots.iter().any(|root| resolved.starts_with(root)) {
                        self.add_file(target, family, scope, false, Some((roots, depth + 1)));
                    } else {
                        self.context.warnings.push(format!(
                            "Skipped import {} in {} because it is outside the folders instructions may import from",
                            target.display(),
                            path.display()
                        ));
                    }
                }
            }
            let (used, label) = if family == "agents" {
                (&mut self.agents_bytes, "AGENTS")
            } else {
                (&mut self.claude_bytes, "Claude")
            };
            if *used + bytes > INSTRUCTIONS_MAX_BYTES {
                status = "limit_exceeded";
                self.context.warnings.push(format!(
                    "Skipped {} because {label} instructions exceed 32 KiB",
                    path.display()
                ));
            } else {
                *used += bytes;
            }
        }
        self.context.instructions.push(WorkspaceInstruction {
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
}

/// The `@path` imports of a Claude instruction file, resolved against the
/// importing file's folder (`@~/` against the home folder). Markdown code
/// spans and fenced code blocks are skipped, so a quoted path stays literal.
fn import_targets(content: &str, importer: &Path) -> Vec<PathBuf> {
    let base = importer.parent().unwrap_or_else(|| Path::new("."));
    let mut targets = Vec::new();
    let mut fence: Option<&str> = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = ["```", "~~~"]
            .into_iter()
            .find(|marker| trimmed.starts_with(marker))
        {
            match fence {
                Some(open) if open == marker => fence = None,
                None => fence = Some(marker),
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        for word in without_code_spans(line).split_whitespace() {
            let Some(raw) = word.strip_prefix('@') else {
                continue;
            };
            let raw = raw.trim_end_matches(|c: char| {
                matches!(
                    c,
                    '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\''
                )
            });
            if raw.is_empty() {
                continue;
            }
            let target = match raw.strip_prefix("~/") {
                Some(rest) => match home_dir() {
                    Some(home) => home.join(rest),
                    None => continue,
                },
                None if Path::new(raw).is_absolute() => PathBuf::from(raw),
                None => base.join(raw),
            };
            targets.push(target);
        }
    }
    targets
}

/// `line` with its Markdown code spans blanked out. An unmatched backtick run
/// is literal text.
fn without_code_spans(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        out.push_str(&rest[..start]);
        let run = rest[start..].chars().take_while(|c| *c == '`').count();
        let delimiter = &rest[start..start + run];
        let after = &rest[start + run..];
        match after.find(delimiter) {
            Some(end) => {
                out.push(' ');
                rest = &after[end + run..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
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
    fn environment_keeps_git_state_out_of_the_stable_block() {
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
        assert!(plain.render_stable().contains("Git repository: no"));
        assert!(plain.render_turn().starts_with("Today's date: "));
        assert!(!plain.render_turn().contains("branch"));

        if git_ok(&["init", "-q", "-b", "main"])
            && git_ok(&["config", "user.email", "test@example.com"])
            && git_ok(&["config", "user.name", "Test"])
        {
            std::fs::write(dir.join("a.txt"), "a").unwrap();
            assert!(git_ok(&["add", "a.txt"]));
            assert!(git_ok(&["commit", "-q", "-m", "first commit"]));
            let clean = RunEnvironment::capture(Some(&dir), "model-a");
            for index in 0..25 {
                std::fs::write(dir.join(format!("new-{index:02}.txt")), "x").unwrap();
            }
            let env = RunEnvironment::capture(Some(&dir), "model-a");
            let git = env.git.as_ref().expect("git snapshot");
            assert_eq!(git.branch.as_deref(), Some("main"));
            assert_eq!(git.status_total, 25);
            assert_eq!(git.status.len(), GIT_STATUS_MAX_LINES);
            assert!(git.commits[0].ends_with("first commit"));
            let stable = env.render_stable();
            assert!(stable.contains(&format!("Workspace root: {}", dir.display())));
            assert!(stable.contains("Git repository: yes"));
            assert!(stable.contains("Model: model-a"));
            assert!(!stable.contains("Today's date:"), "{stable}");
            assert!(!stable.contains("Current branch"), "{stable}");
            assert_eq!(
                stable,
                clean.render_stable(),
                "git changes leave the stable block byte-identical"
            );
            let turn = env.render_turn();
            assert!(turn.contains("Current branch: main"));
            assert!(turn.contains("Git status (25 changed paths, first 20):"));
            assert!(turn.contains("first commit"));
            assert!(clean.render_turn().contains("Git status: clean"));
        }

        let none = RunEnvironment::capture(None, "model-b").render_stable();
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
        let mut loader = InstructionLoader::new(&mut context);
        loader.add_first_agents(&dir, "project");
        loader.add_rules(&dir.join("rules"), "project", &[canonical(&dir)]);

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
    fn agents_and_claude_files_each_load_at_most_32_kib() {
        let dir =
            std::env::temp_dir().join(format!("milim-context-limit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("rules")).unwrap();
        std::fs::write(
            dir.join("AGENTS.md"),
            "x".repeat(INSTRUCTIONS_MAX_BYTES + 1),
        )
        .unwrap();
        std::fs::write(
            dir.join("CLAUDE.md"),
            "c".repeat(INSTRUCTIONS_MAX_BYTES - 10),
        )
        .unwrap();
        std::fs::write(dir.join("rules").join("big.md"), "r".repeat(100)).unwrap();
        std::fs::write(dir.join("rules").join("small.md"), "tiny").unwrap();
        let mut context = empty_context();
        let mut loader = InstructionLoader::new(&mut context);
        let roots = [canonical(&dir)];
        loader.add_first_agents(&dir, "project");
        loader.add_claude(dir.join("CLAUDE.md"), "project", &roots);
        loader.add_rules(&dir.join("rules"), "project", &roots);
        assert_eq!(loader.agents_bytes, 0);
        assert_eq!(loader.claude_bytes, INSTRUCTIONS_MAX_BYTES - 6);
        let status = |name: &str| {
            let file = context
                .instructions
                .iter()
                .find(|file| file.path.ends_with(name))
                .unwrap();
            (file.status, file.content.len())
        };
        assert_eq!(status("AGENTS.md"), ("limit_exceeded", 0));
        assert_eq!(status("CLAUDE.md").0, "loaded");
        assert_eq!(status("big.md"), ("limit_exceeded", 0));
        assert_eq!(status("small.md"), ("loaded", 4));
        assert!(context
            .warnings
            .iter()
            .any(|warning| warning.contains("Claude instructions exceed 32 KiB")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_imports_load_first_within_roots_without_cycles() {
        let base =
            std::env::temp_dir().join(format!("milim-context-imports-{}", uuid::Uuid::new_v4()));
        let dir = base.join("repo");
        std::fs::create_dir_all(dir.join("docs").join("deep")).unwrap();
        std::fs::write(
            dir.join("CLAUDE.md"),
            "Project rules. See @docs/style.md.\n\
             Mention `@docs/quoted.md` literally.\n\
             ```\n@docs/fenced.md\n```\n\
             Escape @../secret.md and @missing.md, email a@docs/style.md.",
        )
        .unwrap();
        std::fs::write(dir.join("docs").join("style.md"), "Style. @deep/one.md").unwrap();
        std::fs::write(dir.join("docs").join("quoted.md"), "quoted").unwrap();
        std::fs::write(dir.join("docs").join("fenced.md"), "fenced").unwrap();
        // A chain past the four-hop limit that also imports its start again.
        std::fs::write(
            dir.join("docs").join("deep").join("one.md"),
            "one @two.md @../../CLAUDE.md",
        )
        .unwrap();
        std::fs::write(
            dir.join("docs").join("deep").join("two.md"),
            "two @three.md",
        )
        .unwrap();
        std::fs::write(
            dir.join("docs").join("deep").join("three.md"),
            "three @four.md",
        )
        .unwrap();
        std::fs::write(dir.join("docs").join("deep").join("four.md"), "four").unwrap();
        std::fs::write(base.join("secret.md"), "secret").unwrap();

        let mut context = empty_context();
        let mut loader = InstructionLoader::new(&mut context);
        loader.add_claude(dir.join("CLAUDE.md"), "project", &[canonical(&dir)]);

        let loaded: Vec<String> = context
            .instructions
            .iter()
            .map(|file| {
                assert_eq!(file.status, "loaded");
                assert_eq!(file.family, "claude");
                file.content.split_whitespace().next().unwrap().to_string()
            })
            .collect();
        // style.md is hop 1 and three.md hop 4, so four.md would be hop 5.
        assert_eq!(loaded, ["three", "two", "one", "Style.", "Project"]);
        assert_eq!(context.warnings.len(), 1, "{:?}", context.warnings);
        assert!(context.warnings[0].contains("secret.md"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn instruction_block_orders_sources_from_broadest_to_most_specific() {
        let file = |scope: &'static str, path: &str, content: &str| WorkspaceInstruction {
            family: "agents",
            scope,
            path: path.to_string(),
            content: content.to_string(),
            bytes: content.len(),
            status: "loaded",
        };
        let mut context = empty_context();
        context.instructions = vec![
            file("global", "/home/u/.codex/AGENTS.md", "USER_TEXT"),
            file("project", "/repo/AGENTS.md", "REPO_TEXT"),
            file("project", "/repo/app/AGENTS.md", "APP_TEXT"),
        ];
        let block = instruction_block(
            &context,
            &InstructionLayers {
                milim: "MILIM_TEXT".into(),
                agent: "AGENT_TEXT".into(),
                thread: "THREAD_TEXT".into(),
            },
        )
        .unwrap();
        let order: Vec<usize> = [
            "MILIM_TEXT",
            "USER_TEXT",
            "AGENT_TEXT",
            "REPO_TEXT",
            "APP_TEXT",
            "THREAD_TEXT",
        ]
        .iter()
        .map(|needle| block.find(needle).unwrap())
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{block}");
        assert!(block.starts_with("# Instructions\n"));
        assert!(block.contains("## Repository instructions from /repo/app/AGENTS.md\nAPP_TEXT"));
        assert!(instruction_block(&empty_context(), &InstructionLayers::default()).is_none());
    }
}
