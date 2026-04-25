use crate::pid;

/// `gateway stop` — send SIGTERM to a running gateway server and wait for exit.
///
/// # Exit codes
/// - `0` — server stopped successfully
/// - `1` — server was not running (or no PID file found)
/// - `2` — system error (signal delivery failed)
pub fn run(port_override: Option<u16>) -> i32 {
    let pid_info = match pid::read() {
        Some(info) => info,
        None => {
            if port_override.is_some() {
                eprintln!(
                    "error: no PID file found at {} — cannot determine server PID.",
                    pid::default_path().display()
                );
                eprintln!("If the server is running, send SIGTERM manually: kill <pid>");
            } else {
                eprintln!(
                    "error: no running gateway found (no PID file at {}).",
                    pid::default_path().display()
                );
                eprintln!("Tip: start with `gateway start`");
            }
            return 1;
        }
    };

    if !pid::is_alive(pid_info.pid) {
        eprintln!(
            "Gateway is not running (PID {} not found). Cleaning up stale PID file.",
            pid_info.pid
        );
        pid::cleanup();
        return 1;
    }

    // Send SIGTERM
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, SIGTERM) sends a termination signal to the target process.
        let rc = unsafe { libc::kill(pid_info.pid as libc::pid_t, libc::SIGTERM) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            eprintln!(
                "error: failed to send SIGTERM to PID {}: {err}",
                pid_info.pid
            );
            return 2;
        }
    }

    #[cfg(not(unix))]
    {
        eprintln!("error: `gateway stop` is only supported on Unix systems.");
        return 2;
    }

    // Poll until process exits or 5-second timeout
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !pid::is_alive(pid_info.pid) {
            eprintln!(
                "Gateway stopped (was running on port {}). Run 'gateway start' to restart.",
                pid_info.port
            );
            // Server's shutdown handler cleans up PID file; remove stale file if still present.
            pid::cleanup();
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    eprintln!(
        "warning: Server (PID {}) did not stop within 5s. \
        Use `kill -9 {}` to force.",
        pid_info.pid, pid_info.pid
    );
    1
}
