// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// Rust port of Tig graph-v1.c. SPDX-License-Identifier: GPL-2.0-or-later
//! Original Tig v1 commit graph, retaining its column and symbol semantics.

#[derive(Clone, Copy, Debug, Default)]
pub struct Symbol {
    pub color: usize,
    commit: bool,
    branch: bool,
    boundary: bool,
    initial: bool,
    merge: bool,
    vbranch: bool,
    branched: bool,
}

impl Symbol {
    pub fn color_id(&self) -> i32 {
        if self.commit {
            -1
        } else {
            self.color as i32
        }
    }
    pub fn ascii(&self) -> &'static str {
        self.chars(true)
    }
    pub fn utf8(&self) -> &'static str {
        self.chars(false)
    }
    fn chars(&self, ascii: bool) -> &'static str {
        let pair = if self.commit {
            if self.boundary {
                (" o", " ◯")
            } else if self.initial {
                (" I", " ◎")
            } else if self.merge {
                (" M", " ●")
            } else {
                (" *", " ∙")
            }
        } else if self.merge {
            if self.branch {
                ("-+", "━┪")
            } else if self.vbranch {
                ("-.", "━┯")
            } else {
                ("-.", "━┑")
            }
        } else if self.branch {
            if self.branched {
                if self.vbranch {
                    ("-+", "─┴")
                } else {
                    ("-'", "─┘")
                }
            } else if self.vbranch {
                ("-|", "─│")
            } else {
                (" |", " │")
            }
        } else if self.vbranch {
            ("--", "──")
        } else {
            ("  ", "  ")
        };
        if ascii {
            pair.0
        } else {
            pair.1
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Canvas {
    pub symbols: Vec<Symbol>,
}
impl Canvas {
    // V1 deliberately checks the first symbol, including when it is not the commit.
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

#[derive(Clone, Debug, Default)]
struct Column {
    id: String,
    symbol: Symbol,
}

#[derive(Default)]
pub struct Graph {
    row: Vec<Column>,
    parents: Vec<Column>,
    position: usize,
    expanded: usize,
    id: String,
    colors: [usize; 14],
    has_parents: bool,
    boundary: bool,
}

fn find(row: &[Column], id: &str) -> usize {
    let mut free = row.len();
    for (i, column) in row.iter().enumerate() {
        if column.id.is_empty() {
            free = i;
        } else if column.id == id {
            return i;
        }
    }
    free
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }
    fn column(&self, id: &str) -> Column {
        Column {
            id: id.to_owned(),
            symbol: Symbol {
                boundary: self.boundary,
                ..Symbol::default()
            },
        }
    }
    /// Begin a commit with parent IDs, excluding the commit ID itself.
    pub fn add_commit(&mut self, id: &str, parents: &[&str], boundary: bool) {
        self.position = find(&self.row, id);
        self.id = id.to_owned();
        self.boundary = boundary;
        self.has_parents = false;
        self.parents.clear();
        for parent in parents {
            self.add_parent(Some(parent));
        }
        if self.parents.is_empty() {
            self.add_parent(None);
        }
        self.has_parents = !parents.is_empty();
    }
    /// Match v1's supplemental-parent behavior, including its initial empty parent.
    pub fn add_parent(&mut self, parent: Option<&str>) {
        if !self.has_parents {
            self.parents.push(self.column(parent.unwrap_or("")));
        }
    }
    pub fn render_parents(&mut self) -> Canvas {
        while self.position + self.parents.len() > self.row.len() {
            self.row
                .insert(self.position + self.expanded, self.column(""));
            self.expanded += 1;
        }
        let mut canvas = Canvas::default();
        let parent_count = self.parents.len();
        let merge = parent_count > 1;
        let mut branched = false;
        for pos in 0..self.position {
            let column = &mut self.row[pos];
            let mut symbol = column.symbol;
            if !column.id.is_empty() {
                if find(&self.parents, &column.id) < self.parents.len() {
                    column.symbol.initial = true;
                }
                symbol.branch = true;
            }
            symbol.vbranch = branched;
            if column.id == self.id {
                branched = true;
                column.id.clear();
            }
            canvas.symbols.push(symbol);
        }
        for index in 0..self.parents.len() {
            let pos = self.position + index;
            let old = &mut self.row[pos];
            let new = &mut self.parents[index];
            let mut symbol = old.symbol;
            symbol.merge = merge;
            if pos == self.position {
                symbol.commit = true;
                if new.symbol.boundary {
                    symbol.boundary = true;
                } else if new.id.is_empty() {
                    symbol.initial = true;
                }
            } else if old.id == new.id {
                symbol.vbranch = true;
                symbol.branch = true;
            } else if merge {
                symbol.merge = true;
                symbol.vbranch = index + 1 != parent_count;
            } else if !old.id.is_empty() {
                symbol.branch = true;
            }
            canvas.symbols.push(symbol);
            if old.id.is_empty() {
                let color = (0..14).min_by_key(|&i| self.colors[i]).unwrap();
                self.colors[color] += 1;
                new.symbol.color = color;
            }
            *old = new.clone();
        }
        for pos in self.position + self.parents.len()..self.row.len() {
            let too = self.row.last().is_some_and(|c| c.id == self.id);
            let last = pos + 1 == self.row.len();
            let column = &mut self.row[pos];
            let mut symbol = column.symbol;
            symbol.vbranch = too;
            if !column.id.is_empty() {
                symbol.branch = true;
                if column.id == self.id {
                    symbol.branched = true;
                    symbol.vbranch = too && !last;
                    column.id.clear();
                }
            }
            canvas.symbols.push(symbol);
        }
        self.parents.clear();
        self.expanded = 0;
        self.position = 0;
        while self.row.len() > 1 && self.row.last().unwrap().id.is_empty() {
            self.row.pop();
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
    fn merge_and_shared_ancestor() {
        let mut graph = Graph::new();
        let first = graph.render_commit("a", &["b", "c"], false);
        assert_eq!(first.render(true), "M-.");
        assert_eq!(first.render(false), "●━┑");
        assert!(first.is_merge());
        assert_eq!(first.symbols[0].color_id(), -1);
        assert_eq!(graph.render_commit("b", &["d"], false).render(true), "* |");
        assert_eq!(graph.render_commit("c", &["d"], false).render(true), "| *");
        assert_eq!(graph.render_commit("d", &[], false).render(false), "◎─┘");
    }
    #[test]
    fn boundary_root_and_octopus() {
        assert_eq!(
            Graph::new().render_commit("a", &[], true).render(false),
            "◯"
        );
        let mut graph = Graph::new();
        assert_eq!(
            graph
                .render_commit("a", &["b", "c", "d"], false)
                .render(false),
            "●━┯━┑"
        );
        assert_eq!(graph.render_commit("b", &[], false).render(true), "I | |");
    }
    #[test]
    fn merge_in_right_column_retains_v1_metadata() {
        let mut graph = Graph::new();
        graph.render_commit("a", &["b", "c"], false);
        graph.render_commit("b", &["d"], false);
        let canvas = graph.render_commit("c", &["e", "f"], false);
        assert_eq!(canvas.render(true), "| M-.");
        assert!(!canvas.is_merge());
        assert_eq!(
            canvas
                .symbols
                .iter()
                .map(Symbol::color_id)
                .collect::<Vec<_>>(),
            [0, -1, 1]
        );
    }
}
