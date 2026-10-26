//! Model-facing explanations of failed file operations.
//!
//! [`Error::Io`] displays the bare OS message ("No such file or directory"),
//! which names neither the path nor a way forward. File tools pass their I/O
//! errors through [`explain_file_error`] instead, which names the path, says
//! what went wrong, and suggests similarly named files or a better tool.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use milim_core::Error;

/// Similar names suggested for a missing path.
pub const MAX_SIMILAR_PATHS: usize = 3;
/// Entries of one directory compared against a missing name.
const MAX_DIR_SCAN: usize = 5000;

/// How close `candidate` is to the wanted file name, lower is closer, or
/// `None` when it is not similar enough to suggest. Case-only differences
/// rank first, then the same stem with another extension, then small edit
/// distances, then names that contain one another.
pub fn name_distance(wanted: &str, candidate: &str) -> Option<usize> {
    let (wanted, candidate) = (wanted.to_lowercase(), candidate.to_lowercase());
    if wanted == candidate {
        return Some(0);
    }
    let stem = |name: &str| {
        Path::new(name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_string())
    };
    let (wanted_stem, candidate_stem) = (stem(&wanted), stem(&candidate));
    if wanted_stem == candidate_stem {
        return Some(1);
    }
    let distance = levenshtein(&wanted, &candidate);
    if distance <= (wanted.chars().count() / 3).max(1) {
        return Some(1 + distance);
    }
    let shorter = wanted_stem.len().min(candidate_stem.len());
    (shorter >= 3
        && (candidate_stem.contains(&wanted_stem) || wanted_stem.contains(&candidate_stem)))
    .then_some(100)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b = b.chars().collect::<Vec<_>>();
    let mut row = (0..=b.len()).collect::<Vec<_>>();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != *cb));
            diagonal = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// Up to [`MAX_SIMILAR_PATHS`] existing entries of `path`'s parent directory
/// whose names resemble `path`'s, closest first.
pub fn similar_in_directory(path: &Path) -> Vec<PathBuf> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let wanted = name.to_string_lossy();
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found = entries
        .flatten()
        .take(MAX_DIR_SCAN)
        .filter_map(|entry| {
            let candidate = entry.file_name().to_string_lossy().into_owned();
            name_distance(&wanted, &candidate).map(|distance| (distance, candidate, entry.path()))
        })
        .collect::<Vec<_>>();
    found.sort();
    found
        .into_iter()
        .take(MAX_SIMILAR_PATHS)
        .map(|(_, _, path)| path)
        .collect()
}

/// Rewrite an I/O failure from `tool` on `path` (named `shown` to the model)
/// as an error that names the path and what to do next. `similar` lists
/// existing paths to suggest when `path` is missing. Other errors pass
/// through unchanged.
pub fn explain_file_error(
    error: Error,
    path: &Path,
    shown: &str,
    tool: &str,
    similar: &[String],
) -> Error {
    let Error::Io(io) = &error else {
        return error;
    };
    let message = match io.kind() {
        ErrorKind::NotFound => {
            let mut message = format!("{shown} does not exist");
            if let Some(parent) = path.parent().filter(|parent| {
                !parent.as_os_str().is_empty() && !parent.exists() && parent.parent().is_some()
            }) {
                message.push_str(&format!(
                    " (its directory {} does not exist either)",
                    parent.display()
                ));
            }
            message.push('.');
            if !similar.is_empty() {
                message.push_str(&format!(" Similar paths: {}.", similar.join(", ")));
            }
            message.push_str(" Use glob or list_dir to find the right path.");
            message
        }
        ErrorKind::PermissionDenied => format!(
            "{tool} cannot access {shown}: permission denied. Check the file's permissions or ask the user; do not retry the same call."
        ),
        ErrorKind::InvalidData => format!(
            "{shown} is not UTF-8 text, so {tool} cannot change it. Inspect it with shell (for example `file` or `xxd`), or replace it entirely with write_file."
        ),
        ErrorKind::IsADirectory => {
            format!("{shown} is a directory, not a file; use list_dir or glob to see its contents.")
        }
        ErrorKind::NotADirectory => {
            format!("{shown}: part of this path is a file, not a directory.")
        }
        _ => format!("{tool} failed on {shown}: {io}"),
    };
    Error::InvalidRequest(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_names_rank_case_extension_and_typos() {
        assert_eq!(name_distance("Main.rs", "main.rs"), Some(0));
        assert_eq!(name_distance("main.ts", "main.tsx"), Some(1));
        assert_eq!(name_distance("utlis.rs", "utils.rs"), Some(3));
        assert_eq!(name_distance("config", "app_config.toml"), Some(100));
        assert_eq!(name_distance("main.rs", "README.md"), None);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn missing_files_name_the_path_and_nearby_names() {
        let root = std::env::temp_dir().join(format!("milim-file-errors-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("utils.rs"), "").unwrap();
        std::fs::write(root.join("util.py"), "").unwrap();
        std::fs::write(root.join("other.txt"), "").unwrap();
        let missing = root.join("utlis.rs");
        let similar = similar_in_directory(&missing);
        assert_eq!(similar, vec![root.join("utils.rs")]);
        assert_eq!(
            similar_in_directory(&root.join("UTIL.PY")),
            vec![root.join("util.py"), root.join("utils.rs")]
        );

        let error = std::fs::read(&missing).unwrap_err();
        let text = explain_file_error(
            error.into(),
            &missing,
            "src/utlis.rs",
            "read_file",
            &["src/utils.rs".into()],
        )
        .to_string();
        assert_eq!(
            text,
            "invalid request: src/utlis.rs does not exist. Similar paths: src/utils.rs. Use glob or list_dir to find the right path."
        );
        let deeper = root.join("nope/x.rs");
        let text = explain_file_error(
            std::fs::read(&deeper).unwrap_err().into(),
            &deeper,
            "nope/x.rs",
            "read_file",
            &[],
        )
        .to_string();
        assert!(text.contains("its directory"), "{text}");

        let utf8 =
            std::io::Error::new(ErrorKind::InvalidData, "stream did not contain valid UTF-8");
        let text = explain_file_error(utf8.into(), &missing, "a.bin", "edit_file", &[]).to_string();
        assert!(
            text.contains("a.bin is not UTF-8 text, so edit_file"),
            "{text}"
        );
        let other = explain_file_error(Error::Other("x".into()), &missing, "a", "t", &[]);
        assert_eq!(other.to_string(), "x");
        let _ = std::fs::remove_dir_all(root);
    }
}
