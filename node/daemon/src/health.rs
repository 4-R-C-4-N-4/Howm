use std::time::Duration;
use tracing::{info, warn};

use crate::{
    capabilities::{self, CapStatus},
    docker,
    state::AppState,
};

/// Background loop that checks the health of all running capability containers.
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

        match docker::check_health(&cap.container_id).await {
            Ok(true) => {} // healthy
            Ok(false) => {
                warn!("Capability '{}' container is not running", cap.name);
                let mut caps_lock = state.capabilities.write().await;
                if let Some(c) = caps_lock.iter_mut().find(|c| c.name == cap.name) {
                    c.status = CapStatus::Error("container exited".to_string());
                    needs_update = true;
                }
                drop(caps_lock);
            }
            Err(e) => {
                warn!("Health check failed for '{}': {}", cap.name, e);
                let mut caps_lock = state.capabilities.write().await;
                if let Some(c) = caps_lock.iter_mut().find(|c| c.name == cap.name) {
                    c.status = CapStatus::Error(format!("health check error: {}", e));
                    needs_update = true;
                }
                drop(caps_lock);
            }
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
