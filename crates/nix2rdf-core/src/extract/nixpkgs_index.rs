//! Layer 0: the nixpkgs-multiverse index as RDF, with zero evaluation.
//! Input: `revisions.json`, `releases.json`, `index/versions.json`.

use crate::error::{Error, Result};
use crate::fragment::{Fragment, FragmentKind};
use crate::hash;
use crate::iri;
use crate::vocab::nix_terms as t;
use crate::vocab::xsd_date;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use tracing::info;

#[derive(Debug, Clone, Deserialize)]
pub struct Revision {
    pub rev: String,
    pub date: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "narHash", default)]
    pub nar_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub build: Option<i64>,
    pub date: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "narHash", default)]
    pub nar_hash: String,
    pub rev: String,
}

#[derive(Debug, Deserialize)]
struct Versions {
    attrs: BTreeMap<String, BTreeMap<String, Option<usize>>>,
    #[serde(rename = "revisionCount", default)]
    revision_count: usize,
}

pub struct IndexFiles {
    pub revisions: Vec<u8>,
    pub releases: Vec<u8>,
    pub versions: Vec<u8>,
}

impl IndexFiles {
    pub fn read(dir: &Path) -> Result<Self> {
        let rd = |p: &Path| std::fs::read(p).map_err(|e| Error::io(p, e));
        Ok(IndexFiles {
            revisions: rd(&dir.join("revisions.json"))?,
            releases: rd(&dir.join("releases.json"))?,
            versions: rd(&dir.join("index/versions.json"))?,
        })
    }
    pub fn content_hash(&self) -> String {
        hash::sha256_hex_items([&self.revisions, &self.releases, &self.versions])
    }
}

pub fn revision_node(f: &mut Fragment, r: &Revision, offset: Option<usize>) -> oxrdf::NamedNode {
    let n = iri::nixpkgs(&r.rev);
    f.add_type(n.clone(), t::NixpkgsRevision());
    f.add_str(n.clone(), t::rev(), &r.rev);
    f.add_typed(n.clone(), t::date(), &r.date, xsd_date());
    if !r.name.is_empty() {
        f.add_str(n.clone(), t::channelName(), &r.name);
    }
    if !r.nar_hash.is_empty() {
        f.add_str(n.clone(), t::narHash(), &r.nar_hash);
    }
    if let Some(o) = offset {
        f.add_int(n.clone(), t::indexOffset(), o as i64);
    }
    n
}

pub fn index_fragment(files: &IndexFiles) -> Result<Fragment> {
    let revisions: Vec<Revision> = serde_json::from_slice(&files.revisions)?;
    let releases: BTreeMap<String, Release> = serde_json::from_slice(&files.releases)?;
    let versions: Versions = serde_json::from_slice(&files.versions)?;
    let h = files.content_hash();
    let mut f = Fragment::new(FragmentKind::Index(h.clone()));
    for (i, r) in revisions.iter().enumerate() {
        revision_node(&mut f, r, Some(i));
    }
    for (name, rel) in &releases {
        let rn = iri::release(name);
        f.add_type(rn.clone(), t::Release());
        f.add_str(rn.clone(), t::name(), name);
        f.add_typed(rn.clone(), t::date(), &rel.date, xsd_date());
        if let Some(b) = rel.build {
            f.add_int(rn.clone(), t::releaseBuild(), b);
        }
        if !rel.name.is_empty() {
            f.add_str(rn.clone(), t::channelName(), &rel.name);
        }
        let rev = revision_node(&mut f, &Revision { rev: rel.rev.clone(), date: rel.date.clone(), name: rel.name.clone(), nar_hash: rel.nar_hash.clone() }, None);
        f.add(rn, t::releasePinnedTo(), rev);
    }
    let tip = revisions.last();
    let mut count = 0usize;
    for (attr, vers) in &versions.attrs {
        for (version, offset) in vers {
            let pv = iri::package_version(attr, version);
            f.add_type(pv.clone(), t::PackageVersion());
            f.add_str(pv.clone(), t::attrPath(), attr);
            f.add_str(pv.clone(), t::version(), version);
            match offset {
                Some(o) => {
                    if let Some(r) = revisions.get(*o) {
                        f.add(pv.clone(), t::lastSeenIn(), iri::nixpkgs(&r.rev));
                    }
                }
                None => {
                    f.add_bool(pv.clone(), t::currentAtIndexTip(), true);
                    if let Some(r) = tip {
                        f.add(pv.clone(), t::lastSeenIn(), iri::nixpkgs(&r.rev));
                    }
                }
            }
            count += 1;
        }
    }
    info!(target: "nix2rdf::index", revisions = revisions.len(), releases = releases.len(), package_versions = count, revision_count = versions.revision_count, index_hash = %h, "index converted");
    Ok(f)
}

/// Look up a revision's NAR hash in the index (for Layer 1 materialization).
pub fn find_revision(files: &IndexFiles, rev_prefix: &str) -> Result<Option<Revision>> {
    let revisions: Vec<Revision> = serde_json::from_slice(&files.revisions)?;
    Ok(revisions.into_iter().find(|r| r.rev.starts_with(rev_prefix)))
}
