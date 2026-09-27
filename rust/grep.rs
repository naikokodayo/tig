// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Safe Rust migration of Tig. SPDX-License-Identifier: GPL-2.0-or-later
use crate::config::Config;
use std::collections::VecDeque;
use std::path::{Component, PathBuf};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone)]
pub struct GrepLine {
    pub label: String,
    pub path: PathBuf,
    pub revision: Option<String>,
    pub cached: bool,
    pub line: usize,
    pub text: String,
}

pub fn safe_grep_path(path: &std::path::Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

#[derive(Default, Debug)]
pub(crate) struct GrepOptions {
    pub args: Vec<String>,
    pub operands: Vec<String>,
    pub before: usize,
    pub after: usize,
    pub cached: bool,
}

impl GrepOptions {
    /// Keep Git's matching language, but own context expansion so a filename
    /// starting with "--\n" can never be confused with Git's group separator.
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut options = Self::default();
        let mut pattern_seen = false;
        let mut positional = false;
        let mut only_matching = false;
        let mut args: VecDeque<_> = args.iter().cloned().collect();
        while let Some(mut arg) = args.pop_front() {
            // Peel only argument-free short flags. Value-taking flags retain the
            // entire suffix (including leading dashes or Unicode) as their value.
            if !positional
                && arg.len() > 2
                && arg.starts_with('-')
                && arg.as_bytes()[2] != b'-'
                && matches!(
                    arg.as_bytes()[1],
                    b'i' | b'w'
                        | b'v'
                        | b'F'
                        | b'E'
                        | b'P'
                        | b'G'
                        | b'a'
                        | b'I'
                        | b'o'
                        | b'r'
                        | b'n'
                        | b'H'
                        | b'z'
                )
            {
                args.push_front(format!("-{}", &arg[2..]));
                arg.truncate(2);
            }
            if arg == "--" {
                if !pattern_seen {
                    return Err(
                        "Git grep '--' before a pattern is not supported in the Rust view".into(),
                    );
                }
                options.args.push(arg.clone());
                options.args.extend(args);
                break;
            }
            if matches!(arg.as_str(), "-e" | "--regexp" | "-f" | "--file") && !positional {
                let value = args
                    .pop_front()
                    .ok_or("Missing Git grep pattern or pattern file")?;
                let flag = if arg == "--regexp" {
                    "-e"
                } else if arg == "--file" {
                    "-f"
                } else {
                    &arg
                };
                options.args.extend([flag.into(), value]);
                pattern_seen = true;
            } else if !positional
                && (arg.starts_with("-e")
                    || arg.starts_with("--regexp=")
                    || arg.starts_with("-f")
                    || arg.starts_with("--file="))
            {
                if let Some(value) = arg.strip_prefix("--regexp=") {
                    options.args.extend(["-e".into(), value.into()]);
                } else if let Some(value) = arg.strip_prefix("--file=") {
                    options.args.extend(["-f".into(), value.into()]);
                } else {
                    options.args.push(arg.clone());
                }
                pattern_seen = true;
            } else if !positional && matches!(arg.as_str(), "--and" | "--or" | "--not" | "(" | ")")
            {
                options.args.push(arg.clone());
            } else if let Some(short) = arg.strip_prefix('-') {
                if positional {
                    return Err(format!(
                        "Git grep option '{arg}' must precede positional arguments"
                    )
                    .into());
                }
                let (name, inline) = if let Some((name, value)) = arg.split_once('=') {
                    (name, Some(value))
                } else if ["-A", "-B", "-C", "-m"]
                    .iter()
                    .any(|prefix| arg.starts_with(prefix))
                {
                    (&arg[..2], (arg.len() > 2).then_some(&arg[2..]))
                } else if !short.is_empty() && short.bytes().all(|b| b.is_ascii_digit()) {
                    ("-C", Some(short))
                } else {
                    (arg.as_str(), None)
                };
                match name {
                    "-A" | "-B" | "-C" | "--after-context" | "--before-context" | "--context"
                    | "-m" | "--max-count" | "--max-depth" | "--threads" => {
                        let value = inline
                            .map(str::to_owned)
                            .or_else(|| args.pop_front())
                            .ok_or("Missing Git grep numeric option value")?;
                        if matches!(name, "-m" | "--max-count" | "--max-depth" | "--threads") {
                            // Git validates its own numeric options, including --max-depth=-1.
                            options.args.extend([name.into(), value]);
                            continue;
                        }
                        let count = value
                            .parse::<usize>()
                            .map_err(|_| "Invalid Git grep numeric option value")?;
                        match name {
                            "-A" | "--after-context" => options.after = count,
                            "-B" | "--before-context" => options.before = count,
                            "-C" | "--context" => {
                                options.before = count;
                                options.after = count;
                            }
                            _ => unreachable!("context option"),
                        }
                    }
                    "--cached" => {
                        options.cached = true;
                        options.args.push(arg.clone());
                    }
                    "--no-cached" => {
                        options.cached = false;
                        options.args.push(arg.clone());
                    }
                    "-i"
                    | "--ignore-case"
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
                    | "--all-match"
                    | "-a"
                    | "--text"
                    | "-I"
                    | "-o"
                    | "--only-matching"
                    | "--untracked"
                    | "--no-untracked"
                    | "--exclude-standard"
                    | "--no-exclude-standard"
                    | "-r"
                    | "--recursive"
                    | "--no-recursive"
                    | "-n"
                    | "--line-number"
                    | "-H"
                    | "-z"
                    | "--null"
                    | "--full-name"
                        if inline.is_none() =>
                    {
                        only_matching |= matches!(name, "-o" | "--only-matching");
                        options.args.push(arg.clone())
                    }
                    _ => {
                        return Err(format!(
                            "Git grep option '{arg}' is not supported in the Rust view"
                        )
                        .into())
                    }
                }
            } else {
                positional = true;
                options.args.push(arg.clone());
                if pattern_seen {
                    options.operands.push(arg.clone());
                } else {
                    pattern_seen = true;
                }
            }
        }
        if (options.before > 0 || options.after > 0) && only_matching {
            return Err(
                "Git grep context with --only-matching is not supported in the Rust view".into(),
            );
        }
        Ok(options)
    }
}

pub(crate) fn grep_rows(bytes: &[u8], revisions: &[String]) -> Result<Vec<GrepLine>> {
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
            cached: false,
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

fn grep_filename(label: &str, width: usize, config: &Config) -> String {
    let text = crate::render::trim_field(&crate::render::sanitize(label), width, config);
    format!(
        "{text}{}",
        " ".repeat(width.saturating_sub(crate::render::cell_width(&text)))
    )
}

/// Merge overlapping ranges, preserving source line numbers and lossless paths.
pub(crate) fn context_rows(
    hits: &[GrepLine],
    content: &[u8],
    before: usize,
    after: usize,
) -> Result<Vec<Option<GrepLine>>> {
    let mut lines: Vec<_> = content.split(|&byte| byte == b'\n').collect();
    if content.ends_with(b"\n") {
        lines.pop();
    }
    let mut rows = Vec::new();
    let mut previous_end: usize = 0;
    for hit in hits {
        if hit.line == 0 || hit.line > lines.len() {
            return Err("Grep source changed while loading context; refresh the view".into());
        }
        let start = hit.line.saturating_sub(before).max(1);
        let end = hit.line.saturating_add(after).min(lines.len());
        if !rows.is_empty() && start > previous_end.saturating_add(1) {
            rows.push(None);
        }
        for line in start.max(previous_end + 1)..=end {
            let mut row = hit.clone();
            row.line = line;
            row.text = String::from_utf8_lossy(lines[line - 1]).into_owned();
            rows.push(Some(row));
        }
        previous_end = previous_end.max(end);
    }
    Ok(rows)
}

pub fn render_rows(
    hits: Vec<Option<GrepLine>>,
    config: &Config,
) -> Vec<(String, Option<GrepLine>, &'static str)> {
    let mut rows = Vec::new();
    let (show_file, width, maxwidth, show_line, interval) = grep_columns(config);
    let width = width
        .unwrap_or_else(|| {
            hits.iter()
                .flatten()
                .map(|hit| crate::render::cell_width(&crate::render::sanitize(&hit.label)))
                .max()
                .unwrap_or(0)
        })
        .min(maxwidth.unwrap_or(usize::MAX));
    let line_width = hits
        .iter()
        .flatten()
        .map(|hit| hit.line.to_string().len())
        .max()
        .unwrap_or(3)
        .max(3);
    let mut last_file = None;
    for entry in hits {
        let Some(hit) = entry else {
            rows.push(("--".into(), None, "delimiter"));
            continue;
        };
        if !show_file && last_file.as_deref() != Some(hit.label.as_str()) {
            let mut header = hit.clone();
            header.line = 1;
            header.text.clear();
            rows.push((hit.label.clone(), Some(header), "file"));
        }
        let mut row = String::new();
        if show_file {
            row.push_str(&grep_filename(&hit.label, width, config));
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
        rows.push((row, Some(hit), "default"));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grep_filename_uses_cells_and_the_configured_delimiter() {
        let mut config = Config::defaults();
        config
            .apply_command("set truncation-delimiter = _")
            .unwrap();
        assert_eq!(grep_filename("LICENSE", 5, &config), "LICE_");
        assert_eq!(grep_filename("作者名", 5, &config), "作者_");
        assert_eq!(grep_filename("作者名", 4, &config), "作_ ");
        assert_eq!(grep_filename("e\u{301}界", 4, &config), "e\u{301}界 ");
        assert_eq!(grep_filename("filename", 0, &config), "");
    }

    #[test]
    fn patterns_options_and_paths_have_distinct_roles() {
        for (args, expected) in [
            (vec!["-i", "needle", "HEAD", "--", "-file"], vec!["HEAD"]),
            (vec!["-e", "needle", "-e", "HEAD"], vec![]),
            (vec!["-f", "HEAD", "HEAD:sub"], vec!["HEAD:sub"]),
            (vec!["--regexp=HEAD", "HEAD"], vec!["HEAD"]),
            (
                vec!["-m", "2", "-C3", "-A", "1", "needle", "HEAD"],
                vec!["HEAD"],
            ),
            (vec!["-e", "--", "HEAD"], vec!["HEAD"]),
            (vec!["--max-depth=-1", "needle", "HEAD"], vec!["HEAD"]),
            (
                vec!["-e", "one", "--and", "--not", "-e", "two", "HEAD"],
                vec!["HEAD"],
            ),
        ] {
            let args: Vec<_> = args.into_iter().map(String::from).collect();
            assert_eq!(GrepOptions::parse(&args).unwrap().operands, expected);
        }
        for args in [
            vec!["--", "needle"],
            vec!["-C", "-1", "needle"],
            vec!["-e"],
            vec!["--heading", "needle"],
            vec!["needle", "-i"],
            vec!["--no-index", "needle"],
            vec!["-o", "-C1", "needle"],
        ] {
            assert!(
                GrepOptions::parse(&args.into_iter().map(String::from).collect::<Vec<_>>())
                    .is_err()
            );
        }
    }

    #[test]
    fn short_clusters_preserve_values_and_boundaries() {
        for (cluster, expanded) in [
            (
                vec!["-inwF", "needle"],
                vec!["-i", "-n", "-w", "-F", "needle"],
            ),
            (vec!["-ivG", "needle"], vec!["-i", "-v", "-G", "needle"]),
            (
                vec!["-aIEPrHzo", "needle"],
                vec!["-a", "-I", "-E", "-P", "-r", "-H", "-z", "-o", "needle"],
            ),
            (vec!["-ine作者", "HEAD"], vec!["-i", "-n", "-e作者", "HEAD"]),
            (
                vec!["-ine", "-iv", "HEAD"],
                vec!["-i", "-n", "-e", "-iv", "HEAD"],
            ),
            (vec!["-ife", "HEAD"], vec!["-i", "-fe", "HEAD"]),
            (vec!["-if", "-in", "HEAD"], vec!["-i", "-f", "-in", "HEAD"]),
            (vec!["-inC2", "needle"], vec!["-i", "-n", "-C2", "needle"]),
            (vec!["-iA", "2", "needle"], vec!["-i", "-A", "2", "needle"]),
            (vec!["-iB2", "needle"], vec!["-i", "-B2", "needle"]),
            (vec!["-im1", "needle"], vec!["-i", "-m1", "needle"]),
            (vec!["-in2", "needle"], vec!["-i", "-n", "-2", "needle"]),
            (vec!["-ie--", "--", "-in"], vec!["-i", "-e--", "--", "-in"]),
        ] {
            let parse = |args: Vec<&str>| {
                GrepOptions::parse(&args.into_iter().map(String::from).collect::<Vec<_>>()).unwrap()
            };
            let actual = parse(cluster);
            let expected = parse(expanded);
            assert_eq!(actual.args, expected.args);
            assert_eq!(actual.operands, expected.operands);
            assert_eq!(
                (actual.before, actual.after),
                (expected.before, expected.after)
            );
        }
        for args in [
            vec!["-ie"],
            vec!["-if"],
            vec!["-iC"],
            vec!["-ic", "needle"],
            vec!["-il", "needle"],
            vec!["-iq", "needle"],
            vec!["-i--cached", "needle"],
            vec!["-i-", "needle"],
            vec!["-i作者", "needle"],
            vec!["-ioC1", "needle"],
            vec!["needle", "-in"],
        ] {
            assert!(
                GrepOptions::parse(&args.into_iter().map(String::from).collect::<Vec<_>>())
                    .is_err()
            );
        }
    }

    #[test]
    fn context_merges_ranges_and_keeps_separator_like_paths() {
        let hits = grep_rows(
            b"--\nfile\x002\0two\n--\nfile\x003\0three\n--\nfile\x008\0eight\n",
            &[],
        )
        .unwrap();
        let rows = context_rows(
            &hits,
            b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine",
            1,
            1,
        )
        .unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.as_ref().map(|hit| hit.line))
                .collect::<Vec<_>>(),
            vec![
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                None,
                Some(7),
                Some(8),
                Some(9)
            ]
        );
        assert!(rows
            .iter()
            .flatten()
            .all(|hit| hit.path == PathBuf::from("--\nfile")));
        assert!(context_rows(&hits, b"short", 1, 1).is_err());
    }

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
}
