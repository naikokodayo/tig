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
    if key.starts_with('^') && key.chars().count() > 1 {
        return Err("Control/escape key mappings must use <Ctrl-X> or <Esc> notation".into());
    }
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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!("File does not exist: {}", path.display()))
            }
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
    /// Apply one prompt command and return its diagnostic without swallowing it.
    pub fn apply_command(&mut self, line: &str) -> Result<(), String> {
        self.apply_command_for_view("main", line)
    }
    /// Column toggles apply to the active view; global toggles remain shared.
    pub fn apply_command_for_view(&mut self, view: &str, line: &str) -> Result<(), String> {
        let args = words(line)?;
        if args.first().is_some_and(|s| s == "toggle") {
            self.toggle(view, &args[1..])
        } else {
            self.apply(&args, &mut Vec::new())
        }
    }
    fn toggle(&mut self, view: &str, args: &[String]) -> Result<(), String> {
        let name = args
            .first()
            .ok_or("No option name given to :toggle")?
            .to_ascii_lowercase()
            .replace('_', "-");
        if let Some(kind) = option_type(&name) {
            if kind == "const char **" {
                let values = args[1..].to_vec();
                let old = self.settings.entry(name).or_default();
                if values.is_empty() {
                    old.clear();
                } else if values.iter().all(|v| old.contains(v)) {
                    old.retain(|v| !values.contains(v));
                } else {
                    *old = values;
                }
                return Ok(());
            }
            let default = match name.as_str() {
                "diff-context" => "3",
                "file-filter" | "rev-filter" => "yes",
                _ => "0",
            };
            let old = self.value(&name).unwrap_or(default);
            let value = toggled_value(&name, kind, old, &args[1..])?;
            self.settings.insert(name, vec![value]);
            return Ok(());
        }
        let (view, suffix) = name.split_once("-view-").unwrap_or((view, name.as_str()));
        if !is_view(view) {
            return Err(format!("Unknown view: {view}"));
        }
        let column = column_names()
            .find(|c| suffix == *c || suffix.starts_with(&format!("{c}-")))
            .ok_or_else(|| format!("`:toggle {name}` not supported"))?;
        let option = suffix
            .strip_prefix(column)
            .and_then(|s| s.strip_prefix('-'))
            .unwrap_or("display");
        let kind = column_type(column, option)
            .ok_or_else(|| format!("Unknown option `{option}' for column {column}"))?;
        let columns = self
            .settings
            .get(&format!("{view}-view"))
            .ok_or_else(|| format!("The {view} view has no columns configured"))?;
        let spec = columns
            .iter()
            .find(|s| s.split(':').next() == Some(column))
            .ok_or_else(|| format!("The {view} view does not have a {column} column configured"))?;
        let mut parts = spec
            .split_once(':')
            .map_or("yes", |(_, rest)| rest)
            .split(',');
        let display = parts.next().unwrap_or("yes");
        let old = if option == "display" {
            display
        } else {
            parts
                .filter_map(|part| {
                    let (key, value) = part.split_once('=').unwrap_or((part, "yes"));
                    (key == option).then_some(value)
                })
                .last()
                .unwrap_or("0")
        };
        let value = toggled_value(&format!("{column}-{option}"), kind, old, &args[1..])?;
        self.set_column(&format!("{view}-view-{column}-{option}"), &[value])
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
            // Upstream strips comments before splitting arguments, even inside quotes.
            let content = logical.split('#').next().unwrap_or("");
            let result = words(content).and_then(|args| self.apply(&args, stack));
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
            "set" if args.len() >= 3 && matches!(args[2].as_str(), "=" | "+=") => {
                let name = args[1].to_ascii_lowercase().replace('_', "-");
                if name.contains("-view-") && option_type(&name).is_none() {
                    if args[2] == "+=" {
                        return Err(format!("Option {name} does not support +="));
                    }
                    self.set_column(&name, &args[3..])?;
                } else {
                    let kind =
                        option_type(&name).ok_or_else(|| format!("Unknown option name: {name}"))?;
                    if args[2] == "+=" && kind != "const char **" {
                        return Err(format!("Option {name} does not support +="));
                    }
                    validate_setting(&name, &args[3..])?;
                    let values = if kind.starts_with("enum ") && args.len() == 4 {
                        vec![normalize_enum(kind.trim_start_matches("enum "), &args[3])?]
                    } else {
                        args[3..].to_vec()
                    };
                    if args[2] == "+=" {
                        self.settings.entry(name).or_default().extend(values);
                    } else {
                        if kind == "view_settings" {
                            let prefix = format!("{name}-");
                            self.settings.retain(|key, _| !key.starts_with(&prefix));
                        }
                        self.settings.insert(name, values);
                    }
                }
            }
            "set" => return Err("Invalid set command: set option = value".into()),
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
                let action = args[3].to_ascii_lowercase().replace('_', "-");
                let request = known_request(&action);
                if !request && !args[3].starts_with([':', '!', '?', '@', '<', '+', '>']) {
                    return Err(format!(
                        "Unknown command flag '{}'; expected one of :!?@<+>",
                        args[3].chars().next().unwrap_or(' ')
                    ));
                }
                if view == "generic" && action == "none" {
                    let old = self.bindings.len();
                    self.bindings.retain(|(_, k), _| k != &key);
                    if self.bindings.len() == old {
                        return Err(format!("No keybinding found for {key}"));
                    }
                } else {
                    self.bindings.insert(
                        (view.into(), key),
                        if request {
                            vec![action]
                        } else {
                            args[3..].to_vec()
                        },
                    );
                }
            }
            "bind" => return Err("Invalid key binding: bind keymap key action".into()),
            "color" if args.len() >= 4 => {
                validate_colors(&args[2..])?;
                self.colors.insert(args[1].clone(), args[2..].to_vec());
            }
            "color" => {
                return Err("Invalid color mapping: color area fgcolor bgcolor [attrs]".into())
            }
            "source" if args.len() == 2 => self.read_file(Path::new(&args[1]), false, stack)?,
            "source" if args.len() == 3 && args[1] == "-q" => {
                self.read_file(Path::new(&args[2]), true, stack)?
            }
            "source" => return Err("Invalid source command: source [-q] <path>".into()),
            _ => return Err(format!("Unknown option command: {}", args[0])),
        }
        Ok(())
    }
    fn set_column(&mut self, name: &str, values: &[String]) -> Result<(), String> {
        if values.len() != 1 {
            return Err(format!("Option {name} only takes one value"));
        }
        let (view, suffix) = name.split_once("-view-").unwrap();
        let base = format!("{view}-view");
        if option_type(&base) != Some("view_settings") {
            return Err(format!("Unknown option name: {name}"));
        }
        let column = column_names()
            .find(|c| suffix == *c || suffix.starts_with(&format!("{c}-")))
            .ok_or_else(|| format!("Unknown view column: {suffix}"))?;
        let old = self
            .settings
            .get(&base)
            .ok_or_else(|| format!("The {view} view has no columns configured"))?;
        let index = old
            .iter()
            .position(|s| s.split(':').next() == Some(column))
            .ok_or_else(|| format!("The {view} view does not have a {column} column configured"))?;
        let mut specs = old.clone();
        let original = specs[index].split_once(':').map_or("yes", |(_, s)| s);
        let mut fields: Vec<String> = original.split(',').map(String::from).collect();
        if let Some(option) = suffix
            .strip_prefix(column)
            .and_then(|s| s.strip_prefix('-'))
        {
            validate_column_value(column, option, &values[0])?;
            if option == "display" {
                fields[0] = values[0].clone();
            } else {
                fields.retain(|s| s.split('=').next() != Some(option));
                fields.push(format!("{option}={}", values[0]));
            }
        } else {
            // Whole-column updates merge specified attributes with existing ones.
            let mut update = values[0].split(',');
            fields[0] = update.next().unwrap_or("yes").into();
            for value in update {
                let key = value.split('=').next().unwrap();
                fields.retain(|s| s.split('=').next() != Some(key));
                fields.push(value.into());
            }
        }
        let spec = format!("{column}:{}", fields.join(","));
        validate_column(&spec)?;
        specs[index] = spec;
        self.settings.insert(base, specs);
        // Column specs are canonical: stale override keys must not supersede them.
        let prefix = format!("{view}-view-{column}");
        self.settings
            .retain(|key, _| key != &prefix && !key.starts_with(&format!("{prefix}-")));
        Ok(())
    }
    pub fn action(&self, view: &str, key: &str) -> Option<&[String]> {
        let key = if key == "<" {
            key.into()
        } else {
            normalize_key(key).ok()?
        };
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
fn toggled_value(name: &str, kind: &str, old: &str, args: &[String]) -> Result<String, String> {
    if args.len() > 1 {
        return Err(format!("Too many arguments for :toggle {name}"));
    }
    let value = if kind == "bool" {
        if !args.is_empty() {
            return Err(format!("Boolean toggle {name} takes no increment"));
        }
        if matches!(old, "yes" | "true" | "1")
            || (name == "show-notes" && !matches!(old, "no" | "false" | "0" | ""))
        {
            "no".into()
        } else {
            "yes".into()
        }
    } else if let Some(kind) = kind.strip_prefix("enum ") {
        if !args.is_empty() {
            return Err(format!("Enum toggle {name} takes no increment"));
        }
        let values = enum_values(kind);
        let old = normalize_enum(kind, old)?;
        let index = values
            .iter()
            .position(|v| v == &old)
            .ok_or_else(|| format!("Unknown current value for {name}: {old}"))?;
        values[(index + 1) % values.len()].clone()
    } else if kind == "int" {
        let current = match old {
            "yes" | "true" => 50,
            "no" | "false" => 0,
            value => value
                .parse::<i64>()
                .map_err(|_| format!("Invalid current value for {name}: {old}"))?,
        };
        if name.ends_with("commit-title-overflow") {
            if !args.is_empty() {
                return Err(format!("Overflow toggle {name} takes no increment"));
            }
            if current == 0 {
                "50".into()
            } else {
                (-current).to_string()
            }
        } else {
            let increment = args.first().map(String::as_str).unwrap_or("1");
            let delta = increment
                .parse::<i64>()
                .map_err(|_| format!("Invalid integer increment: {increment}"))?;
            let delta = if delta == 0 {
                if increment.starts_with('-') {
                    -1
                } else {
                    1
                }
            } else {
                delta
            };
            let mut next = current
                .checked_add(delta)
                .ok_or("Integer toggle overflow")?;
            if name == "diff-context" && delta < 0 {
                if current == 0 {
                    return Err("Diff context cannot be less than zero".into());
                }
                next = next.max(0);
            }
            next.to_string()
        }
    } else if kind == "double" {
        let number = |s: &str| -> Result<f64, String> {
            let (s, scale) = s.strip_suffix('%').map_or((s, 1.0), |s| (s, 100.0));
            let n = s
                .parse::<f64>()
                .map_err(|_| "Invalid numeric increment".to_string())?
                / scale;
            if n.is_finite() {
                Ok(n)
            } else {
                Err("Invalid numeric increment".into())
            }
        };
        let delta = args.first().map(String::as_str).unwrap_or("1");
        let next = number(old)? + number(delta)?;
        if next > 0.0 && next < 1.0 {
            format!("{}%", next * 100.0)
        } else {
            next.to_string()
        }
    } else {
        return Err(format!("Unsupported `:toggle {name}` ({kind})"));
    };
    validate_scalar(name, kind, &value)?;
    Ok(value)
}
fn enum_values(kind: &str) -> Vec<String> {
    let marker = format!("#define {}_ENUM(_) ", kind.to_ascii_uppercase());
    include_str!("../include/tig/types.h")
        .split(&marker)
        .nth(1)
        .unwrap_or("")
        .split("\n\n")
        .next()
        .unwrap_or("")
        .lines()
        .filter_map(|line| {
            let (_, value) = line.trim().strip_prefix("_(")?.split_once(',')?;
            Some(
                value
                    .split(')')
                    .next()?
                    .trim()
                    .to_ascii_lowercase()
                    .replace('_', "-"),
            )
        })
        .collect()
}
fn normalize_enum(kind: &str, value: &str) -> Result<String, String> {
    let values = enum_values(kind);
    let value = value.to_ascii_lowercase().replace('_', "-");
    if kind == "graphic" && value == "auto" {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .filter_map(std::env::var_os)
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        return Ok(if locale.to_string_lossy().contains("UTF")
            || locale.to_string_lossy().contains("utf")
        {
            "utf-8"
        } else {
            "default"
        }
        .into());
    }
    if values.contains(&value) {
        return Ok(value);
    }
    let index = match value.as_str() {
        "yes" | "true" | "1" => Some(1),
        "no" | "false" | "0" => Some(0),
        _ => None,
    };
    index
        .and_then(|i| values.get(i).cloned())
        .ok_or_else(|| format!("Invalid {kind} value: {value}"))
}
fn column_names() -> impl Iterator<Item = &'static str> {
    [
        "author",
        "committer",
        "commit-title",
        "date",
        "file-name",
        "file-size",
        "id",
        "line-number",
        "mode",
        "ref",
        "section",
        "status",
        "text",
    ]
    .into_iter()
}
fn column_type(column: &str, option: &str) -> Option<&'static str> {
    let column = if column == "committer" {
        "author"
    } else {
        column
    };
    let marker = format!(
        "#define {}_COLUMN_OPTIONS(_) ",
        column.to_ascii_uppercase().replace('-', "_")
    );
    include_str!("../include/tig/options.h")
        .split(&marker)
        .nth(1)?
        .split("\n\n")
        .next()?
        .lines()
        .find_map(|line| {
            let mut fields = line.trim().strip_prefix("_(")?.split(',');
            if fields.next()?.trim().replace('_', "-") == option {
                fields.next().map(str::trim)
            } else {
                None
            }
        })
}
fn validate_scalar(name: &str, kind: &str, value: &str) -> Result<(), String> {
    if let Some(kind) = kind.strip_prefix("enum ") {
        return normalize_enum(kind, value).map(|_| ());
    }
    match kind {
        "bool" if name != "show-notes" => {
            if !matches!(value, "yes" | "no" | "true" | "false" | "1" | "0") {
                return Err(format!("Invalid boolean value: {value}"));
            }
        }
        "int" => {
            if name.ends_with("overflow")
                && value
                    .parse::<i64>()
                    .is_ok_and(|n| (-1024..=1024).contains(&n))
            {
                return Ok(());
            }
            if name.ends_with("overflow") && matches!(value, "yes" | "no" | "true" | "false") {
                return Ok(());
            }
            let min = usize::from(name == "tab-size" || name == "line-number-interval");
            let percent = name.ends_with("maxwidth") && value.ends_with('%');
            let max = if percent {
                100
            } else if name == "id-width" {
                40
            } else {
                1024
            };
            let input = if percent {
                &value[..value.len() - 1]
            } else {
                value
            };
            if !input.parse::<usize>().is_ok_and(|n| n >= min && n <= max) {
                return Err(format!("Value must be between {min} and {max}"));
            }
        }
        "double" => {
            let percent = value.ends_with('%');
            let number = if percent {
                &value[..value.len() - 1]
            } else {
                value
            };
            let n = number
                .parse::<f64>()
                .map_err(|_| "Invalid double or percentage")?;
            if !n.is_finite() {
                return Err("Invalid double or percentage".into());
            }
            if percent && n >= 100.0 {
                return Err("Percentage is larger than 100%".into());
            }
            if n < 0.0 {
                return Err("Percentage is less than 0%".into());
            }
        }
        _ => {}
    }
    Ok(())
}
fn validate_column_value(column: &str, option: &str, value: &str) -> Result<(), String> {
    let kind = column_type(column, option)
        .ok_or_else(|| format!("Unknown option `{option}' for column {column}"))?;
    validate_scalar(&format!("{column}-{option}"), kind, value)
}
fn validate_column(spec: &str) -> Result<(), String> {
    let (column, rest) = spec.split_once(':').unwrap_or((spec, "yes"));
    if !column_names().any(|c| c == column) {
        return Err(format!("Failed to parse view column type: {column}"));
    }
    let mut parts = rest.split(',');
    validate_column_value(column, "display", parts.next().unwrap_or("yes"))?;
    for part in parts {
        let (key, value) = part.split_once('=').unwrap_or((part, "yes"));
        validate_column_value(column, key, value)?;
    }
    Ok(())
}
fn validate_setting(name: &str, values: &[String]) -> Result<(), String> {
    let kind = option_type(name).unwrap_or("");
    if kind == "const char **" {
        return Ok(());
    }
    if values.is_empty() {
        return Err("Invalid set command: set option = value".into());
    }
    if kind == "view_settings" {
        return values.iter().try_for_each(|v| validate_column(v));
    }
    if kind == "struct ref_format **" {
        return Ok(());
    }
    if values.len() != 1 {
        return Err(format!("Option {name} only takes one value"));
    }
    validate_scalar(name, kind, &values[0])
}
fn validate_colors(values: &[String]) -> Result<(), String> {
    for color in &values[..2] {
        let value = color.to_ascii_lowercase();
        if !matches!(
            value.as_str(),
            "default"
                | "black"
                | "blue"
                | "cyan"
                | "green"
                | "magenta"
                | "red"
                | "white"
                | "yellow"
        ) && value
            .strip_prefix("color")
            .unwrap_or(&value)
            .parse::<u8>()
            .is_err()
        {
            return Err(format!("Unknown color: {color}"));
        }
    }
    for attr in &values[2..] {
        if !matches!(
            attr.to_ascii_lowercase().as_str(),
            "normal" | "blink" | "bold" | "dim" | "reverse" | "standout" | "underline"
        ) {
            return Err(format!("Unknown color attribute: {attr}"));
        }
    }
    Ok(())
}
fn known_request(name: &str) -> bool {
    if let Some(view) = name.strip_prefix("view-") {
        if is_view(view) {
            return true;
        }
    }
    include_str!("../include/tig/request.h")
        .lines()
        .any(|line| {
            line.trim()
                .strip_prefix("REQ_(")
                .and_then(|s| s.split(',').next())
                .is_some_and(|s| s.to_ascii_lowercase().replace('_', "-") == name)
        })
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
        c.parse("set tab-size = 4\nbind generic <Ctrl-n> move-up\nbind status j none");
        c.apply_command("color \"#literal\" red default").unwrap();
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
    fn original_view_column_fixture_and_atomic_errors() {
        let fixture = include_str!("../test/tigrc/view-column-test");
        let text = fixture
            .split("tigrc <<EOF\n")
            .nth(1)
            .unwrap()
            .split("\nEOF")
            .next()
            .unwrap();
        let mut c = Config::defaults();
        c.parse(text);
        assert_eq!(c.diagnostics.len(), 4, "{:?}", c.diagnostics);
        let columns = c.settings.get("main-view").unwrap();
        assert!(columns.contains(&"date:custom,format=%Y-%m-%d".into()));
        assert!(columns.contains(&"line-number:yes,interval=3".into()));
        assert!(columns.contains(&"author:abbreviated,width=12".into()));
        assert!(columns.contains(&"committer:email-user".into()));
        assert!(!columns.iter().any(|s| s.starts_with("id:")));
        let before = c.settings.clone();
        assert!(c
            .apply_command("set main-view = author:full,width=2000 commit-title")
            .is_err());
        assert!(c
            .apply_command("set main_view_line_number_visibility = yes")
            .is_err());
        assert_eq!(c.settings, before);
        c.apply_command("set main_view_author_width = 8").unwrap();
        assert!(c.settings["main-view"].contains(&"author:abbreviated,width=8".into()));
    }
    #[test]
    fn append_colors_and_numeric_boundaries() {
        let mut c = Config::defaults();
        c.apply_command("set log-options += --minimal").unwrap();
        assert_eq!(c.settings["log-options"], ["--cc", "--stat", "--minimal"]);
        assert!(c
            .apply_command("set mailmap += no")
            .unwrap_err()
            .contains("does not support +="));
        for command in [
            "set ignore-space =",
            "set tab-size = 1025",
            "set id-width = 41",
            "set split-view-height = 110%",
            "set split-view-height = -10%",
            "set split-view-height = NaN",
            "color file green bold",
            "color file dark green",
            "color file green green normally",
            "bind generic x unrecognized",
        ] {
            assert!(c.apply_command(command).is_err(), "{command}");
        }
        c.apply_command("set vertical-split = no").unwrap();
        assert_eq!(c.value("vertical-split"), Some("horizontal"));
        c.apply_command("set ignore_space = AT_EOL").unwrap();
        assert_eq!(c.value("ignore-space"), Some("at-eol"));
        c.apply_command("color file color255 0 bold underline")
            .unwrap();
        assert!(c.apply_command("color file color256 default").is_err());
    }
    #[test]
    fn command_quotes_comments_continuations_and_sequences() {
        let mut c = Config::default();
        c.parse(
            r#"bind generic <Esc>ø !sh -c "echo \"quoted\" 'shell command'"
set log-options = --all \
 --max-count=20
# Entire continued comment \
bind generic q quit
bind generic <Lt> back
"#,
        );
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        assert_eq!(
            c.action("main", "<Escape>ø").unwrap(),
            &["!sh", "-c", "echo \"quoted\" 'shell command'"]
        );
        assert_eq!(c.settings["log-options"], ["--all", "--max-count=20"]);
        assert!(c.action("main", "q").is_none());
        assert_eq!(c.action("main", "<").unwrap(), &["back"]);
        c.apply_command("bind generic <C-n> move-down").unwrap();
        assert_eq!(c.action("main", "<Ctrl-N>").unwrap(), &["move-down"]);
        c.apply_command("bind status <C-N> none").unwrap();
        c.apply_command("bind generic <C-N> none").unwrap();
        assert!(c.action("status", "<C-n>").is_none());
        assert!(c.apply_command("bind generic ^C quit").is_err());
        c.parse("set truncation-delimiter = \"#literal\"");
        assert_eq!(c.diagnostics.len(), 1); // Upstream strips # before quote parsing.
    }
    #[test]
    fn original_main_default_toggle_sequence() {
        let mut c = Config::defaults();
        let fixture = include_str!("../test/main/default-test");
        for line in fixture
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with(":toggle "))
        {
            c.apply_command(line.strip_prefix(':').unwrap()).unwrap();
        }
        let title = c.settings["main-view"]
            .iter()
            .find(|s| s.starts_with("commit-title:"))
            .unwrap();
        assert!(title.contains("refs=no"), "{title}");
        assert!(title.contains("graph=no"), "{title}");
        assert!(!c.settings.keys().any(|s| s.starts_with("main-view-")));
        c.apply_command("toggle commit-title-graph").unwrap();
        assert!(c.settings["main-view"]
            .iter()
            .any(|s| s.contains("graph=v2")));
        let main = c.settings["main-view"].clone();
        c.apply_command_for_view("diff", "toggle line-number")
            .unwrap();
        assert!(c.settings["diff-view"]
            .iter()
            .any(|s| s.starts_with("line-number:yes")));
        assert_eq!(c.settings["main-view"], main);
    }
    #[test]
    fn typed_toggles_validate_and_preserve_state() {
        let mut c = Config::defaults();
        c.apply_command("set show-notes = refs/notes/review")
            .unwrap();
        c.apply_command("toggle show-notes").unwrap();
        assert_eq!(c.value("show-notes"), Some("no"));
        c.apply_command("set line-graphics = auto").unwrap();
        assert!(matches!(
            c.value("line-graphics"),
            Some("default" | "utf-8")
        ));
        c.apply_command("toggle show-changes").unwrap();
        assert_eq!(c.value("show-changes"), Some("no"));
        c.apply_command("toggle ignore-space").unwrap();
        assert_eq!(c.value("ignore-space"), Some("all"));
        c.apply_command("toggle diff-context +2").unwrap();
        assert_eq!(c.value("diff-context"), Some("5"));
        c.apply_command("toggle diff-context -20").unwrap();
        assert_eq!(c.value("diff-context"), Some("0"));
        c.apply_command("toggle horizontal-scroll -10%").unwrap();
        assert_eq!(c.value("horizontal-scroll"), Some("40%"));
        c.apply_command("toggle commit-title-overflow").unwrap();
        assert!(c.settings["main-view"]
            .iter()
            .any(|s| s.contains("overflow=50")));
        c.apply_command("toggle commit-title-overflow").unwrap();
        assert!(c.settings["main-view"]
            .iter()
            .any(|s| s.contains("overflow=-50")));
        for command in [
            "toggle",
            "toggle unknown",
            "toggle diff-context -1",
            "toggle tab-size nan",
            "toggle tab-size 99999999999999999999",
            "toggle show-changes +2",
        ] {
            let before = c.settings.clone();
            assert!(c.apply_command(command).is_err(), "{command}");
            assert_eq!(c.settings, before);
        }
        c.apply_command("toggle log-options --cc").unwrap();
        assert_eq!(c.settings["log-options"], ["--stat"]);
        c.apply_command("toggle log-options --oneline").unwrap();
        assert_eq!(c.settings["log-options"], ["--oneline"]);
        c.apply_command("toggle log-options").unwrap();
        assert!(c.settings["log-options"].is_empty());
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
