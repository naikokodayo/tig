// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Safe Rust migration of Tig. SPDX-License-Identifier: GPL-2.0-or-later
#![forbid(unsafe_code)]
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers, MouseEventKind},
    execute, queue,
    style::{Attribute, SetAttribute},
    terminal::{self, Clear, ClearType},
};
use regex::RegexBuilder;
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, IsTerminal, Write},
    path::{Component, PathBuf},
    process::{Command, Stdio},
};
use tig_rs::{
    config::{Cli, Config},
    git::Repository,
    help_view::HelpView,
    model::{BlameLine, Commit, StatusEntry, TreeEntry},
};
use unicode_width::UnicodeWidthChar;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const HELP: &str = "Tig Rust migration (compatibility work in progress)\n\nUsage: tig [-C path] [log|show|reflog|blame|grep|refs|stash|status] [arguments]\n       git show | tig\n\nKeys: j/k move, Enter open, q back/quit, Q quit, / search, n next match\n      m history, d diff, s status, t tree, r refs, b blame, h help, R refresh\n      u stage/unstage selected file in status; horizontal arrows scroll\n\nThis version is not yet a drop-in replacement for upstream Tig. See MIGRATION.md.";

#[derive(Clone)]
enum Item {
    Commit(Commit),
    Changes(ChangeKind),
    Status(StatusEntry, bool),
    Tree(TreeEntry),
    Ref(String, Option<String>),
    Grep(GrepLine),
    Blame(BlameLine),
    Text,
}
#[derive(Clone)]
struct GrepLine {
    label: String,
    path: PathBuf,
    revision: Option<String>,
    line: usize,
    text: String,
}

fn safe_grep_path(path: &std::path::Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn grep_revision_args(args: &[String]) -> Vec<String> {
    let before_paths: Vec<&String> = args.iter().take_while(|arg| arg.as_str() != "--").collect();
    if before_paths.iter().any(|arg| arg.starts_with('-')) {
        return Vec::new();
    }
    before_paths.into_iter().skip(1).cloned().collect()
}

fn grep_has_leading_delimiter(args: &[String]) -> bool {
    let mut pattern_seen = false;
    let mut pattern_next = false;
    for arg in args {
        if pattern_next {
            if arg == "--" {
                return true;
            }
            pattern_seen = true;
            pattern_next = false;
        } else if arg == "--" {
            return !pattern_seen;
        } else if matches!(arg.as_str(), "-e" | "--regexp" | "-f" | "--file") {
            pattern_next = true;
        } else if arg.starts_with("-e")
            || arg.starts_with("--regexp=")
            || arg.starts_with("-f")
            || arg.starts_with("--file=")
            || !arg.starts_with('-')
        {
            pattern_seen = true;
        }
    }
    false
}

fn grep_tree_oid(repo: &Repository, revision: &str) -> Result<String> {
    let object = repo.command(["rev-parse", "--verify", "--end-of-options", revision])?;
    let object = std::str::from_utf8(&object)?.trim();
    let tree = repo.command([
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("{object}^{{tree}}"),
    ])?;
    Ok(std::str::from_utf8(&tree)?.trim().to_owned())
}

fn unsupported_grep_option(args: &[String]) -> Option<&str> {
    let mut pattern_next = false;
    for arg in args.iter().take_while(|arg| arg.as_str() != "--") {
        if pattern_next {
            pattern_next = false;
            continue;
        }
        if matches!(arg.as_str(), "-e" | "--regexp" | "-f" | "--file") {
            pattern_next = true;
            continue;
        }
        if arg.starts_with('-')
            && !matches!(
                arg.as_str(),
                "-i" | "--ignore-case"
                    | "-w"
                    | "--word-regexp"
                    | "-v"
                    | "--invert-match"
                    | "-F"
                    | "--fixed-strings"
                    | "-E"
                    | "--extended-regexp"
                    | "-P"
                    | "--perl-regexp"
                    | "-G"
                    | "--basic-regexp"
            )
            && !arg.starts_with("-e")
            && !arg.starts_with("--regexp=")
            && !arg.starts_with("-f")
            && !arg.starts_with("--file=")
        {
            return Some(arg);
        }
    }
    None
}

fn ambiguous_grep_ref(hits: &[GrepLine], args: &[String]) -> bool {
    let optioned = args
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| arg.starts_with('-'));
    optioned
        && hits.iter().any(|hit| {
            args.iter()
                .take_while(|arg| arg.as_str() != "--")
                .any(|arg| {
                    hit.label
                        .strip_prefix(arg)
                        .is_some_and(|rest| rest.starts_with(':'))
                })
        })
}

fn grep_rows(bytes: &[u8], revisions: &[String]) -> Result<Vec<GrepLine>> {
    let mut rows = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let file_end = rest
            .iter()
            .position(|&byte| byte == 0)
            .ok_or("Incomplete git grep filename")?;
        let file = &rest[..file_end];
        rest = &rest[file_end + 1..];
        let line_end = rest
            .iter()
            .position(|&byte| byte == 0)
            .ok_or("Incomplete git grep line number")?;
        let line = std::str::from_utf8(&rest[..line_end])?.parse::<usize>()?;
        if line == 0 {
            return Err("Invalid git grep line number".into());
        }
        rest = &rest[line_end + 1..];
        let text_end = rest
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or("Incomplete git grep match text")?;
        let text = String::from_utf8_lossy(&rest[..text_end]).into_owned();
        rest = &rest[text_end + 1..];
        let (revision, path) = revisions
            .iter()
            .filter_map(|rev| {
                file.strip_prefix(rev.as_bytes())
                    .and_then(|rest| rest.strip_prefix(b":"))
                    .map(|path| (rev, path))
            })
            .max_by_key(|(rev, _)| rev.len())
            .map(|(rev, path)| (Some(rev.clone()), path))
            .unwrap_or((None, file));
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(path.to_vec()))
        };
        #[cfg(not(unix))]
        let path = PathBuf::from(String::from_utf8(path.to_vec())?);
        rows.push(GrepLine {
            label: String::from_utf8_lossy(file).into_owned(),
            path,
            revision,
            line,
            text,
        });
    }
    Ok(rows)
}

fn grep_columns(config: &Config) -> (bool, Option<usize>, Option<usize>, bool, usize) {
    let mut show_file = false;
    let mut file_width = None;
    let mut file_maxwidth = None;
    let mut show_line = true;
    let mut interval = 1;
    if let Some(columns) = config.settings.get("grep-view") {
        for spec in columns {
            let mut parts = spec.split([':', ',']);
            match parts.next() {
                Some("file-name") => {
                    for part in parts {
                        if part == "yes" || part == "always" {
                            show_file = true;
                        } else if part == "no" {
                            show_file = false;
                        } else if let Some(width) = part.strip_prefix("width=") {
                            file_width = width.parse().ok();
                        } else if let Some(width) = part.strip_prefix("maxwidth=") {
                            file_maxwidth = width.parse().ok();
                        }
                    }
                }
                Some("line-number") => {
                    for part in parts {
                        if part == "no" {
                            show_line = false;
                        } else if let Some(value) = part.strip_prefix("interval=") {
                            interval = value.parse::<usize>().unwrap_or(1).max(1);
                        }
                    }
                }
                _ => (),
            }
        }
    }
    (show_file, file_width, file_maxwidth, show_line, interval)
}

fn grep_filename(label: &str, width: usize) -> String {
    let count = label.chars().count();
    if count > width {
        let mut text: String = label.chars().take(width.saturating_sub(1)).collect();
        text.push('~');
        text
    } else {
        format!("{label}{}", " ".repeat(width.saturating_sub(count)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChangeKind {
    Untracked,
    Unstaged,
    Staged,
}
impl ChangeKind {
    fn title(self) -> &'static str {
        match self {
            Self::Untracked => "Untracked changes",
            Self::Unstaged => "Unstaged changes",
            Self::Staged => "Staged changes",
        }
    }
}

fn changes(entries: &[StatusEntry], show_untracked: bool) -> Vec<ChangeKind> {
    let mut kinds = Vec::new();
    if show_untracked && entries.iter().any(|entry| entry.index == '?') {
        kinds.push(ChangeKind::Untracked);
    }
    if entries
        .iter()
        .any(|entry| !matches!(entry.worktree, ' ' | '?'))
    {
        kinds.push(ChangeKind::Unstaged);
    }
    if entries
        .iter()
        .any(|entry| entry.staged() && !entry.conflicted())
    {
        kinds.push(ChangeKind::Staged);
    }
    kinds
}

fn status_mark(entry: &StatusEntry, group: usize) -> Option<char> {
    match group {
        0 if entry.staged() && !entry.conflicted() => Some(entry.index),
        1 if entry.conflicted() => Some('U'),
        1 if !matches!(entry.worktree, ' ' | '?' | '!') => Some(entry.worktree),
        2 if entry.index == '?' => Some('?'),
        _ => None,
    }
}

fn changes_date() -> Result<String> {
    tig_rs::date::changes_date().map_err(Into::into)
}

fn changes_commit(kind: ChangeKind, parent: String, date: &str, oid: &str) -> Commit {
    Commit {
        oid: oid.into(),
        parents: vec![parent],
        author: "Not Committed Yet".into(),
        date: date.into(),
        author_email: "not.committed.yet".into(),
        committer: "Not Committed Yet".into(),
        committer_email: "not.committed.yet".into(),
        committer_date: date.into(),
        subject: kind.title().into(),
        decorations: String::new(),
    }
}
#[derive(Clone)]
struct View {
    name: String,
    rows: Vec<String>,
    items: Vec<Item>,
    line_numbers: Vec<usize>,
    selected: usize,
    top: usize,
    left: usize,
    revision: String,
    path: PathBuf,
    staged: bool,
    untracked: bool,
    raw_patch: Vec<u8>,
    from_stdin: bool,
    sort_field: Option<String>,
    sort_reverse: bool,
    args: Vec<String>,
}
impl View {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            rows: vec![],
            items: vec![],
            line_numbers: vec![],
            selected: 0,
            top: 0,
            left: 0,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            staged: false,
            untracked: false,
            raw_patch: Vec::new(),
            from_stdin: false,
            sort_field: None,
            sort_reverse: false,
            args: Vec::new(),
        }
    }
    fn push(&mut self, text: String, item: Item) {
        self.line_numbers.push(self.rows.len() + 1);
        self.rows.push(text);
        self.items.push(item);
    }
    fn text(name: &str, text: &str) -> Self {
        let mut v = Self::new(name);
        for line in text.lines() {
            v.push(line.into(), Item::Text);
        }
        v
    }
    fn redraw_stdin(&mut self, config: &Config, width: usize) -> Result<()> {
        let commits: Vec<_> = self
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Commit(commit) => Some(commit.clone()),
                _ => None,
            })
            .collect();
        // Like main_needs_graph(OPEN_STDIN), raw input has no generated graph.
        let mut config = config.clone();
        config
            .settings
            .insert("main-view-commit-title-graph".into(), vec!["no".into()]);
        self.rows = tig_rs::render::render_commits(&config, &commits, width)?;
        Ok(())
    }
    fn restore_status_selection(&mut self) {
        if self.name != "status" || (self.selected == 0 && self.top == 0) {
            return;
        }
        self.selected = (self.selected..self.items.len())
            .find(|&i| matches!(self.items[i], Item::Status(..)))
            .or_else(|| {
                (0..=self.selected.min(self.items.len().saturating_sub(1)))
                    .rev()
                    .find(|&i| matches!(self.items[i], Item::Status(..)))
            })
            .unwrap_or(0);
        self.top = self.top.min(self.selected);
    }
    fn move_by(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.rows.len().saturating_sub(1));
    }
}
struct App {
    repo: Option<Repository>,
    config: Config,
    view: View,
    help: Option<HelpView>,
    previous: Vec<View>,
    pending_command: Option<tig_rs::commands::PreparedCommand>,
    other: Option<View>,
    split: bool,
    parent_focused: bool,
    revision: String,
    path: PathBuf,
    args: Vec<String>,
    message: String,
    search: String,
    width: usize,
    height: usize,
}
impl App {
    fn repo(&self) -> Result<&Repository> {
        self.repo
            .as_ref()
            .ok_or_else(|| "Not in a Git repository".into())
    }
    fn load(&self, name: &str) -> Result<View> {
        let mut view = self.load_content(name)?;
        if self.view.name == name {
            view.sort_field = self.view.sort_field.clone();
            view.sort_reverse = self.view.sort_reverse;
        }
        view.args = self.args.clone();
        if name != "diff" {
            view.revision = self.revision.clone();
        }
        view.path = self.path.clone();
        view.staged = self.view.staged && name == "stage";
        view.untracked = self.view.untracked && matches!(name, "stage" | "status");
        Ok(view)
    }
    fn status_view(&self, untracked_only: bool) -> Result<View> {
        let repo = self.repo()?;
        let entries = repo.status()?;
        let mut v = View::new("status");
        v.untracked = untracked_only;
        v.push(repo.status_header()?, Item::Text);
        for (group, title) in [
            (0, "Changes to be committed:"),
            (1, "Changes not staged for commit:"),
            (2, "Untracked files:"),
        ] {
            if untracked_only && group != 2 {
                continue;
            }
            v.push(title.into(), Item::Text);
            let start = v.rows.len();
            for e in &entries {
                if let Some(mark) = status_mark(e, group) {
                    v.push(
                        format!("{} {}", mark, e.path.display()),
                        Item::Status(e.clone(), group == 0),
                    );
                }
            }
            if v.rows.len() == start {
                v.push("  (no files)".into(), Item::Text);
            }
        }
        Ok(v)
    }
    fn load_content(&self, name: &str) -> Result<View> {
        let mut v = View::new(name);
        if name == "help" {
            let help = self
                .help
                .clone()
                .unwrap_or_else(|| HelpView::new(&self.config, &self.view.name));
            for row in help.rows {
                v.push(row.text, Item::Text);
            }
            return Ok(v);
        }
        if name == "main" && self.view.from_stdin {
            let mut view = self.view.clone();
            view.redraw_stdin(&self.config, self.width)?;
            return Ok(view);
        }
        let repo = self.repo()?;
        match name {
            "main" => {
                let commits = repo.history(&self.args, 0)?;
                let head = if self.config.bool_value("show-changes", true) && !repo.bare {
                    repo.revision("HEAD").ok()
                } else {
                    None
                };
                let kinds = if head
                    .as_ref()
                    .is_some_and(|id| commits.iter().any(|c| c.oid == *id))
                {
                    changes(
                        &repo.status()?,
                        self.config.bool_value("show-untracked", true),
                    )
                } else {
                    Vec::new()
                };
                let date = if kinds.is_empty() {
                    String::new()
                } else {
                    changes_date()?
                };
                let null_id = head
                    .as_ref()
                    .map(|id| "0".repeat(id.len()))
                    .unwrap_or_default();
                let mut display = Vec::with_capacity(commits.len() + kinds.len());
                let mut items = Vec::with_capacity(commits.len() + kinds.len());
                for commit in commits {
                    if head.as_ref().is_some_and(|id| commit.oid == *id) {
                        for (index, kind) in kinds.iter().copied().enumerate() {
                            let parent = if index + 1 < kinds.len() {
                                null_id.clone()
                            } else {
                                commit.oid.clone()
                            };
                            display.push(changes_commit(kind, parent, &date, &null_id));
                            items.push(Item::Changes(kind));
                        }
                    }
                    items.push(Item::Commit(commit.clone()));
                    display.push(commit);
                }
                let rows = tig_rs::render::render_commits(&self.config, &display, self.width)?;
                for (row, item) in rows.into_iter().zip(items) {
                    v.push(row, item);
                }
            }
            "status" => return self.status_view(self.view.name == "status" && self.view.untracked),

            "tree" => {
                let sort = if self.view.name == name {
                    self.view.sort_field.as_deref()
                } else {
                    None
                };
                for row in tig_rs::tree_view::load(
                    repo,
                    &self.config,
                    &self.revision,
                    &self.path,
                    self.width,
                    sort,
                    self.view.name == name && self.view.sort_reverse,
                )? {
                    v.push(row.text, row.entry.map(Item::Tree).unwrap_or(Item::Text));
                }
                let headers = 1 + usize::from(!self.path.as_os_str().is_empty());
                v.line_numbers = (0..v.items.len())
                    .map(|i| (i + 1).saturating_sub(headers))
                    .collect();
                v.selected = usize::from(v.items.len() > 1);
            }
            "refs" => {
                let sort = if self.view.name == name {
                    self.view.sort_field.as_deref()
                } else {
                    None
                };
                for row in tig_rs::refs_view::load(
                    repo,
                    &self.config,
                    &self.args,
                    self.width,
                    sort.unwrap_or("ref"),
                    self.view.name == name && self.view.sort_reverse,
                )? {
                    let item = row
                        .reference
                        .map(|r| Item::Ref(r.oid, Some(r.name)))
                        .unwrap_or(Item::Text);
                    v.push(row.text, item);
                }
            }
            "blame" => {
                let blame = repo.blame(Some(&self.revision), &self.path)?;
                for (row, line) in tig_rs::render::render_blame(&self.config, &blame, self.width)?
                    .into_iter()
                    .zip(blame)
                {
                    v.push(row, Item::Blame(line));
                }
            }
            "diff" => {
                let oid = repo.revision(&self.revision)?;
                let mut view = View::text(
                    name,
                    &repo.show(
                        &oid,
                        self.config.usize_value("diff-context", 3),
                        self.config.bool_value("word-diff", false),
                    )?,
                );
                view.revision = oid.clone();
                if let Some(commit) = repo.history(&[oid], 1)?.first() {
                    let refs = tig_rs::render::refs(&self.config, &commit.decorations, ", ");
                    if !refs.is_empty() && !view.rows.is_empty() {
                        view.rows.insert(1, format!("Refs: {refs}"));
                        view.items.insert(1, Item::Text);
                        view.line_numbers = (1..=view.rows.len()).collect();
                    }
                }
                return Ok(view);
            }
            "stage" => {
                if self.view.untracked {
                    return Ok(View::text(
                        name,
                        &String::from_utf8_lossy(&fs::read(repo.root.join(&self.path))?),
                    ));
                }
                let raw = if !self.view.staged && self.path.as_os_str().is_empty() {
                    repo.worktree_diff_bytes(None)?
                } else {
                    repo.diff_bytes(
                        self.view.staged,
                        if self.path.as_os_str().is_empty() {
                            None
                        } else {
                            Some(&self.path)
                        },
                    )?
                };
                let mut view = View::text(name, &String::from_utf8_lossy(&raw));
                view.raw_patch = raw;
                return Ok(view);
            }
            "blob" => {
                let parent = self.path.parent().unwrap_or(std::path::Path::new(""));
                let entry = repo
                    .tree(&self.revision, parent)?
                    .into_iter()
                    .find(|entry| entry.path == self.path)
                    .ok_or("No selected blob")?;
                let bytes = repo.blob(&entry.oid)?;
                return Ok(View::text(name, &String::from_utf8_lossy(&bytes)));
            }
            "log" => {
                let mut args = vec![
                    "log".to_string(),
                    "--no-ext-diff".into(),
                    "--no-textconv".into(),
                    "--no-show-signature".into(),
                    "--pretty=medium".into(),
                ];
                if let Some(options) = self.config.settings.get("log-options") {
                    args.extend(options.clone());
                }
                args.extend(["--no-color".into(), "--decorate=full".into()]);
                args.extend(self.args.clone());
                let output = repo.command(&args)?;
                let text = String::from_utf8_lossy(&output);
                let mut revision = String::new();
                for line in text.lines() {
                    let indent = log_header_offset(line);
                    if let Some(indent) = indent {
                        let header = &line[indent + 7..];
                        let oid = header.split_whitespace().next().unwrap_or_default();
                        if oid.len() >= 40 && oid.bytes().all(|c| c.is_ascii_hexdigit()) {
                            revision = oid.into();
                            v.push(
                                format!("{}commit {oid}", &line[..indent]),
                                Item::Ref(revision.clone(), None),
                            );
                            if indent == 0 {
                                if let Some(decorations) = header
                                    .strip_prefix(oid)
                                    .and_then(|s| s.strip_prefix(" ("))
                                    .and_then(|s| s.strip_suffix(')'))
                                {
                                    let refs =
                                        tig_rs::render::refs(&self.config, decorations, ", ");
                                    if !refs.is_empty() {
                                        v.push(
                                            format!("Refs: {refs}"),
                                            Item::Ref(revision.clone(), None),
                                        );
                                    }
                                }
                            }
                            continue;
                        }
                    }
                    v.push(
                        line.into(),
                        if revision.is_empty() {
                            Item::Text
                        } else {
                            Item::Ref(revision.clone(), None)
                        },
                    );
                }
            }
            "grep" => {
                if grep_has_leading_delimiter(&self.args) {
                    return Err(
                        "Git grep '--' before or as a pattern is not supported in the Rust view"
                            .into(),
                    );
                }
                if let Some(option) = unsupported_grep_option(&self.args) {
                    return Err(format!(
                        "Git grep option '{option}' is not supported in the Rust view"
                    )
                    .into());
                }
                let output = Command::new("git")
                    .current_dir(&repo.root)
                    .args(["--no-pager", "--literal-pathspecs", "-c", "color.ui=false"])
                    .args(["grep", "--no-color", "-n", "-z", "--full-name", "-I"])
                    .args(&self.args)
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .env("LC_ALL", "C")
                    .stdin(Stdio::null())
                    .output()?;
                if !output.status.success() && output.status.code() != Some(1) {
                    return Err(format!(
                        "git grep exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    )
                    .into());
                }
                let revisions = grep_revision_args(&self.args);
                let hits = grep_rows(&output.stdout, &revisions)?;
                if ambiguous_grep_ref(&hits, &self.args) {
                    return Err(
                        "Grep revision paths with options are not supported in the Rust view"
                            .into(),
                    );
                }
                let (show_file, width, maxwidth, show_line, interval) = grep_columns(&self.config);
                let width = width
                    .unwrap_or_else(|| {
                        hits.iter()
                            .map(|hit| hit.label.chars().count())
                            .max()
                            .unwrap_or(0)
                    })
                    .min(maxwidth.unwrap_or(usize::MAX));
                let line_width = hits
                    .iter()
                    .map(|hit| hit.line.to_string().len())
                    .max()
                    .unwrap_or(3)
                    .max(3);
                let mut last_file = None;
                for hit in hits {
                    if !show_file && last_file.as_deref() != Some(hit.label.as_str()) {
                        let mut header = hit.clone();
                        header.line = 1;
                        header.text.clear();
                        v.push(hit.label.clone(), Item::Grep(header));
                    }
                    let mut row = String::new();
                    if show_file {
                        row.push_str(&grep_filename(&hit.label, width));
                        row.push(' ');
                    }
                    if show_line {
                        if hit.line == 1 || hit.line % interval == 0 {
                            row.push_str(&format!("{:>line_width$}", hit.line));
                        } else {
                            row.push_str(&" ".repeat(line_width));
                        }
                        row.push_str("| ");
                    }
                    row.push_str(&hit.text);
                    last_file = Some(hit.label.clone());
                    v.push(row, Item::Grep(hit));
                }
            }
            "reflog" | "stash" => {
                let mut args = vec![name.to_string()];
                if name == "stash" {
                    args.push("list".into());
                }
                if name != "stash" {
                    args.push("--no-color".into());
                }
                args.extend(self.args.clone());
                return Ok(View::text(
                    name,
                    &String::from_utf8_lossy(&repo.command(&args)?),
                ));
            }
            _ => return Err(format!("Unsupported view: {name}").into()),
        }
        Ok(v)
    }
    fn open(&mut self, name: &str) -> Result<()> {
        if name == "help" {
            self.help = Some(HelpView::new(&self.config, &self.view.name));
        }
        let next = self.load(name)?;
        self.previous.push(std::mem::replace(&mut self.view, next));
        Ok(())
    }
    fn open_changes(&mut self, kind: ChangeKind) -> Result<()> {
        self.revision = "HEAD".into();
        self.path.clear();
        let mut view = if kind == ChangeKind::Untracked {
            self.status_view(true)?
        } else {
            let staged = kind == ChangeKind::Staged;
            let raw = if staged {
                self.repo()?.diff_bytes(true, None)?
            } else {
                self.repo()?.worktree_diff_bytes(None)?
            };
            let mut view = View::text("stage", &String::from_utf8_lossy(&raw));
            view.staged = staged;
            view.raw_patch = raw;
            view
        };
        view.args = self.args.clone();
        view.revision = self.revision.clone();
        self.previous.push(std::mem::replace(&mut self.view, view));
        Ok(())
    }
    fn grep_query(&mut self, query: &str) -> Result<()> {
        let args: Vec<String> = query.split_whitespace().map(str::to_owned).collect();
        if args.is_empty() {
            return Ok(());
        }
        let previous_args = std::mem::replace(&mut self.args, args);
        let next = match self.load("grep") {
            Ok(next) => next,
            Err(error) => {
                self.args = previous_args;
                return Err(error);
            }
        };
        if self.view.name == "grep" {
            self.view = next;
        } else {
            self.previous.push(std::mem::replace(&mut self.view, next));
        }
        if self.view.rows.is_empty() {
            self.message = "No matches found".into();
        }
        Ok(())
    }
    fn refresh_main_parent(&mut self) -> Result<()> {
        let Some(old) = self.other.as_ref().filter(|view| view.name == "main") else {
            return Ok(());
        };
        let selected = old.items.get(old.selected).cloned();
        let top = old.top;
        let mut next = self.load("main")?;
        next.selected = selected
            .as_ref()
            .and_then(|selected| {
                next.items.iter().position(|item| match (selected, item) {
                    (Item::Changes(a), Item::Changes(b)) => a == b,
                    (Item::Commit(a), Item::Commit(b)) => a.oid == b.oid,
                    _ => false,
                })
            })
            .unwrap_or(0);
        next.top = top;
        self.other = Some(next);
        Ok(())
    }
    fn sync_context(&mut self) {
        self.args = self.view.args.clone();
        self.revision = self.view.revision.clone();
        self.path = self.view.path.clone();
    }
    fn swap_panes(&mut self) {
        if let Some(other) = &mut self.other {
            std::mem::swap(&mut self.view, other);
            self.parent_focused = !self.parent_focused;
            self.sync_context();
        }
    }
    fn selected(&self) -> Item {
        self.view
            .items
            .get(self.view.selected)
            .cloned()
            .unwrap_or(Item::Text)
    }
    fn select_context(&mut self) {
        match self.selected() {
            Item::Commit(c) => self.revision = c.oid,
            Item::Changes(_) => {
                self.revision = "HEAD".into();
                self.path.clear();
            }
            Item::Ref(id, _) => self.revision = id,
            Item::Blame(line) => self.revision = line.oid,
            Item::Tree(e) => self.path = e.path,
            Item::Status(e, _) => self.path = e.path,
            Item::Grep(hit) => {
                self.path = if safe_grep_path(&hit.path)
                    && !hit.revision.as_deref().is_some_and(|rev| rev.contains(':'))
                {
                    hit.path
                } else {
                    PathBuf::new()
                };
                self.revision = hit.revision.unwrap_or_else(|| "HEAD".into());
            }
            Item::Text => (),
        }
    }
    fn tree_parent(&mut self) -> Result<()> {
        let old = self.view.path.clone();
        if let Some(parent) = old.parent() {
            self.path = parent.to_path_buf();
            let saved = self
                .previous
                .iter()
                .rposition(|v| v.name == "tree" && v.path == parent);
            self.view = self.load("tree")?;
            if let Some(index) = self
                .view
                .items
                .iter()
                .position(|item| matches!(item, Item::Tree(entry) if entry.path == old))
            {
                self.view.selected = index;
            }
            if let Some(index) = saved {
                self.view.top = self.previous[index].top;
                self.previous.truncate(index);
            }
        }
        Ok(())
    }
    fn enter(&mut self) -> Result<()> {
        if self.view.name == "help" {
            if let Some(help) = &mut self.help {
                if help.toggle_section(self.view.selected, &self.config) {
                    self.view.rows = help.rows.iter().map(|row| row.text.clone()).collect();
                    self.view.items = vec![Item::Text; self.view.rows.len()];
                    self.view.line_numbers = (1..=self.view.rows.len()).collect();
                    self.view.selected = self
                        .view
                        .selected
                        .min(self.view.rows.len().saturating_sub(1));
                }
            }
            return Ok(());
        }
        if self.view.name == "diff"
            || (self.view.name == "stage" && self.view.path.as_os_str().is_empty())
        {
            let header = if self.view.name == "diff" {
                diff_stat_header(&self.view.rows, self.view.selected)
            } else {
                stage_stat_header(&self.view.rows, self.view.selected)
            };
            if let Some(header) = header {
                self.view.selected = header;
                self.center_selection();
                return Ok(());
            }
        }
        let parent = self.view.clone();
        let depth = self.previous.len();
        match self.selected() {
            Item::Commit(c) => {
                self.revision = c.oid;
                self.open("diff")?;
            }
            Item::Changes(kind) => self.open_changes(kind)?,
            Item::Ref(id, _) => {
                self.revision = id.clone();
                if self.view.name == "refs" {
                    self.args = vec![id];
                    self.open("main")?;
                } else {
                    self.open("diff")?;
                }
            }
            Item::Blame(line) => {
                self.revision = line.oid;
                self.open("diff")?;
            }
            Item::Text if self.view.name == "refs" && self.view.selected == 0 => {
                self.args = vec!["--all".into()];
                self.open("main")?;
            }
            Item::Tree(e) => {
                if e.kind == "tree" && self.view.path.parent() == Some(e.path.as_path()) {
                    return self.tree_parent();
                }
                self.path = e.path.clone();
                if e.kind == "tree" {
                    self.open("tree")?;
                    return Ok(());
                } else {
                    let bytes = self.repo()?.blob(&e.oid)?;
                    let mut v = View::text("blob", &String::from_utf8_lossy(&bytes));
                    v.path = self.path.clone();
                    v.revision = self.revision.clone();
                    self.previous.push(std::mem::replace(&mut self.view, v));
                }
            }
            Item::Status(e, staged) => {
                let raw = if e.index == '?' {
                    fs::read(self.repo()?.root.join(&e.path))?
                } else {
                    self.repo()?.diff_bytes(staged, Some(&e.path))?
                };
                let text = String::from_utf8_lossy(&raw);
                self.path = e.path;
                let mut view = View::text("stage", &text);
                view.path = self.path.clone();
                view.args = self.args.clone();
                view.revision = self.revision.clone();
                view.staged = staged;
                view.untracked = e.index == '?';
                if !view.untracked {
                    view.raw_patch = raw;
                }
                self.previous.push(std::mem::replace(&mut self.view, view));
            }
            Item::Grep(hit) => {
                if !safe_grep_path(&hit.path) {
                    return Err("Invalid grep result path".into());
                }
                let bytes = if let Some(rev) = &hit.revision {
                    let oid = grep_tree_oid(self.repo()?, rev)?;
                    let mut spec = OsString::from(format!("{oid}:"));
                    spec.push(hit.path.as_os_str());
                    self.repo()?.command(vec![
                        OsString::from("cat-file"),
                        OsString::from("blob"),
                        spec,
                    ])?
                } else {
                    fs::read(self.repo()?.root.join(&hit.path))?
                };
                self.path = if hit.revision.as_deref().is_some_and(|rev| rev.contains(':')) {
                    PathBuf::new()
                } else {
                    hit.path.clone()
                };
                self.revision = hit.revision.unwrap_or_else(|| "HEAD".into());
                let mut view = View::text("blob", &String::from_utf8_lossy(&bytes));
                view.path = self.path.clone();
                view.revision = self.revision.clone();
                view.selected = hit
                    .line
                    .saturating_sub(1)
                    .min(view.rows.len().saturating_sub(1));
                self.previous.push(std::mem::replace(&mut self.view, view));
            }
            Item::Text => (),
        }
        if self.previous.len() > depth {
            let from_grep = parent.name == "grep";
            self.previous.truncate(depth);
            self.other = Some(parent);
            self.split = true;
            self.parent_focused = false;
            if from_grep {
                self.center_selection();
            }
        }
        Ok(())
    }
    fn find(&mut self, backwards: bool) {
        let count = self.view.rows.len();
        if self.search.is_empty() {
            self.message = "No previous search".into();
            return;
        }
        if count == 0 {
            return;
        }
        let ignore_case = match self.config.value("ignore-case") {
            Some("yes") => true,
            Some("smart-case") => !self.search.chars().any(char::is_uppercase),
            _ => false,
        };
        let regex = match RegexBuilder::new(&self.search)
            .case_insensitive(ignore_case)
            .build()
        {
            Ok(regex) => regex,
            Err(error) => {
                self.message = format!("Search failed: {error}");
                return;
            }
        };
        let wrap = self.config.bool_value("wrap-search", true);
        let search_full_refs =
            self.view.name == "main" && tig_rs::render::main_refs_searchable(&self.config);
        for offset in 1..=count {
            let i = if backwards {
                (self.view.selected + count - offset) % count
            } else {
                (self.view.selected + offset) % count
            };
            if !wrap
                && ((backwards && i >= self.view.selected)
                    || (!backwards && i <= self.view.selected))
            {
                break;
            }
            let ref_match = search_full_refs
                && matches!(self.view.items.get(i), Some(Item::Commit(commit)) if regex.is_match(&commit.decorations));
            if regex.is_match(&self.view.rows[i]) || ref_match {
                self.view.selected = i;
                self.center_selection();
                return;
            }
        }
        self.message = format!("No match found for '{}'", self.search);
    }
    fn goto_commit(&mut self, target: &str) -> Result<()> {
        let target = self.repo()?.revision(target)?;
        let index = self
            .view
            .items
            .iter()
            .position(|item| match item {
                Item::Commit(commit) => commit.oid == target,
                _ => false,
            })
            .ok_or("Commit not in this view")?;
        self.view.selected = index;
        Ok(())
    }
    fn center_selection(&mut self) {
        let visible = if self.split && self.other.is_some() {
            let (vertical, parent, child) = self.pane_sizes();
            if vertical {
                self.height.saturating_sub(2)
            } else {
                (if self.parent_focused { parent } else { child }).saturating_sub(1)
            }
        } else {
            self.height.saturating_sub(2)
        }
        .max(1);
        if self.view.selected < self.view.top || self.view.selected >= self.view.top + visible {
            self.view.top = self.view.selected.saturating_sub(visible / 2);
        }
    }
    fn edit_target(&self) -> Option<(PathBuf, usize)> {
        match self.view.name.as_str() {
            "status" => match self.selected() {
                Item::Status(entry, _) => Some((entry.path, 0)),
                _ => None,
            },
            "tree" => match self.selected() {
                Item::Tree(entry) if entry.kind != "tree" => Some((entry.path, 0)),
                _ => None,
            },
            "grep" => match self.selected() {
                Item::Grep(hit)
                    if safe_grep_path(&hit.path)
                        && !hit.revision.as_deref().is_some_and(|rev| rev.contains(':')) =>
                {
                    Some((hit.path, hit.line))
                }
                _ => None,
            },
            "blob" => Some((self.view.path.clone(), self.view.selected + 1)),
            "blame" => match self.selected() {
                Item::Blame(line) if line.filename == self.view.path => {
                    Some((self.view.path.clone(), self.view.selected + 1))
                }
                _ => None,
            },
            "stage" if self.view.untracked => {
                Some((self.view.path.clone(), self.view.selected + 1))
            }
            "stage" => diff_edit_target(
                &self.view.rows,
                stage_stat_header(&self.view.rows, self.view.selected)
                    .unwrap_or(self.view.selected),
            ),
            "diff" | "log" | "pager" => diff_edit_target(&self.view.rows, self.view.selected),
            _ => None,
        }
    }
    fn edit(&mut self) -> Result<()> {
        let target = self.edit_target();
        let Some((path, line)) = target else {
            self.message = "Nothing to edit".into();
            return Ok(());
        };
        if !path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
            || !self.repo()?.root.join(&path).is_file()
        {
            self.message = format!("Failed to open file: {}", path.display());
            return Ok(());
        }
        let configured_editor = self
            .repo()?
            .command(["config", "--get", "core.editor"])
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned());
        let editor = env::var("TIG_EDITOR")
            .ok()
            .or_else(|| env::var("GIT_EDITOR").ok())
            .or(configured_editor)
            .or_else(|| env::var("VISUAL").ok())
            .or_else(|| env::var("EDITOR").ok())
            .unwrap_or_else(|| "vi".into());
        let mut argv = vec![
            "sh".into(),
            "-c".into(),
            format!("{editor} \"$@\"").into(),
            "tig-editor".into(),
        ];
        if line != 0 && self.config.bool_value("editor-line-number", true) {
            argv.push(format!("+{line}").into());
        }
        let editor_path = if path.to_string_lossy().starts_with('-') {
            PathBuf::from(".").join(path)
        } else {
            path
        };
        argv.push(editor_path.into_os_string());
        self.pending_command = Some(tig_rs::commands::PreparedCommand {
            argv,
            silent: false,
            confirm: false,
            exit: false,
            echo: false,
            quick: true,
        });
        Ok(())
    }
    fn action(&mut self, action: &str) -> Result<bool> {
        let action = action.strip_prefix(':').unwrap_or(action);
        if let Some(command) = action.strip_prefix("exec ").or_else(|| {
            action
                .starts_with(['!', '@', '?', '<', '+', '>'])
                .then_some(action)
        }) {
            let selected_ref = match self.selected() {
                Item::Ref(_, name) => name,
                _ => None,
            };
            self.select_context();
            let (file, line) = self.edit_target().unwrap_or_else(|| {
                (
                    if self.view.name == "blame" {
                        PathBuf::new()
                    } else {
                        self.path.clone()
                    },
                    0,
                )
            });
            self.pending_command = Some(tig_rs::commands::prepare_with_context(
                self.repo()?,
                command,
                &self.revision,
                &file,
                line,
                selected_ref.as_deref(),
            )?);
            return Ok(true);
        }
        if action.split_whitespace().next() == Some("save-options") {
            let args = tig_rs::config::words(action)?;
            let path = args.get(1).map_or("tig-options.txt", String::as_str);
            self.message = match self.config.save(std::path::Path::new(path)) {
                Ok(()) => format!("Saved options to {path}"),
                Err(error) => format!("Failed to save options: {error}"),
            };
            return Ok(true);
        }
        if matches!(action, "toggle sort-field" | "toggle sort-order") {
            if !matches!(self.view.name.as_str(), "refs" | "tree") {
                return Err("This view does not support sorting".into());
            }
            let old = self.view.clone();
            if action == "toggle sort-order" {
                self.view.sort_reverse = !self.view.sort_reverse;
            } else {
                let setting = format!("{}-view", self.view.name);
                let columns: Vec<&str> = self
                    .config
                    .settings
                    .get(&setting)
                    .ok_or("Missing view columns")?
                    .iter()
                    .filter(|spec| !spec.starts_with("id:no"))
                    .map(|spec| spec.split(':').next().unwrap_or(spec))
                    .collect();
                let current =
                    self.view
                        .sort_field
                        .as_deref()
                        .unwrap_or(if self.view.name == "refs" {
                            "ref"
                        } else {
                            "line-number"
                        });
                let next = columns
                    .iter()
                    .position(|&c| c == current)
                    .map_or(0, |i| (i + 1) % columns.len());
                self.view.sort_field = columns.get(next).map(|s| s.to_string());
            }
            self.action("refresh")?;
            for (index, item) in self.view.items.iter().enumerate() {
                if let Some(previous) =
                    old.items
                        .iter()
                        .position(|previous| match (previous, item) {
                            (Item::Ref(a, an), Item::Ref(b, bn)) => a == b && an == bn,
                            (Item::Tree(a), Item::Tree(b)) => a.path == b.path,
                            _ => false,
                        })
                {
                    self.view.line_numbers[index] = old.line_numbers[previous];
                }
            }
            return Ok(true);
        }
        if action.starts_with("toggle ")
            || action.starts_with("set ")
            || action.starts_with("bind ")
            || action.starts_with("color ")
            || action.starts_with("source ")
        {
            let mut config = self.config.clone();
            config.apply_command_for_view(&self.view.name, action)?;
            self.config = config;
            return self.action("refresh");
        }
        if let Some(pattern) = action.strip_prefix('/') {
            self.search = pattern.into();
            self.find(false);
            return Ok(true);
        }
        if !action.is_empty() && action.bytes().all(|byte| byte.is_ascii_digit()) {
            return self.action(&format!("goto {action}"));
        }
        if let Some(target) = action.strip_prefix("goto ") {
            if let Ok(line) = target.parse::<usize>() {
                self.view.selected = line
                    .saturating_sub(1)
                    .min(self.view.rows.len().saturating_sub(1));
            } else {
                self.goto_commit(target)?;
            }
            self.center_selection();
            return Ok(true);
        }
        if !action.is_empty()
            && action.bytes().all(|byte| byte.is_ascii_hexdigit())
            && action.len() >= 7
        {
            self.goto_commit(action)?;
            self.center_selection();
            return Ok(true);
        }
        let page = if self.split && self.other.is_some() {
            let (vertical, parent, child) = self.pane_sizes();
            if vertical {
                self.height.saturating_sub(2)
            } else {
                (if self.parent_focused { parent } else { child }).saturating_sub(1)
            }
        } else {
            self.height.saturating_sub(2)
        }
        .max(1) as isize;
        match action {
            "quit" => return Ok(false),
            "view-close" | "back" => {
                if self.other.is_some() {
                    if !self.parent_focused {
                        self.swap_panes();
                    }
                    self.other = None;
                    self.split = false;
                    self.parent_focused = false;
                } else if let Some(v) = self.previous.pop() {
                    self.revision = v.revision.clone();
                    self.path = v.path.clone();
                    self.view = v;
                } else {
                    return Ok(false);
                }
            }
            "enter" => self.enter()?,
            "view-next" => {
                if self.split {
                    self.swap_panes();
                } else {
                    self.message = "Only one view is displayed".into();
                }
            }
            "maximize" | "view-maximize" => self.split = false,
            "next" | "previous" if self.other.is_some() && !self.parent_focused => {
                let split = self.split;
                self.swap_panes();
                let old = self.view.selected;
                self.view.move_by(if action == "next" { 1 } else { -1 });
                if self.view.selected != old {
                    self.enter()?;
                }
                if self.parent_focused {
                    self.swap_panes();
                }
                self.split = split;
            }
            "move-down" | "next" => self.view.move_by(1),
            "move-up" | "previous" => self.view.move_by(-1),
            "move-page-down" => self.view.move_by(page),
            "move-page-up" => self.view.move_by(-page),
            "move-half-page-down" => self.view.move_by(page / 2),
            "move-half-page-up" => self.view.move_by(-page / 2),
            "move-first-line" => self.view.selected = 0,
            "move-last-line" => self.view.selected = self.view.rows.len().saturating_sub(1),
            "scroll-left" => self.view.left = self.view.left.saturating_sub(8),
            "scroll-right" => self.view.left = self.view.left.saturating_add(8),
            "scroll-first-col" => self.view.left = 0,
            "scroll-line-down" | "scroll-line-up" => {
                let max_top = self.view.rows.len().saturating_sub(page as usize);
                let next = if action == "scroll-line-down" {
                    self.view.top.saturating_add(1).min(max_top)
                } else {
                    self.view.top.saturating_sub(1)
                };
                if next == self.view.top {
                    self.message = format!(
                        "Cannot scroll beyond the {} line",
                        if action == "scroll-line-down" {
                            "last"
                        } else {
                            "first"
                        }
                    );
                } else {
                    self.view.selected = self
                        .view
                        .selected
                        .saturating_add_signed(if action == "scroll-line-down" { 1 } else { -1 })
                        .min(self.view.rows.len().saturating_sub(1));
                    self.view.top = next;
                }
            }
            "edit" => self.edit()?,
            "find-next" => self.find(false),
            "find-prev" => self.find(true),
            "refresh" => {
                self.sync_context();
                let old = self.view.clone();
                if old.name == "help" {
                    if let Some(help) = &mut self.help {
                        help.refresh(&self.config);
                    }
                }
                if old.name != "pager" {
                    self.view = self.load(&old.name)?;
                }
                self.view.selected = old.selected.min(self.view.rows.len().saturating_sub(1));
                self.view.top = old.top;
                self.view.left = old.left;
                self.view.restore_status_selection();
                if old.name == "diff" {
                    if let Some(selected) = diff_reloaded_line(&old, &self.view.rows) {
                        self.view.selected = selected;
                        self.view.top =
                            selected.saturating_sub(old.selected.saturating_sub(old.top));
                    }
                }
            }
            "status-update" | "stage-update-line" if self.view.name == "stage" => {
                if self.view.untracked {
                    if action != "status-update" {
                        return Err("Select a tracked diff to stage individual lines".into());
                    }
                    let entry = self
                        .repo()?
                        .status()?
                        .into_iter()
                        .find(|entry| entry.path == self.path)
                        .ok_or("File no longer in status")?;
                    self.repo()?.stage(&entry)?;
                } else {
                    let raw = &self.view.raw_patch;
                    let mut offset = 0;
                    for line in raw.split_inclusive(|byte| *byte == b'\n') {
                        if line.starts_with(b"diff --cc ") || line.starts_with(b"diff --combined ")
                        {
                            return Err("Staging a combined merge patch is unsupported".into());
                        }
                        if line.starts_with(b"diff --git ") {
                            break;
                        }
                        offset += line.len();
                    }
                    if offset == raw.len() {
                        return Err("No text patch selected".into());
                    }
                    let prefix_rows = raw[..offset].iter().filter(|byte| **byte == b'\n').count();
                    if action == "status-update"
                        && self.view.path.as_os_str().is_empty()
                        && self.view.selected < prefix_rows
                    {
                        tig_rs::patch::apply_cached(
                            self.repo()?,
                            &raw[offset..],
                            self.view.staged,
                        )?;
                    } else {
                        let patch = tig_rs::patch::Patch::parse(&raw[offset..])?;
                        let (file, hunk, line) =
                            patch.locate(self.view.selected.saturating_sub(prefix_rows))?;
                        let line = if action == "stage-update-line" {
                            Some(line.ok_or("Select an added or removed line")?)
                        } else {
                            None
                        };
                        let selected = patch.select(file, hunk, line, self.view.staged)?;
                        tig_rs::patch::apply_cached(self.repo()?, &selected, self.view.staged)?;
                    }
                }
                self.refresh_main_parent()?;
                self.action("refresh")?;
                if self.view.rows.is_empty() && self.other.is_some() {
                    self.action("view-close")?;
                }
                return Ok(true);
            }
            "status-update" if self.view.name == "status" => {
                if let Item::Status(e, staged) = self.selected() {
                    if staged {
                        self.repo()?.unstage(&e)?;
                    } else {
                        self.repo()?.stage(&e)?;
                    }
                    self.refresh_main_parent()?;
                    self.action("refresh")?;
                    if self.view.untracked
                        && self.other.is_some()
                        && !self
                            .view
                            .items
                            .iter()
                            .any(|item| matches!(item, Item::Status(..)))
                    {
                        self.action("view-close")?;
                    }
                    return Ok(true);
                }
            }
            "show-version" => self.message = format!("tig-rs {}", env!("CARGO_PKG_VERSION")),
            "parent" if self.view.name == "tree" => self.tree_parent()?,
            "screen-redraw" => (),
            _ if action.starts_with("view-") => {
                if action == "view-diff" {
                    if let Item::Changes(kind) = self.selected() {
                        self.open_changes(kind)?;
                        self.other = None;
                        self.split = false;
                        self.parent_focused = false;
                        return Ok(true);
                    }
                }
                if action == "view-tree"
                    && (self.revision.contains(':')
                        || matches!(self.selected(), Item::Grep(hit) if hit.revision.as_deref().is_some_and(|rev| rev.contains(':'))))
                {
                    return Err(
                        "Tree view for a subdirectory grep revision is not supported safely".into(),
                    );
                }
                self.select_context();
                self.open(&action[5..])?;
            }
            _ => return Err(format!("Not implemented in Rust yet: {action}").into()),
        }
        Ok(true)
    }
    fn pane_sizes(&self) -> (bool, usize, usize) {
        let vertical = self
            .config
            .settings
            .get("vertical-split")
            .and_then(|v| v.first())
            .map(String::as_str)
            .unwrap_or("auto");
        let vertical = vertical == "vertical"
            || vertical == "yes"
            || vertical == "true"
            || (vertical == "auto"
                && (self.width > 160 || self.width > self.height.saturating_sub(1) * 4));
        let total = if vertical {
            self.width.saturating_sub(1)
        } else {
            self.height.saturating_sub(1)
        };
        let option = if vertical {
            "split-view-width"
        } else {
            "split-view-height"
        };
        let size = self
            .config
            .settings
            .get(option)
            .and_then(|v| v.first())
            .map(String::as_str)
            .unwrap_or(if vertical { "50%" } else { "67%" });
        let child = if let Some(percent) = size.strip_suffix('%') {
            total * percent.parse::<usize>().unwrap_or(67).min(100) / 100
        } else {
            size.parse::<usize>().unwrap_or(total * 2 / 3)
        };
        let minimum = if vertical { 1 } else { 4.min(total / 2) };
        let child = child.max(minimum).min(total.saturating_sub(minimum));
        (vertical, total - child, child)
    }
    fn screen(&mut self) -> Vec<String> {
        let mut lines = if self.split && self.other.is_some() {
            let (vertical, parent_size, child_size) = self.pane_sizes();
            let other = self.other.as_mut().unwrap();
            let (parent, child) = if self.parent_focused {
                (&mut self.view, other)
            } else {
                (other, &mut self.view)
            };
            if vertical {
                let left = pane_screen(
                    parent,
                    &self.config,
                    parent_size,
                    self.height.saturating_sub(2),
                );
                let right = pane_screen(
                    child,
                    &self.config,
                    child_size,
                    self.height.saturating_sub(2),
                );
                left.into_iter()
                    .zip(right)
                    .map(|(a, b)| {
                        format!(
                            "{}{}|{}",
                            a,
                            " ".repeat(parent_size.saturating_sub(cell_width(&a))),
                            b
                        )
                    })
                    .collect()
            } else {
                let mut lines = pane_screen(
                    parent,
                    &self.config,
                    self.width,
                    parent_size.saturating_sub(1),
                );
                lines.extend(pane_screen(
                    child,
                    &self.config,
                    self.width,
                    child_size.saturating_sub(1),
                ));
                lines
            }
        } else {
            pane_screen(
                &mut self.view,
                &self.config,
                self.width,
                self.height.saturating_sub(2),
            )
        };
        lines.push(clip(&self.message, 0, self.width));
        lines
    }
    fn script(&mut self, path: &str) -> Result<()> {
        let mut grep_prompt = false;
        for raw in fs::read_to_string(path)?.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if grep_prompt {
                self.grep_query(line.strip_suffix("<Enter>").unwrap_or(line))?;
                grep_prompt = false;
            } else if matches!(line, ":g" | ":view-grep") {
                grep_prompt = true;
            } else if let Some(path) = line.strip_prefix(":save-display ") {
                let mut screen = self.screen();
                screen.pop();
                fs::write(path, format!("{}\n", screen.join("\n")))?;
            } else if let Some(pattern) = line.strip_prefix('/').or_else(|| line.strip_prefix('?'))
            {
                let pattern = pattern.strip_suffix("<Enter>").unwrap_or(pattern);
                if !pattern.is_empty() {
                    self.search = pattern.into();
                }
                self.find(line.starts_with('?'));
            } else if let Some(n) = line.strip_prefix(":goto ") {
                self.action(&format!("goto {n}"))?;
            } else {
                let action = if let Some(a) = line.strip_prefix(':') {
                    a.to_string()
                } else {
                    self.binding(line)
                };
                if !self.action(&action)? {
                    break;
                }
                if let Some(command) = self.pending_command.take() {
                    if command.silent && !command.echo {
                        command.run_allow_nonzero(self.repo()?, false, true)?;
                    } else {
                        command.run(self.repo()?, false, true)?;
                    }
                    if command.exit {
                        break;
                    }
                    self.action("refresh")?;
                }
            }
        }
        Ok(())
    }
    fn binding(&self, key: &str) -> String {
        self.config
            .action(&self.view.name, key)
            .map(|a| {
                if a.first().is_some_and(|arg| arg.starts_with(':')) {
                    return a.join(" ");
                }
                a.iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        if index == 0 {
                            arg.clone()
                        } else {
                            format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| key.into())
    }
}

fn cell_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}
fn log_header_offset(line: &str) -> Option<usize> {
    line.find("commit ").filter(|&i| {
        i == 0
            || (line[..i].contains('*')
                && line[..i]
                    .chars()
                    .all(|c| matches!(c, ' ' | '*' | '|' | '/' | '\\')))
    })
}

fn stat_header_after(rows: &[String], selected: usize, start: usize) -> Option<usize> {
    let is_header = |line: &str| {
        line.starts_with("diff --git ")
            || line.starts_with("diff --cc ")
            || line.starts_with("diff --combined ")
    };
    let first_patch = rows.iter().position(|row| is_header(row))?;
    if selected < start || selected >= first_patch || !rows.get(selected)?.contains(" | ") {
        return None;
    }
    let stat_index = rows[start..=selected]
        .iter()
        .filter(|row| row.contains(" | "))
        .count()
        .checked_sub(1)?;
    rows.iter()
        .enumerate()
        .skip(first_patch)
        .filter(|(_, row)| is_header(row))
        .nth(stat_index)
        .map(|(index, _)| index)
}

fn stage_stat_header(rows: &[String], selected: usize) -> Option<usize> {
    stat_header_after(rows, selected, 0)
}

fn diff_stat_header(rows: &[String], selected: usize) -> Option<usize> {
    let start = rows[..=selected].iter().rposition(|line| line == "---")? + 1;
    stat_header_after(rows, selected, start)
}

fn diff_reloaded_line(old: &View, rows: &[String]) -> Option<usize> {
    let (_, target) = diff_edit_target(&old.rows, old.selected)?;
    if target == 0 {
        return None;
    }
    let header = old.rows[..=old.selected]
        .iter()
        .rfind(|row| row.starts_with("diff "))?;
    // ponytail: ordinary hunks only; extend alongside combined-diff navigation.
    if !header.starts_with("diff --git ") {
        return None;
    }
    let start = rows.iter().position(|row| row == header)?;
    let mut line = None;
    for (index, row) in rows.iter().enumerate().skip(start + 1) {
        if row.starts_with("diff ") {
            break;
        }
        if row.starts_with("@@ ") {
            line = diff_hunk_start(row);
        } else if let Some(number) = &mut line {
            if *number == target {
                return Some(index);
            }
            if !row.starts_with(['-', '\\']) {
                *number += 1;
            }
        }
    }
    None
}

fn diff_hunk_start(row: &str) -> Option<usize> {
    row.split_whitespace()
        .find(|field| field.starts_with('+'))?
        .trim_start_matches('+')
        .split(',')
        .next()?
        .parse()
        .ok()
}

fn diff_edit_target(rows: &[String], selected: usize) -> Option<(PathBuf, usize)> {
    rows.get(selected)?;
    let is_header = |line: &str| {
        line.starts_with("diff --git ")
            || line.starts_with("diff --cc ")
            || line.starts_with("diff --combined ")
    };
    let stats_start = rows[..=selected]
        .iter()
        .rposition(|line| line == "---")
        .map(|index| index + 1);
    let stat = stats_start.is_some_and(|start| {
        start <= selected
            && rows[selected].contains(" | ")
            && !rows[start..=selected].iter().any(|line| is_header(line))
    });
    let header = if stat {
        let stats_start = stats_start?;
        let stat_index = rows[stats_start..=selected]
            .iter()
            .filter(|line| line.contains(" | "))
            .count()
            .checked_sub(1)?;
        let next_commit = rows[selected + 1..]
            .iter()
            .position(|line| line.starts_with("commit "))
            .map_or(rows.len(), |index| selected + 1 + index);
        rows[selected + 1..next_commit]
            .iter()
            .enumerate()
            .filter(|(_, line)| is_header(line))
            .nth(stat_index)
            .map(|(index, _)| selected + 1 + index)?
    } else {
        let section = rows[..=selected]
            .iter()
            .rposition(|line| line.starts_with("commit "))
            .unwrap_or(0);
        rows[section..=selected]
            .iter()
            .rposition(|line| is_header(line))
            .map(|index| section + index)?
    };
    let end = rows[header + 1..]
        .iter()
        .position(|line| is_header(line))
        .map_or(rows.len(), |index| header + 1 + index);
    let patch = &rows[header + 1..end];
    let path = if let Some(file) = patch
        .iter()
        .find_map(|line| line.strip_prefix("rename to "))
    {
        git_patch_path(file)?
    } else {
        let file = patch.iter().find_map(|line| line.strip_prefix("+++ "))?;
        if file == "/dev/null" {
            return None;
        }
        let path = git_patch_path(file)?;
        let old = &rows[header];
        let prefix = if (old.starts_with("diff --git a/") || old.starts_with("diff --git \"a/"))
            && path.starts_with("b/")
        {
            Some("b")
        } else if (old.starts_with("diff --git i/") || old.starts_with("diff --git \"i/"))
            && path.starts_with("w/")
        {
            Some("w")
        } else if old
            .strip_prefix("diff --cc ")
            .or_else(|| old.strip_prefix("diff --combined "))
            .and_then(git_patch_path)
            .is_some_and(|name| path.strip_prefix("b").is_ok_and(|target| name == target))
        {
            Some("b")
        } else {
            None
        };
        if let Some(prefix) = prefix {
            path.strip_prefix(prefix).ok()?.to_path_buf()
        } else {
            path
        }
    };
    if stat {
        return Some((path, 0));
    }
    if selected <= header || end == header + 1 {
        return Some((path, 0));
    }
    let hunk = rows[header + 1..=selected.min(end - 1)]
        .iter()
        .rposition(|line| line.starts_with("@@"))
        .map(|index| header + 1 + index);
    let Some(hunk) = hunk else {
        return Some((path, 0));
    };
    let start = diff_hunk_start(&rows[hunk])?;
    let preceding = rows
        .get(hunk + 1..selected)
        .unwrap_or(&[])
        .iter()
        .filter(|line| !line.starts_with('-') && !line.starts_with('\\'))
        .count();
    Some((path, start + preceding))
}

fn git_patch_path(raw: &str) -> Option<PathBuf> {
    tig_rs::git::parse_git_path(raw.as_bytes()).ok()
}

#[cfg(test)]
mod editor_tests {
    use super::{diff_edit_target, git_patch_path, stage_stat_header, App, Config, Item, View};
    use std::path::PathBuf;

    #[test]
    fn blame_history_path_is_not_a_worktree_edit_target() {
        let raw = format!("{} 1 1\nfilename old/file\n\tcontent\n", "a".repeat(40));
        let line = tig_rs::git::parse_blame(raw.as_bytes()).unwrap().remove(0);
        let mut view = View::new("blame");
        view.path = "new/file".into();
        view.push("content".into(), Item::Blame(line.clone()));
        let mut app = App {
            repo: None,
            config: Config::defaults(),
            view,
            help: None,
            previous: vec![],
            pending_command: None,
            other: None,
            split: false,
            parent_focused: false,
            revision: "HEAD".into(),
            path: "new/file".into(),
            args: vec![],
            message: String::new(),
            search: String::new(),
            width: 80,
            height: 20,
        };
        assert_eq!(app.edit_target(), None);
        app.view.items[0] = Item::Blame(tig_rs::model::BlameLine {
            filename: "new/file".into(),
            ..line
        });
        assert_eq!(app.edit_target(), Some((PathBuf::from("new/file"), 1)));
    }

    #[test]
    fn maps_stat_and_patch_rows_to_file_and_new_line() {
        let rows: Vec<String> = [
            "commit abc",
            "---",
            " a | 2 +-",
            " b | 1 +",
            "diff --git a/a b/a",
            "--- a/a",
            "+++ b/a",
            "@@ -9,2 +9,2 @@",
            " unchanged",
            "-old",
            "+new",
            "diff --git a/b b/b",
            "--- a/b",
            "+++ b/b",
            "@@ -1 +1 @@",
            "+text",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(diff_edit_target(&rows, 2), Some((PathBuf::from("a"), 0)));
        assert_eq!(diff_edit_target(&rows, 3), Some((PathBuf::from("b"), 0)));
        assert_eq!(diff_edit_target(&rows, 4), Some((PathBuf::from("a"), 0)));
        assert_eq!(diff_edit_target(&rows, 7), Some((PathBuf::from("a"), 9)));
        assert_eq!(diff_edit_target(&rows, 10), Some((PathBuf::from("a"), 10)));
        assert_eq!(diff_edit_target(&rows, 15), Some((PathBuf::from("b"), 1)));
    }

    #[test]
    fn aggregate_stage_stats_jump_to_matching_patch_and_file() {
        let rows: Vec<String> = [
            " a | 1 +",
            " b | 1 +",
            " 2 files changed, 2 insertions(+)",
            "",
            "diff --git a/a b/a",
            "--- a/a",
            "+++ b/a",
            "@@ -0,0 +1 @@",
            "+one",
            "diff --git a/b b/b",
            "--- a/b",
            "+++ b/b",
            "@@ -0,0 +1 @@",
            "+two",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(stage_stat_header(&rows, 0), Some(4));
        assert_eq!(stage_stat_header(&rows, 1), Some(9));
        assert_eq!(stage_stat_header(&rows, 2), None);
        assert_eq!(diff_edit_target(&rows, 9), Some((PathBuf::from("b"), 0)));
        let mut view = View::new("stage");
        for row in rows {
            view.push(row, Item::Text);
        }
        view.selected = 1;
        let mut app = App {
            repo: None,
            config: Config::default(),
            view,
            help: None,
            previous: vec![],
            pending_command: None,
            other: None,
            split: false,
            parent_focused: false,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            args: vec![],
            message: String::new(),
            search: String::new(),
            width: 80,
            height: 20,
        };
        assert_eq!(app.edit_target(), Some((PathBuf::from("b"), 0)));
        app.enter().unwrap();
        assert_eq!(app.view.selected, 9);
        app.view = View::text(
            "diff",
            "commit abc\n---\n a | 1 +\n b | 1 +\ndiff --git a/a b/a\ndiff --git a/b b/b\n",
        );
        app.view.selected = 3;
        app.enter().unwrap();
        assert_eq!(app.view.selected, 5);
        let mixed = b"diff --cc conflict\n@@@ -1,1 -1,1 +1,1 @@@\n++x\ndiff --git a/other b/other\n--- a/other\n+++ b/other\n@@ -1 +1 @@\n-old\n+new\n";
        app.view = View::text("stage", &String::from_utf8_lossy(mixed));
        app.view.raw_patch = mixed.to_vec();
        app.view.selected = 2;
        assert!(app
            .action("status-update")
            .unwrap_err()
            .to_string()
            .contains("combined merge patch"));
    }

    #[test]
    fn preserves_literal_prefixes_and_ignores_commit_body() {
        let rows: Vec<String> = [
            "commit abc",
            "foo | bar",
            "---",
            " w/foo | 1 +",
            " other | 1 +",
            "diff --git w/foo w/foo",
            "--- w/foo",
            "+++ w/foo",
            "@@ -1 +1,2 @@",
            "+new",
            "diff --git other other",
            "--- other",
            "+++ other",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(diff_edit_target(&rows, 1), None);
        assert_eq!(
            diff_edit_target(&rows, 3),
            Some((PathBuf::from("w/foo"), 0))
        );
        assert_eq!(
            diff_edit_target(&rows, 7),
            Some((PathBuf::from("w/foo"), 0))
        );
        assert_eq!(
            diff_edit_target(&rows, 4),
            Some((PathBuf::from("other"), 0))
        );

        let mut following = rows.clone();
        following.extend(
            [
                "commit def",
                "---",
                " next | 1 +",
                "diff --git a/next b/next",
                "+++ b/next",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        assert_eq!(
            diff_edit_target(&following, 15),
            Some((PathBuf::from("next"), 0))
        );
        assert_eq!(diff_edit_target(&following, 13), None);

        let rename: Vec<String> = [
            "diff --git a/old b/new",
            "similarity index 100%",
            "rename from old",
            "rename to new",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(
            diff_edit_target(&rename, 3),
            Some((PathBuf::from("new"), 0))
        );
        let combined: Vec<String> = ["diff --cc path", "--- a/path", "+++ b/path"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(
            diff_edit_target(&combined, 2),
            Some((PathBuf::from("path"), 0))
        );
        assert_eq!(
            git_patch_path("\"b/name\\twithtab\""),
            Some(PathBuf::from("b/name\twithtab"))
        );
        assert_eq!(git_patch_path("\"bad\\777\""), None);
    }
}

fn pager_line_numbers(config: &Config, view: &View) -> Option<(usize, usize)> {
    if !matches!(
        view.name.as_str(),
        "pager" | "stage" | "log" | "blob" | "diff"
    ) {
        return None;
    }
    let columns = config.settings.get(&format!("{}-view", view.name))?;
    let spec = columns
        .iter()
        .find(|spec| spec.split(':').next() == Some("line-number"))?;
    let mut parts = spec.split([':', ',']).skip(1);
    if matches!(parts.next(), Some("no" | "false" | "0")) {
        return None;
    }
    let mut interval = 5;
    let mut width = view.rows.len().to_string().len().clamp(3, 9);
    for part in parts {
        if let Some(value) = part
            .strip_prefix("interval=")
            .and_then(|s| s.parse::<usize>().ok())
        {
            interval = if value == 0 { 5 } else { value };
        } else if let Some(value) = part
            .strip_prefix("width=")
            .and_then(|s| s.parse::<usize>().ok())
        {
            width = value.clamp(3, 9);
        }
    }
    Some((width, interval))
}

fn pane_screen(view: &mut View, config: &Config, width: usize, visible: usize) -> Vec<String> {
    let visible = visible.max(1);
    if view.selected < view.top {
        view.top = view.selected;
    }
    if view.selected >= view.top + visible {
        view.top = view.selected + 1 - visible;
    }
    let line_numbers = pager_line_numbers(config, view);
    let separator = if config.value("line-graphics") == Some("utf-8") {
        "│ "
    } else {
        "| "
    };
    let mut lines: Vec<String> = (0..visible)
        .map(|i| {
            let index = view.top + i;
            let row = view.rows.get(index).map(String::as_str).unwrap_or("");
            if let Some((number_width, interval)) = line_numbers.filter(|_| index < view.rows.len())
            {
                let number = view.line_numbers.get(index).copied().unwrap_or(0);
                let prefix = if number != 0 && (number == 1 || number % interval == 0) {
                    format!("{number:>number_width$}{separator}")
                } else {
                    format!("{}{separator}", " ".repeat(number_width))
                };
                clip(&format!("{prefix}{row}"), view.left, width)
            } else {
                clip(row, view.left, width)
            }
        })
        .collect();
    let reference = match view.items.get(view.selected) {
        Some(Item::Commit(c)) => c.oid.clone(),
        Some(Item::Changes(kind)) => kind.title().into(),
        Some(Item::Tree(e)) if view.name == "tree" => {
            if e.path == view.path.parent().unwrap_or(std::path::Path::new(""))
                && !view.path.as_os_str().is_empty()
            {
                "Open parent directory".into()
            } else {
                e.oid.clone()
            }
        }
        Some(Item::Ref(id, _)) if matches!(view.name.as_str(), "log" | "refs") => id.clone(),
        Some(Item::Grep(hit)) => hit.label.clone(),
        Some(Item::Blame(line)) if view.name == "blame" => {
            if line.oid.bytes().all(|byte| byte == b'0') {
                line.filename.display().to_string()
            } else {
                format!("{}:{}", line.oid, line.filename.display())
            }
        }
        _ if view.name == "refs" => "All references".into(),
        Some(Item::Status(e, staged)) => format!(
            "Press u to {} '{}'{}",
            if *staged { "unstage" } else { "stage" },
            e.path.display(),
            if *staged {
                ""
            } else if e.index == '?' {
                " for addition"
            } else {
                " for commit"
            }
        ),
        _ if view.name == "status" => "Nothing to update".into(),
        _ if view.name == "diff" && diff_stat_header(&view.rows, view.selected).is_some() => {
            "Press '<Enter>' to jump to file diff".into()
        }
        _ if view.name == "diff" => diff_edit_target(&view.rows, view.selected)
            .map(|(path, _)| format!("Changes to '{}'", path.display()))
            .unwrap_or_else(|| view.revision.clone()),
        _ if view.name == "stage"
            && view.path.as_os_str().is_empty()
            && stage_stat_header(&view.rows, view.selected).is_some() =>
        {
            "Press '<Enter>' to jump to file diff".into()
        }
        _ if view.name == "stage" => {
            let kind = if view.staged { "Staged" } else { "Unstaged" };
            if view.path.as_os_str().is_empty() {
                diff_edit_target(&view.rows, view.selected)
                    .map(|(path, _)| format!("{kind} changes to '{}'", path.display()))
                    .unwrap_or_else(|| format!("{kind} changes"))
            } else {
                format!("{kind} changes to '{}'", view.path.display())
            }
        }
        _ if view.name == "blob" || view.name == "blame" => view.path.display().to_string(),
        _ => String::new(),
    };
    let mut title = format!("[{}]", view.name);
    if !reference.is_empty() {
        title.push_str(&format!(" {reference}"));
    }
    if view.name != "status"
        && !view.rows.is_empty()
        && !(view.name == "refs" && view.selected == 0)
        && reference != "Open parent directory"
        && !matches!(view.items.get(view.selected), Some(Item::Changes(_)))
    {
        title.push_str(&format!(
            " - {} {} of {}",
            match view.name.as_str() {
                "main" => "commit",
                "refs" => "reference",
                "tree" => "file",
                _ => "line",
            },
            if view.name == "tree" {
                view.line_numbers.get(view.selected).copied().unwrap_or(0)
            } else if view.name == "main" {
                view.items[..=view.selected]
                    .iter()
                    .filter(|item| matches!(item, Item::Commit(_)))
                    .count()
            } else {
                view.selected + usize::from(view.name != "refs")
            },
            view.rows.len().saturating_sub(match view.name.as_str() {
                "refs" => 1,
                "tree" => 1 + usize::from(!view.path.as_os_str().is_empty()),
                "main" => view
                    .items
                    .iter()
                    .filter(|item| matches!(item, Item::Changes(_)))
                    .count(),
                _ => 0,
            })
        ));
    }
    let percent = if view.rows.is_empty() {
        0
    } else {
        (view.top + visible).min(view.rows.len()) * 100 / view.rows.len()
    };
    let suffix = format!(" {percent}%");
    let title_width = width.saturating_sub(suffix.len());
    let title = clip(&title, 0, title_width);
    lines.push(format!(
        "{}{}{}",
        title,
        " ".repeat(title_width.saturating_sub(cell_width(&title))),
        suffix
    ));
    lines
}

// Never allow Git/config content to inject terminal escapes. Width is terminal cells,
// not UTF-8 bytes, and clipping never slices inside a character.
fn clip(text: &str, skip: usize, width: usize) -> String {
    let mut out = String::new();
    let mut column = 0;
    for c in text.chars() {
        let chars = if c == '\t' {
            " ".repeat(8 - column % 8)
        } else if c.is_control() {
            format!("\\x{:02x}", c as u32)
        } else {
            c.to_string()
        };
        for c in chars.chars() {
            let w = c.width().unwrap_or(0);
            if column >= skip && column.saturating_add(w) <= skip.saturating_add(width) {
                out.push(c);
            }
            column += w;
            if column >= skip.saturating_add(width) {
                return out;
            }
        }
    }
    out
}
struct Terminal {
    out: fs::File,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    signals: Vec<signal_hook::SigId>,
}
impl Terminal {
    fn open() -> Result<Self> {
        let out = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")?;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut signals = Vec::new();
        for signal in [
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGINT,
        ] {
            signals.push(signal_hook::flag::register(signal, stop.clone())?);
        }
        terminal::enable_raw_mode()?;
        let mut t = Self { out, stop, signals };
        execute!(
            t.out,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            event::EnableMouseCapture
        )?;
        Ok(t)
    }
    fn read(&self) -> Result<Event> {
        loop {
            if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(
                    io::Error::new(io::ErrorKind::Interrupted, "Terminal interrupted").into(),
                );
            }
            if event::poll(std::time::Duration::from_millis(100))? {
                return Ok(event::read()?);
            }
        }
    }
    fn draw(&mut self, app: &mut App) -> Result<()> {
        let lines = app.screen();
        queue!(self.out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
        for (i, line) in lines.iter().enumerate() {
            queue!(self.out, cursor::MoveTo(0, i as u16))?;
            write!(self.out, "{line}")?;
            queue!(self.out, SetAttribute(Attribute::Reset))?;
        }
        let (x, y, width, visible) = if app.split && app.other.is_some() {
            let (vertical, parent, child) = app.pane_sizes();
            if vertical {
                (
                    if app.parent_focused { 0 } else { parent + 1 },
                    0,
                    if app.parent_focused { parent } else { child },
                    app.height.saturating_sub(2),
                )
            } else {
                (
                    0,
                    if app.parent_focused { 0 } else { parent },
                    app.width,
                    (if app.parent_focused { parent } else { child }).saturating_sub(1),
                )
            }
        } else {
            (0, 0, app.width, app.height.saturating_sub(2))
        };
        for row in [
            y + app.view.selected.saturating_sub(app.view.top),
            y + visible,
        ] {
            if let Some(line) = lines.get(row) {
                let text = clip(line, x, width);
                queue!(
                    self.out,
                    cursor::MoveTo(x as u16, row as u16),
                    SetAttribute(Attribute::Reverse)
                )?;
                write!(
                    self.out,
                    "{}{}",
                    text,
                    " ".repeat(width.saturating_sub(cell_width(&text)))
                )?;
                queue!(self.out, SetAttribute(Attribute::Reset))?;
            }
        }
        self.out.flush()?;
        Ok(())
    }
    fn prompt(&mut self, app: &mut App, prefix: &str) -> Result<Option<String>> {
        let mut value = String::new();
        loop {
            queue!(
                self.out,
                cursor::MoveTo(0, app.height.saturating_sub(1) as u16),
                Clear(ClearType::CurrentLine)
            )?;
            write!(
                self.out,
                "{}",
                clip(&format!("{prefix}{value}"), 0, app.width)
            )?;
            self.out.flush()?;
            if let Event::Key(k) = self.read()? {
                match k.code {
                    KeyCode::Enter => return Ok(Some(value)),
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(None)
                    }
                    KeyCode::Char(c) => value.push(c),
                    _ => (),
                }
            }
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(
            self.out,
            SetAttribute(Attribute::Reset),
            event::DisableMouseCapture,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
        for signal in self.signals.drain(..) {
            signal_hook::low_level::unregister(signal);
        }
    }
}
fn key_name(code: KeyCode, modifiers: KeyModifiers) -> String {
    if let KeyCode::Char(c) = code {
        if modifiers.contains(KeyModifiers::CONTROL) {
            return format!("<C-{}>", c.to_ascii_uppercase());
        }
    }
    match code {
        KeyCode::Char(' ') => "<Space>".into(),
        KeyCode::Char('<') => "<Lt>".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "<Enter>".into(),
        KeyCode::Up => "<Up>".into(),
        KeyCode::Down => "<Down>".into(),
        KeyCode::Left => "<Left>".into(),
        KeyCode::Right => "<Right>".into(),
        KeyCode::Home => "<Home>".into(),
        KeyCode::End => "<End>".into(),
        KeyCode::PageUp => "<PgUp>".into(),
        KeyCode::PageDown => "<PgDown>".into(),
        KeyCode::Tab => "<Tab>".into(),
        KeyCode::Esc => "<Esc>".into(),
        KeyCode::F(n) => format!("<F{n}>"),
        _ => String::new(),
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = env::args_os().skip(1).map(|arg| arg.into_string().map_err(|_| "Non-UTF-8 CLI arguments are not supported yet; browse the file through the tree/status view")).collect::<std::result::Result<_,_>>()?;
    let mut cli = Cli::parse(&args, !io::stdin().is_terminal())?;
    if cli.help {
        println!("{HELP}");
        return Ok(());
    }
    if cli.version {
        println!(
            "tig-rs {} (upstream Tig 2.6.1; migration in progress)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    for dir in &cli.directories {
        env::set_current_dir(dir)?;
    }
    let mut config = Config::load();
    if matches!(cli.view.as_str(), "main" | "diff") {
        config.take_diff_options(&mut cli.git_args);
    }
    for message in &config.diagnostics {
        eprintln!("tig warning: {message}");
    }
    let invocation = env::current_dir()?;
    let repo = Repository::discover(&invocation).ok();
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let mut app = App {
        repo,
        config,
        view: View::new(&cli.view),
        help: None,
        previous: vec![],
        pending_command: None,
        other: None,
        split: false,
        parent_focused: false,
        revision: "HEAD".into(),
        path: PathBuf::new(),
        args: cli.git_args.clone(),
        message: String::new(),
        search: String::new(),
        width: width as usize,
        height: height as usize,
    };
    if let (Some(repo), Some(separator)) = (&app.repo, app.args.iter().position(|arg| arg == "--"))
    {
        if cli.view != "blame" {
            let prefix = invocation.strip_prefix(&repo.root)?;
            for arg in &mut app.args[separator + 1..] {
                *arg = prefix
                    .join(&*arg)
                    .into_os_string()
                    .into_string()
                    .map_err(|_| "Non-UTF-8 path prefix is not supported yet")?;
            }
        }
    }
    if cli.view == "pager" {
        let mut text = String::new();
        io::Read::read_to_string(&mut io::stdin(), &mut text)?;
        if cli
            .git_args
            .iter()
            .take_while(|arg| !matches!(arg.as_str(), "--" | "--end-of-options"))
            .any(|arg| arg == "--pretty=raw")
        {
            let commits = tig_rs::git::parse_raw_history(&text)?;
            app.view = View::new("main");
            app.view.from_stdin = true;
            for commit in commits {
                app.view.push(String::new(), Item::Commit(commit));
            }
            app.view.redraw_stdin(&app.config, app.width)?;
        } else {
            app.view = View::text("pager", &text);
        }
    } else {
        if cli.view == "blame" {
            if let Some(path) = app.args.pop() {
                app.path = path.into();
            }
            app.args.retain(|arg| arg != "--");
            if app.args.len() > 1 || app.args.first().is_some_and(|arg| arg.starts_with('-')) {
                return Err("Rust blame currently supports [revision] -- path only".into());
            }
            if let Some(rev) = app.args.first() {
                app.revision = rev.clone();
            }
            if let Some(repo) = &app.repo {
                let cwd = env::current_dir()?;
                let absolute = cwd.join(&app.path);
                app.path = absolute.strip_prefix(&repo.root)?.to_path_buf();
            }
        }
        if cli.view == "diff" {
            app.revision = cli.diff_revision().to_owned();
        }
        app.view = app.load(&cli.view)?;
        if cli.view == "grep" && app.view.rows.is_empty() {
            app.message = "No matches found".into();
        }
    }
    if cli.line > 0 {
        app.view.selected = cli.line.min(app.view.rows.len().saturating_sub(1));
    }
    app.view.restore_status_selection();
    if let Ok(script) = env::var("TIG_SCRIPT") {
        app.width = env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(app.width)
            .max(1);
        app.height = env::var("LINES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(app.height)
            .max(3);
        app.center_selection();
        return app.script(&script);
    }
    app.center_selection();
    let mut terminal = Terminal::open()?;
    loop {
        terminal.draw(&mut app)?;
        let action = match terminal.read()? {
            Event::Resize(w, h) => {
                app.width = w as usize;
                app.height = h as usize;
                continue;
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollUp => "move-up".into(),
                MouseEventKind::ScrollDown => "move-down".into(),
                _ => continue,
            },
            Event::Key(k) => app.binding(&key_name(k.code, k.modifiers)),
            _ => continue,
        };
        app.message.clear();
        if action == "search" || action == "search-back" {
            if let Some(s) =
                terminal.prompt(&mut app, if action == "search" { "/" } else { "?" })?
            {
                if !s.is_empty() {
                    app.search = s;
                }
                app.find(action == "search-back");
            }
        } else if action == "view-grep" {
            if let Some(s) = terminal.prompt(&mut app, "grep: ")? {
                if let Err(e) = app.grep_query(&s) {
                    app.message = e.to_string();
                }
            }
        } else if action == "prompt" {
            if let Some(s) = terminal.prompt(&mut app, ":")? {
                let result = if matches!(s.as_str(), "g" | "view-grep") {
                    terminal
                        .prompt(&mut app, "grep: ")?
                        .map_or(Ok(()), |query| app.grep_query(&query))
                        .map(|()| true)
                } else {
                    app.action(&s)
                };
                match result {
                    Ok(false) => break,
                    Ok(true) => (),
                    Err(e) => app.message = e.to_string(),
                }
            }
        } else {
            match app.action(&action) {
                Ok(false) => break,
                Ok(true) => (),
                Err(e) => app.message = e.to_string(),
            }
        }
        if let Some(command) = app.pending_command.take() {
            let confirmed = if command.confirm {
                let answer = terminal.prompt(
                    &mut app,
                    &format!(
                        "Run {}{}? [y/N] ",
                        command.display(),
                        if command.exit { " and exit" } else { "" }
                    ),
                )?;
                answer.is_some_and(|answer| matches!(answer.as_str(), "y" | "Y" | "yes"))
            } else {
                true
            };
            if !confirmed {
                continue;
            }
            let result = if command.silent || command.echo {
                if command.silent && !command.echo {
                    command.run_allow_nonzero(app.repo()?, true, true)
                } else {
                    command.run(app.repo()?, true, true)
                }
            } else {
                drop(terminal);
                let result = command.run(app.repo()?, true, false);
                if result.is_err() || (!command.quick && !command.exit) {
                    use std::io::{BufRead, BufReader};
                    let mut tty = fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open("/dev/tty")?;
                    if let Err(error) = &result {
                        writeln!(tty, "{error}")?;
                    }
                    write!(tty, "Press Enter to continue")?;
                    tty.flush()?;
                    BufReader::new(tty).read_line(&mut String::new())?;
                }
                terminal = Terminal::open()?;
                result
            };
            match result {
                Ok(output) => {
                    if command.exit {
                        break;
                    }
                    if let Err(error) = app.action("refresh") {
                        app.message = error.to_string();
                    } else if command.echo {
                        app.message = String::from_utf8_lossy(&output.stdout)
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_owned();
                    }
                }
                Err(error) => app.message = error.to_string(),
            }
        }
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("tig: {e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_nul_fields_preserve_colons_newlines_and_revision_paths() {
        let bytes = b"name:part.txt\x003\0worktree\n--\nfile\x005\0separator-like name\nBinary file strange matches\nname.txt\x006\0binary-like name\nodd\nname.txt\x007\0newline\nHEAD:src:name.rs\x008\0revision\n";
        let hits = grep_rows(bytes, &["HEAD".into()]).unwrap();
        assert_eq!(hits.len(), 5);
        assert_eq!(hits[0].path, PathBuf::from("name:part.txt"));
        assert_eq!(hits[0].revision, None);
        assert_eq!(hits[1].path, PathBuf::from("--\nfile"));
        assert_eq!(
            hits[2].path,
            PathBuf::from("Binary file strange matches\nname.txt")
        );
        assert_eq!(hits[3].path, PathBuf::from("odd\nname.txt"));
        assert_eq!(hits[4].path, PathBuf::from("src:name.rs"));
        assert_eq!(hits[4].revision.as_deref(), Some("HEAD"));
        let nested = grep_rows(
            b"HEAD:subdir:file.txt\x009\0nested\n",
            &["HEAD".into(), "HEAD:subdir".into()],
        )
        .unwrap();
        assert_eq!(nested[0].revision.as_deref(), Some("HEAD:subdir"));
        assert_eq!(nested[0].path, PathBuf::from("file.txt"));
        assert!(grep_rows(b"partial\x001\0no newline", &[]).is_err());
    }

    #[test]
    fn grep_options_never_turn_pattern_or_option_values_into_revisions() {
        assert!(grep_has_leading_delimiter(&[
            "--".into(),
            "foo".into(),
            "HEAD".into()
        ]));
        assert!(grep_has_leading_delimiter(&[
            "-i".into(),
            "--".into(),
            "foo".into(),
            "HEAD".into()
        ]));
        assert!(grep_has_leading_delimiter(&[
            "-e".into(),
            "--".into(),
            "HEAD".into()
        ]));
        assert!(!grep_has_leading_delimiter(&[
            "foo".into(),
            "HEAD".into(),
            "--".into(),
            "file".into()
        ]));
        assert_eq!(
            grep_revision_args(&["foo".into(), "HEAD".into()]),
            vec!["HEAD"]
        );
        assert_eq!(
            grep_revision_args(&["foo".into(), "HEAD^{tree}".into(), "HEAD:sub".into()]),
            ["HEAD^{tree}", "HEAD:sub"]
        );
        assert!(
            grep_revision_args(&["-e".into(), "foo".into(), "-e".into(), "HEAD".into()]).is_empty()
        );
        assert!(grep_revision_args(&["-m".into(), "1".into(), "foo".into()]).is_empty());
        let hit = grep_rows(b"HEAD:foo\x003\0match\n", &[]).unwrap();
        assert!(ambiguous_grep_ref(
            &hit,
            &["-e".into(), "foo".into(), "-e".into(), "HEAD".into()]
        ));
        let tree_hit = grep_rows(b"HEAD:sub:file\x003\0match\n", &[]).unwrap();
        assert!(ambiguous_grep_ref(
            &tree_hit,
            &["-e".into(), "foo".into(), "HEAD:sub".into()]
        ));
        assert_eq!(
            unsupported_grep_option(&["foo".into(), "-C1".into()]),
            Some("-C1")
        );
        assert_eq!(
            unsupported_grep_option(&["-e".into(), "--heading".into()]),
            None
        );
    }

    #[test]
    fn unclosed_binding_argument_cannot_become_a_valid_toggle() {
        let mut app = App {
            repo: None,
            config: Config::defaults(),
            view: View::new("main"),
            help: None,
            previous: vec![],
            pending_command: None,
            other: None,
            split: false,
            parent_focused: false,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            args: vec![],
            message: String::new(),
            search: String::new(),
            width: 80,
            height: 20,
        };
        for quote in ["\"", "'"] {
            app.config
                .parse(&format!("bind generic a :toggle {quote}author"));
            let before = app.config.settings.clone();
            let command = app.binding("a");
            assert!(app.action(&command).is_err());
            assert_eq!(app.config.settings, before);
            assert_eq!(
                app.config.action("main", "a").unwrap()[1],
                format!("{quote}author")
            );
        }
    }

    #[test]
    fn failed_grep_query_keeps_previous_arguments() {
        let mut app = App {
            repo: None,
            config: Config::defaults(),
            view: View::new("grep"),
            help: None,
            previous: vec![],
            pending_command: None,
            other: None,
            split: false,
            parent_focused: false,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            args: vec!["old".into()],
            message: String::new(),
            search: String::new(),
            width: 80,
            height: 20,
        };
        assert!(app.grep_query("new").is_err());
        assert_eq!(app.args, ["old"]);
        assert_eq!(app.view.name, "grep");
    }

    #[test]
    fn grep_tree_expression_reads_blob_from_its_own_tree() {
        let root = env::temp_dir().join(format!(
            "tig-grep-tree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("sub")).unwrap();
        Command::new("git")
            .current_dir(&root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        let repo = Repository::discover(&root).unwrap();
        repo.command(["config", "user.name", "Test"]).unwrap();
        repo.command(["config", "user.email", "test@example.invalid"])
            .unwrap();
        repo.command(["config", "commit.gpgsign", "false"]).unwrap();
        fs::write(root.join("sub/file.txt"), "needle in sub\n").unwrap();
        fs::write(root.join("file.txt"), "wrong root file\n").unwrap();
        repo.command(["add", "."]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();
        let output = repo
            .command([
                "grep",
                "--no-color",
                "-n",
                "-z",
                "--full-name",
                "needle",
                "HEAD:sub",
            ])
            .unwrap();
        let hits = grep_rows(&output, &["HEAD:sub".into()]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, PathBuf::from("file.txt"));
        assert_eq!(hits[0].revision.as_deref(), Some("HEAD:sub"));
        let tree = grep_tree_oid(&repo, "HEAD:sub").unwrap();
        let blob = repo
            .command(["cat-file", "blob", &format!("{tree}:file.txt")])
            .unwrap();
        assert_eq!(blob, b"needle in sub\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_grep_tree_cannot_open_root_relative_tree_view() {
        let mut view = View::new("grep");
        view.push(
            "hit".into(),
            Item::Grep(GrepLine {
                label: "HEAD:sub:file.txt".into(),
                path: PathBuf::from("file.txt"),
                revision: Some("HEAD:sub".into()),
                line: 1,
                text: "hit".into(),
            }),
        );
        let mut app = App {
            repo: None,
            config: Config::defaults(),
            view,
            help: None,
            previous: vec![],
            pending_command: None,
            other: None,
            split: false,
            parent_focused: false,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            args: vec![],
            message: String::new(),
            search: String::new(),
            width: 80,
            height: 20,
        };
        assert!(app.action("view-tree").is_err());
        assert_eq!(app.edit_target(), None);
        app.view = View::new("blob");
        app.revision = "HEAD:sub".into();
        assert!(app.action("view-tree").is_err());
    }
    #[test]
    fn staging_last_untracked_file_refreshes_synthetic_main_parent() {
        let root = env::temp_dir().join(format!(
            "tig-main-untracked-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        assert!(Command::new("git")
            .current_dir(&root)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success());
        let repo = Repository::discover(&root).unwrap();
        repo.command(["config", "user.name", "Test"]).unwrap();
        repo.command(["config", "user.email", "test@example.invalid"])
            .unwrap();
        repo.command(["config", "commit.gpgsign", "false"]).unwrap();
        fs::write(root.join("tracked"), "base\n").unwrap();
        repo.command(["add", "tracked"]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();
        fs::write(root.join("new"), "new\n").unwrap();
        let mut app = App {
            repo: Some(repo),
            config: Config::defaults(),
            view: View::new("main"),
            help: None,
            previous: vec![],
            pending_command: None,
            other: None,
            split: false,
            parent_focused: false,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            args: vec![],
            message: String::new(),
            search: String::new(),
            width: 80,
            height: 20,
        };
        app.view = app.load("main").unwrap();
        assert!(matches!(
            app.selected(),
            Item::Changes(ChangeKind::Untracked)
        ));
        app.enter().unwrap();
        assert_eq!(app.view.name, "status");
        app.view.selected = 2;
        app.action("status-update").unwrap();
        assert_eq!(app.view.name, "main");
        assert!(matches!(app.selected(), Item::Changes(ChangeKind::Staged)));
        fs::write(root.join("tracked"), "working\n").unwrap();
        app.action("refresh").unwrap();
        app.view.selected = app
            .view
            .items
            .iter()
            .position(|item| matches!(item, Item::Changes(ChangeKind::Unstaged)))
            .unwrap();
        app.action("view-diff").unwrap();
        assert_eq!(app.view.name, "stage");
        assert!(app
            .view
            .raw_patch
            .windows(b"+working".len())
            .any(|part| part == b"+working"));
        assert!(!app.split);
        assert!(app.other.is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changes_follow_index_order_and_do_not_stage_unmerged_entries() {
        let entry = |index, worktree| StatusEntry {
            index,
            worktree,
            path: PathBuf::from("file"),
            original_path: None,
        };
        let entries = [
            entry('?', '?'),
            entry(' ', 'M'),
            entry('M', ' '),
            entry('U', 'U'),
        ];
        assert_eq!(
            changes(&entries, true),
            vec![
                ChangeKind::Untracked,
                ChangeKind::Unstaged,
                ChangeKind::Staged
            ]
        );
        assert_eq!(
            changes(&[entry('U', 'U')], true),
            vec![ChangeKind::Unstaged]
        );
        assert_eq!(changes(&[entry('?', '?')], false), Vec::<ChangeKind>::new());
        for conflict in [
            entry('U', 'D'),
            entry('D', 'U'),
            entry('A', 'A'),
            entry('D', 'D'),
        ] {
            assert_eq!(status_mark(&conflict, 0), None);
            assert_eq!(status_mark(&conflict, 1), Some('U'));
            assert_eq!(changes(&[conflict], true), vec![ChangeKind::Unstaged]);
        }
        assert_eq!(status_mark(&entry('M', ' '), 0), Some('M'));
        assert_eq!(status_mark(&entry(' ', 'M'), 1), Some('M'));
        assert_eq!(status_mark(&entry('?', '?'), 2), Some('?'));
    }
    #[test]
    fn log_message_cannot_replace_selected_commit() {
        assert_eq!(log_header_offset("commit 0123456789"), Some(0));
        assert_eq!(log_header_offset("| * commit 0123456789"), Some(4));
        assert_eq!(log_header_offset("    commit 0123456789"), None);
        assert_eq!(log_header_offset("|     commit 0123456789"), None);
        assert_eq!(log_header_offset("+commit 0123456789"), None);
    }
    #[test]
    fn titles_split_focus_close_and_refresh_keep_context() {
        let mut parent = View::text("pager", "one\ntwo\nthree\nfour\nfive\nsix");
        parent.revision = "parent".into();
        parent.path = "parent.txt".into();
        parent.selected = 4;
        let mut child = View::text("pager", "child one\nchild two");
        child.revision = "child".into();
        child.path = "child.txt".into();
        child.left = 1;
        let mut app = App {
            repo: None,
            config: Config::default(),
            view: child,
            help: None,
            previous: vec![],
            pending_command: None,
            other: Some(parent),
            split: true,
            parent_focused: false,
            revision: "child".into(),
            path: "child.txt".into(),
            args: vec![],
            message: String::new(),
            search: String::new(),
            width: 80,
            height: 16,
        };
        app.config
            .settings
            .insert("vertical-split".into(), vec!["no".into()]);
        let screen = app.screen();
        assert_eq!(screen.len(), 16);
        assert!(screen[4].starts_with("[pager] - line 5 of 6"));
        assert!(screen[4].ends_with("83%"));
        assert!(screen[14].ends_with("100%"));
        app.action("view-next").unwrap();
        assert!(app.parent_focused);
        assert_eq!(app.path, PathBuf::from("parent.txt"));
        app.action("refresh").unwrap();
        assert_eq!(app.view.selected, 4);
        assert_eq!(app.view.top, 1);
        app.action("view-next").unwrap();
        assert_eq!(app.view.left, 1);
        app.action("maximize").unwrap();
        assert!(!app.split);
        app.action("view-close").unwrap();
        assert_eq!(app.view.revision, "parent");
        assert_eq!(app.view.selected, 4);
        assert!(app.other.is_none());
        app.view.selected = 5;
        let lines = pane_screen(&mut app.view, &app.config, 30, 2);
        assert!(lines[2].ends_with("100%"));
        assert_eq!(cell_width(&lines[2]), 30);
    }
    #[test]
    fn terminal_content_is_safe_and_cell_clipped() {
        assert_eq!(clip("a界b", 0, 3), "a界");
        assert_eq!(clip("é界b", 1, 3), "界b");
        assert_eq!(clip("\x1b[31m", 0, 20), "\\x1b[31m");
        assert_eq!(clip("x\ty", 0, 20), "x       y");
    }

    #[test]
    fn diff_line_column_and_title_leave_patch_rows_intact() {
        let mut config = Config::default();
        config.settings.insert(
            "diff-view".into(),
            vec!["line-number:yes,interval=5".into(), "text".into()],
        );
        config
            .settings
            .insert("line-graphics".into(), vec!["ascii".into()]);
        let mut view = View::text(
            "diff",
            "commit abc\n---\n file | 1 +\ndiff --git a/file b/file\n+++ b/file\n+text\n",
        );
        view.selected = 5;
        let original = view.rows.clone();
        let lines = pane_screen(&mut view, &config, 90, 6);
        assert_eq!(lines[0], "  1| commit abc");
        assert_eq!(lines[1], "   | ---");
        assert_eq!(lines[4], "  5| +++ b/file");
        assert!(lines[6].starts_with("[diff] Changes to 'file' - line 6 of 6"));
        assert_eq!(view.rows, original);
        assert_eq!(pane_screen(&mut view, &config, 90, 8)[6], "");
        view.left = 5;
        assert_eq!(pane_screen(&mut view, &config, 90, 6)[0], "commit abc");
        view.left = 0;
        assert_eq!(
            diff_edit_target(&view.rows, 5),
            Some((PathBuf::from("file"), 0))
        );
        view.selected = 2;
        assert!(pane_screen(&mut view, &config, 90, 6)[6]
            .starts_with("[diff] Press '<Enter>' to jump to file diff"));
    }

    #[test]
    fn diff_reload_tracks_source_line_within_the_same_file() {
        let prefix = "diff --git a/file b/file\n--- a/file\n+++ b/file\n";
        let mut old = View::text(
            "diff",
            &format!("{prefix}@@ -10,3 +10,3 @@\n same\n-old\n+new\n end\n"),
        );
        old.selected = 6;
        let expanded = View::text("diff", &format!("{prefix}@@ -9,5 +9,5 @@\n more\n same\n-old\n+new\n end\n more\ndiff --git a/other b/other\n"));
        // Like C, restoration picks the first row at the new-file line number.
        assert_eq!(diff_reloaded_line(&old, &expanded.rows), Some(6));
        old.selected = 7;
        assert_eq!(diff_reloaded_line(&old, &expanded.rows), Some(8));
        old.selected = 2;
        assert_eq!(diff_reloaded_line(&old, &expanded.rows), None);
        old.selected = 7;
        assert_eq!(diff_reloaded_line(&old, &[]), None);
        let missing = View::text("diff", &format!("{prefix}@@ -1 +1 @@\n elsewhere\ndiff --git a/other b/other\n@@ -12 +12 @@\n end\n"));
        assert_eq!(diff_reloaded_line(&old, &missing.rows), None);
        old = View::text("diff", &format!("{prefix}@@ -1 +1 @@\n first\ndiff --cc combined\n+++ b/combined\n@@@ -1,1 -1,1 +1,1 @@@\n  combined\n"));
        old.selected = old.rows.len() - 1;
        assert_eq!(diff_reloaded_line(&old, &expanded.rows), None);
    }

    #[test]
    fn diff_stat_jump_ignores_commit_message_bars() {
        let rows: Vec<String> = [
            "commit abc",
            "    message a | b",
            "---",
            " first | 1 +",
            " second | 1 +",
            "diff --git a/first b/first",
            "diff --git a/second b/second",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(diff_stat_header(&rows, 1), None);
        assert_eq!(diff_stat_header(&rows, 3), Some(5));
        assert_eq!(diff_stat_header(&rows, 4), Some(6));
    }
}
