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
pub(super) fn complete_prompt(value: &mut String, point: &mut usize) -> Vec<String> {
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
    // Offer only variables currently supplied by Rust command expansion.
    const VARIABLES: &[&str] = &[
        "%(commit)",
        "%(branch)",
        "%(directory)",
        "%(file)",
        "%(head)",
        "%(lineno)",
        "%(ref)",
        "%(remote)",
        "%(tag)",
        "%(refname)",
        "%(repo:head)",
        "%(repo:head-id)",
        "%(repo:remote)",
        "%(repo:upstream)",
        "%(repo:cdup)",
        "%(repo:prefix)",
        "%(repo:git-dir)",
        "%(repo:worktree)",
        "%(repo:exec-dir)",
        "%(repo:is-inside-work-tree)",
        "%(revargs)",
        "%(fileargs)",
        "%(cmdlineargs)",
    ];
    if *point > value.len() || !value.is_char_boundary(*point) {
        return Vec::new();
    }
    let before = &value[..*point];
    let mut start = 0;
    let mut escaped = false;
    let mut quote = None;
    for (i, ch) in before.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if quote == Some(ch) {
            quote = None;
            start = i + ch.len_utf8();
        } else if quote.is_none() && (ch == '\'' || ch == '"') {
            quote = Some(ch);
            start = i + ch.len_utf8();
        } else if quote.is_none() && " \t\n'`@$><=;|&{".contains(ch) {
            start = i + ch.len_utf8();
        }
    }
    let word = &before[start..];
    let mut file_mode = false;
    let mut lookup = word.to_string();
    let (options, toggles) = tig_rs::completion_option_names();
    let candidates: Vec<String> = if start == 0 {
        ACTIONS
            .iter()
            .map(|s| (*s).to_string())
            .chain(tig_rs::request_info().into_iter().map(|(_, name, _)| name))
            .collect()
    } else if before.starts_with("toggle ") {
        toggles
    } else if before.starts_with("set ") && !before.contains('=') {
        options
            .into_iter()
            .map(|name| format!("{name} = "))
            .collect()
    } else if word.starts_with("%(") {
        VARIABLES.iter().map(|s| (*s).to_string()).collect()
    } else {
        file_mode = true;
        lookup = unescape(word);
        file_candidates(&lookup)
    };
    if start == 0 && candidates.iter().any(|candidate| candidate == word) {
        return Vec::new();
    }
    let mut matches: Vec<String> = candidates
        .into_iter()
        .filter(|s| s.starts_with(&lookup) && s != &lookup)
        .collect();
    matches.sort();
    matches.dedup();
    let Some(mut common) = matches.first().cloned() else {
        return Vec::new();
    };
    for candidate in &matches[1..] {
        let shared = common
            .chars()
            .zip(candidate.chars())
            .take_while(|(a, b)| a == b)
            .map(|(ch, _)| ch.len_utf8())
            .sum();
        common.truncate(shared);
    }
    if common.len() > lookup.len() {
        let mut end = value.len();
        let mut right_escaped = false;
        for (offset, ch) in value[*point..].char_indices() {
            if right_escaped {
                right_escaped = false;
            } else if ch == '\\' {
                right_escaped = true;
            } else if quote == Some(ch) || (quote.is_none() && " \t\n\"'`@$><=;|&{".contains(ch)) {
                end = *point + offset;
                break;
            }
        }
        let replacement = if file_mode {
            let quote_char = quote.unwrap_or('"');
            let mut escaped = common
                .replace('\\', "\\\\")
                .replace(quote_char, &format!("\\{quote_char}"));
            let done = matches.len() == 1
                && !common.ends_with('/')
                && !(quote.is_some() && value[end..].starts_with(quote_char));
            if quote.is_some() {
                if done {
                    escaped.push(quote_char);
                }
                escaped
            } else if common.chars().any(char::is_whitespace)
                || common.contains('"')
                || common.contains('\'')
                || common.contains('\\')
            {
                format!("\"{escaped}{}", if done { "\"" } else { "" })
            } else {
                common
            }
        } else {
            common
        };
        value.replace_range(start..end, &replacement);
        *point = start + replacement.len();
    }
    matches
}

fn unescape(word: &str) -> String {
    let mut result = String::new();
    let mut chars = word.chars();
    while let Some(ch) = chars.next() {
        result.push(if ch == '\\' {
            chars.next().unwrap_or(ch)
        } else {
            ch
        });
    }
    result
}

fn file_candidates(word: &str) -> Vec<String> {
    if word == "~" && env::var_os("HOME").is_some() {
        return vec!["~/".into()];
    }
    let expanded = word
        .strip_prefix("~/")
        .and_then(|rest| env::var_os("HOME").map(|home| PathBuf::from(home).join(rest)));
    let path = expanded
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new(word));
    let (parent, prefix) = if word.ends_with('/') {
        (path, "")
    } else {
        (
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new(".")),
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(""),
        )
    };
    let parent_text = if word.ends_with('/') {
        word
    } else {
        word.strip_suffix(prefix).unwrap_or("")
    };
    let Some(directory) = fs::read_dir(parent).ok() else {
        return Vec::new();
    };
    directory
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with(prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let mut candidate = format!("{parent_text}{name}");
            if entry.file_type().ok()?.is_dir() || entry.metadata().is_ok_and(|meta| meta.is_dir())
            {
                candidate.push('/');
            }
            Some(candidate)
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::complete_prompt;

    fn complete(input: &str, point: usize) -> (String, usize, Vec<String>) {
        let mut value = input.to_string();
        let mut point = point;
        let matches = complete_prompt(&mut value, &mut point);
        (value, point, matches)
    }

    #[test]
    fn completes_actions_options_variables_and_paths() {
        assert_eq!(complete("tog", 3).0, "toggle");
        assert_eq!(complete("togXYZ", 3).0, "toggle");
        assert_eq!(complete("view-close", "view-close".len()).0, "view-close");
        assert_eq!(
            complete("set history-s", "set history-s".len()).0,
            "set history-size = "
        );
        assert_eq!(
            complete("toggle wrap-s", "toggle wrap-s".len()).0,
            "toggle wrap-search"
        );
        assert_eq!(
            complete("exec %(repo:head-i", "exec %(repo:head-i".len()).0,
            "exec %(repo:head-id)"
        );
        let directory = std::env::temp_dir().join(format!("tig-prompt-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("a file"), b"").unwrap();
        std::fs::write(directory.join("quote's"), b"").unwrap();
        let input = format!("source {}/a", directory.display());
        let (value, point, _) = complete(&input, input.len());
        assert_eq!(value, format!("source \"{}/a file\"", directory.display()));
        assert_eq!(point, value.len());
        assert_eq!(
            tig_rs::config::words(&value).unwrap()[1],
            directory.join("a file").to_string_lossy()
        );
        let escaped = format!("source {}/a\\ f", directory.display());
        assert_eq!(complete(&escaped, escaped.len()).0, value);
        let quoted = format!("source \"{}/a f\"", directory.display());
        let cursor = quoted.len() - 2;
        assert_eq!(complete(&quoted, cursor).0, value);
        let apostrophe = format!("source {}/quo", directory.display());
        let completed = complete(&apostrophe, apostrophe.len()).0;
        assert_eq!(
            tig_rs::config::words(&completed).unwrap()[1],
            directory.join("quote's").to_string_lossy()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
