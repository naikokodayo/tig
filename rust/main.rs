// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Safe Rust migration of Tig. SPDX-License-Identifier: GPL-2.0-or-later
#![forbid(unsafe_code)]
mod prompt;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers, MouseEventKind},
    execute, queue,
    style::{Attribute, SetAttribute},
    terminal::{self, Clear, ClearType},
};
use prompt::{clip_prompt, complete_prompt_action, inputrc_motion, prompt_text, PromptHistory};
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
    file_finder::{FileFinder, Input as FinderInput},
    git::{validate_diff_options, Repository},
    grep::{safe_grep_path, GrepLine},
    help_view::HelpView,
    model::{BlameLine, Commit, StatusEntry, TreeEntry},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn diff_options(config: &Config) -> Result<&[String]> {
    let options = config
        .settings
        .get("diff-options")
        .map_or(&[][..], Vec::as_slice);
    validate_diff_options(options)?;
    Ok(options)
}
fn word_diff_enabled(config: &Config) -> bool {
    config.bool_value("word-diff", false)
        || (!config.word_diff_cli_seen
            && config.settings.get("diff-options").is_some_and(|options| {
                options.iter().any(|option| {
                    matches!(option.as_str(), "--word-diff" | "--word-diff=plain")
                        || option.starts_with("--word-diff-regex=")
                })
            }))
}
fn highlight_diff(config: &Config, input: &[u8]) -> String {
    let program = match config.value("diff-highlight") {
        Some("yes" | "true" | "1") => "diff-highlight",
        Some("no" | "false" | "0" | "") | None => return String::from_utf8_lossy(input).into(),
        Some(program) => program,
    };
    if word_diff_enabled(config) {
        return String::from_utf8_lossy(input).into();
    }
    let result = Command::new(program)
        .env("GIT_CONFIG", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            let mut stdin = child.stdin.take().expect("piped highlight stdin");
            let (output, written) = std::thread::scope(|scope| {
                let writer = scope.spawn(move || stdin.write_all(input));
                (child.wait_with_output(), writer.join())
            });
            written.map_err(|_| io::Error::other("highlight writer panicked"))??;
            output
        });
    match result {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).into(),
        _ => {
            eprintln!("tig warning: Failed to run the diff-highlight program: {program}");
            String::from_utf8_lossy(input).into()
        }
    }
}
const HELP: &str = "Tig Rust migration (compatibility work in progress)\n\nUsage: tig [-C path] [log|show|reflog|blame|grep|refs|stash|status] [arguments]\n       git show | tig\n\nKeys: j/k move, Enter open, q back/quit, Q quit, / search, n next match\n      m history, d diff, s status, t tree, r refs, b blame, h help, R refresh\n      u stage/unstage; ! revert selected unstaged file (confirmation required)\n      Conflicts: :status-revert ours or :status-revert theirs, then u\n      Horizontal arrows scroll\n\nThis version is not yet a drop-in replacement for upstream Tig. See MIGRATION.md.";

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
        boundary: false,
        annotated: false,
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
struct WrappedText {
    width: usize,
    tab_size: usize,
    source: Vec<String>,
    // Source row and continuation marker for each displayed row.
    lines: Vec<(usize, bool)>,
}

#[derive(Clone)]
struct View {
    name: String,
    rows: Vec<String>,
    row_types: Vec<&'static str>,
    commit_fields: Vec<tig_rs::render::CommitField>,
    commit_row_widths: Vec<usize>,
    rendered_top: usize,
    wrapping: Option<WrappedText>,
    items: Vec<Item>,
    line_numbers: Vec<usize>,
    selected: usize,
    top: usize,
    left: usize,
    revision: String,
    path: PathBuf,
    staged: bool,
    diff_base: Option<String>,
    untracked: bool,
    raw_patch: Vec<u8>,
    from_stdin: bool,
    sort_field: Option<String>,
    sort_reverse: bool,
    args: Vec<String>,
    history: Vec<(usize, usize, usize)>,
    command_title: String,
    grep_source: Option<GrepLine>,
}
impl View {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            rows: vec![],
            row_types: vec![],
            commit_fields: vec![],
            commit_row_widths: vec![],
            rendered_top: 0,
            wrapping: None,
            items: vec![],
            line_numbers: vec![],
            selected: 0,
            top: 0,
            left: 0,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            staged: false,
            diff_base: None,
            untracked: false,
            raw_patch: Vec::new(),
            from_stdin: false,
            sort_field: None,
            sort_reverse: false,
            args: Vec::new(),
            history: Vec::new(),
            command_title: String::new(),
            grep_source: None,
        }
    }
    fn push(&mut self, text: String, item: Item) {
        self.line_numbers.push(self.rows.len() + 1);
        self.rows.push(text);
        self.items.push(item);
        self.row_types.push("default");
    }
    fn push_typed(&mut self, text: String, item: Item, kind: &'static str) {
        self.push(text, item);
        *self.row_types.last_mut().unwrap() = kind;
    }
    fn text(name: &str, text: &str) -> Self {
        let mut v = Self::new(name);
        for line in text.lines() {
            v.push(line.into(), Item::Text);
        }
        v
    }
    fn source_rows(&self) -> &[String] {
        self.wrapping
            .as_ref()
            .map_or(&self.rows, |wrap| &wrap.source)
    }
    fn source_index(&self, index: usize) -> usize {
        self.wrapping
            .as_ref()
            .and_then(|wrap| wrap.lines.get(index))
            .map_or(index, |line| line.0)
    }
    fn display_index(&self, source: usize) -> usize {
        self.wrapping
            .as_ref()
            .and_then(|wrap| wrap.lines.iter().position(|line| line.0 == source))
            .unwrap_or(source)
    }
    fn wrap_text(&mut self, config: &Config, width: usize) {
        if !matches!(self.name.as_str(), "blob" | "diff") {
            return;
        }
        let enabled = config.bool_value("wrap-lines", false);
        if !enabled && self.wrapping.is_none() {
            return;
        }
        let tab_size = config.usize_value("tab-size", 8).max(1);
        if self
            .wrapping
            .as_ref()
            .is_some_and(|wrap| enabled && wrap.width == width && wrap.tab_size == tab_size)
        {
            return;
        }
        let selected = self.source_index(self.selected);
        let top = self.source_index(self.top);
        if let Some(wrap) = self.wrapping.take() {
            self.rows = wrap.source;
        }
        self.selected = selected;
        self.top = top;
        self.line_numbers = (1..=self.rows.len()).collect();
        if enabled {
            let source = std::mem::take(&mut self.rows);
            let mut lines = Vec::new();
            self.line_numbers.clear();
            let mut number = 0;
            for (index, text) in source.iter().enumerate() {
                let first = self.rows.len();
                for (part, chunk) in tig_rs::render::wrap_line(text, width, tab_size, first != 0)
                    .into_iter()
                    .enumerate()
                {
                    let continued = first != 0 && part != 0;
                    self.rows.push(chunk.into());
                    lines.push((index, continued));
                    // C's first row uses zero as the continuation sentinel.
                    if !continued {
                        number += 1;
                    }
                    self.line_numbers.push(number);
                }
            }
            self.wrapping = Some(WrappedText {
                width,
                tab_size,
                source,
                lines,
            });
            self.selected = self.display_index(selected);
            self.top = self.display_index(top);
        }
        self.items = vec![Item::Text; self.rows.len()];
        self.row_types = vec!["default"; self.rows.len()];
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
        (self.rows, self.commit_fields) =
            tig_rs::render::render_commit_fields(&config, &commits, width)?;
        self.commit_row_widths = vec![width; self.rows.len()];
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
    watch: tig_rs::watch::Watch,
    repo: Option<Repository>,
    config: Config,
    view: View,
    help: Option<HelpView>,
    tree_initialized: bool,
    finder: Option<FileFinder>,
    previous: Vec<View>,
    pending_command: Option<tig_rs::commands::PreparedCommand>,
    pending_revert: Option<tig_rs::status_ops::RevertPlan>,
    prompt_answers: Vec<String>,
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
        let (vertical, parent, child) = self.pane_sizes();
        let width = if self.split && self.other.is_some() && vertical {
            if self.parent_focused {
                parent
            } else {
                child
            }
        } else {
            self.width
        };
        self.load_width(name, width)
    }
    fn load_width(&self, name: &str, width: usize) -> Result<View> {
        let mut view = self.load_content(name, width)?;
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
        view.wrap_text(&self.config, width);
        Ok(view)
    }
    fn file_filter(&self) -> &[String] {
        if self.config.bool_value("file-filter", true) {
            self.args
                .iter()
                .position(|arg| arg == "--")
                .map_or(&[][..], |i| &self.args[i + 1..])
        } else {
            &[]
        }
    }
    fn stage_diff(&self, staged: bool, file: Option<&std::path::Path>) -> Result<Vec<u8>> {
        let repo = self.repo()?;
        Ok(if !staged && file.is_none() {
            repo.worktree_diff_bytes_filtered(None, self.file_filter())?
        } else {
            repo.diff_bytes_filtered(staged, file, self.file_filter())?
        })
    }
    fn status_view(&self, untracked_only: bool) -> Result<View> {
        let repo = self.repo()?;
        let show_untracked = self.config.bool_value("status-show-untracked-files", true);
        let header = repo.status_header()?;
        let entries = repo.status_filtered(self.file_filter(), show_untracked)?;
        let mut v = View::new("status");
        v.untracked = untracked_only;
        v.args = self.args.clone();
        v.revision = self.revision.clone();
        v.push_typed(header, Item::Text, "header");
        for (group, title) in [
            (0, "Changes to be committed:"),
            (1, "Changes not staged for commit:"),
            (2, "Untracked files:"),
        ] {
            if untracked_only && group != 2 {
                continue;
            }
            let kind = ["stat-staged", "stat-unstaged", "stat-untracked"][group];
            v.push_typed(title.into(), Item::Text, kind);
            if group == 2 && !show_untracked {
                v.push_typed("  (not shown)".into(), Item::Text, "stat-none");
                continue;
            }
            let start = v.rows.len();
            for e in &entries {
                if let Some(mark) = status_mark(e, group) {
                    v.push_typed(
                        format!("{} {}", mark, e.path.display()),
                        Item::Status(e.clone(), group == 0),
                        kind,
                    );
                }
            }
            if v.rows.len() == start {
                v.push_typed("  (no files)".into(), Item::Text, "stat-none");
            }
        }
        Ok(v)
    }
    fn load_content(&self, name: &str, width: usize) -> Result<View> {
        let mut v = View::new(name);
        if name == "help" {
            let help = self
                .help
                .clone()
                .unwrap_or_else(|| HelpView::new(&self.config, &self.view.name));
            for row in help.rows {
                v.push_typed(row.text, Item::Text, row.line_type);
            }
            return Ok(v);
        }
        if name == "diff" && self.view.name == "diff" && self.view.from_stdin {
            return Ok(self.view.clone());
        }
        if name == "main" && self.view.name == "main" && self.view.from_stdin {
            let mut view = self.view.clone();
            view.redraw_stdin(&self.config, width)?;
            return Ok(view);
        }
        let repo = self.repo()?;
        match name {
            "main" => {
                let mut args = self
                    .config
                    .settings
                    .get("main-options")
                    .cloned()
                    .unwrap_or_default();
                args.extend(self.args.iter().cloned());
                let options = tig_rs::git::HistoryOptions::parse(&args)?;
                let mut config = self.config.clone();
                let configured_graph = tig_rs::render::main_graph_enabled(&config);
                let graph = options.with_graph && configured_graph;
                let order = self.config.value("commit-order").unwrap_or("auto");
                let order = if order == "auto" && !configured_graph {
                    "default"
                } else {
                    order
                };
                let commits = repo.history_ordered(
                    &args,
                    0,
                    order,
                    self.config.value("show-notes").unwrap_or("yes"),
                )?;
                if !graph || order == "reverse" {
                    config
                        .settings
                        .insert("main-view-commit-title-graph".into(), vec!["no".into()]);
                }
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
                let (rows, fields) =
                    tig_rs::render::render_commit_fields(&config, &display, width)?;
                v.commit_fields = fields;
                v.commit_row_widths = vec![width; rows.len()];
                for (row, item) in rows.into_iter().zip(items) {
                    let kind = match &item {
                        Item::Changes(ChangeKind::Untracked) => "stat-untracked",
                        Item::Changes(ChangeKind::Unstaged) => "stat-unstaged",
                        Item::Changes(ChangeKind::Staged) => "stat-staged",
                        Item::Commit(commit) if commit.annotated => "main-annotated",
                        _ => "main-commit",
                    };
                    v.push_typed(row, item, kind);
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
                    let kind = match &row.entry {
                        None => "header",
                        Some(entry) if entry.kind == "tree" => "directory",
                        Some(entry) if entry.kind == "blob" => "file",
                        Some(_) => "default",
                    };
                    v.push_typed(
                        row.text,
                        row.entry.map(Item::Tree).unwrap_or(Item::Text),
                        kind,
                    );
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
                let lower_bound = self.args.iter().find_map(|arg| arg.strip_prefix('^'));
                let blame = repo.blame(Some(&self.revision), &self.path, lower_bound)?;
                for (row, line) in tig_rs::render::render_blame(&self.config, &blame, self.width)?
                    .into_iter()
                    .zip(blame)
                {
                    v.push(row, Item::Blame(line));
                }
            }
            "diff" => {
                let diff_options = diff_options(&self.config)?;
                let oid = repo.revision(&self.revision)?;
                let diff_base = if self.view.name == "stash" {
                    Some(repo.revision(&format!("{oid}^"))?)
                } else if self.view.name == "diff" {
                    self.view.diff_base.clone()
                } else {
                    None
                };
                if let Some(base) = &diff_base {
                    let context = format!("-U{}", self.config.usize_value("diff-context", 3));
                    let mut args = vec![
                        "diff",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--stat",
                        "--patch",
                        &context,
                    ];
                    args.extend(diff_options.iter().map(String::as_str));
                    args.extend([
                        if word_diff_enabled(&self.config) {
                            "--word-diff=plain"
                        } else {
                            "--word-diff=none"
                        },
                        "--no-ext-diff",
                        "--no-textconv",
                        base.as_str(),
                        oid.as_str(),
                        "--",
                    ]);
                    let text = repo.command(args)?;
                    let mut view = View::text(name, &highlight_diff(&self.config, &text));
                    view.revision = oid;
                    view.diff_base = diff_base;
                    return Ok(view);
                }
                let text = repo.show(
                    &oid,
                    self.config.usize_value("diff-context", 3),
                    word_diff_enabled(&self.config),
                    diff_options,
                    (self.config.bool_value("file-filter", true)
                        && !self.path.as_os_str().is_empty())
                    .then_some(self.path.as_path()),
                    width,
                )?;
                let shown = highlight_diff(&self.config, text.as_bytes());
                let mut view = View::text(name, &shown);
                view.revision = if shown.starts_with("commit ") {
                    oid.clone()
                } else {
                    "HEAD".into()
                };
                if shown.starts_with("commit ") {
                    if let Some(commit) = repo.history(&[oid, "--".into()], 1)?.first() {
                        let mut refs =
                            tig_rs::render::refs(&self.config, &commit.decorations, ", ");
                        // C creates an empty Refs line only when annotated tags exist.
                        let describe = !refs.is_empty()
                            || (commit.decorations.is_empty()
                                && repo.refs().is_ok_and(|refs| {
                                    refs.iter().any(|reference| {
                                        reference.name.starts_with("refs/tags/")
                                            && !reference.target.is_empty()
                                    })
                                }));
                        if describe
                            && !commit
                                .decorations
                                .split(", ")
                                .any(|r| r.starts_with("tag: "))
                        {
                            if let Ok(description) =
                                repo.command(["describe", "--tags", &commit.oid])
                            {
                                let description = String::from_utf8_lossy(&description);
                                if !description.trim().is_empty() {
                                    if !refs.is_empty() {
                                        refs.push_str(", ");
                                    }
                                    refs.push_str(description.trim());
                                }
                            }
                        }
                        if !refs.is_empty() && !view.rows.is_empty() {
                            view.rows.insert(1, format!("Refs: {refs}"));
                            view.items.insert(1, Item::Text);
                            view.row_types.insert(1, "pp-refs");
                            view.line_numbers = (1..=view.rows.len()).collect();
                        }
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
                let file = if self.path.as_os_str().is_empty() {
                    None
                } else {
                    Some(self.path.as_path())
                };
                let raw = self.stage_diff(self.view.staged, file)?;
                let mut view = View::text(name, &String::from_utf8_lossy(&raw));
                view.raw_patch = raw;
                return Ok(view);
            }
            "blob" => {
                if let Some(hit) = &self.view.grep_source {
                    let bytes = repo.grep_blob(hit)?;
                    let mut view = View::text(name, &String::from_utf8_lossy(&bytes));
                    view.grep_source = Some(hit.clone());
                    return Ok(view);
                }
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
                // Width does not enable stats when log-options disables them.
                args.push(format!("--stat-width={width}"));
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
                let hits = repo.grep(&self.args)?;
                for (row, hit, kind) in tig_rs::grep::render_rows(hits, &self.config) {
                    v.push_typed(row, hit.map_or(Item::Text, Item::Grep), kind);
                }
            }
            "stash" | "reflog" => {
                let (commits, selectors) = repo.reflog(name == "stash", &self.args)?;
                let mut config = self.config.clone();
                if let Some(columns) = config.settings.get(&format!("{name}-view")).cloned() {
                    config.settings.insert("main-view".into(), columns);
                }
                config
                    .settings
                    .insert("main-view-commit-title-graph".into(), vec!["no".into()]);
                let (rows, fields) =
                    tig_rs::render::render_commit_fields(&config, &commits, width)?;
                v.commit_fields = fields;
                v.commit_row_widths = vec![width; rows.len()];
                for ((row, commit), selector) in rows.into_iter().zip(commits).zip(selectors) {
                    v.push_typed(row, Item::Ref(commit.oid, Some(selector)), "main-commit");
                }
            }
            _ => return Err(format!("Unsupported view: {name}").into()),
        }
        Ok(v)
    }
    fn finder_key(&mut self, key: &str) -> Result<()> {
        let Some(mut finder) = self.finder.take() else {
            return Ok(());
        };
        match finder.input(key, &self.config) {
            FinderInput::Continue => self.finder = Some(finder),
            FinderInput::Cancel => (),
            FinderInput::Accept => {
                let entry = finder.selected().ok_or("No file selected")?;
                let bytes = self.repo()?.blob(&entry.oid)?;
                let mut next = View::text("blob", &String::from_utf8_lossy(&bytes));
                next.path = entry.path.clone();
                next.revision = finder.revision;
                next.args = self.args.clone();
                next.wrap_text(&self.config, self.width);
                self.path = next.path.clone();
                self.revision = next.revision.clone();
                if self.view.name == "blob" || self.other.is_some() {
                    self.view = next;
                } else {
                    self.previous.push(std::mem::replace(&mut self.view, next));
                }
            }
        }
        Ok(())
    }
    fn open(&mut self, name: &str, width: usize) -> Result<()> {
        if name == "blob"
            && (self.path.as_os_str().is_empty()
                || self.view.name == "blob"
                || matches!(self.selected(), Item::Tree(ref entry) if entry.kind == "tree"))
        {
            self.finder = Some(FileFinder::load(self.repo()?, &self.revision)?);
            return Ok(());
        }
        if name == "help" {
            self.help = Some(HelpView::new(&self.config, &self.view.name));
        }
        // C tree_open applies repo.prefix only to the first tree view.
        let old_path = self.path.clone();
        if name == "tree" && !self.tree_initialized {
            self.path = self.repo()?.prefix()?;
        }
        let next = match self.load_width(name, width) {
            Ok(next) => next,
            Err(error) => {
                self.path = old_path;
                return Err(error);
            }
        };
        self.tree_initialized |= name == "tree";
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
            let raw = self.stage_diff(staged, None)?;
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
    fn refresh_parent(&mut self) -> Result<()> {
        let Some(old) = self
            .other
            .as_ref()
            .or_else(|| self.previous.last())
            .filter(|view| matches!(view.name.as_str(), "main" | "status"))
        else {
            return Ok(());
        };
        let selected = old.items.get(old.selected).cloned();
        let top = old.top;
        let args = std::mem::replace(&mut self.args, old.args.clone());
        let revision = std::mem::replace(&mut self.revision, old.revision.clone());
        let path = std::mem::replace(&mut self.path, old.path.clone());
        let next = if old.name == "status" {
            self.status_view(old.untracked)
        } else {
            self.load("main")
        };
        self.args = args;
        self.revision = revision;
        self.path = path;
        let mut next = next?;
        let target = if old.name == "status" && self.view.name == "stage" {
            next.items.iter().position(|item| matches!(item, Item::Status(entry, staged)
                if *staged == self.view.staged && (self.view.path.as_os_str().is_empty() || entry.path == self.view.path)))
                .and_then(|index| {
                    if self.view.path.as_os_str().is_empty() {
                        let title = if self.view.staged { "Changes to be committed:" } else { "Changes not staged for commit:" };
                        next.rows.iter().position(|row| row == title)
                    } else { Some(index) }
                })
        } else {
            selected.as_ref().and_then(|selected| {
                next.items.iter().position(|item| match (selected, item) {
                    (Item::Changes(a), Item::Changes(b)) => a == b,
                    (Item::Commit(a), Item::Commit(b)) => a.oid == b.oid,
                    _ => false,
                })
            })
        };
        next.selected = target.unwrap_or(if old.name == "status" {
            old.selected.min(next.rows.len().saturating_sub(1))
        } else {
            0
        });
        next.top = top;
        next.left = old.left;
        next.history = old.history.clone();
        if target.is_none() {
            next.restore_status_selection();
        }
        if self.other.is_some() {
            self.other = Some(next);
        } else if let Some(previous) = self.previous.last_mut() {
            *previous = next;
        }
        Ok(())
    }
    fn refresh_after_command(&mut self) -> Result<()> {
        if self.config.value("refresh-mode") == Some("manual") {
            return Ok(());
        }
        self.refresh_views()
    }
    fn poll_watch(&mut self) -> bool {
        if self.config.value("refresh-mode") != Some("periodic") {
            return false;
        }
        let Some(repo) = &self.repo else { return false };
        let seconds = self
            .config
            .value("refresh-interval")
            .unwrap_or("10")
            .parse::<u64>()
            .unwrap_or(0);
        match self.watch.poll(
            repo,
            std::time::Duration::from_secs(seconds),
            std::time::Instant::now(),
        ) {
            Ok(false) => false,
            Ok(true) => {
                if let Err(error) = self.refresh_views() {
                    self.message = error.to_string();
                }
                true
            }
            Err(error) => {
                self.message = error.to_string();
                true
            }
        }
    }
    fn reset_watch(&mut self) -> Result<()> {
        if self.config.value("refresh-mode") == Some("periodic") {
            if let Some(repo) = &self.repo {
                self.watch.reset(repo, std::time::Instant::now())?;
            }
        }
        Ok(())
    }
    fn refresh_views(&mut self) -> Result<()> {
        // Record before loading, so a concurrent change after a view's Git read
        // remains visible to the next poll. A failed pane keeps retrying.
        let baseline = self.reset_watch();
        let parent = if self.other.is_none() {
            self.refresh_parent()
        } else {
            Ok(())
        };
        let current = self.action("refresh");
        let other = if self.other.is_some() {
            self.swap_panes();
            let result = self.action("refresh");
            self.swap_panes();
            result.map(|_| ())
        } else {
            Ok(())
        };
        let result = baseline.and(parent).and(current.map(|_| ())).and(other);
        if result.is_err() {
            self.watch.retry();
        }
        result
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
    fn blame_forward(&mut self, parent: bool) -> Result<()> {
        let Item::Blame(line) = self.selected() else {
            return Ok(());
        };
        let (revision, path, selected) = if parent {
            let Some((revision, path)) = &line.previous else {
                self.message = "The selected commit has no parents with this file".into();
                return Ok(());
            };
            let mut from = OsString::from(format!("{revision}:"));
            from.push(path);
            let mut to = OsString::from(format!("{}:", line.oid));
            to.push(&line.filename);
            let patch = self.repo()?.command(vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                "--no-color".into(),
                "-U0".into(),
                from,
                to,
                "--".into(),
            ])?;
            let mut old_line = None;
            let mut new_line = 0;
            let mut selected = self.view.selected;
            for row in String::from_utf8_lossy(&patch).lines() {
                if row.starts_with("@@ ") {
                    old_line = diff_hunk_line(row, '-');
                    new_line = diff_hunk_line(row, '+').unwrap_or(0);
                } else if let (Some(old_line), Some(text)) = (old_line, row.strip_prefix('+')) {
                    if new_line == line.original_line && text == line.text {
                        selected = old_line.saturating_sub(1);
                        break;
                    }
                    new_line += 1;
                }
            }
            (revision.clone(), path.clone(), selected)
        } else {
            if line.oid.bytes().all(|byte| byte == b'0') {
                self.message = "No commit exists for the selected line".into();
                return Ok(());
            }
            (
                line.oid,
                line.filename,
                line.original_line.saturating_sub(1),
            )
        };
        if revision == self.view.revision && path == self.view.path {
            self.message = "The selected commit is already displayed".into();
            return Ok(());
        }
        let top = self.view.top;
        self.revision = revision;
        self.path = path;
        self.open("blame", self.width)?;
        self.view.selected = selected.min(self.view.rows.len().saturating_sub(1));
        self.view.top = top;
        self.center_selection();
        Ok(())
    }
    fn trace_blame(&mut self) -> Result<()> {
        let selected = self.view.source_index(self.view.selected);
        let rows = self.view.source_rows();
        let row = rows.get(selected).ok_or("No selected diff line")?;
        let hunk = rows[..=selected].iter().rfind(|row| {
            row.starts_with("@@") || row.starts_with("diff ") || row.starts_with("commit ")
        });
        if row.starts_with("@@") || !hunk.is_some_and(|row| row.starts_with("@@ ")) {
            return Err("The line to trace must be inside an ordinary diff chunk".into());
        }
        let old = row.starts_with('-');
        let (path, number) = diff_target(rows, selected, old).ok_or("No file and line to blame")?;
        let revision = rows[..=selected]
            .iter()
            .rev()
            .find_map(|row| row.strip_prefix("commit "))
            .unwrap_or(&self.view.revision);
        let revision = if old {
            format!("{revision}^")
        } else {
            revision.to_owned()
        };
        let origin = self
            .repo()?
            .blame(Some(&revision), &path, None)?
            .into_iter()
            .find(|line| line.line == number)
            .ok_or("No blame for selected line")?;
        self.revision = origin.oid;
        self.path = origin.filename;
        self.open("blame", self.width)?;
        self.view.selected = origin.original_line.saturating_sub(1);
        self.center_selection();
        Ok(())
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
    fn enter(&mut self, split: bool) -> Result<()> {
        if self.view.name == "help" {
            if let Some(help) = &mut self.help {
                if help.toggle_section(self.view.selected, &self.config) {
                    self.view.rows = help.rows.iter().map(|row| row.text.clone()).collect();
                    self.view.items = vec![Item::Text; self.view.rows.len()];
                    self.view.row_types = help.rows.iter().map(|row| row.line_type).collect();
                    self.view.line_numbers = (1..=self.view.rows.len()).collect();
                    self.view.selected = self
                        .view
                        .selected
                        .min(self.view.rows.len().saturating_sub(1));
                }
            }
            return Ok(());
        }
        if self.view.name == "diff" || (self.view.name == "stage" && !self.view.untracked) {
            let header = if self.view.name == "diff" {
                diff_stat_header(
                    self.view.source_rows(),
                    self.view.source_index(self.view.selected),
                )
            } else {
                stage_stat_header(&self.view.rows, self.view.selected)
            };
            if let Some(header) = header {
                self.view.selected = self.view.display_index(header);
                self.center_selection();
                return Ok(());
            }
        }
        let parent = self.view.clone();
        let depth = self.previous.len();
        let (vertical, _, child) = self.pane_sizes();
        let child_width = if split && vertical { child } else { self.width };
        match self.selected() {
            Item::Commit(c) => {
                self.revision = c.oid;
                self.open("diff", child_width)?;
            }
            Item::Changes(kind) => self.open_changes(kind)?,
            Item::Ref(id, _) => {
                self.revision = id.clone();
                if matches!(self.view.name.as_str(), "refs" | "reflog") {
                    self.args = vec![id];
                    self.open("main", child_width)?;
                } else {
                    self.open("diff", child_width)?;
                }
            }
            Item::Blame(line) => {
                self.revision = line.oid;
                self.path = line.filename.clone();
                self.open("diff", child_width)?;
                let start = self
                    .view
                    .source_rows()
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.starts_with("diff --git "))
                    .find_map(|(index, _)| {
                        (diff_edit_target(self.view.source_rows(), index)
                            == Some((line.filename.clone(), 0)))
                        .then_some(index)
                    });
                if let Some(selected) = start.and_then(|start| {
                    diff_line_at(self.view.source_rows(), start, line.original_line)
                }) {
                    self.view.selected = self.view.display_index(selected);
                }
            }
            Item::Text if self.view.name == "refs" && self.view.selected == 0 => {
                self.args = vec!["--all".into()];
                self.open("main", child_width)?;
            }
            Item::Tree(e) => {
                if e.kind == "tree" && self.view.path.parent() == Some(e.path.as_path()) {
                    return self.tree_parent();
                }
                self.path = e.path.clone();
                if e.kind == "tree" {
                    self.open("tree", self.width)?;
                    return Ok(());
                } else {
                    let bytes = self.repo()?.blob(&e.oid)?;
                    let mut v = View::text("blob", &String::from_utf8_lossy(&bytes));
                    v.path = self.path.clone();
                    v.revision = self.revision.clone();
                    v.wrap_text(&self.config, child_width);
                    self.previous.push(std::mem::replace(&mut self.view, v));
                }
            }
            Item::Status(e, staged) => {
                let raw = if e.index == '?' {
                    fs::read(self.repo()?.root.join(&e.path))?
                } else {
                    self.stage_diff(staged, Some(&e.path))?
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
                let bytes = self.repo()?.grep_blob(&hit)?;
                let source = hit.clone();
                self.path = if hit.revision.as_deref().is_some_and(|rev| rev.contains(':')) {
                    PathBuf::new()
                } else {
                    hit.path.clone()
                };
                self.revision = hit.revision.unwrap_or_else(|| "HEAD".into());
                let mut view = View::text("blob", &String::from_utf8_lossy(&bytes));
                view.grep_source = Some(source);
                view.path = self.path.clone();
                view.revision = self.revision.clone();
                view.selected = hit
                    .line
                    .saturating_sub(1)
                    .min(view.rows.len().saturating_sub(1));
                view.wrap_text(&self.config, child_width);
                self.previous.push(std::mem::replace(&mut self.view, view));
            }
            Item::Text if self.view.name == "status" => {
                let kind = match self.view.rows.get(self.view.selected).map(String::as_str) {
                    Some("Changes to be committed:") => Some(ChangeKind::Staged),
                    Some("Changes not staged for commit:") => Some(ChangeKind::Unstaged),
                    _ => None,
                };
                if let Some(kind) = kind {
                    self.open_changes(kind)?;
                }
            }
            Item::Text => (),
        }
        if self.previous.len() > depth {
            let from_grep = parent.name == "grep";
            self.previous.truncate(depth);
            self.other = Some(parent);
            self.split = split;
            self.parent_focused = false;
            if from_grep || self.other.as_ref().is_some_and(|view| view.name == "blame") {
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
            "diff" | "log" | "pager" => diff_edit_target(
                self.view.source_rows(),
                self.view.source_index(self.view.selected),
            ),
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
    fn finish_revert(&mut self, confirmed: bool) -> Result<()> {
        use tig_rs::status_ops::RevertOutcome;
        if let Some(plan) = self.pending_revert.take() {
            self.message = match plan.execute(self.repo()?, confirmed)? {
                RevertOutcome::Cancelled => "Revert cancelled".into(),
                RevertOutcome::Applied {
                    backup,
                    needs_stage,
                } => {
                    self.refresh_parent()?;
                    self.action("refresh")?;
                    format!(
                        "{}Recovery: {backup:?}",
                        if needs_stage {
                            "Use u (status-update) to mark resolved. "
                        } else {
                            ""
                        }
                    )
                }
            };
        }
        Ok(())
    }
    fn action(&mut self, action: &str) -> Result<bool> {
        let was_split = self.split && self.other.is_some();
        if let Some(text) = action
            .strip_prefix(":echo ")
            .or_else(|| action.strip_prefix("echo "))
        {
            self.action(&format!("exec !echo {text}"))?;
            if let Some(command) = self.pending_command.take() {
                self.message = command.argv[1..]
                    .iter()
                    .map(|arg| arg.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            return Ok(true);
        }
        if let Some(command) = action.strip_prefix(":!") {
            self.action(&format!("exec !{command}"))?;
            if let Some(command) = self.pending_command.take() {
                let mut stdout = Vec::new();
                if command.argv.first().is_some_and(|arg| !arg.is_empty()) {
                    let mut output = command.run_allow_nonzero(self.repo()?, false, true)?;
                    // ponytail: stdout then stderr; preserve interleaving with future streaming loaders.
                    output.stdout.extend(output.stderr);
                    stdout = output.stdout;
                }
                if self.other.is_some() {
                    if !self.parent_focused {
                        self.swap_panes();
                    }
                    self.other = None;
                    self.split = false;
                    self.parent_focused = false;
                }
                self.refresh_after_command()?;
                let mut view = View::text("pager", &String::from_utf8_lossy(&stdout));
                view.command_title = command
                    .argv
                    .iter()
                    .map(|arg| arg.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ");
                view.args = self.args.clone();
                view.revision = self.revision.clone();
                view.path = self.path.clone();
                self.previous.push(std::mem::replace(&mut self.view, view));
            }
            return Ok(true);
        }
        let action = action.strip_prefix(':').unwrap_or(action);
        if action.split_whitespace().next() == Some("status-revert") {
            use tig_rs::status_ops::{RevertAction, RevertPlan};
            if self.view.name != "status" {
                return Err("File revert is available in the status view".into());
            }
            let action = match action
                .split_whitespace()
                .skip(1)
                .collect::<Vec<_>>()
                .as_slice()
            {
                [] => RevertAction::Unstaged,
                ["ours"] => RevertAction::Ours,
                ["theirs"] => RevertAction::Theirs,
                _ => return Err("Usage: status-revert [ours|theirs]".into()),
            };
            let Item::Status(entry, staged) = self.selected() else {
                return Err("Select one unstaged file to revert".into());
            };
            if entry.conflicted() && action == RevertAction::Unstaged {
                return Err(
                    "Choose :status-revert ours or :status-revert theirs, then confirm".into(),
                );
            }
            self.pending_revert = Some(RevertPlan::prepare(self.repo()?, &entry, staged, action)?);
            return Ok(true);
        }
        if action == "exec" {
            self.message = "Failed to execute command: No arguments".into();
            return Ok(true);
        }
        if let Some(command) = action.strip_prefix("exec ").or_else(|| {
            action
                .starts_with(['!', '@', '?', '<', '+', '>'])
                .then_some(action)
        }) {
            if action.starts_with("exec ")
                && command
                    .trim()
                    .chars()
                    .all(|c| matches!(c, '!' | '@' | '?' | '<' | '+' | '>'))
            {
                self.message = if command.trim().is_empty() {
                    "Failed to execute command: No arguments"
                } else {
                    "Failed to format arguments"
                }
                .into();
                return Ok(true);
            }
            use tig_rs::commands::ReferenceContext;
            let selection = self.selected();
            let selected_ref = match &selection {
                Item::Commit(commit) if self.view.name == "main" => {
                    Some(ReferenceContext::Commit(&commit.decorations))
                }
                Item::Ref(_, name) if self.view.name == "refs" => {
                    name.as_deref().map(ReferenceContext::Ref)
                }
                Item::Text if self.view.name == "refs" => Some(ReferenceContext::Ref("")),
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
            let prompt_answers = std::mem::take(&mut self.prompt_answers);
            self.pending_command = Some(tig_rs::commands::prepare_with_context(
                self.repo()?,
                command,
                &self.revision,
                &file,
                line,
                selected_ref,
                tig_rs::commands::ExpansionInput {
                    args: &self.args,
                    prompt_answers: &prompt_answers,
                },
            )?);
            return Ok(true);
        }
        if action.split_whitespace().next() == Some("save-view") {
            let args = tig_rs::config::words(action)?;
            let path = args.get(1).map_or("tig-view.txt", String::as_str);
            if args.len() > 2 || path.is_empty() {
                return Err("save-view expects one nonempty path".into());
            }
            let text_view = matches!(self.view.name.as_str(), "diff" | "stage" | "pager" | "log");
            if text_view && word_diff_enabled(&self.config) {
                return Err("save-view does not support word diff views yet".into());
            }
            if text_view
                && (self.config.bool_value("wrap-lines", false) || self.view.wrapping.is_some())
            {
                return Err(format!(
                    "save-view does not support wrapped {} views yet",
                    self.view.name
                )
                .into());
            }
            if text_view && self.config.color_commands != Config::defaults().color_commands {
                return Err("save-view does not support custom color rules yet".into());
            }
            if text_view && self.view.rows.iter().any(|row| row.contains('\x1b')) {
                return Err("save-view does not support ANSI-highlighted cells yet".into());
            }
            self.screen(false);
            let (vertical, parent, child) = self.pane_sizes();
            let (width, height) = if self.split && self.other.is_some() {
                let size = if self.parent_focused { parent } else { child };
                if vertical {
                    (size, self.height.saturating_sub(2))
                } else {
                    (self.width, size.saturating_sub(1))
                }
            } else {
                (self.width, self.height.saturating_sub(2))
            };

            let previous = if self.parent_focused {
                self.previous.last()
            } else {
                self.other.as_ref().or_else(|| self.previous.last())
            };
            let parent = if self.parent_focused {
                None
            } else {
                self.other.as_ref()
            };
            let mut data = tig_rs::view_export::header(
                &self.view.name,
                previous.map(|view| view.name.as_str()),
                parent.map(|view| view.name.as_str()),
                &view_reference(&self.view),
                (width, height),
                (self.view.top, self.view.left, self.view.selected),
            );
            match self.view.name.as_str() {
                "diff" | "pager" | "stage" if !self.view.untracked => data.push_str(
                    &tig_rs::render::diff_view_data(&self.view.rows, self.view.selected),
                ),
                "log" => data.push_str(&tig_rs::view_export::log_data(
                    &self.view.rows,
                    self.view.selected,
                )),
                _ => {
                    for (index, row) in self.view.rows.iter().enumerate() {
                        let text = self.view.name == "blob"
                            || (self.view.name == "stage" && self.view.untracked);
                        let kind = if text {
                            "default"
                        } else {
                            self.view.row_types.get(index).copied().unwrap_or("default")
                        };
                        tig_rs::view_export::line(
                            &mut data,
                            index,
                            kind,
                            index == self.view.selected,
                            text.then_some(&[row.as_str()][..]),
                        );
                    }
                }
            }
            tig_rs::view_export::save(std::path::Path::new(path), &data)
                .map_err(|error| format!("Failed to save view to {path}: {error}"))?;
            self.message = format!("Saved view to {path}");
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
            self.action("refresh")?;
            if self.split && self.other.is_some() {
                self.swap_panes();
                let result = self.action("refresh");
                self.swap_panes();
                result?;
            }
            return Ok(true);
        }
        if let Some(pattern) = action.strip_prefix('/') {
            self.search = pattern.into();
            self.find(false);
            return Ok(true);
        }
        if let Some(number) = action
            .split_whitespace()
            .next()
            .filter(|word| word.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return self.action(&format!("goto {number}"));
        }
        if let Some(target) = action.strip_prefix("goto ") {
            if !target.is_empty() && target.bytes().all(|byte| byte.is_ascii_digit()) {
                match target.parse::<usize>() {
                    Ok(line) if line <= self.view.rows.len() => {
                        self.view.selected = line.saturating_sub(1);
                    }
                    _ => {
                        self.message = format!("Unable to parse '{target}' as a line number");
                        return Ok(true);
                    }
                }
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
            "none" => (),
            "quit" => return Ok(false),
            "parent" if self.view.name == "main" => {
                if self.view.history.last().map(|pos| pos.0) != Some(self.view.selected) {
                    self.view
                        .history
                        .push((self.view.selected, self.view.top, self.view.left));
                }
                match self.selected() {
                    Item::Changes(_) => self.view.move_by(1),
                    Item::Commit(commit) => {
                        if let Err(error) = self.goto_commit(&format!("{}^", commit.oid)) {
                            self.message = error.to_string();
                        }
                    }
                    _ => (),
                }
                self.center_selection();
            }
            "back" if self.view.name == "main" => {
                if let Some((selected, top, left)) = self.view.history.pop() {
                    self.view.selected = selected.min(self.view.rows.len().saturating_sub(1));
                    self.view.top = top;
                    self.view.left = left;
                } else {
                    self.message = "Already at start of history".into();
                }
            }
            "view-close" | "view-close-no-quit" | "back" => {
                if action != "back" && self.parent_focused && self.other.is_some() {
                    if let Some(v) = self.previous.pop() {
                        self.view = v;
                        self.sync_context();
                    } else if action == "view-close-no-quit" {
                        self.message = "Can't close last remaining view".into();
                        return Ok(true);
                    } else {
                        return Ok(false);
                    }
                    self.other = None;
                    self.split = false;
                    self.parent_focused = false;
                } else if self.other.is_some() {
                    if !self.parent_focused {
                        self.swap_panes();
                    }
                    self.other = None;
                    self.split = false;
                    self.parent_focused = false;
                } else if let Some(v) = self.previous.pop() {
                    self.view = v;
                    self.sync_context();
                } else {
                    if action != "view-close-no-quit" {
                        return Ok(false);
                    }
                    self.message = "Can't close last remaining view".into();
                }
                if self.config.value("refresh-mode") == Some("periodic") {
                    self.action("refresh")?;
                }
            }
            "enter" => self.enter(true)?,
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
                    self.enter(split)?;
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
            "scroll-left" | "scroll-right" | "scroll-first-col" => {
                let width = if self.split && self.other.is_some() {
                    let (vertical, parent, child) = self.pane_sizes();
                    if vertical {
                        if self.parent_focused {
                            parent
                        } else {
                            child
                        }
                    } else {
                        self.width
                    }
                } else {
                    self.width
                };
                let option = self.config.value("horizontal-scroll").unwrap_or("50%");
                // C parse_step uses the leading integer, even for decimal values.
                let amount = option
                    .split(['.', 'e', 'E', '%'])
                    .next()
                    .and_then(|number| number.parse::<usize>().ok())
                    .unwrap_or(0);
                let step = if option.ends_with('%') {
                    width.saturating_mul(amount) / 100
                } else {
                    amount
                }
                .max(1);
                self.view.commit_row_widths.fill(width);
                self.view.left = if action == "scroll-first-col" {
                    0
                } else if action == "scroll-left" {
                    self.view.left.saturating_sub(step)
                } else {
                    self.view.left.saturating_add(step)
                };
            }
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
                self.view.selected = self
                    .view
                    .display_index(old.source_index(old.selected))
                    .min(self.view.rows.len().saturating_sub(1));
                self.view.top = self.view.display_index(old.source_index(old.top));
                self.view.left = old.left;
                self.view.history = old.history.clone();
                self.view.restore_status_selection();
                if old.name == "diff" {
                    if let Some(selected) = diff_reloaded_line(&old, self.view.source_rows()) {
                        let selected = self.view.display_index(selected);
                        self.view.selected = selected;
                        self.view.top =
                            selected.saturating_sub(old.selected.saturating_sub(old.top));
                    }
                }
            }
            "status-update" | "stage-update-line" | "stage-update-part" | "stage-split-chunk"
                if self.view.name == "stage" =>
            {
                if self.view.untracked {
                    if self.view.rows.is_empty() {
                        return Ok(true);
                    }
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
                    self.refresh_parent()?;
                    if self
                        .other
                        .as_ref()
                        .is_some_and(|parent| parent.name == "status")
                    {
                        self.swap_panes();
                        if matches!(self.selected(), Item::Status(_, false)) {
                            self.enter(self.split)?;
                        } else {
                            self.other = None;
                            self.split = false;
                            self.parent_focused = false;
                        }
                        return Ok(true);
                    }
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
                        if action == "stage-split-chunk" {
                            if line.is_none()
                                && !self.view.rows[self.view.selected].starts_with("@@ ")
                            {
                                return Err("No chunks to split in sight".into());
                            }
                            let split = patch.split_hunk(file, hunk)?;
                            let range = split.range;
                            let boundaries: Vec<usize> = std::iter::once(0)
                                .chain(raw.split_inclusive(|b| *b == b'\n').scan(
                                    0,
                                    |offset, row| {
                                        *offset += row.len();
                                        Some(*offset)
                                    },
                                ))
                                .collect();
                            let mut raw = raw.clone();
                            raw.splice(
                                boundaries[prefix_rows + range.start]
                                    ..boundaries[prefix_rows + range.end],
                                split.patch,
                            );
                            let mut rows = self.view.rows.clone();
                            rows.splice(
                                prefix_rows + range.start..prefix_rows + range.end,
                                split.display,
                            );
                            self.view.items = vec![Item::Text; rows.len()];
                            self.view.line_numbers = (1..=rows.len()).collect();
                            self.view.rows = rows;
                            self.view.row_types = vec!["default"; self.view.rows.len()];
                            self.view.raw_patch = raw;
                            return Ok(true);
                        }
                        let selected = if action == "stage-update-part" {
                            patch.select_part(
                                file,
                                hunk,
                                line.ok_or("Select an added or removed line")?,
                                self.view.staged,
                            )?
                        } else {
                            let line = if action == "stage-update-line" {
                                Some(line.ok_or("Select an added or removed line")?)
                            } else {
                                None
                            };
                            patch.select(file, hunk, line, self.view.staged)?
                        };
                        tig_rs::patch::apply_cached(self.repo()?, &selected, self.view.staged)?;
                    }
                }
                self.refresh_parent()?;
                self.action("refresh")?;
                if self.view.rows.is_empty() && (self.other.is_some() || !self.previous.is_empty())
                {
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
                    self.refresh_parent()?;
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
            "parent" if self.view.name == "blame" => self.blame_forward(true)?,
            "view-blame" if self.view.name == "blame" => self.blame_forward(false)?,
            "view-blame" if matches!(self.view.name.as_str(), "diff" | "log" | "pager") => {
                self.trace_blame()?
            }
            "screen-redraw" => (),
            "view-diff" if self.view.name == "diff" => {
                if let Some(parent) = self.other.take() {
                    self.previous.push(parent);
                }
                self.split = false;
                self.parent_focused = false;
            }
            "view-diff" if self.view.name == "stage" => self.split = false,
            "view-stage" if matches!(self.view.name.as_str(), "main" | "status") => {
                if self.view.name == "main" && !matches!(self.selected(), Item::Changes(_)) {
                    return Err("No stage content; select working tree changes".into());
                }
                self.enter(true)?;
                if self.view.name == "stage" {
                    if let Some(parent) = self.other.take() {
                        self.previous.push(parent);
                    }
                    self.split = false;
                    self.parent_focused = false;
                }
            }
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
                self.open(&action[5..], self.width)?;
            }
            _ => return Err(format!("Not implemented in Rust yet: {action}").into()),
        }
        // Reload width-dependent rows when a vertical split opens/closes.
        if was_split != (self.split && self.other.is_some()) && self.pane_sizes().0 {
            if self.view.name == "log" || (was_split && self.view.name == "main") {
                self.action("refresh")?;
            }
            if self.split && self.other.as_ref().is_some_and(|view| view.name == "log") {
                self.swap_panes();
                let result = self.action("refresh");
                self.swap_panes();
                result?;
            }
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
            self.width
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
        let minimum = 4.min(total / 2);
        let child = child.max(minimum).min(total.saturating_sub(minimum));
        (
            vertical,
            total - child,
            child.saturating_sub(usize::from(vertical)),
        )
    }
    fn screen(&mut self, saved: bool) -> Vec<String> {
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
                    saved,
                );
                let right = pane_screen(
                    child,
                    &self.config,
                    child_size,
                    self.height.saturating_sub(2),
                    saved,
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
                    saved,
                );
                lines.extend(pane_screen(
                    child,
                    &self.config,
                    self.width,
                    child_size.saturating_sub(1),
                    saved,
                ));
                lines
            }
        } else {
            pane_screen(
                &mut self.view,
                &self.config,
                self.width,
                self.height.saturating_sub(2),
                saved,
            )
        };
        lines.push(clip(&self.message, 0, self.width));
        lines
    }
    fn script(&mut self, path: &str) -> Result<()> {
        let mut grep_prompt = false;
        let script = fs::read_to_string(path)?;
        let mut lines = script.lines();
        while let Some(raw) = lines.next() {
            self.screen(false);
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if self.finder.is_some() {
                let mut input = line;
                while !input.is_empty() && self.finder.is_some() {
                    let end = if input.starts_with('<') {
                        input
                            .find('>')
                            .map(|i| i + 1)
                            .ok_or("Unclosed finder key")?
                    } else {
                        input.chars().next().unwrap().len_utf8()
                    };
                    self.finder_key(&input[..end])?;
                    input = &input[end..];
                }
                continue;
            }
            if grep_prompt {
                self.grep_query(line.strip_suffix("<Enter>").unwrap_or(line))?;
                grep_prompt = false;
            } else if matches!(line, ":g" | ":view-grep") {
                grep_prompt = true;
            } else if let Some(path) = line.strip_prefix(":save-display ") {
                let mut screen = self.screen(true);
                screen.pop();
                // C save_display trims the combined line, including empty right panes.
                for line in &mut screen {
                    line.truncate(
                        line.trim_end_matches(|c: char| c.is_ascii_whitespace())
                            .len(),
                    );
                    if line.is_empty() {
                        line.push(' ');
                    }
                }
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
                let action = if line.starts_with(':') {
                    Some(line.to_string())
                } else {
                    self.binding(line)
                };
                let Some(action) = action else {
                    self.message = "Unknown key, press h for help".into();
                    continue;
                };
                self.prompt_answers.clear();
                for _ in tig_rs::commands::prompt_labels(&action) {
                    let answer = lines
                        .next()
                        .ok_or("Missing scripted command prompt answer")?
                        .trim();
                    self.prompt_answers.push(
                        answer
                            .strip_suffix("<Enter>")
                            .ok_or("Invalid scripted command prompt answer")?
                            .into(),
                    );
                }
                if !self.action(&action)? {
                    break;
                }
                self.finish_revert(false)?;
                if let Some(command) = self.pending_command.take() {
                    if command.silent && !command.echo {
                        command.run_allow_nonzero(self.repo()?, false, true)?;
                    } else {
                        command.run(self.repo()?, false, true)?;
                    }
                    if command.exit {
                        break;
                    }
                    self.refresh_after_command()?;
                }
            }
        }
        if self.finder.is_some() {
            return Err("Unfinished scripted file finder input".into());
        }
        Ok(())
    }
    fn binding(&self, key: &str) -> Option<String> {
        self.config.action(&self.view.name, key).map(|a| {
            if a.first()
                .is_some_and(|arg| arg.starts_with(':') && !arg.starts_with(":!") && arg != ":echo")
            {
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
    }
}

fn cell_width(text: &str) -> usize {
    tig_rs::render::cell_width(text)
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
    let start = rows
        .get(..=selected)?
        .iter()
        .rposition(|line| line == "---")?
        + 1;
    stat_header_after(rows, selected, start)
}

fn diff_reloaded_line(old: &View, rows: &[String]) -> Option<usize> {
    let (_, target) = diff_edit_target(old.source_rows(), old.source_index(old.selected))?;
    if target == 0 {
        return None;
    }
    let header = old.source_rows()[..=old.source_index(old.selected)]
        .iter()
        .rfind(|row| row.starts_with("diff "))?;
    // ponytail: ordinary hunks only; extend alongside combined-diff navigation.
    if !header.starts_with("diff --git ") {
        return None;
    }
    let start = rows.iter().position(|row| row == header)?;
    diff_line_at(rows, start, target)
}

fn diff_line_at(rows: &[String], start: usize, target: usize) -> Option<usize> {
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
    diff_hunk_line(row, '+')
}

fn diff_hunk_line(row: &str, marker: char) -> Option<usize> {
    row.split_whitespace()
        .find(|field| field.starts_with(marker))?
        .trim_start_matches(marker)
        .split(',')
        .next()?
        .parse()
        .ok()
}

fn diff_edit_target(rows: &[String], selected: usize) -> Option<(PathBuf, usize)> {
    diff_target(rows, selected, false)
}

fn diff_target(rows: &[String], selected: usize, old_side: bool) -> Option<(PathBuf, usize)> {
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
    let path = if let Some(file) = patch.iter().find_map(|line| {
        line.strip_prefix(if old_side {
            "rename from "
        } else {
            "rename to "
        })
    }) {
        git_patch_path(file)?
    } else {
        let file = patch
            .iter()
            .find_map(|line| line.strip_prefix(if old_side { "--- " } else { "+++ " }))?;
        // A literal tab separates header fields; tabs in filenames are C-quoted.
        let file = file.split('\t').next()?;
        if file == "/dev/null" {
            return None;
        }
        let path = git_patch_path(file)?;
        let old = &rows[header];
        let prefix = if old_side {
            // Require the complete pair: an unprefixed file named a/file
            // must never be mistaken for the different repository path file.
            [("a", "b"), ("i", "w")].into_iter().find_map(|(from, to)| {
                let (quote, raw) = file.strip_prefix('"').map_or(("", file), |raw| ("\"", raw));
                let suffix = raw.strip_prefix(&format!("{from}/"))?;
                (old == &format!("diff --git {file} {quote}{to}/{suffix}")).then_some(from)
            })
        } else if (old.starts_with("diff --git a/") || old.starts_with("diff --git \"a/"))
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
    let start = diff_hunk_line(&rows[hunk], if old_side { '-' } else { '+' })?;
    let preceding = rows
        .get(hunk + 1..selected)
        .unwrap_or(&[])
        .iter()
        .filter(|line| {
            !line.starts_with(if old_side { '+' } else { '-' }) && !line.starts_with('\\')
        })
        .count();
    Some((path, start + preceding))
}

fn git_patch_path(raw: &str) -> Option<PathBuf> {
    tig_rs::git::parse_git_path(raw.as_bytes()).ok()
}

#[cfg(test)]
mod editor_tests {
    use super::{
        diff_edit_target, diff_target, git_patch_path, stage_stat_header, App, Config, Item, View,
    };
    use std::path::PathBuf;

    #[test]
    fn blame_history_path_is_not_a_worktree_edit_target() {
        let raw = format!("{} 1 1\nfilename old/file\n\tcontent\n", "a".repeat(40));
        let line = tig_rs::git::parse_blame(raw.as_bytes()).unwrap().remove(0);
        let mut view = View::new("blame");
        view.path = "new/file".into();
        view.push("content".into(), Item::Blame(line.clone()));
        let mut app = App {
            watch: tig_rs::watch::Watch::default(),
            repo: None,
            config: Config::defaults(),
            view,
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
    fn deleted_diff_line_uses_old_path_and_old_hunk_count() {
        let rows: Vec<String> = [
            "diff --git a/old b/new",
            "rename from old",
            "rename to new",
            "--- a/old",
            "+++ b/new",
            "@@ -9,2 +12,3 @@",
            "+inserted",
            " context",
            "-deleted",
            "+replacement",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(
            diff_target(&rows, 8, true),
            Some((PathBuf::from("old"), 10))
        );
        assert_eq!(diff_edit_target(&rows, 9), Some((PathBuf::from("new"), 14)));
        let rows: Vec<String> = [
            "diff --git a/removed b/removed",
            "--- a/removed",
            "+++ /dev/null",
            "@@ -1 +0,0 @@",
            "-deleted",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(
            diff_target(&rows, 4, true),
            Some((PathBuf::from("removed"), 1))
        );
        assert_eq!(diff_edit_target(&rows, 4), None);
        let rows: Vec<String> = [
            "diff --git a/file a/file",
            "--- a/file",
            "+++ /dev/null",
            "@@ -1 +0,0 @@",
            "-deleted",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(
            diff_target(&rows, 4, true),
            Some((PathBuf::from("a/file"), 1))
        );
    }

    #[test]
    fn patch_header_delimiters_preserve_spaces_and_quoted_tabs() {
        for (header, before, after, path) in [
            (
                "diff --git a/space name b/space name",
                "a/space name\t",
                "b/space name\t",
                "space name",
            ),
            (
                r#"diff --git "a/tab\tname" "b/tab\tname""#,
                r#""a/tab\tname""#,
                r#""b/tab\tname""#,
                "tab\tname",
            ),
        ] {
            let rows = vec![
                header.to_owned(),
                format!("--- {before}"),
                format!("+++ {after}"),
                "@@ -1 +1 @@".into(),
                "-old".into(),
                "+new".into(),
            ];
            assert_eq!(diff_target(&rows, 4, true), Some((PathBuf::from(path), 1)));
            assert_eq!(diff_edit_target(&rows, 5), Some((PathBuf::from(path), 1)));
        }
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
            watch: tig_rs::watch::Watch::default(),
            repo: None,
            config: Config::default(),
            view,
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
        app.enter(true).unwrap();
        assert_eq!(app.view.selected, 9);
        app.view = View::text(
            "diff",
            "commit abc\n---\n a | 1 +\n b | 1 +\ndiff --git a/a b/a\ndiff --git a/b b/b\n",
        );
        app.view.selected = 3;
        app.enter(true).unwrap();
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

fn view_reference(view: &View) -> String {
    match view.items.get(view.selected) {
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
        Some(Item::Ref(_, Some(selector))) if matches!(view.name.as_str(), "stash" | "reflog") => {
            selector.clone()
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
        _ if view.name == "tree" => format!("Files in /{}", view.path.display()),
        _ if view.name == "pager" => view.command_title.clone(),
        _ if view.name == "refs" => "All references".into(),
        Some(Item::Status(e, staged)) => format!(
            "Press u to {} '{}'{}",
            if *staged { "unstage" } else { "stage" },
            e.path.display(),
            if e.index == '?' {
                " for addition"
            } else {
                " for commit"
            }
        ),
        _ if view.name == "status" => "Nothing to update".into(),
        _ if view.name == "diff"
            && diff_stat_header(view.source_rows(), view.source_index(view.selected)).is_some() =>
        {
            "Press '<Enter>' to jump to file diff".into()
        }
        _ if view.name == "diff" => {
            diff_edit_target(view.source_rows(), view.source_index(view.selected))
                .map(|(path, _)| format!("Changes to '{}'", path.display()))
                .unwrap_or_else(|| view.revision.clone())
        }
        _ if view.name == "stage"
            && !view.untracked
            && stage_stat_header(&view.rows, view.selected).is_some() =>
        {
            "Press '<Enter>' to jump to file diff".into()
        }
        _ if view.name == "stage" && view.untracked => {
            format!("Untracked file {}", view.path.display())
        }
        _ if view.name == "stage" => {
            let kind = if view.staged { "Staged" } else { "Unstaged" };
            if view.path.as_os_str().is_empty() {
                diff_edit_target(view.source_rows(), view.source_index(view.selected))
                    .map(|(path, _)| format!("{kind} changes to '{}'", path.display()))
                    .unwrap_or_else(|| format!("{kind} changes"))
            } else {
                format!("{kind} changes to '{}'", view.path.display())
            }
        }
        _ if view.name == "blob" || view.name == "blame" => view.path.display().to_string(),
        _ => String::new(),
    }
}

fn pane_screen(
    view: &mut View,
    config: &Config,
    width: usize,
    visible: usize,
    saved: bool,
) -> Vec<String> {
    view.wrap_text(config, width);
    let visible = visible.max(1);
    let old_top = view.rendered_top;
    if view.selected < view.top {
        view.top = view.selected;
    }
    if view.selected >= view.top + visible {
        view.top = view.selected + 1 - visible;
    }
    // C split_view preserves existing parent cells and redraws only selection.
    // Scrolling or reloading invalidates those retained row widths.
    for index in view.top
        ..view
            .top
            .saturating_add(visible)
            .min(view.commit_row_widths.len())
    {
        if !(old_top..old_top.saturating_add(visible)).contains(&index) {
            view.commit_row_widths[index] = width;
        }
    }
    view.rendered_top = view.top;
    if let Some(row_width) = view.commit_row_widths.get_mut(view.selected) {
        *row_width = width;
    }
    let line_numbers = pager_line_numbers(config, view);
    let separator = if !saved && config.value("line-graphics") != Some("ascii") {
        "│ "
    } else {
        tig_rs::render::line_number_separator(config)
    };
    let mut lines: Vec<String> = (0..visible)
        .map(|i| {
            let index = view.top + i;
            let row = view.rows.get(index).map(String::as_str).unwrap_or("");
            let clipped_fields = tig_rs::render::clip_commit_fields(
                row,
                &view.commit_fields,
                view.commit_row_widths
                    .get(index)
                    .copied()
                    .unwrap_or(width)
                    .saturating_add(view.left),
                config,
                saved,
            );
            let row = clipped_fields.as_str();
            let expanded;
            let row = if let Some(wrap) = &view.wrapping {
                expanded = format!(
                    "{}{}",
                    if wrap.lines.get(index).is_some_and(|line| line.1) {
                        "+"
                    } else {
                        ""
                    },
                    tig_rs::render::expand_pager_text(row, wrap.tab_size)
                );
                expanded.as_str()
            } else {
                row
            };
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
    let reference = view_reference(view);
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
                "refs" | "reflog" => "reference",
                "stash" => "stash",
                "tree" => "file",
                _ => "line",
            },
            if view.name == "tree" || view.wrapping.is_some() {
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
                _ => view.wrapping.as_ref().map_or(0, |wrap| {
                    wrap.lines.iter().filter(|line| line.1).count()
                }),
            })
        ));
    }
    let percent = if view.rows.is_empty() {
        0
    } else {
        (view.top + visible).min(view.rows.len()) * 100 / view.rows.len()
    };
    let suffix = format!(" {percent}%");
    if width < suffix.len() {
        // C cannot position the percentage when it is wider than the title window.
        lines.push(clip(&title, 0, width.saturating_sub(1)));
        return lines;
    }
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
    history: PromptHistory,
    inputrc_motion: std::collections::HashMap<char, bool>,
}
impl Terminal {
    fn open(config: &Config) -> Result<Self> {
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
        let mut t = Self {
            out,
            stop,
            signals,
            history: PromptHistory::load(config),
            inputrc_motion: inputrc_motion(),
        };
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
            if let Some(event) = self.read_tick()? {
                return Ok(event);
            }
        }
    }
    fn read_tick(&self) -> Result<Option<Event>> {
        if self.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Terminal interrupted").into());
        }
        if event::poll(std::time::Duration::from_millis(100))? {
            return Ok(Some(event::read()?));
        }
        Ok(None)
    }
    fn draw(&mut self, app: &mut App) -> Result<()> {
        let lines = app.screen(false);
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
                write!(self.out, "{text}")?;
                queue!(self.out, SetAttribute(Attribute::Reset))?;
            }
        }
        self.out.flush()?;
        Ok(())
    }
    fn find_file(&mut self, app: &mut App) -> Result<()> {
        while let Some(finder) = &app.finder {
            let visible = app.height.saturating_sub(2).max(1);
            let top = finder.selected.saturating_sub(visible - 1);
            queue!(self.out, cursor::MoveTo(0, 0), Clear(ClearType::All))?;
            for (row, &index) in finder.visible.iter().skip(top).take(visible).enumerate() {
                queue!(self.out, cursor::MoveTo(0, row as u16))?;
                if top + row == finder.selected {
                    queue!(self.out, SetAttribute(Attribute::Reverse))?;
                }
                write!(
                    self.out,
                    "{}",
                    clip(&FileFinder::label(&finder.files[index]), 0, app.width)
                )?;
                queue!(self.out, SetAttribute(Attribute::Reset))?;
            }
            queue!(
                self.out,
                cursor::MoveTo(0, app.height.saturating_sub(2) as u16),
                SetAttribute(Attribute::Reverse)
            )?;
            let title = format!(
                "[finder] file {} of {}",
                if finder.visible.is_empty() {
                    0
                } else {
                    finder.selected + 1
                },
                finder.visible.len()
            );
            write!(self.out, "{}", clip(&title, 0, app.width))?;
            queue!(
                self.out,
                SetAttribute(Attribute::Reset),
                cursor::MoveTo(0, app.height.saturating_sub(1) as u16)
            )?;
            write!(
                self.out,
                "{}",
                clip(&format!("Find file: {}", finder.query), 0, app.width)
            )?;
            self.out.flush()?;
            match self.read()? {
                Event::Resize(width, height) => {
                    app.width = width as usize;
                    app.height = height as usize;
                    if let Err(error) = app.refresh_after_command() {
                        app.message = error.to_string();
                    }
                }
                Event::Key(key) => {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        continue;
                    }
                    if let Err(error) = app.finder_key(&key_name(key.code, key.modifiers)) {
                        app.message = error.to_string();
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }
    fn prompt(&mut self, app: &mut App, prefix: &str) -> Result<Option<String>> {
        let mut value = String::new();
        let mut point = 0;
        let mut position = self.history.entries.len();
        let mut draft = String::new();
        loop {
            let line = prompt_text(&format!("{prefix}{value}"));
            let caret = UnicodeWidthStr::width(
                prompt_text(&format!("{prefix}{}", &value[..point])).as_str(),
            );
            let wanted = caret.saturating_sub(app.width.saturating_sub(1));
            let mut skip = 0;
            for grapheme in line.graphemes(true) {
                if skip >= wanted {
                    break;
                }
                skip += UnicodeWidthStr::width(grapheme);
            }
            queue!(
                self.out,
                cursor::MoveTo(0, app.height.saturating_sub(1) as u16),
                Clear(ClearType::CurrentLine)
            )?;
            write!(self.out, "{}", clip_prompt(&line, skip, app.width))?;
            queue!(
                self.out,
                cursor::MoveTo(
                    caret.saturating_sub(skip) as u16,
                    app.height.saturating_sub(1) as u16
                ),
                cursor::Show
            )?;
            self.out.flush()?;
            let event = self.read()?;
            if let Event::Key(k) = &event {
                if let KeyCode::Char(key) = k.code {
                    if key != 'c' && k.modifiers.contains(KeyModifiers::CONTROL) {
                        if let Some(&end) = self.inputrc_motion.get(&key) {
                            point = if end { value.len() } else { 0 };
                            continue;
                        }
                    }
                }
            }
            match event {
                Event::Resize(w, h) => {
                    app.width = w as usize;
                    app.height = h as usize;
                }
                Event::Key(k) => match k.code {
                    KeyCode::Enter => {
                        if self.history.limit > 0
                            && !value.is_empty()
                            && self.history.entries.last() != Some(&value)
                        {
                            self.history.entries.push(value.clone());
                            if self.history.entries.len() > self.history.limit {
                                self.history.entries.remove(0);
                            }
                        }
                        queue!(self.out, cursor::Hide)?;
                        return Ok(Some(value));
                    }
                    KeyCode::Esc => {
                        queue!(self.out, cursor::Hide)?;
                        return Ok(None);
                    }
                    KeyCode::Tab if prefix == ":" && point == value.len() => {
                        complete_prompt_action(&mut value, &mut point);
                    }
                    KeyCode::Up if position > 0 => {
                        if position == self.history.entries.len() {
                            draft = value.clone();
                        }
                        position -= 1;
                        value.clone_from(&self.history.entries[position]);
                        point = value.len();
                    }
                    KeyCode::Down if position < self.history.entries.len() => {
                        position += 1;
                        value = self
                            .history
                            .entries
                            .get(position)
                            .cloned()
                            .unwrap_or_else(|| draft.clone());
                        point = value.len();
                    }
                    KeyCode::Backspace => {
                        if let Some((previous, _)) = value[..point].grapheme_indices(true).last() {
                            value.replace_range(previous..point, "");
                            point = previous;
                        }
                    }
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        queue!(self.out, cursor::Hide)?;
                        return Ok(None);
                    }
                    KeyCode::Left => {
                        point = value[..point]
                            .grapheme_indices(true)
                            .last()
                            .map_or(0, |(i, _)| i)
                    }
                    KeyCode::Right if point < value.len() => {
                        point += value[point..].graphemes(true).next().unwrap().len()
                    }
                    KeyCode::Home => point = 0,
                    KeyCode::End => point = value.len(),
                    KeyCode::Char('a') if k.modifiers.contains(KeyModifiers::CONTROL) => point = 0,
                    KeyCode::Char('e') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        point = value.len()
                    }
                    KeyCode::Delete if point < value.len() => {
                        let end = point + value[point..].graphemes(true).next().unwrap().len();
                        value.replace_range(point..end, "");
                    }
                    KeyCode::Char(c)
                        if !k
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && !c.is_control() =>
                    {
                        value.insert(point, c);
                        point += c.len_utf8();
                        point = value
                            .grapheme_indices(true)
                            .map(|(start, grapheme)| start + grapheme.len())
                            .find(|&end| end >= point)
                            .unwrap_or(value.len());
                    }
                    _ => (),
                },
                _ => (),
            }
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.history.save();
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
fn read_command_prompts(terminal: &mut Terminal, app: &mut App, action: &str) -> Result<bool> {
    app.prompt_answers.clear();
    for label in tig_rs::commands::prompt_labels(action) {
        let prefix = if label.is_empty() {
            "Command argument: "
        } else {
            label
        };
        match terminal.prompt(app, prefix)? {
            Some(answer) => app.prompt_answers.push(answer),
            None => {
                app.prompt_answers.clear();
                return Ok(false);
            }
        }
    }
    Ok(true)
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
        KeyCode::Backspace => "<Backspace>".into(),
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
    // Git shell aliases run at the worktree root; restore the caller's cwd
    // before config, repository discovery and revision/path disambiguation.
    if let Some(prefix) = env::var_os("GIT_PREFIX").filter(|value| !value.is_empty()) {
        let prefix = PathBuf::from(prefix);
        if !prefix
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err("GIT_PREFIX must be a relative path without '..'".into());
        }
        let root = env::current_dir()?;
        let directory = root.join(prefix).canonicalize()?;
        if !directory.starts_with(&root) {
            return Err("GIT_PREFIX escapes the worktree".into());
        }
        env::set_current_dir(directory)?;
        env::set_var("GIT_WORK_TREE", root);
        env::set_var("GIT_PREFIX", "");
    }
    let mut config = Config::load();
    if matches!(cli.view.as_str(), "main" | "diff") {
        config.take_diff_options(&mut cli.git_args);
    }
    for message in &config.diagnostics {
        eprintln!("tig warning: {message}");
    }
    if cli.view == "status" && !cli.git_args.iter().any(|arg| arg == "--") {
        cli.git_args.insert(0, "--".into());
    }
    let invocation = env::current_dir()?;
    let repo = Repository::discover(&invocation).ok();
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let history = PromptHistory::load(&config);
    let search = history.entries.last().cloned().unwrap_or_default();
    let mut app = App {
        watch: tig_rs::watch::Watch::default(),
        repo,
        config,
        view: View::new(&cli.view),
        help: None,
        tree_initialized: false,
        finder: None,
        previous: vec![],
        pending_command: None,
        pending_revert: None,
        prompt_answers: vec![],
        other: None,
        split: false,
        parent_focused: false,
        revision: "HEAD".into(),
        path: PathBuf::new(),
        args: cli.git_args.clone(),
        message: String::new(),
        search,
        width: width as usize,
        height: height as usize,
    };
    if let Err(error) = app.reset_watch() {
        app.message = error.to_string();
    }
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
    // Script dimensions must be applied before Git generates width-dependent stats.
    if env::var_os("TIG_SCRIPT").is_some() {
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
    }
    if cli.view == "pager" || (cli.view == "diff" && !io::stdin().is_terminal()) {
        let forward_stdin = cli
            .git_args
            .iter()
            .take_while(|arg| !matches!(arg.as_str(), "--" | "--end-of-options"))
            .any(|arg| arg == "--stdin");
        let mut input = Vec::new();
        io::Read::read_to_end(&mut io::stdin(), &mut input)?;
        // Revision names are Git bytes; only rendered stdin text needs UTF-8.
        let text = if forward_stdin {
            ""
        } else {
            std::str::from_utf8(&input)?
        };
        if cli.view == "diff" {
            if forward_stdin {
                return Err(
                    "Forwarding revision input to git show --stdin is not supported yet".into(),
                );
            }
            app.view = View::text("diff", text);
            app.view.from_stdin = true;
            if let Some(oid) = text.lines().find_map(|line| {
                line.strip_prefix("commit ")
                    .and_then(|header| header.split_whitespace().next())
                    .filter(|oid| {
                        matches!(oid.len(), 40 | 64) && oid.bytes().all(|c| c.is_ascii_hexdigit())
                    })
            }) {
                app.view.revision = oid.into();
                app.revision = oid.into();
            }
        } else if forward_stdin
            || cli
                .git_args
                .iter()
                .take_while(|arg| !matches!(arg.as_str(), "--" | "--end-of-options"))
                .any(|arg| arg == "--pretty=raw")
        {
            let commits = if forward_stdin {
                let order = app.config.value("commit-order").unwrap_or("auto");
                app.repo()?.history_from_stdin(
                    &app.args,
                    &input,
                    if order == "auto" { "default" } else { order },
                )?
            } else {
                tig_rs::git::parse_raw_history(text)?
            };
            app.view = View::new("main");
            app.view.from_stdin = true;
            for commit in commits {
                app.view
                    .push_typed(String::new(), Item::Commit(commit), "main-commit");
            }
            app.view.redraw_stdin(&app.config, app.width)?;
        } else {
            app.view = View::text("pager", text);
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
            if let Some(rev) = app.args.first().cloned() {
                if let Some((lower, upper)) = rev.split_once("..") {
                    if upper.starts_with('.') {
                        return Err("Blame requires a two-dot revision range".into());
                    }
                    app.revision =
                        app.repo()?
                            .revision(if upper.is_empty() { "HEAD" } else { upper })?;
                    app.args = vec![format!(
                        "^{}",
                        app.repo()?
                            .revision(if lower.is_empty() { "HEAD" } else { lower })?
                    )];
                } else {
                    app.revision = rev;
                    app.args.clear();
                }
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
        app.view = app.load(&cli.view).map_err(|error| {
            if cli.view == "main"
                && error
                    .to_string()
                    .contains("unknown revision or path not in the working tree")
            {
                "No revisions match the given arguments.".into()
            } else {
                error
            }
        })?;
        if cli.view == "grep" && app.view.rows.is_empty() {
            app.message = "No matches found".into();
        }
    }
    app.view.wrap_text(&app.config, app.width);
    if cli.line > 0 {
        app.view.selected = cli.line.min(app.view.rows.len().saturating_sub(1));
    }
    app.view.restore_status_selection();
    if let Ok(script) = env::var("TIG_SCRIPT") {
        app.center_selection();
        let result = app.script(&script);
        let _ = history.save();
        return result;
    }
    app.center_selection();
    let mut terminal = Terminal::open(&app.config)?;
    let mut key_sequence = String::new();
    loop {
        if app.finder.is_some() {
            terminal.find_file(&mut app)?;
        }
        app.poll_watch();
        terminal.draw(&mut app)?;
        let event = loop {
            if let Some(event) = terminal.read_tick()? {
                break event;
            }
            if app.poll_watch() {
                terminal.draw(&mut app)?;
            }
        };
        let action = match event {
            Event::Resize(w, h) => {
                app.width = w as usize;
                app.height = h as usize;
                if let Err(error) = app.refresh_after_command() {
                    app.message = error.to_string();
                }
                continue;
            }
            Event::Mouse(m) => {
                key_sequence.clear();
                app.message.clear();
                match m.kind {
                    MouseEventKind::ScrollUp => Some("move-up".into()),
                    MouseEventKind::ScrollDown => Some("move-down".into()),
                    _ => continue,
                }
            }
            Event::Key(k) => {
                if !key_sequence.is_empty() && k.code == KeyCode::Esc {
                    key_sequence.clear();
                    app.message.clear();
                    continue;
                }
                key_sequence.push_str(&key_name(k.code, k.modifiers));
                if app
                    .config
                    .key_sequence_pending(&app.view.name, &key_sequence)
                {
                    app.message = format!("Keys: {key_sequence}");
                    continue;
                }
                app.binding(&key_sequence)
            }
            _ => continue,
        };
        key_sequence.clear();
        let Some(action) = action else {
            app.message = "Unknown key, press h for help".into();
            continue;
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
                    if !read_command_prompts(&mut terminal, &mut app, &s)? {
                        continue;
                    }
                    app.action(&format!(":{s}"))
                };
                match result {
                    Ok(false) => break,
                    Ok(true) => (),
                    Err(e) => app.message = e.to_string(),
                }
            }
        } else {
            if !read_command_prompts(&mut terminal, &mut app, &action)? {
                continue;
            }
            match app.action(&action) {
                Ok(false) => break,
                Ok(true) => (),
                Err(e) => app.message = e.to_string(),
            }
        }
        if let Some(plan) = &app.pending_revert {
            let prompt = plan.prompt();
            let answer = terminal.prompt(&mut app, &prompt)?;
            let confirmed =
                answer.is_some_and(|answer| matches!(answer.as_str(), "y" | "Y" | "yes"));
            if let Err(error) = app.finish_revert(confirmed) {
                app.message = error.to_string();
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
                terminal = Terminal::open(&app.config)?;
                result
            };
            match result {
                Ok(output) => {
                    if command.exit {
                        break;
                    }
                    if let Err(error) = app.refresh_after_command() {
                        app.message = error.to_string();
                    } else if command.echo {
                        app.message = String::from_utf8_lossy(&output.stdout)
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_owned();
                    }
                }
                Err(error) => {
                    let _ = app.refresh_after_command();
                    app.message = error.to_string();
                }
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
    fn failed_highlighter_keeps_the_original_diff() {
        let mut config = Config::defaults();
        config.parse("set diff-highlight = program-that-does-not-exist");
        assert_eq!(highlight_diff(&config, b"diff content\n"), "diff content\n");
        config.parse("set diff-highlight = wc\nset diff-options = --word-diff=plain");
        assert_eq!(highlight_diff(&config, b"diff content\n"), "diff content\n");
        for option in [
            "--",
            "--output=overwritten",
            "-ooverwritten",
            "--ext-diff",
            "--textconv",
        ] {
            config
                .settings
                .insert("diff-options".into(), vec![option.into()]);
            assert!(diff_options(&config).is_err(), "{option}");
        }
    }

    #[test]
    fn main_graph_uses_history_traversal_options() {
        let root = env::temp_dir().join(format!("tig-main-graph-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        assert!(Command::new("tar")
            .args([
                "-xzf",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/test/files/scala-js-benchmarks.tgz"
                ),
                "-C"
            ])
            .arg(&root)
            .status()
            .unwrap()
            .success());
        let repo = Repository::discover(&root).unwrap();
        repo.command(["reset", "--hard"]).unwrap();
        let mut config = Config::defaults();
        config.parse("set line-graphics = utf-8\nset show-changes = no\nset main-view = commit-title:yes,graph,refs=no");
        let mut app = App {
            watch: tig_rs::watch::Watch::default(),
            repo: Some(repo),
            config,
            view: View::new("main"),
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
            other: None,
            split: false,
            parent_focused: false,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            args: vec!["--first-parent".into()],
            message: String::new(),
            search: String::new(),
            width: 100,
            height: 20,
        };
        for renderer in ["v1", "v2"] {
            app.config
                .apply_command(&format!("set main-view-commit-title-graph = {renderer}"))
                .unwrap();
            let view = app.load("main").unwrap();
            assert_eq!(view.rows[5], "∙ Merge pull request #4 from phaller/patch-1");
            assert!(view
                .items
                .iter()
                .all(|item| matches!(item, Item::Commit(c) if c.parents.len() <= 1)));
        }
        app.args = vec!["--follow".into(), "project/Build.scala".into()];
        let implicit_path = app.load("main").unwrap();
        assert_eq!(implicit_path.rows.len(), 8);
        assert_eq!(
            implicit_path.rows[0],
            "WIP: Upgrade to 0.4-SNAPSHOT and DCE"
        );
        app.args.insert(1, "--".into());
        assert_eq!(app.load("main").unwrap().rows, implicit_path.rows);
        app.args = vec!["--no-merges".into()];
        assert_eq!(app.load("main").unwrap().rows[0], implicit_path.rows[0]);
        app.args.clear();
        assert!(app.load("main").unwrap().rows[5].starts_with("●"));
        app.view = app.load("main").unwrap();
        app.height = 5;
        app.action("goto 6").unwrap();
        app.action("scroll-right").unwrap();
        let position = (app.view.selected, app.view.top, app.view.left);
        for line in ["49", "50", "999999999999999999999999999999999999"] {
            assert!(app.action(line).unwrap());
            assert_eq!((app.view.selected, app.view.top, app.view.left), position);
        }
        let Item::Commit(commit) = app.selected() else {
            panic!("expected commit")
        };
        let parent = app
            .repo()
            .unwrap()
            .revision(&format!("{}^", commit.oid))
            .unwrap();
        app.action("parent").unwrap();
        assert!(matches!(app.selected(), Item::Commit(c) if c.oid == parent));
        app.action("refresh").unwrap();
        assert!(app.action("back").unwrap());
        assert_eq!((app.view.selected, app.view.top, app.view.left), position);
        app.other = Some(View::text("diff", "still open"));
        app.split = true;
        app.parent_focused = true;
        assert!(app.action("back").unwrap());
        assert_eq!(app.view.name, "main");
        assert!(app.split && app.other.is_some());
        app.action("parent").unwrap();
        assert!(app.action(&app.binding("<").unwrap()).unwrap());
        assert_eq!((app.view.selected, app.view.top, app.view.left), position);
        app.other = None;
        app.split = false;
        app.parent_focused = false;
        app.action("goto 48").unwrap();
        let root_position = (app.view.selected, app.view.top, app.view.left);
        app.action("parent").unwrap();
        app.action("parent").unwrap();
        assert_eq!(
            (app.view.selected, app.view.top, app.view.left),
            root_position
        );
        assert!(app.action("back").unwrap());
        assert!(app.action("back").unwrap());
        assert_eq!(app.message, "Already at start of history");
        app.view
            .items
            .insert(0, Item::Changes(ChangeKind::Unstaged));
        app.view.rows.insert(0, "Unstaged changes".into());
        app.view.selected = 0;
        app.action("parent").unwrap();
        assert_eq!(app.view.selected, 1);
        app.action("back").unwrap();
        assert_eq!(app.view.selected, 0);
        app.action("refresh").unwrap();
        app.repo()
            .unwrap()
            .command(["tag", "navigation-ancestor", "HEAD^"])
            .unwrap();
        app.revision = app.repo().unwrap().revision("HEAD").unwrap();
        let described = app
            .repo()
            .unwrap()
            .command(["describe", "--tags", &app.revision])
            .unwrap();
        let diff = app.load("diff").unwrap();
        assert!(diff.rows[1].ends_with(String::from_utf8_lossy(&described).trim()));
        let before_command = (app.view.selected, app.view.top, app.view.left);
        app.action(":!git tag main-navigation").unwrap();
        assert_eq!(app.view.name, "pager");
        assert!(app.view.rows.is_empty());
        assert!(app.action("view-close").unwrap());
        assert_eq!(
            (app.view.selected, app.view.top, app.view.left),
            before_command
        );
        assert!(
            matches!(&app.view.items[0], Item::Commit(c) if c.decorations.contains("main-navigation"))
        );
        app.action(":!echo 'two words; literal'").unwrap();
        assert_eq!(app.view.rows, ["two words; literal"]);
        app.action("view-close").unwrap();
        app.action(":!sh -c 'echo command-error >&2; exit 2'")
            .unwrap();
        assert_eq!(app.view.rows, ["command-error"]);
        app.action("view-close").unwrap();
        app.revision = app.repo().unwrap().revision("HEAD").unwrap();
        let diff = app.load("diff").unwrap();
        assert!(diff.rows[1].contains("<main-navigation>"));
        assert!(!diff.rows[1].contains("navigation-ancestor"));
        for focus_parent in [false, true] {
            app.enter(true).unwrap();
            if focus_parent {
                app.action("view-next").unwrap();
            }
            let tag = if focus_parent {
                "command-parent"
            } else {
                "command-child"
            };
            app.action(&format!("exec @git tag {tag} %(commit)"))
                .unwrap();
            app.pending_command
                .take()
                .unwrap()
                .run(app.repo().unwrap(), true, true)
                .unwrap();
            app.refresh_after_command().unwrap();
            let diff = if focus_parent {
                app.other.as_ref().unwrap()
            } else {
                &app.view
            };
            assert!(diff.rows[1].contains(tag));
            app.action(":!echo split-pager").unwrap();
            assert!(!app.split && app.other.is_none());
            assert_eq!(app.view.rows, ["split-pager"]);
            app.action("view-close").unwrap();
            assert_eq!(app.view.name, "main");
            assert!(app.previous.is_empty());
            assert!(!app.action("view-close").unwrap());
        }
        app.action("view-diff").unwrap();
        app.action(":!git tag navigation-fullscreen").unwrap();
        app.action("view-close").unwrap();
        app.action("view-close").unwrap();
        assert_eq!(app.view.name, "main");
        assert!(
            matches!(&app.view.items[0], Item::Commit(c) if c.decorations.contains("navigation-fullscreen"))
        );
        fs::write(root.join(&app.revision), "revision-shaped filename").unwrap();
        assert!(app.load("diff").is_ok());
        fs::write(
            root.join("Build.scala"),
            "different file at repository root",
        )
        .unwrap();
        app.repo().unwrap().command(["add", "Build.scala"]).unwrap();
        app.repo()
            .unwrap()
            .command([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "unrelated root file",
            ])
            .unwrap();
        app.repo = Some(Repository::discover(root.join("project")).unwrap());
        app.args = vec!["--follow".into(), "Build.scala".into()];
        assert_eq!(app.load("main").unwrap().rows, implicit_path.rows);
        app.args = vec!["--follow".into(), "--".into(), "project/Build.scala".into()];
        assert_eq!(app.load("main").unwrap().rows, implicit_path.rows);
        let expected = app
            .repo()
            .unwrap()
            .history(
                &["HEAD".into(), "--".into(), "project/Build.scala".into()],
                0,
            )
            .unwrap();
        assert_eq!(
            app.repo()
                .unwrap()
                .history(&["HEAD".into(), "Build.scala".into()], 0)
                .unwrap(),
            expected
        );
        app.repo()
            .unwrap()
            .command(["branch", "Build.scala"])
            .unwrap();
        assert!(app
            .repo()
            .unwrap()
            .history(&["Build.scala".into()], 0)
            .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn first_tree_open_uses_invocation_directory_once() {
        let root = env::temp_dir().join(format!("tig-tree-start-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        assert!(Command::new("tar")
            .args([
                "-xzf",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/test/files/scala-js-benchmarks.tgz"
                ),
                "-C"
            ])
            .arg(&root)
            .status()
            .unwrap()
            .success());
        Repository::discover(&root)
            .unwrap()
            .command(["reset", "--hard"])
            .unwrap();
        let repo = Repository::discover(root.join("common/src")).unwrap();
        let mut app = App {
            watch: tig_rs::watch::Watch::default(),
            repo: Some(repo),
            config: Config::defaults(),
            view: View::new("main"),
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
        app.revision = "missing-revision".into();
        assert!(app.action("view-tree").is_err());
        assert!(!app.tree_initialized);
        assert!(app.path.as_os_str().is_empty());
        app.revision = "HEAD".into();
        app.action("view-tree").unwrap();
        assert_eq!(app.view.path, PathBuf::from("common/src"));
        assert_eq!(app.view.rows[0], "Directory path /common/src/");
        assert_eq!(app.edit_target(), None); // Parent entry is never editable.
        app.action("parent").unwrap();
        assert_eq!(app.view.path, PathBuf::from("common"));
        app.action("parent").unwrap();
        assert!(app.view.path.as_os_str().is_empty());
        app.view.selected = app
            .view
            .items
            .iter()
            .position(|item| matches!(item, Item::Tree(e) if e.path == PathBuf::from("README.md")))
            .unwrap();
        assert_eq!(app.edit_target(), Some((PathBuf::from("README.md"), 0)));
        app.edit().unwrap();
        assert_eq!(
            app.pending_command.as_ref().unwrap().argv.last().unwrap(),
            "README.md"
        );
        // Closing and reopening must not reapply the startup prefix.
        app.view = View::new("main");
        app.previous.clear();
        app.path.clear();
        app.action("view-tree").unwrap();
        assert!(app.view.path.as_os_str().is_empty());
        // Display escaping must never change the selected Git/editor path.
        fs::create_dir(root.join("-- foo bar")).unwrap();
        let file = PathBuf::from("-- foo bar/as测试asd");
        fs::write(root.join(&file), "unicode blob\n").unwrap();
        let repo = app.repo().unwrap();
        repo.command(["add", "--", "-- foo bar"]).unwrap();
        repo.command([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "Unicode filename",
        ])
        .unwrap();
        app.path = "-- foo bar".into();
        app.open("tree", app.width).unwrap();
        app.view.selected = 2;
        assert!(app.view.rows[2].ends_with("as测试asd"));
        assert_eq!(app.edit_target(), Some((file.clone(), 0)));
        app.edit().unwrap();
        assert_eq!(
            app.pending_command.as_ref().unwrap().argv.last().unwrap(),
            &PathBuf::from(".").join(&file).into_os_string()
        );
        app.enter(true).unwrap();
        assert_eq!(app.view.path, file);
        assert_eq!(app.view.rows, ["unicode blob"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unclosed_binding_argument_cannot_become_a_valid_toggle() {
        let mut app = App {
            watch: tig_rs::watch::Watch::default(),
            repo: None,
            config: Config::defaults(),
            view: View::new("main"),
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
            let command = app.binding("a").unwrap();
            assert!(app.action(&command).is_err());
            assert_eq!(app.config.settings, before);
            assert_eq!(
                app.config.action("main", "a").unwrap()[1],
                format!("{quote}author")
            );
        }
        app.action("exec").unwrap();
        assert_eq!(app.message, "Failed to execute command: No arguments");
        app.action("exec ").unwrap();
        assert_eq!(app.message, "Failed to execute command: No arguments");
        app.action("exec !").unwrap();
        assert_eq!(app.message, "Failed to format arguments");
        assert!(app.pending_command.is_none());
    }

    #[test]
    fn failed_grep_query_keeps_previous_arguments() {
        let mut app = App {
            watch: tig_rs::watch::Watch::default(),
            repo: None,
            config: Config::defaults(),
            view: View::new("grep"),
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
        let hits = repo
            .grep(&["-i".into(), "needle".into(), "HEAD:sub".into()])
            .unwrap();
        let hits: Vec<_> = hits.into_iter().flatten().collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, PathBuf::from("file.txt"));
        assert_eq!(hits[0].revision.as_deref(), Some("HEAD:sub"));
        let blob = repo.grep_blob(&hits[0]).unwrap();
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
                cached: false,
                line: 1,
                text: "hit".into(),
            }),
        );
        let mut app = App {
            watch: tig_rs::watch::Watch::default(),
            repo: None,
            config: Config::defaults(),
            view,
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
            watch: tig_rs::watch::Watch::default(),
            repo: Some(repo),
            config: Config::defaults(),
            view: View::new("main"),
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
        app.action("parent").unwrap();
        app.action("0").unwrap();
        app.view.left = 8;
        app.enter(true).unwrap();
        assert_eq!(app.view.name, "status");
        app.view.selected = 2;
        app.action("status-update").unwrap();
        assert_eq!(app.view.name, "main");
        assert!(matches!(app.selected(), Item::Changes(ChangeKind::Staged)));
        assert_eq!(app.view.history.len(), 1);
        assert_eq!(app.view.left, 8);
        app.action("back").unwrap();
        assert!(app.view.history.is_empty());
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
        app.action("view-status").unwrap();
        app.view.selected = app.view.items.iter().position(|item|
            matches!(item, Item::Status(entry, false) if entry.path == PathBuf::from("tracked"))).unwrap();
        app.enter(true).unwrap();
        app.action("maximize").unwrap();
        app.view.selected = app
            .view
            .rows
            .iter()
            .position(|row| row == "+working")
            .unwrap();
        app.action("stage-update-line").unwrap();
        app.view.selected = app.view.rows.iter().position(|row| row == "-base").unwrap();
        app.action("stage-update-line").unwrap();
        assert_eq!(app.view.name, "status");
        assert!(app.view.items.iter().any(|item|
            matches!(item, Item::Status(entry, true) if entry.path == PathBuf::from("tracked"))));
        assert!(!app.view.items.iter().any(|item|
            matches!(item, Item::Status(entry, false) if entry.path == PathBuf::from("tracked"))));
        assert_eq!(
            app.repo().unwrap().command(["show", ":tracked"]).unwrap(),
            b"working\n"
        );
        for split in [true, false] {
            fs::write(root.join("advance-a"), "first\n").unwrap();
            fs::write(root.join("advance-b"), "second\n").unwrap();
            app.view = app.status_view(false).unwrap();
            app.view.selected = app.view.items.iter().position(|item|
                matches!(item, Item::Status(entry, false) if entry.path == PathBuf::from("advance-a"))).unwrap();
            app.enter(true).unwrap();
            if !split {
                app.action("maximize").unwrap();
            }
            app.action("status-update").unwrap();
            assert_eq!(app.view.name, "stage");
            assert_eq!(app.view.path, PathBuf::from("advance-b"));
            assert_eq!(
                app.split, split,
                "auto-advance must preserve the pane layout"
            );
            assert!(!app.parent_focused);
            assert_eq!(
                app.repo().unwrap().command(["show", ":advance-a"]).unwrap(),
                b"first\n"
            );
            assert!(app.repo().unwrap().command(["show", ":advance-b"]).is_err());
            app.action("view-close").unwrap();
            app.repo()
                .unwrap()
                .command(["reset", "--", "advance-a"])
                .unwrap();
            fs::remove_file(root.join("advance-a")).unwrap();
            fs::remove_file(root.join("advance-b")).unwrap();
        }
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
            watch: tig_rs::watch::Watch::default(),
            repo: None,
            config: Config::default(),
            view: child,
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
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
        let screen = app.screen(false);
        assert_eq!(screen.len(), 16);
        assert!(screen[4].starts_with("[pager] - line 5 of 6"));
        assert!(screen[4].ends_with("83%"));
        assert!(screen[14].ends_with("100%"));
        app.action("view-next").unwrap();
        assert!(app.parent_focused);
        assert_eq!(app.path, PathBuf::from("parent.txt"));
        app.action("view-close-no-quit").unwrap();
        assert_eq!(app.message, "Can't close last remaining view");
        assert!(app.split && app.other.is_some() && app.parent_focused);
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
        app.action("view-close-no-quit").unwrap();
        assert_eq!(app.message, "Can't close last remaining view");
        assert_eq!(app.view.revision, "parent");
        app.view.selected = 5;
        let lines = pane_screen(&mut app.view, &app.config, 30, 2, false);
        assert!(lines[2].ends_with("100%"));
        assert_eq!(cell_width(&lines[2]), 30);
        for (option, expected) in [("50%", 40), ("12.5%", 9), ("3.5", 3), ("1e1", 1), ("0", 1)] {
            app.config
                .settings
                .insert("horizontal-scroll".into(), vec![option.into()]);
            app.view.left = 0;
            app.action("scroll-right").unwrap();
            assert_eq!(app.view.left, expected, "{option}");
            app.action("scroll-left").unwrap();
            assert_eq!(app.view.left, 0);
        }
        app.other = Some(View::text("pager", "child"));
        app.split = true;
        app.parent_focused = true;
        app.previous.push(View::text("pager", "older"));
        app.action("view-close-no-quit").unwrap();
        assert_eq!(app.view.rows, vec!["older"]);
        assert!(app.other.is_none() && !app.split);
        app.other = Some(View::text("pager", "child"));
        app.split = true;
        app.parent_focused = true;
        assert!(app.action("back").unwrap());
        assert_eq!(app.view.rows, vec!["older"]);
    }
    #[test]
    fn explicit_diff_detaches_parent_and_vertical_split_reserves_separator() {
        let mut app = App {
            watch: tig_rs::watch::Watch::default(),
            repo: None,
            config: Config::defaults(),
            view: View::text("diff", "first\nsecond"),
            help: None,
            tree_initialized: false,
            finder: None,
            previous: vec![],
            pending_command: None,
            pending_revert: None,
            prompt_answers: vec![],
            other: Some(View::text("main", "parent\nother commit")),
            split: true,
            parent_focused: false,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            args: vec![],
            message: String::new(),
            search: String::new(),
            width: 181,
            height: 30,
        };
        assert_eq!(app.pane_sizes(), (true, 91, 89));
        for maximized in [false, true] {
            app.split = !maximized;
            app.other = Some(View::text("main", "parent\nother commit"));
            app.view.selected = 0;
            app.action("view-diff").unwrap();
            assert!(!app.split);
            assert!(app.other.is_none());
            app.action("next").unwrap();
            assert_eq!(app.view.rows, ["first", "second"]);
            assert_eq!(app.view.selected, 1);
        }
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
        let lines = pane_screen(&mut view, &config, 90, 6, false);
        assert_eq!(lines[0], "  1| commit abc");
        assert_eq!(lines[1], "   | ---");
        assert_eq!(lines[4], "  5| +++ b/file");
        assert!(lines[6].starts_with("[diff] Changes to 'file' - line 6 of 6"));
        assert_eq!(view.rows, original);
        assert_eq!(pane_screen(&mut view, &config, 90, 8, false)[6], "");
        view.left = 5;
        assert_eq!(
            pane_screen(&mut view, &config, 90, 6, false)[0],
            "commit abc"
        );
        view.left = 0;
        assert_eq!(
            diff_edit_target(&view.rows, 5),
            Some((PathBuf::from("file"), 0))
        );
        view.selected = 2;
        assert!(pane_screen(&mut view, &config, 90, 6, false)[6]
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
    #[test]
    fn wrapped_pager_rows_preserve_source_and_navigation() {
        use tig_rs::render::{expand_pager_text, wrap_line};
        assert_eq!(
            wrap_line("abcdefghijk", 5, 8, false),
            ["abcde", "fghij", "k"]
        );
        assert_eq!(
            wrap_line("abcdefghijk", 5, 8, true),
            ["abcde", "fghi", "jk"]
        );
        assert_eq!(wrap_line("abcdé界z", 5, 8, true), ["abcdé", "界z"]);
        assert_eq!(wrap_line("界x", 1, 8, true), ["界", "x"]);
        assert_eq!(
            wrap_line("a\u{301}bcdef", 2, 8, true).concat(),
            "a\u{301}bcdef"
        );
        assert_eq!(wrap_line("\tX", 1, 8, true), ["\t", "X"]);
        assert_eq!(wrap_line("", 0, 0, true), [""]);
        assert_eq!(expand_pager_text("é\tx", 4), "é  x");

        let mut config = Config::default();
        config.parse("set wrap-lines = true");
        let text = "commit 0123456789\n\n    long commit title\n---\n very-long-path | 1 +\n 1 file changed, 1 insertion(+)\n\ndiff --git a/very-long-path b/very-long-path\n--- a/very-long-path\n+++ b/very-long-path\n@@ -0,0 +1 @@\n+abcdefghijklmno";
        let mut view = View::text("diff", text);
        view.wrap_text(&config, 10);
        assert_eq!(view.rows[0], "commit 012");
        assert!(!view.wrapping.as_ref().unwrap().lines[1].1);
        let title = view.display_index(2);
        assert!(view.wrapping.as_ref().unwrap().lines[title + 1].1);
        view.selected = view.display_index(11) + 1;
        view.move_by(-1);
        assert_eq!(view.source_index(view.selected), 11);
        assert_eq!(
            diff_edit_target(view.source_rows(), view.source_index(view.selected)),
            Some((PathBuf::from("very-long-path"), 1))
        );
        assert_eq!(diff_stat_header(view.source_rows(), 4), Some(7));
        view.wrap_text(&config, 15);
        assert_eq!(view.source_index(view.selected), 11);
        assert_eq!(view.source_rows().join("\n"), text);
        config.parse("set wrap-lines = false");
        view.wrap_text(&config, 15);
        assert_eq!(view.rows.join("\n"), text);
        assert_eq!(view.selected, 11);
        assert!(view.wrapping.is_none());
    }
}
