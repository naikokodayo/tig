// SPDX-License-Identifier: GPL-2.0-or-later
// Safe status revert and conflict-side checkout; original Tig © 2006-2026 Jonas Fonseca.
use crate::git::{GitError, Repository, Result};
use crate::model::StatusEntry;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevertAction {
    Unstaged,
    Ours,
    Theirs,
}
#[derive(Debug, PartialEq, Eq)]
pub enum RevertOutcome {
    Cancelled,
    Applied { backup: PathBuf, needs_stage: bool },
}
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    index: Vec<u8>,
    worktree: Option<(Vec<u8>, fs::Permissions)>,
}
#[derive(Debug)]
pub struct RevertPlan {
    root: PathBuf,
    path: PathBuf,
    action: RevertAction,
    before: Snapshot,
    target_present: bool,
}
fn error(message: impl Into<String>) -> GitError {
    GitError(message.into())
}
fn snapshot(repo: &Repository, path: &Path) -> Result<Snapshot> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(error("Expected one repository-relative file path"));
    }
    let mut absolute = repo.root.clone();
    for part in path.components() {
        absolute.push(part);
        match fs::symlink_metadata(&absolute) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(error("Revert through symlinks is unsupported"));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(error(e.to_string())),
        }
    }
    let worktree = match fs::symlink_metadata(&absolute) {
        Ok(meta) if meta.is_file() => Some((
            fs::read(&absolute).map_err(|e| error(e.to_string()))?,
            meta.permissions(),
        )),
        Ok(_) => return Err(error("Only regular files or missing files can be reverted")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(error(e.to_string())),
    };
    let index = repo.command([
        OsString::from("ls-files"),
        "--stage".into(),
        "-z".into(),
        "--".into(),
        path.as_os_str().to_owned(),
    ])?;
    Ok(Snapshot { index, worktree })
}
fn private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| error(e.to_string()))
}
impl RevertPlan {
    /// Read-only preparation. `staged` identifies the selected status section.
    /// Conflicts require an explicit side; a normal revert never guesses one.
    pub fn prepare(
        repo: &Repository,
        entry: &StatusEntry,
        staged: bool,
        action: RevertAction,
    ) -> Result<Self> {
        if staged {
            return Err(error("Cannot revert changes to staged files"));
        }
        if matches!(entry.index, '?' | '!') {
            return Err(error("Cannot revert changes to untracked files"));
        }
        if entry.conflicted() != (action != RevertAction::Unstaged) {
            return Err(error("Conflicts require an explicit ours/theirs choice"));
        }
        if !entry.conflicted() && entry.worktree == ' ' {
            return Err(error("Nothing to revert"));
        }
        let before = snapshot(repo, &entry.path)?;
        let wanted = match action {
            RevertAction::Unstaged => 0,
            RevertAction::Ours => 2,
            RevertAction::Theirs => 3,
        };
        let mut stages = [false; 4];
        for record in before.index.split_inclusive(|b| *b == 0) {
            let record = record
                .strip_suffix(&[0])
                .ok_or_else(|| error("Truncated index entry"))?;
            let tab = record
                .iter()
                .position(|b| *b == b'\t')
                .ok_or_else(|| error("Malformed index entry"))?;
            let fields = record[..tab].split(|b| *b == b' ').collect::<Vec<_>>();
            if fields.len() != 3
                || fields[2].len() != 1
                || !(b'0'..=b'3').contains(&fields[2][0])
                || record[tab + 1..] != *entry.path.as_os_str().as_encoded_bytes()
                || !matches!(fields[0], b"100644" | b"100755")
                || !matches!(fields[1].len(), 40 | 64)
                || !fields[1].iter().all(u8::is_ascii_hexdigit)
            {
                return Err(error(
                    "Unsupported index entry; only regular files can be reverted",
                ));
            }
            let stage = (fields[2][0] - b'0') as usize;
            if stages[stage] {
                return Err(error("Duplicate index stage"));
            }
            stages[stage] = true;
        }
        if stages.iter().all(|s| !s)
            || (entry.conflicted() && stages[0])
            || (!entry.conflicted() && (!stages[0] || stages[1..].iter().any(|s| *s)))
        {
            return Err(error("Index no longer matches the selected status"));
        }
        Ok(Self {
            root: repo.root.clone(),
            path: entry.path.clone(),
            action,
            before,
            target_present: stages[wanted],
        })
    }
    pub fn prompt(&self) -> String {
        let operation = match self.action {
            RevertAction::Unstaged => "Revert unstaged changes",
            RevertAction::Ours => "Use ours in the worktree",
            RevertAction::Theirs => "Use theirs in the worktree",
        };
        format!(
            "{operation} for {:?}{}? A recovery copy will be kept. [y/N] ",
            self.path,
            if !self.target_present {
                " (deletion)"
            } else {
                ""
            }
        )
    }
    /// Cancellation writes nothing. A confirmed operation keeps recovery data
    /// even on failure; `checkout-index` deliberately has no force flag.
    pub fn execute(self, repo: &Repository, confirmed: bool) -> Result<RevertOutcome> {
        if !confirmed {
            return Ok(RevertOutcome::Cancelled);
        }
        if repo.root != self.root || snapshot(repo, &self.path)? != self.before {
            return Err(error(
                "Index or worktree changed; refresh and confirm again",
            ));
        }
        let base = repo.git_dir.join("tig-revert");
        match fs::symlink_metadata(&base) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => (),
            Ok(_) => return Err(error("Unsafe recovery directory")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => private_dir(&base)?,
            Err(e) => return Err(error(e.to_string())),
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| error(e.to_string()))?
            .as_nanos();
        let backup = base.join(format!("{}-{stamp}", std::process::id()));
        private_dir(&backup)?;
        let result = (|| {
            fs::write(backup.join("index-entries"), &self.before.index)
                .map_err(|e| error(e.to_string()))?;
            fs::write(backup.join("README"), format!("Repository: {:?}\nPath: {:?}\nAction: {:?}\nOriginal index records: index-entries (git ls-files --stage -z format).\nOriginal worktree file, if present: worktree.\n", repo.root, self.path, self.action)).map_err(|e| error(e.to_string()))?;
            if self.before.worktree.is_some() {
                // Rename preserves bytes/mode and a concurrently open editor's
                // inode. Cross-filesystem backups fail before discarding data.
                fs::rename(repo.root.join(&self.path), backup.join("worktree"))
                    .map_err(|e| error(e.to_string()))?;
            }
            if self.target_present {
                let mut args = vec![OsString::from("checkout-index")];
                match self.action {
                    RevertAction::Unstaged => (),
                    RevertAction::Ours => args.push("--stage=2".into()),
                    RevertAction::Theirs => args.push("--stage=3".into()),
                }
                args.extend([OsString::from("--"), self.path.as_os_str().to_owned()]);
                repo.command(args)?;
            }
            Ok(RevertOutcome::Applied {
                backup: backup.clone(),
                needs_stage: self.action != RevertAction::Unstaged,
            })
        })();
        result.map_err(|e: GitError| error(format!("{e}; recovery data retained at {backup:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        root: PathBuf,
        repo: Repository,
    }
    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tig-revert-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            assert!(std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success());
            let repo = Repository::discover(&root).unwrap();
            repo.command(["config", "user.name", "Test"]).unwrap();
            repo.command(["config", "user.email", "test@example.invalid"])
                .unwrap();
            fs::write(root.join(name), b"base\n").unwrap();
            repo.command(["add", "--", name]).unwrap();
            repo.command(["commit", "-qm", "base"]).unwrap();
            Self { root, repo }
        }
        fn entry(&self, name: &str) -> StatusEntry {
            self.repo
                .status_filtered(&[], true)
                .unwrap()
                .into_iter()
                .find(|e| e.path == Path::new(name) && (!e.staged() || e.conflicted()))
                .unwrap()
        }
        fn index(&self) -> Vec<u8> {
            fs::read(self.repo.git_dir.join("index")).unwrap()
        }
        fn plan(&self, name: &str, action: RevertAction) -> RevertPlan {
            RevertPlan::prepare(&self.repo, &self.entry(name), false, action).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn backup(outcome: RevertOutcome) -> PathBuf {
        match outcome {
            RevertOutcome::Applied { backup, .. } => backup,
            RevertOutcome::Cancelled => panic!("unexpected cancellation"),
        }
    }
    #[test]
    fn cancellation_stale_state_and_normal_revert_preserve_index_and_recovery() {
        let f = Fixture::new(":(glob)*");
        let path = f.root.join(":(glob)*");
        fs::write(&path, b"staged\n").unwrap();
        f.repo.command(["add", "--", ":(glob)*"]).unwrap();
        fs::write(&path, b"unstaged\0bytes\n").unwrap();
        let plan = f.plan(":(glob)*", RevertAction::Unstaged);
        assert!(plan.prompt().contains("[y/N]"));
        let before = f.index();
        assert_eq!(
            plan.execute(&f.repo, false).unwrap(),
            RevertOutcome::Cancelled
        );
        assert_eq!(f.index(), before);
        assert_eq!(fs::read(&path).unwrap(), b"unstaged\0bytes\n");
        assert!(!f.repo.git_dir.join("tig-revert").exists());
        let plan = f.plan(":(glob)*", RevertAction::Unstaged);
        fs::write(&path, b"later edits\n").unwrap();
        assert!(plan.execute(&f.repo, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"later edits\n");
        assert!(!f.repo.git_dir.join("tig-revert").exists());
        let plan = f.plan(":(glob)*", RevertAction::Unstaged);
        let saved = backup(plan.execute(&f.repo, true).unwrap());
        assert_eq!(fs::read(saved.join("worktree")).unwrap(), b"later edits\n");
        assert_eq!(fs::read(&path).unwrap(), b"staged\n");
        assert_eq!(f.index(), before);
        fs::remove_file(&path).unwrap();
        let saved = backup(
            f.plan(":(glob)*", RevertAction::Unstaged)
                .execute(&f.repo, true)
                .unwrap(),
        );
        assert!(!saved.join("worktree").exists());
        assert_eq!(fs::read(&path).unwrap(), b"staged\n");
        assert_eq!(f.index(), before);
    }
    #[test]
    fn conflicts_require_choice_and_keep_original_worktree_and_stages() {
        for action in [RevertAction::Ours, RevertAction::Theirs] {
            let f = Fixture::new("file");
            f.repo.command(["checkout", "-qb", "side"]).unwrap();
            fs::write(f.root.join("file"), b"theirs\n").unwrap();
            f.repo.command(["commit", "-qam", "theirs"]).unwrap();
            f.repo.command(["checkout", "-q", "-"]).unwrap();
            fs::write(f.root.join("file"), b"ours\n").unwrap();
            f.repo.command(["commit", "-qam", "ours"]).unwrap();
            assert!(f.repo.command(["merge", "side"]).is_err());
            let entry = f.entry("file");
            assert!(entry.conflicted());
            assert!(RevertPlan::prepare(&f.repo, &entry, false, RevertAction::Unstaged).is_err());
            let original = fs::read(f.root.join("file")).unwrap();
            let stages = f
                .repo
                .command(["ls-files", "--stage", "-z", "--", "file"])
                .unwrap();
            let before = f.index();
            assert_eq!(
                f.plan("file", action).execute(&f.repo, false).unwrap(),
                RevertOutcome::Cancelled
            );
            assert_eq!(f.index(), before);
            let saved = backup(f.plan("file", action).execute(&f.repo, true).unwrap());
            assert_eq!(fs::read(saved.join("worktree")).unwrap(), original);
            assert_eq!(fs::read(saved.join("index-entries")).unwrap(), stages);
            let expected = if action == RevertAction::Ours {
                b"ours\n".as_slice()
            } else {
                b"theirs\n".as_slice()
            };
            assert_eq!(fs::read(f.root.join("file")).unwrap(), expected);
            assert_eq!(f.index(), before);
            assert_eq!(
                f.repo
                    .command(["ls-files", "--stage", "-z", "--", "file"])
                    .unwrap(),
                stages
            );
        }
    }
    #[test]
    fn conflict_deletion_is_explicit_and_stale_index_is_refused() {
        let f = Fixture::new("file");
        f.repo.command(["checkout", "-qb", "side"]).unwrap();
        fs::write(f.root.join("file"), b"theirs\n").unwrap();
        f.repo.command(["commit", "-qam", "theirs"]).unwrap();
        f.repo.command(["checkout", "-q", "-"]).unwrap();
        f.repo.command(["rm", "--", "file"]).unwrap();
        f.repo.command(["commit", "-qm", "ours deletion"]).unwrap();
        assert!(f.repo.command(["merge", "side"]).is_err());
        let plan = f.plan("file", RevertAction::Ours);
        let conflict_index = f.index();
        assert!(plan.prompt().contains("(deletion)"));
        let saved = backup(plan.execute(&f.repo, true).unwrap());
        assert_eq!(fs::read(saved.join("worktree")).unwrap(), b"theirs\n");
        assert!(!f.root.join("file").exists());
        assert_eq!(f.index(), conflict_index);
        f.repo.command(["reset", "--hard", "HEAD~1"]).unwrap();
        fs::write(f.root.join("file"), b"work\n").unwrap();
        let plan = f.plan("file", RevertAction::Unstaged);
        f.repo.command(["add", "--", "file"]).unwrap();
        let before = f.index();
        assert!(plan.execute(&f.repo, true).is_err());
        assert_eq!(f.index(), before);
        assert_eq!(fs::read(f.root.join("file")).unwrap(), b"work\n");
    }
    #[test]
    fn failed_checkout_retains_original_file_and_index() {
        let f = Fixture::new("file");
        fs::write(f.root.join("file"), b"recover me\n").unwrap();
        let plan = f.plan("file", RevertAction::Unstaged);
        let before = f.index();
        let oid = String::from_utf8(f.repo.command(["rev-parse", ":file"]).unwrap()).unwrap();
        let oid = oid.trim();
        fs::remove_file(
            f.repo
                .git_dir
                .join("objects")
                .join(&oid[..2])
                .join(&oid[2..]),
        )
        .unwrap();
        let failure = plan.execute(&f.repo, true).unwrap_err();
        assert!(failure.to_string().contains("recovery data retained"));
        let saved = fs::read_dir(f.repo.git_dir.join("tig-revert"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read(saved.join("worktree")).unwrap(), b"recover me\n");
        assert_eq!(f.index(), before);
    }
    #[test]
    fn invalid_targets_and_locked_index_worktree_revert_are_safe() {
        let f = Fixture::new("file");
        fs::write(f.root.join("file"), b"work\n").unwrap();
        let entry = f.entry("file");
        assert!(RevertPlan::prepare(&f.repo, &entry, true, RevertAction::Unstaged).is_err());
        for name in ["../outside", "/absolute", "", "."] {
            let mut invalid = entry.clone();
            invalid.path = name.into();
            assert!(RevertPlan::prepare(&f.repo, &invalid, false, RevertAction::Unstaged).is_err());
        }
        fs::write(f.root.join("untracked"), b"private\n").unwrap();
        assert!(RevertPlan::prepare(
            &f.repo,
            &f.entry("untracked"),
            false,
            RevertAction::Unstaged
        )
        .is_err());
        #[cfg(unix)]
        {
            fs::remove_file(f.root.join("file")).unwrap();
            std::os::unix::fs::symlink("untracked", f.root.join("file")).unwrap();
            assert!(RevertPlan::prepare(&f.repo, &entry, false, RevertAction::Unstaged).is_err());
            fs::remove_file(f.root.join("file")).unwrap();
            fs::write(f.root.join("file"), b"work\n").unwrap();
        }
        let plan = f.plan("file", RevertAction::Unstaged);
        // Worktree-only operations do not need or overwrite the index lock.
        fs::write(f.repo.git_dir.join("index.lock"), b"locked").unwrap();
        let saved = backup(plan.execute(&f.repo, true).unwrap());
        assert_eq!(fs::read(saved.join("worktree")).unwrap(), b"work\n");
        assert_eq!(fs::read(f.root.join("file")).unwrap(), b"base\n");
    }
}
