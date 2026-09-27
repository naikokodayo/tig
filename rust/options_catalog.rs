// SPDX-License-Identifier: GPL-2.0-or-later
// Configuration metadata derived from Tig, Copyright (c) 2006-2026 Jonas Fonseca.

const OPTIONS: &[(&str, &str)] = &[
    ("blame-options", "const char **"),
    ("blame-view", "view_settings"),
    ("blob-view", "view_settings"),
    ("commit-order", "enum commit_order"),
    ("diff-context", "int"),
    ("diff-indicator", "bool"),
    ("diff-noprefix", "bool"),
    ("diff-options", "const char **"),
    ("diff-highlight", "const char *"),
    ("word-diff", "bool"),
    ("diff-view", "view_settings"),
    ("editor-line-number", "bool"),
    ("file-args", "const char **"),
    ("file-filter", "bool"),
    ("focus-child", "bool"),
    ("git-colors", "const char **"),
    ("grep-view", "view_settings"),
    ("history-size", "int"),
    ("horizontal-scroll", "double"),
    ("id-width", "int"),
    ("ignore-case", "enum ignore_case"),
    ("ignore-space", "enum ignore_space"),
    ("line-graphics", "enum graphic"),
    ("log-options", "const char **"),
    ("log-view", "view_settings"),
    ("reflog-view", "view_settings"),
    ("mailmap", "bool"),
    ("main-options", "const char **"),
    ("main-view", "view_settings"),
    ("mouse", "bool"),
    ("mouse-scroll", "int"),
    ("mouse-wheel-cursor", "bool"),
    ("pager-autoscroll", "bool"),
    ("pager-view", "view_settings"),
    ("pgrp", "bool"),
    ("recurse-tree", "bool"),
    ("reference-format", "struct ref_format **"),
    ("refresh-interval", "int"),
    ("refresh-mode", "enum refresh_mode"),
    ("refs-view", "view_settings"),
    ("rev-args", "const char **"),
    ("rev-filter", "bool"),
    ("send-child-enter", "bool"),
    ("show-changes", "bool"),
    ("show-notes", "bool"),
    ("show-untracked", "bool"),
    ("split-view-height", "double"),
    ("split-view-width", "double"),
    ("stage-view", "view_settings"),
    ("start-on-head", "bool"),
    ("stash-view", "view_settings"),
    ("status-show-untracked-dirs", "bool"),
    ("status-show-untracked-files", "bool"),
    ("status-view", "view_settings"),
    ("tab-size", "int"),
    ("tree-view", "view_settings"),
    ("truncation-delimiter", "const char *"),
    ("vertical-split", "enum vertical_split"),
    ("wrap-lines", "bool"),
    ("wrap-search", "bool"),
];

const AUTHOR: &[(&str, &str)] = &[
    ("display", "enum author"),
    ("width", "int"),
    ("maxwidth", "int"),
];
const COMMIT_TITLE: &[(&str, &str)] = &[
    ("display", "bool"),
    ("graph", "enum graph_display"),
    ("refs", "bool"),
    ("overflow", "int"),
];
const DATE: &[(&str, &str)] = &[
    ("display", "enum date"),
    ("use-author", "bool"),
    ("local", "bool"),
    ("format", "const char *"),
    ("width", "int"),
];
const FILE_NAME: &[(&str, &str)] = &[
    ("display", "enum filename"),
    ("width", "int"),
    ("maxwidth", "int"),
];
const FILE_SIZE: &[(&str, &str)] = &[("display", "enum file_size"), ("width", "int")];
const ID: &[(&str, &str)] = &[("display", "bool"), ("color", "bool"), ("width", "int")];
const LINE_NUMBER: &[(&str, &str)] = &[("display", "bool"), ("interval", "int"), ("width", "int")];
const MODE: &[(&str, &str)] = &[("display", "bool"), ("width", "int")];
const REF: &[(&str, &str)] = &[("display", "bool"), ("width", "int"), ("maxwidth", "int")];
const SECTION: &[(&str, &str)] = &[("type", "enum line_type"), ("text", "const char *")];
const STATUS: &[(&str, &str)] = &[("display", "enum status_label")];
const TEXT: &[(&str, &str)] = &[("display", "bool"), ("commit-title-overflow", "int")];

// Order is the C COLUMN_OPTIONS order; committer shares author options.
const COLUMNS: &[(&str, &[(&str, &str)])] = &[
    ("author", AUTHOR),
    ("committer", AUTHOR),
    ("commit-title", COMMIT_TITLE),
    ("date", DATE),
    ("file-name", FILE_NAME),
    ("file-size", FILE_SIZE),
    ("id", ID),
    ("line-number", LINE_NUMBER),
    ("mode", MODE),
    ("ref", REF),
    ("section", SECTION),
    ("status", STATUS),
    ("text", TEXT),
];

// Order is the C ENUM_INFO order; values retain their C enum order.
const ENUMS: &[(&str, &[&str])] = &[
    (
        "author",
        &["no", "full", "abbreviated", "email", "email-user"],
    ),
    (
        "commit_order",
        &["auto", "default", "topo", "date", "author-date", "reverse"],
    ),
    (
        "date",
        &["no", "default", "relative", "relative-compact", "custom"],
    ),
    ("file_size", &["no", "default", "units"]),
    ("filename", &["no", "auto", "always"]),
    ("graphic", &["ascii", "default", "utf-8"]),
    ("graph_display", &["no", "v2", "v1"]),
    ("ignore_case", &["no", "yes", "smart-case"]),
    ("ignore_space", &["no", "all", "some", "at-eol"]),
    ("vertical_split", &["horizontal", "vertical", "auto"]),
    (
        "view_column_type",
        &[
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
        ],
    ),
    (
        "reference_type",
        &[
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
        ],
    ),
    (
        "refresh_mode",
        &["manual", "auto", "after-command", "periodic"],
    ),
    ("status_label", &["no", "short", "long"]),
];

pub(crate) fn option_type(name: &str) -> Option<&'static str> {
    OPTIONS
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, kind)| *kind)
}

pub fn completion_names() -> (Vec<String>, Vec<String>) {
    let options = OPTIONS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    let mut toggles = OPTIONS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect::<Vec<_>>();
    toggles.extend(
        COLUMNS
            .iter()
            .filter(|(name, _)| *name != "section")
            .flat_map(|(column, fields)| {
                fields
                    .iter()
                    .map(move |(name, _)| format!("{column}-{name}"))
            }),
    );
    (options, toggles)
}

pub(crate) fn column_names() -> impl Iterator<Item = &'static str> {
    COLUMNS.iter().map(|(name, _)| *name)
}

pub(crate) fn column_type(column: &str, option: &str) -> Option<&'static str> {
    COLUMNS
        .iter()
        .find(|(name, _)| *name == column)?
        .1
        .iter()
        .find(|(name, _)| *name == option)
        .map(|(_, kind)| *kind)
}

pub(crate) fn enum_values(kind: &str) -> Vec<String> {
    ENUMS
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, values)| values.iter().map(|value| value.to_string()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(source: &str) -> Vec<(String, String)> {
        source
            .lines()
            .filter_map(|line| {
                let mut fields = line.trim().strip_prefix("_(")?.split(',');
                Some((
                    fields.next()?.trim().to_ascii_lowercase().replace('_', "-"),
                    fields.next()?.trim().to_string(),
                ))
            })
            .collect()
    }

    #[test]
    fn metadata_matches_reference() {
        // Temporary C parity oracle: remove these includes when C is pruned.
        let options = include_str!("../include/tig/options.h");
        let before_externs = options
            .split("#define DEFINE_OPTION_EXTERNS")
            .next()
            .unwrap();
        let actual: Vec<_> = OPTIONS
            .iter()
            .map(|(name, kind)| (name.to_string(), kind.to_string()))
            .collect();
        assert_eq!(actual, entries(before_externs));

        for (name, fields) in COLUMNS {
            let macro_name = if *name == "committer" { "author" } else { name };
            let marker = format!(
                "#define {}_COLUMN_OPTIONS(_) ",
                macro_name.to_ascii_uppercase().replace('-', "_")
            );
            let section = options
                .split(&marker)
                .nth(1)
                .unwrap()
                .split("\n\n")
                .next()
                .unwrap();
            let actual: Vec<_> = fields
                .iter()
                .map(|(name, kind)| (name.to_string(), kind.to_string()))
                .collect();
            assert_eq!(actual, entries(section), "{name}");
            for (option, kind) in *fields {
                assert_eq!(column_type(name, option), Some(*kind));
            }
        }
        let column_section = options
            .split("#define COLUMN_OPTIONS(_) ")
            .nth(1)
            .unwrap()
            .split("\n\n")
            .next()
            .unwrap();
        let reference_columns: Vec<_> = entries(column_section)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(column_names().collect::<Vec<_>>(), reference_columns);

        let types = include_str!("../include/tig/types.h");
        let enum_section = types
            .split("#define ENUM_INFO(_) ")
            .nth(1)
            .unwrap()
            .split("\n\n")
            .next()
            .unwrap();
        let enum_names: Vec<_> = entries(enum_section)
            .into_iter()
            .map(|(name, _)| name.replace('-', "_"))
            .collect();
        assert_eq!(
            ENUMS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            enum_names
        );
        for (name, values) in ENUMS {
            let macro_name = match *name {
                "view_column_type" => "VIEW_COLUMN",
                "reference_type" => "REFERENCE",
                other => other,
            };
            let marker = format!("#define {}_ENUM(_) ", macro_name.to_ascii_uppercase());
            let section = types
                .split(&marker)
                .nth(1)
                .unwrap()
                .split("\n\n")
                .next()
                .unwrap();
            let reference: Vec<_> = entries(section)
                .into_iter()
                .map(|(_, value)| {
                    value
                        .split(')')
                        .next()
                        .unwrap()
                        .to_ascii_lowercase()
                        .replace('_', "-")
                })
                .collect();
            assert_eq!(*values, reference, "{name}");
            assert_eq!(enum_values(name), reference);
        }
        assert_eq!(option_type("diff-context"), Some("int"));
        assert_eq!(option_type("diff_context"), None);
        assert_eq!(column_type("committer", "width"), Some("int"));
        assert_eq!(column_type("author", "unknown"), None);
        assert!(enum_values("unknown").is_empty());
    }
}
