// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Safe Rust migration of Tig. SPDX-License-Identifier: GPL-2.0-or-later
//! C `save-view` diagnostics, not a dump of rendered screen columns.
use std::{fmt::Write as _, fs::OpenOptions, io, io::Write as _, path::Path};

pub fn header(
    name: &str,
    previous: Option<&str>,
    parent: Option<&str>,
    revision: &str,
    (width, height): (usize, usize),
    (offset, column, lineno): (usize, usize, usize),
) -> String {
    let mut output = format!("View: {name}\n");
    if let Some(previous) = previous {
        writeln!(output, "Prev: {previous}").unwrap();
    }
    if let Some(parent) = parent {
        writeln!(output, "Parent: {parent}").unwrap();
    }
    write!(output, "Ref: {revision}\nDimensions: height={height} width={width}\nPosition: offset={offset} column={column} lineno={lineno}\n").unwrap();
    output
}

/// Only box-backed C rows carry cells; structured views export their types.
pub fn line(output: &mut String, index: usize, kind: &str, selected: bool, cells: Option<&[&str]>) {
    writeln!(
        output,
        "line[{index:3}] type={kind} selected={}",
        usize::from(selected)
    )
    .unwrap();
    if let Some(cells) = cells {
        write!(output, "line[{index:3}] cells={} text=", cells.len()).unwrap();
        for cell in cells {
            write!(output, "[{cell}]").unwrap();
        }
        output.push('\n');
    }
}

/// Unlike diff/pager, C log_read retains literal chunk cells and graph prefixes.
pub fn log_data(rows: &[String], selected: usize) -> String {
    let mut output = String::new();
    let mut graph_indent = 0;
    let mut commit_header = false;
    let mut after_header = false;
    let mut reading_stat = false;
    for (index, row) in rows.iter().enumerate() {
        let indent = row
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'|' | b'/' | b'\\' | b'*' | b'_'))
            .count();
        if row[indent..].starts_with("commit ") {
            graph_indent = indent;
        }
        let text = row.get(graph_indent..).unwrap_or("");
        let mut kind = crate::line::builtin_line_type(text);
        let empty = row.is_empty() || row.len() == graph_indent;
        let mut cells = None;
        if kind == "commit" {
            commit_header = true;
        } else if commit_header && empty {
            commit_header = false;
            after_header = true;
        } else if (after_header && empty) || kind == "diff-start" {
            after_header = false;
            reading_stat = true;
        } else if reading_stat {
            cells = crate::render::diff_stat_cells(row);
            if cells.is_some() {
                kind = "diff-stat";
            } else {
                reading_stat = false;
            }
        }
        let cells = cells.unwrap_or_else(|| vec![row.as_str()]);
        line(&mut output, index, kind, index == selected, Some(&cells));
    }
    output
}

/// Never truncate an existing destination or follow its final symlink.
/// This intentionally differs from C's unconditional fopen(path, "w").
pub fn save(path: &Path, data: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(data.as_bytes())?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_cells_follow_log_read_not_diff_read() {
        let rows = [
            "commit abc",
            "",
            "    title",
            "",
            " file | 1 +",
            "",
            "@@ -1 +1 @@",
            "--- text",
        ]
        .map(str::to_owned);
        let data = log_data(&rows, 4);
        assert!(data.contains(
            "line[  4] type=diff-stat selected=1\nline[  4] cells=3 text=[ file ][| 1 ][+]"
        ));
        assert!(data.contains("line[  6] cells=1 text=[@@ -1 +1 @@]"));
        assert!(data.contains("line[  7] type=diff-del-file"));
        let graph = ["* commit abc", "| Author: Name", "|", "|     title"].map(str::to_owned);
        let data = log_data(&graph, 0);
        assert!(data.contains("line[  0] type=commit selected=1"));
        assert!(data.contains("line[  1] type= selected=0"));
        assert!(data.contains("line[  2] cells=1 text=[|]"));
    }

    #[test]
    fn diagnostic_format_and_exclusive_destination() {
        let mut data = header(
            "blob",
            Some("tree"),
            Some("tree"),
            "HEAD",
            (80, 14),
            (2, 3, 4),
        );
        line(&mut data, 0, "default", false, Some(&["名\t[content]"]));
        line(&mut data, 1, "directory", true, None);
        assert_eq!(data, "View: blob\nPrev: tree\nParent: tree\nRef: HEAD\nDimensions: height=14 width=80\nPosition: offset=2 column=3 lineno=4\nline[  0] type=default selected=0\nline[  0] cells=1 text=[名\t[content]]\nline[  1] type=directory selected=1\n");
        let root = std::env::temp_dir().join(format!("tig-view-export-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("file with spaces");
        save(&path, &data).unwrap();
        assert_eq!(
            save(&path, "replacement").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), data);
        assert!(save(&root, "directory").is_err());
        assert!(save(&root.join("missing/file"), "missing parent").is_err());
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(save(&link, "symlink").is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), data);
            let dangling = root.join("dangling");
            std::os::unix::fs::symlink(root.join("absent"), &dangling).unwrap();
            assert!(save(&dangling, "dangling").is_err());
            assert!(!root.join("absent").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
