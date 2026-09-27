// SPDX-License-Identifier: GPL-2.0-or-later
// Help view assembled from Tig's request descriptions and the active bindings.
use crate::config::Config;
use std::collections::BTreeSet;

const MAPS: &[&str] = &[
    "generic", "search", "main", "diff", "log", "reflog", "tree", "blob", "blame", "refs",
    "status", "stage", "stash", "grep", "pager", "help",
];
const TOGGLES: &[(char, &str, &str)] = &[
    ('.', "line-number", "line numbers"),
    ('D', "date", "dates"),
    ('A', "author", "author"),
    ('T', "committer", "committer"),
    ('~', "line-graphics", "graphics"),
    ('g', "commit-title-graph", "revision graph"),
    ('#', "file-name", "file names"),
    ('*', "file-size", "file sizes"),
    ('W', "ignore-space", "space changes"),
    ('l', "commit-order", "commit order"),
    ('F', "commit-title-refs", "reference display"),
    ('C', "show-changes", "local change display"),
    ('X', "id", "commit ID display"),
    ('%', "file-filter", "file filtering"),
    ('^', "rev-filter", "revision filtering"),
    (
        '$',
        "commit-title-overflow",
        "commit title overflow display",
    ),
    (
        'd',
        "status-show-untracked-dirs",
        "untracked directory info",
    ),
    ('|', "vertical-split", "view split"),
];

#[derive(Clone, Debug)]
pub struct HelpRow {
    pub text: String,
    pub line_type: &'static str,
    /// Present only on a section heading; `all` is the collapse/expand heading.
    pub section: Option<&'static str>,
}
impl HelpRow {
    pub fn search_text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug)]
pub struct HelpView {
    pub rows: Vec<HelpRow>,
    collapsed: BTreeSet<&'static str>,
    all_collapsed: bool,
    keys_width: usize,
    name_width: usize,
}

impl HelpView {
    pub fn new(config: &Config, previous_view: &str) -> Self {
        let collapsed = MAPS
            .iter()
            .copied()
            .filter(|map| !matches!(*map, "generic" | "search") && *map != previous_view)
            .collect();
        let mut view = Self {
            rows: Vec::new(),
            collapsed,
            all_collapsed: false,
            keys_width: 0,
            name_width: 0,
        };
        view.refresh(config);
        view
    }

    /// Toggle a zero-based row. Returns false for a non-section row.
    pub fn toggle_section(&mut self, row: usize, config: &Config) -> bool {
        let Some(section) = self.rows.get(row).and_then(|row| row.section) else {
            return false;
        };
        if section == "all" {
            self.all_collapsed = !self.all_collapsed;
            if self.all_collapsed {
                self.collapsed.extend(MAPS.iter().copied());
                self.collapsed.insert("toggle");
            } else {
                self.collapsed.clear();
            }
        } else if !self.collapsed.insert(section) {
            self.collapsed.remove(section);
        }
        self.refresh(config);
        true
    }

    pub fn refresh(&mut self, config: &Config) {
        let bindings = ordered_bindings(config);
        let requests = request_info();
        let mut sections: Vec<(&'static str, Vec<HelpRow>)> = Vec::new();
        let mut keys_width = self.keys_width;
        let mut name_width = self.name_width;
        for &map in MAPS {
            let local: Vec<_> = bindings.iter().filter(|b| b.map == map).collect();
            if local.is_empty() {
                continue;
            }
            let visible = !self.collapsed.contains(map);
            let mut content = Vec::new();
            let mut previous_group = "";
            for (group, action, description) in &requests {
                let keys: Vec<_> = local
                    .iter()
                    .filter(|b| b.action.len() == 1 && b.action[0] == *action)
                    .map(|b| key_name(b.key))
                    .collect();
                if keys.is_empty() {
                    continue;
                }
                if group != previous_group {
                    content.push(row(group.clone(), "help-group"));
                    previous_group = group;
                }
                let key = keys.join(", ");
                if visible {
                    keys_width = keys_width.max(key.len());
                    name_width = name_width.max(action.len());
                }
                content.push(row(format!("\0{key}\0{action}\0{description}"), "default"));
            }
            for (category, heading) in [
                ("toggle", "Option toggling:"),
                ("internal", "Internal commands:"),
                ("external", "External commands:"),
            ] {
                let mut group_started = false;
                for binding in &local {
                    let Some((kind, command)) = run_command(binding.action) else {
                        continue;
                    };
                    if kind != category {
                        continue;
                    }
                    if !group_started {
                        content.push(row(heading.into(), "help-group"));
                        group_started = true;
                    }
                    let key = key_name(binding.key);
                    if visible {
                        keys_width = keys_width.max(key.len());
                    }
                    content.push(row(format!("\0{key}\0{command}"), "default"));
                }
            }
            if !content.is_empty() {
                sections.push((map, content));
            }
        }
        self.keys_width = keys_width;
        self.name_width = name_width;
        let key_field = keys_width + 2;
        self.rows = vec![
            row("Quick reference for tig keybindings:".into(), "header"),
            HelpRow {
                text: format!(
                    "[{}] {} all sections",
                    if self.all_collapsed { '+' } else { '-' },
                    if self.all_collapsed {
                        "Expand"
                    } else {
                        "Collapse"
                    }
                ),
                line_type: "section",
                section: Some("all"),
            },
            row(String::new(), "default"),
        ];
        for (map, content) in sections {
            let collapsed = self.collapsed.contains(map);
            self.rows.push(HelpRow {
                text: format!("[{}] {map} bindings", if collapsed { '+' } else { '-' }),
                line_type: "section",
                section: Some(map),
            });
            if collapsed {
                continue;
            }
            for mut item in content {
                if let Some(rest) = item.text.strip_prefix('\0') {
                    let parts: Vec<_> = rest.split('\0').collect();
                    item.text = if parts.len() == 3 {
                        format!(
                            "{:>key_field$} {:<name_width$} {}",
                            parts[0], parts[1], parts[2]
                        )
                    } else {
                        format!("{:>key_field$} {}", parts[0], parts[1])
                    };
                }
                self.rows.push(item);
            }
        }
        let collapsed = self.collapsed.contains("toggle");
        self.rows.push(HelpRow {
            text: format!("[{}] toggle bindings", if collapsed { '+' } else { '-' }),
            line_type: "section",
            section: Some("toggle"),
        });
        if !collapsed {
            self.rows
                .push(row("Toggle keys (enter: o <key>):".into(), "help-group"));
            let toggle_width = TOGGLES
                .iter()
                .map(|(_, name, _)| name.len())
                .max()
                .unwrap_or(0);
            for (key, name, description) in TOGGLES {
                self.rows.push(row(
                    format!(
                        "{:>key_field$} {:<toggle_width$} Toggle {description}",
                        key, name
                    ),
                    "help-toggle",
                ));
            }
        }
    }
}

fn row(text: String, line_type: &'static str) -> HelpRow {
    HelpRow {
        text,
        line_type,
        section: None,
    }
}

struct Binding<'a> {
    map: &'a str,
    key: &'a str,
    action: &'a [String],
    order: usize,
}

fn ordered_bindings(config: &Config) -> Vec<Binding<'_>> {
    let mut bindings: Vec<_> = config
        .bindings
        .iter()
        .filter(|(_, action)| action.first().is_some_and(|action| action != "none"))
        .map(|((map, key), action)| {
            let order = if run_command(action).is_some() {
                config
                    .binding_updates
                    .iter()
                    .rposition(|binding| binding == &(map.clone(), key.clone()))
            } else {
                config
                    .binding_updates
                    .iter()
                    .position(|binding| binding == &(map.clone(), key.clone()))
            };
            Binding {
                map,
                key,
                action,
                order: order.unwrap_or(usize::MAX),
            }
        })
        .collect();
    bindings.sort_by(|a, b| (a.order, a.key).cmp(&(b.order, b.key)));
    bindings
}

use crate::request::request_info;

fn key_name(key: &str) -> String {
    let mut result = String::new();
    let mut rest = key;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('<') {
            if let Some(end) = tail.find('>') {
                let token = &tail[..end];
                if let Some(display) = match token {
                    "PgUp" => Some("<PageUp>"),
                    "PgDown" => Some("<PageDown>"),
                    "Ins" => Some("<Insert>"),
                    "Del" => Some("<Delete>"),
                    "SBack" => Some("<ScrollBack>"),
                    "SFwd" => Some("<ScrollFwd>"),
                    _ => None,
                } {
                    result.push_str(display);
                } else if let Some(control) = token.strip_prefix("C-") {
                    result.push_str("<Ctrl-");
                    result.push_str(control);
                    result.push('>');
                } else {
                    result.push('<');
                    result.push_str(token);
                    result.push('>');
                }
                rest = &tail[end + 1..];
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        if ch == ',' {
            result.push_str("','");
        } else if ch == ' ' {
            result.push_str("<Space>");
        } else {
            result.push(ch);
        }
        rest = &rest[ch.len_utf8()..];
    }
    result
}

fn run_command(action: &[String]) -> Option<(&'static str, String)> {
    let first = action.first()?;
    if !first.starts_with([':', '!', '?', '@', '<', '+', '>']) {
        return None;
    }
    let internal = first.starts_with(':');
    let mut rest = first.as_str();
    let mut flags = String::new();
    if internal {
        rest = &rest[1..];
        flags.push(':');
    } else {
        let mut seen = BTreeSet::new();
        while let Some(ch) = rest.chars().next() {
            if !matches!(ch, '!' | '@' | '?' | '<' | '+' | '>') {
                break;
            }
            seen.insert(ch);
            rest = &rest[ch.len_utf8()..];
        }
        for ch in ['@', '?', '<', '+', '>'] {
            if seen.contains(&ch) {
                flags.push(ch);
            }
        }
        if flags.is_empty() {
            flags.push('!');
        }
    }
    let command = std::iter::once(format!("{flags}{rest}"))
        .chain(action.iter().skip(1).cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let category = if internal && rest == "toggle" {
        "toggle"
    } else if internal {
        "internal"
    } else {
        "external"
    };
    Some((category, command))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_collapsed_help_follow_active_bindings() {
        let config = Config::defaults();
        let mut help = HelpView::new(&config, "pager");
        assert_eq!(help.rows[0].text, "Quick reference for tig keybindings:");
        assert_eq!(help.rows[3].text, "[-] generic bindings");
        assert!(help.rows[5].text.contains("m view-main"));
        assert!(help.rows.iter().any(|row| row.text == "[-] pager bindings"));
        assert!(help.rows.iter().any(|row| row.text == "[+] main bindings"));
        assert!(help.toggle_section(3, &config));
        assert!(help.toggle_section(4, &config));
        assert_eq!(help.rows[3].text, "[+] generic bindings");
        assert_eq!(help.rows[4].text, "[+] search bindings");
        assert!(help.toggle_section(1, &config));
        assert_eq!(help.rows[1].text, "[+] Expand all sections");
        assert!(help.toggle_section(1, &config));
        assert_eq!(help.rows[1].text, "[-] Collapse all sections");
        assert!(help.rows.len() > 100);
    }

    #[test]
    fn user_commands_and_key_names_are_searchable() {
        let mut config = Config::defaults();
        config.parse("bind generic a !?@user-test-cmd\nbind generic b ?@user-test-cmd");
        let help = HelpView::new(&config, "pager");
        assert!(help
            .rows
            .iter()
            .any(|row| row.text.contains("a @?user-test-cmd")));
        assert!(help
            .rows
            .iter()
            .any(|row| row.search_text().contains("user-test-cmd")));
        assert_eq!(key_name("<Down><C-N>,"), "<Down><Ctrl-N>','");
    }
}
