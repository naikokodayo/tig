// SPDX-License-Identifier: GPL-2.0-or-later
// Safe rendering port of Tig draw.c/view.c/util.c.
// Original Tig copyright (c) 2006-2026 Jonas Fonseca.
//! Plain terminal-cell rows. Color attributes are intentionally outside this API.
//! Dates use committer_date by default and date for use-author=yes.
//! Short Git decorations do not identify slash-containing local branches;
//! supply refs/heads/ or refs/remotes/ prefixes to disambiguate them.
use crate::{
    config::Config,
    graph::Graph,
    graph_v1,
    model::{BlameLine, Commit},
};
use std::collections::BTreeMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) struct Column<'a> {
    pub(crate) name: &'a str,
    pub(crate) display: &'a str,
    pub(crate) options: BTreeMap<&'a str, &'a str>,
}
impl<'a> Column<'a> {
    pub(crate) fn parse(text: &'a str) -> Result<Self, String> {
        let (name, rest) = text.split_once(':').unwrap_or((text, "yes"));
        let mut parts = rest.split(',');
        let display = parts.next().unwrap_or("yes");
        let mut options = BTreeMap::new();
        for part in parts {
            let (key, value) = part.split_once('=').unwrap_or((part, "yes"));
            options.insert(key, value);
        }
        if !matches!(
            name,
            "id" | "date" | "author" | "committer" | "commit-title" | "line-number"
        ) {
            return Err(format!("Unsupported main-view column: {name}"));
        }
        for key in options.keys() {
            let valid = match name {
                "author" | "committer" => matches!(*key, "width" | "maxwidth"),
                "date" => matches!(*key, "width" | "format" | "local" | "use-author"),
                "id" => matches!(*key, "width" | "color"),
                "line-number" => matches!(*key, "width" | "interval"),
                "commit-title" => matches!(*key, "graph" | "refs" | "overflow"),
                _ => false,
            };
            if !valid {
                return Err(format!("Unsupported {name} column option: {key}"));
            }
        }
        Ok(Self {
            name,
            display,
            options,
        })
    }
    pub(crate) fn enabled(&self) -> bool {
        !matches!(self.display, "no" | "false" | "0")
    }
    pub(crate) fn number(&self, key: &str) -> Result<usize, String> {
        match self.options.get(key) {
            None => Ok(0),
            Some(s) => s
                .parse()
                .map_err(|_| format!("Invalid {} {key}: {s}", self.name)),
        }
    }
    pub(crate) fn flag(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.options.get(key).copied() {
            None => Ok(default),
            Some("yes" | "true" | "1") => Ok(true),
            Some("no" | "false" | "0") => Ok(false),
            Some(v) => Err(format!("Invalid {} {key}: {v}", self.name)),
        }
    }
}

/// Replace control characters before any text reaches a terminal.
pub fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Clip without splitting UTF-8 or exceeding terminal-cell width. Combining
/// characters stay attached to the preceding character; terminal width rules
/// for complex emoji sequences may differ between terminal implementations.
pub fn clip(text: &str, width: usize) -> String {
    let mut cells = 0;
    text.chars()
        .take_while(|c| {
            let size = c.width().unwrap_or(0);
            if cells + size > width {
                false
            } else {
                cells += size;
                true
            }
        })
        .collect()
}

pub(crate) fn author(
    name: &str,
    email: &str,
    display: &str,
    limit: usize,
) -> Result<String, String> {
    if display == "email" {
        return Ok(sanitize(email));
    }
    if display == "email-user" {
        return Ok(sanitize(email.split('@').next().unwrap_or(email)));
    }
    if !matches!(display, "full" | "yes" | "true" | "abbreviated") {
        return Err(format!(
            "Unsupported author mode without email metadata: {display}"
        ));
    }
    let name = sanitize(name);
    if display != "abbreviated" && (limit == 0 || limit > 10) {
        return Ok(name);
    }
    let words: Vec<&str> = name
        .split(|c: char| c.is_ascii_whitespace() || c.is_ascii_punctuation())
        .filter(|w| !w.is_empty())
        .collect();
    let mut result = String::new();
    if let Some((last, first)) = words.split_last() {
        for word in first {
            result.extend(word.chars().next());
        }
        result.push_str(last);
    }
    Ok(result)
}

pub(crate) fn date(iso: &str, column: &Column<'_>) -> Result<String, String> {
    let normalized;
    let iso = if let Some(prefix) = iso.strip_suffix('Z') {
        normalized = format!("{prefix}+00:00");
        normalized.as_str()
    } else {
        iso
    };
    if column.flag("local", false)? {
        return Err("Local date conversion is not supported".into());
    }
    // The history loader supplies strict ISO 8601 (%aI or %cI), including offset.
    let b = iso.as_bytes();
    if b.len() != 25
        || !iso.is_ascii()
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[22] != b':'
        || !matches!(b[19], b'+' | b'-')
        || [0..4, 5..7, 8..10, 11..13, 14..16, 17..19, 20..22, 23..25]
            .iter()
            .any(|r| !b[r.clone()].iter().all(u8::is_ascii_digit))
    {
        return Err(format!("Expected strict ISO 8601 commit date: {iso:?}"));
    }
    let number = |start, end| iso[start..end].parse::<u32>().unwrap_or(u32::MAX);
    let year = number(0, 4);
    let month = number(5, 7);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if number(8, 10) == 0
        || number(8, 10) > days
        || number(11, 13) > 23
        || number(14, 16) > 59
        || number(17, 19) > 60
        || number(20, 22) > 23
        || number(23, 25) > 59
    {
        return Err(format!("Invalid ISO 8601 commit date: {iso:?}"));
    }
    let zone = format!("{}{}", &iso[19..22], &iso[23..25]);
    let format = match column.display {
        "default" | "yes" | "true" => "%Y-%m-%d %H:%M %z",
        "custom" => column.options.get("format").copied().unwrap_or("%Y-%m-%d"),
        other => return Err(format!("Unsupported date mode: {other}")),
    };
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let part = match chars.next().ok_or("Trailing % in date format")? {
            '%' => "%",
            'Y' => &iso[..4],
            'y' => &iso[2..4],
            'm' => &iso[5..7],
            'd' => &iso[8..10],
            'H' => &iso[11..13],
            'M' => &iso[14..16],
            'S' => &iso[17..19],
            'z' | 'Z' => &zone,
            'F' => &iso[..10],
            'R' => &iso[11..16],
            'T' => &iso[11..19],
            spec => return Err(format!("Unsupported date format directive: %{spec}")),
        };
        out.push_str(part);
    }
    Ok(sanitize(&out))
}

fn blame_date(seconds: i64, zone: &str) -> Result<String, String> {
    let zone_bytes = zone.as_bytes();
    if zone_bytes.len() != 5
        || !matches!(zone_bytes[0], b'+' | b'-')
        || !zone_bytes[1..].iter().all(u8::is_ascii_digit)
    {
        return Err("Invalid blame timezone".into());
    }
    let hours = (zone_bytes[1] - b'0') as i64 * 10 + (zone_bytes[2] - b'0') as i64;
    let minutes = (zone_bytes[3] - b'0') as i64 * 10 + (zone_bytes[4] - b'0') as i64;
    if hours > 23 || minutes > 59 {
        return Err("Invalid blame timezone".into());
    }
    let offset = (hours * 60 + minutes) * 60 * if zone_bytes[0] == b'+' { 1 } else { -1 };
    let local = seconds.checked_add(offset).ok_or("Blame date overflow")?;
    let days = local.div_euclid(86_400);
    let time = local.rem_euclid(86_400);
    // Gregorian civil date from days since 1970-01-01.
    let z = days.checked_add(719_468).ok_or("Blame date overflow")?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    if !(0..=9999).contains(&year) {
        return Err("Blame date outside supported years".into());
    }
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{}:{}",
        time / 3_600,
        time / 60 % 60,
        time % 60,
        &zone[..3],
        &zone[3..]
    ))
}

pub fn render_blame(
    config: &Config,
    lines: &[BlameLine],
    width: usize,
) -> Result<Vec<String>, String> {
    let specs = config
        .settings
        .get("blame-view")
        .ok_or("blame-view is not configured")?;
    let show_filename = lines
        .first()
        .is_some_and(|first| lines.iter().any(|line| line.filename != first.filename));
    let ascii = config.value("line-graphics") == Some("ascii");
    let mut rows = vec![String::new(); lines.len()];
    for spec in specs {
        let (name, rest) = spec.split_once(':').unwrap_or((spec, "yes"));
        let mut parts = rest.split(',');
        let mut col = Column {
            name,
            display: parts.next().unwrap_or("yes"),
            options: parts
                .map(|part| part.split_once('=').unwrap_or((part, "yes")))
                .collect(),
        };
        let prefix = format!("blame-view-{name}-");
        for (key, values) in &config.settings {
            if let (Some(option), Some(value)) = (key.strip_prefix(&prefix), values.first()) {
                if option == "display" {
                    col.display = value;
                } else {
                    col.options.insert(option, value);
                }
            }
        }
        if !col.enabled() || (name == "file-name" && col.display == "auto" && !show_filename) {
            continue;
        }
        let fixed = col.number("width")?;
        let values: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| -> Result<String, String> {
                Ok(match name {
                    "id" => sanitize(&line.oid),
                    "file-name" => sanitize(&line.filename.to_string_lossy()),
                    "author" => author(&line.author, &line.author_email, col.display, fixed)?,
                    "committer" => {
                        author(&line.committer, &line.committer_email, col.display, fixed)?
                    }
                    "date" => date(
                        &blame_date(
                            if col.flag("use-author", false)? {
                                line.author_time
                            } else {
                                line.committer_time
                            },
                            if col.flag("use-author", false)? {
                                &line.author_tz
                            } else {
                                &line.committer_tz
                            },
                        )?,
                        &col,
                    )?,
                    "line-number" => {
                        let interval = col.number("interval")?;
                        let interval = if interval == 0 { 5 } else { interval };
                        if i == 0 || line.line % interval == 0 {
                            line.line.to_string()
                        } else {
                            String::new()
                        }
                    }
                    "text" => sanitize(&line.text),
                    _ => return Err(format!("Unsupported blame-view column: {name}")),
                })
            })
            .collect::<Result<_, _>>()?;
        let max = match col.options.get("maxwidth") {
            Some(value) if value.ends_with('%') => {
                width.saturating_mul(
                    value
                        .trim_end_matches('%')
                        .parse::<usize>()
                        .map_err(|_| "Invalid maxwidth")?,
                ) / 100
            }
            _ => col.number("maxwidth")?,
        };
        let mut cells = if fixed > 0 {
            fixed
        } else if name == "id" {
            config.usize_value("id-width", 7).max(1)
        } else {
            values.iter().map(|value| value.width()).max().unwrap_or(0)
        };
        if name == "line-number" {
            cells = cells.clamp(3, 9);
        }
        if fixed == 0 && max > 0 {
            cells = cells.min(max);
        }
        cells = cells.min(width);
        for (row, value) in rows.iter_mut().zip(&values) {
            if name == "text" {
                row.push_str(value);
                continue;
            }
            let mut clipped = clip(value, cells);
            if matches!(name, "author" | "committer" | "file-name")
                && value.width() > cells
                && (name == "file-name" || cells > 10)
            {
                let delimiter = config.value("truncation-delimiter").unwrap_or("~");
                let delimiter = sanitize(if delimiter == "utf-8" {
                    "…"
                } else {
                    delimiter
                });
                clipped = clip(value, cells.saturating_sub(delimiter.width()));
                clipped.push_str(&clip(&delimiter, cells));
            }
            let padding = " ".repeat(cells.saturating_sub(clipped.width()));
            if name == "line-number" {
                row.push_str(&padding);
                row.push_str(&clipped);
                row.push_str(if ascii { "| " } else { "│ " });
            } else {
                row.push_str(&clipped);
                row.push_str(&padding);
                row.push(' ');
            }
        }
    }
    Ok(rows)
}

pub fn refs(config: &Config, decorations: &str, separator: &str) -> String {
    let formats = config.settings.get("reference-format");
    let mut result = Vec::new();
    for item in decorations.split(", ").filter(|s| !s.is_empty()) {
        let (kind, name) = if let Some(name) = item.strip_prefix("tag: ") {
            ("tag", name.trim_start_matches("refs/tags/"))
        } else if let Some(name) = item.strip_prefix("HEAD -> ") {
            ("head", name.trim_start_matches("refs/heads/"))
        } else if item.starts_with("refs/remotes/") {
            ("remote", item.trim_start_matches("refs/remotes/"))
        } else if item.contains('/') && !item.starts_with("refs/heads/") {
            ("remote", item)
        } else {
            ("branch", item.trim_start_matches("refs/heads/"))
        };
        let selected = formats
            .and_then(|f| f.iter().find(|f| f.contains(kind)))
            .or_else(|| formats.and_then(|f| f.iter().find(|f| f.contains("branch"))));
        let rendered = if let Some(format) = selected {
            if format.starts_with("hide:") {
                continue;
            }
            let key = if format.contains(kind) {
                kind
            } else {
                "branch"
            };
            format.replacen(key, name, 1)
        } else {
            match kind {
                "tag" => format!("<{name}>"),
                "remote" => format!("{{{name}}}"),
                _ => format!("[{name}]"),
            }
        };
        result.push(sanitize(&rendered));
    }
    result.join(separator)
}

fn main_columns(config: &Config) -> Result<Vec<Column<'_>>, String> {
    let specs = config
        .settings
        .get("main-view")
        .ok_or("main-view is not configured")?;
    let mut columns: Vec<_> = specs
        .iter()
        .map(|s| Column::parse(s))
        .collect::<Result<_, _>>()?;
    for col in &mut columns {
        let prefix = format!("main-view-{}-", col.name);
        for (key, values) in &config.settings {
            if let (Some(option), Some(value)) = (key.strip_prefix(&prefix), values.first()) {
                if option == "display" {
                    col.display = value;
                } else {
                    col.options.insert(option, value);
                }
            }
        }
    }
    Ok(columns.into_iter().filter(Column::enabled).collect())
}

pub fn main_refs_searchable(config: &Config) -> bool {
    main_columns(config).is_ok_and(|columns| {
        columns
            .iter()
            .any(|column| column.name == "commit-title" && column.flag("refs", false) == Ok(true))
    })
}

/// Render a complete main-view commit list, so autosized columns see all rows.
/// The plain string result cannot represent Tig color/overflow attributes.
pub fn render_commits(
    config: &Config,
    commits: &[Commit],
    width: usize,
) -> Result<Vec<String>, String> {
    let columns = main_columns(config)?;
    let mut graph = Graph::new();
    let ascii = config.value("line-graphics") == Some("ascii");
    let mut canvases = Vec::with_capacity(commits.len());
    let needs_v1 = columns
        .iter()
        .any(|column| column.options.get("graph") == Some(&"v1"));
    let mut graph_v1 = graph_v1::Graph::new();
    let mut canvases_v1 = Vec::new();
    for commit in commits {
        let parents: Vec<_> = commit.parents.iter().map(String::as_str).collect();
        canvases.push(graph.render_commit(&commit.oid, &parents, false));
        if needs_v1 {
            canvases_v1.push(graph_v1.render_commit(&commit.oid, &parents, false));
        }
    }
    let mut fields = Vec::new();
    let mut sizes = Vec::new();
    for col in &columns {
        let fixed = col.number("width")?;
        let max = match col.options.get("maxwidth") {
            Some(value) if value.ends_with('%') => {
                let percent = value
                    .trim_end_matches('%')
                    .parse::<usize>()
                    .map_err(|_| "Invalid maxwidth percentage")?;
                if percent > 100 {
                    return Err("maxwidth percentage exceeds 100".into());
                }
                width.saturating_mul(percent) / 100
            }
            _ => col.number("maxwidth")?,
        };
        let values: Vec<String> = commits
            .iter()
            .enumerate()
            .map(|(i, c)| match col.name {
                "id" => Ok(sanitize(&c.oid)),
                "date" => date(
                    if col.flag("use-author", false)? {
                        &c.date
                    } else {
                        &c.committer_date
                    },
                    col,
                ),
                "author" => author(&c.author, &c.author_email, col.display, fixed.max(max)),
                "committer" => author(
                    &c.committer,
                    &c.committer_email,
                    col.display,
                    fixed.max(max),
                ),
                "line-number" => {
                    let interval = col.number("interval")?;
                    let interval = if interval == 0 { 5 } else { interval };
                    Ok(if i == 0 || (i + 1) % interval == 0 {
                        (i + 1).to_string()
                    } else {
                        String::new()
                    })
                }
                "commit-title" => {
                    let mut text = String::new();
                    match col.options.get("graph").copied().unwrap_or("no") {
                        "yes" | "true" | "1" | "v2" => {
                            text.push_str(&graph_text(canvases[i].render(ascii), config));
                            text.push(' ');
                        }
                        "v1" => {
                            text.push_str(&graph_text(canvases_v1[i].render(ascii), config));
                            text.push(' ');
                        }
                        "no" | "false" | "0" => {}
                        other => return Err(format!("Unsupported graph renderer: {other}")),
                    }
                    if col.flag("refs", false)? {
                        let refs = refs(config, &c.decorations, " ");
                        if !refs.is_empty() {
                            text.push_str(&refs);
                            text.push(' ');
                        }
                    }
                    text.push_str(&sanitize(&c.subject));
                    Ok(text)
                }
                _ => unreachable!(),
            })
            .collect::<Result<_, String>>()?;
        let mut size = if fixed > 0 {
            fixed
        } else if col.name == "id" {
            config.usize_value("id-width", 7).max(1)
        } else {
            values.iter().map(|s| s.width()).max().unwrap_or(0)
        };
        if fixed == 0 && max > 0 {
            size = size.min(max);
        }
        if col.name == "line-number" {
            size = size.clamp(3, 9);
        }
        sizes.push(size.min(width));
        fields.push(values);
    }
    let mut rows = Vec::with_capacity(commits.len());
    for i in 0..commits.len() {
        let mut row = String::new();
        for (index, col) in columns.iter().enumerate() {
            let value = &fields[index][i];
            if col.name == "commit-title" {
                row.push_str(value);
                continue;
            }
            let size = sizes[index];
            let mut value = clip(value, size);
            if matches!(col.name, "author" | "committer")
                && size > 10
                && fields[index][i].width() > size
            {
                let delimiter = match config.value("truncation-delimiter").unwrap_or("~") {
                    "utf-8" => "…",
                    s => s,
                };
                let delimiter = sanitize(delimiter);
                value = clip(&value, size.saturating_sub(delimiter.width()));
                value.push_str(&clip(&delimiter, size));
            }
            let padding = size.saturating_sub(value.width());
            if col.name == "line-number" {
                row.push_str(&" ".repeat(padding));
                row.push_str(&value);
                row.push_str(if ascii { "| " } else { "│ " });
            } else {
                row.push_str(&value);
                row.push_str(&" ".repeat(padding));
                row.push(' ');
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

// Curses uses ASCII commit markers with line-drawing edges in its default mode.
fn graph_text(text: String, config: &Config) -> String {
    if matches!(config.value("line-graphics"), Some("ascii" | "utf-8")) {
        text
    } else {
        text.chars()
            .map(|c| match c {
                '◯' | '∙' => 'o',
                '◎' => 'I',
                '●' => 'M',
                other => other,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blame_uses_configured_dates_columns_and_rename_paths() {
        let mut config = Config::defaults();
        config.parse("set line-graphics = ascii\nset blame-view-date-use-author = yes");
        let mut first = BlameLine {
            oid: "a".repeat(40),
            original_line: 1,
            line: 1,
            author: "Author".into(),
            author_email: "a@example.test".into(),
            author_time: 0,
            author_tz: "-0200".into(),
            committer: "Committer".into(),
            committer_email: "c@example.test".into(),
            committer_time: 3600,
            committer_tz: "+0100".into(),
            filename: "old/name".into(),
            summary: String::new(),
            text: "first".into(),
        };
        let rows = render_blame(&config, &[first.clone()], 200).unwrap();
        assert_eq!(rows[0], "aaaaaaa Author 1969-12-31 22:00 -0200   1| first");
        config.parse("set blame-view-date-use-author = no");
        assert_eq!(
            render_blame(&config, &[first.clone()], 200).unwrap()[0],
            "aaaaaaa Author 1970-01-01 02:00 +0100   1| first"
        );
        config.parse("set blame-view-date-use-author = yes");
        first.filename = "new/name".into();
        first.line = 2;
        let rows = render_blame(
            &config,
            &[
                first.clone(),
                BlameLine {
                    filename: "old/name".into(),
                    line: 3,
                    ..first.clone()
                },
            ],
            200,
        )
        .unwrap();
        assert!(rows[0].starts_with("aaaaaaa new/name "));
        assert!(rows[1].starts_with("aaaaaaa old/name "));
        config.parse("set blame-view = text");
        assert_eq!(
            render_blame(
                &config,
                &[BlameLine {
                    filename: "x".into(),
                    line: 4,
                    ..first
                }],
                80
            )
            .unwrap(),
            ["first"]
        );
    }
    fn commit() -> Commit {
        Commit {
            oid: "ee912870202200a0b9cf4fd86ba57243212d341e".into(),
            parents: vec!["parent".into()],
            author: "Jonas Fonseca".into(),
            author_email: "jonas@example.com".into(),
            committer: "Jonas Fonseca".into(),
            committer_email: "jonas@example.com".into(),
            committer_date: "2014-03-01T17:26:00-05:00".into(),
            date: "2014-03-01T17:26:00-05:00".into(),
            subject: "WIP: Upgrade".into(),
            decorations: "HEAD -> master".into(),
        }
    }
    #[test]
    fn auto_graphics_respects_locale_precedence() {
        const CHILD: &str = "TIG_TEST_AUTO_GRAPHICS_EXPECTED";
        if let Ok(expected) = std::env::var(CHILD) {
            let mut config = Config::defaults();
            config.apply_command("set line-graphics = auto").unwrap();
            assert_eq!(config.value("line-graphics"), Some(expected.as_str()));
            assert_eq!(
                graph_text("∙◎●◯".into(), &config),
                if expected == "utf-8" {
                    "∙◎●◯"
                } else {
                    "oIMo"
                }
            );
            config.apply_command("toggle line-graphics").unwrap();
            assert_eq!(
                config.value("line-graphics"),
                Some(if expected == "utf-8" {
                    "ascii"
                } else {
                    "utf-8"
                })
            );
            return;
        }
        // Separate processes avoid racing other tests through global locale state.
        for (all, ctype, lang, expected) in [
            ("C", "en_US.UTF-8", "en_US.UTF-8", "default"),
            ("", "en_US.utf8", "C", "utf-8"),
            ("", "C", "en_US.UTF-8", "default"),
            ("", "", "en_US.UTF-8", "utf-8"),
            ("", "", "en_US.Utf8", "default"),
            ("", "", "", "default"),
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "render::tests::auto_graphics_respects_locale_precedence",
                ])
                .env(CHILD, expected)
                .env("LC_ALL", all)
                .env("LC_CTYPE", ctype)
                .env("LANG", lang)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
    #[test]
    fn upstream_width_fixture() {
        let mut config = Config::defaults();
        config.parse("set line-graphics = ascii\nset main-view = id:yes,width=5 line-number:yes,interval=5,width=5 date:default,width=5 author:full,width=5 commit-title:yes,graph,refs,overflow=no");
        assert_eq!(
            render_commits(&config, &[commit()], 200).unwrap()[0],
            "ee912     1| 2014- JFons * [master] WIP: Upgrade"
        );
        config.parse(
            "set main-view = date:default,format=%Y author:full commit-title:yes,graph=no,refs=no",
        );
        assert_eq!(
            render_commits(&config, &[commit()], 200).unwrap()[0],
            "2014-03-01 17:26 -0500 Jonas Fonseca WIP: Upgrade"
        );
        config.parse("set main-view = date:custom,format=%F/%R/%z commit-title");
        assert_eq!(
            render_commits(&config, &[commit()], 200).unwrap()[0],
            "2014-03-01/17:26/-0500 WIP: Upgrade"
        );
        config.parse("set main-view = date:custom,format=%Q commit-title");
        assert!(render_commits(&config, &[commit()], 200).is_err());
    }
    #[test]
    fn unicode_padding_clipping_and_controls() {
        let mut config = Config::defaults();
        config.parse("set main-view = author:full,maxwidth=4 commit-title:yes,graph=no,refs=no");
        let mut a = commit();
        a.author = "作者".into();
        a.subject = "a\x1b[31m\n\t終".into();
        let row = &render_commits(&config, &[a], 12).unwrap()[0];
        assert_eq!(row, "作者 a [31m  終");
        assert!(!row.chars().any(char::is_control));
        assert!(clip(row, 12).width() <= 12);
        assert_eq!(clip("a界b", 2), "a");
        assert_eq!(clip("e\u{301}界", 1), "e\u{301}");
    }
    #[test]
    fn dates_email_and_overrides() {
        let mut config = Config::defaults();
        config.parse(
            "set main-view = date:custom,format=%F author:email-user committer:email commit-title",
        );
        let mut c = commit();
        c.committer_date = "2020-02-29T00:00:00Z".into();
        assert_eq!(
            render_commits(&config, &[c.clone()], 100).unwrap()[0],
            "2020-02-29 jonas jonas@example.com WIP: Upgrade"
        );
        config
            .settings
            .insert("main-view-date-use-author".into(), vec!["yes".into()]);
        assert!(render_commits(&config, &[c.clone()], 100).unwrap()[0].starts_with("2014-03-01"));
        c.date = "2023-02-29T00:00:00+00:00".into();
        assert!(render_commits(&config, &[c], 100).is_err());
    }
    #[test]
    fn reference_formats_and_graph_switches() {
        let mut config = Config::defaults();
        config.parse("set line-graphics = ascii\nset main-view = commit-title:yes,graph,refs\nset reference-format = (branch) [tag] hide:remote");
        assert!(main_refs_searchable(&config));
        let mut c = commit();
        c.decorations =
            "HEAD -> refs/heads/master, tag: refs/tags/v1.0, refs/remotes/origin/master".into();
        assert_eq!(
            render_commits(&config, &[c.clone()], 100).unwrap()[0],
            "* (master) [v1.0] WIP: Upgrade"
        );
        config.parse("set main-view = commit-title:yes,graph=no,refs=no");
        assert!(!main_refs_searchable(&config));
        assert_eq!(
            render_commits(&config, &[c], 100).unwrap()[0],
            "WIP: Upgrade"
        );
    }
}
