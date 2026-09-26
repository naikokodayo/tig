// SPDX-License-Identifier: GPL-2.0-or-later
//! TIG_TRACE records actual child argv and captured stderr, in append mode.
use std::{
    fs::OpenOptions,
    io::{self, Write},
    process::{Command, Output},
};

pub fn append(bytes: &[u8]) {
    if let Some(path) = std::env::var_os("TIG_TRACE").filter(|path| !path.is_empty()) {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            // Like upstream, an unavailable trace must not prevent the command.
            let _ = file.write_all(bytes);
        }
    }
}

pub fn command(command: &Command) {
    let mut line = Vec::new();
    for arg in std::iter::once(command.get_program()).chain(command.get_args()) {
        line.extend_from_slice(arg.as_encoded_bytes());
        line.push(b' ');
    }
    line.push(b'\n');
    append(&line);
}

pub fn output(child: &mut Command) -> io::Result<Output> {
    command(child);
    let result = child.output()?;
    append(&result.stderr);
    Ok(result)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn trace_records_actual_commands_appends_and_never_changes_exit_status() {
        const CHILD: &str = "TIG_TRACE_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let result = output(
                Command::new("sh").args(["-c", "printf output; printf failure >&2; exit 7"]),
            )
            .unwrap();
            assert_eq!(result.status.code(), Some(7));
            assert_eq!(result.stdout, b"output");
            assert_eq!(result.stderr, b"failure");
            let result = output(Command::new("printf").args(["%s", "a b", ""])).unwrap();
            assert!(result.status.success());
            assert_eq!(result.stdout, b"a b");
            return;
        }
        let directory = std::env::temp_dir().join(format!("tig-trace-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("trace");
        std::fs::write(&path, b"previous\n").unwrap();
        for trace in [Some(&path), Some(&directory), None] {
            // Each case owns its environment; parallel tests cannot change it.
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "trace::tests::trace_records_actual_commands_appends_and_never_changes_exit_status"])
                .env(CHILD, "1").env_remove("TIG_TRACE");
            if let Some(path) = trace {
                child.env("TIG_TRACE", path);
            }
            let result = child.output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"previous\nsh -c printf output; printf failure >&2; exit 7 \nfailureprintf %s a b  \n"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
