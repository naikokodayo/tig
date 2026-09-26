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
        let output = self.run_allow_nonzero(repo, confirmed, capture)?;
        if !output.status.success() {
            return Err(GitError(format!(
                "Command exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output)
    }

    /// Return a launched command's exit status while still reporting launch/I/O errors.
    pub fn run_allow_nonzero(
        &self,
        repo: &Repository,
        confirmed: bool,
        capture: bool,
    ) -> Result<Output> {
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
        let output = crate::trace::output(&mut command)
            .map_err(|e| GitError(format!("Could not execute command: {e}")))?;
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
    prepare_with_context(repo, command, revision, path, 0, selected_ref)
}

/// `Some("")` supplies the refs heading's empty selection; `None` means the
/// caller has no reference context and selection-only variables stay unsupported.
pub fn prepare_with_context(
    repo: &Repository,
    command: &str,
    revision: &str,
    path: &Path,
    line: usize,
    selected_ref: Option<&str>,
) -> Result<PreparedCommand> {
    if let Some(reference) = selected_ref.filter(|name| !name.is_empty() && *name != "HEAD") {
        // Selection names are data, never Git options. Validate the full name
        // before shortening it for commands such as `git checkout %(branch)`.
        if !reference.starts_with("refs/") {
            return Err(GitError("Expected a full selected reference name".into()));
        }
        repo.command(["check-ref-format", reference])?;
        let short = ["refs/heads/", "refs/tags/", "refs/remotes/"]
            .iter()
            .find_map(|prefix| reference.strip_prefix(prefix))
            .unwrap_or(reference);
        if short.starts_with('-') {
            return Err(GitError(
                "Selected reference must not start with '-'".into(),
            ));
        }
    }
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
    // A known refs selection can have an empty branch/tag. Keep that distinct
    // from a view that has not supplied reference context at all.
    if let Some(reference) = selected_ref {
        let branch = reference.strip_prefix("refs/heads/").unwrap_or("");
        let mut tag = reference.strip_prefix("refs/tags/").unwrap_or("");
        // Checkout prefers a same-named branch, and Git can resolve a short
        // tag as a pseudoref. Keep short display names only when unambiguous.
        if !tag.is_empty()
            && (repo
                .refs()?
                .iter()
                .any(|r| r.name == format!("refs/heads/{tag}"))
                || query(
                    repo,
                    &[
                        "rev-parse",
                        "--symbolic-full-name",
                        "--verify",
                        "--end-of-options",
                        tag,
                    ],
                ) != reference)
        {
            tag = reference;
            variables.insert("ref", reference.into());
        }
        variables.insert("branch", branch.into());
        variables.insert("tag", tag.into());
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
            let branch = &reference[remote.len() + 1..];
            if branch.starts_with('-') {
                return Err(GitError("Selected branch must not start with '-'".into()));
            }
            variables.insert("remote", remote.into());
            variables.insert("branch", branch.into());
        }
    }
    variables.insert("file", path.as_os_str().to_owned());
    variables.insert("lineno", line.to_string().into());
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
        repo.command([
            "-c",
            "user.name=Ref Fixture",
            "-c",
            "user.email=ref@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial fixture",
        ])
        .unwrap();
        repo.command(["tag", "v1"]).unwrap();
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
        assert_eq!(make(Some("refs/tags/v1")).unwrap().argv[1], "");
        assert_eq!(make(Some("")).unwrap().argv[1], "");
        for invalid in [
            "--help",
            "refs/heads/has space",
            "refs/heads/-danger",
            "refs/tags/-danger",
        ] {
            assert!(make(Some(invalid)).is_err(), "{invalid}");
        }
        // Git permits shell metacharacters in refnames; argv expansion must
        // preserve them as one literal argument, without invoking a shell.
        assert_eq!(
            make(Some("refs/heads/topic;literal")).unwrap().argv[1],
            "topic;literal"
        );
        let tag = prepare(
            &repo,
            "!echo %(tag) %(branch)",
            "selected-oid",
            Path::new(""),
            Some("refs/tags/v1"),
        )
        .unwrap();
        assert_eq!(tag.argv, ["echo", "v1", ""]);
        assert_eq!(
            make(Some("refs/remotes/origin/selected")).unwrap().argv[1],
            ""
        );
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
            // New Git rejects overlapping names in `remote add`; older configs
            // can still contain them. Build that legacy fixture directly.
            repo.command(["config", &format!("remote.{name}.url"), "/unused"])
                .unwrap();
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
        assert_eq!(
            make(Some("refs/remotes/upstream/nested/topic"))
                .unwrap()
                .argv[1],
            "topic"
        );
        assert!(make(Some("refs/remotes/upstream/-danger")).is_err());
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
        let editor = prepare_with_context(
            &repo,
            "!vim +%(lineno) %(file)",
            "HEAD",
            Path::new("space name"),
            52,
            None,
        )
        .unwrap();
        assert_eq!(editor.argv, ["vim", "+52", "space name"]);
        assert_eq!(
            command.run(&repo, true, true).unwrap().stdout,
            b"two words\n"
        );
        repo.command([
            "-c",
            "user.name=Ref Fixture",
            "-c",
            "user.email=ref@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ])
        .unwrap();
        repo.command(["branch", "v1"]).unwrap();
        let checkout = prepare(
            &repo,
            "@git checkout -q %(tag)",
            "HEAD",
            Path::new(""),
            Some("refs/tags/v1"),
        )
        .unwrap();
        assert_eq!(checkout.argv[3], "refs/tags/v1");
        checkout.run(&repo, true, true).unwrap();
        assert!(repo.command(["symbolic-ref", "--quiet", "HEAD"]).is_err());
        assert_eq!(
            prepare(
                &repo,
                "!echo %(ref)",
                "HEAD",
                Path::new(""),
                Some("refs/tags/v1"),
            )
            .unwrap()
            .argv[1],
            "refs/tags/v1"
        );
        let older = repo.revision("HEAD").unwrap();
        repo.command([
            "-c",
            "user.name=Ref Fixture",
            "-c",
            "user.email=ref@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "newer fixture",
        ])
        .unwrap();
        let newer = repo.revision("HEAD").unwrap();
        for name in ["HEAD", "ORIG_HEAD"] {
            let reference = format!("refs/tags/{name}");
            repo.command(["update-ref", &reference, &older]).unwrap();
            let display = prepare(
                &repo,
                "!echo %(tag)",
                &older,
                Path::new(""),
                Some(&reference),
            )
            .unwrap();
            assert_eq!(display.argv[1], reference.as_str());
            repo.command(["checkout", "--detach", "-q", &newer])
                .unwrap();
            repo.command(["update-ref", "ORIG_HEAD", &newer]).unwrap();
            let checkout = prepare(
                &repo,
                "@git checkout -q %(tag)",
                &older,
                Path::new(""),
                Some(&reference),
            )
            .unwrap();
            checkout.run(&repo, true, true).unwrap();
            assert_eq!(
                repo.revision("HEAD").unwrap(),
                older,
                "selected {reference}"
            );
            assert_eq!(checkout.argv[3], reference.as_str());
        }
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
    #[test]
    fn launched_nonzero_exit_is_distinct_from_spawn_failure() {
        let repo = Repository {
            root: std::env::temp_dir(),
            git_dir: std::env::temp_dir(),
            bare: false,
        };
        let mut request = PreparedCommand {
            argv: vec!["git".into(), "--invalid-option-for-tig-test".into()],
            silent: true,
            confirm: false,
            exit: false,
            echo: false,
            quick: false,
        };
        assert!(!request
            .run_allow_nonzero(&repo, false, true)
            .unwrap()
            .status
            .success());
        assert!(request.run(&repo, false, true).is_err());
        request.argv = vec!["tig-command-that-does-not-exist".into()];
        assert!(request.run_allow_nonzero(&repo, false, true).is_err());
    }
}
