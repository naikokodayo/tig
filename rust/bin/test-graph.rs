// Copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>
// SPDX-License-Identifier: GPL-2.0-or-later
use std::io::{self, BufRead, IsTerminal, Write};
use tig_rs::graph;
fn main() -> io::Result<()> {
    if io::stdin().is_terminal() {
        eprintln!("test-graph [--ascii]\n\nExample usage:\n\t# git log --pretty=raw --parents | ./test-graph\n\t# git log --pretty=raw --parents | ./test-graph --ascii");
        std::process::exit(1);
    }
    let ascii = std::env::args().nth(1).as_deref() == Some("--ascii");
    let mut graph = graph::Graph::new();
    let mut canvas = None;
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().split(b'\n') {
        let line = line?;
        if let Some(header) = line.strip_prefix(b"commit ") {
            let boundary = header.starts_with(b"-");
            let header = if boundary { &header[1..] } else { header };
            let mut fields = header.splitn(2, |b| *b == 0);
            let ids = String::from_utf8_lossy(fields.next().unwrap());
            let ids: Vec<&str> = ids.split(' ').collect();
            let rendered = graph.render_commit(ids[0], &ids[1..], boundary);
            if let Some(title) = fields.next() {
                write!(out, "{} ", rendered.render(ascii))?;
                out.write_all(title)?;
                out.write_all(b"\n")?;
                canvas = None;
            } else {
                canvas = Some(rendered);
            }
        } else if let Some(title) = line.strip_prefix(b"    ") {
            if let Some(rendered) = canvas.take() {
                write!(out, "{} ", rendered.render(ascii))?;
                out.write_all(title)?;
                out.write_all(b"\n")?;
            }
        }
    }
    out.flush()
}
