//! Per-file and per-hunk staging for the Git side panel.
//!
//! Every operation runs inside the validated repository root, passes paths
//! only after `--` with literal pathspecs, and rebuilds hunk patches from a
//! fresh `git diff` so a stale or edited client hunk can never be applied.

use std::io::Write as _;
use std::process::Stdio;

use super::*;

pub(super) const STAGING_ACTIONS: &[&str] = &[
    "stage_file",
    "unstage_file",
    "discard_file",
    "stage_hunk",
    "unstage_hunk",
    "discard_hunk",
];

const STALE_HUNK_MESSAGE: &str =
    "This hunk changed since the diff was loaded. Refresh the diff and try again.";

/// Run one staging action against `root`. `force` confirms a destructive
/// discard; the UI sets it only after an explicit confirmation.
pub(super) fn workspace_git_staging_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    action: &str,
    path: Option<String>,
    hunk: Option<String>,
    force: bool,
) -> WorkspaceGitActionResponse {
    let paths = match staging_paths(path.as_deref().unwrap_or_default()) {
        Ok(paths) => paths,
        Err(message) => return workspace_git_action_message(action, "git", false, &message),
    };
    let state = match path_status(root, &paths.target) {
        Ok(state) => state,
        Err(message) => return workspace_git_action_message(action, "git status", false, &message),
    };
    let Some((x, y)) = state else {
        return workspace_git_action_message(
            action,
            "git status",
            false,
            "That file no longer has changes. Refresh Git status.",
        );
    };
    let untracked = x == '?' && y == '?';
    let conflicted = is_git_conflict_status(x, y);
    if action.starts_with("discard_") && !force {
        return workspace_git_action_message(
            action,
            "git",
            false,
            if untracked {
                "Confirm deleting the untracked file before discarding it."
            } else {
                "Confirm discarding changes before running this action."
            },
        );
    }

    match action {
        "stage_file" => {
            if !untracked && !conflicted && y == ' ' {
                return workspace_git_action_message(
                    action,
                    "git add -A -- <path>",
                    false,
                    "That file has no unstaged changes.",
                );
            }
            run_staging_git(
                root,
                action,
                &["add", "-A", "--", paths.target.as_str()],
                "File staged.",
            )
        }
        "unstage_file" => {
            if conflicted {
                return workspace_git_action_message(
                    action,
                    "git reset -q HEAD -- <path>",
                    false,
                    "Resolve conflicts before unstaging this file.",
                );
            }
            if untracked || x == ' ' {
                return workspace_git_action_message(
                    action,
                    "git reset -q HEAD -- <path>",
                    false,
                    "That file has no staged changes.",
                );
            }
            let mut args: Vec<&str> = if status.head.is_some() {
                vec!["reset", "-q", "HEAD", "--"]
            } else {
                vec!["rm", "--cached", "-r", "-q", "--"]
            };
            if let Some(source) = paths.source.as_deref() {
                args.push(source);
            }
            args.push(paths.target.as_str());
            run_staging_git(root, action, &args, "File unstaged.")
        }
        "discard_file" => {
            if conflicted {
                return workspace_git_action_message(
                    action,
                    "git checkout -- <path>",
                    false,
                    "Resolve conflicts before discarding this file.",
                );
            }
            if untracked {
                let mut args = vec!["clean", "-f", "-q"];
                if paths.target.ends_with('/') {
                    args.push("-d");
                }
                args.extend(["--", paths.target.as_str()]);
                return run_staging_git(root, action, &args, "Untracked file deleted.");
            }
            if y == ' ' {
                return workspace_git_action_message(
                    action,
                    "git checkout -- <path>",
                    false,
                    "That file has no unstaged changes to discard. Unstage it first.",
                );
            }
            run_staging_git(
                root,
                action,
                &["checkout", "-q", "--", paths.target.as_str()],
                "Changes discarded.",
            )
        }
        "stage_hunk" | "unstage_hunk" | "discard_hunk" => {
            if untracked {
                return workspace_git_action_message(
                    action,
                    "git apply",
                    false,
                    "Stage or discard an untracked file as a whole.",
                );
            }
            if conflicted {
                return workspace_git_action_message(
                    action,
                    "git apply",
                    false,
                    "Resolve conflicts before changing individual hunks.",
                );
            }
            staging_hunk_action(root, action, &paths.target, hunk.as_deref())
        }
        _ => workspace_git_action_message(action, "", false, "Unsupported Git action."),
    }
}

fn staging_hunk_action(
    root: &FsPath,
    action: &str,
    target: &str,
    hunk: Option<&str>,
) -> WorkspaceGitActionResponse {
    let hunk = hunk.unwrap_or_default();
    if !hunk.trim_start().starts_with("@@ -") {
        return workspace_git_action_message(action, "git apply", false, "Hunk text required.");
    }
    let (diff_args, apply_args, done): (&[&str], &[&str], &str) = match action {
        "stage_hunk" => (
            &["diff", "--no-ext-diff", "--patch", "--"],
            &["apply", "--cached", "--whitespace=nowarn", "-"],
            "Hunk staged.",
        ),
        "unstage_hunk" => (
            &["diff", "--cached", "--no-ext-diff", "--patch", "--"],
            &["apply", "--cached", "-R", "--whitespace=nowarn", "-"],
            "Hunk unstaged.",
        ),
        _ => (
            &["diff", "--no-ext-diff", "--patch", "--"],
            &["apply", "-R", "--whitespace=nowarn", "-"],
            "Hunk discarded.",
        ),
    };
    let diff = match git_output(root, diff_args) {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).to_string()
        }
        Ok(output) => {
            return workspace_git_action_message(
                action,
                &git_command_text(diff_args),
                false,
                &output_error_text(&output),
            )
        }
        Err(e) => {
            return workspace_git_action_message(action, &git_command_text(diff_args), false, &e)
        }
    };
    let patch = match hunk_patch(&diff, target, hunk) {
        Ok(patch) => patch,
        Err(message) => {
            return workspace_git_action_message(
                action,
                &git_command_text(apply_args),
                false,
                &message,
            )
        }
    };
    let command = git_command_text(apply_args);
    match git_output_with_stdin(root, apply_args, patch.as_bytes()) {
        Ok(output) if output.status.success() => workspace_git_combined_response(
            action,
            &command,
            true,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code(),
            done.to_string(),
        ),
        Ok(output) => workspace_git_combined_response(
            action,
            &command,
            false,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code(),
            output_error_text(&output),
        ),
        Err(e) => workspace_git_action_message(action, &command, false, &e),
    }
}

fn run_staging_git(
    root: &FsPath,
    action: &str,
    args: &[&str],
    done: &str,
) -> WorkspaceGitActionResponse {
    let command = git_command_text(args);
    let mut literal = Vec::with_capacity(args.len() + 1);
    literal.push("--literal-pathspecs");
    literal.extend_from_slice(args);
    match git_output(root, &literal) {
        Ok(output) => {
            let ok = output.status.success();
            workspace_git_combined_response(
                action,
                &command,
                ok,
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
                output.status.code(),
                if ok {
                    done.to_string()
                } else {
                    output_error_text(&output)
                },
            )
        }
        Err(e) => workspace_git_action_message(action, &command, false, &e),
    }
}

fn git_output_with_stdin(
    cwd: &FsPath,
    args: &[&str],
    input: &[u8],
) -> std::result::Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = milim_core::proc::hide_console(&mut cmd)
        .spawn()
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input)
            .map_err(|e| format!("Failed to write patch to git: {e}"))?;
    }
    child
        .wait_with_output()
        .map_err(|e| format!("Failed to run git: {e}"))
}

#[derive(Debug, PartialEq, Eq)]
struct StagingPaths {
    /// Original path of a staged rename (`old -> new` in porcelain output).
    source: Option<String>,
    target: String,
}

fn staging_paths(raw: &str) -> Result<StagingPaths, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("File path required.".to_string());
    }
    let (source, target) = match raw.split_once(" -> ") {
        Some((source, target)) => (Some(source), target),
        None => (None, raw),
    };
    Ok(StagingPaths {
        source: source
            .map(|source| safe_repo_path(&unquote_git_path(source)))
            .transpose()?,
        target: safe_repo_path(&unquote_git_path(target))?,
    })
}

/// Reject absolute paths, parent traversal, and `.git` internals so an
/// action can only address files inside the repository worktree.
fn safe_repo_path(path: &str) -> Result<String, String> {
    let normalized = if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_string()
    };
    if normalized.is_empty() || normalized.contains('\0') {
        return Err("File path required.".to_string());
    }
    let invalid = || Err("File path must stay inside the repository.".to_string());
    if normalized.starts_with('/') || FsPath::new(&normalized).is_absolute() {
        return invalid();
    }
    for component in FsPath::new(&normalized).components() {
        match component {
            std::path::Component::Normal(part) => {
                if part.to_string_lossy().eq_ignore_ascii_case(".git") {
                    return invalid();
                }
            }
            std::path::Component::CurDir => {}
            _ => return invalid(),
        }
    }
    Ok(normalized)
}

/// Decode Git's C-style quoted path form (`"caf\303\251.txt"`).
fn unquote_git_path(value: &str) -> String {
    let value = value.trim();
    if !(value.len() >= 2 && value.starts_with('"') && value.ends_with('"')) {
        return value.to_string();
    }
    let inner = &value.as_bytes()[1..value.len() - 1];
    let mut bytes = Vec::with_capacity(inner.len());
    let mut index = 0;
    while index < inner.len() {
        let byte = inner[index];
        if byte != b'\\' || index + 1 >= inner.len() {
            bytes.push(byte);
            index += 1;
            continue;
        }
        let next = inner[index + 1];
        let octal = inner
            .get(index + 1..index + 4)
            .filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)));
        if let Some(digits) = octal {
            let value = digits
                .iter()
                .fold(0u32, |acc, digit| acc * 8 + u32::from(digit - b'0'));
            bytes.push(value as u8);
            index += 4;
            continue;
        }
        bytes.push(match next {
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            other => other,
        });
        index += 2;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Index and worktree status letters for one changed path, if any.
fn path_status(root: &FsPath, path: &str) -> Result<Option<(char, char)>, String> {
    let output = git_output(
        root,
        &[
            "--literal-pathspecs",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=normal",
            "--",
            path,
        ],
    )?;
    if !output.status.success() {
        return Err(output_error_text(&output));
    }
    let mut entries = output.stdout.split(|byte| *byte == 0);
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let x = entry[0] as char;
        let y = entry[1] as char;
        if matches!(x, 'R' | 'C') {
            // Renames and copies carry their original path as the next field.
            entries.next();
        }
        let entry_path = String::from_utf8_lossy(&entry[3..]);
        if entry_path == path || entry_path.trim_end_matches('/') == path.trim_end_matches('/') {
            return Ok(Some((x, y)));
        }
    }
    Ok(None)
}

#[derive(Debug, Default)]
struct DiffFileSection<'a> {
    header: Vec<&'a str>,
    hunks: Vec<Vec<&'a str>>,
    paths: Vec<String>,
    rename_or_copy: bool,
    binary: bool,
}

fn diff_file_sections(diff: &str) -> Vec<DiffFileSection<'_>> {
    let mut sections: Vec<DiffFileSection<'_>> = Vec::new();
    for line in diff.split('\n') {
        if line.starts_with("diff --git ") {
            let mut section = DiffFileSection {
                header: vec![line],
                ..DiffFileSection::default()
            };
            let header = line.trim_end_matches('\r');
            if let Some(index) = header.rfind(" b/") {
                section.paths.push(unquote_git_path(&header[index + 3..]));
            } else if let Some(index) = header.rfind(" \"b/") {
                let quoted = format!("\"{}", &header[index + 4..]);
                section.paths.push(unquote_git_path(&quoted));
            }
            sections.push(section);
            continue;
        }
        let Some(section) = sections.last_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            section.hunks.push(vec![line]);
            continue;
        }
        if let Some(hunk) = section.hunks.last_mut() {
            hunk.push(line);
            continue;
        }
        section.header.push(line);
        let text = line.trim_end_matches('\r');
        for prefix in ["+++ ", "--- "] {
            if let Some(path) = text.strip_prefix(prefix) {
                let path = unquote_git_path(path);
                if let Some(path) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
                    section.paths.push(path.to_string());
                }
            }
        }
        for prefix in ["rename to ", "copy to "] {
            if let Some(path) = text.strip_prefix(prefix) {
                section.paths.push(unquote_git_path(path));
            }
        }
        if text.starts_with("rename from ") || text.starts_with("copy from ") {
            section.rename_or_copy = true;
        }
        if text.starts_with("Binary files ") || text == "GIT binary patch" {
            section.binary = true;
        }
    }
    sections
}

fn normalized_hunk_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut normalized: Vec<&str> = lines
        .into_iter()
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    while normalized.last().is_some_and(|line| line.is_empty()) {
        normalized.pop();
    }
    normalized
}

/// Build a single-hunk patch for `target` from a fresh diff, requiring the
/// client's hunk text to match the current hunk exactly.
fn hunk_patch(diff: &str, target: &str, hunk: &str) -> Result<String, String> {
    let requested = normalized_hunk_lines(hunk.split('\n'));
    let target_unquoted = unquote_git_path(target);
    let mut file_found = false;
    for section in diff_file_sections(diff) {
        if !section
            .paths
            .iter()
            .any(|path| path == target || *path == target_unquoted)
        {
            continue;
        }
        file_found = true;
        let Some(lines) = section
            .hunks
            .iter()
            .find(|lines| normalized_hunk_lines(lines.iter().copied()) == requested)
        else {
            continue;
        };
        if section.rename_or_copy {
            return Err("Stage or unstage a renamed file as a whole.".to_string());
        }
        if section.binary {
            return Err("Binary files can only be staged as a whole.".to_string());
        }
        let mut patch = section.header.join("\n");
        patch.push('\n');
        let body_len = normalized_hunk_lines(lines.iter().copied()).len();
        patch.push_str(&lines[..body_len].join("\n"));
        patch.push('\n');
        return Ok(patch);
    }
    Err(if file_found {
        STALE_HUNK_MESSAGE.to_string()
    } else {
        "That file is not in the current diff. Refresh the diff and try again.".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(repo: &FsPath, args: &[&str]) -> String {
        let output = git_output(repo, args).unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn temp_repo() -> PathBuf {
        let repo = std::env::temp_dir().join(format!("milim-staging-{}", gen_id("test")));
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Milim Test"]);
        git(&repo, &["config", "user.email", "milim@example.invalid"]);
        git(&repo, &["config", "core.autocrlf", "false"]);
        let lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
        std::fs::write(repo.join("notes.txt"), lines.join("\n") + "\n").unwrap();
        std::fs::write(repo.join("other.txt"), "keep\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "initial"]);
        repo
    }

    fn two_hunk_edit(repo: &FsPath) {
        let lines: Vec<String> = (1..=30)
            .map(|n| match n {
                2 => "line two edited".to_string(),
                28 => "line twenty-eight edited".to_string(),
                n => format!("line {n}"),
            })
            .collect();
        std::fs::write(repo.join("notes.txt"), lines.join("\n") + "\n").unwrap();
    }

    fn hunks_of(diff: &str, path: &str) -> Vec<String> {
        diff_file_sections(diff)
            .into_iter()
            .filter(|section| section.paths.iter().any(|candidate| candidate == path))
            .flat_map(|section| section.hunks.into_iter().map(|lines| lines.join("\n")))
            .collect()
    }

    fn action(
        repo: &FsPath,
        name: &str,
        path: &str,
        hunk: Option<String>,
        force: bool,
    ) -> WorkspaceGitActionResponse {
        let status = workspace_git_status_blocking(Some(repo.to_path_buf()));
        workspace_git_staging_action(repo, &status, name, Some(path.to_string()), hunk, force)
    }

    #[test]
    fn paths_reject_escape_and_git_internals() {
        assert!(staging_paths("../outside.txt").is_err());
        assert!(staging_paths("/etc/passwd").is_err());
        assert!(staging_paths("src/../../outside").is_err());
        assert!(staging_paths(".git/config").is_err());
        assert!(staging_paths("  ").is_err());
        assert_eq!(
            staging_paths("old name.txt -> \"caf\\303\\251.txt\"").unwrap(),
            StagingPaths {
                source: Some("old name.txt".into()),
                target: "café.txt".into(),
            }
        );
    }

    #[test]
    fn file_stage_unstage_and_discard_round_trip() {
        let repo = temp_repo();
        two_hunk_edit(&repo);
        std::fs::write(repo.join("fresh.txt"), "new\n").unwrap();

        let staged = action(&repo, "stage_file", "notes.txt", None, false);
        assert!(staged.ok, "{}", staged.message);
        assert_eq!(
            git(&repo, &["diff", "--cached", "--name-only"]).trim(),
            "notes.txt"
        );

        let unstaged = action(&repo, "unstage_file", "notes.txt", None, false);
        assert!(unstaged.ok, "{}", unstaged.message);
        assert!(git(&repo, &["diff", "--cached", "--name-only"])
            .trim()
            .is_empty());

        let unconfirmed = action(&repo, "discard_file", "notes.txt", None, false);
        assert!(!unconfirmed.ok);
        let discarded = action(&repo, "discard_file", "notes.txt", None, true);
        assert!(discarded.ok, "{}", discarded.message);
        assert!(git(&repo, &["diff", "--name-only"]).trim().is_empty());

        let blocked = action(&repo, "discard_file", "fresh.txt", None, false);
        assert!(!blocked.ok);
        assert!(blocked.message.contains("untracked"));
        assert!(repo.join("fresh.txt").exists());
        let deleted = action(&repo, "discard_file", "fresh.txt", None, true);
        assert!(deleted.ok, "{}", deleted.message);
        assert!(!repo.join("fresh.txt").exists());

        let outside = action(&repo, "stage_file", "../notes.txt", None, false);
        assert!(!outside.ok);
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn hunk_stage_unstage_and_discard_touch_only_the_selected_hunk() {
        let repo = temp_repo();
        two_hunk_edit(&repo);
        let unstaged = git(&repo, &["diff", "--no-ext-diff", "--patch", "--"]);
        let hunks = hunks_of(&unstaged, "notes.txt");
        assert_eq!(hunks.len(), 2);

        let staged = action(
            &repo,
            "stage_hunk",
            "notes.txt",
            Some(hunks[0].clone()),
            false,
        );
        assert!(staged.ok, "{}", staged.message);
        let cached = git(&repo, &["diff", "--cached"]);
        assert!(cached.contains("+line two edited"));
        assert!(!cached.contains("twenty-eight"));
        let remaining = git(&repo, &["diff"]);
        assert!(remaining.contains("+line twenty-eight edited"));
        assert!(!remaining.contains("line two edited"));

        // The same hunk text is now stale for the unstaged diff.
        let stale = action(
            &repo,
            "stage_hunk",
            "notes.txt",
            Some(hunks[0].clone()),
            false,
        );
        assert!(!stale.ok);
        assert_eq!(stale.message, STALE_HUNK_MESSAGE);

        let staged_diff = git(
            &repo,
            &["diff", "--cached", "--no-ext-diff", "--patch", "--"],
        );
        let staged_hunks = hunks_of(&staged_diff, "notes.txt");
        let unstage = action(
            &repo,
            "unstage_hunk",
            "notes.txt",
            Some(staged_hunks[0].clone()),
            false,
        );
        assert!(unstage.ok, "{}", unstage.message);
        assert!(git(&repo, &["diff", "--cached"]).trim().is_empty());

        let current = git(&repo, &["diff", "--no-ext-diff", "--patch", "--"]);
        let current_hunks = hunks_of(&current, "notes.txt");
        let second = current_hunks
            .iter()
            .find(|hunk| hunk.contains("twenty-eight"))
            .unwrap()
            .clone();
        let unconfirmed = action(
            &repo,
            "discard_hunk",
            "notes.txt",
            Some(second.clone()),
            false,
        );
        assert!(!unconfirmed.ok);
        let discarded = action(&repo, "discard_hunk", "notes.txt", Some(second), true);
        assert!(discarded.ok, "{}", discarded.message);
        let content = std::fs::read_to_string(repo.join("notes.txt")).unwrap();
        assert!(content.contains("line two edited"));
        assert!(content.contains("line 28\n"));
        assert!(!content.contains("twenty-eight"));
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn hunk_actions_reject_untracked_files_and_edited_hunks() {
        let repo = temp_repo();
        two_hunk_edit(&repo);
        std::fs::write(repo.join("fresh.txt"), "new\n").unwrap();
        let untracked = action(
            &repo,
            "stage_hunk",
            "fresh.txt",
            Some("@@ -0,0 +1 @@\n+new".into()),
            false,
        );
        assert!(!untracked.ok);

        let unstaged = git(&repo, &["diff", "--no-ext-diff", "--patch", "--"]);
        let forged = hunks_of(&unstaged, "notes.txt")[0].replace("line two edited", "injected");
        let rejected = action(&repo, "stage_hunk", "notes.txt", Some(forged), false);
        assert!(!rejected.ok);
        assert!(git(&repo, &["diff", "--cached"]).trim().is_empty());
        std::fs::remove_dir_all(repo).ok();
    }
}
