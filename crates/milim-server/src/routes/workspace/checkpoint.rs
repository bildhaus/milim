//! Workspace checkpoints: snapshot commits under `refs/milim/checkpoints/`.
//!
//! A checkpoint records every tracked and non-ignored untracked file in the
//! working tree without touching the user's index, HEAD, or branches. A
//! restore rewrites only the working tree: it first takes a safety
//! checkpoint (returned as `undo_checkpoint`), writes back files the
//! checkpoint differs on, and removes only files that were added after it.
//! Ignored files are never deleted and the staged index is left as it was.

use super::staging::git_output_with_stdin;
use super::*;

pub(super) const CHECKPOINT_REF_PREFIX: &str = "refs/milim/checkpoints/";
/// Pruning keeps at least this many of the newest checkpoints per repository.
const CHECKPOINT_KEEP_RECENT: usize = 200;
/// Pruning keeps every checkpoint younger than this.
const CHECKPOINT_KEEP_SECS: u64 = 30 * 24 * 60 * 60;
const GITLINK_MODE: &str = "160000";

/// A checkpoint the canonical turn path took before a run. Serializes to the
/// desktop `WorkspaceCheckpoint` shape.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnCheckpoint {
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) created_at: i64,
    pub(crate) folder: String,
    pub(crate) root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) head: Option<String>,
}

/// Why a turn ran without a checkpoint.
#[derive(Clone, Debug)]
pub(crate) struct TurnCheckpointSkip {
    pub(crate) message: String,
    /// The folder is not inside a Git worktree, which is expected rather than
    /// a failure.
    pub(crate) not_git: bool,
}

/// Checkpoint `folder` before a turn runs. Blocking; call it off the async
/// runtime.
pub(crate) fn turn_workspace_checkpoint(
    folder: &FsPath,
    label: &str,
) -> Result<TurnCheckpoint, TurnCheckpointSkip> {
    if !folder.is_dir() {
        return Err(TurnCheckpointSkip {
            message: "Selected working folder is unavailable.".to_string(),
            not_git: false,
        });
    }
    let Some(root) = git_text(folder, &["rev-parse", "--show-toplevel"]) else {
        return Err(TurnCheckpointSkip {
            message: "No Git repository found in the selected folder".to_string(),
            not_git: true,
        });
    };
    let root = PathBuf::from(root);
    let head = git_text(&root, &["rev-parse", "--short", "HEAD"]);
    let response = create_checkpoint(&root, head.clone(), Some(label.to_string()));
    match response.checkpoint {
        Some(reference) if response.ok => Ok(TurnCheckpoint {
            reference,
            created_at: now_unix().saturating_mul(1000) as i64,
            folder: folder.to_string_lossy().to_string(),
            root: root.to_string_lossy().to_string(),
            head,
        }),
        _ => Err(TurnCheckpointSkip {
            message: response.message,
            not_git: false,
        }),
    }
}

pub(crate) fn workspace_git_checkpoint_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    message: Option<String>,
) -> WorkspaceGitActionResponse {
    create_checkpoint(root, status.head.clone(), message)
}

fn git_index_path(root: &FsPath) -> Result<PathBuf, String> {
    let path = git_text(root, &["rev-parse", "--git-path", "index"])
        .map(PathBuf::from)
        .ok_or_else(|| "Failed to locate the Git index.".to_string())?;
    Ok(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

/// A throwaway index file next to the real one, so the user's index is never
/// read or written by checkpoint plumbing.
fn scratch_index_path(index: &FsPath, purpose: &str) -> PathBuf {
    index.with_file_name(format!("milim-{}.index", gen_id(purpose)))
}

/// Snapshot the working tree into a commit under `refs/milim/checkpoints/`.
fn create_checkpoint(
    root: &FsPath,
    head: Option<String>,
    message: Option<String>,
) -> WorkspaceGitActionResponse {
    let real_index = match git_index_path(root) {
        Ok(path) => path,
        Err(message) => {
            return workspace_git_action_message(
                "checkpoint",
                "git rev-parse --git-path index",
                false,
                &message,
            )
        }
    };
    let temp_index = scratch_index_path(&real_index, "checkpoint");
    // Seed the scratch index from the real one so tracked files that match an
    // ignore rule stay in the snapshot; `add -A` then records the working tree.
    if real_index.is_file() {
        if let Err(e) = std::fs::copy(&real_index, &temp_index) {
            return workspace_git_action_message(
                "checkpoint",
                "",
                false,
                &format!("Failed to prepare the checkpoint index: {e}"),
            );
        }
    }
    let index_env = [("GIT_INDEX_FILE", temp_index.to_string_lossy().to_string())];
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut commands = Vec::new();

    let mut tree = String::new();
    for args in [["add", "-A", "--"].as_slice(), ["write-tree"].as_slice()] {
        commands.push(format!(
            "GIT_INDEX_FILE={} {}",
            temp_index.display(),
            git_command_text(args)
        ));
        match git_output_with_env(root, args, &index_env) {
            Ok(output) if output.status.success() => {
                append_git_output(&mut stdout, &mut stderr, &output);
                tree = output_text(&output);
            }
            Ok(output) => {
                append_git_output(&mut stdout, &mut stderr, &output);
                let _ = std::fs::remove_file(&temp_index);
                return workspace_git_combined_response(
                    "checkpoint",
                    &commands.join(" && "),
                    false,
                    stdout,
                    stderr,
                    output.status.code(),
                    output_error_text(&output),
                );
            }
            Err(e) => {
                let _ = std::fs::remove_file(&temp_index);
                return workspace_git_action_message(
                    "checkpoint",
                    &commands.join(" && "),
                    false,
                    &e,
                );
            }
        }
    }
    let _ = std::fs::remove_file(&temp_index);

    let checkpoint_ref = format!("{CHECKPOINT_REF_PREFIX}{}", gen_id("turn"));
    let checkpoint_label = message
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("turn");
    let commit_message = format!("milim workspace checkpoint: {checkpoint_label}");
    let parent = git_text(root, &["rev-parse", "HEAD"]);
    let mut commit_args = vec!["commit-tree", tree.as_str()];
    if let Some(parent) = parent.as_deref() {
        commit_args.push("-p");
        commit_args.push(parent);
    }
    commit_args.push("-m");
    commit_args.push(commit_message.as_str());
    commands.push(git_command_text(&commit_args));
    let commit_env = [
        ("GIT_AUTHOR_NAME", "milim".to_string()),
        ("GIT_AUTHOR_EMAIL", "milim@example.invalid".to_string()),
        ("GIT_COMMITTER_NAME", "milim".to_string()),
        ("GIT_COMMITTER_EMAIL", "milim@example.invalid".to_string()),
    ];
    let commit = match git_output_with_env(root, &commit_args, &commit_env) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output);
            output_text(&output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e)
        }
    };

    let update_ref_args = ["update-ref", checkpoint_ref.as_str(), commit.as_str()];
    commands.push(git_command_text(&update_ref_args));
    match git_output(root, &update_ref_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e)
        }
    }
    prune_checkpoints(root, now_unix());

    let mut response = workspace_git_combined_response(
        "checkpoint",
        &commands.join(" && "),
        true,
        stdout,
        stderr,
        Some(0),
        "Workspace checkpoint created.".to_string(),
    );
    response.checkpoint = Some(checkpoint_ref);
    response.root = Some(root.to_string_lossy().to_string());
    response.head = head;
    response
}

/// Checkpoint refs that pruning would delete: those beyond the newest
/// `CHECKPOINT_KEEP_RECENT` that are also older than `CHECKPOINT_KEEP_SECS`.
/// `listing` is `for-each-ref` output of `<refname> <unix time>` lines,
/// newest first.
fn prunable_checkpoints(listing: &str, now_secs: u64) -> Vec<String> {
    let cutoff = now_secs.saturating_sub(CHECKPOINT_KEEP_SECS);
    listing
        .lines()
        .filter_map(|line| {
            let (name, created) = line.trim().rsplit_once(' ')?;
            Some((name.to_string(), created.parse::<u64>().ok()?))
        })
        .skip(CHECKPOINT_KEEP_RECENT)
        .filter(|(name, created)| name.starts_with(CHECKPOINT_REF_PREFIX) && *created < cutoff)
        .map(|(name, _)| name)
        .collect()
}

/// Best-effort pruning of old checkpoint refs. Failures leave refs in place.
fn prune_checkpoints(root: &FsPath, now_secs: u64) {
    let Some(listing) = git_text(
        root,
        &[
            "for-each-ref",
            "--sort=-creatordate",
            "--format=%(refname) %(creatordate:unix)",
            CHECKPOINT_REF_PREFIX,
        ],
    ) else {
        return;
    };
    let prunable = prunable_checkpoints(&listing, now_secs);
    if prunable.is_empty() {
        return;
    }
    let input = prunable
        .iter()
        .map(|name| format!("delete {name}\n"))
        .collect::<String>();
    let _ = git_output_with_stdin(root, &["update-ref", "--stdin"], &[], input.as_bytes());
}

pub(super) fn valid_milim_checkpoint(checkpoint: Option<String>) -> Option<String> {
    let checkpoint = checkpoint.unwrap_or_default().trim().to_string();
    let name = checkpoint.strip_prefix(CHECKPOINT_REF_PREFIX)?;
    (!name.is_empty()
        && !name.starts_with('-')
        && !name.contains("..")
        && !name.contains(['^', '~', ':', '@', '\\'])
        && !name.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then_some(checkpoint)
}

/// Paths that differ between two trees, from the target's point of view.
#[derive(Debug, Default, PartialEq, Eq)]
struct RestorePlan {
    /// Present in the current snapshot but not in the checkpoint.
    remove: Vec<String>,
    /// Missing or different in the current snapshot.
    write: Vec<String>,
}

/// Parse `git diff-tree -r -z --no-renames <checkpoint> <current>`.
/// Submodule entries are skipped: a restore never touches nested
/// repositories.
fn restore_plan(raw: &[u8]) -> RestorePlan {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0');
    let mut plan = RestorePlan::default();
    while let Some(meta) = fields.next() {
        let Some(meta) = meta.strip_prefix(':') else {
            continue;
        };
        let Some(path) = fields.next().filter(|path| !path.is_empty()) else {
            break;
        };
        let parts = meta.split_whitespace().collect::<Vec<_>>();
        let [old_mode, new_mode, _, _, status] = parts.as_slice() else {
            continue;
        };
        if *old_mode == GITLINK_MODE || *new_mode == GITLINK_MODE {
            continue;
        }
        match status.chars().next() {
            Some('A') => plan.remove.push(path.to_string()),
            Some('D' | 'M' | 'T') => plan.write.push(path.to_string()),
            _ => {}
        }
    }
    plan
}

/// Of `paths`, those an ignore rule matches, tracked or not.
fn ignored_paths(root: &FsPath, paths: &[String]) -> Result<HashSet<String>, String> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let input = paths
        .iter()
        .map(|path| format!("{path}\0"))
        .collect::<String>();
    let output = git_output_with_stdin(
        root,
        &["check-ignore", "--no-index", "-z", "--stdin"],
        &[],
        input.as_bytes(),
    )?;
    // Exit 1 means nothing matched.
    match output.status.code() {
        Some(0) | Some(1) => Ok(String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect()),
        _ => Err(output_error_text(&output)),
    }
}

/// Remove `path` (relative to `root`) and any parent folders it leaves
/// empty. Folders are never removed recursively.
fn remove_added_file(root: &FsPath, path: &str) -> Result<(), String> {
    let target = root.join(path);
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => std::fs::remove_file(&target).map_err(|e| format!("{path}: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{path}: {e}")),
    }
    let mut parent = target.parent();
    while let Some(dir) = parent {
        if dir == root || !dir.starts_with(root) || std::fs::remove_dir(dir).is_err() {
            break;
        }
        parent = dir.parent();
    }
    Ok(())
}

pub(super) fn workspace_git_restore_checkpoint_action(
    root: &FsPath,
    checkpoint: Option<String>,
) -> WorkspaceGitActionResponse {
    const ACTION: &str = "restore_checkpoint";
    let Some(checkpoint) = valid_milim_checkpoint(checkpoint) else {
        return workspace_git_action_message(
            ACTION,
            "git rev-parse --verify <checkpoint>",
            false,
            "A milim workspace checkpoint is required.",
        );
    };
    let treeish = format!("{checkpoint}^{{tree}}");
    let verify_args = ["rev-parse", "--verify", "--quiet", treeish.as_str()];
    let target = match git_output(root, &verify_args) {
        Ok(output) if output.status.success() => output_text(&output),
        Ok(_) => {
            return workspace_git_action_message(
                ACTION,
                &git_command_text(&verify_args),
                false,
                "That workspace checkpoint no longer exists.",
            )
        }
        Err(e) => {
            return workspace_git_action_message(ACTION, &git_command_text(&verify_args), false, &e)
        }
    };

    let head = git_text(root, &["rev-parse", "--short", "HEAD"]);
    let safety = create_checkpoint(root, head, Some("before-restore".to_string()));
    let Some(undo_ref) = safety.checkpoint.filter(|_| safety.ok) else {
        return workspace_git_action_message(
            ACTION,
            &safety.command,
            false,
            &format!(
                "Restore canceled: the current files could not be checkpointed first. {}",
                safety.message
            ),
        );
    };

    let mut commands = vec![safety.command.clone()];
    let diff_args = [
        "diff-tree",
        "-r",
        "-z",
        "--no-renames",
        target.as_str(),
        undo_ref.as_str(),
    ];
    commands.push(git_command_text(&diff_args));
    let plan = match git_output(root, &diff_args) {
        Ok(output) if output.status.success() => restore_plan(&output.stdout),
        Ok(output) => {
            return restore_failure(root, commands, &undo_ref, &output_error_text(&output))
        }
        Err(e) => return restore_failure(root, commands, &undo_ref, &e),
    };

    commands.push("git check-ignore --no-index -z --stdin".to_string());
    let ignored = match ignored_paths(root, &plan.remove) {
        Ok(ignored) => ignored,
        Err(e) => return restore_failure(root, commands, &undo_ref, &e),
    };
    let mut kept_ignored = 0;
    for path in &plan.remove {
        if ignored.contains(path) {
            kept_ignored += 1;
            continue;
        }
        if let Err(e) = remove_added_file(root, path) {
            return restore_failure(root, commands, &undo_ref, &e);
        }
    }

    let mut blocked = Vec::new();
    let mut write = Vec::new();
    for path in plan.write {
        let target_path = root.join(&path);
        let is_dir = std::fs::symlink_metadata(&target_path).is_ok_and(|meta| meta.is_dir());
        // A folder now stands where the checkpoint had a file. Only an empty
        // one is replaced, so nothing inside it is deleted.
        if is_dir && std::fs::remove_dir(&target_path).is_err() {
            blocked.push(path);
        } else {
            write.push(path);
        }
    }
    if !write.is_empty() {
        let temp_index = match git_index_path(root) {
            Ok(index) => scratch_index_path(&index, "restore"),
            Err(e) => return restore_failure(root, commands, &undo_ref, &e),
        };
        let index_env = [("GIT_INDEX_FILE", temp_index.to_string_lossy().to_string())];
        let read_tree_args = ["read-tree", target.as_str()];
        commands.push(format!(
            "GIT_INDEX_FILE={} {}",
            temp_index.display(),
            git_command_text(&read_tree_args)
        ));
        let read_tree = git_output_with_env(root, &read_tree_args, &index_env);
        if !matches!(&read_tree, Ok(output) if output.status.success()) {
            let _ = std::fs::remove_file(&temp_index);
            let detail = match read_tree {
                Ok(output) => output_error_text(&output),
                Err(e) => e,
            };
            return restore_failure(root, commands, &undo_ref, &detail);
        }
        let checkout_args = ["checkout-index", "-f", "-z", "--stdin"];
        commands.push(format!(
            "GIT_INDEX_FILE={} {}",
            temp_index.display(),
            git_command_text(&checkout_args)
        ));
        let input = write
            .iter()
            .map(|path| format!("{path}\0"))
            .collect::<String>();
        let checkout = git_output_with_stdin(root, &checkout_args, &index_env, input.as_bytes());
        let _ = std::fs::remove_file(&temp_index);
        match checkout {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                return restore_failure(root, commands, &undo_ref, &output_error_text(&output))
            }
            Err(e) => return restore_failure(root, commands, &undo_ref, &e),
        }
    }

    let mut message =
        "Workspace restored to checkpoint. Staged changes in the Git index were left as they were."
            .to_string();
    if kept_ignored > 0 {
        message.push_str(&format!(
            " Kept {kept_ignored} ignored file(s) added after the checkpoint."
        ));
    }
    if !blocked.is_empty() {
        message.push_str(&format!(
            " Skipped {} path(s) where a non-empty folder now exists.",
            blocked.len()
        ));
    }
    let mut response = workspace_git_combined_response(
        ACTION,
        &commands.join(" && "),
        true,
        String::new(),
        String::new(),
        Some(0),
        message,
    );
    response.checkpoint = Some(checkpoint);
    response.undo_checkpoint = Some(undo_ref);
    response.root = Some(root.to_string_lossy().to_string());
    response.head = git_text(root, &["rev-parse", "--short", "HEAD"]);
    if !blocked.is_empty() {
        response.conflicts = Some(blocked);
    }
    response
}

/// A restore that stopped part way. The safety checkpoint still holds the
/// files as they were, so the response offers it as the undo point.
fn restore_failure(
    root: &FsPath,
    commands: Vec<String>,
    undo_ref: &str,
    detail: &str,
) -> WorkspaceGitActionResponse {
    let mut response = workspace_git_action_message(
        "restore_checkpoint",
        &commands.join(" && "),
        false,
        &format!("Restore did not finish: {detail}"),
    );
    response.undo_checkpoint = Some(undo_ref.to_string());
    response.root = Some(root.to_string_lossy().to_string());
    response
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
        let repo = std::env::temp_dir().join(format!("milim-checkpoint-{}", gen_id("test")));
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Milim Test"]);
        git(&repo, &["config", "user.email", "milim@example.invalid"]);
        git(&repo, &["config", "core.autocrlf", "false"]);
        std::fs::write(repo.join(".gitignore"), "*.log\ntarget/\n").unwrap();
        std::fs::write(repo.join("tracked.txt"), "base\n").unwrap();
        std::fs::write(repo.join("staged.txt"), "base\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "initial"]);
        repo
    }

    fn read(repo: &FsPath, path: &str) -> String {
        std::fs::read_to_string(repo.join(path)).unwrap()
    }

    fn checkpoint(repo: &FsPath, label: &str) -> String {
        let response = create_checkpoint(repo, None, Some(label.to_string()));
        assert!(response.ok, "{}", response.message);
        response.checkpoint.unwrap()
    }

    #[test]
    fn restore_rejects_refs_outside_the_checkpoint_namespace() {
        let repo = temp_repo();
        for candidate in [
            "",
            "HEAD",
            "refs/heads/main",
            "refs/milim/checkpoints/",
            "refs/milim/checkpoints/../../heads/main",
            "refs/milim/checkpoints/turn^{tree}",
            "refs/milim/checkpoints/-turn",
        ] {
            let response =
                workspace_git_restore_checkpoint_action(&repo, Some(candidate.to_string()));
            assert!(!response.ok, "{candidate} was accepted");
            assert!(response.undo_checkpoint.is_none());
        }
        let missing = workspace_git_restore_checkpoint_action(
            &repo,
            Some("refs/milim/checkpoints/turn-missing".into()),
        );
        assert!(!missing.ok);
        assert!(missing.message.contains("no longer exists"));
        // A rejected restore takes no safety checkpoint.
        assert!(git(&repo, &["for-each-ref", CHECKPOINT_REF_PREFIX])
            .trim()
            .is_empty());
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn restore_round_trip_keeps_ignored_files_and_the_staged_index() {
        let repo = temp_repo();
        std::fs::write(repo.join("scratch.txt"), "user scratch\n").unwrap();
        std::fs::write(repo.join("before.log"), "old log\n").unwrap();
        std::fs::create_dir_all(repo.join("target")).unwrap();
        std::fs::write(repo.join("target/cache.bin"), "cache\n").unwrap();
        let turn = checkpoint(&repo, "turn-1");

        // The turn edits, deletes, and adds files, and the user stages a change.
        std::fs::write(repo.join("tracked.txt"), "changed by turn\n").unwrap();
        std::fs::remove_file(repo.join("scratch.txt")).unwrap();
        std::fs::create_dir_all(repo.join("src/nested")).unwrap();
        std::fs::write(repo.join("src/nested/new.rs"), "fn main() {}\n").unwrap();
        std::fs::write(repo.join("after.log"), "new log\n").unwrap();
        std::fs::write(repo.join("target/new.bin"), "build output\n").unwrap();
        std::fs::write(repo.join("staged.txt"), "staged by user\n").unwrap();
        git(&repo, &["add", "staged.txt"]);

        let restored = workspace_git_restore_checkpoint_action(&repo, Some(turn.clone()));
        assert!(restored.ok, "{}", restored.message);
        assert_eq!(restored.checkpoint.as_deref(), Some(turn.as_str()));
        let undo = restored.undo_checkpoint.clone().expect("safety checkpoint");
        assert!(undo.starts_with(CHECKPOINT_REF_PREFIX));
        assert_ne!(undo, turn);

        assert_eq!(read(&repo, "tracked.txt"), "base\n");
        assert_eq!(read(&repo, "scratch.txt"), "user scratch\n");
        assert_eq!(read(&repo, "staged.txt"), "base\n");
        assert!(!repo.join("src").exists(), "empty folders are removed");
        // Ignored files are never deleted, whether they predate the checkpoint
        // or were written after it.
        assert_eq!(read(&repo, "before.log"), "old log\n");
        assert_eq!(read(&repo, "after.log"), "new log\n");
        assert_eq!(read(&repo, "target/cache.bin"), "cache\n");
        assert_eq!(read(&repo, "target/new.bin"), "build output\n");
        // The staged blob stays in the index; only the working tree moved.
        assert_eq!(git(&repo, &["show", ":staged.txt"]), "staged by user\n");
        assert_eq!(
            git(&repo, &["diff", "--cached", "--name-only"]).trim(),
            "staged.txt"
        );

        // The safety checkpoint undoes the restore.
        let undone = workspace_git_restore_checkpoint_action(&repo, Some(undo));
        assert!(undone.ok, "{}", undone.message);
        assert_eq!(read(&repo, "tracked.txt"), "changed by turn\n");
        assert_eq!(read(&repo, "src/nested/new.rs"), "fn main() {}\n");
        assert_eq!(read(&repo, "staged.txt"), "staged by user\n");
        assert!(!repo.join("scratch.txt").exists());
        assert_eq!(read(&repo, "after.log"), "new log\n");
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn checkpoints_keep_tracked_files_that_match_an_ignore_rule() {
        let repo = temp_repo();
        std::fs::write(repo.join("pinned.log"), "tracked log\n").unwrap();
        git(&repo, &["add", "-f", "pinned.log"]);
        git(&repo, &["commit", "-q", "-m", "pin log"]);
        let turn = checkpoint(&repo, "turn-1");
        assert_eq!(
            git(&repo, &["show", &format!("{turn}:pinned.log")]),
            "tracked log\n"
        );
        std::fs::write(repo.join("pinned.log"), "edited\n").unwrap();
        let restored = workspace_git_restore_checkpoint_action(&repo, Some(turn));
        assert!(restored.ok, "{}", restored.message);
        assert_eq!(read(&repo, "pinned.log"), "tracked log\n");
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn turn_checkpoints_report_folders_outside_git() {
        let folder = std::env::temp_dir().join(format!("milim-no-git-{}", gen_id("test")));
        std::fs::create_dir_all(&folder).unwrap();
        let skipped = turn_workspace_checkpoint(&folder, "turn").unwrap_err();
        assert!(skipped.not_git);
        std::fs::remove_dir_all(&folder).ok();
        let missing = turn_workspace_checkpoint(&folder, "turn").unwrap_err();
        assert!(!missing.not_git);

        let repo = temp_repo();
        let created = turn_workspace_checkpoint(&repo.join("."), "turn").unwrap();
        assert!(created.reference.starts_with(CHECKPOINT_REF_PREFIX));
        let value = serde_json::to_value(&created).unwrap();
        assert_eq!(value["ref"], created.reference);
        assert!(value["createdAt"].as_i64().unwrap() > 0);
        std::fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn pruning_keeps_recent_and_young_checkpoints() {
        let now = 100 * 24 * 60 * 60;
        let old = now - CHECKPOINT_KEEP_SECS - 1;
        let young = now - 60;
        let listing = (0..CHECKPOINT_KEEP_RECENT + 3)
            .map(|index| {
                let created = if index == CHECKPOINT_KEEP_RECENT {
                    young
                } else {
                    old
                };
                format!("{CHECKPOINT_REF_PREFIX}turn-{index} {created}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let pruned = prunable_checkpoints(&listing, now);
        assert_eq!(
            pruned,
            vec![
                format!("{CHECKPOINT_REF_PREFIX}turn-{}", CHECKPOINT_KEEP_RECENT + 1),
                format!("{CHECKPOINT_REF_PREFIX}turn-{}", CHECKPOINT_KEEP_RECENT + 2),
            ]
        );
        assert!(prunable_checkpoints("", now).is_empty());
    }

    #[test]
    fn restore_plans_skip_submodules() {
        let raw = b":000000 100644 0000 aaaa A\0added.txt\0:100644 000000 bbbb 0000 D\0gone.txt\0:100644 100644 cccc dddd M\0edited.txt\0:160000 160000 eeee ffff M\0vendor/lib\0";
        assert_eq!(
            restore_plan(raw),
            RestorePlan {
                remove: vec!["added.txt".into()],
                write: vec!["gone.txt".into(), "edited.txt".into()],
            }
        );
    }
}
