use std::time::Duration;
use tracing::{info, warn};

use crate::{
    capabilities::{self, CapStatus},
    executor,
    state::AppState,
};

/// Background loop that checks the health of all running capability processes.
pub async fn start_loop(state: AppState) {
    let interval = Duration::from_secs(30);
    loop {
        tokio::time::sleep(interval).await;
        check_all(&state).await;
    }
}

async fn check_all(state: &AppState) {
    let caps = state.capabilities.read().await.clone();
    let mut needs_update = false;

    for cap in &caps {
        if !matches!(cap.status, CapStatus::Running) {
            continue;
        }

        let alive = cap
            .pid
            .map(|pid| executor::check_health(pid))
            .unwrap_or(false);

        if !alive {
            warn!(
                "Capability '{}' process is not running (pid={:?})",
                cap.name, cap.pid
            );
            let mut caps_lock = state.capabilities.write().await;
            if let Some(c) = caps_lock.iter_mut().find(|c| c.name == cap.name) {
                c.status = CapStatus::Error("process exited".to_string());
                c.pid = None;
                needs_update = true;
            }
            drop(caps_lock);
        }
    }

    if needs_update {
        let caps = state.capabilities.read().await;
        if let Err(e) = capabilities::save(&state.config.data_dir, &caps) {
            warn!("Failed to save capabilities after health check: {}", e);
        }
        let running = caps
            .iter()
            .filter(|c| matches!(c.status, CapStatus::Running))
            .count();
        let errored = caps
            .iter()
            .filter(|c| matches!(c.status, CapStatus::Error(_)))
            .count();
        info!("Health check: {} running, {} errored", running, errored);
    }
}
