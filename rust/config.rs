// SPDX-License-Identifier: GPL-2.0-or-later
// Rust port of Tig configuration and argument handling.
// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// See COPYING for the license. Settings are retained here; their effects belong
// to the consuming views. This module never executes a configured command.
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub settings: BTreeMap<String, Vec<String>>,
    pub bindings: BTreeMap<(String, String), Vec<String>>,
    pub colors: BTreeMap<String, Vec<String>>,
    pub diagnostics: Vec<String>,
}

/// Split a config/prompt line without a shell, retaining empty quoted arguments.
/// Backslashes outside quotes are literal (notably the stage split binding).
pub fn words(line: &str) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' {
                word.push(chars.next().ok_or("Trailing escape in quoted argument")?);
            } else {
                word.push(c);
            }
        } else if c == '#' {
            break;
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            started = true;
        } else if c.is_whitespace() {
            if started {
                result.push(std::mem::take(&mut word));
                started = false;
            }
        } else {
            word.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return Err("Unclosed quoted argument".into());
    }
    if started {
        result.push(word);
    }
    Ok(result)
}

pub fn normalize_key(key: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut remaining = key;
    let mut count = 0;
    while !remaining.is_empty() {
        count += 1;
        if count > 16 {
            return Err("Key sequence exceeds 16 keys".into());
        }
        if remaining.starts_with('<') {
            let end = remaining.find('>').ok_or("Missing > in key name")?;
            let name = remaining[1..end].to_ascii_lowercase();
            let normalized = match name.as_str() {
                "hash" => "#",
                "space" => " ",
                "lt" | "lessthan" => "<",
                "singlequote" => "'",
                "doublequote" => "\"",
                "escape" | "esc" => "<Esc>",
                "enter" => "<Enter>",
                "tab" => "<Tab>",
                "backspace" => "<Backspace>",
                "up" => "<Up>",
                "down" => "<Down>",
                "left" => "<Left>",
                "right" => "<Right>",
                "home" => "<Home>",
                "end" => "<End>",
                "insert" | "ins" => "<Ins>",
                "delete" | "del" => "<Del>",
                "pageup" | "pgup" => "<PgUp>",
                "pagedown" | "pgdown" => "<PgDown>",
                "scrollback" | "sback" => "<SBack>",
                "scrollfwd" | "sfwd" => "<SFwd>",
                "backtab" | "shifttab" => "<BackTab>",
                "shiftleft" => "<ShiftLeft>",
                "shiftright" => "<ShiftRight>",
                "shiftdelete" | "shiftdel" => "<ShiftDel>",
                "shifthome" => "<ShiftHome>",
                "shiftend" => "<ShiftEnd>",
                _ => {
                    if let Some(c) = name
                        .strip_prefix("ctrl-")
                        .or_else(|| name.strip_prefix("c-"))
                    {
                        if c.chars().count() != 1 {
                            return Err(format!("Invalid control key: {key}"));
                        }
                        out.push_str(&format!("<C-{}>", c.to_uppercase()));
                    } else if name
                        .strip_prefix('f')
                        .and_then(|n| n.parse::<u8>().ok())
                        .is_some_and(|n| (1..=19).contains(&n))
                    {
                        out.push_str(&format!("<{}>", name.to_uppercase()));
                    } else {
                        return Err(format!("Unknown key: <{name}>"));
                    }
                    remaining = &remaining[end + 1..];
                    continue;
                }
            };
            out.push_str(normalized);
            remaining = &remaining[end + 1..];
        } else {
            let c = remaining.chars().next().unwrap();
            out.push(c);
            remaining = &remaining[c.len_utf8()..];
        }
    }
    if count == 0 {
        return Err("Empty key binding".into());
    }
    Ok(out)
}

impl Config {
    pub fn defaults() -> Self {
        let mut config = Self::default();
        config.parse_text(
            include_str!("../tigrc"),
            Path::new("<built-in>"),
            &mut Vec::new(),
        );
        config
    }
    pub fn load() -> Self {
        let mut config = Self::default();
        let system = env::var_os("TIGRC_SYSTEM");
        let path = system
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/tigrc"));
        if path.as_os_str().is_empty() || (!path.exists() && system.is_none()) {
            config = Self::defaults();
        } else {
            config.load_file(&path, true);
        }
        if let Some(user) = env::var_os("TIGRC_USER") {
            config.load_file(Path::new(&user), true);
        } else if let Some(home) = env::var_os("HOME") {
            let base = env::var_os("XDG_CONFIG_HOME")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&home).join(".config"));
            let path = base.join("tig/config");
            config.load_file(
                &if path.exists() {
                    path
                } else {
                    PathBuf::from(home).join(".tigrc")
                },
                true,
            );
        }
        if let Ok(value) = env::var("TIG_DIFF_OPTS") {
            if !value.is_empty() {
                match words(&value) {
                    Ok(args) => {
                        config.settings.insert("diff-options".into(), args);
                    }
                    Err(e) => config.diagnostics.push(format!("TIG_DIFF_OPTS: {e}")),
                }
            }
        }
        config
    }
    pub fn load_file(&mut self, path: &Path, quiet_missing: bool) {
        if let Err(e) = self.read_file(path, quiet_missing, &mut Vec::new()) {
            self.diagnostics.push(e);
        }
    }
    fn read_file(
        &mut self,
        path: &Path,
        quiet: bool,
        stack: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let path = expand_home(path)?;
        let canonical = match fs::canonicalize(&path) {
            Ok(p) => p,
            Err(e) if quiet && e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if stack.contains(&canonical) {
            return Err(format!("{}: source cycle", path.display()));
        }
        if stack.len() >= 64 {
            return Err("Source nesting exceeds 64 files".into());
        }
        let text =
            fs::read_to_string(&canonical).map_err(|e| format!("{}: {e}", path.display()))?;
        stack.push(canonical);
        self.parse_text(&text, &path, stack);
        stack.pop();
        Ok(())
    }
    pub fn parse(&mut self, text: &str) {
        self.parse_text(text, Path::new("<input>"), &mut Vec::new());
    }
    fn parse_text(&mut self, text: &str, path: &Path, stack: &mut Vec<PathBuf>) {
        let mut logical = String::new();
        let mut start = 1;
        for (index, line) in text.lines().enumerate() {
            if logical.is_empty() {
                start = index + 1;
            }
            let line = line.trim_end();
            if let Some(prefix) = line.strip_suffix('\\') {
                logical.push_str(prefix);
                logical.push(' ');
                continue;
            }
            logical.push_str(line);
            let result = words(&logical).and_then(|args| self.apply(&args, stack));
            if let Err(e) = result {
                self.diagnostics
                    .push(format!("{}:{start}: {e}", path.display()));
            }
            logical.clear();
        }
        if !logical.is_empty() {
            self.diagnostics.push(format!(
                "{}:{start}: Unterminated continuation",
                path.display()
            ));
        }
    }
    fn apply(&mut self, args: &[String], stack: &mut Vec<PathBuf>) -> Result<(), String> {
        if args.is_empty() {
            return Ok(());
        }
        match args[0].as_str() {
            "set" if args.len() >= 3 && args[2] == "=" => {
                if !known_option(&args[1]) {
                    return Err(format!("Unknown option: {}", args[1]));
                }
                validate_setting(&args[1], &args[3..])?;
                self.settings.insert(args[1].clone(), args[3..].to_vec());
            }
            "bind" if args.len() >= 4 => {
                let view = if args[1] == "branch" {
                    "refs"
                } else {
                    &args[1]
                };
                if !is_view(view) && view != "generic" && view != "search" {
                    return Err(format!("Unknown key map: {view}"));
                }
                let key = normalize_key(&args[2])?;
                if view == "generic" && args[3] == "none" {
                    self.bindings.retain(|(_, k), _| k != &key);
                } else {
                    self.bindings.insert((view.into(), key), args[3..].to_vec());
                }
            }
            "color" if args.len() >= 4 => {
                self.colors.insert(args[1].clone(), args[2..].to_vec());
            }
            "source" if args.len() == 2 => self.read_file(Path::new(&args[1]), false, stack)?,
            "source" if args.len() == 3 && args[1] == "-q" => {
                self.read_file(Path::new(&args[2]), true, stack)?
            }
            _ => return Err(format!("Unknown or malformed command: {}", args[0])),
        }
        Ok(())
    }
    pub fn action(&self, view: &str, key: &str) -> Option<&[String]> {
        let key = normalize_key(key).ok()?;
        self.bindings
            .get(&(view.into(), key.clone()))
            .or_else(|| self.bindings.get(&("generic".into(), key)))
            .map(Vec::as_slice)
    }
    pub fn value(&self, name: &str) -> Option<&str> {
        self.settings.get(name)?.first().map(String::as_str)
    }
    pub fn bool_value(&self, name: &str, fallback: bool) -> bool {
        match self.value(name) {
            Some("yes" | "true" | "1") => true,
            Some("no" | "false" | "0") => false,
            _ => fallback,
        }
    }
    pub fn usize_value(&self, name: &str, fallback: usize) -> usize {
        self.value(name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(fallback)
    }
}
fn expand_home(path: &Path) -> Result<PathBuf, String> {
    if path == Path::new("~") || path.starts_with("~/") {
        let home = env::var_os("HOME").ok_or("HOME is unset")?;
        Ok(PathBuf::from(home).join(path.strip_prefix("~").unwrap()))
    } else {
        Ok(path.into())
    }
}
fn option_type(name: &str) -> Option<&'static str> {
    include_str!("../include/tig/options.h")
        .split("#define DEFINE_OPTION_EXTERNS")
        .next()?
        .lines()
        .find_map(|line| {
            let mut parts = line.trim().strip_prefix("_(")?.split(',');
            (parts.next()?.trim().replace('_', "-") == name)
                .then(|| parts.next().map(str::trim))
                .flatten()
        })
}
fn known_option(name: &str) -> bool {
    option_type(name).is_some()
}
fn validate_setting(name: &str, values: &[String]) -> Result<(), String> {
    let kind = option_type(name).unwrap_or("");
    if kind == "const char **" || kind == "view_settings" || kind == "struct ref_format **" {
        return Ok(());
    }
    if values.len() != 1 {
        return Err(format!("{name} requires one value"));
    }
    let v = values[0].as_str();
    let valid = match kind {
        "bool" if name != "show-notes" => matches!(v, "yes" | "no" | "true" | "false" | "1" | "0"),
        "int" => v
            .parse::<u32>()
            .is_ok_and(|n| n <= i32::MAX as u32 && (name != "tab-size" || n > 0)),
        "double" => v
            .trim_end_matches('%')
            .parse::<f64>()
            .is_ok_and(|n| n.is_finite() && n >= 0.0 && (!v.ends_with('%') || n <= 100.0)),
        "enum commit_order" => matches!(
            v,
            "auto" | "default" | "topo" | "date" | "reverse" | "author-date"
        ),
        "enum ignore_case" => matches!(v, "no" | "yes" | "smart-case"),
        "enum ignore_space" => matches!(v, "no" | "all" | "some" | "at-eol"),
        "enum graphic" => matches!(v, "ascii" | "default" | "utf-8" | "auto"),
        "enum refresh_mode" => matches!(v, "manual" | "auto" | "after-command" | "periodic"),
        "enum vertical_split" => matches!(v, "horizontal" | "vertical" | "auto"),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid value for {name}: {v}"))
    }
}
pub fn is_view(name: &str) -> bool {
    matches!(
        name,
        "main"
            | "diff"
            | "log"
            | "reflog"
            | "tree"
            | "blob"
            | "blame"
            | "refs"
            | "pager"
            | "help"
            | "status"
            | "stage"
            | "stash"
            | "grep"
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    pub view: String,
    /// Original Git arguments including --, preserving path/revision ambiguity for Git.
    pub git_args: Vec<String>,
    /// Apply sequentially, as Git/Tig do with repeated -C.
    pub directories: Vec<PathBuf>,
    pub line: usize,
    pub help: bool,
    pub version: bool,
}
impl Cli {
    pub fn parse(args: &[String], pager_mode: bool) -> Result<Self, String> {
        let mut cli = Self {
            view: if pager_mode { "pager" } else { "main" }.into(),
            git_args: Vec::new(),
            directories: Vec::new(),
            line: 0,
            help: false,
            version: false,
        };
        let mut i = 0;
        while args.get(i).is_some_and(|s| s == "-C") {
            i += 1;
            cli.directories
                .push(PathBuf::from(args.get(i).ok_or("-C requires a directory")?));
            i += 1;
        }
        if let Some(command) = args.get(i) {
            let view = match command.as_str() {
                "show" => Some("diff"),
                "status" | "blame" | "grep" | "log" | "reflog" | "stash" | "refs" => {
                    Some(command.as_str())
                }
                _ => None,
            };
            if let Some(view) = view {
                cli.view = view.into();
                i += 1;
            }
        }
        let mut paths = false;
        for arg in &args[i..] {
            if !paths {
                match arg.as_str() {
                    "--" => paths = true,
                    "-h" | "--help" => {
                        cli.help = true;
                        continue;
                    }
                    "-v" | "--version" => {
                        cli.version = true;
                        continue;
                    }
                    _ => {
                        if let Some(n) = arg
                            .strip_prefix('+')
                            .filter(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
                        {
                            cli.line = n
                                .parse::<usize>()
                                .map_err(|_| "Line number too large")?
                                .saturating_sub(1);
                            continue;
                        }
                    }
                }
            }
            cli.git_args.push(arg.clone());
        }
        Ok(cli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_defaults_and_overrides() {
        let mut c = Config::defaults();
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        assert_eq!(c.action("status", "j").unwrap(), &["move-down"]);
        assert_eq!(c.action("stage", "\\").unwrap(), &["stage-split-chunk"]);
        c.parse("set tab-size = 4\nbind generic <Ctrl-n> move-up\nbind status j none\ncolor \"#literal\" red default");
        assert_eq!(c.usize_value("tab-size", 8), 4);
        assert_eq!(c.action("main", "<C-N>").unwrap(), &["move-up"]);
        assert_eq!(c.action("status", "j").unwrap(), &["none"]);
        assert!(c.colors.contains_key("#literal"));
        assert_eq!(
            words("a '' \"b # c\" # discarded").unwrap(),
            ["a", "", "b # c"]
        );
        assert!(words("'broken").is_err());
        c.parse("set tab-size = 0\nset mouse = maybe\nset invented = yes");
        assert_eq!(c.diagnostics.len(), 3);
        assert_eq!(c.value("tab-size"), Some("4"));
    }
    #[test]
    fn source_cycles_missing_and_recovery() {
        let dir = env::temp_dir().join(format!("tig-rust-config-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config");
        fs::write(
            &path,
            format!("source \"{}\"\nset tab-size = 3\n", path.display()),
        )
        .unwrap();
        let mut c = Config::default();
        c.load_file(&path, false);
        assert!(c.diagnostics.iter().any(|d| d.contains("cycle")));
        assert_eq!(c.value("tab-size"), Some("3"));
        let before = c.diagnostics.len();
        c.load_file(&dir.join("missing"), true);
        assert_eq!(c.diagnostics.len(), before);
        c.load_file(&dir.join("missing"), false);
        assert_eq!(c.diagnostics.len(), before + 1);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn cli_preserves_git_arguments_and_separator() {
        let args = ["-C", "repo", "show", "+12", "HEAD~2", "--", "--help", "a b"].map(String::from);
        let c = Cli::parse(&args, false).unwrap();
        assert_eq!(c.view, "diff");
        assert_eq!(c.line, 11);
        assert!(!c.help);
        assert_eq!(c.git_args, ["HEAD~2", "--", "--help", "a b"]);
        assert!(Cli::parse(&["-C".into()], false).is_err());
    }
}
