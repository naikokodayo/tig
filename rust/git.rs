// SPDX-License-Identifier: GPL-2.0-or-later
// Safe Rust replacement for Tig repo/refdb/status/tree/blame/stage/io.
// Original Tig copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>.
use crate::model::{BlameLine, Commit, Reference, StatusEntry, TreeEntry};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug)]
pub struct GitError(pub String);
impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for GitError {}
pub type Result<T> = std::result::Result<T, GitError>;

#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub bare: bool,
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
fn path(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes.to_vec())
            .map(PathBuf::from)
            .map_err(|_| {
                GitError(
                    "Git returned a filename that cannot be represented on this platform".into(),
                )
            })
    }
}
fn trim_lf(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}
fn run<I, S>(cwd: &Path, args: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(cwd)
        .args(["--no-pager", "--literal-pathspecs", "-c", "color.ui=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| GitError(format!("Could not run git: {e}")))?;
    if !output.status.success() {
        return Err(GitError(format!(
            "git exited with {}: {}",
            output.status,
            text(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}
fn valid_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(GitError(
            "Expected a nonempty repository-relative path without '..'".into(),
        ));
    }
    Ok(())
}
impl Repository {
    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start.as_ref();
        let git_dir = path(trim_lf(&run(start, ["rev-parse", "--absolute-git-dir"])?))?;
        let bare = trim_lf(&run(start, ["rev-parse", "--is-bare-repository"])?) == b"true";
        let root = if bare {
            git_dir.clone()
        } else {
            path(trim_lf(&run(start, ["rev-parse", "--show-toplevel"])?))?
        };
        Ok(Self {
            root,
            git_dir,
            bare,
        })
    }
    pub fn command<I, S>(&self, args: I) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run(&self.root, args)
    }
    pub fn revision(&self, revision: &str) -> Result<String> {
        let spec = format!("{revision}^{{commit}}");
        Ok(text(trim_lf(&self.command([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &spec,
        ])?)))
    }
    pub fn history(&self, revisions: &[String], limit: usize) -> Result<Vec<Commit>> {
        let split = revisions
            .iter()
            .position(|a| a == "--")
            .unwrap_or(revisions.len());
        let filters = &revisions[..split];
        let mut has_revision = false;
        let mut expects_value = false;
        let mut end_options = false;
        for arg in filters {
            if expects_value {
                expects_value = false;
                continue;
            }
            if arg == "--end-of-options" {
                end_options = true;
                continue;
            }
            if end_options || !arg.starts_with('-') {
                has_revision = true;
                continue;
            }
            let (name, inline_value) = arg
                .split_once('=')
                .map_or((arg.as_str(), false), |(name, _)| (name, true));
            match name {
                "--since" | "--after" | "--until" | "--before" | "--author" | "--committer" |
                "--grep" | "--max-count" | "--skip" | "--min-parents" | "--max-parents" | "-n" => {
                    expects_value = !inline_value;
                }
                "--all" | "--branches" | "--tags" | "--remotes" | "--glob" | "--exclude" => {
                    if matches!(name, "--glob" | "--exclude") && !inline_value { expects_value = true; }
                    has_revision = true;
                }
                "--first-parent" | "--no-merges" | "--merges" | "--reverse" | "--topo-order" |
                "--date-order" | "--author-date-order" | "--ancestry-path" | "--full-history" |
                "--simplify-merges" | "--simplify-by-decoration" | "--dense" | "--sparse" |
                "--remove-empty" | "--all-match" | "--invert-grep" | "--regexp-ignore-case" |
                "--extended-regexp" | "--fixed-strings" | "--perl-regexp" | "-i" | "-E" | "-F" => {
                    if inline_value { return Err(GitError(format!("Unexpected value for history option {name}"))); }
                }
                _ if arg.strip_prefix("-n").or_else(|| arg.strip_prefix('-'))
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) => {}
                _ => return Err(GitError(format!("Unsupported history option {arg}: only revision, filtering, ordering and path options are allowed"))),
            }
        }
        if expects_value {
            return Err(GitError("Missing history option value".into()));
        }
        let mut args = vec![
            "log".to_owned(),
            "--topo-order".into(),
            "--no-show-signature".into(),
            "--decorate=full".into(),
            "--format=%H%x00%P%x00%aN%x00%aI%x00%s%x00%D%x00%aE%x00%cN%x00%cE%x00%cI".into(),
            "-z".into(),
        ];
        if limit > 0 {
            args.push(format!("--max-count={limit}"));
        }
        // An unborn default HEAD has no history. Ask Git to validate filters
        // against all refs with zero results, rather than hide command errors.
        let unborn = !has_revision && self.is_unborn()?;
        if unborn {
            args.extend(["--all".into(), "--max-count=0".into()]);
        }
        args.extend(filters.iter().cloned());
        args.push("--".into());
        if split < revisions.len() {
            args.extend(revisions[split + 1..].iter().cloned());
        }
        let result = parse_history(&self.command(args)?)?;
        Ok(if unborn { Vec::new() } else { result })
    }
    fn is_unborn(&self) -> Result<bool> {
        match self.revision("HEAD") {
            Ok(_) => Ok(false),
            Err(error) => {
                let branch = self.command(["symbolic-ref", "--quiet", "HEAD"])?;
                let branch = text(trim_lf(&branch));
                if self.refs()?.iter().any(|r| r.name == branch) {
                    return Err(error);
                }
                Ok(true)
            }
        }
    }
    pub fn refs(&self) -> Result<Vec<Reference>> {
        let bytes = self.command([
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(HEAD)",
        ])?;
        bytes
            .split(|b| *b == b'\n')
            .filter(|r| !r.is_empty())
            .map(|row| {
                let f: Vec<_> = row.split(|b| *b == 0).collect();
                if f.len() != 4 {
                    return Err(GitError("Malformed Git ref record".into()));
                }
                Ok(Reference {
                    name: text(f[0]),
                    oid: text(f[1]),
                    target: text(f[2]),
                    current: f[3] == b"*",
                })
            })
            .collect()
    }
    pub fn show(&self, revision: &str) -> Result<String> {
        let oid = self.revision(revision)?;
        Ok(text(&self.command([
            "show",
            "--no-ext-diff",
            "--no-textconv",
            "--no-show-signature",
            "--format=fuller",
            "--stat",
            "--patch",
            &oid,
            "--",
        ])?))
    }
    pub fn diff(&self, staged: bool, file: Option<&Path>) -> Result<String> {
        Ok(text(&self.diff_bytes(staged, file)?))
    }
    pub fn diff_bytes(&self, staged: bool, file: Option<&Path>) -> Result<Vec<u8>> {
        let mut args: Vec<OsString> = [
            "diff",
            "--no-relative",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--stat",
            "--patch",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        if staged {
            args.push("--cached".into());
        }
        args.push("--".into());
        if let Some(file) = file {
            valid_path(file)?;
            args.push(file.into());
        }
        self.command(args)
    }
    /// Human-readable branch state, matching Tig's status header.
    pub fn status_header(&self) -> Result<String> {
        let output = self.command([
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
        ])?;
        let output = text(&output);
        let field = |name: &str| output.lines().find_map(|line| line.strip_prefix(name));
        let oid = field("# branch.oid ").unwrap_or("");
        if oid == "(initial)" {
            return Ok("Initial commit".into());
        }
        let branch = field("# branch.head ").unwrap_or("");
        let markers = [
            (
                "rebase-apply/rebasing",
                Some("rebase-apply/head-name"),
                "Rebasing",
            ),
            (
                "rebase-apply/applying",
                Some("rebase-apply/head-name"),
                "Applying mailbox to",
            ),
            (
                "rebase-apply",
                Some("rebase-apply/head-name"),
                "Rebasing mailbox onto",
            ),
            (
                "rebase-merge/interactive",
                Some("rebase-merge/head-name"),
                "Interactive rebase",
            ),
            (
                "rebase-merge",
                Some("rebase-merge/head-name"),
                "Rebase merge",
            ),
            ("MERGE_HEAD", None, "Merging"),
            ("BISECT_LOG", None, "Bisecting"),
            ("HEAD", None, "On branch"),
        ];
        for (marker, name_file, prefix) in markers {
            if !self.git_dir.join(marker).exists() {
                continue;
            }
            let mut name = branch.to_owned();
            if let Some(file) = name_file {
                name = std::fs::read_to_string(self.git_dir.join(file))
                    .map_err(|e| GitError(format!("Cannot read operation state: {e}")))?
                    .trim()
                    .trim_start_matches("refs/heads/")
                    .to_owned();
            }
            if marker == "HEAD" && branch == "(detached)" {
                let refs = self.refs()?;
                let tag = refs.iter().find(|r| {
                    r.name.starts_with("refs/tags/") && (r.oid == oid || r.target == oid)
                });
                return Ok(format!(
                    "HEAD detached at {}",
                    tag.map(|r| r.name.trim_start_matches("refs/tags/"))
                        .unwrap_or(oid)
                ));
            }
            let mut header = format!("{prefix} {name}");
            if name_file.is_none() {
                if let (Some(upstream), Some(counts)) =
                    (field("# branch.upstream "), field("# branch.ab "))
                {
                    let mut counts = counts.split_whitespace();
                    let ahead = counts
                        .next()
                        .and_then(|s| s.strip_prefix('+'))
                        .and_then(|s| s.parse::<u64>().ok())
                        .ok_or_else(|| GitError("Malformed ahead count".into()))?;
                    let behind = counts
                        .next()
                        .and_then(|s| s.strip_prefix('-'))
                        .and_then(|s| s.parse::<u64>().ok())
                        .ok_or_else(|| GitError("Malformed behind count".into()))?;
                    let info = match (ahead, behind) {
                        (0, 0) => format!("Your branch is up-to-date with '{upstream}'."),
                        (a, 0) => format!("Your branch is ahead of '{upstream}' by {a} commit{}.", if a == 1 {""} else {"s"}),
                        (0, b) => format!("Your branch is behind '{upstream}' by {b} commit{}.", if b == 1 {""} else {"s"}),
                        (a, b) => format!("Your branch and '{upstream}' have diverged, and have {a} and {b} different commits each, respectively"),
                    };
                    header.push_str(". ");
                    header.push_str(&info);
                }
            }
            return Ok(header);
        }
        Ok("Not currently on any branch".into())
    }
    pub fn status(&self) -> Result<Vec<StatusEntry>> {
        parse_status(&self.command(["status", "--porcelain=v1", "-z", "--untracked-files=all"])?)
    }
    pub fn tree(&self, revision: &str, directory: &Path) -> Result<Vec<TreeEntry>> {
        let oid = self.revision(revision)?;
        let mut spec = OsString::from(format!("{oid}:"));
        if !directory.as_os_str().is_empty() {
            valid_path(directory)?;
            spec.push(directory);
        }
        let args: Vec<OsString> = vec!["ls-tree".into(), "-z".into(), "-l".into(), spec];
        let mut entries = parse_tree(&self.command(args)?)?;
        for entry in &mut entries {
            entry.path = directory.join(&entry.path);
        }
        entries.sort_by(|a, b| (a.kind != "tree", &a.path).cmp(&(b.kind != "tree", &b.path)));
        Ok(entries)
    }
    pub fn blob(&self, oid: &str) -> Result<Vec<u8>> {
        if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(GitError("Expected a full object ID".into()));
        }
        self.command(["cat-file", "blob", oid])
    }
    pub fn blame(&self, revision: Option<&str>, file: &Path) -> Result<Vec<BlameLine>> {
        valid_path(file)?;
        let mut args: Vec<OsString> = vec!["blame".into(), "--line-porcelain".into()];
        if let Some(revision) = revision {
            args.push(self.revision(revision)?.into());
        }
        args.push("--".into());
        args.push(file.into());
        parse_blame(&self.command(args)?)
    }
    fn entry_paths(entry: &StatusEntry) -> Result<Vec<OsString>> {
        let mut paths = vec![];
        for file in std::iter::once(&entry.path).chain(entry.original_path.iter()) {
            valid_path(file)?;
            paths.push(file.into());
        }
        Ok(paths)
    }
    pub fn stage(&self, entry: &StatusEntry) -> Result<()> {
        let mut args: Vec<OsString> = vec!["add".into(), "--all".into(), "--".into()];
        args.extend(Self::entry_paths(entry)?);
        self.command(args).map(|_| ())
    }
    pub fn unstage(&self, entry: &StatusEntry) -> Result<()> {
        // An unborn branch has no HEAD. rm --cached only updates its index.
        let has_head = !self.is_unborn()?;
        let mut args: Vec<OsString> = if has_head {
            vec!["reset".into(), "--quiet".into(), "HEAD".into(), "--".into()]
        } else {
            vec![
                "rm".into(),
                "--cached".into(),
                "--ignore-unmatch".into(),
                "--".into(),
            ]
        };
        args.extend(Self::entry_paths(entry)?);
        self.command(args).map(|_| ())
    }
}

fn records(bytes: &[u8]) -> Result<impl Iterator<Item = &[u8]>> {
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return Err(GitError("Truncated NUL-delimited Git output".into()));
    }
    Ok(bytes
        .split(|b| *b == 0)
        .take(bytes.iter().filter(|b| **b == 0).count()))
}
pub fn parse_status(bytes: &[u8]) -> Result<Vec<StatusEntry>> {
    let mut fields = records(bytes)?;
    let mut entries = Vec::new();
    while let Some(row) = fields.next() {
        if row.len() < 4 || row[2] != b' ' {
            return Err(GitError("Malformed Git status record".into()));
        }
        let original_path = if matches!(row[0], b'R' | b'C') || matches!(row[1], b'R' | b'C') {
            Some(path(
                fields
                    .next()
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| GitError("Missing rename source".into()))?,
            )?)
        } else {
            None
        };
        entries.push(StatusEntry {
            index: char::from(row[0]),
            worktree: char::from(row[1]),
            path: path(&row[3..])?,
            original_path,
        });
    }
    Ok(entries)
}
pub fn parse_tree(bytes: &[u8]) -> Result<Vec<TreeEntry>> {
    records(bytes)?
        .map(|row| {
            let tab = row
                .iter()
                .position(|b| *b == b'\t')
                .ok_or_else(|| GitError("Malformed tree record".into()))?;
            let header = text(&row[..tab]);
            let f: Vec<_> = header.split_whitespace().collect();
            if f.len() != 4 || row.len() == tab + 1 {
                return Err(GitError("Malformed tree fields".into()));
            }
            let size = if f[3] == "-" {
                None
            } else {
                Some(
                    f[3].parse()
                        .map_err(|_| GitError("Invalid blob size".into()))?,
                )
            };
            Ok(TreeEntry {
                mode: f[0].into(),
                kind: f[1].into(),
                oid: f[2].into(),
                size,
                path: path(&row[tab + 1..])?,
            })
        })
        .collect()
}
pub fn parse_history(bytes: &[u8]) -> Result<Vec<Commit>> {
    let f: Vec<_> = records(bytes)?.collect();
    if f.len() % 10 != 0 {
        return Err(GitError("Malformed history fields".into()));
    }
    Ok(f.chunks_exact(10)
        .map(|f| Commit {
            oid: text(f[0]),
            parents: text(f[1]).split_whitespace().map(str::to_owned).collect(),
            author: text(f[2]),
            date: text(f[3]),
            subject: text(f[4]),
            decorations: text(f[5]),
            author_email: text(f[6]),
            committer: text(f[7]),
            committer_email: text(f[8]),
            committer_date: text(f[9]),
        })
        .collect())
}
pub fn parse_blame(bytes: &[u8]) -> Result<Vec<BlameLine>> {
    let mut result = Vec::new();
    let mut current: Option<BlameLine> = None;
    for raw in bytes.split(|b| *b == b'\n') {
        if let Some(content) = raw.strip_prefix(b"\t") {
            let mut line = current
                .take()
                .ok_or_else(|| GitError("Blame content without header".into()))?;
            line.text = text(content);
            result.push(line);
            continue;
        }
        let row = text(raw);
        if let Some(line) = current.as_mut() {
            if let Some(value) = row.strip_prefix("author ") {
                line.author = value.into();
            }
            if let Some(value) = row.strip_prefix("summary ") {
                line.summary = value.into();
            }
        } else if !row.is_empty() {
            let f: Vec<_> = row.split_whitespace().collect();
            if !(f.len() == 3 || f.len() == 4)
                || !matches!(f[0].len(), 40 | 64)
                || !f[0].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(GitError("Malformed blame header".into()));
            }
            current = Some(BlameLine {
                oid: f[0].into(),
                original_line: f[1]
                    .parse()
                    .map_err(|_| GitError("Invalid blame line".into()))?,
                line: f[2]
                    .parse()
                    .map_err(|_| GitError("Invalid blame line".into()))?,
                author: String::new(),
                summary: String::new(),
                text: String::new(),
            });
        }
    }
    if current.is_some() {
        return Err(GitError("Truncated blame record".into()));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "tig-rust-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            run(&dir, ["init", "--quiet"]).unwrap();
            run(&dir, ["config", "user.name", "Test User"]).unwrap();
            run(&dir, ["config", "user.email", "test@example.invalid"]).unwrap();
            Self(dir)
        }
        fn repo(&self) -> Repository {
            Repository::discover(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn nul_parsers_preserve_paths_and_reject_truncation() {
        let rows = parse_status(b"R  new\nname\0old\tname\0?? :(glob)*\0").unwrap();
        assert_eq!(rows[0].path, Path::new("new\nname"));
        assert_eq!(
            rows[0].original_path.as_deref(),
            Some(Path::new("old\tname"))
        );
        assert_eq!(rows[1].path, Path::new(":(glob)*"));
        assert!(parse_status(b"R  new\0").is_err());
        assert!(parse_status(b"?? truncated").is_err());
        assert!(parse_status(b"").unwrap().is_empty());
        let tree = parse_tree(b"100644 blob abc 12\ta\tb\n\0").unwrap();
        assert_eq!(tree[0].path, Path::new("a\tb\n"));
        assert_eq!(tree[0].size, Some(12));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(
                parse_status(b"?? \xff\0").unwrap()[0]
                    .path
                    .as_os_str()
                    .as_bytes(),
                b"\xff"
            );
        }
    }
    #[test]
    fn branch_status_tracks_upstream_and_detached_tags() {
        let f = Fixture::new();
        let repo = f.repo();
        fs::write(f.0.join("file"), "one\n").unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command(["commit", "-qm", "one"]).unwrap();
        repo.command(["branch", "-M", "main"]).unwrap();
        assert_eq!(repo.status_header().unwrap(), "On branch main");
        repo.command(["branch", "baseline"]).unwrap();
        repo.command(["branch", "--set-upstream-to=baseline", "main"])
            .unwrap();
        assert_eq!(
            repo.status_header().unwrap(),
            "On branch main. Your branch is up-to-date with 'baseline'."
        );
        fs::write(f.0.join("file"), "two\n").unwrap();
        repo.command(["commit", "-qam", "two"]).unwrap();
        assert_eq!(
            repo.status_header().unwrap(),
            "On branch main. Your branch is ahead of 'baseline' by 1 commit."
        );
        repo.command(["tag", "v1"]).unwrap();
        repo.command(["checkout", "--detach", "v1"]).unwrap();
        assert_eq!(repo.status_header().unwrap(), "HEAD detached at v1");
    }
    #[test]
    fn real_repository_roundtrip_and_literal_staging() {
        let f = Fixture::new();
        let repo = f.repo();
        assert!(repo.history(&[], 0).unwrap().is_empty());
        assert_eq!(repo.status_header().unwrap(), "Initial commit");
        fs::write(f.0.join(":(glob)*"), "first\nsecond\n").unwrap();
        fs::write(f.0.join("other"), "keep\n").unwrap();
        let entry = repo
            .status()
            .unwrap()
            .into_iter()
            .find(|e| e.path == Path::new(":(glob)*"))
            .unwrap();
        repo.stage(&entry).unwrap();
        assert!(!repo
            .status()
            .unwrap()
            .iter()
            .find(|e| e.path == Path::new("other"))
            .unwrap()
            .staged());
        repo.unstage(&repo.status().unwrap()[0]).unwrap();
        assert!(f.0.join(":(glob)*").exists());
        repo.stage(&entry).unwrap();
        repo.command(["commit", "-m", "initial", "--quiet"])
            .unwrap();
        let history = repo.history(&[], 0).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].subject, "initial");
        assert_eq!(
            repo.history(&["--all".into(), "-n".into(), "1".into()], 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            repo.history(&["--".into(), ":(glob)*".into()], 0)
                .unwrap()
                .len(),
            1
        );
        assert!(repo
            .history(&["--".into(), "other".into()], 0)
            .unwrap()
            .is_empty());
        assert!(repo.history(&["--pretty=raw".into()], 0).is_err());
        assert!(repo.history(&["--stat".into()], 0).is_err());
        assert!(repo.history(&["-z".into()], 0).is_err());
        assert!(!repo.refs().unwrap().is_empty());
        let tree = repo.tree("HEAD", Path::new("")).unwrap();
        assert_eq!(repo.blob(&tree[0].oid).unwrap(), b"first\nsecond\n");
        let blame = repo.blame(Some("HEAD"), Path::new(":(glob)*")).unwrap();
        assert_eq!(blame.len(), 2);
        assert_eq!(blame[1].line, 2);
        assert!(repo.show("HEAD").unwrap().contains("initial"));
        fs::rename(f.0.join(":(glob)*"), f.0.join("renamed")).unwrap();
        repo.command(["add", "--all", "--", ":(glob)*", "renamed"])
            .unwrap();
        let rename = repo
            .status()
            .unwrap()
            .into_iter()
            .find(|e| e.index == 'R')
            .unwrap();
        repo.unstage(&rename).unwrap();
        assert!(f.0.join("renamed").exists());
        assert!(!f.0.join(":(glob)*").exists());
        assert!(repo.diff(false, None).unwrap().contains("deleted"));
        assert!(repo
            .stage(&StatusEntry {
                index: '?',
                worktree: '?',
                path: "../outside".into(),
                original_path: None
            })
            .is_err());
        assert!(repo.show("--output=oops").is_err());
        assert!(repo.history(&["--format=oops".into()], 1).is_err());
    }
}
