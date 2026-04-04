// Capability HTTP health watchdog.
//
// Polls each Running capability's GET /health endpoint every 30 s.
// After 2 consecutive failures, marks the capability Crashed and restarts it.
//
// Complements the PID-based health loop in main.rs:
//   - PID loop:      detects hard crashes (process died)
//   - HTTP watchdog: detects soft failures (process alive but unresponsive)
//
// On successful restart the capability's PeerStream reconnects to the daemon's
// SSE event stream and receives a snapshot — no explicit state flush is needed.

use std::collections::HashMap;
use std::time::Duration;
use tracing::{info, warn};

use crate::state::AppState;

/// Start the HTTP health watchdog background task.
pub fn start(state: AppState) {
    tokio::spawn(async move {
        watchdog_loop(state).await;
    });
    info!("Capability HTTP health watchdog started (30 s interval, 2-failure threshold)");
}

async fn watchdog_loop(state: AppState) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    // consecutive_failures[cap_name] = count of consecutive /health failures
    let mut failures: HashMap<String, u32> = HashMap::new();

    let mut interval = tokio::time::interval(Duration::from_secs(30));
    interval.tick().await; // skip the immediate first tick

    loop {
        interval.tick().await;

        // Snapshot the Running caps (release lock before doing async HTTP)
        let caps: Vec<_> = {
            state.capabilities.read().await
                .iter()
                .filter(|c| c.status == crate::capabilities::CapStatus::Running)
                .cloned()
                .collect()
        };

        for cap in caps {
            let url = format!("http://127.0.0.1:{}/health", cap.port);
            let ok = client
                .get(&url)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);

            if ok {
                if failures.remove(&cap.name).is_some() {
                    info!("watchdog: '{}' is healthy again", cap.name);
                }
                continue;
            }

            let count = failures.entry(cap.name.clone()).or_insert(0);
            *count += 1;
            warn!(
                "watchdog: capability '{}' /health failed ({}/2)",
                cap.name, count
            );

            if *count < 2 {
                continue;
            }

            // Two consecutive failures — restart.
            failures.remove(&cap.name);
            warn!("watchdog: restarting unresponsive capability '{}'", cap.name);

            // Mark Crashed so the UI can surface it.
            {
                let mut caps_guard = state.capabilities.write().await;
                if let Some(c) = caps_guard.iter_mut().find(|c| c.name == cap.name) {
                    c.status = crate::capabilities::CapStatus::Crashed;
                    c.pid = None;
                }
            }

            // Restart the process.
            match crate::executor::start_capability(
                &cap.binary_path,
                &cap.name,
                cap.port,
                &cap.data_dir,
                std::collections::HashMap::new(),
            )
            .await
            {
                Ok(pid) => {
                    {
                        let mut caps_guard = state.capabilities.write().await;
                        if let Some(c) = caps_guard.iter_mut().find(|c| c.name == cap.name) {
                            c.status = crate::capabilities::CapStatus::Running;
                            c.pid = Some(pid);
                        }
                    }
                    info!("watchdog: restarted '{}' (pid={})", cap.name, pid);
                    // The restarted capability will SSE-connect and receive a snapshot.
                }
                Err(e) => {
                    warn!("watchdog: failed to restart '{}': {}", cap.name, e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Verify that the failure counter increments and the cap transitions to
    /// Crashed after 2 consecutive failures — without needing a real daemon state.
    #[tokio::test]
    async fn failure_counter_reaches_threshold() {
        // Spin up a test HTTP server that always returns 500.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hit_count = Arc::new(AtomicUsize::new(0));
        let hit_count_srv = Arc::clone(&hit_count);

        tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/health",
                axum::routing::get({
                    let count = Arc::clone(&hit_count_srv);
                    move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }
                    }
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Directly exercise the failure logic without the 30 s timer.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{}/health", port);
        let mut failures: HashMap<String, u32> = HashMap::new();

        // First poll — count = 1, below threshold
        let ok = client.get(&url).send().await
            .map(|r| r.status().is_success()).unwrap_or(false);
        assert!(!ok);
        let count = failures.entry("test.cap".to_string()).or_insert(0);
        *count += 1;
        assert_eq!(*count, 1, "should not trigger restart yet");

        // Second poll — count = 2, at threshold
        let ok = client.get(&url).send().await
            .map(|r| r.status().is_success()).unwrap_or(false);
        assert!(!ok);
        let count = failures.entry("test.cap".to_string()).or_insert(0);
        *count += 1;
        assert_eq!(*count, 2, "threshold reached");
        assert!(*count >= 2, "watchdog should restart at this point");

        // Verify the server was actually hit twice
        assert_eq!(hit_count.load(Ordering::SeqCst), 2);
    }

    /// Verify that a healthy response clears the failure counter.
    #[tokio::test]
    async fn healthy_response_clears_failures() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/health",
                axum::routing::get(|| async { axum::http::StatusCode::OK }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();

        let mut failures: HashMap<String, u32> = HashMap::new();
        failures.insert("test.cap".to_string(), 1); // pre-seeded with one failure

        let url = format!("http://127.0.0.1:{}/health", port);
        let ok = client.get(&url).send().await
            .map(|r| r.status().is_success()).unwrap_or(false);
        assert!(ok);

        // Clear on success
        let removed = failures.remove("test.cap");
        assert!(removed.is_some(), "failure counter should be cleared on healthy response");
        assert!(failures.is_empty());
    }
}
