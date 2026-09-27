// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Safe Rust port of Tig tree.c. SPDX-License-Identifier: GPL-2.0-or-later
use crate::{
    config::Config,
    git::{self, GitError, Repository},
    model::{Commit, TreeEntry},
    render::{self, Column},
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct TreeRow {
    pub text: String,
    /// None for the directory header; the parent link has kind=tree and path=parent.
    pub entry: Option<TreeEntry>,
    /// Last change, distinct from the selected tree/blob object's entry.oid.
    pub commit: Option<String>,
}
#[derive(Clone)]
struct Entry {
    tree: TreeEntry,
    history: Option<Commit>,
    timestamps: (i64, i64),
    parent: bool,
}

fn filename(path: &[u8]) -> git::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(path.to_vec()).into())
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(path.to_vec())
            .map(PathBuf::from)
            .map_err(|_| GitError("Unrepresentable Git filename".into()))
    }
}

fn annotate(
    entries: &mut [Entry],
    directory: &Path,
    recursive: bool,
    bytes: &[u8],
) -> git::Result<()> {
    let indices: BTreeMap<_, _> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.parent)
        .map(|(i, e)| (e.tree.path.clone(), i))
        .collect();
    let fields: Vec<_> = bytes.split(|b| *b == 0).collect();
    let mut cursor = 0;
    let mut commit = None;
    let mut timestamps = (0, 0);
    while cursor < fields.len() {
        let token = fields[cursor].strip_prefix(b"\n").unwrap_or(fields[cursor]);
        cursor += 1;
        if token.is_empty() {
            continue;
        }
        if token.starts_with(b":") {
            let renamed = token
                .split(|b| *b == b' ')
                .next_back()
                .is_some_and(|s| matches!(s.first(), Some(b'R' | b'C')));
            if cursor + usize::from(renamed) >= fields.len() {
                return Err(GitError("Truncated tree history path".into()));
            }
            let path = filename(fields[cursor + usize::from(renamed)])?;
            cursor += 1 + usize::from(renamed);
            let relative = path.strip_prefix(directory).unwrap_or(&path);
            let key = if recursive {
                path.clone()
            } else {
                relative
                    .components()
                    .next()
                    .map(|part| directory.join(part.as_os_str()))
                    .unwrap_or(path)
            };
            if let Some(&index) = indices.get(&key) {
                if entries[index].history.is_none() {
                    entries[index].history = commit.clone();
                    entries[index].timestamps = timestamps;
                }
            }
        } else {
            if !matches!(token.len(), 40 | 64)
                || !token.iter().all(u8::is_ascii_hexdigit)
                || cursor + 8 > fields.len()
            {
                return Err(GitError("Malformed tree history commit".into()));
            }
            let text = |n| String::from_utf8_lossy(fields[cursor + n]).into_owned();
            commit = Some(Commit {
                oid: String::from_utf8_lossy(token).into_owned(),
                boundary: false,
                annotated: false,
                parents: Vec::new(),
                author: text(0),
                author_email: text(1),
                date: text(2),
                committer: text(3),
                committer_email: text(4),
                committer_date: text(5),
                subject: String::new(),
                decorations: String::new(),
            });
            timestamps = (
                text(6)
                    .parse()
                    .map_err(|_| GitError("Invalid author timestamp".into()))?,
                text(7)
                    .parse()
                    .map_err(|_| GitError("Invalid committer timestamp".into()))?,
            );
            cursor += 8;
        }
    }
    Ok(())
}

fn columns(config: &Config) -> git::Result<Vec<Column<'_>>> {
    let specs = config
        .settings
        .get("tree-view")
        .ok_or_else(|| GitError("tree-view is not configured".into()))?;
    let mut columns = Vec::new();
    for spec in specs {
        let (name, rest) = spec.split_once(':').unwrap_or((spec, "yes"));
        if !matches!(
            name,
            "mode"
                | "author"
                | "committer"
                | "file-size"
                | "date"
                | "id"
                | "file-name"
                | "line-number"
        ) {
            return Err(GitError(format!("Unsupported tree column {name}")));
        }
        let mut parts = rest.split(',');
        let mut col = Column {
            name,
            display: parts.next().unwrap_or("yes"),
            options: parts
                .map(|p| p.split_once('=').unwrap_or((p, "yes")))
                .collect(),
        };
        let prefix = format!("tree-view-{name}-");
        for (key, values) in &config.settings {
            if let (Some(option), Some(value)) = (key.strip_prefix(&prefix), values.first()) {
                if option == "display" {
                    col.display = value;
                } else {
                    col.options.insert(option, value);
                }
            }
        }
        columns.push(col);
    }
    Ok(columns)
}

fn mode(mode: &str) -> &'static str {
    match mode {
        "040000" | "40000" => "drwxr-xr-x",
        "120000" => "lrwxrwxrwx",
        "160000" => "m---------",
        "100755" => "-rwxr-xr-x",
        "100644" => "-rw-r--r--",
        _ => "----------",
    }
}
fn size(size: u64, units: bool) -> String {
    if !units {
        return size.to_string();
    }
    let mut value = size as f64;
    let labels = ['B', 'K', 'M', 'G', 'T', 'P'];
    let mut index = 0;
    while value > 1024.0 && index + 1 < labels.len() {
        value /= 1024.0;
        index += 1;
    }
    if (value * 10.0) as u64 % 10 == 0 {
        format!("{value:.0}{}", labels[index])
    } else {
        format!("{value:.1}{}", labels[index])
    }
}
fn maximum(col: &Column<'_>, width: usize) -> git::Result<usize> {
    match col.options.get("maxwidth") {
        Some(value) if value.ends_with('%') => {
            let n: usize = value[..value.len() - 1]
                .parse()
                .map_err(|_| GitError("Invalid maxwidth".into()))?;
            if n > 100 {
                return Err(GitError("maxwidth exceeds 100%".into()));
            }
            Ok(width.saturating_mul(n) / 100)
        }
        _ => col.number("maxwidth").map_err(GitError),
    }
}

fn draw(
    config: &Config,
    entries: &[Entry],
    directory: &Path,
    width: usize,
) -> git::Result<Vec<TreeRow>> {
    let cols = columns(config)?;
    let mut rows = vec![String::new(); entries.len()];
    for col in cols.into_iter().filter(Column::enabled) {
        let fixed = col.number("width").map_err(GitError)?;
        let max = maximum(&col, width)?;
        let values: Vec<String> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| -> Result<String, String> {
                Ok(match col.name {
                    "mode" => mode(&e.tree.mode).into(),
                    "file-name" => {
                        if e.parent {
                            "..".into()
                        } else {
                            render::sanitize(
                                &e.tree
                                    .path
                                    .strip_prefix(directory)
                                    .unwrap_or(&e.tree.path)
                                    .to_string_lossy(),
                            )
                        }
                    }
                    "file-size" => {
                        if e.tree.mode.starts_with("100") {
                            size(e.tree.size.unwrap_or(0), col.display == "units")
                        } else {
                            String::new()
                        }
                    }
                    "line-number" => {
                        let interval = col.number("interval")?;
                        let interval = if interval == 0 { 5 } else { interval };
                        let number = i + 2; // The directory header is line 1.
                        if number % interval == 0 {
                            number.to_string()
                        } else {
                            String::new()
                        }
                    }
                    _ => match &e.history {
                        None => String::new(),
                        Some(c) => match col.name {
                            "id" => c.oid.clone(),
                            "author" => render::author(
                                &c.author,
                                &c.author_email,
                                col.display,
                                fixed.max(max),
                            )?,
                            "committer" => render::author(
                                &c.committer,
                                &c.committer_email,
                                col.display,
                                fixed.max(max),
                            )?,
                            "date" => render::date(
                                if col.flag("use-author", false)? {
                                    &c.date
                                } else {
                                    &c.committer_date
                                },
                                &col,
                            )?,
                            _ => unreachable!(),
                        },
                    },
                })
            })
            .collect::<Result<_, _>>()
            .map_err(GitError)?;
        let mut cells = if fixed > 0 {
            fixed
        } else if col.name == "id" {
            config.usize_value("id-width", 7).max(1)
        } else {
            values
                .iter()
                .map(|s| render::cell_width(s))
                .max()
                .unwrap_or(0)
        };
        if col.name == "line-number" {
            cells = cells.clamp(3, 9);
        }
        if fixed == 0 && max > 0 {
            cells = cells.min(max);
        }
        cells = cells.min(width);
        if cells == 0 && matches!(col.name, "file-size" | "mode") {
            continue;
        }
        for (row, value) in rows.iter_mut().zip(&values) {
            let mut value_clipped = render::clip(value, cells);
            if matches!(col.name, "author" | "committer" | "file-name")
                && render::cell_width(value) > cells
                && (col.name == "file-name" || cells > 10)
            {
                value_clipped = render::trim_field(value, cells, config);
            }
            let padding = " ".repeat(cells.saturating_sub(render::cell_width(&value_clipped)));
            if matches!(col.name, "file-size" | "line-number") {
                row.push_str(&padding);
                row.push_str(&value_clipped);
            } else {
                row.push_str(&value_clipped);
                row.push_str(&padding);
            }
            row.push_str(if col.name == "line-number" {
                if config.value("line-graphics") == Some("ascii") {
                    "| "
                } else {
                    "│ "
                }
            } else {
                " "
            });
        }
    }
    let name = directory.to_string_lossy();
    let mut result = vec![TreeRow {
        text: render::clip(
            &render::sanitize(&format!(
                "Directory path /{}{}",
                name,
                if name.is_empty() { "" } else { "/" }
            )),
            width,
        ),
        entry: None,
        commit: None,
    }];
    result.extend(rows.into_iter().zip(entries).map(|(text, e)| TreeRow {
        text: render::clip(text.trim_end(), width),
        entry: Some(e.tree.clone()),
        commit: e.history.as_ref().map(|c| c.oid.clone()),
    }));
    Ok(result)
}

/// Load a committed directory. `sort_field=None` uses directories-first filename
/// order; supported explicit fields are the standard tree column names.
pub fn load(
    repo: &Repository,
    config: &Config,
    revision: &str,
    directory: &Path,
    width: usize,
    sort_field: Option<&str>,
    reverse: bool,
) -> git::Result<Vec<TreeRow>> {
    if directory
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(GitError(
            "Expected a repository-relative directory without '..'".into(),
        ));
    }
    let oid = repo.revision(revision)?;
    let recursive = config.bool_value("recurse-tree", false);
    let trees = if recursive {
        let mut spec = OsString::from(format!("{oid}:"));
        spec.push(directory);
        let mut trees = git::parse_tree(&repo.command(vec![
            "ls-tree".into(),
            "-z".into(),
            "-l".into(),
            "-r".into(),
            spec,
        ])?)?;
        for entry in &mut trees {
            entry.path = directory.join(&entry.path);
        }
        trees
    } else {
        repo.tree(&oid, directory)?
    };
    let mut entries: Vec<_> = trees
        .into_iter()
        .map(|tree| Entry {
            tree,
            history: None,
            timestamps: (0, 0),
            parent: false,
        })
        .collect();
    let columns = columns(config)?;
    if columns
        .iter()
        .any(|c| c.enabled() && matches!(c.name, "author" | "committer" | "date" | "id"))
        || matches!(sort_field, Some("author" | "committer" | "date" | "id"))
    {
        let mut args: Vec<OsString> = [
            "log",
            "--no-color",
            "--encoding=UTF-8",
            "--raw",
            "--cc",
            "--no-abbrev",
            "-z",
            "--format=%x00%H%x00%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%at%x00%ct%x00",
            &oid,
            "--",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        if !directory.as_os_str().is_empty() {
            args.push(directory.into());
        }
        // ponytail: one captured history; stream if large histories make memory usage significant.
        annotate(&mut entries, directory, recursive, &repo.command(args)?)?;
    }
    let use_author = columns
        .iter()
        .find(|c| c.name == "date")
        .map(|c| c.flag("use-author", false))
        .transpose()
        .map_err(GitError)?
        .unwrap_or(false);
    let field = sort_field.unwrap_or("file-name");
    if !matches!(
        field,
        "file-name" | "mode" | "author" | "committer" | "id" | "file-size" | "date" | "line-number"
    ) {
        return Err(GitError(format!("Unsupported tree sort field: {field}")));
    }
    entries.sort_by(|a, b| {
        let metadata = |e: &Entry| {
            e.history
                .as_ref()
                .map(|c| match field {
                    "author" => c.author.clone(),
                    "committer" => c.committer.clone(),
                    "date" => {
                        if use_author {
                            c.date.clone()
                        } else {
                            c.committer_date.clone()
                        }
                    }
                    _ => c.oid.clone(),
                })
                .unwrap_or_default()
        };
        let order = match field {
            "file-name" | "line-number" => {
                (a.tree.kind != "tree", &a.tree.path).cmp(&(b.tree.kind != "tree", &b.tree.path))
            }
            "mode" => a.tree.mode.cmp(&b.tree.mode),
            "file-size" => a
                .tree
                .size
                .unwrap_or(u64::MAX)
                .cmp(&b.tree.size.unwrap_or(u64::MAX)),
            "date" => (if use_author {
                a.timestamps.0
            } else {
                a.timestamps.1
            })
            .cmp(
                &(if use_author {
                    b.timestamps.0
                } else {
                    b.timestamps.1
                }),
            ),
            _ => metadata(a).cmp(&metadata(b)),
        }
        .then_with(|| a.tree.path.cmp(&b.tree.path));
        if reverse {
            order.reverse()
        } else {
            order
        }
    });
    if !directory.as_os_str().is_empty() || entries.is_empty() {
        entries.insert(
            0,
            Entry {
                tree: TreeEntry {
                    mode: "040000".into(),
                    kind: "tree".into(),
                    oid,
                    size: None,
                    path: directory.parent().unwrap_or(Path::new("")).into(),
                },
                history: None,
                timestamps: (0, 0),
                parent: true,
            },
        );
    }
    draw(config, &entries, directory, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tig-tree-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn upstream_tree_screen_rows() {
        let fixture = Fixture::new();
        let archive = fixture.0.join("fixture.tgz");
        fs::write(
            &archive,
            include_bytes!("../test/files/scala-js-benchmarks.tgz"),
        )
        .unwrap();
        assert!(Command::new("tar")
            .args(["-xzf"])
            .arg(&archive)
            .arg("-C")
            .arg(&fixture.0)
            .status()
            .unwrap()
            .success());
        let repo = Repository::discover(&fixture.0).unwrap();
        let cases = [
            ("default", include_str!("../test/tree/default-test")),
            ("chdir", include_str!("../test/tree/chdir-test")),
            ("recurse", include_str!("../test/tree/recurse-test")),
        ];
        let mut checked = 0;
        for (kind, source) in cases {
            for block in source.split("assert_equals '").skip(1) {
                let (name, remainder) = block.split_once("' <<EOF\n").unwrap();
                let expected: Vec<_> = remainder
                    .split("\nEOF")
                    .next()
                    .unwrap()
                    .lines()
                    .take_while(|line| !line.trim().is_empty() && !line.starts_with("[tree]"))
                    .collect();
                let Some(path) = expected
                    .first()
                    .and_then(|line| line.strip_prefix("Directory path /"))
                else {
                    continue;
                };
                let mut config = Config::defaults();
                if kind == "default" {
                    config.parse("set tree-view-date-use-author = yes");
                }
                if kind == "recurse" {
                    config.parse("set recurse-tree = yes");
                }
                let field = ["mode", "author", "file-size", "date"]
                    .into_iter()
                    .find(|field| name == format!("tree-default-{field}.screen"));
                let rows = load(
                    &repo,
                    &config,
                    "HEAD",
                    Path::new(path.trim_end_matches('/')),
                    200,
                    field,
                    false,
                )
                .unwrap();
                assert_eq!(
                    rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
                    expected,
                    "{name}"
                );
                assert!(rows[0].entry.is_none());
                if !path.is_empty() {
                    assert_eq!(
                        rows[1].entry.as_ref().unwrap().path,
                        Path::new(path.trim_end_matches('/')).parent().unwrap()
                    );
                    assert!(rows[1].commit.is_none());
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 15);
        let mut config = Config::defaults();
        config.parse(
            "set line-graphics = ascii\nset tree-view = line-number:yes,interval=5 file-name",
        );
        let rows = load(&repo, &config, "HEAD", Path::new(""), 80, None, false).unwrap();
        assert!(rows[1].text.starts_with("   | "));
        assert!(rows[4].text.starts_with("  5| "));
        assert!(rows[9].text.starts_with(" 10| "));
    }
    #[test]
    fn modes_units_and_history_paths() {
        assert_eq!(mode("120000"), "lrwxrwxrwx");
        assert_eq!(mode("160000"), "m---------");
        assert_eq!(size(1024, true), "1024B");
        assert_eq!(size(1536, true), "1.5K");
        let mut entries = vec![Entry {
            tree: TreeEntry {
                mode: "100644".into(),
                kind: "blob".into(),
                oid: "b".repeat(40),
                size: Some(3),
                path: "sub/a\tb\n".into(),
            },
            history: None,
            timestamps: (0, 0),
            parent: false,
        }];
        let data = format!("\0{}\0Name\0mail\02014-03-01T17:26:00-05:00\0Other\0email\02014-03-02T17:26:00-05:00\01393712760\01393799160\0\n:100644 100644 {} {} R100\0old\0sub/a\tb\n\0", "a".repeat(40), "b".repeat(40), "c".repeat(40));
        annotate(&mut entries, Path::new("sub"), false, data.as_bytes()).unwrap();
        assert_eq!(entries[0].history.as_ref().unwrap().author, "Name");
        assert_eq!(entries[0].timestamps, (1393712760, 1393799160));
        let rows = draw(&Config::defaults(), &entries, Path::new("sub"), 200).unwrap();
        assert!(rows[1].text.ends_with("a b"));
        assert!(!rows[1].text.chars().any(char::is_control));
        assert_eq!(
            rows[1].entry.as_ref().unwrap().path,
            Path::new("sub/a\tb\n")
        );
        assert!(annotate(&mut entries, Path::new(""), false, b"\0not-an-oid\0").is_err());
    }

    #[test]
    fn quoted_git_path_keeps_unicode_name_and_history() {
        let fixture = Fixture::new();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .arg(&fixture.0)
            .status()
            .unwrap()
            .success());
        let directory = "-- foo bar";
        let name = "as测试asd";
        fs::create_dir(fixture.0.join(directory)).unwrap();
        fs::write(fixture.0.join(directory).join(name), "data\n").unwrap();
        let repo = Repository::discover(&fixture.0).unwrap();
        repo.command(["config", "user.name", "Tree Fixture"])
            .unwrap();
        repo.command(["config", "user.email", "tree@example.invalid"])
            .unwrap();
        repo.command(["add", "."]).unwrap();
        repo.command(["commit", "-qm", "base"]).unwrap();

        let rows = load(
            &repo,
            &Config::defaults(),
            "HEAD",
            Path::new(directory),
            80,
            None,
            false,
        )
        .unwrap();
        let row = rows.iter().find(|row| row.text.contains(name)).unwrap();
        assert_eq!(
            row.entry.as_ref().unwrap().path,
            Path::new(directory).join(name)
        );
        assert!(row.commit.is_some());
    }
}
