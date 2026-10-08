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
pub fn validate_diff_options(options: &[String]) -> Result<()> {
    if let Some(option) = options.iter().find(|option| {
        matches!(
            option.as_str(),
            "--" | "--end-of-options" | "--output" | "-o" | "--ext-diff" | "--textconv"
        ) || option.starts_with("--output=")
            || option.starts_with("--ext-diff=")
            || option.starts_with("--textconv=")
            || (option.starts_with("-o") && !option.starts_with("--"))
    }) {
        return Err(GitError(format!("Unsupported diff option: {option}")));
    }
    Ok(())
}

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
    run_with_input(cwd, args, None)
}
fn git_command<I, S>(cwd: &Path, args: I) -> Result<Command>
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
    Ok(command)
}
pub(crate) fn run_with_input<I, S>(cwd: &Path, args: I, input: Option<&[u8]>) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_command_with_input(git_command(cwd, args)?, input)
}
fn run_command_with_input(mut command: Command, input: Option<&[u8]>) -> Result<Vec<u8>> {
    let output = if HISTORY_PROCESS.with(|state| state.borrow().is_some()) {
        command_output(&mut command, input)?
    } else if let Some(input) = input {
        use std::io::Write;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::trace::command(&command);
        let mut child = command.spawn().map_err(|e| GitError(e.to_string()))?;
        let mut stdin = child.stdin.take().expect("piped Git stdin");
        // Drain output while writing: either pipe can exceed the OS buffer.
        let (output, written) = std::thread::scope(|scope| {
            let writer = scope.spawn(move || stdin.write_all(input));
            (child.wait_with_output(), writer.join())
        });
        let output = output.map_err(|e| GitError(e.to_string()))?;
        crate::trace::append(&output.stderr);
        if output.status.success() {
            written
                .map_err(|_| GitError("Git stdin writer panicked".into()))?
                .map_err(|e| GitError(format!("Could not write Git stdin: {e}")))?;
        }
        output
    } else {
        command_output(&mut command, None)?
    };
    if HISTORY_PROCESS.with(|state| {
        state
            .borrow()
            .as_ref()
            .is_some_and(|process| process.cancelled.load(std::sync::atomic::Ordering::Acquire))
    }) {
        return Err(GitError("History refresh cancelled".into()));
    }
    if !output.status.success() {
        return Err(GitError(format!(
            "git exited with {}: {}",
            output.status,
            text(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}
// Refresh owns a cancellable worker; Git and notes queries never run on the UI.
pub struct HistoryRefresh {
    worker: Option<std::thread::JoinHandle<Result<Vec<Commit>>>>,
    cancellation: std::sync::Arc<HistoryCancellation>,
}
struct HistoryCancellation {
    cancelled: std::sync::atomic::AtomicBool,
    child: std::sync::Mutex<Option<std::process::Child>>,
}
thread_local! {
    static HISTORY_PROCESS: std::cell::RefCell<Option<std::sync::Arc<HistoryCancellation>>> = const { std::cell::RefCell::new(None) };
}
struct HistoryWire {
    notes: bool,
    annotations: std::collections::HashSet<String>,
    metadata: Option<Command>,
    parent_prefix: bool,
}
fn drain(
    mut pipe: impl std::io::Read + Send + 'static,
) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}
fn command_output(command: &mut Command, input: Option<&[u8]>) -> Result<std::process::Output> {
    let cancellation = HISTORY_PROCESS.with(|state| state.borrow().clone());
    let Some(cancellation) = cancellation else {
        return crate::trace::output(command).map_err(|e| GitError(e.to_string()));
    };
    use std::sync::atomic::Ordering;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let (stdout, stderr, writer) = {
        let mut slot = cancellation
            .child
            .lock()
            .map_err(|_| GitError("History process lock poisoned".into()))?;
        if cancellation.cancelled.load(Ordering::Acquire) {
            return Err(GitError("History refresh cancelled".into()));
        }
        crate::trace::command(command);
        let mut child = command.spawn().map_err(|e| GitError(e.to_string()))?;
        let writer = input.map(|input| {
            let bytes = input.to_vec();
            let mut stdin = child.stdin.take().expect("piped Git stdin");
            std::thread::spawn(move || std::io::Write::write_all(&mut stdin, &bytes))
        });
        let pipes = (
            drain(child.stdout.take().expect("piped Git stdout")),
            drain(child.stderr.take().expect("piped Git stderr")),
            writer,
        );
        *slot = Some(child);
        pipes
    };
    let status = loop {
        {
            let mut slot = cancellation
                .child
                .lock()
                .map_err(|_| GitError("History process lock poisoned".into()))?;
            let child = slot.as_mut().expect("active history query");
            if cancellation.cancelled.load(Ordering::Acquire) {
                // A child may finish forking after the first group signal. Keep
                // its leader unreaped and retry until inherited pipes close.
                stop_history_child(child);
            }
            // Keep the leader PID unreaped while descendants may hold pipes;
            // cancellation can then safely address its isolated process group.
            if stdout.is_finished()
                && stderr.is_finished()
                && writer.as_ref().map_or(true, |writer| writer.is_finished())
            {
                if let Some(status) = child.try_wait().map_err(|e| GitError(e.to_string()))? {
                    *slot = None;
                    break status;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    if let Some(writer) = writer {
        let written = writer
            .join()
            .map_err(|_| GitError("Git stdin writer panicked".into()))?;
        if status.success() {
            written.map_err(|e| GitError(format!("Could not write Git stdin: {e}")))?;
        }
    }
    let read = |handle: std::thread::JoinHandle<std::io::Result<Vec<u8>>>| {
        handle
            .join()
            .map_err(|_| GitError("Git reader panicked".into()))?
            .map_err(|e| GitError(e.to_string()))
    };
    let output = std::process::Output {
        status,
        stdout: read(stdout)?,
        stderr: read(stderr)?,
    };
    crate::trace::append(&output.stderr);
    Ok(output)
}
fn stop_history_child(child: &mut std::process::Child) {
    let _ = child.kill();
    #[cfg(unix)]
    {
        // Native group signalling avoids first-party unsafe FFI. The isolated
        // leader remains unreaped until all inherited output pipes reach EOF.
        // POSIX sh supplies kill even in minimal Git/Rust images without
        // /bin/kill. Only the owned numeric PID crosses the argv boundary.
        let _ = Command::new("/bin/sh")
            .args(["-c", "command kill -KILL \"$1\"", "tig-history-cancel"])
            .arg(format!("-{}", child.id()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
impl HistoryRefresh {
    pub fn cancel(&self) {
        self.cancellation
            .cancelled
            .store(true, std::sync::atomic::Ordering::Release);
    }
    /// Join only after the interactive terminal is restored, or poll completes.
    pub fn finish(mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
    pub fn poll(&mut self) -> Result<Option<Vec<Commit>>> {
        if !self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Ok(None);
        }
        self.worker
            .take()
            .expect("completed history worker")
            .join()
            .map_err(|_| GitError("History worker panicked".into()))?
            .map(Some)
    }
}
impl Drop for HistoryRefresh {
    fn drop(&mut self) {
        self.cancel();
    }
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
    let mut decorations: std::collections::HashMap<&str, Vec<(&Reference, String)>> =
        std::collections::HashMap::new();
    let compare = |a: &(&Reference, String), b: &(&Reference, String)| {
        kind(a.0, upstream)
            .cmp(&kind(b.0, upstream))
            .then_with(|| numeric(&a.0.name, &b.0.name))
    };
    // C applies replacements in input order: clear refs loaded so far, then
    // allow later refs (notably tags after refs/replace/) onto the same commit.
    for reference in references {
        if let Some(original) = reference.name.strip_prefix("refs/replace/") {
            let label = decorations
                .remove(original)
                .and_then(|v| v.into_iter().min_by(compare))
                .map(|(_, label)| {
                    label
                        .trim_start_matches("HEAD -> ")
                        .trim_start_matches("refs/heads/")
                        .to_owned()
                })
                .unwrap_or_else(|| "replaced".into());
            decorations.insert(original, vec![(reference, format!("replace: {label}"))]);
            continue;
        }
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
        decorations.entry(oid).or_default().push((reference, label));
    }
    for labels in decorations.values_mut() {
        labels.sort_by(compare);
    }
    for commit in commits {
        commit.decorations = decorations
            .get(commit.oid.as_str())
            .map(|v| {
                v.iter()
                    .map(|(_, label)| label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
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
        Self::parse_for_view(revisions, false)
    }
    fn parse_for_view(revisions: &[String], reflog: bool) -> Result<Self> {
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
                "--grep-reflog" if reflog => { expects_value = !inline_value; }
                "-g" | "--walk-reflogs" if reflog && !inline_value => {}
                "--since" | "--after" | "--until" | "--before" | "--author" | "--committer" |
                "--grep" | "--max-count" | "--skip" | "--min-parents" | "--max-parents" | "-n" => {
                    expects_value = !inline_value;
                }
                "--stdin" if !inline_value => { options.has_revision = true; }
                "--no-walk" if !inline_value || matches!(arg.as_str(), "--no-walk=sorted" | "--no-walk=unsorted") => {}
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

fn git_option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--since"
            | "--after"
            | "--until"
            | "--before"
            | "--author"
            | "--committer"
            | "--grep"
            | "--grep-reflog"
            | "--max-count"
            | "--skip"
            | "--min-parents"
            | "--max-parents"
            | "--glob"
            | "--exclude"
            | "-n"
            | "-G"
            | "-S"
            | "-L"
    )
}

/// Let Git split implicit filenames from revisions before views consume arguments.
/// Explicit path arguments stay intact, including embedded LF bytes.
pub fn classify_cli_args(cwd: &Path, args: &[String]) -> Result<Vec<String>> {
    if args.is_empty() {
        return Ok(Vec::new());
    }
    let mut revisions = Vec::new();
    let mut remaining = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if matches!(arg.as_str(), "--" | "--end-of-options") {
            remaining.extend_from_slice(&args[i..]);
            break;
        }
        // Keep separate option values as one argv pair; Git's rev-parse does not
        // understand log's value-taking options (and may interpret values as paths).
        let takes_value = git_option_takes_value(arg);
        if crate::config::is_revision_flag(arg) || takes_value {
            revisions.push(arg.clone());
            if takes_value {
                i += 1;
                revisions.push(
                    args.get(i)
                        .ok_or_else(|| GitError("Missing Git option value".into()))?
                        .clone(),
                );
            }
        } else {
            remaining.push(arg.clone());
        }
        i += 1;
    }
    let query = |first, second| {
        let args = ["rev-parse", first, second]
            .into_iter()
            .chain(remaining.iter().map(String::as_str));
        if first == "--no-revs" {
            let output = command_output(&mut git_command(cwd, args)?, None)?;
            if !output.status.success() {
                return Err(GitError("No revisions match the given arguments.".into()));
            }
            Ok(output.stdout)
        } else {
            run(cwd, args)
        }
    };
    let files = query("--no-revs", "--no-flags")?;
    let flags = query("--flags", "--no-revs")?;
    let symbolic = query("--symbolic", "--revs-only")?;
    // Once Git reaches an implicit filename, the rest of argv is paths.
    // Match that whole suffix, never split its LF protocol into filename bytes.
    let encoded = |args: &[String]| -> Vec<u8> {
        args.iter()
            .flat_map(|arg| arg.bytes().chain([b'\n']))
            .collect()
    };
    let paths = if let Some(separator) = remaining.iter().position(|arg| arg == "--") {
        let paths = &remaining[separator + 1..];
        if files != encoded(paths) {
            return Err(GitError("Git returned unexpected explicit paths".into()));
        }
        paths.to_vec()
    } else if files.is_empty() {
        Vec::new()
    } else {
        let first = (0..remaining.len())
            .find(|&i| files == encoded(&remaining[i..]))
            .ok_or_else(|| GitError("Git returned unexpected implicit paths".into()))?;
        remaining[first..].to_vec()
    };
    // Revision ranges may be expanded by Git. Retain the user's option boundary.
    if remaining.iter().any(|arg| arg == "--end-of-options") {
        return Ok(args.to_vec());
    }
    let lines = |bytes: &[u8]| -> Result<Vec<String>> {
        String::from_utf8(bytes.to_vec())
            .map_err(|_| GitError("Non-UTF-8 Git revision argument".into()))
            .map(|s| s.split_terminator('\n').map(str::to_owned).collect())
    };
    let mut result = lines(&flags)?;
    result.extend(revisions);
    result.extend(lines(&symbolic)?);
    if !paths.is_empty() || remaining.iter().any(|arg| arg == "--") {
        result.push("--".into());
        result.extend(paths);
    }
    Ok(result)
}

impl Repository {
    /// Load the NUL-delimited grep protocol without guessing revisions from labels.
    pub fn grep(
        &self,
        args: &[String],
    ) -> std::result::Result<Vec<Option<crate::grep::GrepLine>>, Box<dyn std::error::Error>> {
        let options = crate::grep::GrepOptions::parse(args)?;
        let mut command = Command::new("git");
        command
            .current_dir(&self.root)
            .args(["--no-pager", "--literal-pathspecs", "-c", "color.ui=false"])
            .args(["grep", "--no-color", "-n", "-z", "--full-name", "-I"])
            .args(&options.args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null());
        let output = crate::trace::output(&mut command)?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(format!(
                "git grep exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let mut revisions = Vec::new();
        for operand in options.operands {
            // Git treats the first unresolved operand and everything after it as paths.
            // Resolve separately: a filename such as HEAD:literal is not proof of a revision.
            if self
                .command([
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &operand,
                ])
                .is_err()
            {
                break;
            }
            self.grep_tree_oid(&operand)?;
            revisions.push(operand);
        }
        if revisions.iter().any(|a| {
            revisions
                .iter()
                .any(|b| a != b && b.starts_with(&format!("{a}:")))
        }) {
            return Err(
                "Overlapping Git grep revision labels are ambiguous; search each tree separately"
                    .into(),
            );
        }
        let mut hits = crate::grep::grep_rows(&output.stdout, &revisions)?;
        let mut pinned: std::collections::HashMap<(Option<String>, PathBuf), String> =
            std::collections::HashMap::new();
        for hit in &mut hits {
            hit.cached = options.cached;
            if hit.cached || hit.revision.is_some() {
                let key = (hit.revision.clone(), hit.path.clone());
                let oid = if let Some(oid) = pinned.get(&key) {
                    oid.clone()
                } else {
                    let oid = self.grep_blob_oid(hit)?;
                    pinned.insert(key, oid.clone());
                    oid
                };
                hit.source_oid = Some(oid);
            }
        }
        if options.before == 0 && options.after == 0 {
            return Ok(hits.into_iter().map(Some).collect());
        }
        let mut rows = Vec::new();
        let mut start = 0;
        while start < hits.len() {
            let end = start
                + hits[start..]
                    .iter()
                    .take_while(|hit| {
                        hit.path == hits[start].path && hit.revision == hits[start].revision
                    })
                    .count();
            let content = self.grep_blob(&hits[start])?;
            if !rows.is_empty() {
                rows.push(None);
            }
            rows.extend(crate::grep::context_rows(
                &hits[start..end],
                &content,
                options.before,
                options.after,
            )?);
            start = end;
        }
        Ok(rows)
    }

    pub fn grep_blob(
        &self,
        hit: &crate::grep::GrepLine,
    ) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
        if !crate::grep::safe_grep_path(&hit.path) {
            return Err("Invalid grep result path".into());
        }
        if let Some(oid) = &hit.source_oid {
            return Ok(self.blob(oid)?);
        }
        let prefix = if let Some(revision) = &hit.revision {
            format!("{}:", self.grep_tree_oid(revision)?)
        } else if hit.cached {
            ":".into()
        } else {
            return Ok(std::fs::read(self.root.join(&hit.path))?);
        };
        let mut spec = OsString::from(prefix);
        spec.push(hit.path.as_os_str());
        Ok(self.command([OsString::from("cat-file"), OsString::from("blob"), spec])?)
    }

    fn grep_blob_oid(&self, hit: &crate::grep::GrepLine) -> Result<String> {
        if !crate::grep::safe_grep_path(&hit.path) {
            return Err(GitError("Invalid grep result path".into()));
        }
        let mut spec = OsString::from(if let Some(revision) = &hit.revision {
            format!("{}:", self.grep_tree_oid(revision)?)
        } else {
            ":".into()
        });
        spec.push(hit.path.as_os_str());
        let oid = text(trim_lf(&self.command([
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--end-of-options"),
            spec,
        ])?));
        if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GitError("Expected a full grep blob object ID".into()));
        }
        Ok(oid)
    }

    fn grep_tree_oid(&self, revision: &str) -> Result<String> {
        let object = self.command(["rev-parse", "--verify", "--end-of-options", revision])?;
        let object = text(&object);
        let tree = self.command([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{}^{{tree}}", object.trim()),
        ])?;
        Ok(text(&tree).trim().to_owned())
    }

    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start.as_ref();
        let mut command = git_command(
            start,
            [
                "rev-parse",
                "--git-dir",
                "--is-inside-work-tree",
                "--show-cdup",
                "--show-prefix",
                "HEAD",
                "--symbolic-full-name",
                "HEAD",
            ],
        )?;
        // An unborn HEAD fails after Git has already emitted the repository fields.
        let output = crate::trace::output(&mut command).map_err(|e| GitError(e.to_string()))?;
        let bytes = &output.stdout;
        let markers: Vec<_> = [(&b"\ntrue\n"[..], true), (&b"\nfalse\n"[..], false)]
            .into_iter()
            .flat_map(|(marker, inside)| {
                bytes
                    .windows(marker.len())
                    .enumerate()
                    .filter_map(move |(offset, value)| {
                        (value == marker).then_some(((offset, marker.len()), inside))
                    })
            })
            .collect();
        // A repository pathname can itself contain the LF-delimited boolean.
        // Query each pathname separately in that ambiguous case; never truncate it.
        if markers.len() > 1 {
            let git_dir = path(trim_lf(&run(start, ["rev-parse", "--absolute-git-dir"])?))?;
            let bare = trim_lf(&run(start, ["rev-parse", "--is-bare-repository"])?) == b"true";
            let root = if bare {
                git_dir.clone()
            } else {
                path(trim_lf(&run(start, ["rev-parse", "--show-toplevel"])?))?
            };
            return Ok(Self {
                root,
                git_dir,
                bare,
                invocation: start.canonicalize().map_err(|e| GitError(e.to_string()))?,
            });
        }
        let &(separator, inside) = markers.first().ok_or_else(|| {
            GitError(format!(
                "Could not discover repository: {}",
                text(&output.stderr).trim()
            ))
        })?;
        let (offset, length) = separator;
        let invocation = start.canonicalize().map_err(|e| GitError(e.to_string()))?;
        let git_dir = invocation
            .join(path(&bytes[..offset])?)
            .canonicalize()
            .map_err(|e| GitError(e.to_string()))?;
        let cdup = bytes[offset + length..]
            .split(|b| *b == b'\n')
            .next()
            .ok_or_else(|| GitError("Missing repository root".into()))?;
        let bare = !inside
            && cdup.is_empty()
            && trim_lf(&run(start, ["config", "--bool", "core.bare"])?) == b"true";
        let root = if bare {
            git_dir.clone()
        } else if !inside {
            // Explicit external worktrees can contain LF in the absolute cdup.
            path(trim_lf(&run(start, ["rev-parse", "--show-toplevel"])?))?
        } else {
            invocation
                .join(path(cdup)?)
                .canonicalize()
                .map_err(|e| GitError(e.to_string()))?
        };
        Ok(Self {
            root,
            git_dir,
            bare,
            invocation,
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
        if revision == "HEAD" {
            let output = run_with_input(
                &self.root,
                ["cat-file", "--batch-check=%(objectname)"],
                Some(b"HEAD^{commit}\n"),
            )?;
            let oid = trim_lf(&output);
            if matches!(oid.len(), 40 | 64) && oid.iter().all(u8::is_ascii_hexdigit) {
                return Ok(text(oid));
            }
            return Err(GitError("HEAD does not resolve to a commit".into()));
        }
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
        self.history_ordered(revisions, limit, "topo", "no")
    }
    pub fn history_ordered(
        &self,
        revisions: &[String],
        limit: usize,
        order: &str,
        notes: &str,
    ) -> Result<Vec<Commit>> {
        self.history_with_input(revisions, limit, order, None, notes)
    }
    pub fn history_from_stdin(
        &self,
        revisions: &[String],
        input: &[u8],
        order: &str,
    ) -> Result<Vec<Commit>> {
        self.history_with_input(revisions, 0, order, Some(input), "no")
    }
    fn history_with_input(
        &self,
        revisions: &[String],
        limit: usize,
        order: &str,
        input: Option<&[u8]>,
        notes: &str,
    ) -> Result<Vec<Commit>> {
        let (command, unborn, first_parent, mut wire) =
            self.history_command(revisions, limit, order, notes)?;
        self.finish_history(
            &run_command_with_input(command, input)?,
            unborn,
            first_parent,
            &mut wire,
        )
    }
    fn history_annotations(&self, notes: &str) -> Result<std::collections::HashSet<String>> {
        let mut command = git_command(
            &self.root,
            ["config", "--null", "--get-regexp", "^notes[.]displayref$"],
        )?;
        let config = command_output(&mut command, None)?;
        if !config.status.success() && config.status.code() != Some(1) {
            return Err(GitError(format!(
                "Could not read notes config: {}",
                text(&config.stderr)
            )));
        }
        let default = path(trim_lf(&self.command(["notes", "get-ref"])?))?.into_os_string();
        let mut display = Vec::new();
        for entry in config.stdout.split(|b| *b == 0).filter(|e| !e.is_empty()) {
            let split = entry
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| GitError("Malformed notes config".into()))?;
            let value = path(&entry[split + 1..])?.into_os_string();
            match &entry[..split] {
                b"notes.displayref" => display.push(value),
                _ => return Err(GitError("Unexpected notes config key".into())),
            }
        }
        if let Some(value) = std::env::var_os("GIT_NOTES_DISPLAY_REF") {
            display = value
                .as_encoded_bytes()
                .split(|b| *b == b':')
                .map(|value| path(value).map(PathBuf::into_os_string))
                .collect::<Result<_>>()?;
        }
        display.insert(0, default);
        if !matches!(notes, "yes" | "true" | "1" | "") {
            display.push(notes.into());
        }
        let mut refs = std::collections::BTreeSet::new();
        let names = self.command(["for-each-ref", "--format=%(refname)"])?;
        for pattern in display {
            let mut arg = OsString::from("--ref=");
            arg.push(pattern);
            let pattern = path(trim_lf(&self.command([
                OsStr::new("notes"),
                &arg,
                OsStr::new("get-ref"),
            ])?))?
            .into_os_string();
            if pattern
                .as_encoded_bytes()
                .iter()
                .any(|byte| b"*?[".contains(byte))
            {
                let mut glob = OsString::from("--glob=");
                glob.push(pattern);
                let expanded =
                    self.command([OsStr::new("rev-parse"), OsStr::new("--symbolic"), &glob])?;
                for name in expanded
                    .split(|b| *b == b'\n')
                    .filter(|name| !name.is_empty())
                {
                    refs.insert(path(name)?.into_os_string());
                }
            } else if names
                .split(|b| *b == b'\n')
                .any(|name| name == pattern.as_encoded_bytes())
            {
                refs.insert(pattern);
            }
        }
        let mut mappings = Vec::new();
        for name in refs {
            let mut arg = OsString::from("--ref=");
            arg.push(name);
            let output = self.command([OsStr::new("notes"), &arg, OsStr::new("list")])?;
            for row in output.split(|b| *b == b'\n').filter(|row| !row.is_empty()) {
                let fields: Vec<_> = row.split(|b| *b == b' ').collect();
                if fields.len() != 2
                    || fields.iter().any(|oid| {
                        !matches!(oid.len(), 40 | 64) || !oid.iter().all(u8::is_ascii_hexdigit)
                    })
                {
                    return Err(GitError("Malformed notes mapping".into()));
                }
                mappings.push((text(fields[0]), text(fields[1])));
            }
        }
        let mut result = std::collections::HashSet::new();
        if !mappings.is_empty() {
            let mut input = String::new();
            for (note, _) in &mappings {
                input.push_str(note);
                input.push('\n');
            }
            let output = run_with_input(
                &self.root,
                ["cat-file", "--batch-check=%(objectname) %(objectsize)"],
                Some(input.as_bytes()),
            )?;
            let rows: Vec<_> = output
                .split(|b| *b == b'\n')
                .filter(|row| !row.is_empty())
                .collect();
            if rows.len() != mappings.len() {
                return Err(GitError("Truncated notes sizes".into()));
            }
            for (row, (note, oid)) in rows.into_iter().zip(mappings) {
                let fields: Vec<_> = row.split(|b| *b == b' ').collect();
                if fields.len() != 2 || fields[0] != note.as_bytes() {
                    return Err(GitError("Malformed notes size".into()));
                }
                let size = text(fields[1])
                    .parse::<u64>()
                    .map_err(|_| GitError("Invalid notes size".into()))?;
                if size > 0 {
                    result.insert(oid);
                }
            }
        }
        Ok(result)
    }
    fn history_command(
        &self,
        revisions: &[String],
        limit: usize,
        order: &str,
        notes: &str,
    ) -> Result<(Command, bool, bool, HistoryWire)> {
        let options = HistoryOptions::parse(revisions)?;
        let order_arg = match order {
            "auto" | "topo" => Some("--topo-order"),
            "default" => None,
            "date" => Some("--date-order"),
            "author-date" => Some("--author-date-order"),
            "reverse" => Some("--reverse"),
            _ => return Err(GitError(format!("Invalid commit order: {order}"))),
        };
        let show_notes = !matches!(notes, "no" | "false" | "0");
        let split = revisions
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(revisions.len());
        let mut args = vec!["log".to_owned(), "--encoding=UTF-8".into()];
        if let Some(order_arg) = order_arg {
            args.push(order_arg.into());
        }
        let mut leading = 0;
        while leading < split {
            let arg = &revisions[leading];
            if arg == "--end-of-options" || !arg.starts_with('-') {
                break;
            }
            leading += 1;
            if git_option_takes_value(arg) {
                leading += 1;
            }
        }
        leading = leading.min(split);
        args.extend_from_slice(&revisions[..leading]);
        args.extend(["--date=raw".into(), "--parents".into(), "--no-color".into()]);
        let notes_arg = show_notes.then_some(args.len());
        if show_notes {
            args.push(match notes {
                "yes" | "true" | "1" | "" => "--show-notes".into(),
                reference => format!("--show-notes={reference}"),
            });
        }
        let format = "--pretty=format:commit %m %H %P%x00%aN <%aE> %ad%x00%cN <%cE> %cd%x00%s";
        let pretty_arg = args.len();
        args.push(if show_notes {
            format!("{format}%x00%N%x03")
        } else {
            format.into()
        });
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
        args.extend_from_slice(&revisions[leading..split]);
        if split < revisions.len() || leading == split {
            args.push("--".into());
        }
        if split < revisions.len() {
            args.extend_from_slice(&revisions[split + 1..]);
        }
        let directory = if revisions.iter().any(|arg| arg == "--") {
            &self.root
        } else {
            &self.invocation
        };
        let annotations = if show_notes {
            self.history_annotations(notes)?
        } else {
            std::collections::HashSet::new()
        };
        let parent_prefix = show_notes
            && !options.merge
            && annotations.is_empty()
            && !revisions[..split]
                .iter()
                .any(|arg| matches!(arg.as_str(), "--follow" | "--boundary" | "--stdin"))
            && (split < revisions.len() || leading == split);
        let metadata = if !show_notes {
            None
        } else {
            // C's unescaped %N cannot frame arbitrary note blobs. Keep its real
            // log command, but always obtain authoritative selection/parents/
            // identities without notes. Ref mappings can change while Git runs.
            let mut metadata = args.clone();
            metadata[pretty_arg] = format!("{format}%x00%x03");
            if let Some(index) = notes_arg {
                metadata.remove(index);
            }
            if parent_prefix {
                metadata[0] = "rev-list".into();
                metadata.insert(1, "--no-commit-header".into());
                if !options.has_revision && !unborn {
                    let boundary = metadata
                        .iter()
                        .position(|arg| arg == "--")
                        .expect("history path boundary");
                    metadata.insert(boundary, "HEAD".into());
                }
            } else {
                metadata.insert(1, "--no-show-signature".into());
            }
            Some(git_command(directory, metadata)?)
        };
        let mut command = git_command(directory, args)?;
        // Keep configured signatures from entering this metadata wire format.
        let count = std::env::var("GIT_CONFIG_COUNT")
            .unwrap_or_else(|_| "0".into())
            .parse::<usize>()
            .map_err(|_| GitError("Invalid GIT_CONFIG_COUNT".into()))?
            + 1;
        command
            .env(
                "GIT_CONFIG_COUNT",
                count
                    .checked_add(1)
                    .ok_or_else(|| GitError("Invalid GIT_CONFIG_COUNT".into()))?
                    .to_string(),
            )
            .env(format!("GIT_CONFIG_KEY_{count}"), "log.showSignature")
            .env(format!("GIT_CONFIG_VALUE_{count}"), "false");
        Ok((
            command,
            unborn,
            options.first_parent,
            HistoryWire {
                notes: show_notes,
                annotations,
                metadata,
                parent_prefix,
            },
        ))
    }
    pub fn start_history(
        &self,
        revisions: &[String],
        order: &str,
        notes: &str,
    ) -> Result<HistoryRefresh> {
        let repo = self.clone();
        let revisions = revisions.to_vec();
        let order = order.to_owned();
        let notes = notes.to_owned();
        let cancellation = std::sync::Arc::new(HistoryCancellation {
            cancelled: std::sync::atomic::AtomicBool::new(false),
            child: std::sync::Mutex::new(None),
        });
        let process = cancellation.clone();
        let worker = std::thread::spawn(move || {
            HISTORY_PROCESS.with(|state| *state.borrow_mut() = Some(process));
            repo.history_ordered(&revisions, 0, &order, &notes)
        });
        Ok(HistoryRefresh {
            worker: Some(worker),
            cancellation,
        })
    }
    fn finish_history(
        &self,
        bytes: &[u8],
        unborn: bool,
        first_parent: bool,
        wire: &mut HistoryWire,
    ) -> Result<Vec<Commit>> {
        let metadata = wire
            .metadata
            .take()
            .map(|command| run_command_with_input(command, None))
            .transpose()?;
        let mut result = if let Some(metadata) = metadata.as_deref() {
            let (compact, prefixes) = compact_history(metadata, wire.parent_prefix)?;
            if prefixes.is_empty() && !bytes.is_empty() {
                return Err(GitError(
                    "History changed between notes and metadata queries".into(),
                ));
            }
            // Consume the actual C-format log too: every authoritative metadata
            // prefix must occur in its output in traversal order. Notes may contain
            // lookalike prefixes, but can never select or alter commit metadata.
            let mut remaining = bytes;
            for prefix in prefixes {
                let offset = remaining
                    .windows(prefix.len())
                    .position(|window| window == prefix)
                    .ok_or_else(|| {
                        GitError("History changed between notes and metadata queries".into())
                    })?;
                remaining = &remaining[offset + prefix.len()..];
            }
            parse_raw_history(&text(&compact))?
        } else if wire.notes {
            parse_history(bytes)?
        } else {
            parse_raw_history(&text(bytes))?
        };
        for commit in &mut result {
            commit.annotated = wire.annotations.contains(&commit.oid);
        }
        let references = self.refs()?;
        let upstream = self.upstream()?;
        decorate_history(&mut result, &references, &upstream);
        if first_parent {
            for commit in &mut result {
                commit.parents.truncate(1);
            }
        }
        Ok(if unborn { Vec::new() } else { result })
    }
    /// Reflog subjects and selectors accompany the same commit metadata as history.
    pub fn reflog(&self, stash: bool, revisions: &[String]) -> Result<(Vec<Commit>, Vec<String>)> {
        HistoryOptions::parse_for_view(revisions, true)?;
        let mut args: Vec<String> = if stash {
            vec!["stash".into(), "list".into()]
        } else {
            vec!["reflog".into(), "show".into()]
        };
        args.extend(
            revisions
                .iter()
                .filter(|arg| {
                    !stash
                        || (arg.starts_with('-')
                            && !matches!(arg.as_str(), "--all" | "--branches" | "--remotes"))
                })
                .cloned(),
        );
        args.extend([
            "--no-color".into(),
            "--no-show-signature".into(),
            "--format=%H%x00%P%x00%aN%x00%aI%x00%gs%x00%D%x00%aE%x00%cN%x00%cE%x00%cI%x00%gd"
                .into(),
            "-z".into(),
        ]);
        let output = self.command(args)?;
        let fields: Vec<_> = records(&output)?.collect();
        if fields.len() % 11 != 0 {
            return Err(GitError("Malformed reflog fields".into()));
        }
        let mut metadata = Vec::new();
        let mut selectors = Vec::new();
        for row in fields.chunks_exact(11) {
            for field in &row[..10] {
                metadata.extend_from_slice(field);
                metadata.push(0);
            }
            metadata.push(0); // Reflog rows do not carry main-view annotations.
            selectors.push(text(row[10]));
        }
        let mut commits = parse_nul_commit_metadata(&metadata)?;
        let upstream = self.upstream()?;
        decorate_history(&mut commits, &self.refs()?, &upstream);
        Ok((commits, selectors))
    }
    fn is_unborn(&self) -> Result<bool> {
        match self.command(["cat-file", "-e", "HEAD^{commit}"]) {
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
    fn upstream(&self) -> Result<String> {
        let output = self.command(["for-each-ref", "--format=%(HEAD)%00%(upstream)"])?;
        Ok(output
            .split(|b| *b == b'\n')
            .find_map(|row| row.strip_prefix(b"*\0").map(text))
            .unwrap_or_default())
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
            let output = command_output(
                Command::new(program)
                    .args(args)
                    .current_dir(&self.root)
                    .stdin(Stdio::null()),
                None,
            )
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
                references.insert(
                    0,
                    Reference {
                        name: "HEAD".into(),
                        oid,
                        target: String::new(),
                        current: true,
                    },
                );
            }
        }
        Ok(references)
    }
    pub fn show(
        &self,
        revision: &str,
        context: usize,
        word_diff: bool,
        diff_options: &[String],
        paths: (Option<&Path>, &[String]),
        width: usize,
    ) -> Result<String> {
        validate_diff_options(diff_options)?;
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
            &oid,
            "--",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        args.splice(
            args.len() - 2..args.len() - 2,
            diff_options.iter().map(OsString::from).chain([
                OsString::from(if word_diff {
                    "--word-diff=plain"
                } else {
                    "--word-diff=none"
                }),
                OsString::from("--no-ext-diff"),
                OsString::from("--no-textconv"),
            ]),
        );
        if let Some(file) = paths.0 {
            valid_path(file)?;
            args.push(file.into());
        }
        args.extend(paths.1.iter().map(OsString::from));
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
        self.worktree_diff_options(file, filters, &[])
    }
    pub fn blame_diff(&self, file: &Path, has_parent: bool, options: &[String]) -> Result<Vec<u8>> {
        validate_diff_options(options)?;
        if has_parent {
            return self.worktree_diff_options(Some(file), &[], options);
        }
        valid_path(file)?;
        let mut args: Vec<OsString> = [
            "diff",
            "--no-index",
            "--no-color",
            "--patch-with-stat",
            "--no-ext-diff",
            "--no-textconv",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        args.extend(options.iter().map(OsString::from));
        args.extend([
            OsString::from("--"),
            OsString::from("/dev/null"),
            file.into(),
        ]);
        let output = crate::trace::output(&mut git_command(&self.root, args)?)
            .map_err(|e| GitError(e.to_string()))?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(GitError(format!(
                "git diff exited with {}: {}",
                output.status,
                text(&output.stderr).trim()
            )));
        }
        Ok(output.stdout)
    }
    fn worktree_diff_options(
        &self,
        file: Option<&Path>,
        filters: &[String],
        options: &[String],
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
        args.extend(options.iter().map(OsString::from));
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
    /// Locate an index line in HEAD without attributing newly staged text.
    pub fn index_line_in_head(&self, file: &Path, line: usize) -> Result<(PathBuf, usize)> {
        valid_path(file)?;
        let path = self
            .status_filtered(&[], false)?
            .into_iter()
            .find(|entry| entry.path == file)
            .and_then(|entry| entry.original_path)
            .unwrap_or_else(|| file.to_path_buf());
        let mut old = OsString::from("HEAD:");
        old.push(&path);
        let mut new = OsString::from(":");
        new.push(file);
        let raw = self.command(vec![
            "diff".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--no-color".into(),
            "-U0".into(),
            old,
            new,
            "--".into(),
        ])?;
        let mut number = line;
        if !raw.is_empty() {
            let patch = crate::patch::Patch::parse(&raw)?;
            for hunk in patch.files.iter().flat_map(|file| &file.hunks) {
                let new_start = hunk.new_start + usize::from(hunk.new_count == 0);
                if line < new_start {
                    break;
                }
                if line < new_start + hunk.new_count {
                    return Err(GitError("No committed source for the selected line".into()));
                }
                let old_end = hunk.old_start + hunk.old_count + usize::from(hunk.old_count == 0);
                number = old_end + (line - new_start - hunk.new_count);
            }
        }
        Ok((path, number))
    }
    pub fn blame(
        &self,
        revision: Option<&str>,
        file: &Path,
        options: &[String],
    ) -> Result<Vec<BlameLine>> {
        valid_path(file)?;
        let mut args: Vec<OsString> = vec!["blame".into()];
        args.extend(crate::blame_options::arguments(self, options)?);
        args.extend(["--no-textconv".into(), "--line-porcelain".into()]);
        if let Some(revision) = revision.filter(|value| !value.is_empty()) {
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
    fn entry_pathspecs(entries: &[StatusEntry]) -> Result<Vec<u8>> {
        if entries.is_empty() {
            return Err(GitError("No status entries to update".into()));
        }
        let mut input = Vec::new();
        for entry in entries {
            for path in Self::entry_paths(entry)? {
                input.extend_from_slice(path.as_encoded_bytes());
                input.push(0);
            }
        }
        Ok(input)
    }
    pub fn stage(&self, entry: &StatusEntry) -> Result<()> {
        self.stage_many(std::slice::from_ref(entry))
    }
    pub fn stage_many(&self, entries: &[StatusEntry]) -> Result<()> {
        let input = Self::entry_pathspecs(entries)?;
        run_with_input(
            &self.root,
            [
                "add",
                "--all",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            Some(&input),
        )
        .map(|_| ())
    }
    pub fn unstage(&self, entry: &StatusEntry) -> Result<()> {
        self.unstage_many(std::slice::from_ref(entry))
    }
    pub fn unstage_many(&self, entries: &[StatusEntry]) -> Result<()> {
        let input = Self::entry_pathspecs(entries)?;
        // An unborn branch has no HEAD. rm --cached only updates its index.
        let has_head = !self.is_unborn()?;
        let args: Vec<&str> = if has_head {
            vec![
                "reset",
                "--quiet",
                "HEAD",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ]
        } else {
            vec![
                "rm",
                "--cached",
                "--ignore-unmatch",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ]
        };
        run_with_input(&self.root, args, Some(&input)).map(|_| ())
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
/// Parse the compact upstream wire only when notes are known absent. Notes
/// mappings require an authoritative notes-free traversal, not guessed delimiters.
pub fn parse_history(bytes: &[u8]) -> Result<Vec<Commit>> {
    parse_raw_history(&text(&compact_history(bytes, false)?.0))
}
fn compact_history(mut bytes: &[u8], parent_prefix: bool) -> Result<(Vec<u8>, Vec<Vec<u8>>)> {
    let mut compact = Vec::new();
    let mut prefixes = Vec::new();
    while !bytes.is_empty() {
        let fields: Vec<_> = bytes.splitn(5, |b| *b == 0).collect();
        if fields.len() != 5 {
            return Err(GitError("Truncated compact history".into()));
        }
        let rest = fields[4]
            .strip_prefix(b"\x03")
            .ok_or_else(|| GitError("Unframed history notes".into()))?;
        if !rest.is_empty()
            && rest != b"\n"
            && !(if parent_prefix {
                rest.starts_with(b"\n")
            } else {
                rest.starts_with(b"\ncommit ")
            })
        {
            return Err(GitError("Malformed compact history boundary".into()));
        }
        let start = if parent_prefix {
            let offset = fields[0]
                .windows(b"commit ".len())
                .position(|w| w == b"commit ")
                .ok_or_else(|| GitError("Missing compact history header".into()))?;
            if fields[0][..offset]
                .split(|b| b.is_ascii_whitespace())
                .filter(|id| !id.is_empty())
                .any(|id| !matches!(id.len(), 40 | 64) || !id.iter().all(u8::is_ascii_hexdigit))
            {
                return Err(GitError("Malformed traversal parent prefix".into()));
            }
            offset
        } else {
            if !fields[0].starts_with(b"commit ") {
                return Err(GitError("Missing compact history header".into()));
            }
            0
        };
        let before = compact.len();
        for (i, field) in fields[..4].iter().enumerate() {
            if i != 0 {
                compact.push(0);
            }
            compact.extend_from_slice(if i == 0 { &field[start..] } else { field });
        }
        let mut prefix = compact[before..].to_vec();
        prefix.push(0);
        prefixes.push(prefix);
        compact.push(b'\n');
        bytes = rest.strip_prefix(b"\n").unwrap_or(rest);
    }
    Ok((compact, prefixes))
}

pub(crate) fn parse_nul_commit_metadata(bytes: &[u8]) -> Result<Vec<Commit>> {
    let f: Vec<_> = records(bytes)?.collect();
    if f.len() % 11 != 0 {
        return Err(GitError("Malformed history fields".into()));
    }
    Ok(f.chunks_exact(11)
        .map(|f| Commit {
            oid: text(f[0]).trim_start_matches(['-', '>', '<']).into(),
            boundary: f[0].starts_with(b"-"),
            annotated: !f[10].is_empty(),
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
fn parse_raw_identity(ident: &str) -> Result<(String, String, String)> {
    let (ident, time) = ident
        .rsplit_once("> ")
        .ok_or_else(|| GitError("Invalid raw author header".into()))?;
    let (name, email) = ident
        .rsplit_once(" <")
        .ok_or_else(|| GitError("Invalid raw author identity".into()))?;
    Ok((
        name.into(),
        email.into(),
        crate::date::raw(time).map_err(GitError)?,
    ))
}

/// Read Git's --pretty=raw stream or Tig's NUL-separated one-line records.
/// Keep these records owned so column toggles can redraw without rereading stdin.
pub fn parse_raw_history(input: &str) -> Result<Vec<Commit>> {
    let mut commits: Vec<Commit> = Vec::new();
    let mut in_header = false;
    let valid_oid =
        |oid: &str| matches!(oid.len(), 40 | 64) && oid.bytes().all(|c| c.is_ascii_hexdigit());
    // C's line reader removes LF only; a trailing CR is subject data.
    for line in input.split_terminator('\n') {
        if let Some(header) = line.strip_prefix("commit ") {
            let fields: Vec<_> = header.split('\0').collect();
            let compact = fields.len() > 1;
            if compact && fields.len() != 4 {
                return Err(GitError("Invalid compact raw commit fields".into()));
            }
            // Git's %m marker is separate from the ID, never part of a revision.
            let header = fields[0];
            let boundary = header.starts_with('-');
            let header = header.strip_prefix(['<', '>', '-']).unwrap_or(header);
            let valid_id = |id: &str| {
                valid_oid(id)
                    || (compact
                        && (4..=64).contains(&id.len())
                        && id.bytes().all(|c| c.is_ascii_hexdigit()))
            };
            let mut ids = header.split_ascii_whitespace();
            let oid = ids
                .next()
                .ok_or_else(|| GitError("Missing raw commit ID".into()))?;
            if !valid_id(oid) {
                return Err(GitError("Invalid raw commit ID".into()));
            }
            let parents: Vec<String> = ids.map(str::to_owned).collect();
            if !parents.iter().all(|id| valid_id(id)) {
                return Err(GitError("Invalid raw parent ID".into()));
            }
            let mut commit = Commit {
                oid: oid.into(),
                boundary,
                annotated: false,
                parents,
                author: String::new(),
                date: String::new(),
                author_email: String::new(),
                committer: String::new(),
                committer_email: String::new(),
                committer_date: String::new(),
                subject: String::new(),
                decorations: String::new(),
            };
            if compact {
                (commit.author, commit.author_email, commit.date) = parse_raw_identity(fields[1])?;
                (
                    commit.committer,
                    commit.committer_email,
                    commit.committer_date,
                ) = parse_raw_identity(fields[2])?;
                commit.subject = fields[3].into();
            }
            commits.push(commit);
            in_header = !compact;
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
                let (name, email, date) = parse_raw_identity(ident)?;
                if kind == "author" {
                    (commit.author, commit.author_email, commit.date) = (name, email, date);
                } else {
                    (
                        commit.committer,
                        commit.committer_email,
                        commit.committer_date,
                    ) = (name, email, date);
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
            parse_nul_commit_metadata(format!("{id}\0\0Author\02020-01-01T00:00:00+00:00\0Title\0stale\0a@b\0Author\0a@b\02020-01-01T00:00:00+00:00\0\0").as_bytes()).unwrap().remove(0)
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
        assert!(HistoryOptions::parse_for_view(&["--grep-reflog=checkout".into()], true).is_ok());
        assert!(HistoryOptions::parse_for_view(
            &["--grep-reflog".into(), "moving from main to topic".into()],
            true
        )
        .is_ok());
        assert!(HistoryOptions::parse_for_view(&["--grep-reflog".into()], true).is_err());
        assert!(HistoryOptions::parse_for_view(&["--format=oops".into()], true).is_err());
        assert!(HistoryOptions::parse(&["--grep-reflog=checkout".into()]).is_err());
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
            repo.history_ordered(&[], 0, order, "no")
                .unwrap()
                .into_iter()
                .map(|commit| commit.subject)
                .collect::<Vec<_>>()
        };
        assert_eq!(subjects("topo")[1..3], ["More featuresA", "More master"]);
        assert_eq!(subjects("date")[1..3], ["More master", "More featuresA"]);
        assert!(repo.history_ordered(&[], 0, "other", "no").is_err());
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
    fn grep_hits_keep_their_original_blob_after_head_or_index_moves() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        fs::write(fixture.0.join("file"), b"needle old\n").unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command(["commit", "-qm", "old"]).unwrap();
        let from_head = repo
            .grep(&["needle".into(), "HEAD".into()])
            .unwrap()
            .into_iter()
            .flatten()
            .next()
            .unwrap();
        let from_index = repo
            .grep(&["--cached".into(), "needle".into()])
            .unwrap()
            .into_iter()
            .flatten()
            .next()
            .unwrap();
        assert!(from_head.source_oid.is_some() && from_index.source_oid.is_some());
        fs::write(fixture.0.join("file"), b"new version\n").unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command(["commit", "-qam", "new"]).unwrap();
        assert_eq!(repo.grep_blob(&from_head).unwrap(), b"needle old\n");
        assert_eq!(repo.grep_blob(&from_index).unwrap(), b"needle old\n");
    }
    #[test]
    fn cli_classification_preserves_filters_boundaries_and_path_bytes() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        fs::write(fixture.0.join("file\nname"), "base").unwrap();
        fs::create_dir(fixture.0.join("subdir")).unwrap();
        fs::write(fixture.0.join("subdir/file"), "base").unwrap();
        repo.command(["add", "."]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();
        let classify = |args: &[&str]| {
            classify_cli_args(
                &fixture.0,
                &args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>(),
            )
            .unwrap()
        };
        assert_eq!(
            classify(&["--exclude=refs/heads/other", "--all", "--", "subdir"]),
            ["--exclude=refs/heads/other", "--all", "--", "subdir"]
        );
        assert_eq!(classify(&["subdir"]), ["--", "subdir"]);
        for name in ["a", "b", "a\nb"] {
            fs::write(fixture.0.join(name), "base").unwrap();
        }
        repo.command(["add", "."]).unwrap();
        repo.command(["commit", "-qm", "ambiguous LF boundaries"])
            .unwrap();
        assert_eq!(classify(&["a", "b", "a\nb"]), ["--", "a", "b", "a\nb"]);

        assert_eq!(
            classify(&["HEAD", "file\nname"]),
            ["HEAD", "--", "file\nname"]
        );
        assert_eq!(
            classify(&["--", "file\nname", "--output=sentinel"]),
            ["--", "file\nname", "--output=sentinel"]
        );
        assert_eq!(
            classify(&["--grep", "base\nbody", "HEAD"]),
            ["--grep", "base\nbody", "HEAD"]
        );
        assert_eq!(
            classify(&["--end-of-options", "HEAD", "--", "file\nname"]),
            ["--end-of-options", "HEAD", "--", "file\nname"]
        );
        assert!(classify_cli_args(&fixture.0, &["missing-revision".into()]).is_err());
        let sub = Repository::discover(fixture.0.join("subdir")).unwrap();
        assert_eq!(sub.root, repo.root);
        assert_eq!(sub.git_dir, repo.git_dir);
        assert_eq!(sub.prefix().unwrap(), Path::new("subdir"));
    }
    #[test]
    fn discovery_preserves_repository_directory_newlines() {
        let fixture = Fixture::new();
        let directory = fixture.0.join("prefix\ntrue\nname");
        fs::create_dir(&directory).unwrap();
        run(&directory, ["init", "--quiet"]).unwrap();
        let child = directory.join("sub\nfalse\nname");
        fs::create_dir(&child).unwrap();
        let discovered = Repository::discover(&child).unwrap();
        assert_eq!(discovered.root, directory.canonicalize().unwrap());
        assert_eq!(
            discovered.git_dir,
            directory.join(".git").canonicalize().unwrap()
        );
        assert_eq!(discovered.prefix().unwrap(), Path::new("sub\nfalse\nname"));
    }
    #[test]
    fn discovery_accepts_unborn_and_bare_repositories() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        assert!(!repo.bare);
        assert!(repo.history(&[], 0).unwrap().is_empty());
        let bare = fixture.0.join("bare.git");
        repo.command([OsStr::new("init"), OsStr::new("--bare"), bare.as_os_str()])
            .unwrap();
        let discovered = Repository::discover(&bare).unwrap();
        assert!(discovered.bare);
        assert_eq!(discovered.root, bare.canonicalize().unwrap());
        assert_eq!(discovered.root, discovered.git_dir);
        assert!(discovered.history(&[], 0).unwrap().is_empty());
        assert!(Repository::discover(std::env::temp_dir()).is_err());
    }

    #[test]
    fn history_keeps_control_subjects_and_multiline_option_values() {
        let f = Fixture::new();
        let repo = f.repo();
        fs::write(f.0.join("file"), b"content").unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command(["commit", "-qm", "before\u{3}after"]).unwrap();
        assert_eq!(repo.history(&[], 0).unwrap()[0].subject, "before\u{3}after");
        assert!(repo
            .history(&["--grep".into(), "before\nafter".into()], 0)
            .is_ok());
    }
    #[test]
    fn note_free_authority_preserves_boundary_and_closed_stdin_selection() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        for content in ["base", "head"] {
            fs::write(fixture.0.join("file"), content).unwrap();
            repo.command(["add", "."]).unwrap();
            repo.command(["commit", "-qm", content]).unwrap();
        }
        let args = ["--boundary", "HEAD^..HEAD", "--", "file"].map(str::to_owned);
        let boundary = repo.history_ordered(&args, 0, "topo", "yes").unwrap();
        assert_eq!(boundary.len(), 2);
        assert_eq!(boundary[0].subject, "head");
        assert!(boundary[1].boundary);
        assert_eq!(boundary[1].subject, "base");
        let input = repo
            .history_ordered(&["--stdin".into()], 0, "topo", "yes")
            .unwrap();
        assert_eq!(
            input.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["head", "base"]
        );
    }

    #[test]
    fn merge_history_keeps_implicit_boundary_records_with_notes_enabled() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        fs::write(fixture.0.join("file"), "base\n").unwrap();
        repo.command(["add", "."]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();
        let branch = path(trim_lf(
            &repo.command(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        ))
        .unwrap()
        .into_os_string();
        repo.command(["checkout", "-qb", "topic"]).unwrap();
        fs::write(fixture.0.join("file"), "topic\n").unwrap();
        repo.command(["commit", "-qam", "topic"]).unwrap();
        repo.command([OsStr::new("checkout"), &branch]).unwrap();
        fs::write(fixture.0.join("file"), "head\n").unwrap();
        repo.command(["commit", "-qam", "head"]).unwrap();
        assert!(repo.command(["merge", "--no-edit", "topic"]).is_err());
        let commits = repo
            .history_ordered(
                &["--merge".into(), "--".into(), "file".into()],
                0,
                "topo",
                "yes",
            )
            .unwrap();
        let native = repo
            .command(["log", "--merge", "--boundary", "--format=%H", "--", "file"])
            .unwrap();
        assert_eq!(
            commits.iter().map(|c| c.oid.as_str()).collect::<Vec<_>>(),
            text(&native).split_terminator('\n').collect::<Vec<_>>()
        );
        assert!(commits.iter().any(|c| c.boundary));
    }

    #[test]
    fn history_refresh_cancels_queries_without_blocking_poll() {
        // Cover both cancellation during the shell's startup fork and after its
        // pipe-owning descendant exists. Timing assertions stay identical.
        for wait_for_descendant in [false, true] {
            let cancellation = std::sync::Arc::new(HistoryCancellation {
                cancelled: std::sync::atomic::AtomicBool::new(false),
                child: std::sync::Mutex::new(None),
            });
            let fixture = Fixture::new();
            let ready = fixture.0.join("descendant-ready");
            let marker = ready.clone();
            let process = cancellation.clone();
            let (completed, completion) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                HISTORY_PROCESS.with(|state| *state.borrow_mut() = Some(process));
                let output = command_output(
                    Command::new("sh")
                        .args([
                            "-c",
                            "sleep 30 & printf '%s\\n' \"$!\" > \"$1\"; wait",
                            "tig-cancel-fixture",
                        ])
                        .arg(marker),
                    None,
                )?;
                assert!(!output.status.success());
                completed.send(()).unwrap();
                Ok(Vec::new())
            });
            let mut refresh = HistoryRefresh {
                worker: Some(worker),
                cancellation,
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while refresh.cancellation.child.lock().unwrap().is_none()
                || (wait_for_descendant
                    && !fs::read(&ready).is_ok_and(|bytes| bytes.ends_with(b"\n")))
            {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let started = std::time::Instant::now();
            assert!(refresh.poll().unwrap().is_none());
            drop(refresh);
            assert!(started.elapsed() < std::time::Duration::from_millis(500));
            completion
                .recv_timeout(std::time::Duration::from_secs(5).saturating_sub(started.elapsed()))
                .unwrap();
        }
    }

    #[test]
    fn history_notes_cannot_inject_commits_or_metadata() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        for name in ["--show-notes.txt", "--pretty=foo"] {
            fs::write(fixture.0.join(name), "content").unwrap();
        }
        repo.command(["add", "."]).unwrap();
        for subject in ["real base", "real head"] {
            repo.command(["commit", "--allow-empty", "-qm", subject])
                .unwrap();
        }
        let base = repo.revision("HEAD^").unwrap();
        let spoof = format!("\x03\ncommit > {base}\0Fake <fake@invalid> 0 +0000\0Fake <fake@invalid> 0 +0000\0injected\0\x03");
        let note = text(trim_lf(
            &run_with_input(
                &repo.root,
                ["hash-object", "-w", "--stdin"],
                Some(spoof.as_bytes()),
            )
            .unwrap(),
        ));
        let (command, unborn, first_parent, mut wire) =
            repo.history_command(&[], 0, "topo", "yes").unwrap();
        repo.command(["notes", "add", "-C", &note, "HEAD"]).unwrap();
        let primary = run_command_with_input(command, None).unwrap();
        let raced = repo
            .finish_history(&primary, unborn, first_parent, &mut wire)
            .unwrap();
        assert_eq!(raced.len(), 2);
        assert_eq!(raced[0].subject, "real head");
        assert!(raced.iter().all(|c| c.author == "Test User"));

        let commits = repo.history_ordered(&[], 0, "topo", "yes").unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(
            commits
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            ["real head", "real base"]
        );
        assert!(commits[0].annotated && !commits[1].annotated);
        assert!(commits
            .iter()
            .all(|c| c.author == "Test User" && c.committer == "Test User"));
        for name in ["--show-notes.txt", "--pretty=foo"] {
            let filtered = repo
                .history_ordered(&["--".into(), name.into()], 0, "topo", "yes")
                .unwrap();
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].subject, "real base");
            let followed = repo
                .history_ordered(
                    &["--follow".into(), "--".into(), name.into()],
                    0,
                    "topo",
                    "yes",
                )
                .unwrap();
            assert_eq!(followed.len(), 1);
            assert_eq!(followed[0].subject, "real base");
            let implicit = repo.history_ordered(&[name.into()], 0, "topo", "yes");
            assert!(implicit.is_err()); // An option-like path always requires --.
        }
        repo.command(["config", "core.notesRef", "refs/notes/custom"])
            .unwrap();
        repo.command(["notes", "add", "-C", &note, "HEAD"]).unwrap();
        for setting in ["yes", "notes/custom"] {
            let commits = repo.history_ordered(&[], 0, "topo", setting).unwrap();
            assert_eq!(commits.len(), 2);
            assert_eq!(commits[0].subject, "real head");
            assert!(commits[0].annotated && !commits[1].annotated);
        }
        let mut refresh = repo.start_history(&[], "topo", "yes").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(commits) = refresh.poll().unwrap() {
                assert_eq!(commits.len(), 2);
                assert_eq!(commits[0].subject, "real head");
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn history_notes_display_refs_match_native_git() {
        let fixture = Fixture::new();
        let repo = fixture.repo();
        repo.command(["commit", "--allow-empty", "-qm", "base"])
            .unwrap();
        let base = repo.revision("HEAD").unwrap();
        repo.command(["notes", "--ref=review/topic", "add", "-m", "nested"])
            .unwrap();
        repo.command(["commit", "--allow-empty", "-qm", "head"])
            .unwrap();
        let head = repo.revision("HEAD").unwrap();
        repo.command(["notes", "add", "-m", "default"]).unwrap();
        let empty = text(trim_lf(
            &run_with_input(&repo.root, ["hash-object", "-w", "--stdin"], Some(b"")).unwrap(),
        ));
        repo.command([
            "notes",
            "--ref=empty",
            "add",
            "--allow-empty",
            "-C",
            &empty,
            "HEAD",
        ])
        .unwrap();
        assert!(!repo
            .command(["notes", "--ref=empty", "list"])
            .unwrap()
            .is_empty());
        assert!(!repo.history_annotations("empty").unwrap().is_empty()); // default note remains visible
        repo.command(["config", "core.notesRef", "refs/notes/empty"])
            .unwrap();
        assert!(repo.history_annotations("yes").unwrap().is_empty());
        repo.command(["config", "--unset", "core.notesRef"])
            .unwrap();
        for pattern in [
            "refs/notes/review/*",
            "refs/notes/*",
            "refs/notes/**",
            "refs/notes/[[:alpha:]]*",
            "refs/notes/[!a]*",
            "refs/notes/r?view/*",
            "refs/notes/review/",
            "refs/notes/review/topic",
        ] {
            repo.command(["config", "notes.displayRef", pattern])
                .unwrap();
            let annotations = repo.history_annotations("yes").unwrap();
            for oid in [&base, &head] {
                let native = repo
                    .command(["show", "--no-patch", "--show-notes", "--format=%N", oid])
                    .unwrap();
                assert_eq!(
                    annotations.contains(oid),
                    !trim_lf(&native).is_empty(),
                    "{pattern}: {oid}"
                );
            }
        }
        assert!(repo
            .history_annotations("notes/review/topic")
            .unwrap()
            .contains(&base));
        let mapped = repo
            .history_ordered(&[], 0, "topo", "notes/review/topic")
            .unwrap();
        assert!(mapped.iter().find(|c| c.oid == base).unwrap().annotated);
        repo.command(["config", "core.notesRef", "refs/notes/review/topic"])
            .unwrap();
        repo.command(["config", "--unset-all", "notes.displayRef"])
            .unwrap();
        assert_eq!(
            repo.history_annotations("yes").unwrap(),
            [base].into_iter().collect()
        );
    }

    #[test]
    fn history_notes_follow_selected_ref_without_leaking_into_fields() {
        let f = Fixture::new();
        let repo = f.repo();
        repo.command(["commit", "--allow-empty", "-qm", "base"])
            .unwrap();
        repo.command(["commit", "--allow-empty", "-qm", "noted"])
            .unwrap();
        repo.command(["notes", "add", "-m", "review\n\ncommit fake\n\u{3}text"])
            .unwrap();
        repo.command(["notes", "--ref=review", "add", "-m", "custom", "HEAD^"])
            .unwrap();
        for (notes, expected) in [
            ("yes", [true, false]),
            ("no", [false, false]),
            ("false", [false, false]),
            ("0", [false, false]),
            ("refs/notes/review", [true, true]),
            ("refs/notes/missing", [true, false]),
        ] {
            let commits = repo.history_ordered(&[], 0, "topo", notes).unwrap();
            assert_eq!(
                commits.iter().map(|c| c.annotated).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                commits
                    .iter()
                    .map(|c| c.subject.as_str())
                    .collect::<Vec<_>>(),
                ["noted", "base"]
            );
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
    fn compact_raw_history_and_untrusted_fields() {
        let input = include_str!("../test/main/escape-control-characters-test.in");
        let commits = parse_raw_history(input).unwrap();
        assert_eq!(commits.len(), 19);
        assert_eq!(commits[0].oid, "7363156");
        assert_eq!(commits[0].parents, ["e75e9f3"]);
        assert_eq!(commits[0].author, "a");
        assert_eq!(commits[0].author_email, "b.c");
        assert_eq!(commits[0].date, "2015-08-19T11:12:48-07:00");
        assert_eq!(commits[0].committer_date, commits[0].date);
        assert!(commits[18].parents.is_empty());
        assert_eq!(
            crate::render::sanitize(&commits[1].subject),
            "extend conditional group GBM_BO_USE_LINEAR  over both usages"
        );
        let record = input.lines().next().unwrap();
        assert!(parse_raw_history(&format!("{record}\r\n")).unwrap()[0]
            .subject
            .ends_with('\r'));
        for marker in ["<", ">", "-", ""] {
            let commit = parse_raw_history(&record.replacen(">", marker, 1))
                .unwrap()
                .remove(0);
            assert_eq!(commit.boundary, marker == "-");
            assert_eq!(commit.oid, "7363156");
        }
        for invalid in [
            record.replace("7363156", "--output=/tmp/injected"),
            record.replace("e75e9f3", "HEAD^{tree}"),
            record.replace("7363156", "abc"),
            record.replace("7363156", &"a".repeat(65)),
            record.replace("1440007968 -0700", "broken"),
            record.replace("a <b.c>", "broken"),
            record.rsplit_once('\0').unwrap().0.to_owned(),
            format!("{record}\0extra"),
            record.replacen('>', "!", 1),
        ] {
            assert!(parse_raw_history(&invalid).is_err(), "{invalid:?}");
        }
        let hostile = record.replace("correctly v2", "\x1b[2J\x07\r\tend");
        let commits = parse_raw_history(&hostile).unwrap();
        let mut config = crate::config::Config::default();
        config.parse("set main-view = commit-title:yes,graph=no,refs=no");
        let rows = crate::render::render_commits(&config, &commits, 100).unwrap();
        assert!(rows[0].contains("[2J   end"));
        assert!(!rows[0].chars().any(char::is_control));
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
                let show = repo
                    .show("HEAD", context, word, &[], (None, &[]), 80)
                    .unwrap();
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
        repo.command(["config", "diff.external", "false"]).unwrap();
        assert!(repo
            .show("HEAD", 3, false, &["--src-prefix".into()], (None, &[]), 80)
            .unwrap()
            .contains("-line 10\n+changed 10"));
        assert!(repo
            .show("HEAD", 3, false, &["--ext-diff".into()], (None, &[]), 80)
            .is_err());
        assert!(repo
            .show(
                "HEAD",
                3,
                true,
                &["--word-diff=none".into()],
                (None, &[]),
                80
            )
            .unwrap()
            .contains("[-line-]{+changed+} 10"));
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
            .blame(Some("HEAD"), Path::new(":(glob)*"), &[])
            .unwrap();
        assert_eq!(blame.len(), 2);
        assert_eq!(blame[1].line, 2);
        assert!(repo
            .show("HEAD", 3, false, &[], (None, &[]), 80)
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
        assert!(repo
            .show("--output=oops", 3, false, &[], (None, &[]), 80)
            .is_err());
        assert!(repo.history(&["--format=oops".into()], 1).is_err());
    }
}
