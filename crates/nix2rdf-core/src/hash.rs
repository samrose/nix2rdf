//! Content hashing helpers. Everything content-addressed in nix2rdf uses
//! sha256 and prints it as lowercase hex, except where Nix's own hash form
//! (a store-path hash or an SRI NAR hash) already identifies the thing.

use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Hash an ordered sequence of items with an unambiguous separator, so that
/// `["a","b"]` and `["ab"]` differ.
pub fn sha256_hex_items<I, S>(items: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<[u8]>,
{
    let mut h = Sha256::new();
    for it in items {
        let b = it.as_ref();
        h.update((b.len() as u64).to_le_bytes());
        h.update(b);
    }
    hex::encode(h.finalize())
}

/// Short (16 hex chars) prefix of a full sha256, used only where the full
/// hash is already stored alongside as a literal.
pub fn short(hash: &str) -> &str {
    &hash[..hash.len().min(16)]
}

/// Canonical JSON: keys sorted, no whitespace. serde_json with
/// `preserve_order` keeps insertion order, so we re-sort explicitly.
pub fn canonical_json(v: &serde_json::Value) -> String {
    fn sort(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(m) => {
                let mut entries: Vec<(&String, &serde_json::Value)> = m.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                let mut out = serde_json::Map::new();
                for (k, v) in entries {
                    out.insert(k.clone(), sort(v));
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(a) => serde_json::Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort(v)).expect("json serialization cannot fail")
}
