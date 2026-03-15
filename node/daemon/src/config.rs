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

    // Tailscale / networking
    #[arg(long, default_value = "true", env = "HOWM_TAILNET_ENABLED")]
    pub tailnet_enabled: bool,

    #[arg(long, env = "HOWM_COORDINATION_URL")]
    pub coordination_url: Option<String>,

    #[arg(long, env = "TS_AUTHKEY")]
    pub tailscale_authkey: Option<String>,

    #[arg(long, env = "HOWM_TSNET_STATE_DIR")]
    pub tsnet_state_dir: Option<PathBuf>,

    #[arg(long, default_value = "false")]
    pub headscale: bool,

    #[arg(long, default_value = "8080")]
    pub headscale_port: u16,

    #[arg(long, default_value = "900", env = "HOWM_INVITE_TTL_S")]
    pub invite_ttl_s: u64,

    #[arg(long, default_value = "false")]
    pub dev: bool,
}
