//! Conservative analysis of shell command lines for approval policy.
//!
//! Parsing is intentionally strict: anything this module cannot prove to be a
//! plain word sequence (substitution, redirection to files, backgrounding,
//! heredocs, variables in command position) is treated as an arbitrary
//! command. A `false` answer never means "dangerous", only "not proven safe".

/// The shell a command line will be handed to.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ShellDialect {
    /// `sh -c` semantics: backslash escapes outside single quotes.
    Posix,
    /// PowerShell: backslashes are literal path separators.
    PowerShell,
}

impl ShellDialect {
    /// The dialect the host shell tool uses on this platform.
    pub fn host() -> Self {
        if cfg!(windows) {
            Self::PowerShell
        } else {
            Self::Posix
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum Token {
    Word(String),
    /// `|`, `||`, `&&`, `;`, or a newline: separates independent segments.
    Separator,
}

/// Tokenize a command line. Returns `None` for any construct outside the
/// supported plain subset.
fn tokenize(command: &str, dialect: ShellDialect) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();

    fn flush(tokens: &mut Vec<Token>, word: &mut String, in_word: &mut bool) {
        if *in_word {
            tokens.push(Token::Word(std::mem::take(word)));
            *in_word = false;
        }
    }

    while let Some(ch) = chars.next() {
        match ch {
            '`' => return None,
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '`' => return None,
                        '$' if matches!(chars.peek(), Some('(') | Some('{')) => return None,
                        '\\' if dialect == ShellDialect::Posix => {
                            word.push(chars.next()?);
                        }
                        c => word.push(c),
                    }
                }
            }
            '\\' if dialect == ShellDialect::Posix => {
                in_word = true;
                match chars.next()? {
                    '\n' => {}
                    c => word.push(c),
                }
            }
            '$' if matches!(chars.peek(), Some('(') | Some('{')) => return None,
            '(' | ')' | '{' | '}' | '<' => return None,
            '>' => {
                // Only discarding output or merging stderr is accepted; any
                // other redirection may write a file.
                if in_word && (word == "1" || word == "2") {
                    word.clear();
                    in_word = false;
                } else {
                    flush(&mut tokens, &mut word, &mut in_word);
                }
                if chars.peek() == Some(&'&') {
                    chars.next();
                    match chars.next()? {
                        '1' | '2' => {}
                        _ => return None,
                    }
                } else {
                    while chars.peek() == Some(&' ') {
                        chars.next();
                    }
                    let target: String = std::iter::from_fn(|| {
                        chars.next_if(|c| !c.is_whitespace() && !"|&;".contains(*c))
                    })
                    .collect();
                    if !matches!(target.as_str(), "/dev/null" | "$null" | "nul" | "NUL") {
                        return None;
                    }
                }
            }
            '|' => {
                flush(&mut tokens, &mut word, &mut in_word);
                chars.next_if_eq(&'|');
                tokens.push(Token::Separator);
            }
            '&' => {
                flush(&mut tokens, &mut word, &mut in_word);
                chars.next_if_eq(&'&')?;
                tokens.push(Token::Separator);
            }
            ';' | '\n' => {
                flush(&mut tokens, &mut word, &mut in_word);
                tokens.push(Token::Separator);
            }
            c if c.is_whitespace() => flush(&mut tokens, &mut word, &mut in_word),
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    flush(&mut tokens, &mut word, &mut in_word);
    Some(tokens)
}

fn segments(tokens: Vec<Token>) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    for token in tokens {
        match token {
            Token::Word(word) => current.push(word),
            Token::Separator => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Words of a single simple command, or `None` when the line chains,
/// pipes, redirects, or substitutes.
pub fn simple_words(command: &str, dialect: ShellDialect) -> Option<Vec<String>> {
    let tokens = tokenize(command, dialect)?;
    if tokens.contains(&Token::Separator) {
        return None;
    }
    let words = segments(tokens).into_iter().next()?;
    Some(words)
}

/// Whether every segment of the command line is a known read-only
/// inspection command.
pub fn is_read_only(command: &str, dialect: ShellDialect) -> bool {
    let Some(tokens) = tokenize(command, dialect) else {
        return false;
    };
    let segments = segments(tokens);
    !segments.is_empty() && segments.iter().all(|words| segment_is_read_only(words))
}

/// Whether `command` is a single simple command whose leading words equal
/// the words of `prefix`. Both sides must parse as simple commands, so a
/// granted `cargo test` never covers `cargo test && rm -rf target`.
pub fn prefix_matches(prefix: &str, command: &str, dialect: ShellDialect) -> bool {
    let (Some(prefix), Some(command)) = (
        simple_words(prefix, dialect),
        simple_words(command, dialect),
    ) else {
        return false;
    };
    !prefix.is_empty() && command.len() >= prefix.len() && command[..prefix.len()] == prefix[..]
}

fn program_name(word: &str) -> String {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    base.to_ascii_lowercase()
}

fn has_flag(args: &[String], flags: &[&str]) -> bool {
    args.iter().any(|arg| {
        flags.iter().any(|flag| {
            arg == flag || (flag.starts_with("--") && arg.starts_with(&format!("{flag}=")))
        })
    })
}

fn positional(args: &[String]) -> Vec<&String> {
    args.iter().filter(|arg| !arg.starts_with('-')).collect()
}

fn segment_is_read_only(words: &[String]) -> bool {
    let Some(first) = words.first() else {
        return false;
    };
    if first.contains('$') || first.contains('=') {
        return false;
    }
    let args = &words[1..];
    match program_name(first).as_str() {
        "ls" | "dir" | "cat" | "head" | "tail" | "wc" | "pwd" | "echo" | "printf" | "which"
        | "where" | "whoami" | "uname" | "file" | "stat" | "du" | "df" | "grep" | "egrep"
        | "fgrep" | "diff" | "cmp" | "basename" | "dirname" | "realpath" | "readlink" | "cut"
        | "tr" | "nl" | "jq" | "true" | "false" | "id" | "seq" | "sha256sum" | "sha1sum"
        | "shasum" | "md5sum" | "md5" | "column" | "fold" | "type" | "get-childitem" | "gci"
        | "get-content" | "gc" | "select-string" | "sls" | "get-location" | "gl" | "test-path"
        | "resolve-path" | "get-item" | "gi" | "get-command" | "gcm" | "measure-object"
        | "measure" | "select-object" | "format-table" | "ft" | "format-list" | "fl"
        | "out-string" => true,
        "rg" => !has_flag(args, &["--pre"]),
        "sort" | "sort-object" => !has_flag(args, &["-o", "--output"]),
        "uniq" => positional(args).len() <= 1,
        "tree" => !has_flag(args, &["-o"]),
        "date" => !has_flag(args, &["-s", "--set"]),
        "hostname" => args.is_empty(),
        "find" => !has_flag(
            args,
            &[
                "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf",
                "-fls",
            ],
        ),
        "git" => git_is_read_only(args),
        _ => false,
    }
}

fn git_is_read_only(args: &[String]) -> bool {
    let mut rest = args;
    loop {
        match rest.first().map(String::as_str) {
            Some("--no-pager") => rest = &rest[1..],
            Some("-C") if rest.len() >= 2 => rest = &rest[2..],
            _ => break,
        }
    }
    let Some(sub) = rest.first() else {
        return false;
    };
    let args = &rest[1..];
    match sub.as_str() {
        "status" | "rev-parse" | "ls-files" | "ls-tree" | "blame" | "describe" | "shortlog"
        | "grep" | "cat-file" | "count-objects" | "merge-base" | "name-rev" | "for-each-ref"
        | "show-ref" | "rev-list" => true,
        "diff" | "log" | "show" => !has_flag(args, &["--output", "--ext-diff"]),
        "branch" => {
            !has_flag(
                args,
                &[
                    "-d",
                    "-D",
                    "-m",
                    "-M",
                    "-c",
                    "-C",
                    "-f",
                    "--delete",
                    "--move",
                    "--copy",
                    "--force",
                    "--set-upstream-to",
                    "-u",
                    "--unset-upstream",
                    "--edit-description",
                ],
            ) && positional(args).is_empty()
        }
        "tag" => {
            args.is_empty()
                || has_flag(args, &["-l", "--list"]) && !has_flag(args, &["-d", "--delete"])
        }
        "remote" => args.is_empty() || args.iter().all(|arg| arg == "-v" || arg == "--verbose"),
        "config" => has_flag(
            args,
            &["--get", "--get-all", "--list", "-l", "--get-regexp"],
        ),
        "stash" | "worktree" => matches!(args.first().map(String::as_str), Some("list")),
        "reflog" => args.is_empty() || matches!(args.first().map(String::as_str), Some("show")),
        "symbolic-ref" => positional(args).len() <= 1,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: ShellDialect = ShellDialect::Posix;

    #[test]
    fn read_only_commands_and_pipelines() {
        for command in [
            "ls -la",
            "git status",
            "git --no-pager log --oneline -20",
            "git -C crates diff HEAD~1",
            "rg -n 'fn main' crates | head -20",
            "cat Cargo.toml && git branch",
            "grep -r foo src 2>/dev/null",
            "find . -name '*.rs'",
            "git log 2>&1 | head",
            "wc -l src/*.rs",
        ] {
            assert!(is_read_only(command, P), "{command}");
        }
    }

    #[test]
    fn mutating_or_unprovable_commands() {
        for command in [
            "rm -rf target",
            "echo hi > out.txt",
            "cat a >> b",
            "git commit -m x",
            "git branch new-branch",
            "git -c core.pager=sh log",
            "find . -delete",
            "find . -exec rm {} ;",
            "ls $(pwd)",
            "ls `pwd`",
            "echo \"$(whoami)\"",
            "sort -o out a",
            "sleep 10 &",
            "$CMD status",
            "FOO=1 ls",
            "rg --pre ./x foo",
            "git diff --output=patch",
            "cat <<EOF",
            "sed -i s/a/b/ f",
            "ls; rm x",
            "",
        ] {
            assert!(!is_read_only(command, P), "{command}");
        }
    }

    #[test]
    fn powershell_paths_keep_backslashes() {
        let d = ShellDialect::PowerShell;
        assert!(is_read_only(r"Get-ChildItem C:\repo\src", d));
        assert_eq!(
            simple_words(r"type C:\a\b.txt", d).unwrap(),
            vec!["type".to_string(), r"C:\a\b.txt".to_string()]
        );
    }

    #[test]
    fn prefix_matching_requires_simple_commands() {
        assert!(prefix_matches("cargo test", "cargo test -p milim-tools", P));
        assert!(prefix_matches("cargo test", "cargo   test", P));
        assert!(!prefix_matches("cargo test", "cargo testing", P));
        assert!(!prefix_matches("cargo test", "cargo test && rm -rf /", P));
        assert!(!prefix_matches("cargo test", "cargo test; ls", P));
        assert!(!prefix_matches("cargo test", "cargo", P));
        assert!(!prefix_matches("", "cargo", P));
        assert!(prefix_matches(
            "npm run 'build app'",
            "npm run \"build app\" --x",
            P
        ));
    }
}
