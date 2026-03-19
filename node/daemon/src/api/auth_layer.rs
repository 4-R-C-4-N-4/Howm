use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

/// Load or create the API bearer token for local management auth.
/// Token is a random 256-bit hex string stored in `{data_dir}/api_token`.
pub fn load_or_create_token(data_dir: &Path) -> anyhow::Result<String> {
    let token_path = data_dir.join("api_token");
    if token_path.exists() {
        let token = std::fs::read_to_string(&token_path)?.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }

    // Generate new token
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);

    std::fs::write(&token_path, &token)?;
    tracing::warn!(
        "╔══════════════════════════════════════════════════════════╗\n\
         ║  NEW API TOKEN GENERATED — paste this into the Dashboard  ║\n\
         ║  {}  ║\n\
         ║  Saved to: {}  ║\n\
         ╚══════════════════════════════════════════════════════════╝",
        token,
        token_path.display(),
    );

    // Restrict permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(token)
}

// ── Simple rate limiter (S8) ─────────────────────────────────────────────────

/// A simple in-memory rate limiter: max `limit` requests per `window_secs` per key.
pub struct RateLimiter {
    limit: u32,
    window_secs: u64,
    buckets: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window_secs: u64) -> Self {
        Self {
            limit,
            window_secs,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if the request is allowed, false if rate-limited.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap();
        let entries = buckets.entry(key.to_string()).or_default();

        // Purge old entries outside the window
        let cutoff = now - std::time::Duration::from_secs(self.window_secs);
        entries.retain(|t| *t > cutoff);

        if entries.len() < self.limit as usize {
            entries.push(now);
            true
        } else {
            false
        }
    }
}
