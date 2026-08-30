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
//!
//! Windows is excluded: GUI processes there do inherit the user environment,
//! and each bridge already resolves its `.cmd`/`.exe` shims itself.

use std::env;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio::process::Command;

/// Build a [`Command`] for `program`, resolved to an absolute path when it can
/// be found on [`search_path`], and carrying that `PATH` for the child.
pub(crate) fn command(program: &str) -> Command {
    let mut command = match resolve(program) {
        Some(path) => Command::new(path),
        None => Command::new(program),
    };
    command.env("PATH", search_path());
    command
}

/// Resolve `program` to the real executable target on [`search_path`].
pub(crate) fn resolve(program: &str) -> Option<PathBuf> {
    env::split_paths(&search_path()).find_map(|dir| executable_target(dir.join(program)))
}

fn executable_target(candidate: PathBuf) -> Option<PathBuf> {
    if !is_executable(&candidate) {
        return None;
    }
    Some(candidate.canonicalize().unwrap_or(candidate))
}

/// The inherited `PATH` followed by the install directories a GUI launch does
/// not get. Inherited entries stay first, so a `PATH` the user really did set
/// still wins.
pub(crate) fn search_path() -> OsString {
    let inherited = env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = env::split_paths(&inherited).collect();
    for dir in extra_dirs() {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    env::join_paths(dirs).unwrap_or(inherited)
}

/// Where CLIs are installed when the shell is not there to say so.
///
/// Tool-manager *shim* directories have fixed paths and resolve the active
/// version themselves. Installs owned by a version manager that only exports
/// its bin directory from shell startup (nvm, fnm) are not reachable this way;
/// those still need Milim launched from a terminal.
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
        let path = resolve("sh").expect("`sh` is installed on every unix");
        assert!(path.is_absolute(), "{path:?} should be absolute");
        assert_eq!(path, path.canonicalize().unwrap());
        assert!(is_executable(&path), "{path:?} should be executable");
    }

    #[test]
    fn missing_binary_resolves_to_none() {
        assert!(resolve("milim-cli-that-does-not-exist").is_none());
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
