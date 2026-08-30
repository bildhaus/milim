use super::*;

// ----- Workspace (host working folder for filesystem/shell tools) -----

#[derive(Deserialize)]
pub(crate) struct WorkspaceSet {
    #[serde(default)]
    folder: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitStatus {
    state: String,
    folder: Option<String>,
    pub(crate) is_repo: bool,
    pub(crate) root: Option<String>,
    branch: Option<String>,
    head: Option<String>,
    upstream: Option<String>,
    remote: Option<String>,
    ahead: u32,
    behind: u32,
    staged: u32,
    unstaged: u32,
    untracked: u32,
    conflicts: u32,
    insertions: u32,
    deletions: u32,
    has_changes: bool,
    changed_file_count: u32,
    changed_files: Vec<WorkspaceGitFileChange>,
    branches: Vec<WorkspaceGitBranch>,
    recent_commits: Vec<WorkspaceGitCommit>,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitFileChange {
    status: String,
    path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitBranch {
    name: String,
    current: bool,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitCommit {
    hash: String,
    subject: String,
}

#[derive(Deserialize)]
pub(crate) struct WorkspaceGitActionRequest {
    action: String,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    checkpoint: Option<String>,
    #[serde(default)]
    stage_all: bool,
    #[serde(default)]
    staged_only: bool,
    #[serde(default)]
    diff_scope: Option<String>,
    #[serde(default)]
    diff_base: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    worktree: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    pull_request: Option<u64>,
    #[serde(default)]
    review_action: Option<String>,
    #[serde(default)]
    merge_method: Option<String>,
    #[serde(default)]
    expected_head: Option<String>,
    #[serde(default)]
    repository: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitActionResponse {
    pub(crate) ok: bool,
    action: String,
    command: String,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    message: String,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worktree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    undo_checkpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflicts: Option<Vec<String>>,
    pull_request: Option<Value>,
}

#[derive(Debug, Default)]
struct GitChangeCounts {
    staged: u32,
    unstaged: u32,
    untracked: u32,
    conflicts: u32,
}

/// `GET /workspace` — the current host working folder (or null).
pub(crate) async fn workspace_get(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let cur = st
        .workspace
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|p| p.to_string_lossy().to_string());
    Ok(Json(json!({ "folder": cur })).into_response())
}

/// `POST /workspace` — set (or clear, with empty/null) the host working folder
/// that the filesystem/shell tools operate within.
pub(crate) async fn workspace_set(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<WorkspaceSet>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let folder = req
        .folder
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .map(|path| {
            let canonical = std::fs::canonicalize(&path).map_err(|error| {
                ApiError(Error::InvalidRequest(format!(
                    "invalid workspace {}: {error}",
                    path.display()
                )))
            })?;
            if !canonical.is_dir() {
                return Err(ApiError(Error::InvalidRequest(format!(
                    "workspace is not a directory: {}",
                    canonical.display()
                ))));
            }
            Ok(canonical)
        })
        .transpose()?;
    if let Ok(mut g) = st.workspace.write() {
        *g = folder.clone();
    }
    Ok(Json(json!({ "folder": folder.map(|p| p.to_string_lossy().to_string()) })).into_response())
}

/// `GET /preview-apps/{thread_id}` - status for a managed preview app.
pub(crate) async fn preview_app_get(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.status(&thread_id)?).into_response())
}

/// `POST /preview-apps/{thread_id}/stage` - stage no-folder named artifact files.
pub(crate) async fn preview_app_stage(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    Json(req): Json<PreviewAppStageRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.stage(&thread_id, &req.files)?).into_response())
}

/// `POST /preview-apps/{thread_id}/preflight` - inspect commands without staging or executing.
pub(crate) async fn preview_app_preflight(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    Json(req): Json<PreviewAppPreflightRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.preflight(&thread_id, &req)?).into_response())
}

/// `POST /preview-apps/{thread_id}/start` - install and start the preview app.
pub(crate) async fn preview_app_start(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    req: Option<Json<PreviewAppStartRequest>>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let request = req.map(|Json(req)| req).unwrap_or_default();
    Ok(Json(st.preview_runtime.start(&thread_id, &request)?).into_response())
}

/// `POST /preview-apps/{thread_id}/static` - serve a workspace HTML file without running commands.
pub(crate) async fn preview_app_static(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    Json(req): Json<PreviewStaticStartRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.start_static(&thread_id, &req).await?).into_response())
}

/// `POST /preview-apps/{thread_id}/stop` - stop the preview app process tree.
pub(crate) async fn preview_app_stop(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.stop(&thread_id).await?).into_response())
}

/// `POST /preview-apps/{thread_id}/restart` - stop, then start the preview app.
pub(crate) async fn preview_app_restart(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    req: Option<Json<PreviewAppStartRequest>>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let request = req.map(|Json(req)| req).unwrap_or_default();
    Ok(Json(st.preview_runtime.restart(&thread_id, &request).await?).into_response())
}

/// `GET /preview-apps/{thread_id}/logs` - recent preview app logs.
pub(crate) async fn preview_app_logs(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    Query(query): Query<PreviewAppLogsQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.logs_after(&thread_id, query.after_seq)?).into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PreviewAppLogsQuery {
    after_seq: Option<u64>,
}

/// `GET /workspace/git` - Git status for the current host working folder.
pub(crate) async fn workspace_git_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let folder = st.workspace.read().ok().and_then(|g| g.clone());
    let status = tokio::task::spawn_blocking(move || workspace_git_status_blocking(folder))
        .await
        .map_err(|e| ApiError(Error::Other(format!("git status task failed: {e}"))))?;
    Ok(Json(status).into_response())
}

/// `GET /workspace/context` - resolved repository instructions and stable identity.
pub(crate) async fn workspace_context(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let folder = workspace_snapshot(&st);
    let context =
        tokio::task::spawn_blocking(move || crate::workspace_context::resolve(folder.as_deref()))
            .await
            .map_err(|error| {
                ApiError(Error::Other(format!(
                    "workspace context task failed: {error}"
                )))
            })?;
    Ok(Json(context).into_response())
}

/// `POST /workspace/git/action` - run a narrow, guarded Git sidebar action.
pub(crate) async fn workspace_git_action(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(mut req): Json<WorkspaceGitActionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let action = req.action.trim().to_string();
    if !matches!(
        action.as_str(),
        "diff"
            | "fetch"
            | "pull"
            | "push"
            | "publish"
            | "commit"
            | "commit_push"
            | "checkout_branch"
            | "create_branch"
            | "checkpoint"
            | "restore_checkpoint"
            | "create_retry_worktree"
            | "apply_retry_worktree"
            | "remove_retry_worktree"
            | "create_thread_worktree"
            | "remove_thread_worktree"
            | "pr_list"
            | "pr_view"
            | "pr_status"
            | "pr_create"
            | "pr_ready"
            | "pr_comment"
            | "pr_review"
            | "pr_merge"
    ) {
        return Err(ApiError(Error::InvalidRequest(format!(
            "unsupported git action: {action}"
        ))));
    }

    let active_folder = st.workspace.read().ok().and_then(|g| g.clone());
    let folder = if action.starts_with("pr_") {
        req.folder
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or(active_folder)
    } else {
        if req
            .folder
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(ApiError(Error::InvalidRequest(
                "folder override is only supported for pull request actions".to_string(),
            )));
        }
        active_folder
    };
    let hot_swap_root = milim_core::paths::Paths::resolve()
        .root()
        .join("runtime")
        .join("hot-swap");
    let thread_root = milim_core::paths::Paths::resolve()
        .root()
        .join("runtime")
        .join("threads");
    req.action = action;
    let result = tokio::task::spawn_blocking(move || {
        workspace_git_action_blocking(folder, req, hot_swap_root, thread_root)
    })
    .await
    .map_err(|e| ApiError(Error::Other(format!("git action task failed: {e}"))))?;
    Ok(Json(result).into_response())
}

pub(crate) fn workspace_git_status_blocking(folder: Option<PathBuf>) -> WorkspaceGitStatus {
    let Some(folder) = folder else {
        return workspace_git_status_message(
            "no_folder",
            None,
            false,
            "No working folder selected",
        );
    };
    let folder_text = folder.to_string_lossy().to_string();

    match std::fs::metadata(&folder) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return workspace_git_status_message(
                "error",
                Some(folder_text),
                false,
                "Selected working folder is not a directory",
            )
        }
        Err(e) => {
            return workspace_git_status_message(
                "error",
                Some(folder_text),
                false,
                &format!("Failed to read working folder metadata: {e}"),
            )
        }
    }

    let inside = match git_output(&folder, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(output) if output.status.success() => output_text(&output),
        Ok(_) => {
            return workspace_git_status_message(
                "not_git",
                Some(folder_text),
                false,
                "No Git repository found in the selected folder",
            )
        }
        Err(e) => return workspace_git_status_message("error", Some(folder_text), false, &e),
    };

    if inside.trim() != "true" {
        return workspace_git_status_message(
            "not_git",
            Some(folder_text),
            false,
            "No Git worktree found in the selected folder",
        );
    }

    let root_text = git_text(&folder, &["rev-parse", "--show-toplevel"])
        .unwrap_or_else(|| folder.to_string_lossy().to_string());
    let root = PathBuf::from(&root_text);
    let branch = git_text(&root, &["branch", "--show-current"]);
    let head = git_text(&root, &["rev-parse", "--short", "HEAD"]);
    let upstream = git_text(
        &root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    let remote = git_text(&root, &["remote", "get-url", "origin"]);
    let branches = workspace_git_branches(&root, branch.as_deref());
    let recent_commits = workspace_git_recent_commits(&root);
    let (ahead, behind) = if upstream.is_some() {
        git_text(
            &root,
            &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
        )
        .map(|text| parse_ahead_behind(&text))
        .unwrap_or((0, 0))
    } else {
        (0, 0)
    };

    let porcelain = match git_output(
        &root,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=normal",
        ],
    ) {
        Ok(output) if output.status.success() => output_text(&output),
        Ok(output) => {
            return workspace_git_status_message(
                "error",
                Some(folder_text),
                true,
                &format!("Failed to read git status: {}", output_error_text(&output)),
            )
        }
        Err(e) => return workspace_git_status_message("error", Some(folder_text), true, &e),
    };

    let counts = parse_git_porcelain_counts(&porcelain);
    let (changed_file_count, changed_files) = parse_git_porcelain_files(&porcelain);
    let (insertions, deletions) = git_text(&root, &["diff", "--shortstat", "HEAD"])
        .map(|text| parse_git_shortstat(&text))
        .unwrap_or((0, 0));
    let has_changes = counts.staged + counts.unstaged + counts.untracked + counts.conflicts > 0;

    WorkspaceGitStatus {
        state: "ready".to_string(),
        folder: Some(folder_text),
        is_repo: true,
        root: Some(root_text),
        branch,
        head,
        upstream,
        remote,
        ahead,
        behind,
        staged: counts.staged,
        unstaged: counts.unstaged,
        untracked: counts.untracked,
        conflicts: counts.conflicts,
        insertions,
        deletions,
        has_changes,
        changed_file_count,
        changed_files,
        branches,
        recent_commits,
        message: None,
    }
}

fn workspace_git_status_message(
    state: &str,
    folder: Option<String>,
    is_repo: bool,
    message: &str,
) -> WorkspaceGitStatus {
    WorkspaceGitStatus {
        state: state.to_string(),
        folder,
        is_repo,
        root: None,
        branch: None,
        head: None,
        upstream: None,
        remote: None,
        ahead: 0,
        behind: 0,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        insertions: 0,
        deletions: 0,
        has_changes: false,
        changed_file_count: 0,
        changed_files: Vec::new(),
        branches: Vec::new(),
        recent_commits: Vec::new(),
        message: Some(message.to_string()),
    }
}

fn workspace_git_recent_commits(root: &FsPath) -> Vec<WorkspaceGitCommit> {
    let Ok(output) = git_output(root, &["log", "-n", "5", "--format=%h%x00%s"]) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (hash, subject) = line.split_once('\0')?;
            let hash = hash.trim();
            let subject = subject.trim();
            if hash.is_empty() || subject.is_empty() {
                return None;
            }
            Some(WorkspaceGitCommit {
                hash: hash.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

fn workspace_git_branches(root: &FsPath, current: Option<&str>) -> Vec<WorkspaceGitBranch> {
    let Ok(output) = git_output(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)%00%(upstream:short)",
            "refs/heads",
        ],
    ) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut branches: Vec<WorkspaceGitBranch> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let upstream = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let (ahead, behind) = upstream
                .as_deref()
                .and_then(|upstream| {
                    let range = format!("{name}...{upstream}");
                    git_text(
                        root,
                        &["rev-list", "--left-right", "--count", range.as_str()],
                    )
                })
                .map(|text| parse_ahead_behind(&text))
                .unwrap_or((0, 0));
            Some(WorkspaceGitBranch {
                name: name.to_string(),
                current: current == Some(name),
                upstream,
                ahead,
                behind,
            })
        })
        .collect();

    branches.sort_by(|a, b| b.current.cmp(&a.current).then_with(|| a.name.cmp(&b.name)));
    branches
}

fn git_output(cwd: &FsPath, args: &[&str]) -> std::result::Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    milim_core::proc::hide_console(&mut cmd)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))
}

fn git_output_with_env(
    cwd: &FsPath,
    args: &[&str],
    envs: &[(&str, String)],
) -> std::result::Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    milim_core::proc::hide_console(&mut cmd)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))
}

pub(crate) fn git_text(cwd: &FsPath, args: &[&str]) -> Option<String> {
    let output = git_output(cwd, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = output_text(&output);
    (!text.trim().is_empty()).then(|| text.trim().to_string())
}

fn output_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn output_error_text(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("git exited with {}", output.status)
    } else {
        stderr
    }
}

fn parse_ahead_behind(text: &str) -> (u32, u32) {
    let mut parts = text.split_whitespace();
    let ahead = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let behind = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (ahead, behind)
}

fn parse_git_porcelain_counts(text: &str) -> GitChangeCounts {
    let mut counts = GitChangeCounts::default();
    for line in text.lines() {
        if line.starts_with("##") || line.len() < 2 {
            continue;
        }
        let bytes = line.as_bytes();
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == '?' && y == '?' {
            counts.untracked += 1;
            continue;
        }
        if x == '!' && y == '!' {
            continue;
        }
        if is_git_conflict_status(x, y) {
            counts.conflicts += 1;
            continue;
        }
        if x != ' ' {
            counts.staged += 1;
        }
        if y != ' ' {
            counts.unstaged += 1;
        }
    }
    counts
}

fn is_git_conflict_status(x: char, y: char) -> bool {
    matches!(
        (x, y),
        ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
    )
}

fn parse_git_porcelain_files(text: &str) -> (u32, Vec<WorkspaceGitFileChange>) {
    let mut count = 0;
    let mut files = Vec::new();
    for line in text.lines() {
        if line.starts_with("##") || line.len() < 3 {
            continue;
        }
        let status = line[..2].trim().to_string();
        let path = line[3..].trim().to_string();
        if !status.is_empty() && !path.is_empty() {
            count += 1;
            files.push(WorkspaceGitFileChange { status, path });
        }
    }
    (count, files)
}

fn parse_git_shortstat(text: &str) -> (u32, u32) {
    let mut insertions = 0;
    let mut deletions = 0;
    for part in text.split(',').map(str::trim) {
        let mut words = part.split_whitespace();
        let Some(value) = words.next().and_then(|word| word.parse::<u32>().ok()) else {
            continue;
        };
        let Some(kind) = words.next() else {
            continue;
        };
        if kind.starts_with("insertion") {
            insertions = value;
        } else if kind.starts_with("deletion") {
            deletions = value;
        }
    }
    (insertions, deletions)
}

fn workspace_git_action_blocking(
    folder: Option<PathBuf>,
    request: WorkspaceGitActionRequest,
    hot_swap_root: PathBuf,
    thread_root: PathBuf,
) -> WorkspaceGitActionResponse {
    const OUTPUT_LIMIT: usize = 24_000;
    let WorkspaceGitActionRequest {
        action,
        folder: _,
        message,
        checkpoint,
        stage_all,
        staged_only,
        diff_scope,
        diff_base,
        branch,
        worktree,
        thread_id,
        force,
        title,
        body,
        base,
        draft,
        pull_request,
        review_action,
        merge_method,
        expected_head,
        repository,
    } = request;

    if action == "pr_list" {
        return workspace_git_pr_list_action();
    }
    if action == "pr_view" {
        return workspace_git_pr_view_action(repository, pull_request);
    }
    if matches!(
        action.as_str(),
        "pr_ready" | "pr_comment" | "pr_review" | "pr_merge"
    ) && repository.is_some()
    {
        return workspace_git_pr_mutation_action_global(
            repository,
            &action,
            pull_request,
            body,
            review_action,
            merge_method,
            expected_head,
        );
    }

    let status = workspace_git_status_blocking(folder);
    let Some(root) = status.root.as_ref().map(PathBuf::from) else {
        return workspace_git_action_message(
            &action,
            "",
            false,
            &git_state_label_for_action(&status),
        );
    };

    if action == "diff" {
        let scope = diff_scope
            .as_deref()
            .unwrap_or(if staged_only { "staged" } else { "all" });
        return workspace_git_diff_action(
            &root,
            &action,
            status.head.is_some(),
            scope,
            diff_base.as_deref(),
        );
    }
    if action == "checkpoint" {
        return workspace_git_checkpoint_action(&root, &status, message);
    }
    if action == "restore_checkpoint" {
        return workspace_git_restore_checkpoint_action(&root, checkpoint);
    }
    if action == "create_retry_worktree" {
        return workspace_git_create_retry_worktree_action(&root, checkpoint, &hot_swap_root);
    }
    if action == "apply_retry_worktree" {
        return workspace_git_apply_retry_worktree_action(
            &root,
            checkpoint,
            worktree,
            &hot_swap_root,
        );
    }
    if action == "remove_retry_worktree" {
        return workspace_git_remove_retry_worktree_action(&root, worktree, &hot_swap_root);
    }
    if action == "create_thread_worktree" {
        return workspace_git_create_thread_worktree_action(
            &root,
            &status,
            thread_id,
            &thread_root,
        );
    }
    if action == "remove_thread_worktree" {
        return workspace_git_remove_thread_worktree_action(&root, thread_id, force, &thread_root);
    }
    if action == "pr_status" {
        return workspace_git_pr_status_action(&root, &status);
    }
    if action == "pr_create" {
        return workspace_git_pr_create_action(&root, &status, title, body, base, draft);
    }
    if matches!(
        action.as_str(),
        "pr_ready" | "pr_comment" | "pr_review" | "pr_merge"
    ) {
        return workspace_git_pr_mutation_action(
            &root,
            &status,
            &action,
            pull_request,
            body,
            review_action,
            merge_method,
            expected_head,
        );
    }
    if matches!(action.as_str(), "commit" | "commit_push") {
        return workspace_git_commit_action(&root, &action, &status, message, stage_all);
    }

    let requested_branch = branch.unwrap_or_default().trim().to_string();
    let args: Vec<String> = match action.as_str() {
        "fetch" => vec!["fetch".into(), "--prune".into()],
        "checkout_branch" => {
            if requested_branch.is_empty() {
                return workspace_git_action_message(
                    &action,
                    "git checkout <branch>",
                    false,
                    "Branch name required.",
                );
            }
            vec!["checkout".into(), requested_branch.clone()]
        }
        "create_branch" => {
            if requested_branch.is_empty() {
                return workspace_git_action_message(
                    &action,
                    "git checkout -b <branch>",
                    false,
                    "Branch name required.",
                );
            }
            vec!["checkout".into(), "-b".into(), requested_branch.clone()]
        }
        "pull" => {
            if status.has_changes {
                return workspace_git_action_message(
                    &action,
                    "git pull --ff-only",
                    false,
                    "Pull requires a clean worktree.",
                );
            }
            if status.upstream.is_none() || status.behind == 0 {
                return workspace_git_action_message(
                    &action,
                    "git pull --ff-only",
                    false,
                    "Nothing to pull from an upstream branch.",
                );
            }
            vec!["pull".into(), "--ff-only".into()]
        }
        "push" => {
            if status.upstream.is_none() || status.ahead == 0 {
                return workspace_git_action_message(
                    &action,
                    "git push",
                    false,
                    "Nothing to push to an upstream branch.",
                );
            }
            vec!["push".into()]
        }
        "publish" => {
            let Some(branch) = status.branch.as_deref().filter(|s| !s.trim().is_empty()) else {
                return workspace_git_action_message(
                    &action,
                    "git push -u origin <branch>",
                    false,
                    "Publish requires a named branch.",
                );
            };
            if status.remote.is_none() || status.upstream.is_some() {
                return workspace_git_action_message(
                    &action,
                    &format!("git push -u origin {branch}"),
                    false,
                    "Publish requires a remote and no upstream.",
                );
            }
            vec![
                "push".into(),
                "-u".into(),
                "origin".into(),
                branch.to_string(),
            ]
        }
        _ => return workspace_git_action_message(&action, "", false, "Unsupported Git action."),
    };

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = format!("git {}", args.join(" "));
    match git_output(&root, &arg_refs) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let (stdout, stdout_truncated) = truncate_git_action_output(stdout, OUTPUT_LIMIT);
            let (stderr, stderr_truncated) = truncate_git_action_output(stderr, OUTPUT_LIMIT);
            let ok = output.status.success();
            let message = if ok {
                match action.as_str() {
                    "diff" if stdout.trim().is_empty() => "No diff to show.".to_string(),
                    "diff" => "Diff ready.".to_string(),
                    "fetch" => "Fetch complete.".to_string(),
                    "checkout_branch" => "Branch checked out.".to_string(),
                    "create_branch" => "Branch created.".to_string(),
                    "pull" => "Pull complete.".to_string(),
                    "push" => "Push complete.".to_string(),
                    "publish" => "Branch published.".to_string(),
                    _ => "Git action complete.".to_string(),
                }
            } else {
                output_error_text(&output)
            };
            WorkspaceGitActionResponse {
                ok,
                action,
                command,
                stdout,
                stderr,
                exit_code: output.status.code(),
                message,
                truncated: stdout_truncated || stderr_truncated,
                checkpoint: None,
                root: None,
                head: None,
                worktree: None,
                undo_checkpoint: None,
                conflicts: None,
                pull_request: None,
            }
        }
        Err(e) => workspace_git_action_message(&action, &command, false, &e),
    }
}

fn workspace_git_commit_action(
    root: &FsPath,
    action: &str,
    status: &WorkspaceGitStatus,
    message: Option<String>,
    stage_all: bool,
) -> WorkspaceGitActionResponse {
    let message = message.unwrap_or_default().trim().to_string();
    if message.is_empty() {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "Commit message required.",
        );
    }
    if !status.has_changes {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "Nothing to commit.",
        );
    }
    if status.conflicts > 0 {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "Resolve conflicts before committing.",
        );
    }
    if !stage_all && status.staged == 0 {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "No staged changes to commit.",
        );
    }

    let push_args: Option<Vec<String>> = if action == "commit_push" {
        if status.behind > 0 {
            return workspace_git_action_message(action, "git push", false, "Pull before pushing.");
        }
        if status.remote.is_none() {
            return workspace_git_action_message(
                action,
                "git push",
                false,
                "No remote configured.",
            );
        }
        if status.upstream.is_some() {
            Some(vec!["push".into()])
        } else {
            let Some(branch) = status.branch.as_deref().filter(|s| !s.trim().is_empty()) else {
                return workspace_git_action_message(
                    action,
                    "git push -u origin <branch>",
                    false,
                    "Publish requires a named branch.",
                );
            };
            Some(vec![
                "push".into(),
                "-u".into(),
                "origin".into(),
                branch.to_string(),
            ])
        }
    } else {
        None
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut commands = Vec::new();

    if stage_all {
        let args = ["add", "-A"];
        commands.push(git_command_text(&args));
        match git_output(root, &args) {
            Ok(output) if output.status.success() => {
                append_git_output(&mut stdout, &mut stderr, &output)
            }
            Ok(output) => {
                append_git_output(&mut stdout, &mut stderr, &output);
                return workspace_git_combined_response(
                    action,
                    &commands.join(" && "),
                    false,
                    stdout,
                    stderr,
                    output.status.code(),
                    output_error_text(&output),
                );
            }
            Err(e) => {
                return workspace_git_action_message(action, &commands.join(" && "), false, &e)
            }
        }
    }

    let commit_args = ["commit", "-m", message.as_str()];
    commands.push(git_command_text(&commit_args));
    match git_output(root, &commit_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                action,
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => return workspace_git_action_message(action, &commands.join(" && "), false, &e),
    }

    if let Some(args) = push_args {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        commands.push(git_command_text(&refs));
        match git_output(root, &refs) {
            Ok(output) if output.status.success() => {
                append_git_output(&mut stdout, &mut stderr, &output)
            }
            Ok(output) => {
                append_git_output(&mut stdout, &mut stderr, &output);
                return workspace_git_combined_response(
                    action,
                    &commands.join(" && "),
                    false,
                    stdout,
                    stderr,
                    output.status.code(),
                    output_error_text(&output),
                );
            }
            Err(e) => {
                return workspace_git_action_message(action, &commands.join(" && "), false, &e)
            }
        }
    }

    workspace_git_combined_response(
        action,
        &commands.join(" && "),
        true,
        stdout,
        stderr,
        Some(0),
        if action == "commit_push" {
            "Commit and push complete.".to_string()
        } else {
            "Commit complete.".to_string()
        },
    )
}

pub(crate) fn workspace_git_checkpoint_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    message: Option<String>,
) -> WorkspaceGitActionResponse {
    let Some(index_path) = git_text(root, &["rev-parse", "--git-path", "index"]) else {
        return workspace_git_action_message(
            "checkpoint",
            "git rev-parse --git-path index",
            false,
            "Failed to locate the Git index.",
        );
    };
    let index_path = {
        let path = PathBuf::from(index_path);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let temp_index = index_path.with_file_name(format!("milim-{}.index", gen_id("checkpoint")));
    let index_env = [("GIT_INDEX_FILE", temp_index.to_string_lossy().to_string())];
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut commands = Vec::new();

    let add_args = ["add", "-A", "--"];
    commands.push(format!(
        "GIT_INDEX_FILE={} {}",
        temp_index.display(),
        git_command_text(&add_args)
    ));
    match git_output_with_env(root, &add_args, &index_env) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
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
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e);
        }
    }

    let write_tree_args = ["write-tree"];
    commands.push(format!(
        "GIT_INDEX_FILE={} {}",
        temp_index.display(),
        git_command_text(&write_tree_args)
    ));
    let tree = match git_output_with_env(root, &write_tree_args, &index_env) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output);
            output_text(&output)
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
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e);
        }
    };
    let _ = std::fs::remove_file(&temp_index);

    let checkpoint_ref = format!("refs/milim/checkpoints/{}", gen_id("turn"));
    let checkpoint_label = message
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("turn");
    let commit_message = format!("milim workspace checkpoint: {checkpoint_label}");
    let head = git_text(root, &["rev-parse", "HEAD"]);
    let mut commit_args = vec!["commit-tree", tree.as_str()];
    if let Some(head) = head.as_deref() {
        commit_args.push("-p");
        commit_args.push(head);
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
    response.head = status.head.clone();
    response
}

fn workspace_git_restore_checkpoint_action(
    root: &FsPath,
    checkpoint: Option<String>,
) -> WorkspaceGitActionResponse {
    let checkpoint = checkpoint.unwrap_or_default().trim().to_string();
    if checkpoint.is_empty() {
        return workspace_git_action_message(
            "restore_checkpoint",
            "git read-tree --reset -u <checkpoint>",
            false,
            "Checkpoint ref required.",
        );
    }

    let treeish = format!("{checkpoint}^{{tree}}");
    let tree = match git_output(root, &["rev-parse", "--verify", treeish.as_str()]) {
        Ok(output) if output.status.success() => output_text(&output),
        Ok(output) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &format!("git rev-parse --verify {treeish}"),
                false,
                &output_error_text(&output),
            )
        }
        Err(e) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &format!("git rev-parse --verify {treeish}"),
                false,
                &e,
            )
        }
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    let read_tree_args = ["read-tree", "--reset", "-u", tree.as_str()];
    let clean_args = ["clean", "-fd"];
    let commands = [
        git_command_text(&read_tree_args),
        git_command_text(&clean_args),
    ];

    match git_output(root, &read_tree_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                &e,
            )
        }
    }
    match git_output(root, &clean_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                &e,
            )
        }
    }

    let mut response = workspace_git_combined_response(
        "restore_checkpoint",
        &commands.join(" && "),
        true,
        stdout,
        stderr,
        Some(0),
        "Workspace restored to checkpoint.".to_string(),
    );
    response.checkpoint = Some(checkpoint);
    response.root = Some(root.to_string_lossy().to_string());
    response.head = git_text(root, &["rev-parse", "--short", "HEAD"]);
    response
}

fn valid_milim_checkpoint(checkpoint: Option<String>) -> Option<String> {
    let checkpoint = checkpoint.unwrap_or_default().trim().to_string();
    if checkpoint.starts_with("refs/milim/checkpoints/") {
        Some(checkpoint)
    } else {
        None
    }
}

fn retry_worktree_path(
    worktree: Option<String>,
    hot_swap_root: &FsPath,
) -> Result<PathBuf, String> {
    let requested = PathBuf::from(worktree.unwrap_or_default());
    let root = std::fs::canonicalize(hot_swap_root)
        .map_err(|e| format!("Hot Swap runtime is unavailable: {e}"))?;
    let path = std::fs::canonicalize(&requested)
        .map_err(|e| format!("Retry worktree is unavailable: {e}"))?;
    if path.starts_with(&root) {
        Ok(path)
    } else {
        Err("Retry worktree must be inside Milim's runtime directory.".to_string())
    }
}

fn git_common_dir(root: &FsPath) -> Option<PathBuf> {
    let raw = PathBuf::from(git_text(root, &["rev-parse", "--git-common-dir"])?);
    std::fs::canonicalize(if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    })
    .ok()
}

pub(crate) fn workspace_git_create_retry_worktree_action(
    root: &FsPath,
    checkpoint: Option<String>,
    hot_swap_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let Some(checkpoint) = valid_milim_checkpoint(checkpoint) else {
        return workspace_git_action_message(
            "create_retry_worktree",
            "git rev-parse --verify <checkpoint>",
            false,
            "A Milim workspace checkpoint is required.",
        );
    };
    if let Err(e) = std::fs::create_dir_all(hot_swap_root) {
        return workspace_git_action_message(
            "create_retry_worktree",
            "",
            false,
            &format!("Failed to create Hot Swap runtime directory: {e}"),
        );
    }
    let worktree = hot_swap_root.join(gen_id("retry"));
    let worktree_text = worktree.to_string_lossy().to_string();
    let args = [
        "worktree",
        "add",
        "--detach",
        worktree_text.as_str(),
        checkpoint.as_str(),
    ];
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(e) => {
            return workspace_git_action_message(
                "create_retry_worktree",
                &git_command_text(&args),
                false,
                &e,
            )
        }
    };
    let ok = output.status.success();
    let mut response = workspace_git_combined_response(
        "create_retry_worktree",
        &git_command_text(&args),
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            "Retry worktree created.".to_string()
        } else {
            output_error_text(&output)
        },
    );
    if ok {
        response.checkpoint = Some(checkpoint);
        response.root = Some(root.to_string_lossy().to_string());
        response.worktree = Some(worktree_text);
    }
    response
}

pub(crate) fn workspace_git_apply_retry_worktree_action(
    root: &FsPath,
    checkpoint: Option<String>,
    worktree: Option<String>,
    hot_swap_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let Some(checkpoint) = valid_milim_checkpoint(checkpoint) else {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "git rev-parse --verify <checkpoint>",
            false,
            "A Milim workspace checkpoint is required.",
        );
    };
    if let Err(e) = std::fs::create_dir_all(hot_swap_root) {
        return workspace_git_action_message("apply_retry_worktree", "", false, &e.to_string());
    }
    let worktree = match retry_worktree_path(worktree, hot_swap_root) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("apply_retry_worktree", "", false, &message)
        }
    };
    if git_common_dir(root) != git_common_dir(&worktree) {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "",
            false,
            "Retry worktree does not belong to the selected repository.",
        );
    }

    let retry_status = workspace_git_status_blocking(Some(worktree.clone()));
    let retry_checkpoint = workspace_git_checkpoint_action(
        &worktree,
        &retry_status,
        Some("hot-swap-retry-result".to_string()),
    );
    let Some(retry_ref) = retry_checkpoint.checkpoint else {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "",
            false,
            &retry_checkpoint.message,
        );
    };

    let diff_args = [
        "diff",
        "--binary",
        checkpoint.as_str(),
        retry_ref.as_str(),
        "--",
    ];
    let diff = match git_output(&worktree, &diff_args) {
        Ok(output) if output.status.success() => output.stdout,
        Ok(output) => {
            return workspace_git_action_message(
                "apply_retry_worktree",
                &git_command_text(&diff_args),
                false,
                &output_error_text(&output),
            )
        }
        Err(e) => {
            return workspace_git_action_message(
                "apply_retry_worktree",
                &git_command_text(&diff_args),
                false,
                &e,
            )
        }
    };
    if diff.is_empty() {
        return workspace_git_action_message(
            "apply_retry_worktree",
            &git_command_text(&diff_args),
            true,
            "Retry workspace has no changes to apply.",
        );
    }
    let names = git_text(
        &worktree,
        &[
            "diff",
            "--name-only",
            checkpoint.as_str(),
            retry_ref.as_str(),
            "--",
        ],
    )
    .unwrap_or_default()
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>();
    let patch_path = hot_swap_root.join(format!("{}.patch", gen_id("apply")));
    if let Err(e) = std::fs::write(&patch_path, &diff) {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "",
            false,
            &format!("Failed to prepare retry patch: {e}"),
        );
    }
    let patch_text = patch_path.to_string_lossy().to_string();
    let check_args = ["apply", "--check", "--binary", patch_text.as_str()];
    let check = git_output(root, &check_args);
    if !matches!(&check, Ok(output) if output.status.success()) {
        let _ = std::fs::remove_file(&patch_path);
        let detail = match check {
            Ok(output) => output_error_text(&output),
            Err(e) => e,
        };
        let mut response = workspace_git_action_message(
            "apply_retry_worktree",
            &git_command_text(&check_args),
            false,
            &format!("Retry changes conflict with the original workspace: {detail}"),
        );
        response.conflicts = Some(names);
        return response;
    }

    let original_status = workspace_git_status_blocking(Some(root.to_path_buf()));
    let undo = workspace_git_checkpoint_action(
        root,
        &original_status,
        Some("before-hot-swap-apply".to_string()),
    );
    let Some(undo_ref) = undo.checkpoint else {
        let _ = std::fs::remove_file(&patch_path);
        return workspace_git_action_message("apply_retry_worktree", "", false, &undo.message);
    };
    let apply_args = ["apply", "--binary", patch_text.as_str()];
    let output = match git_output(root, &apply_args) {
        Ok(output) => output,
        Err(e) => {
            let _ = std::fs::remove_file(&patch_path);
            return workspace_git_action_message(
                "apply_retry_worktree",
                &git_command_text(&apply_args),
                false,
                &e,
            );
        }
    };
    let _ = std::fs::remove_file(&patch_path);
    if !output.status.success() {
        let restored = workspace_git_restore_checkpoint_action(root, Some(undo_ref));
        return workspace_git_action_message(
            "apply_retry_worktree",
            &git_command_text(&apply_args),
            false,
            &format!(
                "Retry apply failed and the original workspace was {}: {}",
                if restored.ok {
                    "restored"
                } else {
                    "not restored"
                },
                output_error_text(&output)
            ),
        );
    }
    let mut response = workspace_git_combined_response(
        "apply_retry_worktree",
        &git_command_text(&apply_args),
        true,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        "Retry changes applied to the original workspace.".to_string(),
    );
    response.root = Some(root.to_string_lossy().to_string());
    response.worktree = Some(worktree.to_string_lossy().to_string());
    response.undo_checkpoint = Some(undo_ref);
    response
}

fn workspace_git_remove_retry_worktree_action(
    root: &FsPath,
    worktree: Option<String>,
    hot_swap_root: &FsPath,
) -> WorkspaceGitActionResponse {
    if let Err(e) = std::fs::create_dir_all(hot_swap_root) {
        return workspace_git_action_message("remove_retry_worktree", "", false, &e.to_string());
    }
    let worktree = match retry_worktree_path(worktree, hot_swap_root) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("remove_retry_worktree", "", false, &message)
        }
    };
    if git_common_dir(root) != git_common_dir(&worktree) {
        return workspace_git_action_message(
            "remove_retry_worktree",
            "",
            false,
            "Retry worktree does not belong to the selected repository.",
        );
    }
    let worktree_text = worktree.to_string_lossy().to_string();
    let args = ["worktree", "remove", "--force", worktree_text.as_str()];
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(e) => {
            return workspace_git_action_message(
                "remove_retry_worktree",
                &git_command_text(&args),
                false,
                &e,
            )
        }
    };
    let ok = output.status.success();
    workspace_git_combined_response(
        "remove_retry_worktree",
        &git_command_text(&args),
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            "Retry worktree removed.".to_string()
        } else {
            output_error_text(&output)
        },
    )
}

fn valid_thread_runtime_id(thread_id: Option<String>) -> Result<String, String> {
    let value = thread_id.unwrap_or_default().trim().to_string();
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("A valid thread id is required.".to_string());
    }
    Ok(value)
}

fn workspace_git_create_thread_worktree_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    thread_id: Option<String>,
    thread_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let thread_id = match valid_thread_runtime_id(thread_id) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("create_thread_worktree", "", false, &message)
        }
    };
    if status.head.is_none() {
        return workspace_git_action_message(
            "create_thread_worktree",
            "git worktree add",
            false,
            "An initial commit is required before creating an isolated worktree.",
        );
    }
    if let Err(error) = std::fs::create_dir_all(thread_root) {
        return workspace_git_action_message(
            "create_thread_worktree",
            "",
            false,
            &format!("Failed to create the thread runtime directory: {error}"),
        );
    }
    let target = thread_root.join(&thread_id);
    if target.exists() {
        return workspace_git_action_message(
            "create_thread_worktree",
            "",
            false,
            "This thread already has a runtime worktree.",
        );
    }
    let short_id: String = thread_id.chars().take(8).collect();
    let branch = format!("milim/thread-{short_id}");
    let target_text = target.to_string_lossy().to_string();
    let args = [
        "worktree",
        "add",
        "-b",
        branch.as_str(),
        target_text.as_str(),
        "HEAD",
    ];
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(error) => {
            return workspace_git_action_message(
                "create_thread_worktree",
                &git_command_text(&args),
                false,
                &error,
            )
        }
    };
    let ok = output.status.success();
    let mut response = workspace_git_combined_response(
        "create_thread_worktree",
        &git_command_text(&args),
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            "Isolated thread worktree created. Uncommitted changes in the original checkout were not included.".to_string()
        } else {
            output_error_text(&output)
        },
    );
    if ok {
        response.root = Some(root.to_string_lossy().to_string());
        response.worktree = Some(target_text);
        response.head = git_text(root, &["rev-parse", "HEAD"]);
        response.stdout = branch;
    }
    response
}

fn workspace_git_remove_thread_worktree_action(
    root: &FsPath,
    thread_id: Option<String>,
    force: bool,
    thread_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let thread_id = match valid_thread_runtime_id(thread_id) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("remove_thread_worktree", "", false, &message)
        }
    };
    let runtime_root = match std::fs::canonicalize(thread_root) {
        Ok(path) => path,
        Err(error) => {
            return workspace_git_action_message(
                "remove_thread_worktree",
                "",
                false,
                &format!("Thread runtime is unavailable: {error}"),
            )
        }
    };
    let target = match std::fs::canonicalize(thread_root.join(thread_id)) {
        Ok(path) if path.starts_with(&runtime_root) => path,
        _ => {
            return workspace_git_action_message(
                "remove_thread_worktree",
                "",
                false,
                "Thread worktree is outside Milim's runtime directory or no longer exists.",
            )
        }
    };
    if git_common_dir(root) != git_common_dir(&target) {
        return workspace_git_action_message(
            "remove_thread_worktree",
            "",
            false,
            "Thread worktree does not belong to the selected repository.",
        );
    }
    if !force {
        let dirty = git_text(&target, &["status", "--porcelain"]).unwrap_or_default();
        if !dirty.trim().is_empty() {
            return workspace_git_action_message(
                "remove_thread_worktree", "git worktree remove", false,
                "The isolated worktree has uncommitted changes. Confirm force-discard to delete it; its branch will be retained.",
            );
        }
    }
    let target_text = target.to_string_lossy().to_string();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(target_text.as_str());
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(error) => {
            return workspace_git_action_message(
                "remove_thread_worktree",
                &git_command_text(&args),
                false,
                &error,
            )
        }
    };
    workspace_git_combined_response(
        "remove_thread_worktree",
        &git_command_text(&args),
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if output.status.success() {
            "Thread worktree removed. Its branch was retained.".to_string()
        } else {
            output_error_text(&output)
        },
    )
}

fn gh_output(root: &FsPath, args: &[&str]) -> Result<Output, String> {
    let mut command = gh_command();
    command.current_dir(root).args(args);
    milim_core::proc::hide_console(&mut command)
        .output()
        .map_err(|error| format!("Failed to run GitHub CLI: {error}"))
}

fn gh_output_owned(root: &FsPath, args: &[String]) -> Result<Output, String> {
    let mut command = gh_command();
    command.current_dir(root).args(args);
    milim_core::proc::hide_console(&mut command)
        .output()
        .map_err(|error| format!("Failed to run GitHub CLI: {error}"))
}

fn gh_output_global(args: &[String]) -> Result<Output, String> {
    let mut command = gh_command();
    command.args(args);
    milim_core::proc::hide_console(&mut command)
        .output()
        .map_err(|error| format!("Failed to run GitHub CLI: {error}"))
}

fn gh_command() -> Command {
    #[cfg(not(windows))]
    {
        crate::cli_path::blocking_command("gh")
    }
    #[cfg(windows)]
    {
        Command::new("gh")
    }
}

fn github_repository(value: Option<String>) -> Result<String, String> {
    let value = value.unwrap_or_default().trim().to_string();
    let mut parts = value.split('/');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    };
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(repository), None) if valid_part(owner) && valid_part(repository) => {
            Ok(value)
        }
        _ => Err("A GitHub repository in owner/name format is required.".to_string()),
    }
}

fn github_viewer_permission(output: Result<Output, String>) -> Option<String> {
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice::<Value>(&output.stdout)
        .ok()?
        .get("viewerPermission")?
        .as_str()
        .map(str::to_string)
}

fn workspace_git_pr_list_action() -> WorkspaceGitActionResponse {
    let fields = "number,title,url,state,isDraft,author,repository,updatedAt,commentsCount,body";
    let mut pull_requests = HashMap::<String, Value>::new();
    let mut errors = Vec::new();
    let mut successes = 0;

    for (flag, filter) in [
        ("authored", ["--author", "@me"]),
        ("reviewing", ["--review-requested", "@me"]),
    ] {
        let args = vec![
            "search".to_string(),
            "prs".to_string(),
            filter[0].to_string(),
            filter[1].to_string(),
            "--state".to_string(),
            "open".to_string(),
            "--sort".to_string(),
            "updated".to_string(),
            "--order".to_string(),
            "desc".to_string(),
            "--limit".to_string(),
            "100".to_string(),
            "--json".to_string(),
            fields.to_string(),
        ];
        let output = match gh_output_global(&args) {
            Ok(output) => output,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        if !output.status.success() {
            errors.push(output_error_text(&output));
            continue;
        }
        let items = match serde_json::from_slice::<Vec<Value>>(&output.stdout) {
            Ok(items) => items,
            Err(error) => {
                errors.push(format!("GitHub CLI returned invalid PR data: {error}"));
                continue;
            }
        };
        successes += 1;
        for mut item in items {
            let repository = item
                .get("repository")
                .and_then(|value| value.get("nameWithOwner"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let key = item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if key.is_empty() || repository.is_empty() {
                continue;
            }
            if let Some(existing) = pull_requests.get_mut(&key) {
                existing[flag] = Value::Bool(true);
                continue;
            }
            item["repository"] = Value::String(repository);
            item["authored"] = Value::Bool(flag == "authored");
            item["reviewing"] = Value::Bool(flag == "reviewing");
            pull_requests.insert(key, item);
        }
    }

    if successes == 0 {
        return workspace_git_action_message("pr_list", "gh search prs", false, &errors.join("\n"));
    }

    let mut pull_requests = pull_requests.into_values().collect::<Vec<_>>();
    pull_requests.sort_by(|left, right| {
        right
            .get("updatedAt")
            .and_then(Value::as_str)
            .cmp(&left.get("updatedAt").and_then(Value::as_str))
    });
    let mut response = workspace_git_combined_response(
        "pr_list",
        "gh search prs",
        true,
        Value::Array(pull_requests).to_string(),
        errors.join("\n"),
        Some(0),
        if errors.is_empty() {
            "Pull requests loaded.".to_string()
        } else {
            "Pull requests loaded with one unavailable filter.".to_string()
        },
    );
    response.truncated = false;
    response
}

fn workspace_git_pr_view_action(
    repository: Option<String>,
    pull_request: Option<u64>,
) -> WorkspaceGitActionResponse {
    let repository = match github_repository(repository) {
        Ok(repository) => repository,
        Err(message) => {
            return workspace_git_action_message("pr_view", "gh pr view", false, &message)
        }
    };
    let number = match pull_request.filter(|number| *number > 0) {
        Some(number) => number,
        None => {
            return workspace_git_action_message(
                "pr_view",
                "gh pr view",
                false,
                "Pull request number is required.",
            )
        }
    };
    let fields = "number,title,url,state,isDraft,baseRefName,headRefName,headRefOid,body,author,reviewRequests,latestReviews,reviewDecision,mergeStateStatus,mergeable,comments,additions,deletions,changedFiles,updatedAt,files";
    let args = vec![
        "pr".to_string(),
        "view".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repository.clone(),
        "--json".to_string(),
        fields.to_string(),
    ];
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_global(&args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message("pr_view", &command, false, &error),
    };
    if !output.status.success() {
        return workspace_git_combined_response(
            "pr_view",
            &command,
            false,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code(),
            output_error_text(&output),
        );
    }
    let mut pull_request = match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(pull_request) => pull_request,
        Err(error) => {
            return workspace_git_action_message(
                "pr_view",
                &command,
                false,
                &format!("GitHub CLI returned invalid PR data: {error}"),
            )
        }
    };
    let check_args = vec![
        "pr".to_string(),
        "checks".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repository.clone(),
        "--json".to_string(),
        "bucket,completedAt,description,event,link,name,startedAt,state,workflow".to_string(),
    ];
    match gh_output_global(&check_args) {
        Ok(checks) if checks.status.success() || checks.status.code() == Some(8) => {
            if let Ok(value) = serde_json::from_slice::<Value>(&checks.stdout) {
                pull_request["checks"] = value;
            }
        }
        Ok(checks) => {
            pull_request["checksError"] = Value::String(output_error_text(&checks));
        }
        Err(error) => {
            pull_request["checksError"] = Value::String(error);
        }
    }
    pull_request["exists"] = Value::Bool(true);
    pull_request["repository"] = Value::String(repository.clone());
    let permission_args = vec![
        "repo".to_string(),
        "view".to_string(),
        repository,
        "--json".to_string(),
        "viewerPermission".to_string(),
    ];
    if let Some(permission) = github_viewer_permission(gh_output_global(&permission_args)) {
        pull_request["viewerPermission"] = Value::String(permission);
    }
    let mut response = workspace_git_combined_response(
        "pr_view",
        &command,
        true,
        pull_request.to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        "Pull request found.".to_string(),
    );
    response.pull_request = Some(pull_request);
    response
}

fn workspace_git_pr_prerequisite(
    root: &FsPath,
    status: &WorkspaceGitStatus,
) -> Result<String, String> {
    let remote = git_text(root, &["remote", "get-url", "origin"])
        .ok_or_else(|| "A GitHub origin remote is required.".to_string())?;
    if !remote.to_ascii_lowercase().contains("github.com") {
        return Err("PR creation currently requires a GitHub origin remote.".to_string());
    }
    let branch = status
        .branch
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "A named branch is required.".to_string())?;
    if status.upstream.is_none() {
        return Err("Publish the branch before creating a PR.".to_string());
    }
    let auth = gh_output(root, &["auth", "status"])?;
    if !auth.status.success() {
        return Err("GitHub CLI is not authenticated. Run `gh auth login`.".to_string());
    }
    Ok(branch)
}

fn workspace_git_pr_status_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
) -> WorkspaceGitActionResponse {
    let branch = match workspace_git_pr_prerequisite(root, status) {
        Ok(branch) => branch,
        Err(message) => {
            return workspace_git_action_message(
                "pr_status",
                "gh pr list --head <branch>",
                false,
                &message,
            )
        }
    };
    let fields = "number,title,url,state,isDraft,baseRefName,headRefName,headRefOid,body,author,reviewRequests,latestReviews,reviewDecision,mergeStateStatus,mergeable,comments,additions,deletions,changedFiles,updatedAt";
    let args = vec![
        "pr".to_string(),
        "list".to_string(),
        "--head".to_string(),
        branch,
        "--state".to_string(),
        "all".to_string(),
        "--limit".to_string(),
        "10".to_string(),
        "--json".to_string(),
        fields.to_string(),
    ];
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_owned(root, &args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message("pr_status", &command, false, &error),
    };
    if !output.status.success() {
        return workspace_git_combined_response(
            "pr_status",
            &command,
            false,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code(),
            output_error_text(&output),
        );
    }

    let candidates = match serde_json::from_slice::<Vec<Value>>(&output.stdout) {
        Ok(candidates) => candidates,
        Err(error) => {
            return workspace_git_action_message(
                "pr_status",
                &command,
                false,
                &format!("GitHub CLI returned invalid PR data: {error}"),
            )
        }
    };
    let Some(mut pull_request) = select_pull_request_for_head(candidates, status.head.as_deref())
    else {
        let base = git_text(
            root,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .and_then(|value| value.strip_prefix("origin/").map(str::to_string))
        .unwrap_or_else(|| "main".to_string());
        let title = git_text(root, &["log", "-1", "--pretty=%s"]).unwrap_or_default();
        let mut response = workspace_git_combined_response(
            "pr_status",
            &command,
            true,
            json!({ "exists": false, "baseRefName": base, "title": title }).to_string(),
            String::new(),
            output.status.code(),
            "No pull request exists for this branch.".to_string(),
        );
        response.pull_request = None;
        return response;
    };

    if let Some(number) = pull_request.get("number").and_then(Value::as_u64) {
        let check_args = vec![
            "pr".to_string(),
            "checks".to_string(),
            number.to_string(),
            "--json".to_string(),
            "bucket,completedAt,description,event,link,name,startedAt,state,workflow".to_string(),
        ];
        match gh_output_owned(root, &check_args) {
            Ok(checks) if checks.status.success() || checks.status.code() == Some(8) => {
                if let Ok(value) = serde_json::from_slice::<Value>(&checks.stdout) {
                    pull_request["checks"] = value;
                }
            }
            Ok(checks) => {
                pull_request["checksError"] = Value::String(output_error_text(&checks));
            }
            Err(error) => {
                pull_request["checksError"] = Value::String(error);
            }
        }
    }
    pull_request["exists"] = Value::Bool(true);
    if let Some(permission) = github_viewer_permission(gh_output(
        root,
        &["repo", "view", "--json", "viewerPermission"],
    )) {
        pull_request["viewerPermission"] = Value::String(permission);
    }
    let mut response = workspace_git_combined_response(
        "pr_status",
        &command,
        true,
        pull_request.to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        "Pull request found.".to_string(),
    );
    response.pull_request = Some(pull_request);
    response
}

fn select_pull_request_for_head(candidates: Vec<Value>, head: Option<&str>) -> Option<Value> {
    let is_open = |candidate: &&Value| {
        candidate
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|state| state.eq_ignore_ascii_case("open"))
    };
    let matches_head = |candidate: &&Value| {
        head.is_some_and(|head| {
            candidate
                .get("headRefOid")
                .and_then(Value::as_str)
                .is_some_and(|candidate_head| candidate_head == head)
        })
    };
    candidates
        .iter()
        .find(|candidate| is_open(candidate) && matches_head(candidate))
        .or_else(|| candidates.iter().find(is_open))
        .or_else(|| candidates.iter().find(matches_head))
        .or_else(|| candidates.first())
        .cloned()
}

fn workspace_git_pr_action_args(
    action: &str,
    pull_request: Option<u64>,
    body: Option<&str>,
    review_action: Option<&str>,
    merge_method: Option<&str>,
    expected_head: Option<&str>,
) -> Result<Vec<String>, String> {
    let number = pull_request
        .filter(|number| *number > 0)
        .ok_or_else(|| "Pull request number is required.".to_string())?
        .to_string();
    let mut args = vec!["pr".to_string()];
    match action {
        "pr_ready" => args.extend(["ready".to_string(), number]),
        "pr_comment" => {
            let body = body
                .map(str::trim)
                .filter(|body| !body.is_empty())
                .ok_or_else(|| "Comment body is required.".to_string())?;
            args.extend([
                "comment".to_string(),
                number,
                "--body".to_string(),
                body.to_string(),
            ]);
        }
        "pr_review" => {
            let review_action = review_action
                .map(str::trim)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let flag = match review_action.as_str() {
                "approve" => "--approve",
                "request_changes" => "--request-changes",
                "comment" => "--comment",
                _ => return Err("Choose approve, request changes, or comment.".to_string()),
            };
            let body = body.map(str::trim).unwrap_or_default();
            if review_action != "approve" && body.is_empty() {
                return Err("Review body is required.".to_string());
            }
            args.extend(["review".to_string(), number, flag.to_string()]);
            if !body.is_empty() {
                args.extend(["--body".to_string(), body.to_string()]);
            }
        }
        "pr_merge" => {
            let method = match merge_method.map(str::trim) {
                Some("merge") => "--merge",
                Some("squash") => "--squash",
                Some("rebase") => "--rebase",
                _ => return Err("Choose merge, squash, or rebase.".to_string()),
            };
            let expected_head = expected_head
                .map(str::trim)
                .filter(|head| !head.is_empty())
                .ok_or_else(|| "Current pull request head is required.".to_string())?;
            args.extend([
                "merge".to_string(),
                number,
                method.to_string(),
                "--match-head-commit".to_string(),
                expected_head.to_string(),
            ]);
        }
        _ => return Err("Unsupported pull request action.".to_string()),
    }
    Ok(args)
}

#[allow(clippy::too_many_arguments)]
fn workspace_git_pr_mutation_action_global(
    repository: Option<String>,
    action: &str,
    pull_request: Option<u64>,
    body: Option<String>,
    review_action: Option<String>,
    merge_method: Option<String>,
    expected_head: Option<String>,
) -> WorkspaceGitActionResponse {
    let repository = match github_repository(repository) {
        Ok(repository) => repository,
        Err(message) => return workspace_git_action_message(action, "gh pr", false, &message),
    };
    let mut args = match workspace_git_pr_action_args(
        action,
        pull_request,
        body.as_deref(),
        review_action.as_deref(),
        merge_method.as_deref(),
        expected_head.as_deref(),
    ) {
        Ok(args) => args,
        Err(message) => return workspace_git_action_message(action, "gh pr", false, &message),
    };
    args.extend(["--repo".to_string(), repository]);
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_global(&args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message(action, &command, false, &error),
    };
    let ok = output.status.success();
    workspace_git_combined_response(
        action,
        &command,
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            match action {
                "pr_ready" => "Pull request marked ready for review.",
                "pr_comment" => "Comment added.",
                "pr_review" => "Review submitted.",
                "pr_merge" => "Pull request merged.",
                _ => "Pull request updated.",
            }
            .to_string()
        } else {
            output_error_text(&output)
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn workspace_git_pr_mutation_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    action: &str,
    pull_request: Option<u64>,
    body: Option<String>,
    review_action: Option<String>,
    merge_method: Option<String>,
    expected_head: Option<String>,
) -> WorkspaceGitActionResponse {
    if let Err(message) = workspace_git_pr_prerequisite(root, status) {
        return workspace_git_action_message(action, "gh pr", false, &message);
    }
    let args = match workspace_git_pr_action_args(
        action,
        pull_request,
        body.as_deref(),
        review_action.as_deref(),
        merge_method.as_deref(),
        expected_head.as_deref(),
    ) {
        Ok(args) => args,
        Err(message) => return workspace_git_action_message(action, "gh pr", false, &message),
    };
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_owned(root, &args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message(action, &command, false, &error),
    };
    let ok = output.status.success();
    workspace_git_combined_response(
        action,
        &command,
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            match action {
                "pr_ready" => "Pull request marked ready for review.",
                "pr_comment" => "Comment added.",
                "pr_review" => "Review submitted.",
                "pr_merge" => "Pull request merged.",
                _ => "Pull request updated.",
            }
            .to_string()
        } else {
            output_error_text(&output)
        },
    )
}

fn workspace_git_pr_create_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    title: Option<String>,
    body: Option<String>,
    base: Option<String>,
    draft: bool,
) -> WorkspaceGitActionResponse {
    let branch = match workspace_git_pr_prerequisite(root, status) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("pr_create", "gh pr create", false, &message)
        }
    };
    let title = title.unwrap_or_default().trim().to_string();
    let body = body.unwrap_or_default();
    let base = base.unwrap_or_default().trim().to_string();
    let base = if base.is_empty() {
        git_text(
            root,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .and_then(|value| value.strip_prefix("origin/").map(str::to_string))
        .unwrap_or_else(|| "main".to_string())
    } else {
        base
    };
    if title.is_empty() {
        return workspace_git_action_message(
            "pr_create",
            "gh pr create",
            false,
            "PR title is required.",
        );
    }
    if branch == base {
        return workspace_git_action_message(
            "pr_create",
            "gh pr create",
            false,
            "Create the PR from a non-default branch.",
        );
    }
    let ahead = git_text(
        root,
        &["rev-list", "--count", &format!("origin/{base}..HEAD")],
    )
    .and_then(|value| value.parse::<u64>().ok())
    .unwrap_or(0);
    if ahead == 0 {
        return workspace_git_action_message(
            "pr_create",
            "gh pr create",
            false,
            "Commit changes on this branch before creating a PR.",
        );
    }
    let mut args = vec![
        "pr",
        "create",
        "--title",
        title.as_str(),
        "--body",
        body.as_str(),
        "--base",
        base.as_str(),
        "--head",
        branch.as_str(),
    ];
    if draft {
        args.push("--draft");
    }
    let output = match gh_output(root, &args) {
        Ok(output) => output,
        Err(error) => {
            return workspace_git_action_message("pr_create", "gh pr create", false, &error)
        }
    };
    workspace_git_combined_response(
        "pr_create",
        "gh pr create",
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if output.status.success() {
            "Draft pull request created.".to_string()
        } else {
            output_error_text(&output)
        },
    )
}

fn append_git_output(stdout: &mut String, stderr: &mut String, output: &Output) {
    stdout.push_str(&String::from_utf8_lossy(&output.stdout));
    stderr.push_str(&String::from_utf8_lossy(&output.stderr));
}

fn git_command_text(args: &[&str]) -> String {
    let rendered = args
        .iter()
        .map(|arg| {
            if arg.chars().any(char::is_whitespace) {
                format!("{arg:?}")
            } else {
                (*arg).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {rendered}")
}

fn workspace_git_combined_response(
    action: &str,
    command: &str,
    ok: bool,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    message: String,
) -> WorkspaceGitActionResponse {
    const OUTPUT_LIMIT: usize = 24_000;
    let (stdout, stdout_truncated) = truncate_git_action_output(stdout, OUTPUT_LIMIT);
    let (stderr, stderr_truncated) = truncate_git_action_output(stderr, OUTPUT_LIMIT);
    WorkspaceGitActionResponse {
        ok,
        action: action.to_string(),
        command: command.to_string(),
        stdout,
        stderr,
        exit_code,
        message,
        truncated: stdout_truncated || stderr_truncated,
        checkpoint: None,
        root: None,
        head: None,
        worktree: None,
        undo_checkpoint: None,
        conflicts: None,
        pull_request: None,
    }
}

fn workspace_git_diff_action(
    root: &FsPath,
    action: &str,
    has_head: bool,
    scope: &str,
    base: Option<&str>,
) -> WorkspaceGitActionResponse {
    let mut temp_index = None;
    let mut index_env = Vec::new();
    let args: Vec<String> = match scope {
        "all" if has_head => ["diff", "--no-ext-diff", "--patch", "HEAD", "--"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        "all" | "unstaged" => ["diff", "--no-ext-diff", "--patch", "--"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        "staged" => ["diff", "--cached", "--no-ext-diff", "--patch", "--"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        "last_turn" => {
            let checkpoint =
                if let Some(candidate) = base.map(str::trim).filter(|value| !value.is_empty()) {
                    if !candidate.starts_with("refs/milim/checkpoints/")
                        || git_text(root, &["show-ref", "--verify", candidate]).is_none()
                    {
                        return workspace_git_action_message(
                            action,
                            "git diff <last-turn-checkpoint> --",
                            false,
                            "Select a valid Milim turn checkpoint.",
                        );
                    }
                    candidate.to_string()
                } else {
                    let Some(checkpoint) = git_text(
                        root,
                        &[
                            "for-each-ref",
                            "--sort=-refname",
                            "--sort=-creatordate",
                            "--count=1",
                            "--format=%(refname)",
                            "refs/milim/checkpoints/",
                        ],
                    ) else {
                        return workspace_git_action_message(
                            action,
                            "git diff <last-turn-checkpoint> --",
                            false,
                            "No turn checkpoint is available.",
                        );
                    };
                    checkpoint
                };
            let Some(index_path) = git_text(root, &["rev-parse", "--git-path", "index"]) else {
                return workspace_git_action_message(
                    action,
                    "git rev-parse --git-path index",
                    false,
                    "Failed to locate the Git index.",
                );
            };
            let index_path = PathBuf::from(index_path);
            let index_path = if index_path.is_absolute() {
                index_path
            } else {
                root.join(index_path)
            };
            let path = index_path.with_file_name(format!("milim-{}.index", gen_id("diff")));
            index_env.push(("GIT_INDEX_FILE", path.to_string_lossy().to_string()));
            let read_tree =
                git_output_with_env(root, &["read-tree", checkpoint.as_str()], &index_env);
            if !matches!(read_tree, Ok(ref output) if output.status.success()) {
                let _ = std::fs::remove_file(&path);
                return workspace_git_action_message(
                    action,
                    "git read-tree <last-turn-checkpoint>",
                    false,
                    "Failed to read the last turn checkpoint.",
                );
            }
            temp_index = Some(path);
            vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--patch".into(),
                checkpoint,
                "--".into(),
            ]
        }
        "commit" => {
            let candidate = base.unwrap_or_default().trim();
            if candidate.len() < 7 || !candidate.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return workspace_git_action_message(
                    action,
                    "git show <commit>",
                    false,
                    "Select a valid commit.",
                );
            }
            let verify = format!("{candidate}^{{commit}}");
            let Some(commit) = git_text(root, &["rev-parse", "--verify", verify.as_str()]) else {
                return workspace_git_action_message(
                    action,
                    "git show <commit>",
                    false,
                    "Commit not found.",
                );
            };
            vec![
                "show".into(),
                "--no-ext-diff".into(),
                "--format=".into(),
                "--patch".into(),
                commit,
                "--".into(),
            ]
        }
        "branch" => {
            let branch = base.unwrap_or_default().trim();
            if branch.is_empty() || branch.starts_with('-') {
                return workspace_git_action_message(
                    action,
                    "git diff <branch>...HEAD --",
                    false,
                    "Select a valid branch.",
                );
            }
            let reference = format!("refs/heads/{branch}");
            let verify = format!("{reference}^{{commit}}");
            if git_text(root, &["rev-parse", "--verify", verify.as_str()]).is_none() {
                return workspace_git_action_message(
                    action,
                    "git diff <branch>...HEAD --",
                    false,
                    "Branch not found.",
                );
            }
            vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--patch".into(),
                format!("{reference}...HEAD"),
                "--".into(),
            ]
        }
        _ => {
            return workspace_git_action_message(
                action,
                "git diff",
                false,
                "Unsupported diff scope.",
            )
        }
    };
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = format!("git {}", args.join(" "));
    let output = match if index_env.is_empty() {
        git_output(root, &arg_refs)
    } else {
        git_output_with_env(root, &arg_refs, &index_env)
    } {
        Ok(output) => output,
        Err(e) => {
            if let Some(path) = temp_index.as_ref() {
                let _ = std::fs::remove_file(path);
            }
            return workspace_git_action_message(action, &command, false, &e);
        }
    };
    if let Some(path) = temp_index.as_ref() {
        let _ = std::fs::remove_file(path);
    }

    let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() && matches!(scope, "all" | "unstaged" | "last_turn") {
        let excluded = if scope == "last_turn" {
            base_paths_from_diff_args(root, &args)
        } else {
            HashSet::new()
        };
        stdout.push_str(&untracked_git_diff(root, &excluded));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let ok = output.status.success();
    let message = if ok {
        if stdout.trim().is_empty() {
            "No diff to show.".to_string()
        } else {
            "Diff ready.".to_string()
        }
    } else {
        output_error_text(&output)
    };

    WorkspaceGitActionResponse {
        ok,
        action: action.to_string(),
        command,
        stdout,
        stderr,
        exit_code: output.status.code(),
        message,
        truncated: false,
        checkpoint: None,
        root: None,
        head: None,
        worktree: None,
        undo_checkpoint: None,
        conflicts: None,
        pull_request: None,
    }
}

fn base_paths_from_diff_args(root: &FsPath, args: &[String]) -> HashSet<String> {
    let Some(base) = args.get(3) else {
        return HashSet::new();
    };
    let Ok(output) = git_output(root, &["ls-tree", "-r", "--name-only", "-z", base]) else {
        return HashSet::new();
    };
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
        .collect()
}

fn untracked_git_diff(root: &FsPath, excluded: &HashSet<String>) -> String {
    let Ok(output) = git_output(root, &["ls-files", "--others", "--exclude-standard", "-z"]) else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }

    let mut patch = String::new();
    for raw in output
        .stdout
        .split(|b| *b == 0)
        .filter(|item| !item.is_empty())
    {
        let path = String::from_utf8_lossy(raw).replace('\\', "/");
        if excluded.contains(&path) {
            continue;
        }
        let Some(file_patch) = untracked_file_patch(root, &path) else {
            continue;
        };
        if !patch.ends_with('\n') && !patch.is_empty() {
            patch.push('\n');
        }
        patch.push_str(&file_patch);
    }
    patch
}

fn untracked_file_patch(root: &FsPath, path: &str) -> Option<String> {
    let full_path = root.join(path);
    if !full_path.is_file() {
        return None;
    }
    let bytes = std::fs::read(full_path).ok()?;
    let mut patch = format!(
        "\ndiff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
    );
    match String::from_utf8(bytes) {
        Ok(text) => {
            let line_count = text.lines().count().max(1);
            patch.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
            if text.is_empty() {
                return Some(patch);
            }
            for line in text.split_inclusive('\n') {
                patch.push('+');
                patch.push_str(line);
                if !line.ends_with('\n') {
                    patch.push('\n');
                }
            }
        }
        Err(_) => {
            patch.push_str(&format!("Binary files /dev/null and b/{path} differ\n"));
        }
    }
    Some(patch)
}

fn workspace_git_action_message(
    action: &str,
    command: &str,
    ok: bool,
    message: &str,
) -> WorkspaceGitActionResponse {
    WorkspaceGitActionResponse {
        ok,
        action: action.to_string(),
        command: command.to_string(),
        stdout: String::new(),
        stderr: String::new(),
        exit_code: None,
        message: message.to_string(),
        truncated: false,
        checkpoint: None,
        root: None,
        head: None,
        worktree: None,
        undo_checkpoint: None,
        conflicts: None,
        pull_request: None,
    }
}

fn truncate_git_action_output(text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}...\n[truncated]", &text[..end]), true)
}

fn git_state_label_for_action(status: &WorkspaceGitStatus) -> String {
    status
        .message
        .clone()
        .unwrap_or_else(|| "No Git repository is available.".to_string())
}

#[cfg(test)]
mod git_control_plane_tests {
    use super::*;

    #[test]
    fn pull_request_actions_validate_review_and_guard_merge_head() {
        assert_eq!(
            github_repository(Some("openai/codex".to_string())).as_deref(),
            Ok("openai/codex")
        );
        assert!(github_repository(Some("../codex".to_string())).is_err());
        assert!(
            workspace_git_pr_action_args("pr_comment", Some(12), Some(" "), None, None, None,)
                .is_err()
        );
        assert!(workspace_git_pr_action_args(
            "pr_review",
            Some(12),
            None,
            Some("request_changes"),
            None,
            None,
        )
        .is_err());

        let args = workspace_git_pr_action_args(
            "pr_merge",
            Some(12),
            None,
            None,
            Some("squash"),
            Some("abc123"),
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "pr",
                "merge",
                "12",
                "--squash",
                "--match-head-commit",
                "abc123"
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--admin"));
    }

    #[test]
    fn pull_request_selection_prefers_open_matching_head() {
        let selected = select_pull_request_for_head(
            vec![
                json!({ "number": 1, "state": "MERGED", "headRefOid": "abc" }),
                json!({ "number": 2, "state": "OPEN", "headRefOid": "def" }),
                json!({ "number": 3, "state": "OPEN", "headRefOid": "abc" }),
            ],
            Some("abc"),
        )
        .unwrap();
        assert_eq!(selected["number"], 3);
    }

    #[test]
    fn thread_worktree_lifecycle_keeps_branch_and_guards_dirty_state() {
        let base = std::env::temp_dir().join(format!("milim-thread-worktree-{}", gen_id("test")));
        let repo = base.join("repo");
        let runtime = base.join("runtime");
        std::fs::create_dir_all(&repo).unwrap();
        assert!(git_output(&repo, &["init"]).unwrap().status.success());
        std::fs::write(repo.join("README.md"), "hello").unwrap();
        assert!(git_output(&repo, &["add", "README.md"])
            .unwrap()
            .status
            .success());
        assert!(git_output(
            &repo,
            &[
                "-c",
                "user.name=Milim Test",
                "-c",
                "user.email=milim@example.invalid",
                "commit",
                "-m",
                "initial",
            ]
        )
        .unwrap()
        .status
        .success());
        let status = workspace_git_status_blocking(Some(repo.clone()));
        let created = workspace_git_create_thread_worktree_action(
            &repo,
            &status,
            Some("thread_12345678".into()),
            &runtime,
        );
        assert!(created.ok, "{}", created.message);
        let worktree = PathBuf::from(created.worktree.unwrap());
        assert!(worktree.is_dir());
        std::fs::write(worktree.join("dirty.txt"), "dirty").unwrap();
        let blocked = workspace_git_remove_thread_worktree_action(
            &repo,
            Some("thread_12345678".into()),
            false,
            &runtime,
        );
        assert!(!blocked.ok);
        assert!(blocked.message.contains("uncommitted changes"));
        let removed = workspace_git_remove_thread_worktree_action(
            &repo,
            Some("thread_12345678".into()),
            true,
            &runtime,
        );
        assert!(removed.ok, "{}", removed.message);
        assert!(
            git_text(&repo, &["branch", "--list", "milim/thread-thread_1"])
                .unwrap_or_default()
                .contains("milim/thread-thread_1")
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn thread_runtime_ids_reject_paths() {
        assert!(valid_thread_runtime_id(Some("../../outside".into())).is_err());
        assert_eq!(
            valid_thread_runtime_id(Some("abc_DEF-123".into())).unwrap(),
            "abc_DEF-123"
        );
    }
}
