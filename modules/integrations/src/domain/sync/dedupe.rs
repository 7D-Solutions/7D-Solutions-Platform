use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Truncate a timestamp to millisecond precision.
///
/// This is the single canonical truncation point.  All code paths that produce
/// a `last_updated_time` for the observations table MUST call this function so
/// that equality comparisons never fail due to sub-millisecond differences in
/// ISO-8601 strings from different providers.
pub fn truncate_to_millis(ts: DateTime<Utc>) -> DateTime<Utc> {
    let millis = ts.timestamp_millis();
    Utc.timestamp_millis_opt(millis).single().unwrap_or(ts)
}

/// Compute the observation fingerprint from provider signals.
///
/// Priority order (first non-None wins):
///   1. `sync_token` present  → `"st:<token>"`
///   2. `last_updated_time` present → `"ts:<epoch_millis>"`
///   3. neither present       → `"ph:<sha256(canonical_payload)>"`
///
/// The fallback to payload hash prevents fingerprint collapse: without it,
/// every observation for the same entity with no versioning metadata would
/// share an identical key and later rows would silently overwrite earlier ones.
pub fn compute_fingerprint(
    sync_token: Option<&str>,
    last_updated_time: Option<DateTime<Utc>>,
    raw_payload: &Value,
) -> String {
    if let Some(token) = sync_token {
        return format!("st:{token}");
    }
    if let Some(ts) = last_updated_time {
        let ms = truncate_to_millis(ts).timestamp_millis();
        return format!("ts:{ms}");
    }
    // Neither sync_token nor timestamp: hash the canonical payload bytes.
    let canonical = canonical_payload_bytes(raw_payload);
    let hash = hex::encode(Sha256::digest(&canonical));
    format!("ph:{hash}")
}

/// Compute the comparable hash for a projection.
///
/// The hash is a SHA-256 over the canonical JSON of the comparable fields plus
/// the millisecond-truncated timestamp (as epoch millis).  Using the truncated
/// timestamp ensures that two observations representing the same logical state
/// always produce the same hash regardless of sub-millisecond string formatting.
///
/// `comparable_fields` should contain only the semantically meaningful fields
/// from the provider payload — exclude ephemeral metadata (e.g., raw timestamps,
/// internal provider IDs that change on re-sync).
pub fn compute_comparable_hash(
    comparable_fields: &Value,
    last_updated_time: DateTime<Utc>,
) -> String {
    let truncated_ms = truncate_to_millis(last_updated_time).timestamp_millis();
    let combined = serde_json::json!({
        "fields": comparable_fields,
        "last_updated_ms": truncated_ms,
    });
    let canonical = canonical_payload_bytes(&combined);
    hex::encode(Sha256::digest(&canonical))
}

/// Canonical byte representation of a JSON value for hashing.
///
/// Uses compact serialization with sorted object keys (via `serde_json`'s
/// deterministic output) so the same logical value always hashes identically.
fn canonical_payload_bytes(value: &Value) -> Vec<u8> {
    // serde_json serializes object keys in insertion order; to guarantee
    // determinism regardless of provider key order we re-parse through a
    // BTreeMap-based pass.
    let sorted = sort_keys(value);
    serde_json::to_vec(&sorted).unwrap_or_default()
}

/// Recursively sort object keys so JSON serialization is deterministic.
fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: serde_json::Map<String, Value> = serde_json::Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                sorted.insert(k.clone(), sort_keys(&map[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_keys).collect()),
        other => other.clone(),
    }
}

/// Compute the server-side deterministic idempotency key for a conflict resolution item.
///
/// Key is `sha256("{conflict_id}:{action}:{authority_version}")` as a hex string.
/// The server always computes this — caller-supplied keys are stored as aliases only.
pub fn compute_resolve_det_key(conflict_id: Uuid, action: &str, authority_version: i64) -> String {
    let input = format!("{conflict_id}:{action}:{authority_version}");
    hex::encode(Sha256::digest(input.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn ts(millis: i64) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(millis)
            .single()
            .expect("valid test millis")
    }

    #[test]
    fn truncate_drops_sub_millisecond() {
        // 1_000_000_001_234_000 ns = 1_000_000_001.234 s = slightly past epoch+1B s
        // Build a timestamp with microsecond precision that truncation removes.
        let precise = Utc.timestamp_nanos(1_700_000_000_123_456_789_i64);
        let truncated = truncate_to_millis(precise);
        // Sub-millisecond (456_789 ns) must be gone; milliseconds kept.
        assert_eq!(truncated.timestamp_subsec_micros() % 1000, 0);
        assert_eq!(truncated.timestamp_millis(), precise.timestamp_millis());
    }

    #[test]
    fn compute_fingerprint_prefers_sync_token() {
        let ts = ts(1_700_000_000_000);
        let payload = json!({"id": 1});
        let fp = compute_fingerprint(Some("tok-abc"), Some(ts), &payload);
        assert_eq!(fp, "st:tok-abc");
    }

    #[test]
    fn compute_fingerprint_falls_back_to_timestamp() {
        let ts = ts(1_700_000_000_123);
        let payload = json!({"id": 1});
        let fp = compute_fingerprint(None, Some(ts), &payload);
        assert_eq!(fp, "ts:1700000000123");
    }

    #[test]
    fn compute_fingerprint_falls_back_to_payload_hash() {
        let payload = json!({"id": 42, "name": "test"});
        let fp = compute_fingerprint(None, None, &payload);
        assert!(
            fp.starts_with("ph:"),
            "must start with 'ph:' prefix, got: {fp}"
        );
        assert_eq!(fp.len(), 3 + 64, "ph: + 64-char hex sha256");
    }

    #[test]
    fn payload_hash_fingerprint_is_deterministic() {
        let payload = json!({"b": 2, "a": 1});
        let fp1 = compute_fingerprint(None, None, &payload);
        let fp2 = compute_fingerprint(None, None, &payload);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn payload_hash_fingerprint_is_key_order_independent() {
        // Same logical object, different insertion order → same hash.
        let a = json!({"a": 1, "b": 2});
        let b = json!({"b": 2, "a": 1});
        let fp_a = compute_fingerprint(None, None, &a);
        let fp_b = compute_fingerprint(None, None, &b);
        assert_eq!(fp_a, fp_b, "key order must not affect fingerprint");
    }

    #[test]
    fn comparable_hash_is_timestamp_precision_independent() {
        let fields = json!({"amount": 100});
        // Two timestamps that are equal at millisecond precision but differ at
        // nanosecond level must produce the same comparable_hash.
        let ts_ns1 = Utc.timestamp_nanos(1_700_000_000_123_000_000_i64);
        let ts_ns2 = Utc.timestamp_nanos(1_700_000_000_123_999_999_i64);
        assert_ne!(ts_ns1, ts_ns2, "test setup: timestamps must differ");

        let h1 = compute_comparable_hash(&fields, ts_ns1);
        let h2 = compute_comparable_hash(&fields, ts_ns2);
        assert_eq!(h1, h2, "same millisecond → same comparable_hash");
    }

    #[test]
    fn comparable_hash_differs_on_different_millis() {
        let fields = json!({"amount": 100});
        let h1 = compute_comparable_hash(&fields, ts(1_700_000_000_000));
        let h2 = compute_comparable_hash(&fields, ts(1_700_000_001_000));
        assert_ne!(h1, h2);
    }
}
