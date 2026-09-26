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
use std::{
    env, fs,
    io::{self, IsTerminal, Write},
    path::PathBuf,
};
use tig_rs::{
    config::{Cli, Config},
    git::Repository,
    model::{Commit, StatusEntry, TreeEntry},
};
use unicode_width::UnicodeWidthChar;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const HELP: &str = "Tig Rust migration (compatibility work in progress)\n\nUsage: tig [-C path] [log|show|reflog|blame|grep|refs|stash|status] [arguments]\n       git show | tig\n\nKeys: j/k move, Enter open, q back/quit, Q quit, / search, n next match\n      m history, d diff, s status, t tree, r refs, b blame, h help, R refresh\n      u stage/unstage selected file in status; horizontal arrows scroll\n\nThis version is not yet a drop-in replacement for upstream Tig. See MIGRATION.md.";

#[derive(Clone)]
enum Item {
    Commit(Commit),
    Status(StatusEntry, bool),
    Tree(TreeEntry),
    Ref(String, Option<String>),
    Text,
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
        view.revision = self.revision.clone();
        view.path = self.path.clone();
        view.staged = self.view.staged && name == "stage";
        view.untracked = self.view.untracked && name == "stage";
        Ok(view)
    }
    fn load_content(&self, name: &str) -> Result<View> {
        let mut v = View::new(name);
        if name == "help" {
            return Ok(View::text(name, HELP));
        }
        let repo = self.repo()?;
        match name {
            "main" => {
                let commits = repo.history(&self.args, 0)?;
                let rows = tig_rs::render::render_commits(&self.config, &commits, self.width)?;
                for (row, commit) in rows.into_iter().zip(commits) {
                    v.push(row, Item::Commit(commit));
                }
            }
            "status" => {
                let entries = repo.status()?;
                v.push(repo.status_header()?, Item::Text);
                for (group, title) in [
                    (0, "Changes to be committed:"),
                    (1, "Changes not staged for commit:"),
                    (2, "Untracked files:"),
                ] {
                    v.push(title.into(), Item::Text);
                    let start = v.rows.len();
                    for e in &entries {
                        let visible = match group {
                            0 => e.staged(),
                            1 => e.index != '?' && e.worktree != ' ' && e.worktree != '!',
                            _ => e.index == '?',
                        };
                        if visible {
                            v.push(
                                format!(
                                    "{} {}",
                                    if group == 0 { e.index } else { e.worktree },
                                    e.path.display()
                                ),
                                Item::Status(e.clone(), group == 0),
                            );
                        }
                    }
                    if v.rows.len() == start {
                        v.push("  (no files)".into(), Item::Text);
                    }
                }
            }

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
                for b in repo.blame(Some(&self.revision), &self.path)? {
                    v.push(
                        format!(
                            "{} {:<18} {:>5} {}",
                            b.oid.get(..8).unwrap_or(&b.oid),
                            b.author,
                            b.line,
                            b.text
                        ),
                        Item::Ref(b.oid, None),
                    );
                }
            }
            "diff" => {
                let oid = repo.revision(&self.revision)?;
                let mut view = View::text(name, &repo.show(&oid)?);
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
                let raw = repo.diff_bytes(
                    self.view.staged,
                    if self.path.as_os_str().is_empty() {
                        None
                    } else {
                        Some(&self.path)
                    },
                )?;
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
            "reflog" | "stash" | "grep" => {
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
        let next = self.load(name)?;
        self.previous.push(std::mem::replace(&mut self.view, next));
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
            Item::Ref(id, _) => self.revision = id,
            Item::Tree(e) => self.path = e.path,
            Item::Status(e, _) => self.path = e.path,
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
        let parent = self.view.clone();
        let depth = self.previous.len();
        match self.selected() {
            Item::Commit(c) => {
                self.revision = c.oid;
                self.open("diff")?;
            }
            Item::Ref(id, _) => {
                self.revision = id.clone();
                if self.view.name == "refs" {
                    self.args = vec![id];
                    self.open("main")?;
                } else {
                    self.open("diff")?;
                }
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
            Item::Text => (),
        }
        if self.previous.len() > depth {
            self.previous.truncate(depth);
            self.other = Some(parent);
            self.split = true;
            self.parent_focused = false;
        }
        Ok(())
    }
    fn find(&mut self, backwards: bool) {
        let count = self.view.rows.len();
        if count == 0 || self.search.is_empty() {
            return;
        }
        for offset in 1..=count {
            let i = if backwards {
                (self.view.selected + count - offset) % count
            } else {
                (self.view.selected + offset) % count
            };
            if self.view.rows[i].contains(&self.search) {
                self.view.selected = i;
                return;
            }
        }
        self.message = format!("No match: {}", self.search);
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
            self.pending_command = Some(tig_rs::commands::prepare(
                self.repo()?,
                command,
                &self.revision,
                &self.path,
                selected_ref.as_deref(),
            )?);
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
            "find-next" => self.find(false),
            "find-prev" => self.find(true),
            "refresh" => {
                self.sync_context();
                let old = self.view.clone();
                if old.name != "pager" {
                    self.view = self.load(&old.name)?;
                }
                self.view.selected = old.selected.min(self.view.rows.len().saturating_sub(1));
                self.view.top = old.top;
                self.view.left = old.left;
                self.view.restore_status_selection();
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
                        if line.starts_with(b"diff --git ") {
                            break;
                        }
                        offset += line.len();
                    }
                    if offset == raw.len() {
                        return Err("No text patch selected".into());
                    }
                    let prefix_rows = raw[..offset].iter().filter(|byte| **byte == b'\n').count();
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
                return self.action("refresh");
            }
            "status-update" if self.view.name == "status" => {
                if let Item::Status(e, staged) = self.selected() {
                    if staged {
                        self.repo()?.unstage(&e)?;
                    } else {
                        self.repo()?.stage(&e)?;
                    }
                    return self.action("refresh");
                }
            }
            "show-version" => self.message = format!("tig-rs {}", env!("CARGO_PKG_VERSION")),
            "parent" if self.view.name == "tree" => self.tree_parent()?,
            "screen-redraw" => (),
            _ if action.starts_with("view-") => {
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
                let left = pane_screen(parent, parent_size, self.height.saturating_sub(2));
                let right = pane_screen(child, child_size, self.height.saturating_sub(2));
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
                let mut lines = pane_screen(parent, self.width, parent_size.saturating_sub(1));
                lines.extend(pane_screen(child, self.width, child_size.saturating_sub(1)));
                lines
            }
        } else {
            pane_screen(&mut self.view, self.width, self.height.saturating_sub(2))
        };
        lines.push(clip(&self.message, 0, self.width));
        lines
    }
    fn script(&mut self, path: &str) -> Result<()> {
        for raw in fs::read_to_string(path)?.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(path) = line.strip_prefix(":save-display ") {
                let mut screen = self.screen();
                screen.pop();
                fs::write(path, format!("{}\n", screen.join("\n")))?;
            } else if let Some(pattern) = line.strip_prefix('/') {
                self.search = pattern.into();
                self.find(false);
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
                    command.run(self.repo()?, false, true)?;
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

fn pane_screen(view: &mut View, width: usize, visible: usize) -> Vec<String> {
    let visible = visible.max(1);
    if view.selected < view.top {
        view.top = view.selected;
    }
    if view.selected >= view.top + visible {
        view.top = view.selected + 1 - visible;
    }
    let mut lines: Vec<String> = (0..visible)
        .map(|i| {
            clip(
                view.rows
                    .get(view.top + i)
                    .map(String::as_str)
                    .unwrap_or(""),
                view.left,
                width,
            )
        })
        .collect();
    let reference = match view.items.get(view.selected) {
        Some(Item::Commit(c)) => c.oid.clone(),
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
        _ if view.name == "diff" => view.revision.clone(),
        _ if view.name == "stage" => format!(
            "{} changes to '{}'",
            if view.staged { "Staged" } else { "Unstaged" },
            view.path.display()
        ),
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
            } else {
                view.selected + usize::from(view.name != "refs")
            },
            view.rows.len().saturating_sub(match view.name.as_str() {
                "refs" => 1,
                "tree" => 1 + usize::from(!view.path.as_os_str().is_empty()),
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
    let cli = Cli::parse(&args, !io::stdin().is_terminal())?;
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
    let config = Config::load();
    for message in &config.diagnostics {
        eprintln!("tig: {message}");
    }
    let invocation = env::current_dir()?;
    let repo = Repository::discover(&invocation).ok();
    let (width, height) = terminal::size().unwrap_or((80, 24));
    let mut app = App {
        repo,
        config,
        view: View::new(&cli.view),
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
        app.view = View::text("pager", &text);
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
            if let Some(rev) = app.args.first() {
                app.revision = rev.clone();
            }
        }
        app.view = app.load(&cli.view)?;
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
                app.search = s;
                app.find(action == "search-back");
            }
        } else if action == "prompt" {
            if let Some(s) = terminal.prompt(&mut app, ":")? {
                match app.action(&s) {
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
                command.run(app.repo()?, true, true)
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
        let lines = pane_screen(&mut app.view, 30, 2);
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
}
