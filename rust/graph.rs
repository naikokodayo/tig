// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Rust port of Tig graph-v2.c. SPDX-License-Identifier: GPL-2.0-or-later
//! Stateful Tig v2 commit graph. Each rendered canvas owns its symbols.
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default)]
pub struct Symbol {
    pub color: usize,
    commit: bool,
    boundary: bool,
    initial: bool,
    merge: bool,
    continued_down: bool,
    continued_up: bool,
    continued_right: bool,
    continued_left: bool,
    continued_up_left: bool,
    parent_down: bool,
    parent_right: bool,
    below_commit: bool,
    flanked: bool,
    next_right: bool,
    matches_commit: bool,
    shift_left: bool,
    continue_shift: bool,
    below_shift: bool,
    new_column: bool,
    empty: bool,
}

impl Symbol {
    pub fn color_id(&self) -> i32 {
        if self.commit {
            -1
        } else {
            self.color as i32
        }
    }
    fn forks(&self) -> bool {
        if !self.continued_down {
            return false;
        }

        if !self.continued_right {
            return false;
        }

        if !self.continued_up {
            return false;
        }

        true
    }
    fn cross_merge(&self) -> bool {
        if self.empty {
            return false;
        }

        if !self.continued_up && !self.new_column && !self.below_commit {
            return false;
        }

        if self.shift_left && self.continued_up_left {
            return false;
        }

        if self.next_right {
            return false;
        }

        if self.merge
            && self.continued_up
            && self.continued_right
            && self.continued_left
            && self.parent_down
            && !self.next_right
        {
            return true;
        }

        false
    }
    fn vertical_merge(&self) -> bool {
        if self.empty {
            return false;
        }

        if !self.continued_up && !self.new_column && !self.below_commit {
            return false;
        }

        if self.shift_left && self.continued_up_left {
            return false;
        }

        if self.next_right {
            return false;
        }

        if !self.matches_commit {
            return false;
        }

        if self.merge
            && self.continued_up
            && self.continued_left
            && self.parent_down
            && !self.continued_right
        {
            return true;
        }

        false
    }
    fn cross_over(&self) -> bool {
        if self.empty {
            return false;
        }

        if !self.continued_down {
            return false;
        }

        if !self.continued_up && !self.new_column && !self.below_commit {
            return false;
        }

        if self.shift_left {
            return false;
        }

        if self.parent_right && self.merge {
            return true;
        }

        if self.flanked {
            return true;
        }

        false
    }
    fn turn_left(&self) -> bool {
        if self.matches_commit && self.continued_right && !self.continued_down {
            return false;
        }

        if self.continue_shift {
            return false;
        }

        if self.continued_up || self.new_column || self.below_commit {
            if self.matches_commit {
                return true;
            }

            if self.shift_left {
                return true;
            }
        }

        false
    }
    fn turn_down_cross_over(&self) -> bool {
        if !self.continued_down {
            return false;
        }

        if !self.continued_right {
            return false;
        }

        if !self.parent_right && !self.flanked {
            return false;
        }

        if self.flanked {
            return true;
        }

        if self.merge {
            return true;
        }

        false
    }
    fn turn_down(&self) -> bool {
        if !self.continued_down {
            return false;
        }

        if !self.continued_right {
            return false;
        }

        true
    }
    fn merge(&self) -> bool {
        if self.continued_down {
            return false;
        }

        if !self.parent_down {
            return false;
        }

        if self.parent_right {
            return false;
        }

        if self.continued_right {
            return false;
        }

        true
    }
    fn multi_merge(&self) -> bool {
        if !self.parent_down {
            return false;
        }

        if !self.parent_right && !self.continued_right {
            return false;
        }

        true
    }
    fn vertical_bar(&self) -> bool {
        if self.empty {
            return false;
        }

        if self.shift_left {
            return false;
        }

        if !self.continued_down {
            return false;
        }

        if self.continued_up {
            return true;
        }

        if self.parent_right {
            return false;
        }

        if self.flanked {
            return false;
        }

        if self.continued_right {
            return false;
        }

        true
    }
    fn horizontal_bar(&self) -> bool {
        if !self.next_right {
            return false;
        }

        if self.shift_left {
            return true;
        }

        if self.continued_down {
            return false;
        }

        if !self.parent_right && !self.continued_right {
            return false;
        }

        if self.continued_up && !self.continued_up_left {
            return false;
        }

        if !self.below_commit {
            return true;
        }

        false
    }
    fn multi_branch(&self) -> bool {
        if self.continued_down {
            return false;
        }

        if !self.continued_right {
            return false;
        }

        if self.below_shift {
            return false;
        }

        if self.continued_up || self.new_column || self.below_commit {
            if self.matches_commit {
                return true;
            }

            if self.shift_left {
                return true;
            }
        }

        false
    }
    pub fn ascii(&self) -> &'static str {
        if self.commit {
            if self.boundary {
                return " o";
            }
            if self.initial {
                return " I";
            }
            if self.merge {
                return " M";
            }
            return " *";
        }

        if self.cross_merge() {
            return "-+";
        }

        if self.vertical_merge() {
            return "-|";
        }

        if self.cross_over() {
            return "-|";
        }

        if self.vertical_bar() {
            return " |";
        }

        if self.turn_left() {
            return "-'";
        }

        if self.multi_branch() {
            return "-+";
        }

        if self.horizontal_bar() {
            return "--";
        }

        if self.forks() {
            return " +";
        }

        if self.turn_down_cross_over() {
            return "-.";
        }

        if self.turn_down() {
            return " .";
        }

        if self.merge() {
            return "-.";
        }

        if self.multi_merge() {
            return "-+";
        }

        "  "
    }
    pub fn utf8(&self) -> &'static str {
        if self.commit {
            if self.boundary {
                return " ◯";
            }
            if self.initial {
                return " ◎";
            }
            if self.merge {
                return " ●";
            }
            return " ∙";
        }

        if self.cross_merge() {
            return "─┼";
        }

        if self.vertical_merge() {
            return "─┤";
        }

        if self.cross_over() {
            return "─│";
        }

        if self.vertical_bar() {
            return " │";
        }

        if self.turn_left() {
            return "─╯";
        }

        if self.multi_branch() {
            return "─┴";
        }

        if self.horizontal_bar() {
            return "──";
        }

        if self.forks() {
            return " ├";
        }

        if self.turn_down_cross_over() {
            return "─╭";
        }

        if self.turn_down() {
            return " ╭";
        }

        if self.merge() {
            return "─╮";
        }

        if self.multi_merge() {
            return "─┬";
        }

        "  "
    }
}

#[derive(Clone, Debug, Default)]
struct Column {
    id: Option<String>,
    symbol: Symbol,
}

#[derive(Clone, Debug, Default)]
pub struct Canvas {
    pub symbols: Vec<Symbol>,
}
impl Canvas {
    pub fn is_merge(&self) -> bool {
        self.symbols.first().is_some_and(|s| s.merge)
    }
    pub fn render(&self, ascii: bool) -> String {
        let mut text = String::new();
        for (i, symbol) in self.symbols.iter().enumerate() {
            let chars = if ascii { symbol.ascii() } else { symbol.utf8() };
            text.push_str(if i == 0 { &chars[1..] } else { chars });
        }
        text
    }
}

#[derive(Default)]
pub struct Graph {
    row: Vec<Column>,
    prev: Vec<Column>,
    next: Vec<Column>,
    parents: Vec<Column>,
    position: usize,
    prev_position: usize,
    id: String,
    boundary: bool,
    has_parents: bool,
    colors: HashMap<String, usize>,
    counts: [usize; 14],
}

fn contains(row: &[Column], id: &Option<String>) -> bool {
    id.is_some() && row.iter().any(|c| &c.id == id)
}
fn down(row: &[Column], next: &[Column], pos: usize) -> bool {
    row[pos].id == next[pos].id && !row[pos].symbol.shift_left
}
fn shift(row: &[Column], prev: &[Column], pos: usize) -> bool {
    if row[pos].id.is_none() {
        return false;
    }
    for i in (0..pos).rev() {
        if row[i].id.is_some() && row[i].id == row[pos].id {
            return !down(prev, row, i);
        }
    }
    false
}
fn right(row: &[Column], pos: usize, commit: usize) -> bool {
    let end = if pos < commit { commit } else { row.len() };
    (pos + 1..end).any(|i| row[pos].id == row[i].id)
}
fn left(row: &[Column], pos: usize, commit: usize) -> bool {
    let start = if pos < commit { 0 } else { commit };
    (start..pos).any(|i| row[i].id.is_some() && row[pos].id == row[i].id)
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }
    /// Begin a commit. `parents` contains parent IDs (without the commit itself).
    pub fn add_commit(&mut self, id: &str, parents: &[&str], boundary: bool) {
        self.id = id.to_owned();
        self.position = self
            .row
            .iter()
            .position(|c| c.id.as_deref() == Some(id))
            .or_else(|| self.row.iter().position(|c| c.id.is_none()))
            .unwrap_or(self.row.len());
        self.boundary = boundary;
        self.parents.clear();
        self.has_parents = false;
        for parent in parents {
            self.add_parent(if parent.is_empty() {
                None
            } else {
                Some(parent)
            });
        }
        self.has_parents = !parents.is_empty();
    }
    /// Add parents from raw log records when the commit header did not supply any.
    pub fn add_parent(&mut self, parent: Option<&str>) {
        if !self.has_parents {
            self.parents.push(self.column(parent));
        }
    }
    fn column(&self, id: Option<&str>) -> Column {
        Column {
            id: id.map(str::to_owned),
            symbol: Symbol {
                boundary: self.boundary,
                ..Symbol::default()
            },
        }
    }
    fn expand(&mut self) {
        let empty = self.column(None);
        let len = self.position + self.parents.len();
        if len > self.row.len() {
            self.row.resize(len, empty.clone());
            self.prev.resize(len, empty.clone());
            self.next.resize(len, empty);
        }
    }
    fn generate_next(&mut self) {
        for col in &mut self.next {
            if col.id.as_deref() == Some(&self.id) {
                col.id = None;
            }
        }
        for parent in self.parents.clone() {
            if parent.id.is_none() {
                continue;
            }
            if let Some(i) = self.next.iter().position(|c| c.id.is_none()) {
                self.next[i] = parent;
            } else {
                let empty = self.column(None);
                self.next.push(self.column(parent.id.as_deref()));
                self.row.push(empty.clone());
                self.prev.push(empty);
            }
        }
        for i in (1..self.next.len()).rev() {
            if i == self.position
                || i == self.position + 1
                || self.next[i].id.as_deref() == Some(&self.id)
                || self.next[i].id != self.next[i - 1].id
            {
                continue;
            }
            if contains(&self.parents, &self.next[i].id) && self.prev[i].id.is_none() {
                continue;
            }
            if self.next[i - 1].id != self.prev[i - 1].id || self.prev[i - 1].symbol.shift_left {
                self.next[i] = self.next.get(i + 1).cloned().unwrap_or_default();
            }
        }
        for i in (0..self.next.len().saturating_sub(1)).rev() {
            if self.next[i].id.is_none() {
                self.next[i] = self.next[i + 1].clone();
            }
        }
    }
    fn color(&mut self, id: String) -> usize {
        if let Some(&color) = self.colors.get(&id) {
            return color;
        }
        let color = (0..14).min_by_key(|&i| self.counts[i]).unwrap();
        self.colors.insert(id, color);
        self.counts[color] += 1;
        color
    }
    pub fn render_parents(&mut self) -> Canvas {
        if self.parents.is_empty() {
            self.add_parent(None);
        }
        self.expand();
        self.generate_next();
        let commits = self.parents.iter().filter(|c| c.id.is_some()).count();
        let mut canvas = Canvas::default();
        for pos in 0..self.row.len() {
            let row = &self.row;
            let prev = &self.prev;
            let next = &self.next;
            // Preserve the C evaluation order: down() reads the old shift flag,
            // while subsequent columns see flags already updated to their left.
            let s = Symbol {
                commit: pos == self.position,
                boundary: pos == self.position && next[pos].symbol.boundary,
                initial: commits == 0,
                merge: commits > 1,
                continued_down: down(row, next, pos),
                continued_up: down(prev, row, pos),
                continued_right: right(row, pos, self.position),
                continued_left: left(row, pos, self.position),
                continued_up_left: left(prev, pos, prev.len()),
                parent_down: contains(&self.parents, &next[pos].id),
                parent_right: pos > self.position
                    && self.parents.iter().filter(|p| p.id.is_some()).any(|p| {
                        (pos + 1..next.len()).any(|i| p.id == next[i].id && p.id != row[i].id)
                    }),
                below_commit: pos == self.prev_position && row[pos].id == prev[pos].id,
                flanked: if pos < self.position {
                    (0..pos).any(|i| row[i].id.as_deref() == Some(&self.id))
                } else {
                    (pos + 1..row.len()).any(|i| row[i].id.as_deref() == Some(&self.id))
                },
                next_right: right(next, pos, 0),
                matches_commit: row[pos].id.as_deref() == Some(&self.id),
                shift_left: shift(row, prev, pos),
                continue_shift: pos + 1 < row.len() && shift(row, prev, pos + 1),
                below_shift: prev[pos].symbol.shift_left,
                new_column: prev[pos].id.is_none()
                    || !(pos..row.len()).any(|i| row[pos].id == prev[i].id),
                empty: row[pos].id.is_none(),
                color: 0,
            };
            let id = row[pos]
                .id
                .as_ref()
                .or(next[pos].id.as_ref())
                .cloned()
                .unwrap_or_default();
            let s = Symbol {
                color: self.color(id),
                ..s
            };
            self.row[pos].symbol = s;
            canvas.symbols.push(s);
        }
        if let Some(color) = self.colors.remove(&self.id) {
            self.counts[color] -= 1;
        }
        for i in 0..self.row.len() {
            self.prev[i] = self.row[i].clone();
            if (i == self.position && commits > 0) || self.prev[i].id.is_none() {
                self.prev[i] = self.next[i].clone();
            }
            self.row[i] = self.next[i].clone();
        }
        self.prev_position = self.position;
        self.position = 0;
        self.parents.clear();
        while self.row.len() > 1 && self.row.last().unwrap().id.is_none() {
            self.row.pop();
            self.prev.pop();
            self.next.pop();
        }
        canvas
    }
    pub fn render_commit(&mut self, id: &str, parents: &[&str], boundary: bool) -> Canvas {
        self.add_commit(id, parents, boundary);
        self.render_parents()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linear_merge_and_boundary() {
        let mut g = Graph::new();
        assert_eq!(g.render_commit("a", &["b", "c"], false).render(true), "M-.");
        assert_eq!(g.render_commit("b", &["d"], false).render(true), "* |");
        assert_eq!(g.render_commit("c", &["d"], false).render(true), "| *");
        assert_eq!(g.render_commit("d", &[], false).render(true), "I-'");
        assert_eq!(
            Graph::new().render_commit("x", &[], true).render(false),
            "◯"
        );
    }
    #[test]
    fn raw_parents_and_colors() {
        let mut g = Graph::new();
        g.add_commit("a", &[], false);
        g.add_parent(Some("b"));
        let canvas = g.render_parents();
        assert_eq!(canvas.render(false), "∙");
        assert_eq!(canvas.symbols[0].color_id(), -1);
        assert!(!canvas.is_merge());
        assert_eq!(g.render_commit("b", &[], false).render(false), "◎");
    }
}

#[cfg(test)]
mod golden_tests {
    use super::*;
    #[test]
    fn upstream_graph_fixtures() {
        let fixtures = [
            include_str!("../test/graph/00-simple-test"),
            include_str!("../test/graph/01-merge-from-left-test"),
            include_str!("../test/graph/02-duplicate-parent-test"),
            include_str!("../test/graph/03-octo-merge-test"),
            include_str!("../test/graph/04-missing-bar-test"),
            include_str!("../test/graph/05-extra-pipe-test"),
            include_str!("../test/graph/06-extra-bars-test"),
            include_str!("../test/graph/07-multi-collapse-test"),
            include_str!("../test/graph/08-multi-collapse-2-test"),
            include_str!("../test/graph/09-parallel-siblings-test"),
            include_str!("../test/graph/10-shorter-merge-than-branch-test"),
            include_str!("../test/graph/11-new-branch-in-middle-test"),
            include_str!("../test/graph/12-cross-over-collapse-test"),
            include_str!(
                "../test/graph/13-collapse-parallel-branches-with-different-middle-branch-test"
            ),
            include_str!("../test/graph/14-long-collapse-line-test"),
            include_str!("../test/graph/15-many-merges-test"),
            include_str!("../test/graph/16-changes-test"),
            include_str!("../test/graph/17-more-merges-test"),
            include_str!("../test/graph/18-tig-test"),
            include_str!("../test/graph/19-tig-all-test"),
        ];
        for fixture in fixtures {
            let input = fixture
                .split("test_graph <<EOF\n")
                .nth(1)
                .unwrap()
                .split("\nEOF")
                .next()
                .unwrap();
            let expected = fixture
                .split("assert_equals stdout <<EOF\n")
                .nth(1)
                .unwrap()
                .split("\nEOF")
                .next()
                .unwrap();
            let mut graph = Graph::new();
            let mut canvas = None;
            let mut actual = Vec::new();
            for line in input.lines() {
                if let Some(ids) = line.strip_prefix("commit ") {
                    let boundary = ids.starts_with('-');
                    let ids = ids.strip_prefix('-').unwrap_or(ids);
                    let ids: Vec<_> = ids.split(' ').collect();
                    canvas = Some(graph.render_commit(ids[0], &ids[1..], boundary));
                } else if let Some(title) = line.strip_prefix("    ") {
                    if let Some(canvas) = canvas.take() {
                        actual.push(format!("{} {}", canvas.render(false), title));
                    }
                }
            }
            assert_eq!(actual.join("\n"), expected);
        }
    }
}
