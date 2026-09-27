// SPDX-License-Identifier: GPL-2.0-or-later
// Safe port of Tig's stage.c hunk/line selection semantics.
// Original Tig © 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>.
use crate::git::{GitError, Repository, Result};
use std::io::Write;
use std::ops::Range;
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub struct Patch {
    pub files: Vec<FilePatch>,
}
pub struct SplitHunk {
    pub range: Range<usize>,
    pub patch: Vec<u8>,
    pub display: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct FilePatch {
    pub headers: Vec<Vec<u8>>,
    pub hunks: Vec<Hunk>,
}
#[derive(Clone, Debug)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub suffix: Vec<u8>,
    pub lines: Vec<Vec<u8>>,
}
fn error(message: &str) -> GitError {
    GitError(message.into())
}
fn canonical_path<'a>(
    line: &'a [u8],
    marker: &[u8],
    prefix: &[u8],
) -> Result<Option<(&'a [u8], bool)>> {
    let value = line
        .strip_prefix(marker)
        .ok_or_else(|| error("Missing patch file header"))?;
    let value = value.strip_suffix(b"\t").unwrap_or(value);
    if value == b"/dev/null" {
        return Ok(None);
    }
    let quoted = value.starts_with(b"\"");
    let value = if quoted {
        value
            .strip_prefix(b"\"")
            .and_then(|value| value.strip_suffix(b"\""))
            .ok_or_else(|| error("Malformed quoted patch path"))?
    } else {
        value
    };
    let path = value
        .strip_prefix(prefix)
        .ok_or_else(|| error("Patch has a noncanonical path prefix"))?;
    if path.is_empty()
        || path
            .split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b"..")
        || path
            .iter()
            .any(|byte| *byte < b' ' || *byte == 0x7f || *byte == b'\\' || *byte == b'"')
    {
        return Err(error("Patch path is not repository-relative"));
    }
    Ok(Some((path, quoted)))
}
fn validate_apply_paths(patch: &Patch) -> Result<()> {
    for file in &patch.files {
        let (old, new) = if file.hunks.is_empty() {
            // A mode-only diff has no ---/+++ paths. Its two canonical paths
            // must be identical, so their encoded lengths are equal, even with
            // spaces or quotes. The exact header comparison below checks both.
            let paths = file.headers[0]
                .strip_prefix(b"diff --git ")
                .ok_or_else(|| error("Missing patch file header"))?;
            let middle = paths.len() / 2;
            if paths.get(middle) != Some(&b' ') {
                return Err(error("Malformed mode-only patch paths"));
            }
            (
                canonical_path(&paths[..middle], b"", b"a/")?,
                canonical_path(&paths[middle + 1..], b"", b"b/")?,
            )
        } else {
            let old = canonical_path(
                file.headers
                    .iter()
                    .find(|line| line.starts_with(b"--- "))
                    .ok_or_else(|| error("Missing old file header"))?,
                b"--- ",
                b"a/",
            )?;
            let new = canonical_path(
                file.headers
                    .iter()
                    .find(|line| line.starts_with(b"+++ "))
                    .ok_or_else(|| error("Missing new file header"))?,
                b"+++ ",
                b"b/",
            )?;
            (old, new)
        };
        let (path, quoted) = match (old, new) {
            (Some(old), Some(new)) if old == new => old,
            (Some(old), None) | (None, Some(old)) => old,
            _ => return Err(error("Renamed or missing patch paths are unsupported")),
        };
        let mut expected = if quoted {
            b"diff --git \"a/".to_vec()
        } else {
            b"diff --git a/".to_vec()
        };
        expected.extend_from_slice(path);
        expected.extend_from_slice(if quoted { b"\" \"b/" } else { b" b/" });
        expected.extend_from_slice(path);
        if quoted {
            expected.push(b'"');
        }
        if file.headers.first() != Some(&expected) {
            return Err(error("Patch file header does not match its path"));
        }
    }
    Ok(())
}
fn range(value: &str, prefix: char) -> Result<(usize, usize)> {
    let value = value
        .strip_prefix(prefix)
        .ok_or_else(|| error("Invalid hunk range"))?;
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    let start = start.parse().map_err(|_| error("Invalid hunk start"))?;
    let count = count.parse().map_err(|_| error("Invalid hunk count"))?;
    if start == 0 && count != 0 {
        return Err(error("Nonempty hunk range starts at zero"));
    }
    Ok((start, count))
}
impl Hunk {
    fn parse_header(line: &[u8]) -> Result<Self> {
        let end = line
            .windows(3)
            .position(|w| w == b" @@")
            .ok_or_else(|| error("Invalid hunk header"))?;
        let fields = std::str::from_utf8(
            line.get(3..end)
                .ok_or_else(|| error("Invalid hunk header"))?,
        )
        .map_err(|_| error("Invalid hunk header encoding"))?;
        let (old, new) = fields
            .split_once(' ')
            .ok_or_else(|| error("Missing hunk range"))?;
        let (old_start, old_count) = range(old, '-')?;
        let (new_start, new_count) = range(new, '+')?;
        Ok(Self {
            old_start,
            old_count,
            new_start,
            new_count,
            suffix: line[end + 3..].to_vec(),
            lines: Vec::new(),
        })
    }
    fn change_range(&self, index: usize) -> Result<Range<usize>> {
        if !self
            .lines
            .get(index)
            .is_some_and(|row| matches!(row.first(), Some(b'+' | b'-')))
        {
            return Err(error("Select an added or removed line"));
        }
        let is_change = |row: &Vec<u8>| matches!(row.first(), Some(b'+' | b'-' | b'\\'));
        let start = self.lines[..index]
            .iter()
            .rposition(|row| !is_change(row))
            .map_or(0, |i| i + 1);
        let end = self.lines[index..]
            .iter()
            .position(|row| !is_change(row))
            .map_or(self.lines.len(), |i| index + i);
        Ok(start..end)
    }
    fn write(&self, output: &mut Vec<u8>) -> Result<()> {
        let (old, new) = self.counts()?;
        write!(
            output,
            "@@ -{},{} +{},{} @@",
            self.old_start, old, self.new_start, new
        )
        .map_err(|e| error(&e.to_string()))?;
        output.extend_from_slice(&self.suffix);
        output.push(b'\n');
        for row in &self.lines {
            output.extend_from_slice(row);
            output.push(b'\n');
        }
        Ok(())
    }
    fn counts(&self) -> Result<(usize, usize)> {
        let mut old = 0;
        let mut new = 0;
        let mut previous_content = false;
        for line in &self.lines {
            match line.first() {
                Some(b' ') => {
                    old += 1;
                    new += 1;
                    previous_content = true;
                }
                Some(b'-') => {
                    old += 1;
                    previous_content = true;
                }
                Some(b'+') => {
                    new += 1;
                    previous_content = true;
                }
                Some(b'\\') if line == b"\\ No newline at end of file" && previous_content => {
                    previous_content = false;
                }
                _ => return Err(error("Invalid hunk body or no-newline marker")),
            }
        }
        Ok((old, new))
    }
}
impl Patch {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            return Err(error("Truncated patch"));
        }
        let mut files: Vec<FilePatch> = Vec::new();
        for line in bytes
            .split(|b| *b == b'\n')
            .take(bytes.iter().filter(|b| **b == b'\n').count())
        {
            if line.starts_with(b"diff --cc ")
                || line.starts_with(b"diff --combined ")
                || line.starts_with(b"@@@")
            {
                return Err(error("Combined merge patches are unsupported"));
            }
            if line.starts_with(b"diff --git ") {
                files.push(FilePatch {
                    headers: vec![line.to_vec()],
                    hunks: Vec::new(),
                });
                continue;
            }
            let file = files
                .last_mut()
                .ok_or_else(|| error("Expected a Git unified diff header"))?;
            if line.starts_with(b"GIT binary patch") || line.starts_with(b"Binary files ") {
                return Err(error("Binary patches are unsupported"));
            }
            if line.starts_with(b"@@ ") {
                file.hunks.push(Hunk::parse_header(line)?);
            } else if let Some(hunk) = file.hunks.last_mut() {
                hunk.lines.push(line.to_vec());
            } else {
                file.headers.push(line.to_vec());
            }
        }
        for file in &files {
            if file.hunks.is_empty() {
                if file.headers.len() != 3
                    || !matches!(
                        (file.headers[1].as_slice(), file.headers[2].as_slice()),
                        (b"old mode 100644", b"new mode 100755")
                            | (b"old mode 100755", b"new mode 100644")
                    )
                {
                    return Err(error("Unsupported patch without text hunks"));
                }
                continue;
            }
            if !file.headers.iter().any(|l| l.starts_with(b"--- "))
                || !file.headers.iter().any(|l| l.starts_with(b"+++ "))
            {
                return Err(error("Missing file headers"));
            }
            for hunk in &file.hunks {
                if hunk.counts()? != (hunk.old_count, hunk.new_count) {
                    return Err(error("Hunk body does not match declared counts"));
                }
            }
        }
        if files.is_empty() {
            return Err(error("Patch contains no files"));
        }
        Ok(Self { files })
    }
    /// Map a zero-based raw patch display row to a hunk and optional body row.
    /// File header rows select that file's first hunk.
    pub fn locate(&self, raw_line: usize) -> Result<(usize, usize, Option<usize>)> {
        let mut start = 0;
        for (file_index, file) in self.files.iter().enumerate() {
            if raw_line < start + file.headers.len() {
                return Ok((file_index, 0, None));
            }
            start += file.headers.len();
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                if raw_line == start {
                    return Ok((file_index, hunk_index, None));
                }
                start += 1;
                if raw_line < start + hunk.lines.len() {
                    return Ok((file_index, hunk_index, Some(raw_line - start)));
                }
                start += hunk.lines.len();
            }
        }
        Err(error("Patch row out of range"))
    }
    /// Split at context separating changes, sharing that context on both sides.
    /// Returns the replacement hunk bytes and its raw row range; never writes Git.
    pub fn split_hunk(&self, file: usize, hunk: usize) -> Result<SplitHunk> {
        let source = self
            .files
            .get(file)
            .and_then(|file| file.hunks.get(hunk))
            .ok_or_else(|| error("Hunk index out of range"))?;
        let mut changes = Vec::new();
        let mut index = 0;
        let counted_end = source
            .lines
            .iter()
            .position(|row| row.starts_with(b"\\"))
            .unwrap_or(source.lines.len());
        while index < counted_end {
            if matches!(source.lines[index].first(), Some(b'+' | b'-')) {
                let range = source.change_range(index)?;
                index = range.end;
                changes.push(range);
            } else {
                index += 1;
            }
        }
        if changes.len() < 2 {
            return Err(error("The chunk cannot be split"));
        }
        let mut output = Vec::new();
        let mut display = Vec::new();
        let mut old = source.old_start;
        let mut new = source.new_start;
        let mut previous_start = 0;
        for (index, _) in changes.iter().enumerate() {
            let start = if index == 0 {
                0
            } else {
                changes[index - 1].end
            };
            let end = changes
                .get(index + 1)
                .map_or(source.lines.len(), |range| range.start);
            for row in &source.lines[previous_start..start] {
                old = old
                    .checked_add(usize::from(matches!(row.first(), Some(b' ' | b'-'))))
                    .ok_or_else(|| error("Hunk position overflow"))?;
                new = new
                    .checked_add(usize::from(matches!(row.first(), Some(b' ' | b'+'))))
                    .ok_or_else(|| error("Hunk position overflow"))?;
            }
            let split = Hunk {
                old_start: old,
                new_start: new,
                suffix: Vec::new(),
                lines: source.lines[start..end].to_vec(),
                old_count: 0,
                new_count: 0,
            };
            let offset = output.len();
            split.write(&mut output)?;
            let mut rows = String::from_utf8_lossy(&output[offset..])
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            // C stops counting the display header at the first no-newline marker.
            // Keep the complete, validated patch separately for subsequent staging.
            if counted_end < end {
                let mut counted = split.clone();
                counted.lines.truncate(counted_end.saturating_sub(start));
                let (old_count, new_count) = counted.counts()?;
                rows[0] = format!("@@ -{old},{old_count} +{new},{new_count} @@");
            }
            display.extend(rows);
            previous_start = start;
        }
        let start = self.files[..file]
            .iter()
            .map(|file| {
                file.headers.len()
                    + file
                        .hunks
                        .iter()
                        .map(|hunk| 1 + hunk.lines.len())
                        .sum::<usize>()
            })
            .sum::<usize>()
            + self.files[file].headers.len()
            + self.files[file].hunks[..hunk]
                .iter()
                .map(|hunk| 1 + hunk.lines.len())
                .sum::<usize>();
        Ok(SplitHunk {
            range: start..start + 1 + source.lines.len(),
            patch: output,
            display,
        })
    }
    /// `line` indexes the hunk body (including marker rows). `reverse` selects
    /// changes from the index side for unstage, then apply_cached uses -R.
    pub fn select(
        &self,
        file: usize,
        hunk: usize,
        line: Option<usize>,
        reverse: bool,
    ) -> Result<Vec<u8>> {
        self.select_range(
            file,
            hunk,
            line.map(|line| line..line.saturating_add(1)),
            reverse,
        )
    }
    pub fn select_part(
        &self,
        file: usize,
        hunk: usize,
        line: usize,
        reverse: bool,
    ) -> Result<Vec<u8>> {
        let source = self
            .files
            .get(file)
            .and_then(|file| file.hunks.get(hunk))
            .ok_or_else(|| error("Hunk index out of range"))?;
        self.select_range(file, hunk, Some(source.change_range(line)?), reverse)
    }
    fn select_range(
        &self,
        file: usize,
        hunk: usize,
        lines: Option<Range<usize>>,
        reverse: bool,
    ) -> Result<Vec<u8>> {
        let file = self
            .files
            .get(file)
            .ok_or_else(|| error("File index out of range"))?;
        if file.hunks.is_empty() {
            if hunk != 0 || lines.is_some() {
                return Err(error("Mode-only patches have no text lines"));
            }
            let mut output = file.headers.join(&b'\n');
            output.push(b'\n');
            return Ok(output);
        }
        let source = file
            .hunks
            .get(hunk)
            .ok_or_else(|| error("Hunk index out of range"))?;
        if file.headers.iter().any(|l| {
            [b"rename ".as_slice(), b"copy ", b"similarity index "]
                .iter()
                .any(|p| l.starts_with(p))
        }) {
            return Err(error(
                "Partial rename and copy changes are unsupported; stage the whole file",
            ));
        }
        // C carries executable-bit changes with the selected text. Keep Git's
        // checked, atomic index apply, but do not partially change file types.
        if file.headers.iter().any(|header| {
            header
                .strip_prefix(b"old mode ")
                .or_else(|| header.strip_prefix(b"new mode "))
                .is_some_and(|mode| mode != b"100644" && mode != b"100755")
        }) {
            return Err(error(
                "Partial file type changes are unsupported; stage the whole file",
            ));
        }
        let mut selected = source.clone();
        if let Some(range) = lines {
            let added = file.headers.iter().any(|l| l == b"--- /dev/null");
            let deleted = file.headers.iter().any(|l| l == b"+++ /dev/null");
            let restore_regular = reverse
                && file.headers.iter().any(|l| {
                    matches!(
                        l.as_slice(),
                        b"deleted file mode 100644" | b"deleted file mode 100755"
                    )
                });
            if added || (deleted && !restore_regular) {
                return Err(error(
                    "Partial selection of this added/deleted file is unsupported; select the hunk",
                ));
            }
            let row = source
                .lines
                .get(range.start)
                .ok_or_else(|| error("Line index out of range"))?;
            if !matches!(row.first(), Some(b'+' | b'-')) {
                return Err(error("Select an added or removed line"));
            }
            selected.lines.clear();
            let mut retained = false;
            for (i, row) in source.lines.iter().enumerate() {
                if row.first() == Some(&b'\\') {
                    if retained {
                        selected.lines.push(row.clone());
                    }
                    continue;
                }
                retained = true;
                if range.contains(&i) || row[0] == b' ' {
                    selected.lines.push(row.clone());
                } else if row[0] == if reverse { b'+' } else { b'-' } {
                    let mut context = row.clone();
                    context[0] = b' ';
                    selected.lines.push(context);
                } else {
                    retained = false;
                }
            }
        }
        let (old, new) = selected.counts()?;
        // A single selected hunk is applied independently: preceding unselected
        // changes must not shift its output-side position.
        let base = if reverse {
            source.new_start
        } else {
            source.old_start
        };
        let base_count = if reverse {
            source.new_count
        } else {
            source.old_count
        };
        let base = if base_count == 0 {
            base.checked_add(1)
                .ok_or_else(|| error("Hunk position overflow"))?
        } else {
            base
        };
        selected.old_start = if old == 0 {
            base.saturating_sub(1)
        } else {
            base
        };
        selected.new_start = if new == 0 {
            base.saturating_sub(1)
        } else {
            base
        };
        let mut output = Vec::new();
        for header in &file.headers {
            output.extend_from_slice(header);
            output.push(b'\n');
        }
        selected.write(&mut output)?;
        Ok(output)
    }
}
fn apply_once(repo: &Repository, patch: &[u8], reverse: bool, check: bool) -> Result<()> {
    let mut command = Command::new("git");
    command
        .current_dir(&repo.root)
        .args([
            "-c",
            "apply.ignoreWhitespace=no",
            "apply",
            "--cached",
            "--whitespace=nowarn",
        ])
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if reverse {
        command.arg("--reverse");
    }
    if check {
        command.arg("--check");
    }
    command.arg("-");
    crate::trace::command(&command);
    let mut child = command
        .spawn()
        .map_err(|e| error(&format!("Could not run git apply: {e}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| error("Missing Git stdin"))?;
    let bytes = patch.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let result = child.wait_with_output();
    let written = writer
        .join()
        .map_err(|_| error("Git patch writer failed"))?;
    let output = result.map_err(|e| error(&format!("Could not wait for git apply: {e}")))?;
    crate::trace::append(&output.stderr);
    // Git reports a stale executable mode as a warning with exit status 0.
    // Refuse preflight diagnostics instead of silently accepting that mismatch.
    if !output.status.success() || (check && !output.stderr.is_empty()) {
        return Err(error(&format!(
            "git apply {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    written.map_err(|e| error(&format!("Could not send patch: {e}")))?;
    Ok(())
}
/// Git applies the index patch atomically; no --reject, worktree writes or force.
/// The second invocation independently revalidates after the preflight check.
pub fn apply_cached(repo: &Repository, patch: &[u8], reverse: bool) -> Result<()> {
    let parsed = Patch::parse(patch)?;
    validate_apply_paths(&parsed)?;
    apply_once(repo, patch, reverse, true)?;
    apply_once(repo, patch, reverse, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        root: std::path::PathBuf,
        repo: Repository,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "tig-patch-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            assert!(Command::new("git")
                .current_dir(&root)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success());
            let repo = Repository::discover(&root).unwrap();
            repo.command(["config", "user.name", "Test"]).unwrap();
            repo.command(["config", "user.email", "test@example.invalid"])
                .unwrap();
            fs::write(
                root.join("space name"),
                "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\niota\nkappa\n",
            )
            .unwrap();
            repo.command(["add", "--", "space name"]).unwrap();
            repo.command(["commit", "-qm", "base"]).unwrap();
            Self { root, repo }
        }
        fn diff(&self, staged: bool) -> Vec<u8> {
            if staged {
                self.repo
                    .command([
                        "diff",
                        "--cached",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--",
                        "space name",
                    ])
                    .unwrap()
            } else {
                self.repo
                    .command(["diff", "--no-ext-diff", "--no-textconv", "--", "space name"])
                    .unwrap()
            }
        }
        fn index(&self) -> Vec<u8> {
            self.repo.command(["show", ":space name"]).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn single_line_hunk_reverse_and_failure_are_index_only() {
        let f = Fixture::new();
        let original = f.index();
        let working = b"alpha\nnew one\nbeta\nnew two\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\niota\nkappa\n";
        fs::write(f.root.join("space name"), working).unwrap();
        let patch = Patch::parse(&f.diff(false)).unwrap();
        assert_eq!(patch.locate(0).unwrap(), (0, 0, None));
        assert_eq!(
            patch.locate(patch.files[0].headers.len()).unwrap(),
            (0, 0, None)
        );
        assert_eq!(
            patch.locate(patch.files[0].headers.len() + 1).unwrap(),
            (0, 0, Some(0))
        );
        assert!(patch.locate(usize::MAX).is_err());
        let line = patch.files[0].hunks[0]
            .lines
            .iter()
            .position(|l| l == b"+new one")
            .unwrap();
        let selected = patch.select(0, 0, Some(line), false).unwrap();
        apply_cached(&f.repo, &selected, false).unwrap();
        assert!(String::from_utf8_lossy(&f.index()).contains("new one"));
        assert!(!String::from_utf8_lossy(&f.index()).contains("new two"));
        assert_eq!(fs::read(f.root.join("space name")).unwrap(), working);
        let before = f.index();
        assert!(apply_cached(&f.repo, &selected, false).is_err());
        assert_eq!(f.index(), before);
        let staged = Patch::parse(&f.diff(true)).unwrap();
        let line = staged.files[0].hunks[0]
            .lines
            .iter()
            .position(|l| l == b"+new one")
            .unwrap();
        apply_cached(
            &f.repo,
            &staged.select(0, 0, Some(line), true).unwrap(),
            true,
        )
        .unwrap();
        assert_eq!(f.index(), original);
        let patch = Patch::parse(&f.diff(false)).unwrap();
        apply_cached(&f.repo, &patch.select(0, 0, None, false).unwrap(), false).unwrap();
        assert!(String::from_utf8_lossy(&f.index()).contains("new one"));
        let staged = Patch::parse(&f.diff(true)).unwrap();
        apply_cached(&f.repo, &staged.select(0, 0, None, true).unwrap(), true).unwrap();
        assert_eq!(f.index(), original);
        assert_eq!(fs::read(f.root.join("space name")).unwrap(), working);
        f.repo.command(["add", "--", "space name"]).unwrap();
        let staged = Patch::parse(&f.diff(true)).unwrap();
        let line = staged.files[0].hunks[0]
            .lines
            .iter()
            .position(|l| l == b"+new one")
            .unwrap();
        apply_cached(
            &f.repo,
            &staged.select(0, 0, Some(line), true).unwrap(),
            true,
        )
        .unwrap();
        assert!(!String::from_utf8_lossy(&f.index()).contains("new one"));
        assert!(String::from_utf8_lossy(&f.index()).contains("new two"));
    }
    #[test]
    fn configured_diff_prefix_cannot_stage_a_different_path() {
        let f = Fixture::new();
        fs::create_dir(f.root.join("sub")).unwrap();
        for path in ["foo", "sub/foo"] {
            fs::write(f.root.join(path), b"original\n").unwrap();
        }
        f.repo.command(["add", "--", "foo", "sub/foo"]).unwrap();
        f.repo
            .command(["commit", "-qm", "same content different paths"])
            .unwrap();
        f.repo.command(["config", "diff.noprefix", "true"]).unwrap();
        f.repo.command(["config", "diff.relative", "true"]).unwrap();
        f.repo.command(["config", "color.diff", "always"]).unwrap();
        fs::write(f.root.join("sub/foo"), b"original\nselected\n").unwrap();
        let raw = f
            .repo
            .diff_bytes(false, Some(std::path::Path::new("sub/foo")))
            .unwrap();
        let offset = raw.windows(11).position(|s| s == b"diff --git ").unwrap();
        let patch = Patch::parse(&raw[offset..]).unwrap();
        apply_cached(&f.repo, &patch.select(0, 0, None, false).unwrap(), false).unwrap();
        assert_eq!(f.repo.command(["show", ":foo"]).unwrap(), b"original\n");
        assert_eq!(
            f.repo.command(["show", ":sub/foo"]).unwrap(),
            b"original\nselected\n"
        );
        assert_eq!(fs::read(f.root.join("foo")).unwrap(), b"original\n");
        let cached = f
            .repo
            .diff_bytes(true, Some(std::path::Path::new("sub/foo")))
            .unwrap();
        let offset = cached
            .windows(11)
            .position(|s| s == b"diff --git ")
            .unwrap();
        let patch = Patch::parse(&cached[offset..]).unwrap();
        apply_cached(&f.repo, &patch.select(0, 0, None, true).unwrap(), true).unwrap();
        assert_eq!(f.repo.command(["show", ":sub/foo"]).unwrap(), b"original\n");
    }
    #[test]
    fn unprefixed_patch_cannot_stage_same_named_root_file() {
        let f = Fixture::new();
        fs::create_dir(f.root.join("sub")).unwrap();
        for path in ["foo", "sub/foo"] {
            fs::write(f.root.join(path), b"original\n").unwrap();
        }
        f.repo.command(["add", "--", "foo", "sub/foo"]).unwrap();
        f.repo.command(["commit", "-qm", "base"]).unwrap();
        f.repo.command(["config", "diff.noprefix", "true"]).unwrap();
        fs::write(f.root.join("sub/foo"), b"original\nselected\n").unwrap();
        let raw = f
            .repo
            .worktree_diff_bytes(Some(std::path::Path::new("sub/foo")))
            .unwrap();
        let offset = raw
            .windows(11)
            .position(|part| part == b"diff --git ")
            .unwrap();
        let patch = Patch::parse(&raw[offset..]).unwrap();
        assert!(apply_cached(&f.repo, &patch.select(0, 0, None, false).unwrap(), false).is_err());
        assert_eq!(f.repo.command(["show", ":foo"]).unwrap(), b"original\n");
        assert_eq!(f.repo.command(["show", ":sub/foo"]).unwrap(), b"original\n");
    }
    #[test]
    fn replacement_selection_and_no_newline_markers() {
        let f = Fixture::new();
        fs::write(
            f.root.join("space name"),
            b"ALPHA\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\niota\nKAPPA",
        )
        .unwrap();
        let patch = Patch::parse(&f.diff(false)).unwrap();
        let last = patch.files[0].hunks.len() - 1;
        apply_cached(&f.repo, &patch.select(0, last, None, false).unwrap(), false).unwrap();
        assert!(f.index().ends_with(b"KAPPA"));
        assert!(f.index().starts_with(b"alpha\n"));
        let staged = Patch::parse(&f.diff(true)).unwrap();
        apply_cached(&f.repo, &staged.select(0, 0, None, true).unwrap(), true).unwrap();
        assert!(f.index().ends_with(b"kappa\n"));
        let patch = Patch::parse(&f.diff(false)).unwrap();
        let line = patch.files[0].hunks[0]
            .lines
            .iter()
            .position(|l| l == b"-alpha")
            .unwrap();
        apply_cached(
            &f.repo,
            &patch.select(0, 0, Some(line), false).unwrap(),
            false,
        )
        .unwrap();
        assert!(f.index().starts_with(b"beta\n"));
        let staged = Patch::parse(&f.diff(true)).unwrap();
        let line = staged.files[0].hunks[0]
            .lines
            .iter()
            .position(|l| l == b"-alpha")
            .unwrap();
        apply_cached(
            &f.repo,
            &staged.select(0, 0, Some(line), true).unwrap(),
            true,
        )
        .unwrap();
        assert!(f.index().starts_with(b"alpha\n"));
    }
    #[test]
    fn partial_blocks_and_split_hunks_keep_context_markers_and_index_safety() {
        let f = Fixture::new();
        let original = b"a\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
        let working = b"a CHANGED\n1\n2\nedited-too\n4\n5\nedited-too\n7\n8";
        fs::write(f.root.join("space name"), original).unwrap();
        f.repo.command(["add", "--", "space name"]).unwrap();
        f.repo
            .command(["commit", "-qm", "partial fixture"])
            .unwrap();
        fs::write(f.root.join("space name"), working).unwrap();
        let raw = f.diff(false);
        let patch = Patch::parse(&raw).unwrap();
        assert_eq!(patch.files[0].hunks.len(), 1);
        assert!(patch.select_part(0, 0, 2, false).is_err()); // context row
        for reverse in [false, true] {
            for selected in [b"-3".as_slice(), b"+edited-too", b"-10", b"+8"] {
                f.repo
                    .command(["reset", "-q", "HEAD", "--", "space name"])
                    .unwrap();
                if reverse {
                    f.repo.command(["add", "--", "space name"]).unwrap();
                }
                let row = patch.files[0].hunks[0]
                    .lines
                    .iter()
                    .position(|row| row == selected)
                    .unwrap();
                let bytes = patch.select_part(0, 0, row, reverse).unwrap();
                apply_cached(&f.repo, &bytes, reverse).unwrap();
                let mut expected =
                    String::from_utf8_lossy(if reverse { working } else { original }).into_owned();
                expected = if selected == b"-10" || selected == b"+8" {
                    if reverse {
                        expected + "\n9\n10"
                    } else {
                        expected.trim_end_matches("\n9\n10").into()
                    }
                } else if reverse {
                    expected.replacen("edited-too", "3", 1)
                } else {
                    expected.replacen("\n3\n", "\nedited-too\n", 1)
                };
                assert_eq!(f.index(), expected.as_bytes());
                assert_eq!(fs::read(f.root.join("space name")).unwrap(), working);
            }
        }
        f.repo
            .command(["reset", "-q", "HEAD", "--", "space name"])
            .unwrap();
        let index_before = fs::read(f.root.join(".git/index")).unwrap();
        let split = patch.split_hunk(0, 0).unwrap();
        assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), index_before);
        assert_eq!(fs::read(f.root.join("space name")).unwrap(), working);
        assert_eq!(
            split
                .display
                .iter()
                .filter(|row| row.starts_with("@@"))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "@@ -1,3 +1,3 @@",
                "@@ -2,5 +2,5 @@",
                "@@ -5,4 +5,4 @@",
                "@@ -8,4 +8,1 @@"
            ]
        );
        let range = split.range;
        let mut split_patch = raw
            .split_inclusive(|byte| *byte == b'\n')
            .take(range.start)
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        split_patch.extend(split.patch);
        let split = Patch::parse(&split_patch).unwrap();
        assert_eq!(split.files[0].hunks.len(), 4);
        assert_eq!(
            split.files[0]
                .hunks
                .iter()
                .map(|h| (h.old_start, h.old_count, h.new_start, h.new_count))
                .collect::<Vec<_>>(),
            [(1, 3, 1, 3), (2, 5, 2, 5), (5, 4, 5, 4), (8, 4, 8, 2)]
        );
        for index in 0..4 {
            apply_cached(
                &f.repo,
                &split.select(0, index, None, false).unwrap(),
                false,
            )
            .unwrap();
        }
        assert_eq!(f.index(), working);
        assert_eq!(fs::read(f.root.join("space name")).unwrap(), working);
    }
    #[test]
    fn executable_mode_selection_keeps_checked_index_updates() {
        let f = Fixture::new();
        let raw = b"diff --git a/space name b/space name\nold mode 100644\nnew mode 100755\n--- a/space name\n+++ b/space name\n@@ -1,3 +1,4 @@\n alpha\n+selected\n beta\n gamma\n";
        let patch = Patch::parse(raw).unwrap();
        let selected = patch.select(0, 0, Some(1), false).unwrap();
        let working = fs::read(f.root.join("space name")).unwrap();
        apply_cached(&f.repo, &selected, false).unwrap();
        assert!(f
            .repo
            .command(["ls-files", "--stage"])
            .unwrap()
            .starts_with(b"100755 "));
        let before = fs::read(f.root.join(".git/index")).unwrap();
        assert!(apply_cached(&f.repo, &selected, false).is_err());
        assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), before);
        assert_eq!(fs::read(f.root.join("space name")).unwrap(), working);
        for mode in ["120000", "160000", "100600"] {
            let raw = String::from_utf8_lossy(raw).replace("100755", mode);
            let patch = Patch::parse(raw.as_bytes()).unwrap();
            for reverse in [false, true] {
                assert!(patch.select(0, 0, None, reverse).is_err());
                assert!(patch.select(0, 0, Some(1), reverse).is_err());
                assert!(patch.select_part(0, 0, 1, reverse).is_err());
            }
        }
        assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), before);
    }
    #[cfg(unix)]
    #[test]
    fn mode_only_selection_and_stale_index_are_checked() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let path = f.root.join("space name");
        let original = f.index();
        f.repo.command(["config", "core.filemode", "true"]).unwrap();
        for (old, new) in [(0o644, 0o755), (0o755, 0o644)] {
            fs::set_permissions(&path, fs::Permissions::from_mode(old)).unwrap();
            f.repo.command(["add", "--", "space name"]).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(new)).unwrap();
            let raw = f.diff(false);
            let patch = Patch::parse(&raw).unwrap();
            assert_eq!(patch.locate(2).unwrap(), (0, 0, None));
            assert!(patch.select(0, 0, Some(0), false).is_err());
            assert!(patch.select_part(0, 0, 0, false).is_err());
            assert!(patch.split_hunk(0, 0).is_err());
            let selected = patch.select(0, 0, None, false).unwrap();
            apply_cached(&f.repo, &selected, false).unwrap();
            assert!(f
                .repo
                .command(["ls-files", "--stage"])
                .unwrap()
                .starts_with(format!("100{new:o} ").as_bytes()));
            let before = fs::read(f.root.join(".git/index")).unwrap();
            assert!(apply_cached(&f.repo, &selected, false).is_err());
            assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), before);
            apply_cached(&f.repo, &selected, true).unwrap();
            assert!(f
                .repo
                .command(["ls-files", "--stage"])
                .unwrap()
                .starts_with(format!("100{old:o} ").as_bytes()));
            assert_eq!(f.index(), original);
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                new
            );
        }
        let before = fs::read(f.root.join(".git/index")).unwrap();
        for paths in [
            "a/space name b/other name",
            "a/../x b/../x",
            "space name space name",
            "a/x b/y",
            "a/x b/x\nrename from x",
        ] {
            let raw = format!("diff --git {paths}\nold mode 100644\nnew mode 100755\n");
            assert!(apply_cached(&f.repo, raw.as_bytes(), false).is_err());
        }
        for (old, new) in [
            ("100644", "120000"),
            ("160000", "100755"),
            ("100644", "100644"),
        ] {
            let raw = format!("diff --git a/x b/x\nold mode {old}\nnew mode {new}\n");
            assert!(Patch::parse(raw.as_bytes()).is_err());
        }
        assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), before);
    }
    #[test]
    fn deleted_file_line_unstage_preserves_bytes_and_rejects_stale_index() {
        let f = Fixture::new();
        let path = f.root.join("space name");
        fs::write(&path, b"first\nlast").unwrap();
        f.repo.command(["add", "--", "space name"]).unwrap();
        f.repo
            .command(["commit", "-qm", "deleted fixture"])
            .unwrap();
        fs::remove_file(&path).unwrap();
        f.repo.command(["add", "-u"]).unwrap();
        let raw = f.diff(true);
        let patch = Patch::parse(&raw).unwrap();
        assert!(patch.select(0, 0, Some(0), false).is_err());
        for (row, expected) in [(0, b"first\n".as_slice()), (1, b"last".as_slice())] {
            let selected = patch.select(0, 0, Some(row), true).unwrap();
            apply_cached(&f.repo, &selected, true).unwrap();
            assert_eq!(f.index(), expected);
            let before = fs::read(f.root.join(".git/index")).unwrap();
            assert!(apply_cached(&f.repo, &selected, true).is_err());
            assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), before);
            assert!(!path.exists());
            f.repo
                .command(["rm", "--cached", "--", "space name"])
                .unwrap();
        }
        let non_regular = String::from_utf8_lossy(&raw)
            .replace("deleted file mode 100644", "deleted file mode 120000");
        assert!(Patch::parse(non_regular.as_bytes())
            .unwrap()
            .select(0, 0, Some(0), true)
            .is_err());
        let added = b"diff --git a/x b/x\nnew file mode 100644\n--- /dev/null\n+++ b/x\n@@ -0,0 +1,2 @@\n+first\n+last\n";
        let added = Patch::parse(added).unwrap();
        for reverse in [false, true] {
            assert!(added.select(0, 0, Some(0), reverse).is_err());
            assert!(added.select_part(0, 0, 0, reverse).is_err());
        }
    }
    #[test]
    fn malformed_and_unsupported_patches_fail() {
        assert!(Patch::parse(b"diff --cc x\n").is_err());
        assert!(Patch::parse(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ @@\n").is_err());
        assert!(Patch::parse(b"diff --git a/x b/x\nGIT binary patch\n").is_err());
        assert!(
            Patch::parse(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,1 @@\n-x\n+y\n")
                .is_err()
        );
        assert!(Patch::parse(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n\\ No newline at end of file\n-x\n+y\n").is_err());
        assert!(
            Patch::parse(b"diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-x\n+y").is_err()
        );
    }
}
