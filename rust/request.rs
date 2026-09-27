// SPDX-License-Identifier: GPL-2.0-or-later
// Request metadata derived from Tig, Copyright (c) 2006-2026 Jonas Fonseca.

/// The C VIEW_INFO order also determines the view-switching help order.
pub(crate) const VIEWS: &[&str] = &[
    "main", "diff", "log", "reflog", "tree", "blob", "blame", "refs", "status", "stage", "stash",
    "grep", "pager", "help",
];

/// The C REQ_INFO order, including the hidden but valid `none` action.
const GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "View manipulation",
        &[
            ("enter", "Enter and open selected line"),
            ("back", "Go back to the previous view state"),
            ("next", "Move to next"),
            ("previous", "Move to previous"),
            ("parent", "Move to parent"),
            ("view-next", "Move focus to the next view"),
            ("refresh", "Reload and refresh view"),
            ("maximize", "Maximize the current view"),
            ("view-close", "Close the current view"),
            (
                "view-close-no-quit",
                "Close the current view without quitting",
            ),
            ("quit", "Close all views and quit"),
        ],
    ),
    (
        "View-specific actions",
        &[
            ("status-update", "Stage/unstage chunk or file changes"),
            ("status-revert", "Revert chunk or file changes"),
            ("status-merge", "Merge file using external tool"),
            ("stage-update-line", "Stage/unstage single line"),
            ("stage-update-part", "Stage/unstage part of a chunk"),
            ("stage-split-chunk", "Split current diff chunk"),
        ],
    ),
    (
        "Cursor navigation",
        &[
            ("move-up", "Move cursor one line up"),
            ("move-down", "Move cursor one line down"),
            ("move-page-up", "Move cursor one page up"),
            ("move-page-down", "Move cursor one page down"),
            ("move-half-page-up", "Move cursor half a page up"),
            ("move-half-page-down", "Move cursor half a page down"),
            ("move-first-line", "Move cursor to first line"),
            ("move-last-line", "Move cursor to last line"),
            ("move-next-merge", "Move cursor to next merge commit"),
            ("move-prev-merge", "Move cursor to previous merge commit"),
        ],
    ),
    (
        "Scrolling",
        &[
            ("scroll-line-up", "Scroll one line up"),
            ("scroll-line-down", "Scroll one line down"),
            ("scroll-page-up", "Scroll one page up"),
            ("scroll-page-down", "Scroll one page down"),
            ("scroll-half-page-up", "Scroll half a page up"),
            ("scroll-half-page-down", "Scroll half a page down"),
            ("scroll-first-col", "Scroll to the first line columns"),
            ("scroll-left", "Scroll two columns left"),
            ("scroll-right", "Scroll two columns right"),
        ],
    ),
    (
        "Searching",
        &[
            ("search", "Search the view"),
            ("search-back", "Search backwards in the view"),
            ("find-next", "Find next search match"),
            ("find-prev", "Find previous search match"),
        ],
    ),
    (
        "Misc",
        &[
            ("edit", "Open in editor"),
            ("prompt", "Open the prompt"),
            ("options", "Open the options menu"),
            ("screen-redraw", "Redraw the screen"),
            ("stop-loading", "Stop all loading views"),
            ("show-version", "Show version information"),
            ("none", "Do nothing"),
        ],
    ),
];

pub(crate) fn is_view(name: &str) -> bool {
    VIEWS.contains(&name)
}

pub(crate) fn known_request(name: &str) -> bool {
    name.strip_prefix("view-").is_some_and(is_view)
        || GROUPS
            .iter()
            .any(|(_, requests)| requests.iter().any(|(request, _)| *request == name))
}

pub fn request_info() -> Vec<(String, String, String)> {
    VIEWS
        .iter()
        .map(|view| {
            (
                "View switching".into(),
                format!("view-{view}"),
                format!("Show {view} view"),
            )
        })
        .chain(GROUPS.iter().flat_map(|(group, requests)| {
            requests
                .iter()
                .filter(|(name, _)| *name != "none")
                .map(|(name, help)| (group.to_string(), name.to_string(), help.to_string()))
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_matches_reference() {
        // Temporary C parity oracle; remove the C reads at final pruning.
        let views: Vec<_> = include_str!("../include/tig/tig.h")
            .split("#define VIEW_INFO(_)")
            .nth(1)
            .unwrap()
            .split("\n\n")
            .next()
            .unwrap()
            .lines()
            .filter_map(|line| line.trim().strip_prefix("_("))
            .map(|line| {
                line.split(',')
                    .nth(1)
                    .unwrap()
                    .split(')')
                    .next()
                    .unwrap()
                    .trim()
            })
            .collect();
        assert_eq!(VIEWS, views);
        let source = include_str!("../include/tig/request.h");
        let source = source
            .split("#define REQ_INFO")
            .nth(1)
            .unwrap()
            .split("/* User action requests. */")
            .next()
            .unwrap();
        let mut group = "";
        let mut expected = Vec::new();
        for line in source.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("REQ_GROUP(") {
                group = rest.split('"').nth(1).unwrap();
            } else if line.starts_with("VIEW_INFO(VIEW_REQ)") {
                expected.extend(views.iter().map(|view| {
                    (
                        "View switching".into(),
                        format!("view-{view}"),
                        format!("Show {view} view"),
                    )
                }));
            } else if let Some(rest) = line.strip_prefix("REQ_(") {
                let (name, help) = rest.split_once(',').unwrap();
                let name = name.to_ascii_lowercase().replace('_', "-");
                if name != "none" {
                    expected.push((group.into(), name, help.split('"').nth(1).unwrap().into()));
                }
            }
        }
        assert_eq!(request_info(), expected);
        assert!(known_request("none"));
        assert!(known_request("view-blame"));
        assert!(!known_request("view-unknown"));
        assert!(!known_request("move-up-extra"));
        assert!(!known_request("MOVE-UP"));
    }
}
