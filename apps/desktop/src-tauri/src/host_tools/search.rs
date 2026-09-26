//! `glob` and `grep`: gitignore-aware file discovery and content search.
//!
//! Inside a git repository the file set comes from `git ls-files`, so ignored
//! files stay out; elsewhere a bounded walk skips VCS, dependency, build, and
//! hidden directories. `grep` runs ripgrep when it is installed and falls back
//! to an in-process regex scan over the same file set.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};
use milim_tools::{Tool, ToolConcurrency, ToolEffect};

use super::{
    arg_str, host_tool_scoping, optional_arg_str, optional_bool, optional_u64, root_of, safe_join,
    HostCtx, PathDisplay,
};

/// Paths one `glob` call returns.
const MAX_GLOB_RESULTS: usize = 500;
/// Files a directory walk visits before it stops.
const MAX_WALK_FILES: usize = 200_000;
/// Directories a walk outside git never enters (hidden directories are skipped too).
const SKIPPED_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build"];
/// Alternatives one brace expression may expand to.
const MAX_ALTERNATIVES: usize = 256;
const GIT_TIMEOUT: Duration = Duration::from_secs(20);
const RG_TIMEOUT: Duration = Duration::from_secs(60);
/// Larger files are skipped by `grep`.
const MAX_GREP_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Leading bytes inspected to skip binary files.
const SNIFF_BYTES: usize = 8192;
/// Upper bound on the text of one `grep` reply.
const MAX_GREP_OUTPUT_BYTES: usize = 256 * 1024;
/// ripgrep output read before the search is cut short.
const MAX_RG_STDOUT: usize = 16 * 1024 * 1024;
const MAX_GIT_STDOUT: usize = 64 * 1024 * 1024;
const DEFAULT_MAX_RESULTS: u64 = 200;
const MAX_MAX_RESULTS: u64 = 5000;
const MAX_CONTEXT: u64 = 10;
const MAX_LINE_CHARS: usize = 2000;

/// A compiled glob supporting `*`, `**`, `?`, `[...]`, and `{a,b}`. Patterns
/// without a `/` match file names at any depth, like ripgrep's `--glob`.
pub(super) struct Glob {
    alternatives: Vec<Vec<Segment>>,
}

#[derive(Debug)]
enum Segment {
    /// `**`: zero or more path components.
    AnyDepth,
    Name(Vec<Token>),
}

#[derive(Debug)]
enum Token {
    Char(char),
    Star,
    Question,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

impl Token {
    fn accepts(&self, c: char) -> bool {
        match self {
            Token::Char(expected) => *expected == c,
            Token::Star | Token::Question => true,
            Token::Class { negated, ranges } => {
                ranges.iter().any(|(low, high)| *low <= c && c <= *high) != *negated
            }
        }
    }
}

impl Glob {
    pub(super) fn new(pattern: &str) -> Result<Self> {
        let pattern = if cfg!(windows) {
            pattern.trim().replace('\\', "/")
        } else {
            pattern.trim().to_string()
        };
        let pattern = pattern.strip_prefix("./").unwrap_or(&pattern);
        if pattern.is_empty() {
            return Err(Error::InvalidRequest(
                "glob pattern must not be empty".into(),
            ));
        }
        Ok(Self {
            alternatives: expand_braces(pattern)
                .iter()
                .map(|alternative| compile(alternative))
                .collect(),
        })
    }

    /// Whether a `/`-separated path relative to the search base matches.
    pub(super) fn matches(&self, path: &str) -> bool {
        let parts = path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        self.alternatives
            .iter()
            .any(|segments| match_segments(segments, &parts))
    }
}

fn compile(pattern: &str) -> Vec<Segment> {
    let anchored = pattern.trim_start_matches('/');
    let mut segments = Vec::new();
    if !anchored.contains('/') {
        segments.push(Segment::AnyDepth);
    }
    for part in anchored.split('/').filter(|part| !part.is_empty()) {
        if part == "**" {
            if !matches!(segments.last(), Some(Segment::AnyDepth)) {
                segments.push(Segment::AnyDepth);
            }
        } else {
            segments.push(Segment::Name(tokens(part)));
        }
    }
    if anchored.ends_with('/') {
        segments.push(Segment::AnyDepth);
    }
    segments
}

fn tokens(part: &str) -> Vec<Token> {
    let chars = part.chars().collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' => {
                while chars.get(index + 1) == Some(&'*') {
                    index += 1;
                }
                out.push(Token::Star);
            }
            '?' => out.push(Token::Question),
            '[' => match class(&chars, index) {
                Some((token, next)) => {
                    out.push(token);
                    index = next;
                    continue;
                }
                None => out.push(Token::Char('[')),
            },
            '\\' if !cfg!(windows) && index + 1 < chars.len() => {
                index += 1;
                out.push(Token::Char(chars[index]));
            }
            c => out.push(Token::Char(c)),
        }
        index += 1;
    }
    out
}

/// Parse `[...]` starting at `open`; `None` leaves an unclosed `[` literal.
fn class(chars: &[char], open: usize) -> Option<(Token, usize)> {
    let mut index = open + 1;
    let negated = matches!(chars.get(index), Some('!' | '^'));
    if negated {
        index += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    while index < chars.len() {
        let c = chars[index];
        if c == ']' && !first {
            return Some((Token::Class { negated, ranges }, index + 1));
        }
        first = false;
        if index + 2 < chars.len() && chars[index + 1] == '-' && chars[index + 2] != ']' {
            ranges.push((c, chars[index + 2]));
            index += 3;
        } else {
            ranges.push((c, c));
            index += 1;
        }
    }
    None
}

fn match_segments(segments: &[Segment], parts: &[&str]) -> bool {
    match segments.split_first() {
        None => parts.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=parts.len()).any(|skip| match_segments(rest, &parts[skip..]))
        }
        Some((Segment::Name(tokens), rest)) => parts
            .split_first()
            .is_some_and(|(part, tail)| match_name(tokens, part) && match_segments(rest, tail)),
    }
}

/// Wildcard match of one path component, backtracking to the last `*`.
fn match_name(tokens: &[Token], name: &str) -> bool {
    let chars = name.chars().collect::<Vec<_>>();
    let (mut token, mut position) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while position < chars.len() {
        if let Some(current) = tokens.get(token) {
            if matches!(current, Token::Star) {
                star = Some((token, position));
                token += 1;
                continue;
            }
            if current.accepts(chars[position]) {
                token += 1;
                position += 1;
                continue;
            }
        }
        match star {
            Some((star_token, star_position)) => {
                token = star_token + 1;
                position = star_position + 1;
                star = Some((star_token, star_position + 1));
            }
            None => return false,
        }
    }
    tokens[token..]
        .iter()
        .all(|token| matches!(token, Token::Star))
}

/// Expand the first `{a,b,...}` group (recursively) into separate patterns.
fn expand_braces(pattern: &str) -> Vec<String> {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' if !cfg!(windows) => index += 1,
            '{' => {
                if let Some((close, commas)) = matching_brace(&chars, index) {
                    if !commas.is_empty() {
                        let prefix = chars[..index].iter().collect::<String>();
                        let suffix = chars[close + 1..].iter().collect::<String>();
                        let bounds = std::iter::once(index)
                            .chain(commas)
                            .chain(std::iter::once(close))
                            .collect::<Vec<_>>();
                        let mut out = Vec::new();
                        for pair in bounds.windows(2) {
                            let alternative =
                                chars[pair[0] + 1..pair[1]].iter().collect::<String>();
                            for expanded in expand_braces(&format!("{prefix}{alternative}{suffix}"))
                            {
                                out.push(expanded);
                                if out.len() >= MAX_ALTERNATIVES {
                                    return out;
                                }
                            }
                        }
                        return out;
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }
    vec![pattern.to_string()]
}

/// The closing brace for `open` and the top-level comma positions inside it.
fn matching_brace(chars: &[char], open: usize) -> Option<(usize, Vec<usize>)> {
    let mut depth = 0;
    let mut commas = Vec::new();
    let mut index = open;
    while index < chars.len() {
        match chars[index] {
            '\\' if !cfg!(windows) => index += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((index, commas));
                }
            }
            ',' if depth == 1 => commas.push(index),
            _ => {}
        }
        index += 1;
    }
    None
}

/// A regular file under the search base.
pub(super) struct Listed {
    /// Path relative to the base, `/`-separated.
    pub(super) rel: String,
    pub(super) path: PathBuf,
    pub(super) modified: SystemTime,
}

/// Every regular file under `base`, honoring `.gitignore` inside a git
/// repository. Symlinks are skipped so results never leave the tree.
pub(super) fn list_files(base: &Path) -> Vec<Listed> {
    let rels = git_listing(base).unwrap_or_else(|| walk_listing(base));
    rels.into_iter()
        .filter_map(|rel| {
            let path = base.join(&rel);
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            metadata.is_file().then(|| Listed {
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                path,
                rel,
            })
        })
        .collect()
}

/// Tracked plus untracked-but-not-ignored files, or `None` outside a repository.
fn git_listing(base: &Path) -> Option<Vec<String>> {
    let mut command = helper_command("git");
    command.arg("-C").arg(base).args([
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "-z",
    ]);
    let output = run_bounded(command, GIT_TIMEOUT, MAX_GIT_STDOUT).ok()?;
    if !output.status.is_some_and(|status| status.success()) || output.truncated {
        return None;
    }
    let mut rels = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|rel| !rel.is_empty())
        .map(|rel| String::from_utf8_lossy(rel).into_owned())
        .collect::<Vec<_>>();
    rels.sort_unstable();
    rels.dedup();
    Some(rels)
}

fn walk_listing(base: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![(base.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                if !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_str()) {
                    pending.push((entry.path(), rel));
                }
            } else if kind.is_file() {
                out.push(rel);
                if out.len() >= MAX_WALK_FILES {
                    return out;
                }
            }
        }
    }
    out
}

/// The `PATH` searched for helper binaries, resolved once per process.
fn helper_search_path() -> &'static OsString {
    static SEARCH_PATH: OnceLock<OsString> = OnceLock::new();
    SEARCH_PATH.get_or_init(|| {
        #[cfg(not(windows))]
        {
            milim_server::cli_search_path()
        }
        #[cfg(windows)]
        {
            std::env::var_os("PATH").unwrap_or_default()
        }
    })
}

fn find_helper(program: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    std::env::split_paths(helper_search_path())
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

fn helper_command(program: &str) -> Command {
    let mut command = Command::new(find_helper(program).unwrap_or_else(|| program.into()));
    milim_core::proc::hide_console(&mut command);
    command
}

/// The ripgrep binary, when installed.
fn ripgrep() -> Option<&'static Path> {
    static RIPGREP: OnceLock<Option<PathBuf>> = OnceLock::new();
    RIPGREP.get_or_init(|| find_helper("rg")).as_deref()
}

struct BoundedOutput {
    /// `None` when the process was killed after its output limit.
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// Stdout reached its limit and the pipe was closed early.
    truncated: bool,
}

fn read_limited(mut stream: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return (kept, false),
            Ok(count) => {
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..count.min(room)]);
                if count > room {
                    return (kept, true);
                }
            }
        }
    }
}

/// Run a helper with a deadline, keeping at most `max_stdout` bytes of output.
/// Hitting the limit closes the pipe so the helper stops early.
fn run_bounded(
    mut command: Command,
    timeout: Duration,
    max_stdout: usize,
) -> std::result::Result<BoundedOutput, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child.stdout.take().ok_or("stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("stderr unavailable")?;
    let stdout = std::thread::spawn(move || read_limited(stdout, max_stdout));
    let stderr = std::thread::spawn(move || read_limited(stderr, 64 * 1024));
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if stdout.is_finished() && Instant::now() < deadline => {
                // The reader stopped at its limit; give the helper a moment to
                // notice the closed pipe, then stop it.
                std::thread::sleep(Duration::from_millis(50));
                if let Ok(Some(status)) = child.try_wait() {
                    break Some(status);
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let (stdout, truncated) = stdout.join().unwrap_or_default();
    let (stderr, _) = stderr.join().unwrap_or_default();
    if timed_out {
        return Err(format!("timed out after {} seconds", timeout.as_secs()));
    }
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
        truncated,
    })
}

/// Find files by glob pattern.
pub struct GlobTool {
    pub(super) ctx: HostCtx,
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "Find files by glob pattern (`*`, `**`, `?`, `[...]`, `{a,b}`), e.g. `src/**/*.ts`. A pattern without `/` matches file names at any depth. Honors .gitignore; returns paths relative to the working folder, newest first, up to 500."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "pattern":{"type":"string","description":"Glob pattern relative to path."},
            "path":{"type":"string","description":"Directory to search, default the working folder."}
        },"required":["pattern"],"additionalProperties":false})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        let files = result.get("files")?.as_array()?;
        let pattern = result.get("pattern")?.as_str()?;
        if files.is_empty() {
            return Some(format!("No files matched {pattern}."));
        }
        let mut out = files
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        if result["truncated"].as_bool().unwrap_or(false) {
            let _ = write!(
                out,
                "\n(Showing the {} newest of {} matches. Narrow the pattern or path.)",
                files.len(),
                result["count"]
            );
        }
        Some(out)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let pattern = arg_str(&args, "pattern")?.to_string();
        let rel = optional_arg_str(&args, "path")?.unwrap_or("");
        let root = root_of(&self.ctx.ws)?;
        let base = safe_join(&self.ctx.ws, rel)?;
        if !base.is_dir() {
            return Err(Error::InvalidRequest(format!(
                "{} is not a directory",
                if rel.is_empty() { "." } else { rel }
            )));
        }
        let relative_pattern = relative_to_base(&pattern, &base)?;
        let glob = Glob::new(&relative_pattern)?;
        tokio::task::spawn_blocking(move || {
            let mut files = list_files(&base)
                .into_iter()
                .filter(|file| glob.matches(&file.rel))
                .collect::<Vec<_>>();
            files.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.rel.cmp(&b.rel)));
            let count = files.len();
            files.truncate(MAX_GLOB_RESULTS);
            let display = PathDisplay::new(&root);
            json!({
                "pattern": pattern,
                "files": files.iter().map(|file| display.show(&file.path)).collect::<Vec<_>>(),
                "count": count,
                "truncated": count > MAX_GLOB_RESULTS,
            })
        })
        .await
        .map_err(|error| Error::Other(format!("glob task failed: {error}")))
    }
}

/// Accept absolute patterns that point inside the search base.
fn relative_to_base(pattern: &str, base: &Path) -> Result<String> {
    if !Path::new(pattern).is_absolute() {
        return Ok(pattern.to_string());
    }
    let canonical = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    [canonical.as_path(), base]
        .iter()
        .find_map(|prefix| {
            let prefix = prefix.to_string_lossy();
            pattern
                .strip_prefix(prefix.as_ref())
                .filter(|rest| rest.is_empty() || rest.starts_with(['/', '\\']))
                .map(|rest| rest.trim_start_matches(['/', '\\']).to_string())
        })
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| {
            Error::InvalidRequest(
                "glob patterns are relative to `path`; pass the directory as `path` instead".into(),
            )
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GrepMode {
    Content,
    FilesWithMatches,
    Count,
}

struct GrepRequest {
    pattern: String,
    target: PathBuf,
    target_is_file: bool,
    glob: Option<String>,
    case_insensitive: bool,
    mode: GrepMode,
    context: usize,
    max_results: usize,
}

/// One output line of content mode.
#[derive(Debug)]
struct Hit {
    path: PathBuf,
    line: u64,
    text: String,
    matched: bool,
}

#[derive(Debug)]
enum Findings {
    Files(Vec<PathBuf>),
    Counts(Vec<(PathBuf, u64)>),
    Content(Vec<Hit>),
}

/// Search file contents with a regular expression.
pub struct GrepTool {
    pub(super) ctx: HostCtx,
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search file contents with a regular expression (ripgrep syntax). Honors .gitignore and skips binary files and files over 2 MiB. output_mode: files_with_matches (default, newest first), content (path:line:text, with optional context lines), or count."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "pattern":{"type":"string","description":"Regular expression to search for."},
            "path":{"type":"string","description":"File or directory to search, default the working folder."},
            "glob":{"type":"string","description":"Only search files matching this glob, e.g. `*.rs` or `src/**/*.ts`."},
            "case_insensitive":{"type":"boolean","description":"Match case-insensitively. Default false."},
            "output_mode":{"type":"string","enum":["content","files_with_matches","count"],"description":"Default files_with_matches."},
            "context":{"type":"integer","minimum":0,"maximum":MAX_CONTEXT,"description":"Lines of context around each match in content mode. Default 0."},
            "max_results":{"type":"integer","minimum":1,"maximum":MAX_MAX_RESULTS,"description":"Maximum files (or matching lines in content mode). Default 200."}
        },"required":["pattern"],"additionalProperties":false})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        let truncated = result["truncated"].as_bool().unwrap_or(false);
        let mut out = match result.get("mode")?.as_str()? {
            "content" => {
                let output = result.get("output")?.as_str()?;
                if output.is_empty() {
                    "No matches.".to_string()
                } else {
                    output.to_string()
                }
            }
            "count" => {
                let counts = result.get("counts")?.as_array()?;
                if counts.is_empty() {
                    "No matches.".to_string()
                } else {
                    let mut out = String::new();
                    for entry in counts {
                        let _ = writeln!(out, "{}:{}", entry["path"].as_str()?, entry["count"]);
                    }
                    let _ = write!(
                        out,
                        "Total: {} matching lines in {} files",
                        result["total"], result["files"]
                    );
                    out
                }
            }
            _ => {
                let files = result.get("files")?.as_array()?;
                if files.is_empty() {
                    "No matches.".to_string()
                } else {
                    files
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
        };
        if truncated {
            out.push_str("\n(Results were truncated. Narrow the pattern, path, or glob.)");
        }
        Some(out)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let pattern = arg_str(&args, "pattern")?;
        if pattern.is_empty() {
            return Err(Error::InvalidRequest("pattern must not be empty".into()));
        }
        let rel = optional_arg_str(&args, "path")?.unwrap_or("");
        let root = root_of(&self.ctx.ws)?;
        let target = safe_join(&self.ctx.ws, rel)?;
        let target_is_file = std::fs::metadata(&target)?.is_file();
        let mode = match optional_arg_str(&args, "output_mode")?.unwrap_or("files_with_matches") {
            "content" => GrepMode::Content,
            "files_with_matches" => GrepMode::FilesWithMatches,
            "count" => GrepMode::Count,
            other => {
                return Err(Error::InvalidRequest(format!(
                    "unknown output_mode: {other} (use content, files_with_matches, or count)"
                )))
            }
        };
        let context = optional_u64(&args, "context", 0)?;
        if context > MAX_CONTEXT {
            return Err(Error::InvalidRequest(format!(
                "context must be at most {MAX_CONTEXT}"
            )));
        }
        let max_results =
            optional_u64(&args, "max_results", DEFAULT_MAX_RESULTS)?.clamp(1, MAX_MAX_RESULTS);
        let request = GrepRequest {
            pattern: pattern.to_string(),
            target,
            target_is_file,
            glob: optional_arg_str(&args, "glob")?
                .filter(|glob| !glob.trim().is_empty())
                .map(ToString::to_string),
            case_insensitive: optional_bool(&args, "case_insensitive")?,
            mode,
            context: context as usize,
            max_results: max_results as usize,
        };
        tokio::task::spawn_blocking(move || {
            let searched = match ripgrep() {
                Some(rg) => grep_ripgrep(rg, &request)?.map(|found| (found, "ripgrep")),
                None => None,
            };
            let (found, engine) = match searched {
                Some(searched) => searched,
                None => (grep_builtin(&request)?, "builtin"),
            };
            Ok(render_findings(
                found,
                &request,
                &PathDisplay::new(&root),
                engine,
            ))
        })
        .await
        .map_err(|error| Error::Other(format!("grep task failed: {error}")))?
    }
}

/// Search with ripgrep. `Ok(None)` means ripgrep could not start.
fn grep_ripgrep(rg: &Path, request: &GrepRequest) -> Result<Option<(Findings, bool)>> {
    let mut command = Command::new(rg);
    milim_core::proc::hide_console(&mut command);
    command
        .env_remove("RIPGREP_CONFIG_PATH")
        .args([
            "--no-config",
            "--no-messages",
            "--color",
            "never",
            "--hidden",
            "--max-filesize",
            "2M",
        ])
        .arg(if request.case_insensitive {
            "--ignore-case"
        } else {
            "--case-sensitive"
        });
    if let Some(dir) = request.target.parent().filter(|_| request.target_is_file) {
        command.current_dir(dir);
    } else {
        command.current_dir(&request.target);
    }
    if let Some(glob) = &request.glob {
        command.arg("--glob").arg(glob);
    }
    // Later globs take precedence, so this exclusion stays last.
    command.args(["--glob", "!.git"]);
    match request.mode {
        GrepMode::FilesWithMatches => {
            command.args(["--files-with-matches", "--null"]);
        }
        GrepMode::Count => {
            command.args(["--count", "--null", "--sort", "path"]);
        }
        GrepMode::Content => {
            command.args(["--json", "--sort", "path"]);
            if request.context > 0 {
                command.arg("--context").arg(request.context.to_string());
            }
        }
    }
    command
        .arg("--regexp")
        .arg(&request.pattern)
        .arg("--")
        .arg(&request.target);
    let output = match run_bounded(command, RG_TIMEOUT, MAX_RG_STDOUT) {
        Ok(output) => output,
        Err(error) if error.starts_with("timed out") => {
            return Err(Error::Other(format!("grep {error}")));
        }
        Err(_) => return Ok(None),
    };
    let failed = match output.status {
        Some(status) => !matches!(status.code(), Some(0 | 1)),
        None => !output.truncated,
    };
    if failed && output.stdout.is_empty() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(Error::InvalidRequest(format!(
            "grep failed: {}",
            message.trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let findings = match request.mode {
        GrepMode::FilesWithMatches => Findings::Files(
            stdout
                .split('\0')
                .map(|path| path.trim_matches('\n'))
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .collect(),
        ),
        GrepMode::Count => Findings::Counts(
            stdout
                .lines()
                .filter_map(|line| {
                    let (path, count) = line.split_once('\0')?;
                    Some((PathBuf::from(path), count.trim().parse().ok()?))
                })
                .collect(),
        ),
        GrepMode::Content => Findings::Content(
            stdout
                .lines()
                .filter_map(|line| {
                    let event: Value = serde_json::from_str(line).ok()?;
                    let matched = match event["type"].as_str()? {
                        "match" => true,
                        "context" => false,
                        _ => return None,
                    };
                    let data = &event["data"];
                    let text = data["lines"]["text"].as_str()?;
                    Some(Hit {
                        path: PathBuf::from(data["path"]["text"].as_str()?),
                        line: data["line_number"].as_u64()?,
                        text: text.trim_end_matches(['\n', '\r']).to_string(),
                        matched,
                    })
                })
                .collect(),
        ),
    };
    Ok(Some((findings, output.truncated)))
}

/// Search in-process over the `glob` file set.
fn grep_builtin(request: &GrepRequest) -> Result<(Findings, bool)> {
    let regex = regex::RegexBuilder::new(&request.pattern)
        .case_insensitive(request.case_insensitive)
        .build()
        .map_err(|error| Error::InvalidRequest(format!("invalid regex: {error}")))?;
    let filter = request.glob.as_deref().map(Glob::new).transpose()?;
    let mut files = if request.target_is_file {
        vec![request.target.clone()]
    } else {
        let mut listed = list_files(&request.target)
            .into_iter()
            .filter(|file| filter.as_ref().is_none_or(|glob| glob.matches(&file.rel)))
            .collect::<Vec<_>>();
        listed.sort_by(|a, b| a.rel.cmp(&b.rel));
        listed.into_iter().map(|file| file.path).collect()
    };
    files.retain(|path| {
        std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= MAX_GREP_FILE_BYTES)
    });

    let mut matched_files = Vec::new();
    let mut counts = Vec::new();
    let mut hits = Vec::new();
    let mut matched_lines = 0;
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes[..bytes.len().min(SNIFF_BYTES)].contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let body = text.strip_suffix('\n').unwrap_or(&text);
        let lines = body
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect::<Vec<_>>();
        match request.mode {
            GrepMode::FilesWithMatches => {
                if lines.iter().any(|line| regex.is_match(line)) {
                    matched_files.push(path);
                }
            }
            GrepMode::Count => {
                let count = lines.iter().filter(|line| regex.is_match(line)).count();
                if count > 0 {
                    counts.push((path, count as u64));
                }
            }
            GrepMode::Content => {
                let matches = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| regex.is_match(line))
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                let mut next = 0;
                for (position, index) in matches.iter().enumerate() {
                    let start = index.saturating_sub(request.context).max(next);
                    let end = (index + request.context + 1).min(lines.len());
                    let end = matches
                        .get(position + 1)
                        .map(|following| end.min(*following))
                        .unwrap_or(end);
                    for (line, text) in lines.iter().enumerate().take(end).skip(start) {
                        hits.push(Hit {
                            path: path.clone(),
                            line: line as u64 + 1,
                            text: text.to_string(),
                            matched: line == *index,
                        });
                    }
                    next = end;
                    matched_lines += 1;
                }
                if matched_lines > request.max_results {
                    return Ok((Findings::Content(hits), false));
                }
            }
        }
    }
    Ok((
        match request.mode {
            GrepMode::FilesWithMatches => Findings::Files(matched_files),
            GrepMode::Count => Findings::Counts(counts),
            GrepMode::Content => Findings::Content(hits),
        },
        false,
    ))
}

fn cut_line(text: &str) -> String {
    match text.char_indices().nth(MAX_LINE_CHARS) {
        Some((cut, _)) => format!("{}... [line cut]", &text[..cut]),
        None => text.to_string(),
    }
}

fn render_findings(
    (findings, cut_short): (Findings, bool),
    request: &GrepRequest,
    display: &PathDisplay,
    engine: &str,
) -> Value {
    let limit = request.max_results;
    match findings {
        Findings::Files(files) => {
            let mut files = files
                .into_iter()
                .map(|path| {
                    let modified = std::fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    (modified, display.show(&path))
                })
                .collect::<Vec<_>>();
            files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let count = files.len();
            files.truncate(limit);
            json!({
                "mode": "files_with_matches",
                "files": files.into_iter().map(|(_, path)| path).collect::<Vec<_>>(),
                "count": count,
                "truncated": cut_short || count > limit,
                "engine": engine,
            })
        }
        Findings::Counts(counts) => {
            let total: u64 = counts.iter().map(|(_, count)| count).sum();
            let files = counts.len();
            json!({
                "mode": "count",
                "counts": counts
                    .iter()
                    .take(limit)
                    .map(|(path, count)| json!({"path": display.show(path), "count": count}))
                    .collect::<Vec<_>>(),
                "total": total,
                "files": files,
                "truncated": cut_short || files > limit,
                "engine": engine,
            })
        }
        Findings::Content(hits) => {
            let mut output = String::new();
            let mut shown = 0;
            let mut truncated = cut_short;
            let mut previous: Option<(&Path, u64)> = None;
            let mut shown_path = (PathBuf::new(), String::new());
            for hit in &hits {
                if hit.matched && shown == limit {
                    truncated = true;
                    break;
                }
                if output.len() > MAX_GREP_OUTPUT_BYTES {
                    truncated = true;
                    break;
                }
                let contiguous =
                    previous.is_some_and(|(path, line)| path == hit.path && line + 1 == hit.line);
                if request.context > 0 && previous.is_some() && !contiguous {
                    output.push_str("--\n");
                }
                if shown_path.0 != hit.path {
                    shown_path = (hit.path.clone(), display.show(&hit.path));
                }
                let separator = if hit.matched { ':' } else { '-' };
                let _ = writeln!(
                    output,
                    "{}{separator}{}{separator}{}",
                    shown_path.1,
                    hit.line,
                    cut_line(&hit.text)
                );
                shown += usize::from(hit.matched);
                previous = Some((&hit.path, hit.line));
            }
            json!({
                "mode": "content",
                "output": output.trim_end(),
                "matches": shown,
                "truncated": truncated,
                "engine": engine,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "milim-host-search-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::canonicalize(root).unwrap()
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn git_available() -> bool {
        helper_command("git")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn request(target: &Path, pattern: &str, mode: GrepMode) -> GrepRequest {
        GrepRequest {
            pattern: pattern.into(),
            target: target.to_path_buf(),
            target_is_file: false,
            glob: None,
            case_insensitive: false,
            mode,
            context: 0,
            max_results: 200,
        }
    }

    #[test]
    fn glob_matching_covers_wildcards_classes_and_braces() {
        let cases = [
            ("*.rs", "main.rs", true),
            ("*.rs", "src/deep/lib.rs", true),
            ("*.rs", "src/lib.rsx", false),
            ("src/*.rs", "src/lib.rs", true),
            ("src/*.rs", "src/nested/lib.rs", false),
            ("src/**/*.rs", "src/lib.rs", true),
            ("src/**/*.rs", "src/a/b/lib.rs", true),
            ("src/**", "src/a/b/c.txt", true),
            ("**/test_?.py", "pkg/test_a.py", true),
            ("**/test_?.py", "pkg/test_ab.py", false),
            ("file[0-9].txt", "file7.txt", true),
            ("file[!0-9].txt", "file7.txt", false),
            ("file[!0-9].txt", "filex.txt", true),
            ("*.{ts,tsx}", "app/view.tsx", true),
            ("*.{ts,tsx}", "app/view.js", false),
            ("{src,lib}/**/*.{c,h}", "lib/x/y.h", true),
            ("{src,lib}/**/*.{c,h}", "docs/y.h", false),
            ("a*b*c", "axxbyyc", true),
            ("a*b*c", "axxbyy", false),
            ("**", "anything/at/all", true),
            ("./src/*.rs", "src/main.rs", true),
            ("src/", "src/main.rs", true),
        ];
        for (pattern, path, expected) in cases {
            assert_eq!(
                Glob::new(pattern).unwrap().matches(path),
                expected,
                "{pattern} vs {path}"
            );
        }
        assert!(Glob::new("  ").is_err());
    }

    #[test]
    fn listing_honors_gitignore_inside_a_repository() {
        if !git_available() {
            return;
        }
        let root = temp_dir();
        write(&root, ".gitignore", "ignored/\n*.log\n");
        write(&root, "src/main.rs", "fn main() {}\n");
        write(&root, "notes.txt", "notes\n");
        write(&root, "debug.log", "noise\n");
        write(&root, "ignored/skip.rs", "skip\n");
        let init = helper_command("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(init.success());
        let mut rels = list_files(&root)
            .into_iter()
            .map(|file| file.rel)
            .collect::<Vec<_>>();
        rels.sort();
        assert_eq!(rels, [".gitignore", "notes.txt", "src/main.rs"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn walk_skips_dependency_build_and_hidden_directories() {
        let root = temp_dir();
        write(&root, "src/lib.rs", "");
        write(&root, "node_modules/pkg/index.js", "");
        write(&root, "target/debug/out", "");
        write(&root, ".cache/entry", "");
        write(&root, ".env", "");
        let mut rels = walk_listing(&root);
        rels.sort();
        assert_eq!(rels, [".env", "src/lib.rs"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn builtin_grep_supports_every_mode_and_skips_binary_files() {
        let root = temp_dir();
        write(&root, "a.rs", "fn alpha() {}\nlet x = 1;\nfn beta() {}\n");
        write(&root, "b.txt", "nothing here\nFN GAMMA\n");
        std::fs::write(root.join("blob.bin"), b"fn \0binary").unwrap();

        let (files, _) =
            grep_builtin(&request(&root, r"fn \w+", GrepMode::FilesWithMatches)).unwrap();
        let Findings::Files(files) = files else {
            panic!("expected files")
        };
        assert_eq!(files, [root.join("a.rs")]);

        let mut insensitive = request(&root, "fn gamma", GrepMode::Count);
        insensitive.case_insensitive = true;
        let (counts, _) = grep_builtin(&insensitive).unwrap();
        let Findings::Counts(counts) = counts else {
            panic!("expected counts")
        };
        assert_eq!(counts, [(root.join("b.txt"), 1)]);

        let mut content = request(&root, "beta", GrepMode::Content);
        content.context = 1;
        content.glob = Some("*.rs".into());
        let found = grep_builtin(&content).unwrap();
        let rendered = render_findings(found, &content, &PathDisplay::new(&root), "builtin");
        assert_eq!(rendered["output"], "a.rs-2-let x = 1;\na.rs:3:fn beta() {}");
        assert_eq!(rendered["matches"], 1);

        let mut limited = request(&root, "fn", GrepMode::Content);
        limited.max_results = 1;
        let found = grep_builtin(&limited).unwrap();
        let rendered = render_findings(found, &limited, &PathDisplay::new(&root), "builtin");
        assert_eq!(rendered["output"], "a.rs:1:fn alpha() {}");
        assert_eq!(rendered["truncated"], true);

        assert!(grep_builtin(&request(&root, "(", GrepMode::Count)).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn builtin_grep_separates_non_adjacent_context_groups() {
        let root = temp_dir();
        write(&root, "a.txt", "hit\n2\n3\n4\n5\nhit\n");
        let mut content = request(&root, "hit", GrepMode::Content);
        content.context = 1;
        let found = grep_builtin(&content).unwrap();
        let rendered = render_findings(found, &content, &PathDisplay::new(&root), "builtin");
        assert_eq!(
            rendered["output"],
            "a.txt:1:hit\na.txt-2-2\n--\na.txt-5-5\na.txt:6:hit"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ripgrep_and_builtin_agree_when_ripgrep_is_installed() {
        let Some(rg) = ripgrep() else {
            return;
        };
        let root = temp_dir();
        write(&root, "a.rs", "fn alpha() {}\nlet x = 1;\nfn beta() {}\n");
        write(&root, "b.txt", "nothing here\n");
        for mode in [
            GrepMode::FilesWithMatches,
            GrepMode::Count,
            GrepMode::Content,
        ] {
            let mut search = request(&root, "fn|let", mode);
            search.context = 1;
            let display = PathDisplay::new(&root);
            let ripgrep = render_findings(
                grep_ripgrep(rg, &search).unwrap().unwrap(),
                &search,
                &display,
                "",
            );
            let builtin = render_findings(grep_builtin(&search).unwrap(), &search, &display, "");
            assert_eq!(ripgrep, builtin, "{mode:?}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
