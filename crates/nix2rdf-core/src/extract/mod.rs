//! The core extractor and the thin front-ends.
//!
//! `drv` is the core: derivation graph → drv/src/out fragments. Everything
//! else (flake, nixos, image, nixpkgs index/eval) feeds it and adds one
//! snapshot-level fragment of its own.

pub mod drv;
pub mod flake;
pub mod nixos;
pub mod image;
pub mod nixpkgs_index;
pub mod nixpkgs_eval;

pub use drv::{extract_graph, ExtractOptions, GraphExtraction};

use crate::error::Result;
use crate::fragment::Fragment;
use crate::store::{Store, WriteOutcome};
use tracing::info;

/// Write a batch of fragments, logging one structured line per file and a summary.
pub fn write_all(store: &Store, frags: &[Fragment], label: &str) -> Result<Vec<WriteOutcome>> {
    let mut outcomes = Vec::with_capacity(frags.len());
    let mut new = 0usize;
    let mut bytes = 0u64;
    for f in frags {
        let o = store.write(f)?;
        if o.written {
            new += 1;
            bytes += o.bytes;
        }
        outcomes.push(o);
    }
    info!(target: "nix2rdf::store", label, fragments = frags.len(), new, bytes_written = bytes, "fragments written");
    Ok(outcomes)
}
