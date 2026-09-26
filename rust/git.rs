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
    pub(crate) invocation: PathBuf,
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
    let mut command = Command::new("git");
    // Environment equivalents keep Git's global execution policy separate from
    // the subcommand argv, as in upstream's trace. Preserve inherited config.
    let count = std::env::var("GIT_CONFIG_COUNT")
        .unwrap_or_else(|_| "0".into())
        .parse::<usize>()
        .map_err(|_| GitError("Invalid GIT_CONFIG_COUNT".into()))?;
    command
        .current_dir(cwd)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env(
            "GIT_CONFIG_COUNT",
            count
                .checked_add(1)
                .ok_or_else(|| GitError("Invalid GIT_CONFIG_COUNT".into()))?
                .to_string(),
        )
        .env(format!("GIT_CONFIG_KEY_{count}"), "color.ui")
        .env(format!("GIT_CONFIG_VALUE_{count}"), "false")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    let output = crate::trace::output(&mut command)
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
// Both history decorations and the refs view use the same filtered ref records.
fn parse_remote_refs(bytes: &[u8], head: &str) -> Result<Vec<Reference>> {
    let input = std::str::from_utf8(bytes)
        .map_err(|_| GitError("TIG_LS_REMOTE returned non-UTF-8 refs".into()))?;
    let mut refs: Vec<Reference> = Vec::new();
    for line in input.lines().filter(|s| !s.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2
            || !matches!(fields[0].len(), 40 | 64)
            || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
            || !(fields[1] == "HEAD" || fields[1].starts_with("refs/"))
        {
            return Err(GitError("Malformed TIG_LS_REMOTE ref record".into()));
        }
        let (oid, name) = (fields[0], fields[1]);
        if name == "HEAD" && !head.is_empty() {
            continue;
        }
        if let Some(name) = name.strip_suffix("^{}") {
            let reference = refs
                .iter_mut()
                .find(|r| r.name == name)
                .ok_or_else(|| GitError("Peeled ref has no preceding tag".into()))?;
            reference.target = oid.into();
        } else {
            refs.push(Reference {
                name: name.into(),
                oid: oid.into(),
                target: String::new(),
                current: name == head || name == "HEAD",
            });
        }
    }
    Ok(refs)
}

fn decorate_history(commits: &mut [Commit], references: &[Reference], upstream: &str) {
    use crate::refs_view::{kind, numeric};
    let mut ordered: Vec<_> = references.iter().collect();
    ordered.sort_by(|a, b| {
        kind(a, upstream)
            .cmp(&kind(b, upstream))
            .then_with(|| numeric(&a.name, &b.name))
    });
    let mut decorations: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();
    for reference in ordered
        .iter()
        .filter(|r| !r.name.starts_with("refs/replace/"))
    {
        let oid = if reference.target.is_empty() {
            &reference.oid
        } else {
            &reference.target
        };
        let label = if reference.current {
            format!("HEAD -> {}", reference.name)
        } else if reference.name.starts_with("refs/tags/") {
            format!("tag: {}", reference.name)
        } else {
            reference.name.clone()
        };
        decorations.entry(oid).or_default().push(label);
    }
    for reference in ordered
        .iter()
        .filter(|r| r.name.starts_with("refs/replace/"))
    {
        let original = &reference.name["refs/replace/".len()..];
        let label = decorations
            .remove(original)
            .and_then(|v| v.into_iter().next())
            .map(|s| {
                s.trim_start_matches("HEAD -> ")
                    .trim_start_matches("refs/heads/")
                    .to_owned()
            })
            .unwrap_or_else(|| "replaced".into());
        decorations.insert(original, vec![format!("replace: {label}")]);
    }
    for commit in commits {
        commit.decorations = decorations
            .get(commit.oid.as_str())
            .map(|v| v.join(", "))
            .unwrap_or_default();
    }
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
/// Traversal flags shared by the history loader and main-view graph.
#[derive(Debug)]
pub struct HistoryOptions {
    pub with_graph: bool,
    first_parent: bool,
    merge: bool,
    has_revision: bool,
}
impl HistoryOptions {
    pub fn parse(revisions: &[String]) -> Result<Self> {
        let split = revisions
            .iter()
            .position(|a| a == "--")
            .unwrap_or(revisions.len());
        let filters = &revisions[..split];
        let mut options = Self {
            with_graph: true,
            first_parent: false,
            merge: false,
            has_revision: false,
        };
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
                options.has_revision = true;
                continue;
            }
            let (name, inline_value) = arg
                .split_once('=')
                .map_or((arg.as_str(), false), |(name, _)| (name, true));
            if matches!(name, "--follow" | "--no-merges" | "--author" | "--grep") {
                options.with_graph = false;
            }
            if name == "--merge" {
                options.merge = true;
            }
            if name == "--first-parent" {
                options.first_parent = true;
            }
            match name {
                "--since" | "--after" | "--until" | "--before" | "--author" | "--committer" |
                "--grep" | "--max-count" | "--skip" | "--min-parents" | "--max-parents" | "-n" => {
                    expects_value = !inline_value;
                }
                "--all" | "--branches" | "--tags" | "--remotes" | "--glob" | "--exclude" => {
                    if matches!(name, "--glob" | "--exclude") && !inline_value { expects_value = true; }
                    options.has_revision = true;
                }
                "--follow" | "--first-parent" | "--no-merges" | "--merges" | "--merge" | "--boundary" | "--reverse" | "--topo-order" |
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
        Ok(options)
    }
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
            invocation: start.canonicalize().map_err(|e| GitError(e.to_string()))?,
        })
    }
    /// Git returns an empty prefix when an explicit worktree is outside the cwd.
    pub fn prefix(&self) -> Result<PathBuf> {
        let prefix = path(trim_lf(&run(
            &self.invocation,
            ["rev-parse", "--show-prefix"],
        )?))?;
        if !prefix.as_os_str().is_empty() {
            valid_path(&prefix)?;
        }
        // Drop Git's trailing separator without converting filename bytes.
        Ok(prefix.components().collect())
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
    /// Paths after -- are root-relative; implicit paths use the discovery directory.
    pub fn history(&self, revisions: &[String], limit: usize) -> Result<Vec<Commit>> {
        self.history_ordered(revisions, limit, "topo")
    }
    pub fn history_ordered(
        &self,
        revisions: &[String],
        limit: usize,
        order: &str,
    ) -> Result<Vec<Commit>> {
        let options = HistoryOptions::parse(revisions)?;
        let order_arg = match order {
            "auto" | "topo" => Some("--topo-order"),
            "default" => None,
            "date" => Some("--date-order"),
            "author-date" => Some("--author-date-order"),
            "reverse" => Some("--reverse"),
            _ => return Err(GitError(format!("Invalid commit order: {order}"))),
        };
        let mut args = vec![
            "log".to_owned(),
            "--parents".into(),
            "--no-show-signature".into(),
            "--decorate=full".into(),
            "--format=%m%H%x00%P%x00%aN%x00%aI%x00%s%x00%D%x00%aE%x00%cN%x00%cE%x00%cI".into(),
            "-z".into(),
        ];
        if let Some(order_arg) = order_arg {
            args.push(order_arg.into());
        }
        if options.merge {
            args.push("--boundary".into());
        }
        if limit > 0 {
            args.push(format!("--max-count={limit}"));
        }
        // An unborn default HEAD has no history. Ask Git to validate filters
        // against all refs with zero results, rather than hide command errors.
        let unborn = !options.has_revision && self.is_unborn()?;
        if unborn {
            args.extend(["--all".into(), "--max-count=0".into()]);
        }
        // Preserve Git's revision/path disambiguation when no -- was supplied.
        args.extend(revisions.iter().cloned());
        let directory = if revisions.iter().any(|arg| arg == "--") {
            &self.root
        } else {
            &self.invocation
        };
        let mut result = parse_history(&run(directory, args)?)?;
        let references = self.refs()?;
        let upstream = self
            .command(["rev-parse", "--symbolic-full-name", "@{upstream}"])
            .map(|b| text(trim_lf(&b)))
            .unwrap_or_default();
        decorate_history(&mut result, &references, &upstream);
        if options.first_parent {
            for commit in &mut result {
                commit.parents.truncate(1);
            }
        }
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
        if let Some(command) = std::env::var_os("TIG_LS_REMOTE").filter(|s| !s.is_empty()) {
            let command = command
                .to_str()
                .ok_or_else(|| GitError("TIG_LS_REMOTE must be UTF-8".into()))?;
            let args = crate::config::words(command).map_err(GitError)?;
            let (program, args) = args
                .split_first()
                .ok_or_else(|| GitError("Empty TIG_LS_REMOTE command".into()))?;
            let output = Command::new(program)
                .args(args)
                .current_dir(&self.root)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| GitError(format!("Could not run TIG_LS_REMOTE: {e}")))?;
            if !output.status.success() {
                return Err(GitError(format!(
                    "TIG_LS_REMOTE exited with {}: {}",
                    output.status,
                    text(&output.stderr).trim()
                )));
            }
            let head = self
                .command(["symbolic-ref", "--quiet", "HEAD"])
                .map(|b| text(trim_lf(&b)))
                .unwrap_or_default();
            return parse_remote_refs(&output.stdout, &head);
        }
        let bytes = self.command([
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(HEAD)",
        ])?;
        let mut references: Vec<Reference> = bytes
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
            .collect::<Result<_>>()?;
        if self.command(["symbolic-ref", "--quiet", "HEAD"]).is_err() {
            if let Ok(oid) = self.revision("HEAD") {
                references.push(Reference {
                    name: "HEAD".into(),
                    oid,
                    target: String::new(),
                    current: true,
                });
            }
        }
        Ok(references)
    }
    pub fn show(
        &self,
        revision: &str,
        context: usize,
        word_diff: bool,
        file: Option<&Path>,
        width: usize,
    ) -> Result<String> {
        let oid = self.revision(revision)?;
        let mut args: Vec<OsString> = [
            "show",
            "--no-ext-diff",
            "--no-textconv",
            "--no-show-signature",
            "--format=fuller",
            &format!("--stat={width}"),
            "--patch",
            &format!("-U{context}"),
            if word_diff {
                "--word-diff=plain"
            } else {
                "--word-diff=none"
            },
            &oid,
            "--",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        if let Some(file) = file {
            valid_path(file)?;
            args.push(file.into());
        }
        Ok(text(&self.command(args)?))
    }
    pub fn diff(&self, staged: bool, file: Option<&Path>) -> Result<String> {
        Ok(text(&self.diff_bytes(staged, file)?))
    }
    pub fn diff_bytes(&self, staged: bool, file: Option<&Path>) -> Result<Vec<u8>> {
        self.diff_bytes_filtered(staged, file, &[])
    }
    pub fn diff_bytes_filtered(
        &self,
        staged: bool,
        file: Option<&Path>,
        filters: &[String],
    ) -> Result<Vec<u8>> {
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
        } else {
            // Literal query pathspecs stay after --; mutation paths are validated separately.
            args.extend(filters.iter().map(OsString::from));
        }
        self.command(args)
    }
    /// Read the worktree/index diff without overriding Git's configured prefixes.
    pub fn worktree_diff_bytes(&self, file: Option<&Path>) -> Result<Vec<u8>> {
        self.worktree_diff_bytes_filtered(file, &[])
    }
    pub fn worktree_diff_bytes_filtered(
        &self,
        file: Option<&Path>,
        filters: &[String],
    ) -> Result<Vec<u8>> {
        if let Some(file) = file {
            valid_path(file)?;
        }
        let mut args: Vec<OsString> = [
            "diff-files",
            "--patch-with-stat",
            "-C",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        // `diff-files` does not apply diff.noprefix by itself; Tig passes it explicitly.
        if self
            .command(["config", "--bool", "--get", "diff.noprefix"])
            .is_ok_and(|value| trim_lf(&value) == b"true")
        {
            args.push("--no-prefix".into());
        }
        args.push("--".into());
        if let Some(file) = file {
            args.push(file.into());
        } else {
            args.extend(filters.iter().map(OsString::from));
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
                match std::fs::read_to_string(self.git_dir.join(file)) {
                    Ok(value) if !value.trim().is_empty() => {
                        name = value.trim().trim_start_matches("refs/heads/").to_owned();
                    }
                    Ok(_) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => {
                        return Err(GitError(format!("Cannot read operation state: {error}")));
                    }
                }
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
    /// The status view reads each group from Git, including Git's path filtering.
    pub fn status_filtered(
        &self,
        paths: &[String],
        show_untracked: bool,
    ) -> Result<Vec<StatusEntry>> {
        let query = |args: &[&str]| {
            self.command(
                args.iter()
                    .copied()
                    .chain(std::iter::once("--"))
                    .chain(paths.iter().map(String::as_str)),
            )
        };
        let mut entries = if self.is_unborn()? {
            parse_status_paths(
                &query(&["ls-files", "-z", "--cached", "--exclude-standard"])?,
                'A',
            )?
        } else {
            parse_status_diff(
                &query(&[
                    "diff-index",
                    "-z",
                    "--diff-filter=ACDMRTXB",
                    "-C",
                    "--cached",
                    "HEAD",
                ])?,
                true,
            )?
        };
        entries.extend(parse_status_diff(&query(&["diff-files", "-z"])?, false)?);
        if show_untracked {
            entries.extend(parse_status_paths(
                &query(&["ls-files", "-z", "--others", "--exclude-standard"])?,
                '?',
            )?);
        }
        Ok(entries)
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
    pub fn blame(
        &self,
        revision: Option<&str>,
        file: &Path,
        lower_bound: Option<&str>,
    ) -> Result<Vec<BlameLine>> {
        valid_path(file)?;
        let mut args: Vec<OsString> = vec!["blame".into(), "--line-porcelain".into()];
        if let Some(revision) = revision {
            args.push(self.revision(revision)?.into());
        }
        if let Some(lower_bound) = lower_bound {
            args.push(format!("^{}", self.revision(lower_bound)?).into());
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
fn parse_status_paths(bytes: &[u8], index: char) -> Result<Vec<StatusEntry>> {
    records(bytes)?
        .map(|name| {
            let path = path(name)?;
            valid_path(&path)?;
            Ok(StatusEntry {
                index,
                worktree: ' ',
                path,
                original_path: None,
            })
        })
        .collect()
}

fn parse_status_diff(bytes: &[u8], staged: bool) -> Result<Vec<StatusEntry>> {
    let mut fields = records(bytes)?;
    let mut entries: Vec<StatusEntry> = Vec::new();
    while let Some(header) = fields.next() {
        let header =
            std::str::from_utf8(header).map_err(|_| GitError("Invalid diff header".into()))?;
        let fields_header: Vec<_> = header.split_whitespace().collect();
        if fields_header.len() != 5 || !fields_header[0].starts_with(':') {
            return Err(GitError("Malformed raw diff record".into()));
        }
        let mark = fields_header[4]
            .chars()
            .next()
            .ok_or_else(|| GitError("Missing diff status".into()))?;
        if !"ACDMRTUXB".contains(mark) {
            return Err(GitError("Invalid diff status".into()));
        }
        let mut next_path = || -> Result<PathBuf> {
            let name = fields
                .next()
                .ok_or_else(|| GitError("Missing diff path".into()))?;
            let name = path(name)?;
            valid_path(&name)?;
            Ok(name)
        };
        let first = next_path()?;
        let (path, original_path) = if matches!(mark, 'R' | 'C') {
            (next_path()?, Some(first))
        } else {
            (first, None)
        };
        // Git emits U followed by M for the same conflicted worktree path.
        if entries
            .last()
            .is_some_and(|entry| entry.conflicted() && entry.path == path)
        {
            continue;
        }
        entries.push(StatusEntry {
            index: if staged { mark } else { ' ' },
            worktree: if staged { ' ' } else { mark },
            path,
            original_path,
        });
    }
    Ok(entries)
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
            oid: text(f[0]).trim_start_matches(['-', '>', '<']).into(),
            boundary: f[0].starts_with(b"-"),
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
/// Read the header and first nonempty subject of Git's --pretty=raw stream.
/// Keep these records owned so column toggles can redraw without rereading stdin.
pub fn parse_raw_history(input: &str) -> Result<Vec<Commit>> {
    let mut commits: Vec<Commit> = Vec::new();
    let mut in_header = false;
    let valid_oid =
        |oid: &str| matches!(oid.len(), 40 | 64) && oid.bytes().all(|c| c.is_ascii_hexdigit());
    for line in input.lines() {
        if let Some(header) = line.strip_prefix("commit ") {
            let mut ids = header.split_whitespace();
            let oid = ids
                .next()
                .ok_or_else(|| GitError("Missing raw commit ID".into()))?;
            let boundary = oid.starts_with('-');
            let oid = oid.strip_prefix('-').unwrap_or(oid);
            if !valid_oid(oid) {
                return Err(GitError("Invalid raw commit ID".into()));
            }
            let parents: Vec<String> = ids.map(str::to_owned).collect();
            if !parents.iter().all(|id| valid_oid(id)) {
                return Err(GitError("Invalid raw parent ID".into()));
            }
            commits.push(Commit {
                oid: oid.into(),
                boundary,
                parents,
                author: String::new(),
                date: String::new(),
                author_email: String::new(),
                committer: String::new(),
                committer_email: String::new(),
                committer_date: String::new(),
                subject: String::new(),
                decorations: String::new(),
            });
            in_header = true;
            continue;
        }
        let Some(commit) = commits.last_mut() else {
            continue;
        };
        if line.is_empty() {
            in_header = false;
        } else if in_header {
            if let Some(parent) = line.strip_prefix("parent ") {
                if !valid_oid(parent) {
                    return Err(GitError("Invalid raw parent ID".into()));
                }
                if !commit.parents.iter().any(|id| id == parent) {
                    commit.parents.push(parent.into());
                }
            } else if let Some((kind, ident)) = line
                .split_once(' ')
                .filter(|(kind, _)| matches!(*kind, "author" | "committer"))
            {
                let (ident, time) = ident
                    .rsplit_once("> ")
                    .ok_or_else(|| GitError("Invalid raw author header".into()))?;
                let (name, email) = ident
                    .rsplit_once(" <")
                    .ok_or_else(|| GitError("Invalid raw author identity".into()))?;
                let date = crate::date::raw(time).map_err(GitError)?;
                if kind == "author" {
                    commit.author = name.into();
                    commit.author_email = email.into();
                    commit.date = date;
                } else {
                    commit.committer = name.into();
                    commit.committer_email = email.into();
                    commit.committer_date = date;
                }
            }
        } else if commit.subject.is_empty() {
            if let Some(subject) = line.strip_prefix("    ") {
                commit.subject = subject.trim_start().into();
            }
        }
    }
    if commits
        .iter()
        .any(|commit| commit.date.is_empty() || commit.committer_date.is_empty())
    {
        return Err(GitError(
            "Missing raw commit author or committer date".into(),
        ));
    }
    Ok(commits)
}

/// Decode Git's C-quoted pathname without treating it as a worktree path.
pub fn parse_git_path(raw: &[u8]) -> Result<PathBuf> {
    if !raw.starts_with(b"\"") {
        return path(raw);
    }
    let quoted = raw
        .strip_prefix(b"\"")
        .and_then(|s| s.strip_suffix(b"\""))
        .ok_or_else(|| GitError("Malformed quoted Git path".into()))?;
    let mut bytes = Vec::with_capacity(quoted.len());
    let mut i = 0;
    while i < quoted.len() {
        if quoted[i] != b'\\' {
            bytes.push(quoted[i]);
        } else {
            i += 1;
            let escaped = *quoted
                .get(i)
                .ok_or_else(|| GitError("Malformed quoted Git path".into()))?;
            match escaped {
                b'\\' | b'"' => bytes.push(escaped),
                b'a' => bytes.push(7),
                b'b' => bytes.push(8),
                b'f' => bytes.push(12),
                b't' => bytes.push(b'\t'),
                b'n' => bytes.push(b'\n'),
                b'r' => bytes.push(b'\r'),
                b'v' => bytes.push(11),
                b'0'..=b'7' => {
                    let digits = quoted
                        .get(i..i + 3)
                        .filter(|s| s.iter().all(|b| (b'0'..=b'7').contains(b)))
                        .ok_or_else(|| GitError("Malformed quoted Git path".into()))?;
                    let value = u16::from(digits[0] - b'0') * 64
                        + u16::from(digits[1] - b'0') * 8
                        + u16::from(digits[2] - b'0');
                    bytes.push(
                        u8::try_from(value)
                            .map_err(|_| GitError("Invalid Git path byte".into()))?,
                    );
                    i += 2;
                }
                _ => return Err(GitError("Malformed quoted Git path".into())),
            }
        }
        i += 1;
    }
    path(&bytes)
}

pub fn parse_blame(bytes: &[u8]) -> Result<Vec<BlameLine>> {
    let mut result = Vec::new();
    let mut current: Option<BlameLine> = None;
    for raw in bytes.split(|b| *b == b'\n') {
        if let Some(content) = raw.strip_prefix(b"\t") {
            let mut line = current
                .take()
                .ok_or_else(|| GitError("Blame content without header".into()))?;
            if line.filename.as_os_str().is_empty() {
                return Err(GitError("Blame record has no filename".into()));
            }
            line.text = text(content);
            result.push(line);
            continue;
        }
        let row = text(raw);
        if let Some(line) = current.as_mut() {
            if let Some(value) = row.strip_prefix("author ") {
                line.author = value.into();
            } else if let Some(value) = row.strip_prefix("author-mail ") {
                line.author_email = value.trim_start_matches('<').trim_end_matches('>').into();
            } else if let Some(value) = row.strip_prefix("author-time ") {
                line.author_time = value
                    .parse()
                    .map_err(|_| GitError("Invalid author time".into()))?;
            } else if let Some(value) = row.strip_prefix("author-tz ") {
                line.author_tz = value.into();
            } else if let Some(value) = row.strip_prefix("committer ") {
                line.committer = value.into();
            } else if let Some(value) = row.strip_prefix("committer-mail ") {
                line.committer_email = value.trim_start_matches('<').trim_end_matches('>').into();
            } else if let Some(value) = row.strip_prefix("committer-time ") {
                line.committer_time = value
                    .parse()
                    .map_err(|_| GitError("Invalid committer time".into()))?;
            } else if let Some(value) = row.strip_prefix("committer-tz ") {
                line.committer_tz = value.into();
            } else if let Some(value) = raw.strip_prefix(b"previous ") {
                let split = value
                    .iter()
                    .position(|b| *b == b' ')
                    .ok_or_else(|| GitError("Malformed blame previous record".into()))?;
                let oid = std::str::from_utf8(&value[..split])
                    .map_err(|_| GitError("Invalid blame parent ID".into()))?;
                if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(GitError("Invalid blame parent ID".into()));
                }
                let filename = parse_git_path(&value[split + 1..])?;
                valid_path(&filename)?;
                line.previous = Some((oid.into(), filename));
            } else if let Some(value) = raw.strip_prefix(b"filename ") {
                line.filename = parse_git_path(value)?;
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
                author_email: String::new(),
                author_time: 0,
                author_tz: String::new(),
                committer: String::new(),
                committer_email: String::new(),
                committer_time: 0,
                committer_tz: String::new(),
                filename: PathBuf::new(),
                previous: None,
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
    #[test]
    fn filtered_refs_feed_history_and_replacement_decorations() {
        let oid = "1".repeat(40);
        let tag = "2".repeat(40);
        let replaced = "3".repeat(40);
        let input = format!("{oid} HEAD\n{oid} refs/heads/main\n{oid} refs/heads/topic\n{tag} refs/tags/v1\n{oid} refs/tags/v1^{{}}\n{tag} refs/replace/{replaced}\n");
        let refs = parse_remote_refs(input.as_bytes(), "refs/heads/main").unwrap();
        assert_eq!(refs.len(), 4);
        assert!(refs[0].current);
        assert_eq!(refs[2].target, oid);
        let commit = |id: &str| {
            parse_history(format!("{id}\0\0Author\02020-01-01T00:00:00+00:00\0Title\0stale\0a@b\0Author\0a@b\02020-01-01T00:00:00+00:00\0").as_bytes()).unwrap().remove(0)
        };
        let mut commits = vec![commit(&oid), commit(&replaced)];
        decorate_history(&mut commits, &refs, "");
        assert_eq!(
            commits[0].decorations,
            "HEAD -> refs/heads/main, refs/heads/topic, tag: refs/tags/v1"
        );
        assert_eq!(commits[1].decorations, "replace: replaced");
        let remotes = parse_remote_refs(
            format!("{oid} refs/remotes/origin/HEAD\n{oid} refs/remotes/origin/main\n").as_bytes(),
            "",
        )
        .unwrap();
        decorate_history(&mut commits, &remotes, "refs/remotes/origin/main");
        assert_eq!(
            commits[0].decorations,
            "refs/remotes/origin/main, refs/remotes/origin/HEAD"
        );
        let mut filtered = parse_remote_refs(
            format!("{oid} refs/heads/topic\n{tag} refs/replace/{oid}\n").as_bytes(),
            "refs/heads/main",
        )
        .unwrap();
        decorate_history(&mut commits, &filtered, "");
        assert_eq!(commits[0].decorations, "replace: topic");
        filtered.clear();
        decorate_history(&mut commits, &filtered, "");
        assert!(commits.iter().all(|c| c.decorations.is_empty()));
        assert!(parse_remote_refs(b"invalid refs/heads/main", "").is_err());
        assert!(parse_remote_refs(format!("{oid} refs/tags/v1^{{}}").as_bytes(), "").is_err());
        assert!(parse_remote_refs(&[0xff], "").is_err());
        assert!(parse_remote_refs(format!("{oid} HEAD").as_bytes(), "").unwrap()[0].current);
    }

    #[test]
    fn history_option_values_and_paths_do_not_become_graph_flags() {
        for args in [
            vec!["--committer", "--no-merges"],
            vec!["--committer=--follow"],
            vec!["--", "--follow", "--first-parent"],
            vec!["--end-of-options", "--follow", "--first-parent"],
        ] {
            let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            let options = HistoryOptions::parse(&args).unwrap();
            assert!(options.with_graph, "{args:?}");
            assert!(!options.first_parent, "{args:?}");
        }
        let options = HistoryOptions::parse(&["--grep".into(), "--first-parent".into()]).unwrap();
        assert!(!options.with_graph);
        assert!(!options.first_parent);
        assert!(
            HistoryOptions::parse(&["--merge".into()])
                .unwrap()
                .with_graph
        );
        assert!(HistoryOptions::parse(&["--merge=oops".into()]).is_err());
        assert!(HistoryOptions::parse(&["--follow=yes".into()]).is_err());
        assert!(HistoryOptions::parse(&["--format=oops".into()]).is_err());
    }
    #[test]
    fn configured_commit_order_changes_real_history() {
        let fixture = Fixture::new();
        assert!(Command::new("tar")
            .args([
                "-xzf",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/test/main/commit-order-edge-case-test.tgz"
                ),
                "-C"
            ])
            .arg(&fixture.0)
            .status()
            .unwrap()
            .success());
        let repo = fixture.repo();
        let subjects = |order| {
            repo.history_ordered(&[], 0, order)
                .unwrap()
                .into_iter()
                .map(|commit| commit.subject)
                .collect::<Vec<_>>()
        };
        assert_eq!(subjects("topo")[1..3], ["More featuresA", "More master"]);
        assert_eq!(subjects("date")[1..3], ["More master", "More featuresA"]);
        assert!(repo.history_ordered(&[], 0, "other").is_err());
    }
    #[test]
    fn blame_porcelain_keeps_dates_and_historical_path() {
        let oid = "a".repeat(40);
        let raw = format!("{oid} 2 1 1\nauthor A\nauthor-mail <a@example.test>\nauthor-time 0\nauthor-tz -0200\ncommitter C\ncommitter-mail <c@example.test>\ncommitter-time 3600\ncommitter-tz +0100\nsummary Subject\nprevious {oid} \"parent\\tname\"\nfilename \"old\\tname\"\n\tcontent\n");
        let lines = parse_blame(raw.as_bytes()).unwrap();
        assert_eq!(lines[0].author_email, "a@example.test");
        assert_eq!(lines[0].author_time, 0);
        assert_eq!(lines[0].author_tz, "-0200");
        assert_eq!(lines[0].committer, "C");
        assert_eq!(lines[0].committer_time, 3600);
        assert_eq!(lines[0].committer_tz, "+0100");
        assert_eq!(lines[0].filename, Path::new("old\tname"));
        assert_eq!(
            lines[0].previous,
            Some((oid.clone(), PathBuf::from("parent\tname")))
        );
        for previous in [
            "not-an-id file".to_owned(),
            format!("{oid} ../outside"),
            format!("{oid} "),
        ] {
            assert!(parse_blame(
                format!("{oid} 1 1\nprevious {previous}\nfilename file\n\tcontent\n").as_bytes()
            )
            .is_err());
        }
        assert!(parse_git_path(b"\"bad\\777\"").is_err());
        assert!(parse_blame(format!("{oid} 1 1\n\tmissing path\n").as_bytes()).is_err());
    }
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
    fn filtered_status_preserves_paths_renames_and_conflicts() {
        let f = Fixture::new();
        let repo = f.repo();
        fs::create_dir(f.0.join("sub")).unwrap();
        fs::write(f.0.join("sub/old\nname"), "base\n").unwrap();
        fs::write(f.0.join("other"), "base\n").unwrap();
        repo.command(["add", "."]).unwrap();
        let initial = repo.status_filtered(&["sub".into()], true).unwrap();
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].index, 'A');
        repo.command(["commit", "-qm", "base"]).unwrap();
        repo.command(["mv", "sub/old\nname", "sub/new\tname"])
            .unwrap();
        fs::write(f.0.join("sub/untracked"), "new\n").unwrap();
        fs::write(f.0.join("other"), "changed\n").unwrap();
        let index = fs::read(repo.git_dir.join("index")).unwrap();
        let entries = repo.status_filtered(&["sub".into()], true).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].index, 'R');
        assert_eq!(entries[0].path, Path::new("sub/new\tname"));
        assert_eq!(
            entries[0].original_path.as_deref(),
            Some(Path::new("sub/old\nname"))
        );
        assert_eq!(entries[1].index, '?');
        assert_eq!(
            repo.status_filtered(&["sub".into()], false).unwrap(),
            entries[..1]
        );
        assert_eq!(fs::read(repo.git_dir.join("index")).unwrap(), index);
        assert!(repo
            .status_filtered(&[":(glob)*".into()], true)
            .unwrap()
            .is_empty());
        let header = format!(":100644 100644 {} {}", "0".repeat(40), "0".repeat(40));
        let conflicts =
            parse_status_diff(format!("{header} U\0a\0{header} M\0a\0").as_bytes(), false).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].conflicted());
        for invalid in [
            format!("{header} R100\0old\0"),
            format!("{header} M\0../escape\0"),
            format!("{header} M\0truncated"),
        ] {
            assert!(parse_status_diff(invalid.as_bytes(), false).is_err());
        }
    }

    #[test]
    fn raw_date_fixture_and_invalid_headers() {
        let input = include_str!("../test/main/date-test.in");
        let commits = parse_raw_history(input).unwrap();
        assert_eq!(commits.len(), 25);
        assert_eq!(commits[0].date, "2015-08-31T04:01:32+09:00");
        assert_eq!(commits[0].committer_date, "2015-08-31T04:15:58+09:00");
        assert_eq!(commits[0].subject, "Add zsh completion file for autoload");
        assert_eq!(commits[1].author, "Jonas Fonseca");
        assert!(parse_raw_history(&input.replace("1440961292 +0900", "broken")).is_err());
        assert!(parse_raw_history("commit not-an-id").is_err());
        assert!(parse_raw_history("commit 91912eb97da4f6907015dab41ef9bba315730854").is_err());
        let body = input.replace(
            "Add zsh completion file for autoload",
            "commit this subject",
        );
        assert_eq!(
            parse_raw_history(&body).unwrap()[0].subject,
            "commit this subject"
        );
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
    fn applying_mailbox_without_head_name_uses_current_branch() {
        let f = Fixture::new();
        let repo = f.repo();
        fs::write(f.0.join("file"), "base\n").unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();
        repo.command(["branch", "-M", "main"]).unwrap();
        let state = repo.git_dir.join("rebase-apply");
        fs::create_dir(&state).unwrap();
        fs::write(state.join("applying"), "").unwrap();
        assert_eq!(repo.status_header().unwrap(), "Applying mailbox to main");
        fs::write(state.join("head-name"), "refs/heads/topic\n").unwrap();
        assert_eq!(repo.status_header().unwrap(), "Applying mailbox to topic");
        fs::remove_file(state.join("head-name")).unwrap();
        fs::create_dir(state.join("head-name")).unwrap();
        assert!(repo.status_header().is_err());
    }
    #[test]
    fn worktree_diff_preserves_conflict_and_configured_prefixes_without_changing_index() {
        let f = Fixture::new();
        let repo = f.repo();
        fs::write(f.0.join("file"), "base\n").unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();
        repo.command(["branch", "-M", "main"]).unwrap();
        repo.command(["branch", "side"]).unwrap();
        fs::write(f.0.join("file"), "ours\n").unwrap();
        repo.command(["commit", "-qam", "ours"]).unwrap();
        repo.command(["checkout", "-q", "side"]).unwrap();
        fs::write(f.0.join("file"), "theirs\n").unwrap();
        repo.command(["commit", "-qam", "theirs"]).unwrap();
        repo.command(["checkout", "-q", "main"]).unwrap();
        assert!(repo.command(["merge", "side"]).is_err());
        assert!(repo.status().unwrap()[0].conflicted());
        let default = repo.worktree_diff_bytes(Some(Path::new("file"))).unwrap();
        assert!(default
            .windows(b"+++ b/file".len())
            .any(|part| part == b"+++ b/file"));
        repo.command(["config", "diff.noprefix", "true"]).unwrap();
        let index = repo.command(["ls-files", "-s", "-z"]).unwrap();
        let diff = repo.worktree_diff_bytes(Some(Path::new("file"))).unwrap();
        assert_eq!(repo.worktree_diff_bytes(None).unwrap(), diff);
        assert!(diff.starts_with(b"diff --cc file\n"));
        assert!(diff
            .windows(b"+++ file".len())
            .any(|part| part == b"+++ file"));
        assert!(diff
            .windows(b"+<<<<<<< HEAD".len())
            .any(|part| part == b"+<<<<<<< HEAD"));
        assert_eq!(repo.command(["ls-files", "-s", "-z"]).unwrap(), index);
        assert!(repo
            .worktree_diff_bytes(Some(Path::new("../file")))
            .is_err());
    }
    #[test]
    fn path_filtered_history_connects_visible_commits() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        for (file, value) in [
            ("selected", "first"),
            ("other", "hidden"),
            ("selected", "last"),
        ] {
            fs::write(fixture.0.join(file), value).unwrap();
            repo.command(["add", file]).unwrap();
            repo.command(["commit", "-qm", value]).unwrap();
        }
        let commits = repo.history(&["--".into(), "selected".into()], 0).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, [commits[1].oid.clone()]);
        assert!(commits[1].parents.is_empty());
    }

    #[test]
    fn show_applies_context_and_word_diff_without_changing_history() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        repo.command(["config", "commit.gpgsign", "false"]).unwrap();
        use std::fmt::Write;
        let mut before = String::new();
        for n in 1..=20 {
            writeln!(before, "line {n}").unwrap();
        }
        fs::write(fixture.0.join("file"), &before).unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();
        fs::write(
            fixture.0.join("file"),
            before.replace("line 10\n", "changed 10\n"),
        )
        .unwrap();
        repo.command(["commit", "-qam", "change"]).unwrap();
        for context in [0, 3, 4, 5, 8] {
            for word in [false, true] {
                let show = repo.show("HEAD", context, word, None, 80).unwrap();
                let span = if context == 0 {
                    "10".into()
                } else {
                    format!("{},{}", 10 - context, 2 * context + 1)
                };
                assert!(show.contains(&format!("@@ -{span} +{span} @@")), "{show}");
                assert!(show.contains(if word {
                    "[-line-]{+changed+} 10"
                } else {
                    "-line 10\n+changed 10"
                }));
            }
        }
        assert_eq!(repo.history(&[], 0).unwrap().len(), 2);
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
        let blame = repo
            .blame(Some("HEAD"), Path::new(":(glob)*"), None)
            .unwrap();
        assert_eq!(blame.len(), 2);
        assert_eq!(blame[1].line, 2);
        assert!(repo
            .show("HEAD", 3, false, None, 80)
            .unwrap()
            .contains("initial"));
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
        assert!(repo.show("--output=oops", 3, false, None, 80).is_err());
        assert!(repo.history(&["--format=oops".into()], 1).is_err());
    }
}
