// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Safe Rust migration of Tig. SPDX-License-Identifier: GPL-2.0-or-later
use std::{
    env, fs,
    io::{self, BufRead, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};
use tig_rs::config::Config;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(super) fn clip_prompt(text: &str, skip: usize, width: usize) -> String {
    let mut out = String::new();
    let mut column = 0;
    for grapheme in text.graphemes(true) {
        let cells = UnicodeWidthStr::width(grapheme);
        if column >= skip && column.saturating_add(cells) <= skip.saturating_add(width) {
            out.push_str(grapheme);
        }
        column += cells;
        if column >= skip.saturating_add(width) {
            break;
        }
    }
    out
}
pub(super) fn prompt_text(text: &str) -> String {
    let mut out = String::new();
    let mut column = 0;
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            let spaces = 8 - column % 8;
            out.push_str(&" ".repeat(spaces));
            column += spaces;
        } else if grapheme.chars().any(char::is_control) {
            for c in grapheme.chars() {
                let escaped = format!("\\x{:02x}", c as u32);
                column += escaped.len();
                out.push_str(&escaped);
            }
        } else {
            out.push_str(grapheme);
            column += UnicodeWidthStr::width(grapheme);
        }
    }
    out
}
pub(super) fn inputrc_motion() -> std::collections::HashMap<char, bool> {
    let path = env::var_os("INPUTRC")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".inputrc")));
    let Some(contents) = path.and_then(|path| fs::read_to_string(path).ok()) else {
        return Default::default();
    };
    let mut motions = std::collections::HashMap::new();
    let mut conditions = Vec::new();
    let mut active = true;
    for line in contents.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("$if ") {
            // Unknown condition types stay inactive in both branches.
            let matched = (!name.contains('=')).then(|| name.trim() == "tig");
            conditions.push((active, matched));
            active &= matched == Some(true);
        } else if line == "$else" {
            if let Some((parent, matched)) = conditions.last() {
                active = *parent && *matched == Some(false);
            }
        } else if line == "$endif" {
            if let Some((parent, _)) = conditions.pop() {
                active = parent;
            }
        } else if active {
            let Some((key, action)) = line.split_once(':') else {
                continue;
            };
            let Some(key) = key
                .trim()
                .strip_prefix("\"\\C-")
                .and_then(|s| s.strip_suffix('"'))
            else {
                continue;
            };
            let Some(key) = key.chars().next().filter(|_| key.len() == 1) else {
                continue;
            };
            let end = match action.trim() {
                "beginning-of-line" => false,
                "end-of-line" => true,
                _ => continue,
            };
            motions.insert(key.to_ascii_lowercase(), end);
        }
    }
    motions
}
pub(super) fn complete_prompt_action(value: &mut String, point: &mut usize) {
    const ACTIONS: &[&str] = &[
        "!",
        "source",
        "color",
        "bind",
        "set",
        "toggle",
        "goto",
        "save-display",
        "save-options",
        "exec",
        "echo",
        "none",
    ];
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return;
    }
    let mut matches = ACTIONS
        .iter()
        .map(|action| (*action).to_string())
        .chain(tig_rs::request_info().into_iter().map(|(_, name, _)| name))
        .filter(|action| action.starts_with(&*value));
    let Some(mut common) = matches.next() else {
        return;
    };
    for action in matches {
        let shared = common
            .bytes()
            .zip(action.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        common.truncate(shared);
    }
    if common.len() > value.len() {
        value.clear();
        value.push_str(&common);
        *point = value.len();
    }
}
pub(super) struct PromptHistory {
    path: Option<PathBuf>,
    pub(super) entries: Vec<String>,
    pub(super) limit: usize,
}
impl PromptHistory {
    pub(super) fn load(config: &Config) -> Self {
        let limit = config.usize_value("history-size", 500);
        if limit == 0 {
            return Self {
                path: None,
                entries: Vec::new(),
                limit,
            };
        }
        let home = env::var_os("HOME").map(PathBuf::from);
        let preferred = env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(|value| PathBuf::from(value).join("tig/history"))
            .or_else(|| {
                home.as_ref()
                    .map(|home| home.join(".local/share/tig/history"))
            });
        let mut path = preferred
            .and_then(|path| {
                if env::var_os("XDG_DATA_HOME").is_some_and(|value| !value.is_empty()) {
                    if let Some(parent) = path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                }
                fs::OpenOptions::new()
                    .read(true)
                    .append(true)
                    .create(true)
                    .open(&path)
                    .map(|_| path)
                    .ok()
            })
            .or_else(|| home.map(|home| home.join(".tig_history")));
        let mut entries = std::collections::VecDeque::new();
        if let Some(file) = path.as_ref().map(fs::File::open) {
            match file {
                Ok(file) => {
                    for line in io::BufReader::new(file).lines() {
                        match line {
                            Ok(line) => {
                                if entries.len() == limit {
                                    entries.pop_front();
                                }
                                entries.push_back(line);
                            }
                            Err(_) => {
                                path = None;
                                break;
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(_) => path = None,
            }
        }
        Self {
            path,
            entries: entries.into(),
            limit,
        }
    }
    pub(super) fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "history path is a symlink",
                ));
            }
            Ok(_) => {
                // Check the file itself before replacing it; a writable parent is not enough.
                fs::OpenOptions::new().write(true).open(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        let temporary = path.with_extension(format!("tig-rs-{}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let result = (|| {
            if let Ok(metadata) = fs::metadata(path) {
                fs::set_permissions(&temporary, metadata.permissions())?;
            }
            let mut seen = std::collections::HashSet::new();
            let mut entries = self
                .entries
                .iter()
                .rev()
                .filter(|entry| seen.insert(*entry))
                .collect::<Vec<_>>();
            entries.reverse();
            for entry in entries {
                writeln!(file, "{entry}")?;
            }
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}
