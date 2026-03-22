// CBOR wire encoding/decoding for P2P-CD-01 v0.3.
// All wire messages use integer map keys per spec §5.3.
// Do NOT use serde for wire format — use ciborium directly.

use crate::{
    capability_keys, manifest_keys, message_keys, scope_keys, CapabilityDeclaration, CloseReason,
    DiscoveryManifest, MessageType, ProtocolMessage, Role, ScopeParams, ScopeValue, PEER_ID_LEN,
};
use anyhow::{anyhow, bail, Context, Result};
use ciborium::value::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn cbor_to_bytes(val: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(val, &mut out).map_err(|e| anyhow!("CBOR encode error: {e}"))?;
    Ok(out)
}

fn decode_cbor(bytes: &[u8]) -> Result<Value> {
    ciborium::de::from_reader(bytes).map_err(|e| anyhow!("CBOR decode error: {e}"))
}

/// Get integer value from a CBOR map by integer key.
fn map_get_int(map: &[(Value, Value)], key: u64) -> Option<u64> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Integer(vi) = v {
                    return u64::try_from(*vi).ok();
                }
            }
        }
    }
    None
}

/// Get boolean value from a CBOR map by integer key.
fn map_get_bool(map: &[(Value, Value)], key: u64) -> Option<bool> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Bool(b) = v {
                    return Some(*b);
                }
            }
        }
    }
    None
}

/// Get text string from a CBOR map by integer key.
fn map_get_text(map: &[(Value, Value)], key: u64) -> Option<String> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Text(s) = v {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

/// Get bytes from a CBOR map by integer key.
fn map_get_bytes(map: &[(Value, Value)], key: u64) -> Option<Vec<u8>> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Bytes(b) = v {
                    return Some(b.clone());
                }
            }
        }
    }
    None
}

/// Get array from a CBOR map by integer key.
fn map_get_array(map: &[(Value, Value)], key: u64) -> Option<Vec<Value>> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Array(arr) = v {
                    return Some(arr.clone());
                }
            }
        }
    }
    None
}

/// Get map from a CBOR map by integer key.
fn map_get_map(map: &[(Value, Value)], key: u64) -> Option<Vec<(Value, Value)>> {
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if u64::try_from(*ki).ok() == Some(key) {
                if let Value::Map(m) = v {
                    return Some(m.clone());
                }
            }
        }
    }
    None
}

fn int_key(k: u64) -> Value {
    Value::Integer(ciborium::value::Integer::from(k))
}

// ─── ScopeParams CBOR ─────────────────────────────────────────────────────────

pub fn scope_to_cbor_value(scope: &ScopeParams) -> Value {
    let mut pairs = Vec::new();
    if scope.rate_limit > 0 {
        pairs.push((
            int_key(scope_keys::RATE_LIMIT),
            Value::Integer(ciborium::value::Integer::from(scope.rate_limit)),
        ));
    }
    if scope.ttl > 0 {
        pairs.push((
            int_key(scope_keys::TTL),
            Value::Integer(ciborium::value::Integer::from(scope.ttl)),
        ));
    }
    // Extension keys (3+) — sorted by key for deterministic encoding
    for (key, val) in &scope.extensions {
        pairs.push((int_key(*key), scope_value_to_cbor(val)));
    }
    Value::Map(pairs)
}

pub fn scope_from_cbor_value(val: &Value) -> Result<ScopeParams> {
    let map = match val {
        Value::Map(m) => m,
        _ => bail!("scope_params: expected map"),
    };
    let rate_limit = map_get_int(map, scope_keys::RATE_LIMIT).unwrap_or(0);
    let ttl = map_get_int(map, scope_keys::TTL).unwrap_or(0);

    // Collect extension keys (anything beyond 1 and 2)
    let mut extensions = BTreeMap::new();
    for (k, v) in map {
        if let Value::Integer(ki) = k {
            if let Ok(key) = u64::try_from(*ki) {
                if key > scope_keys::TTL {
                    if let Some(sv) = cbor_to_scope_value(v) {
                        extensions.insert(key, sv);
                    }
                }
            }
        }
    }

    Ok(ScopeParams {
        rate_limit,
        ttl,
        extensions,
    })
}

/// Convert a ScopeValue to CBOR Value.
fn scope_value_to_cbor(sv: &ScopeValue) -> Value {
    match sv {
        ScopeValue::Uint(v) => Value::Integer(ciborium::value::Integer::from(*v)),
        ScopeValue::Text(s) => Value::Text(s.clone()),
        ScopeValue::Bool(b) => Value::Bool(*b),
        ScopeValue::Bytes(b) => Value::Bytes(b.clone()),
        ScopeValue::Array(arr) => Value::Array(arr.iter().map(scope_value_to_cbor).collect()),
    }
}

/// Convert a CBOR Value to ScopeValue.
fn cbor_to_scope_value(val: &Value) -> Option<ScopeValue> {
    match val {
        Value::Integer(i) => u64::try_from(*i).ok().map(ScopeValue::Uint),
        Value::Text(s) => Some(ScopeValue::Text(s.clone())),
        Value::Bool(b) => Some(ScopeValue::Bool(*b)),
        Value::Bytes(b) => Some(ScopeValue::Bytes(b.clone())),
        Value::Array(arr) => {
            let items: Vec<ScopeValue> = arr.iter().filter_map(cbor_to_scope_value).collect();
            Some(ScopeValue::Array(items))
        }
        _ => None, // Map, Null, etc. — ignore unknown types
    }
}

// ─── CapabilityDeclaration CBOR ───────────────────────────────────────────────

pub fn cap_to_cbor_value(cap: &CapabilityDeclaration) -> Value {
    let mut pairs = vec![
        (
            int_key(capability_keys::NAME),
            Value::Text(cap.name.clone()),
        ),
        (
            int_key(capability_keys::ROLE),
            Value::Integer(ciborium::value::Integer::from(cap.role as u64)),
        ),
    ];
    if cap.mutual {
        pairs.push((int_key(capability_keys::MUTUAL), Value::Bool(true)));
    }
    // NOTE: classification is intentionally omitted from the wire (spec §5.3 note)
    if let Some(scope) = &cap.scope {
        pairs.push((int_key(capability_keys::SCOPE), scope_to_cbor_value(scope)));
    }
    if let Some(keys) = &cap.applicable_scope_keys {
        let arr = keys
            .iter()
            .map(|k| Value::Integer(ciborium::value::Integer::from(*k)))
            .collect();
        pairs.push((
            int_key(capability_keys::APPLICABLE_SCOPE_KEYS),
            Value::Array(arr),
        ));
    }
    Value::Map(pairs)
}

pub fn cap_from_cbor_value(val: &Value) -> Result<CapabilityDeclaration> {
    let map = match val {
        Value::Map(m) => m,
        _ => bail!("capability_declaration: expected map"),
    };
    let name = map_get_text(map, capability_keys::NAME)
        .ok_or_else(|| anyhow!("capability_declaration: missing name"))?;
    let role_u64 = map_get_int(map, capability_keys::ROLE)
        .ok_or_else(|| anyhow!("capability_declaration: missing role"))?;
    let role = Role::from_u64(role_u64)
        .ok_or_else(|| anyhow!("capability_declaration: unknown role {role_u64}"))?;
    let mutual = map_get_bool(map, capability_keys::MUTUAL).unwrap_or(false);
    let scope = map_get_map(map, capability_keys::SCOPE)
        .map(|m| scope_from_cbor_value(&Value::Map(m)))
        .transpose()?;
    let applicable_scope_keys =
        map_get_array(map, capability_keys::APPLICABLE_SCOPE_KEYS).map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    if let Value::Integer(i) = v {
                        u64::try_from(*i).ok()
                    } else {
                        None
                    }
                })
                .collect()
        });

    Ok(CapabilityDeclaration {
        name,
        role,
        mutual,
        scope,
        applicable_scope_keys,
    })
}

// ─── DiscoveryManifest CBOR ───────────────────────────────────────────────────

impl DiscoveryManifest {
    /// Encode manifest to deterministic CBOR with integer keys (§5.3).
    /// Capabilities are sorted lexicographically before encoding.
    pub fn to_cbor(&self) -> Vec<u8> {
        let mut m = self.clone();
        m.sort_capabilities();

        let caps_array: Vec<Value> = m.capabilities.iter().map(cap_to_cbor_value).collect();

        let pairs = vec![
            (
                int_key(manifest_keys::PROTOCOL_VERSION),
                Value::Integer(ciborium::value::Integer::from(m.protocol_version)),
            ),
            (
                int_key(manifest_keys::PEER_ID),
                Value::Bytes(m.peer_id.to_vec()),
            ),
            (
                int_key(manifest_keys::SEQUENCE_NUM),
                Value::Integer(ciborium::value::Integer::from(m.sequence_num)),
            ),
            (
                int_key(manifest_keys::CAPABILITIES),
                Value::Array(caps_array),
            ),
            (
                int_key(manifest_keys::PERSONAL_HASH),
                Value::Bytes(m.personal_hash.clone()),
            ),
            (
                int_key(manifest_keys::HASH_ALGORITHM),
                Value::Text(m.hash_algorithm.clone()),
            ),
        ];

        cbor_to_bytes(&Value::Map(pairs)).expect("manifest CBOR encode should not fail")
    }

    /// Decode manifest from CBOR bytes.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self> {
        let val = decode_cbor(bytes).context("manifest from_cbor")?;
        let map = match &val {
            Value::Map(m) => m,
            _ => bail!("manifest: expected map"),
        };

        let protocol_version = map_get_int(map, manifest_keys::PROTOCOL_VERSION)
            .ok_or_else(|| anyhow!("manifest: missing protocol_version"))?;
        let peer_id_bytes = map_get_bytes(map, manifest_keys::PEER_ID)
            .ok_or_else(|| anyhow!("manifest: missing peer_id"))?;
        if peer_id_bytes.len() != PEER_ID_LEN {
            bail!(
                "manifest: peer_id must be {PEER_ID_LEN} bytes, got {}",
                peer_id_bytes.len()
            );
        }
        let mut peer_id = [0u8; PEER_ID_LEN];
        peer_id.copy_from_slice(&peer_id_bytes);

        let sequence_num = map_get_int(map, manifest_keys::SEQUENCE_NUM)
            .ok_or_else(|| anyhow!("manifest: missing sequence_num"))?;

        let caps_array = map_get_array(map, manifest_keys::CAPABILITIES)
            .ok_or_else(|| anyhow!("manifest: missing capabilities"))?;
        let capabilities = caps_array
            .iter()
            .map(cap_from_cbor_value)
            .collect::<Result<Vec<_>>>()
            .context("manifest: capabilities decode")?;

        let personal_hash = map_get_bytes(map, manifest_keys::PERSONAL_HASH)
            .ok_or_else(|| anyhow!("manifest: missing personal_hash"))?;
        let hash_algorithm = map_get_text(map, manifest_keys::HASH_ALGORITHM)
            .ok_or_else(|| anyhow!("manifest: missing hash_algorithm"))?;

        Ok(DiscoveryManifest {
            protocol_version,
            peer_id,
            sequence_num,
            capabilities,
            personal_hash,
            hash_algorithm,
        })
    }
}

// ─── personal_hash (§5.5) ─────────────────────────────────────────────────────

/// Compute SHA-256 of deterministic CBOR-encoded manifest with sequence_num=0.
/// This represents the capability configuration hash — not a per-sequence value.
pub fn personal_hash(manifest: &DiscoveryManifest) -> Vec<u8> {
    let mut m = manifest.clone();
    m.sequence_num = 0;
    // personal_hash field is also zeroed (empty) for the hash computation
    m.personal_hash = vec![];
    let encoded = m.to_cbor();
    let mut hasher = Sha256::new();
    hasher.update(&encoded);
    hasher.finalize().to_vec()
}

// ─── ProtocolMessage encode/decode ────────────────────────────────────────────

impl ProtocolMessage {
    /// Encode message as length-prefixed CBOR: 4-byte big-endian length + CBOR payload.
    pub fn encode(&self) -> Vec<u8> {
        let cbor_payload = self.to_cbor_bytes();
        let len = cbor_payload.len() as u32;
        let mut out = Vec::with_capacity(4 + cbor_payload.len());
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&cbor_payload);
        out
    }

    /// Read a length-prefixed CBOR message from a reader.
    pub fn decode(reader: &mut impl Read) -> Result<Self> {
        let mut len_buf = [0u8; 4];
        reader
            .read_exact(&mut len_buf)
            .context("read message length prefix")?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Sanity cap: 64 MiB
        if len > 64 * 1024 * 1024 {
            bail!("message too large: {len} bytes");
        }

        let mut payload = vec![0u8; len];
        reader
            .read_exact(&mut payload)
            .context("read message payload")?;

        Self::from_cbor_bytes(&payload)
    }

    fn to_cbor_bytes(&self) -> Vec<u8> {
        let val = match self {
            ProtocolMessage::Offer { manifest } => {
                let manifest_cbor = manifest.to_cbor();
                let manifest_val = decode_cbor(&manifest_cbor).expect("encode manifest for offer");
                Value::Map(vec![
                    (
                        int_key(message_keys::MESSAGE_TYPE),
                        Value::Integer(ciborium::value::Integer::from(MessageType::Offer as u64)),
                    ),
                    (int_key(message_keys::MANIFEST), manifest_val),
                ])
            }

            ProtocolMessage::Confirm {
                personal_hash,
                active_set,
                accepted_params,
            } => {
                let active_array =
                    Value::Array(active_set.iter().map(|s| Value::Text(s.clone())).collect());
                let mut pairs = vec![
                    (
                        int_key(message_keys::MESSAGE_TYPE),
                        Value::Integer(ciborium::value::Integer::from(MessageType::Confirm as u64)),
                    ),
                    (
                        int_key(message_keys::PERSONAL_HASH),
                        Value::Bytes(personal_hash.clone()),
                    ),
                    (int_key(message_keys::ACTIVE_SET), active_array),
                ];
                if let Some(params) = accepted_params {
                    let params_map: Vec<(Value, Value)> = params
                        .iter()
                        .map(|(k, v)| (Value::Text(k.clone()), scope_to_cbor_value(v)))
                        .collect();
                    pairs.push((
                        int_key(message_keys::ACCEPTED_PARAMS),
                        Value::Map(params_map),
                    ));
                }
                Value::Map(pairs)
            }

            ProtocolMessage::Close {
                personal_hash,
                reason,
            } => Value::Map(vec![
                (
                    int_key(message_keys::MESSAGE_TYPE),
                    Value::Integer(ciborium::value::Integer::from(MessageType::Close as u64)),
                ),
                (
                    int_key(message_keys::PERSONAL_HASH),
                    Value::Bytes(personal_hash.clone()),
                ),
                (
                    int_key(message_keys::REASON),
                    Value::Integer(ciborium::value::Integer::from(*reason as u64)),
                ),
            ]),

            ProtocolMessage::Ping { timestamp } => Value::Map(vec![
                (
                    int_key(message_keys::MESSAGE_TYPE),
                    Value::Integer(ciborium::value::Integer::from(MessageType::Ping as u64)),
                ),
                (
                    int_key(message_keys::TIMESTAMP),
                    Value::Integer(ciborium::value::Integer::from(*timestamp)),
                ),
            ]),

            ProtocolMessage::Pong { timestamp } => Value::Map(vec![
                (
                    int_key(message_keys::MESSAGE_TYPE),
                    Value::Integer(ciborium::value::Integer::from(MessageType::Pong as u64)),
                ),
                (
                    int_key(message_keys::TIMESTAMP),
                    Value::Integer(ciborium::value::Integer::from(*timestamp)),
                ),
            ]),

            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                // Re-wrap: decode payload back to CBOR value, add message_type key
                let inner = decode_cbor(payload).unwrap_or(Value::Map(vec![]));
                let mut pairs = vec![(
                    int_key(message_keys::MESSAGE_TYPE),
                    Value::Integer(ciborium::value::Integer::from(*message_type)),
                )];
                // Merge inner map entries (if it's a map)
                if let Value::Map(m) = inner {
                    pairs.extend(m);
                }
                Value::Map(pairs)
            }
        };
        cbor_to_bytes(&val).expect("protocol message CBOR encode should not fail")
    }

    fn from_cbor_bytes(bytes: &[u8]) -> Result<Self> {
        let val = decode_cbor(bytes).context("protocol message decode")?;
        let map = match &val {
            Value::Map(m) => m,
            _ => bail!("protocol message: expected map"),
        };

        let msg_type_u64 = map_get_int(map, message_keys::MESSAGE_TYPE)
            .ok_or_else(|| anyhow!("protocol message: missing message_type"))?;

        // Types 6+ are capability messages — extract payload and route to handlers
        if MessageType::from_u64(msg_type_u64).is_none() {
            // Build payload: re-encode the map without the message_type key
            let filtered: Vec<(Value, Value)> = map
                .iter()
                .filter(|(k, _)| {
                    !matches!(k, Value::Integer(ki) if u64::try_from(*ki).ok() == Some(message_keys::MESSAGE_TYPE))
                })
                .cloned()
                .collect();
            let payload = cbor_to_bytes(&Value::Map(filtered))?;
            return Ok(ProtocolMessage::CapabilityMsg {
                message_type: msg_type_u64,
                payload,
            });
        }
        let msg_type = MessageType::from_u64(msg_type_u64).unwrap();

        match msg_type {
            MessageType::Offer => {
                let manifest_val = map.iter()
                    .find(|(k, _)| matches!(k, Value::Integer(ki) if u64::try_from(*ki).ok() == Some(message_keys::MANIFEST)))
                    .map(|(_, v)| v)
                    .ok_or_else(|| anyhow!("offer: missing manifest"))?;
                let manifest_bytes = cbor_to_bytes(manifest_val)?;
                let manifest = DiscoveryManifest::from_cbor(&manifest_bytes)?;
                Ok(ProtocolMessage::Offer { manifest })
            }

            MessageType::Confirm => {
                let personal_hash = map_get_bytes(map, message_keys::PERSONAL_HASH)
                    .ok_or_else(|| anyhow!("confirm: missing personal_hash"))?;
                let active_array = map_get_array(map, message_keys::ACTIVE_SET)
                    .ok_or_else(|| anyhow!("confirm: missing active_set"))?;
                let active_set = active_array
                    .iter()
                    .map(|v| match v {
                        Value::Text(s) => Ok(s.clone()),
                        _ => bail!("active_set entry not text"),
                    })
                    .collect::<Result<Vec<_>>>()?;

                let accepted_params =
                    if let Some(params_map) = map_get_map(map, message_keys::ACCEPTED_PARAMS) {
                        let mut out = BTreeMap::new();
                        for (k, v) in &params_map {
                            if let Value::Text(cap_name) = k {
                                let scope = scope_from_cbor_value(v)?;
                                out.insert(cap_name.clone(), scope);
                            }
                        }
                        Some(out)
                    } else {
                        None
                    };

                Ok(ProtocolMessage::Confirm {
                    personal_hash,
                    active_set,
                    accepted_params,
                })
            }

            MessageType::Close => {
                let personal_hash = map_get_bytes(map, message_keys::PERSONAL_HASH)
                    .ok_or_else(|| anyhow!("close: missing personal_hash"))?;
                let reason_u64 = map_get_int(map, message_keys::REASON).unwrap_or(0);
                let reason = CloseReason::from_u64(reason_u64).unwrap_or(CloseReason::Error);
                Ok(ProtocolMessage::Close {
                    personal_hash,
                    reason,
                })
            }

            MessageType::Ping => {
                let timestamp = map_get_int(map, message_keys::TIMESTAMP)
                    .ok_or_else(|| anyhow!("ping: missing timestamp"))?;
                Ok(ProtocolMessage::Ping { timestamp })
            }

            MessageType::Pong => {
                let timestamp = map_get_int(map, message_keys::TIMESTAMP)
                    .ok_or_else(|| anyhow!("pong: missing timestamp"))?;
                Ok(ProtocolMessage::Pong { timestamp })
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CapabilityDeclaration, DiscoveryManifest, Role, ScopeParams};

    fn sample_manifest() -> DiscoveryManifest {
        let peer_id = [0xA1u8; 32];
        let caps = vec![
            CapabilityDeclaration {
                name: "core.session.heartbeat.1".to_string(),
                role: Role::Both,
                mutual: true,
                scope: None,
                applicable_scope_keys: None,
            },
            CapabilityDeclaration {
                name: "howm.social.feed.1".to_string(),
                role: Role::Provide,
                mutual: false,
                scope: Some(ScopeParams {
                    rate_limit: 10,
                    ttl: 3600,
                    ..Default::default()
                }),
                applicable_scope_keys: None,
            },
        ];
        let mut m = DiscoveryManifest {
            protocol_version: 1,
            peer_id,
            sequence_num: 1,
            capabilities: caps,
            personal_hash: vec![],
            hash_algorithm: "sha-256".to_string(),
        };
        m.personal_hash = personal_hash(&m);
        m
    }

    #[test]
    fn manifest_round_trip() {
        let m = sample_manifest();
        let encoded = m.to_cbor();
        let decoded = DiscoveryManifest::from_cbor(&encoded).unwrap();
        assert_eq!(m.protocol_version, decoded.protocol_version);
        assert_eq!(m.peer_id, decoded.peer_id);
        assert_eq!(m.sequence_num, decoded.sequence_num);
        assert_eq!(m.capabilities.len(), decoded.capabilities.len());
        assert_eq!(m.capabilities[0].name, decoded.capabilities[0].name);
        assert_eq!(m.hash_algorithm, decoded.hash_algorithm);
    }

    #[test]
    fn personal_hash_determinism() {
        let m = sample_manifest();
        let h1 = personal_hash(&m);
        let h2 = personal_hash(&m);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 32); // SHA-256 = 32 bytes
    }

    #[test]
    fn personal_hash_ignores_sequence_num() {
        let mut m1 = sample_manifest();
        m1.sequence_num = 1;
        let mut m2 = sample_manifest();
        m2.sequence_num = 99;
        // personal_hash should be the same regardless of sequence_num
        assert_eq!(personal_hash(&m1), personal_hash(&m2));
    }

    #[test]
    fn capabilities_sorted_in_cbor() {
        let mut m = sample_manifest();
        // Reverse the order
        m.capabilities.reverse();
        let encoded = m.to_cbor();
        let decoded = DiscoveryManifest::from_cbor(&encoded).unwrap();
        // Should be sorted after decode (encoding sorts them)
        let names: Vec<_> = decoded
            .capabilities
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["core.session.heartbeat.1", "howm.social.feed.1"]
        );
    }

    #[test]
    fn offer_round_trip() {
        let m = sample_manifest();
        let msg = ProtocolMessage::Offer {
            manifest: m.clone(),
        };
        let encoded = msg.encode();
        let decoded = ProtocolMessage::decode(&mut encoded.as_slice()).unwrap();
        match decoded {
            ProtocolMessage::Offer { manifest } => {
                assert_eq!(manifest.peer_id, m.peer_id);
                assert_eq!(manifest.capabilities.len(), m.capabilities.len());
            }
            _ => panic!("expected Offer"),
        }
    }

    #[test]
    fn confirm_round_trip() {
        let mut params = BTreeMap::new();
        params.insert(
            "howm.social.feed.1".to_string(),
            ScopeParams {
                rate_limit: 5,
                ttl: 3600,
                ..Default::default()
            },
        );
        let msg = ProtocolMessage::Confirm {
            personal_hash: vec![0xDE, 0xAD],
            active_set: vec![
                "core.session.heartbeat.1".to_string(),
                "howm.social.feed.1".to_string(),
            ],
            accepted_params: Some(params.clone()),
        };
        let encoded = msg.encode();
        let decoded = ProtocolMessage::decode(&mut encoded.as_slice()).unwrap();
        match decoded {
            ProtocolMessage::Confirm {
                personal_hash,
                active_set,
                accepted_params,
            } => {
                assert_eq!(personal_hash, vec![0xDE, 0xAD]);
                assert_eq!(active_set.len(), 2);
                let ap = accepted_params.unwrap();
                assert_eq!(ap["howm.social.feed.1"].rate_limit, 5);
            }
            _ => panic!("expected Confirm"),
        }
    }

    #[test]
    fn close_round_trip() {
        let msg = ProtocolMessage::Close {
            personal_hash: vec![0x01, 0x02],
            reason: CloseReason::NoMatch,
        };
        let encoded = msg.encode();
        let decoded = ProtocolMessage::decode(&mut encoded.as_slice()).unwrap();
        match decoded {
            ProtocolMessage::Close { reason, .. } => {
                assert_eq!(reason, CloseReason::NoMatch);
            }
            _ => panic!("expected Close"),
        }
    }

    #[test]
    fn ping_pong_round_trip() {
        let ts = 1_700_000_000u64;
        for msg in [
            ProtocolMessage::Ping { timestamp: ts },
            ProtocolMessage::Pong { timestamp: ts },
        ] {
            let encoded = msg.encode();
            let decoded = ProtocolMessage::decode(&mut encoded.as_slice()).unwrap();
            let decoded_ts = match decoded {
                ProtocolMessage::Ping { timestamp } | ProtocolMessage::Pong { timestamp } => {
                    timestamp
                }
                _ => panic!("unexpected message type"),
            };
            assert_eq!(decoded_ts, ts);
        }
    }

    #[test]
    fn length_prefix_correct() {
        let msg = ProtocolMessage::Ping { timestamp: 42 };
        let encoded = msg.encode();
        let len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize;
        assert_eq!(len + 4, encoded.len());
    }
}
