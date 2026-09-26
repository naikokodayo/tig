// SPDX-License-Identifier: GPL-2.0-or-later
//! Refs view preparation. The synthetic first row selects all history.
use crate::{
    config::Config,
    git::{GitError, Repository, Result},
    model::{Commit, Reference},
    render,
};
use std::{cmp::Ordering, collections::HashMap};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub struct RefRow {
    pub text: String,
    pub reference: Option<Reference>,
    pub name: String,
}

fn name(reference: &Reference) -> &str {
    ["refs/heads/", "refs/remotes/", "refs/tags/"]
        .iter()
        .find_map(|prefix| reference.name.strip_prefix(prefix))
        .unwrap_or(&reference.name)
}
fn kind(reference: &Reference, upstream: &str) -> usize {
    if reference.current {
        0
    } else if reference.name.starts_with("refs/heads/") {
        1
    } else if reference.name == upstream {
        2
    } else if reference.name.starts_with("refs/remotes/") {
        3
    } else if reference.name.starts_with("refs/tags/") {
        if reference.target.is_empty() {
            5
        } else {
            4
        }
    } else if reference.name.starts_with("refs/replace/") {
        6
    } else if reference.name == "refs/stash" {
        7
    } else if reference.name.starts_with("refs/notes/") {
        8
    } else if reference.name.starts_with("refs/prefetch/") {
        9
    } else {
        10
    }
}
// Tig orders numeric suffixes newest first, but other characters alphabetically.
fn numeric(a: &str, b: &str) -> Ordering {
    let shared = a.bytes().zip(b.bytes()).take_while(|(a, b)| a == b).count();
    let start = a.as_bytes()[..shared]
        .iter()
        .rposition(|c| !c.is_ascii_digit())
        .map_or(0, |i| i + 1);
    let digits = |s: &str| {
        s.as_bytes()[start..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .copied()
            .collect::<Vec<_>>()
    };
    let aa = digits(a);
    let bb = digits(b);
    let aa = aa
        .iter()
        .skip_while(|c| **c == b'0')
        .copied()
        .collect::<Vec<_>>();
    let bb = bb
        .iter()
        .skip_while(|c| **c == b'0')
        .copied()
        .collect::<Vec<_>>();
    bb.len()
        .cmp(&aa.len())
        .then_with(|| bb.cmp(&aa))
        .then_with(|| {
            if shared == a.len() || shared == b.len() {
                b.len().cmp(&a.len())
            } else {
                a.as_bytes()[shared].cmp(&b.as_bytes()[shared])
            }
        })
}
fn timestamp(iso: &str) -> i64 {
    let number = |a, b| {
        iso.get(a..b)
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let y = number(0, 4);
    let m = number(5, 7);
    let leap = |year: i64| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let mut days = 365 * (y - 1) + (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400;
    for month in 1..m {
        days += match month {
            2 => {
                if leap(y) {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
    }
    days += number(8, 10);
    let offset = (number(20, 22) * 60 + number(23, 25)) * 60;
    days * 86400 + number(11, 13) * 3600 + number(14, 16) * 60 + number(17, 19)
        - if iso.as_bytes().get(19) == Some(&b'-') {
            -offset
        } else {
            offset
        }
}
fn empty_commit() -> Commit {
    Commit {
        oid: String::new(),
        boundary: false,
        parents: vec![],
        author: String::new(),
        date: "1970-01-01T00:00:00+00:00".into(),
        author_email: String::new(),
        committer: String::new(),
        committer_email: String::new(),
        committer_date: "1970-01-01T00:00:00+00:00".into(),
        subject: String::new(),
        decorations: String::new(),
    }
}

/// `sort_field` is ref/date/author/committer/id/commit-title/line-number.
/// References retain full Git names; annotated tags' oid is their peeled target.
pub fn load(
    repo: &Repository,
    config: &Config,
    args: &[String],
    width: usize,
    sort_field: &str,
    reverse: bool,
) -> Result<Vec<RefRow>> {
    let mut filter = "";
    for arg in args {
        for candidate in ["--tags", "--branches", "--remotes", "--all"] {
            if arg.starts_with(candidate) {
                filter = candidate;
            }
        }
    }
    let heading = match filter {
        "--tags" => "All tags",
        "--branches" => "All branches",
        "--remotes" => "All remotes",
        _ => "All references",
    };
    let upstream = repo
        .command(["rev-parse", "--symbolic-full-name", "@{upstream}"])
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
        .unwrap_or_default();
    let history = repo.history(&["--all".into(), "--simplify-by-decoration".into()], 0)?;
    let commits: HashMap<_, _> = history.into_iter().map(|c| (c.oid.clone(), c)).collect();
    let mut entries = Vec::new();
    let mut references = repo.refs()?;
    if !references.iter().any(|r| r.current) {
        if let Ok(oid) = repo.revision("HEAD") {
            references.push(Reference {
                name: "HEAD".into(),
                oid,
                target: String::new(),
                current: true,
            });
        }
    }
    let replacements: Vec<String> = references
        .iter()
        .filter_map(|r| r.name.strip_prefix("refs/replace/").map(str::to_owned))
        .collect();
    let branch_names: Vec<String> = references
        .iter()
        .filter_map(|r| r.name.strip_prefix("refs/heads/").map(str::to_owned))
        .collect();
    let ordinary_ids: Vec<String> = references
        .iter()
        .filter(|r| !r.name.starts_with("refs/replace/"))
        .map(|r| r.oid.clone())
        .collect();
    for mut reference in references {
        let replacement = reference
            .name
            .strip_prefix("refs/replace/")
            .map(str::to_owned);
        if let Some(original) = replacement {
            if ordinary_ids.contains(&original) {
                continue;
            }
            reference.oid = original;
            reference.target.clear();
        }
        let k = if replacements.contains(&reference.oid) {
            6
        } else {
            kind(&reference, &upstream)
        };
        if match filter {
            "--tags" => !matches!(k, 4 | 5),
            "--branches" => k > 1,
            "--remotes" => !matches!(k, 2 | 3),
            "--all" => false,
            _ => matches!(k, 7..=9),
        } {
            continue;
        }
        let kind_name = [
            "head",
            "branch",
            "tracked-remote",
            "remote",
            "tag",
            "local-tag",
            "replace",
            "stash",
            "note",
            "prefetch",
            "other",
        ][k];
        if filter.is_empty()
            && config
                .settings
                .get("reference-format")
                .is_some_and(|formats| formats.iter().any(|f| f == &format!("hide:{kind_name}")))
        {
            continue;
        }
        let label = if reference.name.starts_with("refs/replace/") {
            "replaced".into()
        } else if reference.name.starts_with("refs/tags/")
            && branch_names.iter().any(|b| b == name(&reference))
        {
            reference.name.clone()
        } else {
            name(&reference).to_owned()
        };
        if !reference.target.is_empty() {
            reference.oid = reference.target.clone();
        }
        let commit = commits
            .get(&reference.oid)
            .cloned()
            .unwrap_or_else(empty_commit);
        entries.push((reference, label, k, commit));
    }
    if !matches!(
        sort_field,
        "ref" | "date" | "author" | "committer" | "id" | "commit-title" | "line-number"
    ) {
        return Err(GitError(format!(
            "Unsupported refs sort field: {sort_field}"
        )));
    }
    let use_author = config
        .value("refs-view-date-use-author")
        .map(|v| matches!(v, "yes" | "true" | "1"))
        .unwrap_or_else(|| {
            config.settings.get("refs-view").is_some_and(|cols| {
                cols.iter().any(|c| {
                    c.starts_with("date:")
                        && c.split(',').any(|o| {
                            matches!(
                                o,
                                "use-author"
                                    | "use-author=yes"
                                    | "use-author=true"
                                    | "use-author=1"
                            )
                        })
                })
            })
        });
    entries.sort_by(|a, b| {
        let order = match sort_field {
            "date" => timestamp(if use_author {
                &b.3.date
            } else {
                &b.3.committer_date
            })
            .cmp(&timestamp(if use_author {
                &a.3.date
            } else {
                &a.3.committer_date
            })),
            "author" => a.3.author.to_lowercase().cmp(&b.3.author.to_lowercase()),
            "committer" => {
                a.3.committer
                    .to_lowercase()
                    .cmp(&b.3.committer.to_lowercase())
            }
            "id" => a.0.oid.cmp(&b.0.oid),
            "commit-title" => a.3.subject.cmp(&b.3.subject),
            _ => a.2.cmp(&b.2).then_with(|| numeric(&a.1, &b.1)),
        }
        .then_with(|| a.2.cmp(&b.2))
        .then_with(|| numeric(&a.1, &b.1));
        if reverse {
            order.reverse()
        } else {
            order
        }
    });
    let mut rows = vec![RefRow {
        text: String::new(),
        reference: None,
        name: heading.into(),
    }];
    let mut data = vec![empty_commit()];
    for (reference, label, _, commit) in entries {
        rows.push(RefRow {
            text: String::new(),
            reference: Some(reference),
            name: label,
        });
        data.push(commit);
    }
    let specs = config
        .settings
        .get("refs-view")
        .ok_or_else(|| GitError("refs-view is not configured".into()))?;
    for spec in specs {
        let column = spec.split([':', ',']).next().unwrap_or("");
        if column == "ref" {
            let display = config.value("refs-view-ref-display").unwrap_or_else(|| {
                spec.split_once(':')
                    .map(|(_, s)| s.split(',').next().unwrap_or("yes"))
                    .unwrap_or("yes")
            });
            if matches!(display, "no" | "false" | "0") {
                continue;
            }
            let fixed = config
                .value("refs-view-ref-width")
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    spec.split(',')
                        .find_map(|s| s.strip_prefix("width=").and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0);
            let max = config
                .value("refs-view-ref-maxwidth")
                .or_else(|| spec.split(',').find_map(|s| s.strip_prefix("maxwidth=")));
            let max = match max {
                Some(value) if value.ends_with('%') => {
                    let percent: usize = value[..value.len() - 1]
                        .parse()
                        .map_err(|_| GitError("Invalid ref maxwidth".into()))?;
                    if percent > 100 {
                        return Err(GitError("Ref maxwidth exceeds 100%".into()));
                    }
                    width.saturating_mul(percent) / 100
                }
                Some(value) => value
                    .parse()
                    .map_err(|_| GitError("Invalid ref maxwidth".into()))?,
                None => 0,
            };
            let size = if fixed > 0 {
                fixed
            } else {
                let inferred = rows.iter().map(|r| r.name.width()).max().unwrap_or(0);
                if max > 0 {
                    inferred.min(max)
                } else {
                    inferred
                }
            }
            .min(width);
            for row in &mut rows {
                let value = render::clip(&render::sanitize(&row.name), size);
                row.text.push_str(&value);
                row.text
                    .push_str(&" ".repeat(size.saturating_sub(value.width()) + 1));
            }
        } else {
            let mut cfg = config.clone();
            cfg.settings.retain(|key, _| !key.starts_with("main-view-"));
            cfg.settings.insert("main-view".into(), vec![spec.clone()]);
            for (key, value) in &config.settings {
                if let Some(option) = key.strip_prefix("refs-view-") {
                    cfg.settings
                        .insert(format!("main-view-{option}"), value.clone());
                }
            }
            let mut fields = render::render_commits(&cfg, &data, width).map_err(GitError)?;
            // An empty decoration log has no observed date width in upstream.
            if column == "date" && data.iter().all(|c| c.oid.is_empty()) {
                for field in &mut fields {
                    if !field.is_empty() {
                        *field = " ".into();
                    }
                }
            }
            for (i, (row, field)) in rows.iter_mut().zip(fields).enumerate() {
                if column != "line-number" && (i == 0 || data[i].oid.is_empty()) {
                    row.text.push_str(&" ".repeat(field.width()));
                } else {
                    row.text.push_str(&field);
                }
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refs_metadata_filter_and_columns() {
        let dir = std::env::temp_dir().join(format!("tig-refs-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success());
        let repo = Repository::discover(&dir).unwrap();
        std::fs::write(dir.join("file"), "content\n").unwrap();
        repo.command(["add", "file"]).unwrap();
        repo.command([
            "-c",
            "user.name=Ref User",
            "-c",
            "user.email=ref@example.test",
            "commit",
            "--allow-empty",
            "-qm",
            "First subject",
        ])
        .unwrap();
        repo.command(["branch", "-M", "main"]).unwrap();
        repo.command([
            "-c",
            "user.name=Ref User",
            "-c",
            "user.email=ref@example.test",
            "tag",
            "-am",
            "annotated",
            "v2",
        ])
        .unwrap();
        repo.command(["tag", "v1"]).unwrap();
        let config = Config::defaults();
        let rows = load(&repo, &config, &[], 120, "ref", false).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["All references", "main", "v2", "v1"]
        );
        assert!(rows[0].reference.is_none());
        assert!(rows[1].text.contains("Ref User"), "{:?}", rows[1]);
        assert!(rows[2].text.contains("First subject"));
        assert_eq!(
            rows[1].reference.as_ref().unwrap().oid,
            rows[2].reference.as_ref().unwrap().oid
        );
        let mut config = config;
        config.parse("set refs-view = ref commit-title");
        let rows = load(&repo, &config, &["--tags".into()], 120, "ref", true).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["All tags", "v1", "v2"]
        );
        assert!(rows[1].text.starts_with("v1"));
        config.parse("set refs-view = ref:yes,maxwidth=5 commit-title");
        let rows = load(&repo, &config, &[], 120, "ref", false).unwrap();
        assert!(rows[0].text.starts_with("All r "));
        assert!(rows[1].text.starts_with("main  "));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn tig_reference_numeric_order() {
        let mut refs = vec![
            "v1.1", "v2.0", "v1.10", "v1.2", "r1.1.x", "r1.1.2", "master",
        ];
        refs.sort_by(|a, b| numeric(a, b));
        assert_eq!(
            refs,
            ["master", "r1.1.2", "r1.1.x", "v2.0", "v1.10", "v1.2", "v1.1"]
        );
        assert_eq!(numeric("作者", "作品"), "作者".cmp("作品"));
        assert_eq!(
            timestamp("2020-01-01T01:00:00+01:00"),
            timestamp("2020-01-01T00:00:00+00:00")
        );
    }
}
