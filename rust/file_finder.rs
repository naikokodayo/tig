// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Safe Rust port of Tig ui.c file finder. SPDX-License-Identifier: GPL-2.0-or-later
use crate::{
    config::Config,
    git::{self, GitError, Repository},
    model::TreeEntry,
};

/// An immutable tree snapshot. Display strings are never used as Git paths.
pub struct FileFinder {
    pub revision: String,
    pub files: Vec<TreeEntry>,
    pub visible: Vec<usize>,
    pub selected: usize,
    pub query: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Input {
    Continue,
    Accept,
    Cancel,
}

impl FileFinder {
    pub fn load(repo: &Repository, revision: &str) -> git::Result<Self> {
        let revision = if revision.is_empty()
            || (matches!(revision.len(), 40 | 64) && revision.bytes().all(|b| b == b'0'))
        {
            "HEAD"
        } else {
            revision
        };
        // Git normally succeeds after warning about an ambiguous short ref.
        // Refuse that warning instead of silently showing the wrong tree.
        let output = crate::trace::output(
            std::process::Command::new("git")
                .current_dir(&repo.root)
                .args([
                    "-c",
                    "core.warnAmbiguousRefs=true",
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("{revision}^{{commit}}"),
                ])
                .env("LC_ALL", "C")
                .stdin(std::process::Stdio::null()),
        )
        .map_err(|error| GitError(error.to_string()))?;
        if !output.status.success() || !output.stderr.is_empty() {
            return Err(GitError(format!(
                "Cannot resolve file finder revision: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let revision = String::from_utf8(output.stdout)
            .map_err(|_| GitError("Invalid revision output".into()))?
            .trim()
            .to_owned();
        if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(GitError("Expected a full commit ID".into()));
        }
        let files = git::parse_tree(&repo.command([
            "ls-tree",
            "-r",
            "-z",
            "-l",
            "--full-tree",
            &revision,
        ])?)?
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .collect::<Vec<_>>();
        let visible = (0..files.len()).collect();
        Ok(Self {
            revision,
            files,
            visible,
            selected: 0,
            query: String::new(),
        })
    }

    /// Case-sensitive ordered character matching, as in C's incremental prompt.
    pub fn filter(&mut self, query: String) {
        let current = self.visible.get(self.selected).copied().unwrap_or(0);
        self.visible = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let bytes = entry.path.as_os_str().as_encoded_bytes();
                let mut remaining = bytes;
                for c in query.chars() {
                    let mut buf = [0; 4];
                    let needle = c.encode_utf8(&mut buf).as_bytes();
                    let pos = remaining
                        .windows(needle.len())
                        .position(|part| part == needle)?;
                    remaining = &remaining[pos + needle.len()..];
                }
                Some(index)
            })
            .collect();
        self.selected = self
            .visible
            .partition_point(|&index| index < current)
            .min(self.visible.len().saturating_sub(1));
        self.query = query;
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.visible.len().saturating_sub(1));
    }

    pub fn selected(&self) -> Option<&TreeEntry> {
        self.visible
            .get(self.selected)
            .map(|&index| &self.files[index])
    }

    pub fn input(&mut self, key: &str, config: &Config) -> Input {
        match key {
            "<Enter>" => {
                return if self.selected().is_some() {
                    Input::Accept
                } else {
                    Input::Cancel
                }
            }
            "<Esc>" => return Input::Cancel,
            "<Backspace>" => {
                let mut query = self.query.clone();
                if query.pop().is_none() {
                    return Input::Cancel;
                }
                self.filter(query);
                return Input::Continue;
            }
            _ => (),
        }
        match config
            .action("search", key)
            .and_then(|action| action.first())
            .map(String::as_str)
        {
            Some("find-next") => self.move_selection(1),
            Some("find-prev") => self.move_selection(-1),
            Some("back" | "parent" | "view-close" | "view-close-no-quit") => return Input::Cancel,
            _ => {
                let value = match key {
                    "<Space>" => " ",
                    "<Lt>" => "<",
                    _ => key,
                };
                if value.chars().count() == 1 && !value.chars().any(char::is_control) {
                    self.filter(format!("{}{value}", self.query));
                }
            }
        }
        Input::Continue
    }

    pub fn label(entry: &TreeEntry) -> String {
        // Escape controls and invalid bytes so distinct Git paths stay distinguishable.
        let mut label = String::new();
        for chunk in entry.path.as_os_str().as_encoded_bytes().utf8_chunks() {
            for c in chunk.valid().chars() {
                if c.is_control() || c == '\\' {
                    label.extend(c.escape_default());
                } else {
                    label.push(c);
                }
            }
            for byte in chunk.invalid() {
                use std::fmt::Write;
                let _ = write!(label, "\\x{byte:02x}");
            }
        }
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_git_snapshot_and_errors() {
        use std::{fs, path::PathBuf, process::Command};
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let dir = Fixture(std::env::temp_dir().join(format!(
                "tig-finder-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        fs::create_dir(&dir.0).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .arg(&dir.0)
            .status()
            .unwrap()
            .success());
        let repo = Repository::discover(&dir.0).unwrap();
        repo.command(["config", "user.name", "Finder test"])
            .unwrap();
        repo.command(["config", "user.email", "finder@example.invalid"])
            .unwrap();
        assert!(FileFinder::load(&repo, "HEAD").is_err());
        fs::create_dir(dir.0.join("sub")).unwrap();
        for name in ["--option", "sub/a b\t\n", "sub/é"] {
            fs::write(dir.0.join(name), name).unwrap();
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/nonexistent", dir.0.join("link")).unwrap();
        }
        repo.command(["add", "."]).unwrap();
        repo.command(["commit", "-qm", "files"]).unwrap();
        #[cfg(unix)]
        {
            use std::{ffi::OsString, os::unix::ffi::OsStringExt};
            let oid = repo.command(["rev-parse", "HEAD:--option"]).unwrap();
            repo.command(vec![
                OsString::from("update-index"),
                "--add".into(),
                "--cacheinfo".into(),
                "100644".into(),
                String::from_utf8(oid).unwrap().trim().into(),
                OsString::from_vec(b"bad-\xff".to_vec()),
            ])
            .unwrap();
            repo.command(["commit", "-qm", "raw path in tree"]).unwrap();
        }
        let finder = FileFinder::load(&repo, "HEAD").unwrap();
        for entry in &finder.files {
            assert!(!FileFinder::label(entry).chars().any(char::is_control));
            if entry.mode == "120000" {
                assert_eq!(repo.blob(&entry.oid).unwrap(), b"/nonexistent");
            } else if entry.path.as_os_str().as_encoded_bytes() == b"bad-\xff" {
                assert_eq!(repo.blob(&entry.oid).unwrap(), b"--option");
            } else {
                assert_eq!(
                    repo.blob(&entry.oid).unwrap(),
                    fs::read(dir.0.join(&entry.path)).unwrap()
                );
            }
        }
        let nested = Repository::discover(dir.0.join("sub")).unwrap();
        assert_eq!(
            FileFinder::load(&nested, "HEAD").unwrap().files.len(),
            finder.files.len()
        );
        assert_eq!(
            FileFinder::load(&repo, "HEAD~0").unwrap().revision,
            finder.revision
        );
        fs::write(dir.0.join("--option"), "changed").unwrap();
        let entry = finder
            .files
            .iter()
            .find(|e| e.path == std::path::Path::new("--option"))
            .unwrap();
        assert_eq!(repo.blob(&entry.oid).unwrap(), b"--option");
        repo.command(["branch", "ambiguous"]).unwrap();
        repo.command(["tag", "ambiguous"]).unwrap();
        assert!(FileFinder::load(&repo, "ambiguous").is_err());
        assert!(FileFinder::load(&repo, "refs/heads/ambiguous").is_ok());
        assert!(FileFinder::load(&repo, "--output=oops").is_err());
        assert!(FileFinder::load(&repo, "missing-ref").is_err());
        assert!(FileFinder::load(&repo, &entry.oid).is_err());
    }
    #[test]
    fn filtering_selection_and_lossless_labels() {
        let files = ["a/foo", "b/bar", "c/fóo", "d/foo"]
            .map(|path| TreeEntry {
                mode: "100644".into(),
                kind: "blob".into(),
                oid: "a".repeat(40),
                size: Some(1),
                path: path.into(),
            })
            .to_vec();
        let mut finder = FileFinder {
            revision: String::new(),
            visible: (0..4).collect(),
            files,
            selected: 2,
            query: String::new(),
        };
        finder.filter("fó".into());
        assert_eq!(finder.visible, [2]);
        finder.filter("f".into());
        assert_eq!(finder.selected, 1);
        finder.filter("missing".into());
        assert!(finder.selected().is_none());
        finder.move_selection(-1);
        finder.filter(String::new());
        finder.move_selection(isize::MAX);
        assert_eq!(finder.selected, 3);
        finder.filter("fo".into());
        assert_eq!(finder.visible, [0, 2, 3]);
        let config = Config::defaults();
        finder.filter(String::new());
        finder.selected = 0;
        assert_eq!(finder.input("<Backspace>", &config), Input::Cancel);
        finder.input("<C-N>", &config);
        assert_eq!(finder.selected, 1);
        finder.input("<C-P>", &config);
        assert_eq!(finder.selected, 0);
        finder.input("<Space>", &config);
        assert_eq!(finder.query, " ");
        assert_eq!(finder.input("<Enter>", &config), Input::Cancel);
        finder.input("<Backspace>", &config);
        assert_eq!(finder.input("<Enter>", &config), Input::Accept);
        assert_eq!(finder.input("<C-C>", &config), Input::Cancel);
        finder.filter("ff".into());
        assert!(finder.visible.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            finder.files[0].path = std::ffi::OsString::from_vec(b"a\t\n\\\xff".to_vec()).into();
            assert_eq!(FileFinder::label(&finder.files[0]), "a\\t\\n\\\\\\xff");
            assert_eq!(
                finder.files[0].path.as_os_str().as_encoded_bytes(),
                b"a\t\n\\\xff"
            );
        }
    }
}
