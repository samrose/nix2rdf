//! Fragment publishing and fetching, so expensive evaluations (Layer 1)
//! happen once globally.
//!
//! A published set is a directory (served over HTTP or on disk) containing
//! the fragments under their store-relative paths and a `manifest.json`:
//! `{"version":1,"fragments":[{"path":"drv/<h>.nq.zst","sha256":"...","bytes":N}]}`.
//! Fetching downloads only fragments missing from the local store and
//! verifies the hash of every file.

use crate::error::{Error, Result};
use crate::hash;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub fragments: Vec<ManifestEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ManifestEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Write the given fragments (store-relative paths) and a manifest to `dest`.
pub fn publish(store: &Store, paths: &[PathBuf], dest: &Path) -> Result<Manifest> {
    std::fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;
    let mut entries = Vec::new();
    for p in paths {
        let abs = if p.is_absolute() {
            p.clone()
        } else {
            store.root().join(p)
        };
        let rel = store.relative(&abs);
        let bytes = std::fs::read(&abs).map_err(|e| Error::io(&abs, e))?;
        let target = dest.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(&target, &bytes).map_err(|e| Error::io(&target, e))?;
        entries.push(ManifestEntry {
            path: rel.to_string_lossy().to_string(),
            sha256: hash::sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let m = Manifest {
        version: 1,
        fragments: entries,
    };
    let mp = dest.join("manifest.json");
    std::fs::write(&mp, serde_json::to_vec_pretty(&m)?).map_err(|e| Error::io(&mp, e))?;
    info!(target: "nix2rdf::publish", fragments = m.fragments.len(), dest = %dest.display(), "published");
    Ok(m)
}

fn get_bytes(base: &str, rel: &str) -> Result<Vec<u8>> {
    if let Some(dir) = base.strip_prefix("file://") {
        let p = Path::new(dir).join(rel);
        return std::fs::read(&p).map_err(|e| Error::io(&p, e));
    }
    if !base.starts_with("http://") && !base.starts_with("https://") {
        let p = Path::new(base).join(rel);
        return std::fs::read(&p).map_err(|e| Error::io(&p, e));
    }
    let url = format!("{}/{}", base.trim_end_matches('/'), rel);
    let mut resp = ureq::get(&url)
        .call()
        .map_err(|e| Error::Other(format!("GET {url}: {e}")))?;
    let mut buf = Vec::new();
    resp.body_mut()
        .as_reader()
        .read_to_end(&mut buf)
        .map_err(|e| Error::Other(format!("GET {url}: {e}")))?;
    Ok(buf)
}

/// Fetch a published set into the store. Returns the number of new files.
pub fn fetch(store: &Store, base: &str) -> Result<usize> {
    let m: Manifest = serde_json::from_slice(&get_bytes(base, "manifest.json")?)?;
    let mut fetched = 0;
    for e in &m.fragments {
        if e.path.contains("..") || e.path.starts_with('/') {
            warn!(target: "nix2rdf::publish", path = %e.path, "rejecting unsafe manifest path");
            continue;
        }
        let target = store.root().join(&e.path);
        if target.exists() {
            continue;
        }
        let bytes = get_bytes(base, &e.path)?;
        let h = hash::sha256_hex(&bytes);
        if h != e.sha256 {
            return Err(Error::Other(format!(
                "{}: hash mismatch (manifest {}, got {h})",
                e.path, e.sha256
            )));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let tmp = target.with_extension("part");
        std::fs::write(&tmp, &bytes).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, &target).map_err(|e| Error::io(&target, e))?;
        fetched += 1;
    }
    info!(target: "nix2rdf::publish", source = base, listed = m.fragments.len(), fetched, "fetch done");
    Ok(fetched)
}
