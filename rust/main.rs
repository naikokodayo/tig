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
    Ref(String),
    Text,
}
#[derive(Clone)]
struct View {
    name: String,
    rows: Vec<String>,
    items: Vec<Item>,
    selected: usize,
    top: usize,
    left: usize,
    revision: String,
    path: PathBuf,
    staged: bool,
    untracked: bool,
    raw_patch: Vec<u8>,
}
impl View {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            rows: vec![],
            items: vec![],
            selected: 0,
            top: 0,
            left: 0,
            revision: "HEAD".into(),
            path: PathBuf::new(),
            staged: false,
            untracked: false,
            raw_patch: Vec::new(),
        }
    }
    fn push(&mut self, text: String, item: Item) {
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
                for (staged, title) in [
                    (true, "Changes to be committed:"),
                    (false, "Changes not staged for commit:"),
                ] {
                    v.push(title.into(), Item::Text);
                    for e in &entries {
                        let visible = if staged {
                            e.staged()
                        } else {
                            e.worktree != ' ' && e.worktree != '!'
                        };
                        if visible {
                            v.push(
                                format!(
                                    "  {} {}",
                                    if staged { e.index } else { e.worktree },
                                    e.path.display()
                                ),
                                Item::Status(e.clone(), staged),
                            );
                        }
                    }
                }
            }
            "tree" => {
                for e in repo.tree(&self.revision, &self.path)? {
                    v.push(
                        format!("{} {} {}", e.mode, e.kind, e.path.display()),
                        Item::Tree(e),
                    );
                }
            }
            "refs" => {
                for r in repo.refs()? {
                    v.push(
                        format!(
                            "{} {} {}",
                            if r.current { "*" } else { " " },
                            r.oid.get(..8).unwrap_or(&r.oid),
                            r.name
                        ),
                        Item::Ref(r.oid),
                    );
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
                        Item::Ref(b.oid),
                    );
                }
            }
            "diff" => return Ok(View::text(name, &repo.show(&self.revision)?)),
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
            "log" | "reflog" | "stash" | "grep" => {
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
            Item::Ref(id) => self.revision = id,
            Item::Tree(e) => self.path = e.path,
            Item::Status(e, _) => self.path = e.path,
            Item::Text => (),
        }
    }
    fn enter(&mut self) -> Result<()> {
        match self.selected() {
            Item::Commit(c) => {
                self.revision = c.oid;
                self.open("diff")?;
            }
            Item::Ref(id) => {
                self.revision = id;
                self.open("diff")?;
            }
            Item::Tree(e) => {
                self.path = e.path.clone();
                if e.kind == "tree" {
                    self.open("tree")?;
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
        let index = self
            .view
            .items
            .iter()
            .position(|item| match item {
                Item::Commit(commit) => commit.oid.starts_with(target),
                _ => false,
            })
            .ok_or("Commit not in this view")?;
        self.view.selected = index;
        Ok(())
    }
    fn action(&mut self, action: &str) -> Result<bool> {
        let action = action.strip_prefix(':').unwrap_or(action);
        if action.starts_with("set ")
            || action.starts_with("bind ")
            || action.starts_with("color ")
            || action.starts_with("source ")
        {
            let mut config = self.config.clone();
            let errors = config.diagnostics.len();
            config.parse(action);
            if config.diagnostics.len() > errors {
                return Err(config.diagnostics[errors..].join("; ").into());
            }
            self.config = config;
            return self.action("refresh");
        }
        if let Some(pattern) = action.strip_prefix('/') {
            self.search = pattern.into();
            self.find(false);
            return Ok(true);
        }
        if let Some(target) = action.strip_prefix("goto ") {
            if let Ok(line) = target.parse::<usize>() {
                self.view.selected = line
                    .saturating_sub(1)
                    .min(self.view.rows.len().saturating_sub(1));
            } else {
                self.goto_commit(target)?;
            }
            return Ok(true);
        }
        if !action.is_empty()
            && action.bytes().all(|byte| byte.is_ascii_hexdigit())
            && action.len() >= 7
        {
            self.goto_commit(action)?;
            return Ok(true);
        }
        let page = self.height.saturating_sub(2).max(1) as isize;
        match action {
            "quit" => return Ok(false),
            "view-close" | "back" => {
                if let Some(v) = self.previous.pop() {
                    self.revision = v.revision.clone();
                    self.path = v.path.clone();
                    self.view = v;
                } else {
                    return Ok(false);
                }
            }
            "enter" => self.enter()?,
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
                let selected = self.view.selected;
                self.view = self.load(&self.view.name)?;
                self.view.selected = selected.min(self.view.rows.len().saturating_sub(1));
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
            "screen-redraw" => (),
            _ if action.starts_with("view-") => {
                self.select_context();
                self.open(&action[5..])?;
            }
            _ => return Err(format!("Not implemented in Rust yet: {action}").into()),
        }
        Ok(true)
    }
    fn screen(&mut self) -> Vec<String> {
        let visible = self.height.saturating_sub(2).max(1);
        if self.view.selected < self.view.top {
            self.view.top = self.view.selected;
        }
        if self.view.selected >= self.view.top + visible {
            self.view.top = self.view.selected + 1 - visible;
        }
        let mut lines: Vec<String> = (0..visible)
            .map(|i| {
                clip(
                    self.view
                        .rows
                        .get(self.view.top + i)
                        .map(String::as_str)
                        .unwrap_or(""),
                    self.view.left,
                    self.width,
                )
            })
            .collect();
        lines.push(clip(
            &format!(
                "[{}] {} of {}",
                self.view.name,
                self.view.selected + usize::from(!self.view.rows.is_empty()),
                self.view.rows.len()
            ),
            0,
            self.width,
        ));
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
                self.view.selected = n
                    .parse::<usize>()?
                    .saturating_sub(1)
                    .min(self.view.rows.len().saturating_sub(1));
            } else {
                let action = if let Some(a) = line.strip_prefix(':') {
                    a.to_string()
                } else {
                    self.binding(line)
                };
                if !self.action(&action)? {
                    break;
                }
            }
        }
        Ok(())
    }
    fn binding(&self, key: &str) -> String {
        self.config
            .action(&self.view.name, key)
            .map(|a| a.join(" "))
            .unwrap_or_else(|| key.into())
    }
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
}
impl Terminal {
    fn open() -> Result<Self> {
        let out = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")?;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        for signal in [
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGINT,
        ] {
            signal_hook::flag::register(signal, stop.clone())?;
        }
        terminal::enable_raw_mode()?;
        let mut t = Self { out, stop };
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
            if i == app.view.selected.saturating_sub(app.view.top) || i + 2 == lines.len() {
                queue!(self.out, SetAttribute(Attribute::Reverse))?;
            }
            write!(self.out, "{line}")?;
            queue!(self.out, SetAttribute(Attribute::Reset))?;
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
    app.view.selected = cli.line.min(app.view.rows.len().saturating_sub(1));
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
        return app.script(&script);
    }
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
                if let Err(e) = app.action(&s) {
                    app.message = e.to_string();
                }
            }
        } else {
            match app.action(&action) {
                Ok(false) => break,
                Ok(true) => (),
                Err(e) => app.message = e.to_string(),
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
    fn terminal_content_is_safe_and_cell_clipped() {
        assert_eq!(clip("a界b", 0, 3), "a界");
        assert_eq!(clip("é界b", 1, 3), "界b");
        assert_eq!(clip("\x1b[31m", 0, 20), "\\x1b[31m");
        assert_eq!(clip("x\ty", 0, 20), "x       y");
    }
}
