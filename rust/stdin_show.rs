// SPDX-License-Identifier: GPL-2.0-or-later
// Safe Rust migration of Tig's forwarded diff stdin.
// Original Tig copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>.

use crate::git::{
    run_with_input, validate_diff_options, GitError, HistoryOptions, Repository, Result,
};

/// Git owns revision/range parsing. Stdin is data, never another option source.
/// Keep it byte-for-byte intact, including non-UTF-8 refs and literal path lines.
fn validate_input(input: &[u8]) -> Result<()> {
    let mut paths = false;
    for line in input.split(|byte| *byte == b'\n') {
        if line.contains(&0) || (!paths && line.contains(&b'\r')) {
            return Err(GitError(
                "NUL and carriage returns are not supported in revision stdin".into(),
            ));
        }
        if !paths && line == b"--" {
            paths = true;
        } else if !paths && line.starts_with(b"-") {
            return Err(GitError(
                "Options are not allowed in revision stdin; pass them on the command line".into(),
            ));
        }
    }
    Ok(())
}

/// Run the existing Git pipe transport without decoding or shell-splitting input.
/// Arguments and stdin paths are relative to the original invocation directory.
pub fn show(
    repo: &Repository,
    arguments: &[String],
    input: &[u8],
    diff_options: &[String],
    width: usize,
) -> Result<Vec<u8>> {
    HistoryOptions::parse(arguments)?;
    validate_diff_options(diff_options)?;
    validate_input(input)?;
    let mut args = vec!["show".to_owned(), "--root".into()];
    args.extend(diff_options.iter().cloned());
    args.extend([
        "--no-ext-diff".into(),
        "--no-textconv".into(),
        "--no-show-signature".into(),
        "--no-color".into(),
        "--format=fuller".into(),
        format!("--stat={width}"),
        "--patch".into(),
    ]);
    args.extend(arguments.iter().cloned());
    run_with_input(&repo.invocation, args, Some(input))
}

/// Resolve header candidates together: blob prose may resemble a commit header.
/// Reuse the history parser and decoration policy with a bounded number of Git calls.
pub fn commits(repo: &Repository, ids: &[String]) -> Result<Vec<crate::model::Commit>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    if ids
        .iter()
        .any(|id| !matches!(id.len(), 40 | 64) || !id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(GitError("Expected full hexadecimal commit IDs".into()));
    }
    let input = ids.join("\n") + "\n";
    let checked = run_with_input(
        &repo.root,
        ["cat-file", "--batch-check=%(objectname) %(objecttype)"],
        Some(input.as_bytes()),
    )?;
    let mut valid = Vec::new();
    for line in checked.split(|byte| *byte == b'\n') {
        if let Some(oid) = line.strip_suffix(b" commit") {
            valid.extend_from_slice(oid);
            valid.push(b'\n');
        }
    }
    if valid.is_empty() {
        return Ok(Vec::new());
    }
    repo.history_from_stdin(&["--stdin".into(), "--no-walk".into()], &valid, "default")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_preserves_revision_and_literal_path_boundaries() {
        for input in [
            &b""[..],
            b"HEAD\n^HEAD~2\n",
            b"HEAD~2..HEAD",
            b"refs/heads/byte-\xff\n--\nspace name\n--output=literal\n",
            b"HEAD\n\n",
            b"HEAD\n--\npath\rname\n",
        ] {
            assert!(validate_input(input).is_ok(), "{input:?}");
        }
        for input in [
            &b"--output=stolen\nHEAD\n"[..],
            b"--format=evil\nHEAD\n",
            b"--not\nHEAD\n",
            b"HEAD\0ignored\n",
            b"HEAD\r\n",
            b"HEAD\n--\npath\0suffix\n",
        ] {
            assert!(validate_input(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn forwards_bytes_paths_and_large_input_without_mutating_repository() {
        use std::{
            fs,
            process::Command,
            time::{SystemTime, UNIX_EPOCH},
        };
        let directory = std::env::temp_dir().join(format!(
            "tig-show-stdin-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .arg(&directory)
            .status()
            .unwrap()
            .success());
        let repo = Repository::discover(&directory).unwrap();
        repo.command(["config", "user.name", "Tester"]).unwrap();
        repo.command(["config", "user.email", "test@example.com"])
            .unwrap();
        fs::create_dir(directory.join("sub")).unwrap();
        fs::write(directory.join("sub/space name"), "content\n").unwrap();
        fs::write(directory.join("other"), "other\n").unwrap();
        repo.command(["add", "--all"]).unwrap();
        repo.command(["-c", "commit.gpgsign=false", "commit", "-qm", "stdin test"])
            .unwrap();
        let before = repo.command(["ls-files", "--stage", "-z"]).unwrap();
        let oid = repo.revision("HEAD").unwrap();
        let metadata = commits(&repo, &[oid.clone(), "a".repeat(40), oid.clone()]).unwrap();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].oid, oid);
        assert!(commits(&repo, &["a".repeat(40)]).unwrap().is_empty());
        assert!(commits(&repo, &["--all".into()]).is_err());

        let args = ["--stdin".into()];
        let output = show(&repo, &args, b"HEAD\n--\nsub/space name\n", &[], 80).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("    stdin test\n"));
        assert!(output.contains("+++ b/sub/space name"));
        assert!(!output.contains("+++ b/other"));
        let sub = Repository::discover(directory.join("sub")).unwrap();
        assert_eq!(
            show(&sub, &args, b"HEAD\n--\nspace name\n", &[], 80).unwrap(),
            output.as_bytes()
        );
        let large = b"HEAD\n".repeat(10000);
        let all = show(&repo, &args, &large, &[], 80).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&all)
                .matches("\n    stdin test\n")
                .count(),
            1
        );
        #[cfg(unix)]
        {
            let oid = repo.revision("HEAD").unwrap();
            fs::write(
                repo.git_dir.join("packed-refs"),
                [oid.as_bytes(), b" refs/heads/byte-\xff\n"].concat(),
            )
            .unwrap();
            assert_eq!(show(&repo, &args, b"byte-\xff\n", &[], 80).unwrap(), all);
        }
        for input in [
            &b"--output=stolen\nHEAD\n"[..],
            b"HEAD\0ignored\n",
            b"missing-revision\n",
        ] {
            assert!(show(&repo, &args, input, &[], 80).is_err());
        }
        assert!(show(
            &repo,
            &["--stdin".into(), "--output=stolen".into()],
            b"HEAD\n",
            &[],
            80
        )
        .is_err());
        assert!(!directory.join("stolen").exists());
        assert_eq!(repo.command(["ls-files", "--stage", "-z"]).unwrap(), before);
        fs::remove_dir_all(directory).unwrap();
    }
}
