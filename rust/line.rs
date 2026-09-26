// SPDX-License-Identifier: GPL-2.0-or-later
// Line metadata derived from Tig, Copyright (c) 2006-2026 Jonas Fonseca.

/// Canonical color names and prefixes, in first-match order.
/// Empty prefixes name color areas but never match row text.
const TYPES: &[(&str, &str)] = &[
    ("diff-header", "diff --"),
    ("diff-del-file", "--- "),
    ("diff-add-file", "+++ "),
    ("diff-start", "---"),
    ("diff-chunk", "@@"),
    ("diff-add", "+"),
    ("diff-add2", " +"),
    ("diff-del", "-"),
    ("diff-del2", " -"),
    ("diff-index", "index "),
    ("diff-oldmode", "old mode "),
    ("diff-newmode", "new mode "),
    ("diff-newfmode", "new file mode "),
    ("diff-delfmode", "deleted file mode "),
    ("diff-rename-from", "rename from "),
    ("diff-rename-to", "rename to "),
    ("diff-similarity", "similarity "),
    ("diff-no-newline", "\\ No newline at end of file"),
    ("diff-add-highlight", ""),
    ("diff-del-highlight", ""),
    ("pp-merge", "Merge: "),
    ("pp-refs", "Refs: "),
    ("pp-reflog", "Reflog: "),
    ("pp-reflogmsg", "Reflog message: "),
    ("commit", "commit "),
    ("parent", "parent "),
    ("tree", "tree "),
    ("author", "author "),
    ("committer", "committer "),
    ("default", ""),
    ("cursor", ""),
    ("cursor-blur", ""),
    ("status", ""),
    ("delimiter", ""),
    ("date", ""),
    ("mode", ""),
    ("id", ""),
    ("overflow", ""),
    ("directory", ""),
    ("file", ""),
    ("file-size", ""),
    ("line-number", ""),
    ("title-blur", ""),
    ("title-focus", ""),
    ("header", ""),
    ("section", ""),
    ("main-commit", ""),
    ("main-annotated", ""),
    ("main-tag", ""),
    ("main-local-tag", ""),
    ("main-remote", ""),
    ("main-stash", ""),
    ("main-note", ""),
    ("main-prefetch", ""),
    ("main-other", ""),
    ("main-replace", ""),
    ("main-tracked", ""),
    ("main-ref", ""),
    ("main-head", ""),
    ("stat-none", ""),
    ("stat-staged", ""),
    ("stat-unstaged", ""),
    ("stat-untracked", ""),
    ("help-group", ""),
    ("help-action", ""),
    ("help-toggle", ""),
    ("diff-stat", ""),
    ("palette-0", ""),
    ("palette-1", ""),
    ("palette-2", ""),
    ("palette-3", ""),
    ("palette-4", ""),
    ("palette-5", ""),
    ("palette-6", ""),
    ("palette-7", ""),
    ("palette-8", ""),
    ("palette-9", ""),
    ("palette-10", ""),
    ("palette-11", ""),
    ("palette-12", ""),
    ("palette-13", ""),
    ("graph-commit", ""),
    ("search-result", ""),
];

pub(crate) fn is_named(name: &str) -> bool {
    let name = name.replace('_', "-");
    TYPES
        .iter()
        .any(|(kind, _)| kind.eq_ignore_ascii_case(&name))
}

pub(crate) fn builtin_line_type(row: &str) -> &'static str {
    // tigrc remains the source of extra literal color rules, after named types.
    let literals = include_str!("../tigrc").lines().filter_map(|line| {
        let prefix = line.trim().strip_prefix("color \"")?.split('"').next()?;
        Some(("", prefix))
    });
    TYPES
        .iter()
        .copied()
        .chain(literals)
        .find_map(|(kind, prefix)| {
            (!prefix.is_empty())
                .then(|| row.get(..prefix.len()))
                .flatten()
                .filter(|start| start.eq_ignore_ascii_case(prefix))
                .map(|_| kind)
        })
        .unwrap_or("default")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_matches_reference() {
        // Temporary parity oracle only: remove this header comparison when the
        // C reference is pruned; TYPES and the behavioral checks remain Rust-owned.
        let reference: Vec<_> = include_str!("../include/tig/line.h")
            .lines()
            .filter_map(|line| {
                let (name, rest) = line.trim().strip_prefix("_(")?.split_once(',')?;
                Some((
                    name.to_ascii_lowercase().replace('_', "-"),
                    rest.split('"').nth(1)?.replace("\\\\", "\\"),
                ))
            })
            .collect();
        let actual: Vec<_> = TYPES
            .iter()
            .map(|(name, prefix)| (name.to_string(), prefix.to_string()))
            .collect();
        assert_eq!(actual, reference);
        for (name, prefix) in TYPES {
            assert!(is_named(&name.to_ascii_uppercase().replace('-', "_")));
            if !prefix.is_empty() {
                assert_eq!(builtin_line_type(&prefix.to_ascii_uppercase()), *name);
            }
        }
        assert!(!is_named("+"));
        assert!(!is_named("copy from "));
        assert_eq!(builtin_line_type(""), "default");
        assert_eq!(builtin_line_type("中文行"), "default");
        assert_eq!(builtin_line_type("COPY FROM file"), "");
    }
}
