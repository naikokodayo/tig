// SPDX-License-Identifier: GPL-2.0-or-later
// Blame argument routing; original Tig © 2006-2026 Jonas Fonseca.
use crate::{
    config::Config,
    git::{GitError, Repository, Result},
};
use std::{ffi::OsString, path::PathBuf};

pub struct Invocation {
    pub revision: String,
    pub path: PathBuf,
    pub options: Vec<String>,
}

/// Only options which leave porcelain framing and Git's read-only boundary intact.
/// Never forward arbitrary flags: --contents, --textconv, --output and abbreviations
/// can read unrelated files, execute configured programs or replace our output.
fn option(arg: &str) -> bool {
    matches!(arg, "-w" | "--root" | "--reverse" | "--first-parent")
        || ["-C", "-M"].iter().any(|prefix| {
            arg.strip_prefix(prefix)
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        })
        || revision_option(arg)
        || arg
            .strip_prefix("-L")
            .is_some_and(|range| !range.is_empty())
}

fn revision_option(arg: &str) -> bool {
    arg == "--first-parent"
        || [
            "--since=",
            "--after=",
            "--until=",
            "--before=",
            "--max-age=",
            "--min-age=",
        ]
        .iter()
        .any(|prefix| {
            arg.strip_prefix(prefix)
                .is_some_and(|value| !value.is_empty())
        })
}

pub fn arguments(repo: &Repository, options: &[String]) -> Result<Vec<OsString>> {
    let mut result = Vec::new();
    let mut args = options.iter();
    while let Some(arg) = args.next() {
        if let Some(lower) = arg.strip_prefix('^') {
            result.push(format!("^{}", repo.revision(lower)?).into());
        } else if arg == "-L" {
            let value = args
                .next()
                .ok_or_else(|| GitError("-L requires a range".into()))?;
            result.push(format!("-L{value}").into());
        } else if arg == "--ignore-rev" || arg.starts_with("--ignore-rev=") {
            let revision = match arg.strip_prefix("--ignore-rev=") {
                Some(value) => value,
                None => args
                    .next()
                    .ok_or_else(|| GitError("--ignore-rev requires a revision".into()))?,
            };
            result.push(format!("--ignore-rev={}", repo.revision(revision)?).into());
        } else if option(arg) {
            result.push(arg.into());
        } else {
            return Err(GitError(format!("Unsupported blame option: {arg}")));
        }
    }
    Ok(result)
}

impl Invocation {
    pub fn parse(repo: &Repository, args: &[OsString], config: &Config) -> Result<Self> {
        let (before, files) = match args.iter().position(|arg| arg == "--") {
            Some(split) => (&args[..split], &args[split + 1..]),
            None if !args.is_empty() => (&args[..args.len() - 1], &args[args.len() - 1..]),
            None => (args, args),
        };
        if files.len() != 1 || files[0].is_empty() {
            return Err(GitError("Blame requires exactly one file".into()));
        }
        let absolute = repo.invocation.join(&files[0]);
        let path = absolute
            .strip_prefix(&repo.root)
            .map_err(|_| GitError("Blame path is outside the repository".into()))?
            .to_path_buf();
        let mut options = Vec::new();
        let mut bounds = Vec::new();
        let mut revision = String::new();
        let mut order = config.value("commit-order").unwrap_or("auto");
        let mut input = before.iter();
        while let Some(arg) = input.next() {
            let arg = arg.to_str().ok_or_else(|| {
                GitError("Non-UTF-8 blame options/revisions are not supported".into())
            })?;
            match arg {
                "--reverse" => order = "reverse",
                "--topo-order" | "--date-order" | "--author-date-order" => order = "default",
                "-L" | "--ignore-rev" => {
                    options.push(arg.to_owned());
                    options.push(
                        input
                            .next()
                            .ok_or_else(|| GitError(format!("{arg} requires a value")))?
                            .to_str()
                            .ok_or_else(|| GitError("Non-UTF-8 blame option value".into()))?
                            .to_owned(),
                    );
                }
                _ if revision_option(arg) => bounds.push(arg.to_owned()),
                _ if arg.starts_with('-') => options.push(arg.to_owned()),
                _ => {
                    // Git expands ranges, ^ exclusions, and shorthand expressions.
                    // --end-of-options keeps even malicious revision names inert.
                    let expanded =
                        repo.command(["rev-parse", "--revs-only", "--end-of-options", arg])?;
                    if expanded.is_empty() {
                        return Err(GitError(format!("Invalid blame revision: {arg}")));
                    }
                    for value in String::from_utf8_lossy(&expanded).lines() {
                        if let Some(lower) = value.strip_prefix('^') {
                            bounds.push(format!("^{}", repo.revision(lower)?));
                        } else if revision.is_empty() {
                            revision = repo.revision(value)?;
                        } else {
                            return Err(GitError(
                                "Blame requires at most one positive revision".into(),
                            ));
                        }
                    }
                }
            }
        }
        // C command-line blame flags replace configured blame-options, while
        // revision bounds and commit ordering are appended independently.
        if options.is_empty() {
            options = config
                .settings
                .get("blame-options")
                .cloned()
                .unwrap_or_default();
        }
        if order == "reverse" {
            options.push("--reverse".into());
        }
        options.extend(bounds);
        arguments(repo, &options)?;
        Ok(Self {
            revision,
            path,
            options,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_read_only_porcelain_compatible_flags() {
        for arg in [
            "-C",
            "-C20",
            "-M",
            "-M3",
            "-w",
            "--reverse",
            "--max-age=123",
            "-L1,4",
        ] {
            assert!(option(arg), "{arg}");
        }
        for arg in [
            "--",
            "--output=x",
            "--contents=x",
            "--textconv",
            "--porcelain",
            "--incremental",
            "-Cfoo",
            "--max-age=",
            "--rever",
            "--ignore-revs-file=x",
        ] {
            assert!(!option(arg), "{arg}");
        }
    }
}
