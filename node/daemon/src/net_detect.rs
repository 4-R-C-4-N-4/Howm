/// Attempt to detect this machine's public IPv4 address by querying well-known
/// lightweight IP-echo services. Tries each in order; returns the first success.
///
/// This is a best-effort helper used at startup when `--wg-endpoint` is not
/// provided. The detected IP is logged clearly so the user can verify it.
pub async fn detect_public_ip() -> Option<String> {
    // Services that return a bare IP address as plain text (no JSON parsing needed).
    // Mix of HTTPS and HTTP — some environments (e.g. sudo) may have TLS issues.
    let candidates = [
        "https://api.ipify.org",
        "http://ipv4.icanhazip.com",
        "https://api4.my-ip.io/ip",
        "http://checkip.amazonaws.com",
    ];

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Public IP detection: failed to build HTTP client: {}", e);
            return None;
        }
    };

    for url in &candidates {
        tracing::debug!("Public IP detection: trying {}", url);
        match client.get(*url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    let ip = body.trim().to_string();
                    // Sanity-check: must look like an IPv4 address
                    if ip.split('.').count() == 4
                        && ip.chars().all(|c| c.is_ascii_digit() || c == '.')
                    {
                        tracing::info!("Public IP detected: {}", ip);
                        return Some(ip);
                    }
                    tracing::debug!(
                        "Public IP detection: {} returned non-IPv4 response: {:?}",
                        url,
                        &ip[..ip.len().min(50)]
                    );
                }
            }
            Ok(resp) => {
                tracing::debug!(
                    "Public IP detection: {} returned status {}",
                    url,
                    resp.status()
                );
            }
            Err(e) => {
                tracing::debug!("Public IP detection: {} failed: {}", url, e);
            }
        }
    }

    tracing::warn!(
        "Public IP detection: all {} services failed. \
         Pass --wg-endpoint <ip:port> manually.",
        candidates.len()
    );
    None
}
