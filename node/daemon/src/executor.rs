use std::collections::HashMap;
use std::path::Path;
use tracing::info;

/// Spawn a capability process. Returns the PID.
///
/// Sets env vars: PORT, DATA_DIR, HOWM_CAP_PORT plus any extra `env_vars`.
/// Redirects stdout/stderr to `{data_dir}/logs/{cap_name}.log`.
pub async fn start_capability(
    binary_path: &str,
    cap_name: &str,
    port: u16,
    data_dir: &str,
    env_vars: HashMap<String, String>,
) -> anyhow::Result<u32> {
    // Ensure log directory exists
    let log_dir = Path::new(data_dir).join("logs");
    std::fs::create_dir_all(&log_dir)?;

    let log_file_path = log_dir.join(format!("{}.log", cap_name));
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;
    let stderr_file = log_file.try_clone()?;

    let mut cmd = tokio::process::Command::new(binary_path);
    cmd.env("PORT", port.to_string())
        .env("DATA_DIR", data_dir)
        .env("HOWM_CAP_PORT", port.to_string())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(stderr_file))
        // Don't let the child die when we drop the handle
        .kill_on_drop(false);

    for (k, v) in &env_vars {
        cmd.env(k, v);
    }

    let child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "Failed to spawn capability '{}' ({}): {}",
            cap_name,
            binary_path,
            e
        )
    })?;

    let pid = child
        .id()
        .ok_or_else(|| anyhow::anyhow!("Failed to get PID for capability '{}'", cap_name))?;

    info!(
        "Started capability '{}' (pid={}, port={}, log={})",
        cap_name,
        pid,
        port,
        log_file_path.display()
    );
    Ok(pid)
}

/// Send SIGTERM (unix) or TerminateProcess (windows) and wait briefly for exit.
pub async fn stop_capability(pid: u32) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        let nix_pid = Pid::from_raw(pid as i32);

        // Send SIGTERM
        match kill(nix_pid, Signal::SIGTERM) {
            Ok(()) => {}
            Err(nix::errno::Errno::ESRCH) => {
                // Process already gone
                return Ok(());
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to send SIGTERM to pid {}: {}",
                    pid,
                    e
                ))
            }
        }

        // Wait up to 10 seconds for the process to exit
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if !check_health(pid) {
                info!("Process {} exited after SIGTERM", pid);
                return Ok(());
            }
        }

        // Force kill with SIGKILL
        info!(
            "Process {} did not exit after SIGTERM, sending SIGKILL",
            pid
        );
        let _ = kill(nix_pid, Signal::SIGKILL);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    #[cfg(windows)]
    {
        // Open process handle with TERMINATE permission, then terminate it.
        // Falls back to `taskkill /F /PID` if the handle approach fails.
        use std::process::Command;

        let status = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        match status {
            Ok(s) if s.success() => {
                info!("Process {} terminated via taskkill", pid);
            }
            Ok(s) => {
                // taskkill returns non-zero if process already gone
                info!(
                    "taskkill exited with {} for pid {} (probably already gone)",
                    s, pid
                );
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to terminate pid {}: {}", pid, e));
            }
        }
    }

    Ok(())
}

/// Check if a process is still alive.
pub fn check_health(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        let nix_pid = Pid::from_raw(pid as i32);
        // Signal 0 doesn't actually send a signal, just checks if process exists
        kill(nix_pid, None).is_ok()
    }

    #[cfg(windows)]
    {
        use std::process::Command;

        // Use tasklist to check if PID exists
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|o| {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout.contains(&pid.to_string())
            })
            .unwrap_or(false)
    }
}
