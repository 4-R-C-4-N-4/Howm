#![allow(deprecated)]
// p2pcd-peer.toml configuration schema and helpers.
// This replaces the CLI-flag-based config in daemon/src/config.rs.

use crate::{
    CapabilityDeclaration, ClassificationTier, DiscoveryManifest, PeerId, Role, ScopeParams,
    TrustPolicy,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─── Top-level config struct ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    pub identity: IdentityConfig,
    pub protocol: ProtocolConfig,
    pub transport: TransportConfig,
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub capabilities: HashMap<String, CapabilityConfig>,
    #[serde(default)]
    pub friends: FriendsConfig,
    #[serde(default)]
    pub invite: InviteConfig,
    #[serde(default)]
    pub data: DataConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityConfig {
    /// Path to WireGuard private key file (derive public key from this).
    #[serde(default)]
    pub wireguard_private_key_file: Option<String>,
    /// Alternatively, specify the WireGuard interface to read the key from.
    #[serde(default)]
    pub wireguard_interface: Option<String>,
    /// Human-readable display name (not transmitted in protocol).
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolConfig {
    /// Protocol version (must be 1 for P2P-CD-01 v0.3).
    #[serde(default = "default_protocol_version")]
    pub version: u64,
    /// IANA hash algorithm name (e.g. "sha-256").
    #[serde(default = "default_hash_algorithm")]
    pub hash_algorithm: String,
}

fn default_protocol_version() -> u64 {
    1
}
fn default_hash_algorithm() -> String {
    "sha-256".to_string()
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            version: default_protocol_version(),
            hash_algorithm: default_hash_algorithm(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// TCP port for P2P-CD protocol (default 7654).
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
    /// WireGuard interface name.
    #[serde(default = "default_wg_interface")]
    pub wireguard_interface: String,
    /// HTTP port for daemon management API (default 7000).
    #[serde(default = "default_http_port")]
    pub http_port: u16,
}

fn default_listen_port() -> u16 {
    7654
}
fn default_wg_interface() -> String {
    "howm0".to_string()
}
fn default_http_port() -> u16 {
    7000
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            listen_port: default_listen_port(),
            wireguard_interface: default_wg_interface(),
            http_port: default_http_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    /// How to detect new WireGuard peers: "wireguard" | "mdns" | "manual"
    #[serde(default = "default_discovery_mode")]
    pub mode: String,
    /// How often to poll WireGuard peer state (milliseconds).
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    /// Also broadcast on mDNS for peers not yet in WG config.
    #[serde(default)]
    pub mdns_fallback: bool,
    /// Send full manifest in initial discovery (normally only hash).
    #[serde(default)]
    pub broadcast_full_manifest: bool,
}

fn default_discovery_mode() -> String {
    "wireguard".to_string()
}
fn default_poll_interval_ms() -> u64 {
    2000
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            mode: default_discovery_mode(),
            poll_interval_ms: default_poll_interval_ms(),
            mdns_fallback: false,
            broadcast_full_manifest: false,
        }
    }
}

// ─── Per-capability config ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityConfig {
    /// Fully-qualified capability name (e.g. "howm.social.feed.1").
    /// Must match the namespace grammar §4.4.
    pub name: String,
    /// Role: "provide" | "consume" | "both"
    pub role: RoleConfig,
    /// Required for both+both matching (heartbeat).
    #[serde(default)]
    pub mutual: bool,
    /// Scope params advertised to remote peers.
    pub scope: Option<ScopeConfig>,
    /// Trust gate classification (local-only, not on wire).
    pub classification: Option<ClassificationConfig>,
    /// Heartbeat-specific params.
    pub params: Option<HeartbeatParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoleConfig {
    Provide,
    Consume,
    Both,
}

impl From<&RoleConfig> for Role {
    fn from(r: &RoleConfig) -> Self {
        match r {
            RoleConfig::Provide => Role::Provide,
            RoleConfig::Consume => Role::Consume,
            RoleConfig::Both => Role::Both,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeConfig {
    #[serde(default)]
    pub rate_limit: u64,
    #[serde(default)]
    pub ttl: u64,
}

impl From<&ScopeConfig> for ScopeParams {
    fn from(s: &ScopeConfig) -> Self {
        ScopeParams {
            rate_limit: s.rate_limit,
            ttl: s.ttl,
            extensions: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[deprecated(
    since = "0.2.0",
    note = "replaced by howm-access group-based permissions"
)]
pub struct ClassificationConfig {
    /// Default tier: "public" | "friends" | "blocked"
    pub default_tier: ClassificationTier,
    /// Per-peer overrides: base64-encoded WG public key -> tier.
    #[serde(default)]
    pub overrides: HashMap<String, ClassificationTier>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatParams {
    #[serde(default = "default_heartbeat_interval")]
    pub interval_ms: u64,
    #[serde(default = "default_heartbeat_timeout")]
    pub timeout_ms: u64,
}

fn default_heartbeat_interval() -> u64 {
    5000
}
fn default_heartbeat_timeout() -> u64 {
    15000
}

// ─── Friends list ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FriendsConfig {
    /// WireGuard public keys (base64-encoded, same format as wg0.conf).
    #[serde(default)]
    pub list: Vec<String>,
}

// ─── Invite config ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteConfig {
    #[serde(default = "default_invite_ttl")]
    pub ttl_s: u64,
    #[serde(default = "default_open_max_peers")]
    pub open_max_peers: u32,
    #[serde(default = "default_open_rate_limit")]
    pub open_rate_limit: u32,
    #[serde(default = "default_open_prune_days")]
    pub open_prune_days: u64,
}

fn default_invite_ttl() -> u64 {
    900
}
fn default_open_max_peers() -> u32 {
    256
}
fn default_open_rate_limit() -> u32 {
    10
}
fn default_open_prune_days() -> u64 {
    5
}

impl Default for InviteConfig {
    fn default() -> Self {
        Self {
            ttl_s: default_invite_ttl(),
            open_max_peers: default_open_max_peers(),
            open_rate_limit: default_open_rate_limit(),
            open_prune_days: default_open_prune_days(),
        }
    }
}

// ─── Data dir config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataConfig {
    pub dir: String,
}

impl Default for DataConfig {
    fn default() -> Self {
        let dir = dirs_next::data_local_dir()
            .map(|d| d.join("howm"))
            .unwrap_or_else(|| PathBuf::from("~/.local/howm"))
            .to_string_lossy()
            .to_string();
        Self { dir }
    }
}

// ─── PeerConfig methods ───────────────────────────────────────────────────────

/// Namespace grammar §4.4: <org>.<component>[.<subcomponent>].<version>
/// Components: lowercase alpha, digits, hyphens. Dots as separators. Version is integer.
pub fn validate_capability_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    // Last part must be a positive integer version
    if parts.last().unwrap().parse::<u32>().is_err() {
        return false;
    }
    // All other parts: [a-z0-9-]+
    parts[..parts.len() - 1].iter().all(|p| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    })
}

impl PeerConfig {
    /// Load config from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("read config file: {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("parse config file: {}", path.display()))
    }

    /// Generate a default Normal User archetype config (POC §6.1).
    #[allow(deprecated)]
    pub fn generate_default(data_dir: &Path) -> Self {
        PeerConfig {
            identity: IdentityConfig {
                wireguard_private_key_file: None,
                wireguard_interface: Some("howm0".to_string()),
                display_name: "howm-user".to_string(),
            },
            protocol: ProtocolConfig::default(),
            transport: TransportConfig::default(),
            discovery: DiscoveryConfig::default(),
            capabilities: {
                let mut m = HashMap::new();
                // Single social capability — Both/mutual:true, direction handled at app layer
                m.insert(
                    "social_feed".to_string(),
                    CapabilityConfig {
                        name: "howm.social.feed.1".to_string(),
                        role: RoleConfig::Both,
                        mutual: true,
                        scope: Some(ScopeConfig {
                            rate_limit: 10,
                            ttl: 3600,
                        }),
                        classification: Some(ClassificationConfig {
                            default_tier: ClassificationTier::Public,
                            overrides: HashMap::new(),
                        }),
                        params: None,
                    },
                );
                m.insert(
                    "heartbeat".to_string(),
                    CapabilityConfig {
                        name: "core.session.heartbeat.1".to_string(),
                        role: RoleConfig::Both,
                        mutual: true,
                        scope: None,
                        classification: None,
                        params: Some(HeartbeatParams {
                            interval_ms: 5000,
                            timeout_ms: 15000,
                        }),
                    },
                );
                m
            },
            friends: FriendsConfig::default(),
            invite: InviteConfig::default(),
            data: DataConfig {
                dir: data_dir.to_string_lossy().to_string(),
            },
        }
    }

    /// Convert config into a DiscoveryManifest.
    /// personal_hash is computed and embedded.
    pub fn to_manifest(&self, peer_id: PeerId, sequence_num: u64) -> DiscoveryManifest {
        let mut caps: Vec<CapabilityDeclaration> = self
            .capabilities
            .values()
            .map(|c| CapabilityDeclaration {
                name: c.name.clone(),
                role: Role::from(&c.role),
                mutual: c.mutual,
                // Scope from config
                scope: c.scope.as_ref().map(ScopeParams::from),
                applicable_scope_keys: None,
            })
            .collect();
        caps.sort_by(|a, b| a.name.cmp(&b.name));

        let mut manifest = DiscoveryManifest {
            protocol_version: self.protocol.version,
            peer_id,
            sequence_num,
            capabilities: caps,
            personal_hash: vec![],
            hash_algorithm: self.protocol.hash_algorithm.clone(),
        };
        // Compute and embed personal_hash
        manifest.personal_hash = crate::cbor::personal_hash(&manifest);
        manifest
    }

    /// Build TrustPolicy map for all capabilities with classification config.
    #[deprecated(
        since = "0.2.0",
        note = "trust gate now uses howm-access AccessDb; this method is unused"
    )]
    #[allow(deprecated)]
    pub fn trust_policies(&self) -> HashMap<String, TrustPolicy> {
        // Parse friends list once
        let friends_set: std::collections::HashSet<PeerId> = self
            .friends
            .list
            .iter()
            .filter_map(|b64| parse_wg_pubkey(b64))
            .collect();

        let mut policies = HashMap::new();
        for cap_cfg in self.capabilities.values() {
            if let Some(class) = &cap_cfg.classification {
                // Parse per-peer overrides
                let overrides: HashMap<PeerId, ClassificationTier> = class
                    .overrides
                    .iter()
                    .filter_map(|(b64, tier)| parse_wg_pubkey(b64).map(|pk| (pk, *tier)))
                    .collect();
                policies.insert(
                    cap_cfg.name.clone(),
                    TrustPolicy {
                        default_tier: class.default_tier,
                        overrides,
                        friends: friends_set.clone(),
                    },
                );
            }
        }
        policies
    }

    /// Return data directory as PathBuf.
    pub fn data_dir(&self) -> PathBuf {
        PathBuf::from(&self.data.dir)
    }
}

/// Parse a base64-encoded WireGuard public key to a 32-byte PeerId.
pub fn parse_wg_pubkey(b64: &str) -> Option<PeerId> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(arr)
}

/// Encode a PeerId as base64 (standard, padded).
pub fn peer_id_to_base64(peer_id: &PeerId) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(peer_id)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(deprecated)]
    use super::*;

    const SAMPLE_TOML: &str = r#"
[identity]
wireguard_interface = "howm0"
display_name = "alice"

[protocol]
version = 1
hash_algorithm = "sha-256"

[transport]
listen_port = 7654
wireguard_interface = "howm0"
http_port = 7000

[discovery]
mode = "wireguard"
poll_interval_ms = 2000

[capabilities.social_feed]
name = "howm.social.feed.1"
role = "both"
mutual = true

[capabilities.social_feed.scope]
rate_limit = 10
ttl = 3600

[capabilities.social_feed.classification]
default_tier = "public"

[capabilities.heartbeat]
name = "core.session.heartbeat.1"
role = "both"
mutual = true

[capabilities.heartbeat.params]
interval_ms = 5000
timeout_ms = 15000

[friends]
list = []
"#;

    #[test]
    #[allow(deprecated)]
    fn parse_sample_config() {
        let cfg: PeerConfig = toml::from_str(SAMPLE_TOML).unwrap();
        assert_eq!(cfg.identity.display_name, "alice");
        assert_eq!(cfg.transport.listen_port, 7654);
        assert_eq!(cfg.capabilities.len(), 2);
        assert!(cfg.capabilities.contains_key("heartbeat"));
    }

    #[test]
    #[allow(deprecated)]
    #[allow(deprecated)]
    fn generate_default_round_trip() {
        let data_dir = PathBuf::from("/tmp/howm-test");
        let cfg = PeerConfig::generate_default(&data_dir);
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let reparsed: PeerConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg.identity.display_name, reparsed.identity.display_name);
        assert_eq!(cfg.capabilities.len(), reparsed.capabilities.len());
    }

    #[test]
    fn to_manifest_has_all_caps() {
        let data_dir = PathBuf::from("/tmp/howm-test");
        let cfg = PeerConfig::generate_default(&data_dir);
        let peer_id = [0xA1u8; 32];
        let manifest = cfg.to_manifest(peer_id, 1);
        assert_eq!(manifest.capabilities.len(), 2);
        // Capabilities must be sorted
        let names: Vec<_> = manifest
            .capabilities
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        // personal_hash must be non-empty
        assert_eq!(manifest.personal_hash.len(), 32);
    }

    #[test]
    #[allow(deprecated)]
    fn trust_policies_built_correctly() {
        let cfg: PeerConfig = toml::from_str(SAMPLE_TOML).unwrap();
        let policies = cfg.trust_policies();
        assert!(policies.contains_key("howm.social.feed.1"));
        // Heartbeat has no classification config → no policy
        assert!(!policies.contains_key("core.session.heartbeat.1"));
    }

    #[test]
    fn validate_capability_names() {
        assert!(validate_capability_name("howm.social.feed.1"));
        assert!(validate_capability_name("core.session.heartbeat.1"));
        assert!(validate_capability_name("org.example.cap.2"));
        assert!(!validate_capability_name("invalid"));
        assert!(!validate_capability_name("p2pcd.1"));
        assert!(!validate_capability_name("p2pcd.social.post.alpha")); // version must be int
        assert!(!validate_capability_name("P2PCD.social.post.1")); // uppercase not allowed
    }
}
