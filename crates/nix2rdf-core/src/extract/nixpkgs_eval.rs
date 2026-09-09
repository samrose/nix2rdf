//! Layer 1 (opt-in): evaluate a whole nixpkgs revision with `nix-eval-jobs`,
//! emit every derivation and its direct inputs, plus `evaluatesTo` edges
//! from the revision. Output is publishable so evaluation happens once globally.
//!
//! Materialization: the tree is fetched by NAR hash (`builtins.fetchTree`
//! pinned with `narHash`), which is the same code path nixpkgs-multiverse's
//! `at` takes underneath. A custom `--expr` can substitute the multiverse
//! flake directly.

use super::drv::{add_meta, extract_from_drvs, ExtractOptions};
use super::nixpkgs_index::{revision_node, Revision};
use crate::error::{Error, Result};
use crate::fragment::{Fragment, FragmentKind};
use crate::iri;
use crate::nix::{drv_file, CliNix, EvalJob, Meta};
use crate::vocab::nix_terms as t;
use std::collections::BTreeMap;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct EvalOptions {
    pub revision: Revision,
    pub system: String,
    /// Override the expression handed to nix-eval-jobs.
    pub expr: Option<String>,
    pub workers: usize,
    pub max_memory_mb: usize,
    pub allow_unfree: bool,
    /// Read .drv files directly (fast) instead of `nix derivation show` batches.
    pub drv_files: bool,
    pub extract: ExtractOptions,
}

pub fn default_expr(o: &EvalOptions) -> String {
    let unfree = if o.allow_unfree { "true" } else { "false" };
    let nar = if o.revision.nar_hash.is_empty() { String::new() } else { format!(" narHash = \"{}\";", o.revision.nar_hash) };
    format!(
        "import (builtins.fetchTree {{ type = \"github\"; owner = \"NixOS\"; repo = \"nixpkgs\"; rev = \"{}\";{nar} }}) {{ system = \"{}\"; config.allowUnfree = {unfree}; config.allowBroken = false; }}",
        o.revision.rev, o.system
    )
}

/// Convert nix-eval-jobs output into a revision fragment plus the derivation graph.
pub fn fragments_from_jobs(nix: &CliNix, o: &EvalOptions, jobs: &[EvalJob]) -> Result<Vec<Fragment>> {
    let rev = &o.revision.rev;
    let mut f = Fragment::new(FragmentKind::Nixpkgs(rev.clone()));
    let rn = revision_node(&mut f, &o.revision, None);
    f.add_type(rn.clone(), t::Snapshot());

    let mut roots: Vec<String> = Vec::new();
    let mut metas: BTreeMap<String, (String, Meta)> = BTreeMap::new(); // drv path → (attr, meta)
    let mut errors = 0usize;
    for j in jobs {
        if let Some(e) = &j.error {
            errors += 1;
            let _ = e;
            continue;
        }
        let Some(drv) = &j.drv_path else { continue };
        roots.push(drv.clone());
        let attr = if j.attr_path.is_empty() { j.attr.clone() } else { j.attr_path.join(".") };
        let meta = j.meta.as_ref().map(Meta::from_json).unwrap_or_default();
        metas.insert(drv.clone(), (attr, meta));
    }
    roots.sort();
    roots.dedup();
    info!(target: "nix2rdf::nixpkgs_eval", rev = %rev, attributes = roots.len(), errors, "evaluation finished");

    let drvs = if o.drv_files {
        drv_file::read_closure(&roots)?
    } else {
        let mut all = BTreeMap::new();
        for chunk in roots.chunks(200) {
            let m = crate::nix::NixSource::derivation_show(nix, chunk, true)?;
            all.extend(m);
        }
        all
    };
    let graph = extract_from_drvs(nix, drvs, &o.extract)?;
    for (drv, (attr, meta)) in &metas {
        let Some(d) = graph.drv_iri(drv) else { continue };
        let a = iri::nixpkgs_attr(rev, attr);
        f.add_type(a.clone(), t::Attribute());
        f.add_str(a.clone(), t::attrPath(), attr);
        f.add(a.clone(), t::evaluatesTo(), d.clone());
        f.add(rn.clone(), t::hasAttribute(), a);
        f.add(rn.clone(), t::evaluatesTo(), d.clone());
        add_meta(&mut f, &d, meta);
    }
    // Every derivation in the closure is buildable from this tree.
    for h in graph.drv_hashes.values() {
        f.add(rn.clone(), t::evaluatesTo(), iri::drv(h));
    }
    let mut frags = graph.fragments;
    frags.push(f);
    Ok(frags)
}

pub fn extract_nixpkgs(nix: &CliNix, o: &EvalOptions) -> Result<Vec<Fragment>> {
    let expr = o.expr.clone().unwrap_or_else(|| default_expr(o));
    let workers = o.workers.max(1).to_string();
    let mem = o.max_memory_mb.max(512).to_string();
    let args = vec!["--expr", &expr, "--meta", "--workers", &workers, "--max-memory-size", &mem, "--force-recurse"];
    if o.revision.nar_hash.is_empty() {
        warn!(target: "nix2rdf::nixpkgs_eval", "no NAR hash for revision; fetchTree will not verify the tree");
    }
    let jobs = nix.eval_jobs(&args)?;
    fragments_from_jobs(nix, o, &jobs).map_err(|e| Error::Other(format!("nixpkgs-eval {}: {e}", o.revision.rev)))
}
