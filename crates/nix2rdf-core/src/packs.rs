//! Rule packs: discovery, dependency resolution, content hashing, and the
//! concatenation Nemo runs. Packs are authored; the tool never interprets
//! their semantics.
//!
//! ```text
//! packs/<name>/
//!   pack.toml        # name, version, depends_on = ["core", ...]
//!   vocab.ttl        # vocabulary (RDFS/OWL as notation) — loaded as input facts
//!   semantics.rls    # rules giving the vocab meaning
//!   mapping.rls      # rules from nix: core terms → this pack's terms
//!   policy/*.rls     # optional: rules deriving nix:Violation nodes
//!   tests/<case>/    # input.nq + expected.nq (+ forbidden.nq)
//! ```
//!
//! Rule convention (see PACKS.md): every rule reads and writes the ternary
//! predicate `t(?s, ?p, ?o)`; quad-scoped rules read `q(?g, ?s, ?p, ?o)`.

use crate::error::{Error, Result};
use crate::hash;
use crate::store::Store;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tracing::debug;

#[derive(Debug, Clone, Deserialize)]
pub struct PackManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct Pack {
    pub manifest: PackManifest,
    pub dir: PathBuf,
    pub vocab_ttl: Option<String>,
    /// (relative file name, text) in load order.
    pub rule_files: Vec<(String, String)>,
    pub content_hash: String,
}

fn read(p: &Path) -> Result<String> {
    std::fs::read_to_string(p).map_err(|e| Error::io(p, e))
}

impl Pack {
    pub fn load(dir: &Path) -> Result<Pack> {
        let manifest: PackManifest = toml::from_str(&read(&dir.join("pack.toml"))?)
            .map_err(|e| Error::Pack(format!("{}: {e}", dir.display())))?;
        let vocab_p = dir.join("vocab.ttl");
        let vocab_ttl = if vocab_p.exists() {
            Some(read(&vocab_p)?)
        } else {
            None
        };
        let mut rule_files = Vec::new();
        for name in ["semantics.rls", "mapping.rls"] {
            let p = dir.join(name);
            if p.exists() {
                rule_files.push((name.to_string(), read(&p)?));
            }
        }
        let policy = dir.join("policy");
        if policy.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&policy)
                .map_err(|e| Error::io(&policy, e))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map(|x| x == "rls").unwrap_or(false))
                .collect();
            entries.sort();
            for p in entries {
                let rel = format!("policy/{}", p.file_name().unwrap().to_string_lossy());
                rule_files.push((rel, read(&p)?));
            }
        }
        // Content hash over every file except tests/, by relative path.
        let mut items: Vec<(String, Vec<u8>)> = Vec::new();
        for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
            let entry = entry.map_err(|e| Error::Pack(e.to_string()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .to_string();
            if rel.starts_with("tests/") {
                continue;
            }
            items.push((
                rel,
                std::fs::read(entry.path()).map_err(|e| Error::io(entry.path(), e))?,
            ));
        }
        items.sort();
        let content_hash = hash::sha256_hex_items(
            items
                .iter()
                .flat_map(|(n, b)| [n.as_bytes().to_vec(), b.clone()]),
        );
        Ok(Pack {
            manifest,
            dir: dir.to_path_buf(),
            vocab_ttl,
            rule_files,
            content_hash,
        })
    }

    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// All rule text of the pack, concatenated in load order.
    pub fn rules_text(&self) -> String {
        self.rule_files
            .iter()
            .map(|(n, t)| format!("%% ---- {}/{} ----\n{t}\n", self.manifest.name, n))
            .collect()
    }
}

/// Where packs live: `NIX2RDF_PACKS` (colon-separated), else `./packs`, else
/// `<exe>/../share/nix2rdf/packs` (the flake install layout).
pub fn default_pack_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(env) = std::env::var("NIX2RDF_PACKS") {
        dirs.extend(env.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    dirs.push(PathBuf::from("packs"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(prefix) = exe.parent().and_then(|p| p.parent()) {
            dirs.push(prefix.join("share/nix2rdf/packs"));
        }
    }
    if let Some(p) = option_env!("CARGO_MANIFEST_DIR") {
        dirs.push(PathBuf::from(p).join("../../packs"));
    }
    dirs
}

/// Discover packs: each dir is either a pack itself or a directory of packs.
pub fn discover(dirs: &[PathBuf]) -> Result<BTreeMap<String, Pack>> {
    let mut packs = BTreeMap::new();
    for d in dirs {
        if !d.is_dir() {
            continue;
        }
        if d.join("pack.toml").exists() {
            let p = Pack::load(d)?;
            packs.entry(p.manifest.name.clone()).or_insert(p);
            continue;
        }
        let mut subs: Vec<PathBuf> = std::fs::read_dir(d)
            .map_err(|e| Error::io(d, e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        subs.sort();
        for s in subs {
            if s.join("pack.toml").exists() {
                let p = Pack::load(&s)?;
                debug!(target: "nix2rdf::packs", pack = %p.manifest.name, hash = %p.content_hash, dir = %s.display(), "pack found");
                packs.entry(p.manifest.name.clone()).or_insert(p);
            }
        }
    }
    Ok(packs)
}

/// Resolve the selected packs and their dependencies into load order
/// (dependencies first; ties broken by name for determinism).
pub fn resolve(all: &BTreeMap<String, Pack>, selected: &[String]) -> Result<Vec<Pack>> {
    let mut order: Vec<Pack> = Vec::new();
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut visiting: BTreeSet<String> = BTreeSet::new();
    fn visit(
        name: &str,
        all: &BTreeMap<String, Pack>,
        done: &mut BTreeSet<String>,
        visiting: &mut BTreeSet<String>,
        order: &mut Vec<Pack>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            return Err(Error::Pack(format!("dependency cycle at pack '{name}'")));
        }
        let p = all
            .get(name)
            .ok_or_else(|| Error::Pack(format!("unknown pack '{name}'")))?;
        let mut deps = p.manifest.depends_on.clone();
        deps.sort();
        for d in deps {
            visit(&d, all, done, visiting, order)?;
        }
        visiting.remove(name);
        done.insert(name.to_string());
        order.push(p.clone());
        Ok(())
    }
    let mut sel = selected.to_vec();
    sel.sort();
    for s in sel {
        visit(&s, all, &mut done, &mut visiting, &mut order)?;
    }
    Ok(order)
}

/// The ruleset hash: sha256 over (name, version, content hash) of every pack
/// in load order plus the concatenated rule text. Same packs → same hash.
pub fn ruleset_hash(packs: &[Pack]) -> String {
    let mut items: Vec<Vec<u8>> = Vec::new();
    for p in packs {
        items.push(
            format!(
                "{}@{}#{}",
                p.manifest.name, p.manifest.version, p.content_hash
            )
            .into_bytes(),
        );
        items.push(p.rules_text().into_bytes());
    }
    hash::sha256_hex_items(items)
}

/// Split rule text into (prefix declarations, body). Prefixes are collected
/// across packs and emitted once; conflicting definitions are an error.
pub fn collect_prefixes(packs: &[Pack]) -> Result<(BTreeMap<String, String>, String)> {
    let mut prefixes: BTreeMap<String, String> = BTreeMap::new();
    let mut body = String::new();
    for p in packs {
        for (name, text) in &p.rule_files {
            body.push_str(&format!("%% ---- {}/{} ----\n", p.manifest.name, name));
            for line in text.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("@prefix") {
                    let rest = rest.trim().trim_end_matches('.').trim();
                    let (pfx, iri) = rest.split_once(':').ok_or_else(|| {
                        Error::Pack(format!("{}: bad @prefix line: {line}", p.manifest.name))
                    })?;
                    let iri = iri
                        .trim()
                        .trim_matches(|c| c == '<' || c == '>')
                        .to_string();
                    let pfx = pfx.trim().to_string();
                    if let Some(prev) = prefixes.get(&pfx) {
                        if prev != &iri {
                            return Err(Error::Pack(format!(
                                "prefix '{pfx}' bound to both <{prev}> and <{iri}> (pack {})",
                                p.manifest.name
                            )));
                        }
                    } else {
                        prefixes.insert(pfx, iri);
                    }
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
        }
    }
    Ok((prefixes, body))
}

/// Copy a pack's files into `store/packs/<content-hash>/` for reproducibility.
pub fn copy_to_store(store: &Store, pack: &Pack) -> Result<PathBuf> {
    let dest = store.packs_dir().join(&pack.content_hash);
    if dest.exists() {
        return Ok(dest);
    }
    for entry in walkdir::WalkDir::new(&pack.dir) {
        let entry = entry.map_err(|e| Error::Pack(e.to_string()))?;
        let rel = entry.path().strip_prefix(&pack.dir).unwrap();
        if rel.starts_with("tests") {
            continue;
        }
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| Error::io(&target, e))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::copy(entry.path(), &target).map_err(|e| Error::io(&target, e))?;
        }
    }
    Ok(dest)
}

/// Test cases of a pack: `tests/<case>/{input.nq,expected.nq,forbidden.nq}`.
#[derive(Debug, Clone)]
pub struct PackTest {
    pub name: String,
    pub dir: PathBuf,
    pub input: String,
    pub expected: String,
    pub forbidden: Option<String>,
}

pub fn tests_of(pack: &Pack) -> Result<Vec<PackTest>> {
    let tdir = pack.dir.join("tests");
    if !tdir.is_dir() {
        return Ok(vec![]);
    }
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&tdir)
        .map_err(|e| Error::io(&tdir, e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    let mut out = Vec::new();
    for c in cases {
        let input = c.join("input.nq");
        let expected = c.join("expected.nq");
        if !input.exists() || !expected.exists() {
            continue;
        }
        let forbidden = c.join("forbidden.nq");
        out.push(PackTest {
            name: c.file_name().unwrap().to_string_lossy().to_string(),
            dir: c.clone(),
            input: read(&input)?,
            expected: read(&expected)?,
            forbidden: if forbidden.exists() {
                Some(read(&forbidden)?)
            } else {
                None
            },
        });
    }
    Ok(out)
}
