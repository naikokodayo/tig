// SPDX-License-Identifier: GPL-2.0-or-later
#![forbid(unsafe_code)]
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
use tig_rs::git;
use tig_rs::watch;

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Watch Test")
            .env("GIT_AUTHOR_EMAIL", "watch@example.invalid")
            .env("GIT_COMMITTER_NAME", "Watch Test")
            .env("GIT_COMMITTER_EMAIL", "watch@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn real_git_changes_and_deadlines() {
    let root = std::env::temp_dir().join(format!("tig-watch-{}", std::process::id()));
    fs::create_dir(&root).unwrap(); // Never reuse or erase an existing directory.
    let fixture = Fixture(root);
    fixture.git(&["init", "-q"]);
    let repo = git::Repository::discover(&fixture.0).unwrap();
    let mut watcher = watch::Watch::default();
    let mut now = Instant::now();
    let interval = Duration::from_secs(1);
    watcher.reset(&repo, now).unwrap();
    assert!(!watcher.poll(&repo, interval, now + interval).unwrap());
    now += interval;
    let mut changed = |watcher: &mut watch::Watch| {
        assert!(!watcher.poll(&repo, interval, now).unwrap());
        assert!(!watcher.poll(&repo, Duration::ZERO, now + interval).unwrap());
        now += interval;
        assert!(watcher.poll(&repo, interval, now).unwrap());
        now += interval;
        assert!(!watcher.poll(&repo, interval, now).unwrap());
    };
    fs::write(fixture.0.join("file"), "one\n").unwrap();
    changed(&mut watcher);
    fixture.git(&["add", "file"]);
    changed(&mut watcher);
    fixture.git(&["commit", "-qm", "base"]);
    changed(&mut watcher);
    fs::write(fixture.0.join("file"), "two\n").unwrap();
    changed(&mut watcher);
    fs::write(fixture.0.join("file"), "six\n").unwrap();
    changed(&mut watcher); // Same length and status, different content.
    fixture.git(&["stash", "push", "-qm", "saved"]);
    changed(&mut watcher);
    fixture.git(&["stash", "drop", "-q"]);
    changed(&mut watcher);
    for content in ["older stash\n", "newer stash\n"] {
        fs::write(fixture.0.join("file"), content).unwrap();
        fixture.git(&["stash", "push", "-qm", "saved"]);
        changed(&mut watcher);
    }
    fixture.git(&["stash", "drop", "-q", "stash@{1}"]);
    changed(&mut watcher); // Top stash OID is unchanged; the list is different.
    fixture.git(&["branch", "other"]);
    changed(&mut watcher);
    fixture.git(&["checkout", "-q", "other"]);
    changed(&mut watcher); // Same commit, different symbolic HEAD.
    fixture.git(&["tag", "nested/tag"]);
    changed(&mut watcher);
    fixture.git(&["pack-refs", "--all", "--prune"]);
    now += interval;
    assert!(!watcher.poll(&repo, interval, now).unwrap());
    watcher.retry();
    now += interval;
    assert!(watcher.poll(&repo, interval, now).unwrap());
    now += interval;
    assert!(watcher.poll(&repo, interval, now).unwrap());
    watcher.reset(&repo, now).unwrap();
    now += interval;
    assert!(!watcher.poll(&repo, interval, now).unwrap());
    let index = fs::read(repo.git_dir.join("index")).unwrap();
    watcher.reset(&repo, now).unwrap();
    assert_eq!(fs::read(repo.git_dir.join("index")).unwrap(), index);
    // Failure must not eat the old baseline and must not trigger a busy loop.
    let head = repo.git_dir.join("HEAD");
    let bytes = fs::read(&head).unwrap();
    fs::remove_file(&head).unwrap();
    now += interval;
    assert!(watcher.poll(&repo, interval, now).is_err());
    assert!(!watcher.poll(&repo, interval, now).unwrap());
    fs::write(head, bytes).unwrap();
    fixture.git(&["tag", "recovered"]);
    now += interval;
    assert!(watcher.poll(&repo, interval, now).unwrap());
}

#[test]
fn linked_worktree_and_bare_refs() {
    let root = std::env::temp_dir().join(format!("tig-watch-layouts-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let fixture = Fixture(root);
    fixture.git(&["init", "-q"]);
    fixture.git(&["commit", "--allow-empty", "-qm", "base"]);
    fixture.git(&["worktree", "add", "-qb", "linked", "linked"]);
    let linked = git::Repository::discover(fixture.0.join("linked")).unwrap();
    assert_ne!(linked.git_dir, fixture.0.join(".git"));
    let now = Instant::now();
    let interval = Duration::from_secs(1);
    let mut watcher = watch::Watch::default();
    watcher.reset(&linked, now).unwrap();
    fixture.git(&["tag", "shared-ref"]);
    assert!(watcher.poll(&linked, interval, now + interval).unwrap());
    fixture.git(&["clone", "--bare", ".", "bare.git"]);
    let bare = git::Repository::discover(fixture.0.join("bare.git")).unwrap();
    assert!(bare.bare);
    watcher.reset(&bare, now).unwrap();
    fixture.git(&["--git-dir=bare.git", "tag", "bare-ref"]);
    assert!(watcher.poll(&bare, interval, now + interval).unwrap());
}
