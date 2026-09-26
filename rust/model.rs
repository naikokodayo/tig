// SPDX-License-Identifier: GPL-2.0-or-later
// Safe Rust reimplementation of Tig's models; original Tig © 2006-2026 Jonas Fonseca.
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub oid: String,
    pub boundary: bool,
    pub parents: Vec<String>,
    pub author: String,
    pub date: String,
    pub author_email: String,
    pub committer: String,
    pub committer_email: String,
    pub committer_date: String,
    pub subject: String,
    pub decorations: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    pub oid: String,
    pub target: String,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub index: char,
    pub worktree: char,
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
}
impl StatusEntry {
    pub fn staged(&self) -> bool {
        !matches!(self.index, ' ' | '?' | '!')
    }
    pub fn conflicted(&self) -> bool {
        self.index == 'U'
            || self.worktree == 'U'
            || matches!((self.index, self.worktree), ('A', 'A') | ('D', 'D'))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: String,
    pub kind: String,
    pub oid: String,
    pub size: Option<u64>,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub oid: String,
    pub original_line: usize,
    pub line: usize,
    pub author: String,
    pub author_email: String,
    pub author_time: i64,
    pub author_tz: String,
    pub committer: String,
    pub committer_email: String,
    pub committer_time: i64,
    pub committer_tz: String,
    /// Path at the blamed commit; never an implicit worktree edit target.
    pub filename: PathBuf,
    /// Previous commit and path reported by Git, including renames.
    pub previous: Option<(String, PathBuf)>,
    pub summary: String,
    pub text: String,
}
