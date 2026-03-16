use clap::Parser;
use std::path::PathBuf;

fn default_data_dir() -> String {
    dirs::data_local_dir()
        .map(|d| d.join("howm").to_string_lossy().to_string())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".local").join("howm").to_string_lossy().to_string())
                .unwrap_or_else(|| "./data".to_string())
        })
}

#[derive(Parser, Debug, Clone)]
#[command(name = "howm", about = "Howm — P2P capability platform")]
pub struct Config {
    #[arg(long, default_value = "7000", env = "HOWM_PORT")]
    pub port: u16,

    #[arg(long, default_value_os_t = PathBuf::from(default_data_dir()), env = "HOWM_DATA_DIR")]
    pub data_dir: PathBuf,

    #[arg(long, default_value = "5000", env = "HOWM_PEER_TIMEOUT_MS")]
    pub peer_timeout_ms: u64,

    #[arg(long, default_value = "60", env = "HOWM_DISCOVERY_INTERVAL_S")]
    pub discovery_interval_s: u64,

    #[arg(long, env = "HOWM_NODE_NAME")]
    pub name: Option<String>,

    // WireGuard networking
    /// Disable WireGuard (LAN-only mode)
    #[arg(long, default_value = "false", env = "HOWM_NO_WG")]
    pub no_wg: bool,

    #[arg(long, default_value = "51820", env = "HOWM_WG_PORT")]
    pub wg_port: u16,

    #[arg(long, env = "HOWM_WG_ENDPOINT")]
    pub wg_endpoint: Option<String>, // e.g. "1.2.3.4:51820" or "myhost.ddns.net:51820"

    #[arg(long, env = "HOWM_WG_ADDRESS")]
    pub wg_address: Option<String>, // override auto-assigned WG address

    #[arg(long, default_value = "900", env = "HOWM_INVITE_TTL_S")]
    pub invite_ttl_s: u64,

    #[arg(long, default_value = "256", env = "HOWM_OPEN_MAX_PEERS")]
    pub open_invite_max_peers: u32,

    #[arg(long, default_value = "10", env = "HOWM_OPEN_RATE_LIMIT")]
    pub open_invite_rate_limit: u32,

    #[arg(long, default_value = "5", env = "HOWM_OPEN_PRUNE_DAYS")]
    pub open_invite_prune_days: u64,

    #[arg(long, default_value = "false")]
    pub dev: bool,

    /// Enable debug logging (logs to stdout + files instead of files only)
    #[arg(long, default_value = "false")]
    pub debug: bool,

    /// Path to UI dist directory to serve as static files
    #[arg(long, env = "HOWM_UI_DIR")]
    pub ui_dir: Option<PathBuf>,
}


impl Config {
    /// WireGuard is enabled unless --no-wg is passed.
    pub fn wg_enabled(&self) -> bool {
        !self.no_wg
    }
}
