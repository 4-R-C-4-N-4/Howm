// Shared CBOR encode/decode helpers for capability handlers.
//
// All capability messages use CBOR maps with integer keys. These helpers
// provide a uniform interface for building and parsing those payloads.

use anyhow::{anyhow, bail, Result};
use ciborium::value::{Integer, Value};

use p2pcd_types::ProtocolMessage;

/// Encode a list of (key, value) pairs into a CBOR map byte vector.
pub fn cbor_encode_map(pairs: Vec<(u64, Value)>) -> Vec<u8> {
    let map: Vec<(Value, Value)> = pairs
        .into_iter()
        .map(|(k, v)| (Value::Integer(Integer::from(k)), v))
        .collect();
    let mut out = Vec::new();
    ciborium::ser::into_writer(&Value::Map(map), &mut out).expect("CBOR encode");
    out
}

/// Decode a CBOR payload into a map of (key, value) pairs.
pub fn decode_payload(payload: &[u8]) -> Result<Vec<(Value, Value)>> {
    let val: Value =
        ciborium::de::from_reader(payload).map_err(|e| anyhow!("CBOR decode: {e}"))?;
    match val {
        Value::Map(m) => Ok(m),
        _ => bail!("expected CBOR map payload"),
    }
}

/// Extract a uint value from a CBOR map by integer key.
pub fn cbor_get_int(map: &[(Value, Value)], key: u64) -> Option<u64> {
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

/// Extract a text string from a CBOR map by integer key.
pub fn cbor_get_text(map: &[(Value, Value)], key: u64) -> Option<String> {
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

/// Extract a byte string from a CBOR map by integer key.
pub fn cbor_get_bytes(map: &[(Value, Value)], key: u64) -> Option<Vec<u8>> {
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

/// Extract an array from a CBOR map by integer key.
pub fn cbor_get_array(map: &[(Value, Value)], key: u64) -> Option<Vec<Value>> {
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

/// Wrap a raw CBOR payload as a CapabilityMsg with the given message type.
pub fn make_capability_msg(msg_type: u64, payload: Vec<u8>) -> ProtocolMessage {
    ProtocolMessage::CapabilityMsg {
        message_type: msg_type,
        payload,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let encoded = cbor_encode_map(vec![
            (1, Value::Integer(Integer::from(42u64))),
            (2, Value::Text("hello".into())),
            (3, Value::Bytes(vec![0xDE, 0xAD])),
        ]);
        let map = decode_payload(&encoded).unwrap();
        assert_eq!(cbor_get_int(&map, 1), Some(42));
        assert_eq!(cbor_get_text(&map, 2).unwrap(), "hello");
        assert_eq!(cbor_get_bytes(&map, 3).unwrap(), vec![0xDE, 0xAD]);
        assert!(cbor_get_int(&map, 99).is_none());
    }

    #[test]
    fn array_round_trip() {
        let encoded = cbor_encode_map(vec![(
            1,
            Value::Array(vec![
                Value::Integer(Integer::from(10u64)),
                Value::Integer(Integer::from(20u64)),
            ]),
        )]);
        let map = decode_payload(&encoded).unwrap();
        let arr = cbor_get_array(&map, 1).unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn make_capability_msg_wraps_correctly() {
        let msg = make_capability_msg(42, vec![1, 2, 3]);
        match msg {
            ProtocolMessage::CapabilityMsg {
                message_type,
                payload,
            } => {
                assert_eq!(message_type, 42);
                assert_eq!(payload, vec![1, 2, 3]);
            }
            _ => panic!("expected CapabilityMsg"),
        }
    }
}
