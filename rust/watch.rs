// SPDX-License-Identifier: GPL-2.0-or-later
// Safe Rust replacement for Tig's change monitoring.
// Original Tig copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>.
use crate::git::{GitError, Repository, Result};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Watch {
    snapshot: Option<Vec<Vec<u8>>>,
    checked: Option<Instant>,
    retry: bool,
}

impl Watch {
    /// Establish a baseline before loading all affected views.
    /// A failed query keeps the previous baseline so changes are not lost.
    pub fn reset(&mut self, repo: &Repository, now: Instant) -> Result<()> {
        let snapshot = snapshot(repo)?;
        self.snapshot = Some(snapshot);
        self.retry = false;
        self.checked = Some(now);
        Ok(())
    }

    /// A view failed to reload; retry at the next deadline even if Git is unchanged.
    pub fn retry(&mut self) {
        self.retry = true;
    }

    /// Call from the existing input loop; zero disables periodic checks.
    /// Advance the deadline even on error to avoid retrying on every input tick.
    pub fn poll(&mut self, repo: &Repository, interval: Duration, now: Instant) -> Result<bool> {
        if interval.is_zero()
            || self
                .checked
                .is_some_and(|last| now.saturating_duration_since(last) < interval)
        {
            return Ok(false);
        }
        self.checked = Some(now);
        let next = snapshot(repo)?;
        let changed = self.retry || self.snapshot.as_ref().map_or(true, |old| old != &next);
        self.snapshot = Some(next);
        Ok(changed)
    }
}

fn snapshot(repo: &Repository) -> Result<Vec<Vec<u8>>> {
    // Git resolves packed refs, linked worktrees, and alternate ref backends.
    // Reading HEAD also distinguishes switching between branches at one commit.
    let mut state = vec![
        std::fs::read(repo.git_dir.join("HEAD")).map_err(|error| GitError(error.to_string()))?,
        repo.command(["for-each-ref", "--format=%(refname)%00%(objectname)"])?,
    ];
    // Dropping an older stash changes its reflog without moving refs/stash.
    if state[1]
        .split(|byte| *byte == b'\n')
        .any(|line| line.starts_with(b"refs/stash\0"))
    {
        state.push(repo.command(["reflog", "show", "--format=%H%x00%gs", "refs/stash", "--"])?);
    }
    if !repo.bare {
        state.push(repo.command([
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ])?);
        // Status alone misses edits to an already-modified tracked file. Disable
        // user diff programs/text conversion: monitoring must only read data.
        // ponytail: Git output is buffered like existing loaders; stream it if
        // large patch memory becomes a measured problem.
        for cached in [false, true] {
            let mut args = vec![
                "--no-optional-locks",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "--no-color",
            ];
            if cached {
                args.push("--cached");
            }
            args.push("--");
            state.push(repo.command(args)?);
        }
    }
    Ok(state)
}
