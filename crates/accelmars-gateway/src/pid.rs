use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Information written to the PID file on `gateway serve` startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PidInfo {
    pub pid: u32,
    pub port: u16,
    /// ISO-8601 UTC timestamp of when the server started.
    pub started: String,
}

/// Default PID file path: `~/.accelmars-gateway.pid`
///
/// Respects `GATEWAY_PID_PATH` env var for override (useful in tests).
pub fn default_path() -> PathBuf {
    std::env::var("GATEWAY_PID_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir())
                .join(".accelmars-gateway.pid")
        })
}

/// Write PID info to the default PID file path.
pub fn write(info: &PidInfo) -> anyhow::Result<()> {
    let path = default_path();
    let json = serde_json::to_string(info)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Read PID info from the default PID file path. Returns None if missing or malformed.
pub fn read() -> Option<PidInfo> {
    let contents = std::fs::read_to_string(default_path()).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Delete the PID file. Best-effort — no error on missing.
pub fn cleanup() {
    let _ = std::fs::remove_file(default_path());
}

/// Returns true if a process with the given PID is currently running.
///
/// Uses `kill(pid, 0)` on Unix — checks process existence without sending a signal.
#[cfg(unix)]
pub fn is_alive(pid: u32) -> bool {
    // SAFETY: kill(pid, 0) only checks process existence, sends no signal.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
pub fn is_alive(_pid: u32) -> bool {
    // Windows is not a current target; assume alive to fail-safe.
    false
}

/// Generate a minimal ISO-8601 UTC timestamp from the current time.
pub fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_to_iso(secs)
}

fn epoch_to_iso(epoch: u64) -> String {
    let sec = (epoch % 60) as u32;
    let epoch = epoch / 60;
    let min = (epoch % 60) as u32;
    let epoch = epoch / 60;
    let hour = (epoch % 24) as u32;
    let mut days = epoch / 24;

    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    let day = days as u32 + 1;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn is_leap(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

// ---------------------------------------------------------------------------
// Internal helpers (testable without env var races)
// ---------------------------------------------------------------------------

/// Write PID info to an explicit path.
pub fn write_at(path: &std::path::Path, info: &PidInfo) -> anyhow::Result<()> {
    let json = serde_json::to_string(info)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Read PID info from an explicit path. Returns None if missing or malformed.
pub fn read_at(path: &std::path::Path) -> Option<PidInfo> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Delete the file at an explicit path. Best-effort.
pub fn cleanup_at(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_pid_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            ".test-gateway-pid-{}-{}.pid",
            std::process::id(),
            tag
        ))
    }

    #[test]
    fn write_read_roundtrip() {
        let path = tmp_pid_path("roundtrip");
        let _ = std::fs::remove_file(&path); // start clean

        let info = PidInfo {
            pid: 12345,
            port: 8090,
            started: "2026-04-22T00:00:00Z".to_string(),
        };

        write_at(&path, &info).expect("write_at should succeed");
        let read_back = read_at(&path).expect("read_at should return Some");
        assert_eq!(read_back.pid, 12345);
        assert_eq!(read_back.port, 8090);
        assert_eq!(read_back.started, "2026-04-22T00:00:00Z");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_returns_none_on_missing_file() {
        let path = tmp_pid_path("missing");
        let _ = std::fs::remove_file(&path); // ensure it doesn't exist
        assert!(
            read_at(&path).is_none(),
            "read_at should return None when file is missing"
        );
    }

    #[cfg(unix)]
    #[test]
    fn is_alive_true_for_own_pid() {
        let own_pid = std::process::id();
        assert!(is_alive(own_pid), "our own PID should be alive");
    }

    #[cfg(unix)]
    #[test]
    fn is_alive_false_for_nonexistent_pid() {
        // PID 99999999 is extremely unlikely to exist
        assert!(!is_alive(99_999_999), "nonexistent PID should not be alive");
    }

    #[test]
    fn cleanup_removes_file() {
        let path = tmp_pid_path("cleanup");
        std::fs::write(&path, b"{}").expect("test setup write");
        assert!(path.exists(), "file should exist before cleanup");
        cleanup_at(&path);
        assert!(!path.exists(), "file should be removed after cleanup");
    }

    #[test]
    fn cleanup_is_noop_on_missing_file() {
        let path = tmp_pid_path("cleanup-noop");
        let _ = std::fs::remove_file(&path);
        // Should not panic
        cleanup_at(&path);
    }

    #[test]
    fn iso_now_produces_valid_format() {
        let ts = iso_now();
        // Should look like: 2026-04-22T14:30:00Z
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
        assert!(
            ts.contains('T'),
            "timestamp should contain T separator: {ts}"
        );
        assert_eq!(ts.len(), 20, "timestamp should be 20 chars: {ts}");
    }
}
