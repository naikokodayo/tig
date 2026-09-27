// SPDX-License-Identifier: GPL-2.0-or-later
// Safe Rust migration of Tig's historical blob editor.
// Original Tig copyright (c) 2006-2026 Jonas Fonseca <jonas.fonseca@gmail.com>.
use crate::git::{GitError, Result};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// A private copy of immutable content. Keep this alive until the editor exits.
pub struct BlobEditorSnapshot(PathBuf);

impl BlobEditorSnapshot {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for BlobEditorSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Copy already selected immutable bytes, preserving the basename for editor syntax rules.
pub fn prepare_bytes(bytes: &[u8], displayed_name: &OsStr) -> Result<BlobEditorSnapshot> {
    let name = Path::new(displayed_name)
        .file_name()
        .unwrap_or_else(|| OsStr::new("unknown"));
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| GitError(error.to_string()))?
        .as_nanos();
    for _ in 0..16 {
        let mut filename = OsString::from(format!(
            "tigblob.{}.{}.{stamp}.",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        filename.push(name);
        let path = std::env::temp_dir().join(filename);
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(GitError(format!(
                    "Failed to create temporary blob: {error}"
                )))
            }
        };
        let snapshot = BlobEditorSnapshot(path);
        file.write_all(bytes)
            .map_err(|error| GitError(format!("Failed to save blob data: {error}")))?;
        return Ok(snapshot);
    }
    Err(GitError("Failed to create temporary blob".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_snapshot_preserves_bytes_and_cleans_up() {
        let snapshot = prepare_bytes(b"old\0bytes\n", OsStr::new("dir/example.rs")).unwrap();
        let path = snapshot.path().to_owned();
        assert_eq!(fs::read(&path).unwrap(), b"old\0bytes\n");
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".example.rs"));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(snapshot);
        assert!(!path.exists());
    }
}
