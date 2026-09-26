// SPDX-License-Identifier: GPL-2.0-or-later
// Safe argv expansion for Tig user commands; original Tig © 2006-2026 Jonas Fonseca.
use crate::{
    config,
    git::{GitError, Repository, Result},
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    process::{Command, Output, Stdio},
};

#[derive(Debug)]
pub struct PreparedCommand {
    pub argv: Vec<OsString>,
    pub silent: bool,
    pub confirm: bool,
    pub exit: bool,
    pub echo: bool,
    pub quick: bool,
}
impl PreparedCommand {
    /// Quoted, escaped argv for display only; never execute this string through a shell.
    pub fn display(&self) -> String {
        self.argv
            .iter()
            .map(|arg| format!("{arg:?}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Explicit confirmation must come from the UI; scripted '?' is not consent.
    /// The caller restores terminal mode before interactive, uncaptured commands.
    pub fn run(&self, repo: &Repository, confirmed: bool, capture: bool) -> Result<Output> {
        if self.confirm && !confirmed {
            return Err(GitError("Command requires confirmation".into()));
        }
        let executable = self
            .argv
            .first()
            .ok_or_else(|| GitError("No command arguments".into()))?;
        let mut command = Command::new(executable);
        command.args(&self.argv[1..]).current_dir(&repo.root);
        if capture {
            command.stdin(Stdio::null());
        } else {
            #[cfg(unix)]
            {
                let tty = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open("/dev/tty")
                    .map_err(|e| GitError(format!("Could not open controlling terminal: {e}")))?;
                command
                    .stdin(tty.try_clone().map_err(|e| GitError(e.to_string()))?)
                    .stdout(tty.try_clone().map_err(|e| GitError(e.to_string()))?)
                    .stderr(tty);
            }
            #[cfg(not(unix))]
            command
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        }
        let output = command
            .output()
            .map_err(|e| GitError(format!("Could not execute command: {e}")))?;
        if !output.status.success() {
            return Err(GitError(format!(
                "Command exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output)
    }
}
fn query(repo: &Repository, args: &[&str]) -> String {
    repo.command(args)
        .ok()
        .map(|bytes| {
            String::from_utf8_lossy(&bytes)
                .trim_end_matches('\n')
                .to_owned()
        })
        .unwrap_or_default()
}
/// Expand placeholders without a shell, keeping a filename with spaces one argv.
/// Unknown placeholders fail instead of unexpectedly changing command meaning.
pub fn prepare(
    repo: &Repository,
    command: &str,
    revision: &str,
    path: &Path,
    selected_ref: Option<&str>,
) -> Result<PreparedCommand> {
    let mut argv = config::words(command).map_err(GitError)?;
    if argv.is_empty() {
        return Err(GitError("No command arguments".into()));
    }
    let mut result = PreparedCommand {
        argv: vec![],
        silent: false,
        confirm: false,
        exit: false,
        echo: false,
        quick: false,
    };
    let first = argv[0].clone();
    let mut prefix = 0;
    for flag in first.chars() {
        match flag {
            '!' => (),
            '@' => result.silent = true,
            '?' => result.confirm = true,
            '<' => result.exit = true,
            '+' => result.echo = true,
            '>' => result.quick = true,
            _ => break,
        }
        prefix += flag.len_utf8();
    }
    if prefix == 0 {
        return Err(GitError("Expected command flags: ! @ ? < + >".into()));
    }
    argv[0] = first[prefix..].into();
    if argv[0].is_empty() {
        return Err(GitError("Missing command executable".into()));
    }
    let head = query(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let upstream = query(
        repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    let head_id = query(repo, &["rev-parse", "--verify", "HEAD"]);
    let remote = if head.is_empty() {
        String::new()
    } else {
        query(repo, &["config", "--get", &format!("branch.{head}.remote")])
    };
    let current = std::env::current_dir().map_err(|e| GitError(e.to_string()))?;
    let relative = current.strip_prefix(&repo.root).unwrap_or(Path::new(""));
    let prefix = if relative.as_os_str().is_empty() {
        OsString::new()
    } else {
        let mut s = relative.as_os_str().to_owned();
        s.push("/");
        s
    };
    let mut variables: BTreeMap<&str, OsString> = BTreeMap::new();
    for (key, value) in [
        ("commit", revision),
        (
            "ref",
            selected_ref
                .map(|name| {
                    name.strip_prefix("refs/heads/")
                        .or_else(|| name.strip_prefix("refs/tags/"))
                        .or_else(|| name.strip_prefix("refs/remotes/"))
                        .unwrap_or(name)
                })
                .unwrap_or(revision),
        ),
        ("repo:head", head.as_str()),
        ("repo:head-id", head_id.as_str()),
        ("repo:remote", remote.as_str()),
        ("repo:upstream", upstream.as_str()),
    ] {
        variables.insert(key, value.into());
    }
    // A remote branch or tag must never fall back to the checked-out branch.
    if let Some(branch) = selected_ref.and_then(|name| name.strip_prefix("refs/heads/")) {
        variables.insert("branch", branch.into());
    }
    // refs_select() sets the viewed head to this ref's OID. Other views need
    // their viewed-head context supplied before this variable can be supported.
    if selected_ref.is_some() {
        variables.insert("head", revision.into());
    }
    if let Some(reference) = selected_ref.and_then(|name| name.strip_prefix("refs/remotes/")) {
        let remotes = query(repo, &["remote"]);
        if let Some(remote) = remotes
            .lines()
            .filter(|name| {
                reference
                    .strip_prefix(name)
                    .is_some_and(|tail| tail.starts_with('/'))
            })
            .max_by_key(|name| name.len())
        {
            variables.insert("remote", remote.into());
        }
    }
    variables.insert("file", path.as_os_str().to_owned());
    variables.insert(
        "directory",
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .as_os_str()
            .to_owned(),
    );
    variables.insert("repo:prefix", prefix);
    variables.insert(
        "repo:cdup",
        "../".repeat(relative.components().count()).into(),
    );
    variables.insert("repo:git-dir", repo.git_dir.as_os_str().to_owned());
    variables.insert(
        "repo:worktree",
        query(repo, &["config", "--get", "core.worktree"]).into(),
    );
    variables.insert("repo:exec-dir", repo.root.as_os_str().to_owned());
    variables.insert(
        "repo:is-inside-work-tree",
        if repo.bare { "false" } else { "true" }.into(),
    );
    for arg in argv {
        result.argv.push(expand(&arg, &variables)?);
    }
    Ok(result)
}
fn expand(arg: &str, variables: &BTreeMap<&str, OsString>) -> Result<OsString> {
    let mut out = OsString::new();
    let mut rest = arg;
    while let Some(index) = rest.find('%') {
        out.push(&rest[..index]);
        rest = &rest[index..];
        if let Some(next) = rest.strip_prefix("%%") {
            out.push("%");
            rest = next;
            continue;
        }
        if let Some(next) = rest.strip_prefix("%(") {
            let end = next
                .find(')')
                .ok_or_else(|| GitError("Unclosed command variable".into()))?;
            let key = &next[..end];
            out.push(
                variables
                    .get(key)
                    .ok_or_else(|| GitError(format!("Unsupported command variable: {key}")))?,
            );
            rest = &next[end + 1..];
        } else {
            out.push("%");
            rest = &rest[1..];
        }
    }
    out.push(rest);
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_branch_never_uses_an_unrelated_checked_out_branch() {
        let root = std::env::temp_dir().join(format!("tig-command-context-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .arg(&root)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["symbolic-ref", "HEAD", "refs/heads/checked-out"])
            .status()
            .unwrap()
            .success());
        let repo = Repository {
            root: root.clone(),
            git_dir: root.join(".git"),
            bare: false,
        };
        let make = |name| {
            prepare(
                &repo,
                "!echo %(branch)",
                "selected-oid",
                Path::new(""),
                name,
            )
        };
        assert_eq!(
            make(Some("refs/heads/selected")).unwrap().argv[1],
            "selected"
        );
        assert!(make(None).is_err());
        assert!(make(Some("refs/tags/v1")).is_err());
        assert!(make(Some("refs/remotes/origin/selected")).is_err());
        assert_eq!(
            prepare(
                &repo,
                "!echo %(head)",
                "selected-oid",
                Path::new(""),
                Some("refs/heads/selected")
            )
            .unwrap()
            .argv[1],
            "selected-oid"
        );
        assert!(prepare(&repo, "!echo %(head)", "selected-oid", Path::new(""), None).is_err());
        for (selected, expected) in [
            ("refs/heads/topic", "topic"),
            ("refs/tags/v1", "v1"),
            ("refs/remotes/upstream/topic", "upstream/topic"),
        ] {
            assert_eq!(
                prepare(
                    &repo,
                    "!echo %(ref)",
                    "selected-oid",
                    Path::new(""),
                    Some(selected)
                )
                .unwrap()
                .argv[1],
                expected
            );
        }

        for name in ["origin", "upstream", "upstream/nested"] {
            repo.command(["remote", "add", name, "/unused"]).unwrap();
        }
        repo.command(["config", "branch.checked-out.remote", "origin"])
            .unwrap();
        let remote_command = |name| {
            prepare(
                &repo,
                "!echo %(remote) %(repo:remote)",
                "selected-oid",
                Path::new(""),
                name,
            )
        };
        let remote = remote_command(Some("refs/remotes/upstream/main")).unwrap();
        assert_eq!(remote.argv[1], "upstream");
        assert_eq!(remote.argv[2], "origin");
        assert_eq!(
            remote_command(Some("refs/remotes/upstream/nested/main"))
                .unwrap()
                .argv[1],
            "upstream/nested"
        );
        assert!(remote_command(Some("refs/remotes/upstreamish/main")).is_err());
        assert!(remote_command(Some("refs/heads/main")).is_err());
        assert!(remote_command(None).is_err());
        let command = prepare(&repo, "!echo 'two words'", "HEAD", Path::new(""), None).unwrap();
        assert_eq!(command.display(), "\"echo\" \"two words\"");
        assert_eq!(
            command.run(&repo, true, true).unwrap().stdout,
            b"two words\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn expansion_preserves_arguments_and_requires_known_variables() {
        let mut vars = BTreeMap::new();
        vars.insert("file", OsString::from("space name; echo no"));
        assert_eq!(
            expand("[%(file)] %% %(file)", &vars).unwrap(),
            OsString::from("[space name; echo no] % space name; echo no")
        );
        assert!(expand("%(unknown)", &vars).is_err());
        assert!(expand("%(file", &vars).is_err());
        let request = PreparedCommand {
            argv: vec!["must-not-execute".into()],
            silent: false,
            confirm: true,
            exit: false,
            echo: false,
            quick: false,
        };
        let repo = Repository {
            root: std::env::temp_dir(),
            git_dir: std::env::temp_dir(),
            bare: false,
        };
        assert!(request
            .run(&repo, false, true)
            .unwrap_err()
            .to_string()
            .contains("requires confirmation"));
    }
}
