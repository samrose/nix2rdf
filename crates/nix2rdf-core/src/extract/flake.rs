//! Flake front-end: lock graph (pins, inputs, follows), the reachable
//! derivation graph of the selected outputs, a snapshot rooted at the flake's
//! own pin, and optionally a Commit node handed in by CI.

use super::drv::{add_meta, extract_graph, ExtractOptions, GraphExtraction};
use crate::error::{Error, Result};
use crate::fragment::{Fragment, FragmentKind};
use crate::iri;
use crate::nix::{EvalArgs, Meta, NixSource};
use crate::vocab::nix_terms as t;
use crate::vocab::xsd_date_time;
use oxrdf::NamedNode;
use std::collections::BTreeMap;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub repo: String,
    pub sha: String,
    /// RFC 3339 timestamp.
    pub committed_at: String,
}

#[derive(Debug, Clone)]
pub struct FlakeOptions {
    pub flake_ref: String,
    /// Attribute paths relative to the flake (e.g. `packages.x86_64-linux.hello`).
    /// Empty → every `packages.<system>.*` from `nix flake show`.
    pub attrs: Vec<String>,
    pub system: String,
    pub commit: Option<CommitInfo>,
    pub with_meta: bool,
    pub extract: ExtractOptions,
}

#[derive(Debug)]
pub struct FlakeExtraction {
    pub root_nar_hash: String,
    pub snapshot: NamedNode,
    pub fragments: Vec<Fragment>,
    pub graph: GraphExtraction,
    pub root_drvs: BTreeMap<String, String>, // attr → drv path
}

/// The system string Nix would use on this host, without calling Nix.
pub fn current_system() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => other,
    };
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{arch}-{os}")
}

fn str_of<'a>(v: &'a serde_json::Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str())
}

/// Render a lock `original`/`locked` entry as a flake reference string.
pub fn flake_ref_string(o: &serde_json::Value) -> String {
    let ty = str_of(o, "type").unwrap_or("");
    match ty {
        "github" | "gitlab" | "sourcehut" => {
            let mut s = format!("{ty}:{}/{}", str_of(o, "owner").unwrap_or(""), str_of(o, "repo").unwrap_or(""));
            if let Some(r) = str_of(o, "rev") {
                s.push('/');
                s.push_str(r);
            } else if let Some(r) = str_of(o, "ref") {
                s.push('/');
                s.push_str(r);
            }
            if let Some(d) = str_of(o, "dir") {
                s.push_str("?dir=");
                s.push_str(d);
            }
            s
        }
        "indirect" => format!("flake:{}", str_of(o, "id").unwrap_or("")),
        "path" => format!("path:{}", str_of(o, "path").unwrap_or("")),
        "git" | "hg" | "tarball" | "file" => {
            let mut s = format!("{ty}+{}", str_of(o, "url").unwrap_or(""));
            let mut q = vec![];
            if let Some(r) = str_of(o, "ref") {
                q.push(format!("ref={r}"));
            }
            if let Some(r) = str_of(o, "rev") {
                q.push(format!("rev={r}"));
            }
            if !q.is_empty() {
                s.push('?');
                s.push_str(&q.join("&"));
            }
            s
        }
        _ => crate::hash::canonical_json(o),
    }
}

/// Add the description of one locked node to a fragment.
fn add_pin(f: &mut Fragment, pin: &NamedNode, locked: &serde_json::Value) {
    f.add_type(pin.clone(), t::FlakePin());
    if let Some(n) = str_of(locked, "narHash") {
        f.add_str(pin.clone(), t::narHash(), n);
    }
    if let Some(v) = str_of(locked, "type") {
        f.add_str(pin.clone(), t::lockedType(), v);
    }
    if let Some(v) = str_of(locked, "rev") {
        f.add_str(pin.clone(), t::rev(), v);
    }
    if let Some(v) = str_of(locked, "ref") {
        f.add_str(pin.clone(), t::r#ref(), v);
    }
    if let Some(v) = str_of(locked, "owner") {
        f.add_str(pin.clone(), t::lockedOwner(), v);
    }
    if let Some(v) = str_of(locked, "repo") {
        f.add_str(pin.clone(), t::lockedRepo(), v);
    }
    if let Some(v) = str_of(locked, "url") {
        f.add_str(pin.clone(), t::lockedUrl(), v);
    }
    if let Some(v) = str_of(locked, "dir") {
        f.add_str(pin.clone(), t::lockedDir(), v);
    }
    if let Some(v) = locked.get("lastModified").and_then(|x| x.as_i64()) {
        f.add_int(pin.clone(), t::lastModified(), v);
    }
}

/// Resolve a `follows` path (e.g. ["a", "nixpkgs"]) from the root to
/// (parent node id, input name).
fn resolve_follows<'a>(nodes: &'a serde_json::Value, root: &'a str, path: &[String]) -> Option<(String, String)> {
    if path.is_empty() {
        return None;
    }
    let mut cur = root.to_string();
    for name in &path[..path.len() - 1] {
        let inputs = nodes.get(&cur)?.get("inputs")?;
        match inputs.get(name)? {
            serde_json::Value::String(id) => cur = id.clone(),
            serde_json::Value::Array(p) => {
                let p: Vec<String> = p.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                let (parent, n) = resolve_follows(nodes, root, &p)?;
                let inputs = nodes.get(&parent)?.get("inputs")?;
                cur = inputs.get(&n)?.as_str()?.to_string();
            }
            _ => return None,
        }
    }
    Some((cur, path[path.len() - 1].clone()))
}

/// Build the lock graph into `root_frag`, plus one small fragment per nested pin.
pub fn lock_graph(meta: &serde_json::Value, root_nar: &str, root_frag: &mut Fragment) -> Result<Vec<Fragment>> {
    let locks = meta.get("locks").ok_or_else(|| Error::Other("flake metadata has no locks".into()))?;
    let nodes = locks.get("nodes").ok_or_else(|| Error::Other("flake.lock has no nodes".into()))?;
    let root_id = str_of(locks, "root").unwrap_or("root").to_string();
    let root_pin = iri::pin(root_nar);
    let mut extra = Vec::new();

    let node_ids: Vec<String> = nodes.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    // node id → pin IRI (root node → root pin; locked nodes → pin by narHash)
    let mut pin_of: BTreeMap<String, NamedNode> = BTreeMap::new();
    for id in &node_ids {
        let node = &nodes[id];
        if id == &root_id {
            pin_of.insert(id.clone(), root_pin.clone());
        } else if let Some(nar) = node.get("locked").and_then(|l| str_of(l, "narHash")) {
            pin_of.insert(id.clone(), iri::pin(nar));
        }
    }
    for id in &node_ids {
        let node = &nodes[id];
        let Some(parent_pin) = pin_of.get(id).cloned() else { continue };
        if id != &root_id {
            if let Some(locked) = node.get("locked") {
                let nar = str_of(locked, "narHash").unwrap_or_default();
                let mut pf = Fragment::new(FragmentKind::Pin(nar.to_string()));
                add_pin(&mut pf, &parent_pin, locked);
                extra.push(pf);
                // Also describe it in the root fragment so the lock graph is self-contained.
                add_pin(root_frag, &parent_pin, locked);
            }
        }
        let Some(inputs) = node.get("inputs").and_then(|i| i.as_object()) else { continue };
        for (name, target) in inputs {
            let input = iri::flake_input(root_nar, id, name);
            root_frag.add_type(input.clone(), t::FlakeInput());
            root_frag.add_str(input.clone(), t::name(), name);
            root_frag.add(parent_pin.clone(), t::locksInput(), input.clone());
            match target {
                serde_json::Value::String(target_id) => {
                    root_frag.add_str(input.clone(), t::lockNodeId(), target_id);
                    if let Some(tp) = pin_of.get(target_id) {
                        root_frag.add(input.clone(), t::pinnedTo(), tp.clone());
                    }
                    if let Some(tn) = nodes.get(target_id) {
                        if let Some(orig) = tn.get("original") {
                            root_frag.add_str(input.clone(), t::originalRef(), &flake_ref_string(orig));
                        }
                        if tn.get("flake").and_then(|b| b.as_bool()) == Some(false) {
                            root_frag.add_bool(input.clone(), t::isFlake(), false);
                        }
                    }
                }
                serde_json::Value::Array(p) => {
                    let path: Vec<String> = p.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                    match resolve_follows(nodes, &root_id, &path) {
                        Some((parent, n)) => {
                            root_frag.add(input.clone(), t::follows(), iri::flake_input(root_nar, &parent, &n));
                        }
                        None => warn!(target: "nix2rdf::flake", input = %name, "unresolvable follows path"),
                    }
                }
                _ => {}
            }
        }
    }
    Ok(extra)
}

/// Default installables: `packages.<system>.*` from `nix flake show`.
fn default_attrs(nix: &dyn NixSource, flake_ref: &str, system: &str) -> Result<Vec<String>> {
    let show = nix.flake_show(flake_ref)?;
    let mut attrs = Vec::new();
    if let Some(pk) = show.get("packages").and_then(|p| p.get(system)).and_then(|s| s.as_object()) {
        for name in pk.keys() {
            attrs.push(format!("packages.{system}.{name}"));
        }
    }
    if attrs.is_empty() {
        return Err(Error::Other(format!("no packages.{system}.* in {flake_ref}; pass --attr")));
    }
    Ok(attrs)
}

pub fn meta_for(nix: &dyn NixSource, flake_ref: &str, attr: &str) -> Option<Meta> {
    let args = EvalArgs {
        installable: Some(format!("{flake_ref}#{attr}.meta")),
        apply: Some("m: { license = m.license or null; homepage = m.homepage or null; description = m.description or null; sourceProvenance = m.sourceProvenance or []; }".into()),
        label: format!("meta/{}", iri::encode_segment(attr)),
        ..Default::default()
    };
    match nix.eval_json(&args) {
        Ok(v) => Some(Meta::from_json(&v)),
        Err(e) => {
            warn!(target: "nix2rdf::flake", attr, error = %e, "meta not available");
            None
        }
    }
}

pub fn extract_flake(nix: &dyn NixSource, opts: &FlakeOptions) -> Result<FlakeExtraction> {
    let meta = nix.flake_metadata(&opts.flake_ref)?;
    let locked = meta.get("locked").cloned().unwrap_or(serde_json::Value::Null);
    let root_nar = match str_of(&locked, "narHash") {
        Some(n) => n.to_string(),
        None => {
            // A dirty local checkout: hash the store copy of the source.
            let path = str_of(&meta, "path").ok_or_else(|| Error::Other("flake metadata has neither locked.narHash nor path".into()))?.to_string();
            let infos = nix.path_info(&[path.clone()])?;
            infos.get(&path).cloned().flatten().and_then(|i| i.nar_hash).ok_or_else(|| Error::Other(format!("no NAR hash for {path}")))?
        }
    };
    info!(target: "nix2rdf::flake", flake = %opts.flake_ref, root_nar_hash = %root_nar, "flake resolved");

    let root_pin = iri::pin(&root_nar);
    let mut root_frag = Fragment::new(FragmentKind::Pin(root_nar.clone()));
    add_pin(&mut root_frag, &root_pin, &locked);
    root_frag.add_type(root_pin.clone(), t::Snapshot());
    if let Some(u) = str_of(&meta, "resolvedUrl").or(str_of(&meta, "url")) {
        root_frag.add_str(root_pin.clone(), t::lockedUrl(), u);
    }
    let mut fragments = lock_graph(&meta, &root_nar, &mut root_frag)?;

    let attrs = if opts.attrs.is_empty() { default_attrs(nix, &opts.flake_ref, &opts.system)? } else { opts.attrs.clone() };
    let installables: Vec<String> = attrs.iter().map(|a| format!("{}#{a}", opts.flake_ref)).collect();

    // Roots first (non-recursive), then the whole graph.
    let roots = nix.derivation_show(&installables, false)?;
    let graph = extract_graph(nix, &installables, &opts.extract)?;

    // Map attr → root drv path. `derivation show` keys by drv path and loses the
    // attr; recover it by asking one installable at a time only when there are several.
    let mut root_drvs: BTreeMap<String, String> = BTreeMap::new();
    if attrs.len() == 1 {
        if let Some(p) = roots.keys().next() {
            root_drvs.insert(attrs[0].clone(), p.clone());
        }
    } else {
        for (a, inst) in attrs.iter().zip(&installables) {
            match nix.derivation_show(std::slice::from_ref(inst), false) {
                Ok(m) => {
                    if let Some(p) = m.keys().next() {
                        root_drvs.insert(a.clone(), p.clone());
                    }
                }
                Err(e) => warn!(target: "nix2rdf::flake", attr = %a, error = %e, "cannot resolve root"),
            }
        }
        if root_drvs.is_empty() {
            for (i, p) in roots.keys().enumerate() {
                root_drvs.insert(attrs.get(i).cloned().unwrap_or_else(|| p.clone()), p.clone());
            }
        }
    }

    for (attr, drv_path) in &root_drvs {
        let Some(d) = graph.drv_iri(drv_path) else { continue };
        let a = iri::pin_attr(&root_nar, attr);
        root_frag.add_type(a.clone(), t::Attribute());
        root_frag.add_str(a.clone(), t::attrPath(), attr);
        root_frag.add(a.clone(), t::evaluatesTo(), d.clone());
        root_frag.add(root_pin.clone(), t::hasAttribute(), a);
        root_frag.add(root_pin.clone(), t::hasRoot(), d.clone());
        if opts.with_meta {
            if let Some(m) = meta_for(nix, &opts.flake_ref, attr) {
                add_meta(&mut root_frag, &d, &m);
            }
        }
    }
    for h in graph.drv_hashes.values() {
        root_frag.add(root_pin.clone(), t::closureContains(), iri::drv(h));
    }

    fragments.extend(graph.fragments.iter().cloned());
    fragments.push(root_frag);

    if let Some(c) = &opts.commit {
        fragments.push(commit_fragment(c, &root_pin));
    }
    Ok(FlakeExtraction { root_nar_hash: root_nar, snapshot: root_pin, fragments, graph, root_drvs })
}

pub fn commit_fragment(c: &CommitInfo, snapshot: &NamedNode) -> Fragment {
    let node = iri::commit(&c.repo, &c.sha);
    let mut f = Fragment::new(FragmentKind::Commit { repo: c.repo.clone(), sha: c.sha.clone() });
    f.add_type(node.clone(), t::Commit());
    f.add_str(node.clone(), t::rev(), &c.sha);
    f.add_str(node.clone(), t::repo(), &c.repo);
    f.add_typed(node.clone(), t::committedAt(), &c.committed_at, xsd_date_time());
    f.add(node, t::snapshot(), snapshot.clone());
    f
}
