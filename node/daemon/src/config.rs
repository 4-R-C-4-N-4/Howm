use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "daemon", about = "Howm node daemon")]
pub struct Config {
    #[arg(long, default_value = "7000", env = "HOWM_PORT")]
    pub port: u16,

    #[arg(long, default_value = "./data", env = "HOWM_DATA_DIR")]
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
    pub wg_endpoint: Option<String>,  // e.g. "1.2.3.4:51820" or "myhost.ddns.net:51820"

    #[arg(long, env = "HOWM_WG_ADDRESS")]
    pub wg_address: Option<String>,   // override auto-assigned WG address

    #[arg(long, default_value = "900", env = "HOWM_INVITE_TTL_S")]
    pub invite_ttl_s: u64,

    #[arg(long, default_value = "false")]
    pub dev: bool,
}

impl Config {
    /// WireGuard is enabled unless --no-wg is passed.
    pub fn wg_enabled(&self) -> bool {
        !self.no_wg
    }
}
