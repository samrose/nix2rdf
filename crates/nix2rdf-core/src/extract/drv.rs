//! Core extractor: a derivation graph (as returned by `nix derivation show -r`
//! or read from `.drv` files) → one fragment per derivation, one per source,
//! and optionally one per derivation's outputs once built.
//!
//! Every fragment here is a pure function of a single `DrvInfo` (plus the
//! NAR hashes of its inputSrcs), so it is byte-identical no matter which
//! front-end, pin, image or commit reached it. Dedupe is by file name.

use crate::error::{Error, Result};
use crate::fragment::{Fragment, FragmentKind};
use crate::hash;
use crate::iri;
use crate::nix::{DrvInfo, Meta, NixSource, PathInfo};
use crate::vocab::nix_terms as t;
use crate::vocab::xsd_any_uri;
use std::collections::{BTreeMap, BTreeSet};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvMode {
    /// Only `nix:envHash` plus the well-known keys (pname, version, ...).
    #[default]
    HashOnly,
    /// Additionally write the full environment as a side fragment `env/<hash>`.
    Full,
}

#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    pub env: EnvMode,
}

/// The result of extracting one derivation graph.
#[derive(Debug, Default)]
pub struct GraphExtraction {
    /// drv path → its store hash (IRI key).
    pub drv_hashes: BTreeMap<String, String>,
    /// Source store path → SRI NAR hash.
    pub src_hashes: BTreeMap<String, String>,
    pub fragments: Vec<Fragment>,
    /// The parsed graph, for front-ends that need more (e.g. roots' outputs).
    pub drvs: BTreeMap<String, DrvInfo>,
}

impl GraphExtraction {
    pub fn drv_iri(&self, drv_path: &str) -> Option<oxrdf::NamedNode> {
        self.drv_hashes.get(drv_path).map(|h| iri::drv(h))
    }
}

/// Well-known env keys recorded as properties. Everything else is only in
/// the env hash (or the optional env fragment).
const WELL_KNOWN: &[&str] = &["pname", "version"];

/// The derivation fragment. Contains exactly what the .drv says; nothing
/// that depends on build state or on who is asking.
pub fn derivation_fragment(
    drv_path: &str,
    d: &DrvInfo,
    src_hashes: &BTreeMap<String, String>,
) -> Result<Fragment> {
    let h = iri::store_path_hash(drv_path)
        .ok_or_else(|| Error::Other(format!("not a store path: {drv_path}")))?;
    let s = iri::drv(h);
    let mut f = Fragment::new(FragmentKind::Drv(h.to_string()));
    f.add_type(s.clone(), t::Derivation());
    f.add_str(s.clone(), t::drvPath(), drv_path);
    f.add_str(s.clone(), t::name(), &d.name);
    f.add_str(s.clone(), t::system(), &d.system);
    f.add_str(s.clone(), t::builder(), &d.builder);
    for (i, a) in d.args.iter().enumerate() {
        f.add_str(s.clone(), t::arg(), &format!("{i}:{a}"));
    }
    for k in WELL_KNOWN {
        if let Some(v) = d.attr(k) {
            let p = match *k {
                "pname" => t::pname(),
                _ => t::version(),
            };
            f.add_str(s.clone(), p, &v);
        }
    }
    let env_json = serde_json::to_value(&d.env)?;
    f.add_str(
        s.clone(),
        t::envHash(),
        &hash::sha256_hex(hash::canonical_json(&env_json).as_bytes()),
    );
    f.add_bool(s.clone(), t::structuredAttrs(), d.uses_structured_attrs());
    let ca = d.is_content_addressed();
    f.add_bool(s.clone(), t::contentAddressed(), ca);

    for (in_path, outs) in &d.input_drvs {
        let ih = iri::store_path_hash(in_path)
            .ok_or_else(|| Error::Other(format!("not a store path: {in_path}")))?;
        let dep = iri::drv(ih);
        f.add(s.clone(), t::inputDrv(), dep.clone());
        f.add(s.clone(), t::dependsOn(), dep);
        for o in outs.outputs() {
            f.add_str(s.clone(), t::inputDrvOutput(), &format!("{ih}!{o}"));
        }
    }
    for src in &d.input_srcs {
        let nar = src_hashes.get(src).ok_or_else(|| {
            Error::Other(format!("no NAR hash known for inputSrc {src} of {drv_path}; run with a store that has it (nix path-info)"))
        })?;
        f.add(s.clone(), t::inputSrc(), iri::src(nar));
    }
    // Input-addressed outputs are known now and belong to the derivation
    // fragment. Content-addressed outputs arrive later in an `out/` fragment
    // so this fragment stays byte-identical before and after the build.
    for (name, o) in &d.outputs {
        if let Some(p) = &o.path {
            let oh = iri::store_path_hash(p)
                .ok_or_else(|| Error::Other(format!("not a store path: {p}")))?;
            let on = iri::out(oh);
            f.add(s.clone(), t::hasOutput(), on.clone());
            f.add_type(on.clone(), t::Output());
            f.add_str(on.clone(), t::outputName(), name);
            f.add_str(on.clone(), t::storePath(), p);
        }
    }
    Ok(f)
}

/// The full environment as a side fragment (opt-in; can be large).
pub fn env_fragment(drv_path: &str, d: &DrvInfo) -> Result<Fragment> {
    let h = iri::store_path_hash(drv_path)
        .ok_or_else(|| Error::Other(format!("not a store path: {drv_path}")))?;
    let s = iri::drv(h);
    let mut f = Fragment::new(FragmentKind::Env(h.to_string()));
    for (k, v) in &d.env {
        f.add_str(s.clone(), t::envVar(), &format!("{k}={v}"));
    }
    Ok(f)
}

/// A source fragment: the store path as a `nix:Source` identified by NAR hash.
pub fn source_fragment(store_path: &str, nar_hash: &str) -> Fragment {
    let s = iri::src(nar_hash);
    let mut f = Fragment::new(FragmentKind::Src(nar_hash.to_string()));
    f.add_type(s.clone(), t::Source());
    f.add_str(s.clone(), t::narHash(), nar_hash);
    f.add_str(s.clone(), t::storePath(), store_path);
    if let Some(n) = iri::store_path_name(store_path) {
        f.add_str(s, t::name(), n);
    }
    f
}

/// The outputs fragment: outputs (with paths) and their runtime references.
/// Written after a build. For input-addressed derivations it adds only the
/// `references` edges (hasOutput is already in the drv fragment).
pub fn outputs_fragment(
    drv_path: &str,
    outputs: &BTreeMap<String, String>,
    infos: &BTreeMap<String, Option<PathInfo>>,
) -> Result<Fragment> {
    let h = iri::store_path_hash(drv_path)
        .ok_or_else(|| Error::Other(format!("not a store path: {drv_path}")))?;
    let s = iri::drv(h);
    let mut f = Fragment::new(FragmentKind::Out(h.to_string()));
    for (name, p) in outputs {
        let oh = iri::store_path_hash(p)
            .ok_or_else(|| Error::Other(format!("not a store path: {p}")))?;
        let on = iri::out(oh);
        f.add(s.clone(), t::hasOutput(), on.clone());
        f.add_type(on.clone(), t::Output());
        f.add_str(on.clone(), t::outputName(), name);
        f.add_str(on.clone(), t::storePath(), p);
        if let Some(Some(info)) = infos.get(p) {
            if let Some(nh) = &info.nar_hash {
                f.add_str(on.clone(), t::narHash(), nh);
            }
            for r in &info.references {
                if r == p {
                    continue; // self-reference
                }
                if let Some(rh) = iri::store_path_hash(r) {
                    f.add(on.clone(), t::references(), iri::out(rh));
                }
            }
        }
    }
    Ok(f)
}

/// Attach meta.* to a derivation inside a *snapshot* fragment (never inside
/// the drv fragment, because meta depends on the attribute path asked for).
pub fn add_meta(f: &mut Fragment, drv: &oxrdf::NamedNode, meta: &Meta) {
    for l in &meta.licenses {
        f.add_str(drv.clone(), t::license(), l);
    }
    if meta.unfree {
        f.add_bool(drv.clone(), t::unfree(), true);
    }
    if let Some(h) = &meta.homepage {
        f.add_typed(drv.clone(), t::homepage(), h, xsd_any_uri());
    }
    if let Some(d) = &meta.description {
        f.add_str(drv.clone(), t::description(), d);
    }
    for sp in &meta.source_provenance {
        f.add_str(drv.clone(), t::sourceProvenance(), sp);
    }
}

/// Extract a derivation graph reachable from `installables`.
pub fn extract_graph(
    nix: &dyn NixSource,
    installables: &[String],
    opts: &ExtractOptions,
) -> Result<GraphExtraction> {
    let drvs = nix.derivation_show(installables, true)?;
    extract_from_drvs(nix, drvs, opts)
}

/// Same, from an already-parsed graph (used by the .drv fast path and nix-eval-jobs).
pub fn extract_from_drvs(
    nix: &dyn NixSource,
    drvs: BTreeMap<String, DrvInfo>,
    opts: &ExtractOptions,
) -> Result<GraphExtraction> {
    info!(target: "nix2rdf::extract", derivations = drvs.len(), "extracting derivation graph");
    let srcs: BTreeSet<String> = drvs
        .values()
        .flat_map(|d| d.input_srcs.iter().cloned())
        .collect();
    let src_list: Vec<String> = srcs.iter().cloned().collect();
    let infos = if src_list.is_empty() {
        BTreeMap::new()
    } else {
        nix.path_info(&src_list)?
    };
    let mut src_hashes = BTreeMap::new();
    let mut missing = Vec::new();
    for p in &src_list {
        match infos.get(p) {
            Some(Some(i)) if i.nar_hash.is_some() => {
                src_hashes.insert(p.clone(), i.nar_hash.clone().unwrap());
            }
            _ => missing.push(p.clone()),
        }
    }
    if !missing.is_empty() {
        warn!(target: "nix2rdf::extract", count = missing.len(), first = %missing[0], "inputSrcs without NAR hash");
        return Err(Error::Other(format!(
            "{} inputSrcs have no NAR hash (first: {}); they must be present in the local store",
            missing.len(),
            missing[0]
        )));
    }

    let mut ex = GraphExtraction {
        drvs: BTreeMap::new(),
        ..Default::default()
    };
    for (path, nar) in &src_hashes {
        ex.fragments.push(source_fragment(path, nar));
    }
    for (path, d) in &drvs {
        let h = iri::store_path_hash(path)
            .ok_or_else(|| Error::Other(format!("not a store path: {path}")))?;
        ex.drv_hashes.insert(path.clone(), h.to_string());
        ex.fragments
            .push(derivation_fragment(path, d, &src_hashes)?);
        if opts.env == EnvMode::Full {
            ex.fragments.push(env_fragment(path, d)?);
        }
    }
    ex.src_hashes = src_hashes;
    ex.drvs = drvs;
    Ok(ex)
}

/// After a build: outputs fragments for the given drvs, with references.
pub fn extract_outputs(
    nix: &dyn NixSource,
    built: &[crate::nix::BuildResult],
) -> Result<Vec<Fragment>> {
    let paths: Vec<String> = built
        .iter()
        .flat_map(|b| b.outputs.values().cloned())
        .collect();
    let infos = if paths.is_empty() {
        BTreeMap::new()
    } else {
        nix.path_info(&paths)?
    };
    let mut out = Vec::new();
    for b in built {
        out.push(outputs_fragment(&b.drv_path, &b.outputs, &infos)?);
    }
    Ok(out)
}

/// Walk the runtime closure of the given store paths using path-info
/// references, returning outputs fragments for every derivation reached.
/// Used by the image and NixOS front-ends to record `references` edges for
/// the whole runtime closure (`nix path-info -r` semantics, but batched).
pub fn extract_runtime_closure(
    nix: &dyn NixSource,
    roots: &[String],
) -> Result<(Vec<Fragment>, BTreeMap<String, PathInfo>)> {
    let mut seen: BTreeMap<String, PathInfo> = BTreeMap::new();
    let mut frontier: Vec<String> = roots.to_vec();
    while !frontier.is_empty() {
        let infos = nix.path_info(&frontier)?;
        frontier.clear();
        for (p, i) in infos {
            if let Some(i) = i {
                for r in &i.references {
                    if !seen.contains_key(r) && r != &p {
                        frontier.push(r.clone());
                    }
                }
                seen.insert(p, i);
            }
        }
        frontier.sort();
        frontier.dedup();
        frontier.retain(|p| !seen.contains_key(p));
    }
    // Group by deriver; paths without a known deriver are still emitted as
    // Output nodes under a synthetic per-path fragment keyed by the output hash.
    let mut by_drv: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (p, i) in &seen {
        if let Some(d) = &i.deriver {
            let name = iri::store_path_name(p).unwrap_or("out").to_string();
            by_drv.entry(d.clone()).or_default().insert(name, p.clone());
        }
    }
    let infos_opt: BTreeMap<String, Option<PathInfo>> = seen
        .iter()
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect();
    let mut frags = Vec::new();
    for (drv, outs) in &by_drv {
        // The output *name* is not in path-info; recover it from the drv when possible.
        let named = match nix.derivation_show(std::slice::from_ref(drv), false) {
            Ok(m) => m.get(drv).map(|d| {
                d.outputs
                    .iter()
                    .filter_map(|(n, o)| o.path.clone().map(|p| (n.clone(), p)))
                    .filter(|(_, p)| outs.values().any(|x| x == p))
                    .collect::<BTreeMap<_, _>>()
            }),
            Err(_) => None,
        };
        let outs = named
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| outs.clone());
        frags.push(outputs_fragment(drv, &outs, &infos_opt)?);
    }
    Ok((frags, seen))
}
