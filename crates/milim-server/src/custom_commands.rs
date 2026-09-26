//! User- and project-defined slash commands loaded from Markdown files.
//!
//! Project commands live in `<workspace>/.milim/commands` and
//! `<workspace>/.claude/commands`; user commands live in the same folders
//! under the home directory. Each `*.md` file is a prompt template with
//! optional frontmatter (`description`, `argument-hint`). Subdirectories
//! namespace commands as `dir:name`. Project commands win over user commands,
//! and `.milim` wins over `.claude` within the same scope.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

const COMMAND_DIRS: [&str; 2] = [".milim", ".claude"];
const MAX_COMMAND_BYTES: u64 = 64 * 1024;
const MAX_COMMAND_DEPTH: usize = 3;
const MAX_COMMANDS: usize = 500;
const MAX_DESCRIPTION_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CommandSource {
    Project,
    User,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CustomCommand {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub source: CommandSource,
    pub path: String,
    #[serde(skip)]
    pub template: String,
}

/// Discover commands for `workspace` (project scope) and `home` (user scope),
/// ordered by name. The first definition of a name wins.
pub(crate) fn discover(workspace: Option<&Path>, home: Option<&Path>) -> Vec<CustomCommand> {
    let mut commands = BTreeMap::new();
    let scopes = [
        (workspace, CommandSource::Project),
        (home, CommandSource::User),
    ];
    for (root, source) in scopes {
        let Some(root) = root else { continue };
        for dir in COMMAND_DIRS {
            let base = root.join(dir).join("commands");
            let mut found = Vec::new();
            collect_markdown(&base, &base, 0, &mut found);
            found.sort();
            for (name, path) in found {
                if commands.len() >= MAX_COMMANDS {
                    break;
                }
                if commands.contains_key(&name) {
                    continue;
                }
                let Some(command) = load(&name, &path, source) else {
                    continue;
                };
                commands.insert(name, command);
            }
        }
    }
    commands.into_values().collect()
}

/// Find one command by name using the same precedence as [`discover`].
pub(crate) fn find(
    workspace: Option<&Path>,
    home: Option<&Path>,
    name: &str,
) -> Option<CustomCommand> {
    let name = name.trim().trim_start_matches('/').to_ascii_lowercase();
    discover(workspace, home)
        .into_iter()
        .find(|command| command.name == name)
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn collect_markdown(base: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if depth + 1 < MAX_COMMAND_DEPTH {
                collect_markdown(base, &path, depth + 1, out);
            }
            continue;
        }
        let is_markdown = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
        if !is_markdown || !path.is_file() {
            continue;
        }
        if let Some(name) = command_name(base, &path) {
            out.push((name, path));
        }
    }
}

/// `review.md` -> `review`, `git/commit.md` -> `git:commit`. Names are
/// lowercased; segments with characters outside `[a-z0-9_-]` are skipped so
/// every name can be typed after `/`.
fn command_name(base: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(base).ok()?.with_extension("");
    let mut segments = Vec::new();
    for component in relative.components() {
        let segment = component.as_os_str().to_str()?.to_ascii_lowercase();
        let valid = !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            return None;
        }
        segments.push(segment);
    }
    (!segments.is_empty()).then(|| segments.join(":"))
}

fn load(name: &str, path: &Path, source: CommandSource) -> Option<CustomCommand> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_COMMAND_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let parsed = parse(&text);
    let description = parsed
        .description
        .or_else(|| {
            parsed
                .body
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| line.trim_start_matches('#').trim().to_string())
        })
        .map(|value| value.chars().take(MAX_DESCRIPTION_CHARS).collect())
        .unwrap_or_default();
    Some(CustomCommand {
        name: name.to_string(),
        description,
        argument_hint: parsed.argument_hint,
        source,
        path: path.display().to_string(),
        template: parsed.body,
    })
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct ParsedCommand {
    pub description: Option<String>,
    pub argument_hint: Option<String>,
    pub body: String,
}

/// Split optional `---` frontmatter from the template body. Only flat
/// `key: value` lines are read; other keys and nested YAML are ignored.
pub(crate) fn parse(text: &str) -> ParsedCommand {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parsed = ParsedCommand::default();
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        parsed.body = text.trim().to_string();
        return parsed;
    };
    let mut offset = 0;
    let mut closed = None;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            closed = Some(offset + line.len());
            break;
        }
        offset += line.len();
    }
    let Some(end) = closed else {
        parsed.body = text.trim().to_string();
        return parsed;
    };
    for line in rest[..offset].lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let value = unquote(value.trim());
        if value.is_empty() {
            continue;
        }
        match key.trim().to_ascii_lowercase().as_str() {
            "description" => parsed.description = Some(value),
            "argument-hint" | "argument_hint" => parsed.argument_hint = Some(value),
            _ => {}
        }
    }
    parsed.body = rest[end..].trim().to_string();
    parsed
}

fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// Fill a template: `$ARGUMENTS` becomes the whole argument string and
/// `$1`..`$9` become whitespace-separated positional arguments (double or
/// single quotes group words). When the template uses no placeholder, non-empty
/// arguments are appended after a blank line so they are never dropped.
pub(crate) fn expand(template: &str, arguments: &str) -> String {
    let arguments = arguments.trim();
    let positional = split_arguments(arguments);
    let mut out = String::with_capacity(template.len() + arguments.len());
    let mut used_placeholder = false;
    let mut rest = template;
    while let Some(index) = rest.find('$') {
        out.push_str(&rest[..index]);
        let tail = &rest[index + 1..];
        if let Some(after) = tail.strip_prefix("ARGUMENTS") {
            out.push_str(arguments);
            used_placeholder = true;
            rest = after;
            continue;
        }
        let digit = tail.chars().next().filter(|c| ('1'..='9').contains(c));
        if let Some(digit) = digit {
            let position = digit as usize - '1' as usize;
            out.push_str(positional.get(position).map(String::as_str).unwrap_or(""));
            used_placeholder = true;
            rest = &tail[1..];
            continue;
        }
        out.push('$');
        rest = tail;
    }
    out.push_str(rest);
    if !used_placeholder && !arguments.is_empty() {
        out.push_str("\n\n");
        out.push_str(arguments);
    }
    out
}

fn split_arguments(arguments: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut in_word = false;
    for c in arguments.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    values.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            None => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        values.push(current);
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "milim-custom-commands-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_frontmatter_subset_and_body() {
        let parsed = parse(
            "---\ndescription: \"Review a diff\"\nargument-hint: [file] [focus]\nallowed-tools: Bash(git:*)\nnested:\n  key: ignored\n---\n\nReview $1 with focus on $2.\n",
        );
        assert_eq!(parsed.description.as_deref(), Some("Review a diff"));
        assert_eq!(parsed.argument_hint.as_deref(), Some("[file] [focus]"));
        assert_eq!(parsed.body, "Review $1 with focus on $2.");

        let plain = parse("Just a prompt\n");
        assert_eq!(plain.description, None);
        assert_eq!(plain.body, "Just a prompt");

        let unterminated = parse("---\ndescription: x\nbody");
        assert_eq!(unterminated.description, None);
        assert!(unterminated.body.starts_with("---"));

        let crlf = parse("---\r\ndescription: 'Windows'\r\n---\r\nBody\r\n");
        assert_eq!(crlf.description.as_deref(), Some("Windows"));
        assert_eq!(crlf.body, "Body");
    }

    #[test]
    fn expands_arguments_and_positionals() {
        assert_eq!(
            expand("Fix issue $ARGUMENTS now", "  #42 in parser "),
            "Fix issue #42 in parser now"
        );
        assert_eq!(
            expand("Compare $1 to $2; ignore $3.", "\"src/a b.rs\" main"),
            "Compare src/a b.rs to main; ignore ."
        );
        assert_eq!(expand("Cost is $5 or $x", ""), "Cost is  or $x");
        assert_eq!(
            expand("Summarize the diff", "briefly"),
            "Summarize the diff\n\nbriefly"
        );
        assert_eq!(expand("Summarize the diff", ""), "Summarize the diff");
        assert_eq!(
            expand("All: $ARGUMENTS / first: $1", "a b"),
            "All: a b / first: a"
        );
    }

    #[test]
    fn discovers_namespaced_commands_with_project_precedence() {
        let workspace = temp_dir("workspace");
        let home = temp_dir("home");
        write(
            &workspace.join(".milim/commands/review.md"),
            "---\ndescription: Project milim review\n---\nReview $ARGUMENTS",
        );
        write(
            &workspace.join(".claude/commands/review.md"),
            "---\ndescription: Project claude review\n---\nOther",
        );
        write(
            &workspace.join(".claude/commands/git/commit.md"),
            "# Write a commit message\n\nUse conventional commits.",
        );
        write(&workspace.join(".milim/commands/notes.txt"), "ignored");
        write(&workspace.join(".milim/commands/bad name.md"), "ignored");
        write(
            &home.join(".milim/commands/review.md"),
            "---\ndescription: User review\n---\nUser",
        );
        write(
            &home.join(".claude/commands/Deploy.md"),
            "---\nargument-hint: <env>\n---\nDeploy to $1",
        );

        let commands = discover(Some(&workspace), Some(&home));
        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["deploy", "git:commit", "review"]);

        let review = commands.iter().find(|c| c.name == "review").unwrap();
        assert_eq!(review.source, CommandSource::Project);
        assert_eq!(review.description, "Project milim review");
        assert!(review.path.contains(".milim"));

        let commit = commands.iter().find(|c| c.name == "git:commit").unwrap();
        assert_eq!(commit.description, "Write a commit message");

        let deploy = commands.iter().find(|c| c.name == "deploy").unwrap();
        assert_eq!(deploy.source, CommandSource::User);
        assert_eq!(deploy.argument_hint.as_deref(), Some("<env>"));
        assert_eq!(expand(&deploy.template, "staging"), "Deploy to staging");

        let user_only = discover(None, Some(&home));
        let review = user_only.iter().find(|c| c.name == "review").unwrap();
        assert_eq!(review.source, CommandSource::User);
        assert_eq!(review.template, "User");

        assert_eq!(
            find(Some(&workspace), Some(&home), "/Git:Commit")
                .unwrap()
                .source,
            CommandSource::Project
        );
        assert!(find(Some(&workspace), Some(&home), "missing").is_none());

        std::fs::remove_dir_all(workspace).ok();
        std::fs::remove_dir_all(home).ok();
    }
}
