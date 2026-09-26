// SPDX-License-Identifier: GPL-2.0-or-later
// Date semantics ported from Tig src/util.c (Jonas Fonseca, 2006-2026).
use chrono::{DateTime, FixedOffset, Utc};
use std::{env, fmt::Write, process::Command, time::SystemTime};

pub fn now() -> Result<DateTime<Utc>, String> {
    match env::var("TEST_TIME_NOW") {
        Ok(value) => value
            .parse::<i64>()
            .ok()
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
            .ok_or_else(|| format!("Invalid TEST_TIME_NOW: {value:?}")),
        Err(env::VarError::NotPresent) => Ok(SystemTime::now().into()),
        Err(error) => Err(format!("Invalid TEST_TIME_NOW: {error}")),
    }
}

pub fn raw(value: &str) -> Result<String, String> {
    let invalid = || format!("Invalid raw commit date: {value:?}");
    let (seconds, zone) = value.split_once(' ').ok_or_else(invalid)?;
    if zone.len() != 5
        || !matches!(zone.as_bytes()[0], b'+' | b'-')
        || !zone.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(invalid());
    }
    let offset: FixedOffset = zone.parse().map_err(|_| invalid())?;
    let date = seconds
        .parse()
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .ok_or_else(invalid)?;
    Ok(date.with_timezone(&offset).to_rfc3339())
}

pub fn changes_date() -> Result<String, String> {
    let now = now()?;
    if env::var_os("TEST_TIME_NOW").is_some() {
        Ok(now.to_rfc3339())
    } else {
        let value = native_format(now.timestamp(), "%Y-%m-%dT%H:%M:%S%z", true)?;
        DateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%z")
            .map(|date| date.to_rfc3339())
            .map_err(|error| format!("Invalid current date: {error}"))
    }
}

// Keep the OS timezone and locale rules without handwritten unsafe libc calls.
// ponytail: a subprocess per local/locale date; batch if profiling warrants it.
fn native_format(seconds: i64, format: &str, local: bool) -> Result<String, String> {
    let mut command = Command::new("date");
    if cfg!(target_os = "linux") {
        command.arg("-d").arg(format!("@{seconds}"));
    } else if cfg!(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )) {
        command.arg("-r").arg(seconds.to_string());
    } else {
        return Err("Local/locale dates require GNU or BSD date".into());
    }
    if !local {
        command.env("TZ", "UTC");
    }
    let output = command
        .arg(format!("+{format}"))
        .output()
        .map_err(|error| format!("Could not run date: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "date failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.strip_suffix('\n').unwrap_or(&value).to_owned())
        .map_err(|error| format!("Invalid date output: {error}"))
}

fn relative(timestamp: i64, now: i64, compact: bool) -> String {
    let seconds = now.abs_diff(timestamp);
    let units = [
        ("second", 's', 1, 120),
        ("minute", 'm', 60, 7200),
        ("hour", 'h', 3600, 172800),
        ("day", 'D', 86400, 1209600),
        ("week", 'W', 604800, 3024000),
        ("month", 'M', 2592000, 31536000),
        ("year", 'Y', 31536000, u64::MAX),
    ];
    let (name, symbol, divisor, _) = units
        .iter()
        .find(|unit| seconds < unit.3)
        .unwrap_or(&units[6]);
    let count = seconds / divisor;
    if compact {
        format!("{}{count}{symbol}", if now < timestamp { "-" } else { "" })
    } else {
        format!(
            "{count} {name}{} {}",
            if count > 1 { "s" } else { "" },
            if now < timestamp { "ahead" } else { "ago" }
        )
    }
}

pub fn format(
    iso: &str,
    display: &str,
    local: bool,
    custom: Option<&str>,
) -> Result<String, String> {
    let date = DateTime::parse_from_rfc3339(iso)
        .map_err(|error| format!("Invalid ISO 8601 commit date {iso:?}: {error}"))?;
    // C stores the commit's wall time in time->sec and treats zero as absent.
    // Preserve the valid instant/offset; only suppress its displayed date.
    if date.naive_local().and_utc().timestamp() == 0 {
        return Ok(String::new());
    }
    if matches!(display, "relative" | "relative-compact") {
        return Ok(relative(
            date.timestamp(),
            now()?.timestamp(),
            display == "relative-compact",
        ));
    }
    let default = if local {
        "%Y-%m-%d %H:%M"
    } else {
        "%Y-%m-%d %H:%M %z"
    };
    let format = match display {
        "default" | "yes" | "true" => default,
        "custom" => custom.unwrap_or(default),
        other => return Err(format!("Unsupported date mode: {other}")),
    };
    format_date(date, format, local)
}

fn format_date(date: DateTime<FixedOffset>, format: &str, local: bool) -> Result<String, String> {
    let mut normalized = String::new();
    let mut native = local;
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            normalized.push(c);
            continue;
        }
        let spec = chars.next().ok_or("Trailing % in date format")?;
        // Locale-sensitive directives must use strftime, not Chrono's English defaults.
        if !"%YymdHMSFRTzZ".contains(spec) {
            if !"aAbBcCDeGghIjklpnPrstTuUVwWxX".contains(spec) {
                return Err(format!("Unsupported date format directive: %{spec}"));
            }
            native = true;
        }
        if !local && matches!(spec, 'z' | 'Z') {
            write!(normalized, "{}", date.format("%z")).map_err(|error| error.to_string())?;
        } else {
            normalized.push('%');
            normalized.push(spec);
        }
    }
    if native {
        let seconds = if local {
            date.timestamp()
        } else {
            date.naive_local().and_utc().timestamp()
        };
        native_format(seconds, &normalized, local)
    } else {
        let mut output = String::new();
        write!(output, "{}", date.format(&normalized))
            .map_err(|_| format!("Invalid date format: {format:?}"))?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tig_dates_and_relative_boundaries() {
        let iso = raw("1440961292 +0900").unwrap();
        assert_eq!(iso, "2015-08-31T04:01:32+09:00");
        assert_eq!(
            format(&iso, "custom", false, Some("%F z=%z Z=%Z %%Z")).unwrap(),
            "2015-08-31 z=+0900 Z=+0900 %Z"
        );
        assert_eq!(relative(1440961292, 1441051553, false), "25 hours ago");
        for (seconds, verbose, compact) in [
            (0, "0 second ago", "0s"),
            (119, "119 seconds ago", "119s"),
            (120, "2 minutes ago", "2m"),
            (7199, "119 minutes ago", "119m"),
            (7200, "2 hours ago", "2h"),
            (172799, "47 hours ago", "47h"),
            (172800, "2 days ago", "2D"),
            (1209599, "13 days ago", "13D"),
            (1209600, "2 weeks ago", "2W"),
            (3023999, "4 weeks ago", "4W"),
            (3024000, "1 month ago", "1M"),
            (31535999, "12 months ago", "12M"),
            (31536000, "1 year ago", "1Y"),
        ] {
            assert_eq!(relative(0, seconds, false), verbose);
            assert_eq!(relative(0, seconds, true), compact);
            if seconds > 0 {
                assert_eq!(relative(seconds, 0, false), verbose.replace("ago", "ahead"));
                assert_eq!(relative(seconds, 0, true), format!("-{compact}"));
            }
        }
        assert!(format("2023-02-29T00:00:00Z", "default", false, None).is_err());
        for invalid in ["%", "%Q", "%#z", "%Ec", "%Od"] {
            assert!(format(&iso, "custom", false, Some(invalid)).is_err());
        }
        assert!(raw("1440961292 +2460").is_err());
        assert!(raw("9223372036854775807 +0000").is_err());
        assert_eq!(raw("-1 +0000").unwrap(), "1969-12-31T23:59:59+00:00");
    }
}
