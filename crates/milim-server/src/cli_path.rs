//! Locating CLI binaries when the process did not inherit a shell `PATH`.
//!
//! A desktop launch (macOS Dock/Finder, a Linux `.desktop` entry) starts the
//! app from `launchd` or the session manager rather than from a shell, so it
//! inherits a minimal `PATH` — on current macOS that is
//! `/usr/bin:/bin:/usr/sbin:/sbin`. None of the directories people actually
//! install these CLIs into are on it, so `Command::new("claude")` fails from
//! the Dock while the very same command works in a terminal.
//!
//! Two things matter for the fix:
//!
//! * `execvp`/`posix_spawnp` resolve a bare program name against the *calling*
//!   process's `PATH`. `command.env("PATH", …)` does not affect that lookup, so
//!   the binary has to be resolved to an absolute path here.
//! * The child still needs the enriched `PATH` for its own helpers (node, git,
//!   ripgrep), so it is exported as well.
//! * App-bundled CLIs can locate companion executables relative to their own
//!   binary. Symlinks are resolved before launch so that lookup starts from the
//!   real installation directory rather than the directory containing a shim.
//! * Version managers such as nvm and fnm only export their bin directory from
//!   shell startup files. The user's login shell is asked for its `PATH` once
//!   (bounded by a short timeout) and those entries join the search after the
//!   inherited ones. A per-runtime "Locate binary..." override still wins.
//!
//! Windows is excluded: GUI processes there do inherit the user environment,
//! and each bridge already resolves its `.cmd`/`.exe` shims itself.

use std::env;
use std::ffi::OsString;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command as BlockingCommand, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tokio::process::Command;

/// Build a [`Command`] for `program`, resolved to an absolute path when it can
/// be found on [`search_path`], and carrying that `PATH` for the child.
pub(crate) fn command(program: &str) -> Command {
    if let Some(command) = crate::runtime_binaries::override_command(program) {
        return command;
    }
    let search_path = search_path();
    let mut command = match resolve_on_path(program, &search_path) {
        Some(path) => Command::new(path),
        None => Command::new(program),
    };
    command.env("PATH", search_path);
    command
}

/// Blocking equivalent of [`command`] for synchronous native integrations.
pub(crate) fn blocking_command(program: &str) -> BlockingCommand {
    blocking_command_on_path(program, search_path())
}

fn blocking_command_on_path(program: &str, search_path: OsString) -> BlockingCommand {
    let mut command = match resolve_on_path(program, &search_path) {
        Some(path) => BlockingCommand::new(path),
        None => BlockingCommand::new(program),
    };
    command.env("PATH", search_path);
    command
}

fn resolve_on_path(program: &str, search_path: &OsString) -> Option<PathBuf> {
    env::split_paths(search_path).find_map(|dir| executable_target(dir.join(program)))
}

fn executable_target(candidate: PathBuf) -> Option<PathBuf> {
    if !is_executable(&candidate) {
        return None;
    }
    Some(candidate.canonicalize().unwrap_or(candidate))
}

/// The inherited `PATH`, then the login shell's `PATH`, then the install
/// directories a GUI launch does not get. Inherited entries stay first, so a
/// `PATH` the user really did set still wins.
pub(crate) fn search_path() -> OsString {
    let inherited = env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = env::split_paths(&inherited).collect();
    let login = login_shell_path()
        .map(|path| env::split_paths(path).collect::<Vec<_>>())
        .unwrap_or_default();
    for dir in login.into_iter().chain(extra_dirs()) {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    env::join_paths(dirs).unwrap_or(inherited)
}

const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(3);
const LOGIN_PATH_MARKER: &str = "__MILIM_LOGIN_PATH__";

/// Start resolving the login shell's `PATH` in the background so the first
/// runtime spawn does not wait for it.
pub(crate) fn warm_login_shell_path() {
    if LOGIN_SHELL_PATH.get().is_none() {
        std::thread::spawn(|| {
            login_shell_path();
        });
    }
}

static LOGIN_SHELL_PATH: OnceLock<Option<OsString>> = OnceLock::new();

/// The `PATH` an interactive login shell exports, resolved once per process.
/// `None` when `$SHELL` is unset, fails, or exceeds the timeout.
fn login_shell_path() -> Option<&'static OsString> {
    LOGIN_SHELL_PATH
        .get_or_init(|| {
            let shell = env::var_os("SHELL").filter(|shell| !shell.is_empty())?;
            read_shell_path(Path::new(&shell), LOGIN_SHELL_TIMEOUT)
        })
        .as_ref()
}

/// Ask `shell` for its `PATH`. The shell runs as a login and interactive
/// shell because nvm and fnm installers write to the interactive rc file
/// (`.zshrc`, `.bashrc`); markers separate the value from any startup output.
fn read_shell_path(shell: &Path, timeout: Duration) -> Option<OsString> {
    let script = format!("printf '{LOGIN_PATH_MARKER}%s{LOGIN_PATH_MARKER}' \"$PATH\"");
    let mut child = BlockingCommand::new(shell)
        .args(["-i", "-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        output
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = String::from_utf8_lossy(&reader.join().ok()?).into_owned();
    parse_marked_path(&output).map(OsString::from)
}

fn parse_marked_path(output: &str) -> Option<String> {
    let start = output.find(LOGIN_PATH_MARKER)? + LOGIN_PATH_MARKER.len();
    let length = output[start..].find(LOGIN_PATH_MARKER)?;
    let path = output[start..start + length].trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// Where CLIs are installed when the shell is not there to say so.
///
/// Tool-manager *shim* directories have fixed paths and resolve the active
/// version themselves. Installs owned by a version manager that only exports
/// its bin directory from shell startup (nvm, fnm) come from
/// [`login_shell_path`] instead, or from a per-runtime binary override.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        for suffix in [
            ".opencode/bin",
            ".local/bin",
            "bin",
            ".bun/bin",
            ".deno/bin",
            ".cargo/bin",
            ".volta/bin",
            ".asdf/shims",
            ".local/share/mise/shims",
            ".local/share/pnpm",
            "Library/pnpm",
            ".npm-global/bin",
            ".yarn/bin",
        ] {
            dirs.push(home.join(suffix));
        }
    }
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/usr/local/sbin",
            "/opt/local/bin",
            "/snap/bin",
        ]
        .map(PathBuf::from),
    );
    dirs
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_an_existing_binary_to_an_absolute_path() {
        let path = resolve_on_path("sh", &search_path()).expect("`sh` is installed on every unix");
        assert!(path.is_absolute(), "{path:?} should be absolute");
        assert_eq!(path, path.canonicalize().unwrap());
        assert!(is_executable(&path), "{path:?} should be executable");
    }

    #[test]
    fn login_shell_path_is_read_between_markers_and_bounded() {
        assert_eq!(
            parse_marked_path(&format!(
                "motd noise\n{LOGIN_PATH_MARKER}/a/bin:/b/bin{LOGIN_PATH_MARKER}trailing"
            )),
            Some("/a/bin:/b/bin".into())
        );
        assert_eq!(parse_marked_path("no markers here"), None);
        assert_eq!(
            parse_marked_path(&format!("{LOGIN_PATH_MARKER}{LOGIN_PATH_MARKER}")),
            None
        );

        let root = env::temp_dir().join(format!("milim-cli-path-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let write_shell = |name: &str, body: &str| {
            let path = root.join(name);
            std::fs::write(&path, body).unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
            path
        };
        // A fake shell that ignores its flags and reports a manager's bin dir.
        let fake = write_shell(
            "fake-shell",
            &format!(
                "#!/bin/sh\necho 'welcome'\nprintf '{LOGIN_PATH_MARKER}%s{LOGIN_PATH_MARKER}' '/home/me/.nvm/versions/node/v22/bin:/usr/bin'\n"
            ),
        );
        assert_eq!(
            read_shell_path(&fake, Duration::from_secs(5)),
            Some(OsString::from(
                "/home/me/.nvm/versions/node/v22/bin:/usr/bin"
            ))
        );
        let slow = write_shell("slow-shell", "#!/bin/sh\nsleep 5\n");
        let started = Instant::now();
        assert_eq!(read_shell_path(&slow, Duration::from_millis(200)), None);
        assert!(started.elapsed() < Duration::from_secs(3));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_binary_resolves_to_none() {
        assert!(resolve_on_path("milim-cli-that-does-not-exist", &search_path()).is_none());
    }

    #[test]
    fn resolves_an_executable_symlink_to_its_real_target() {
        let root = env::temp_dir().join(format!("milim-cli-path-test-{}", uuid::Uuid::new_v4()));
        let resources = root.join("ChatGPT.app/Contents/Resources");
        let bin = root.join(".local/bin");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::create_dir_all(&bin).unwrap();

        let target = resources.join("codex");
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        let mut permissions = std::fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&target, permissions).unwrap();

        let link = bin.join("codex");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert_eq!(
            executable_target(link),
            Some(target.canonicalize().unwrap())
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocking_command_resolves_against_its_child_path() {
        let root = env::temp_dir().join(format!("milim-cli-path-test-{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();

        let target = bin.join("gh");
        std::fs::write(&target, "#!/bin/sh\nprintf fake-gh").unwrap();
        let mut permissions = std::fs::metadata(&target).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&target, permissions).unwrap();

        let search_path = env::join_paths([bin]).unwrap();
        let mut command = blocking_command_on_path("gh", search_path.clone());

        assert_eq!(command.get_program(), target.canonicalize().unwrap());
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == "PATH")
                .and_then(|(_, value)| value),
            Some(search_path.as_os_str())
        );
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"fake-gh");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn search_path_keeps_inherited_entries_and_adds_install_dirs() {
        let dirs: Vec<PathBuf> = env::split_paths(&search_path()).collect();
        for inherited in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
            assert!(dirs.contains(&inherited), "dropped inherited {inherited:?}");
        }
        if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
            assert!(dirs.contains(&home.join(".opencode/bin")));
            assert!(dirs.contains(&home.join(".local/bin")));
        }
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
    }

    #[test]
    fn install_dirs_already_on_path_are_not_appended_again() {
        let inherited: Vec<PathBuf> =
            env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect();
        let dirs: Vec<PathBuf> = env::split_paths(&search_path()).collect();
        for appended in &dirs[inherited.len()..] {
            assert!(!inherited.contains(appended), "re-appended {appended:?}");
        }
    }
}
