//! A fragment is the immutable unit of storage: one named graph, serialized
//! as canonical N-Quads (sorted, deduplicated, no comments, no timestamps),
//! compressed with zstd at a fixed level. Same quads → same bytes.

use crate::error::{Error, Result};
use crate::iri;
use oxrdf::{GraphName, Literal, NamedNode, NamedNodeRef, NamedOrBlankNode, Quad, Term};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// zstd level. Fixed forever for a given store; changing it would change
/// bytes (not content) of every fragment.
pub const ZSTD_LEVEL: i32 = 9;

/// Where a fragment lives and what its graph IRI is. The variant carries the
/// identity of the unit; `path()` and `graph()` are total functions of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FragmentKind {
    /// One derivation. `drv/<store-hash>.nq.zst`, graph `nixid:drv/<hash>`.
    Drv(String),
    /// Outputs (and references, once built) of one derivation.
    Out(String),
    /// One source store path, by SRI NAR hash.
    Src(String),
    /// A flake pin: lock graph and, when used as a root, membership.
    Pin(String),
    /// Layer 1: a nixpkgs revision's evaluatesTo edges and metadata.
    Nixpkgs(String),
    /// A consumer commit: snapshot + time axis. `commit/<repo>/<sha>`.
    Commit { repo: String, sha: String },
    /// A NixOS generation with its options.
    Gen(String),
    /// An OCI image with its layers.
    Image(String),
    /// Layer 0: the nixpkgs-multiverse index, by its content hash.
    Index(String),
    /// Output of a rule run.
    Derived { ruleset: String, input: String },
    /// Provenance (timestamped RuleRun node) of a rule run.
    Run { ruleset: String, input: String },
    /// A large option value, by hash.
    Value(String),
    /// Full environment of a derivation (opt-in).
    Env(String),
    /// A Kubernetes cluster snapshot (live or desired), by content hash.
    K8sSnapshot { cluster: String, hash: String },
    /// Node inventory of a cluster, by content hash.
    K8sNodes { cluster: String, hash: String },
    /// Timestamped observation record for a snapshot.
    K8sObserved {
        cluster: String,
        hash: String,
        observed_at: String,
    },
    /// A kind owned by a downstream tool that stores its own fragments beside
    /// these (rdf2nix: `trace`). `ext/<kind>/<id>.nq.zst`; the graph IRI is
    /// the entity `nixid:<kind>/<id>`, so the fragment IS the entity, as with
    /// `Drv`. `id` must be a function of the content, like every other kind.
    Ext { kind: String, id: String },
}

impl FragmentKind {
    pub fn kind_name(&self) -> &'static str {
        match self {
            FragmentKind::Drv(_) => "drv",
            FragmentKind::Out(_) => "out",
            FragmentKind::Src(_) => "src",
            FragmentKind::Pin(_) => "pin",
            FragmentKind::Nixpkgs(_) => "nixpkgs",
            FragmentKind::Commit { .. } => "commit",
            FragmentKind::Gen(_) => "gen",
            FragmentKind::Image(_) => "image",
            FragmentKind::Index(_) => "index",
            FragmentKind::Derived { .. } => "derived",
            FragmentKind::Run { .. } => "run",
            FragmentKind::Value(_) => "values",
            FragmentKind::Env(_) => "env",
            FragmentKind::K8sSnapshot { .. } => "k8s-snapshot",
            FragmentKind::K8sNodes { .. } => "k8s-nodes",
            FragmentKind::K8sObserved { .. } => "k8s-observed",
            FragmentKind::Ext { .. } => "ext",
        }
    }

    /// Path relative to the store root.
    pub fn relative_path(&self) -> PathBuf {
        let seg = iri::encode_segment;
        let p = match self {
            FragmentKind::Drv(h) => format!("drv/{h}.nq.zst"),
            FragmentKind::Out(h) => format!("out/{h}.nq.zst"),
            FragmentKind::Src(n) => format!("src/{}.nq.zst", iri::nar_hash_segment(n)),
            FragmentKind::Pin(n) => format!("pin/{}.nq.zst", iri::nar_hash_segment(n)),
            FragmentKind::Nixpkgs(r) => format!("nixpkgs/{r}.nq.zst"),
            FragmentKind::Commit { repo, sha } => format!("commit/{}/{sha}.nq.zst", seg(repo)),
            FragmentKind::Gen(h) => format!("gen/{h}.nq.zst"),
            FragmentKind::Image(d) => format!("image/{d}.nq.zst"),
            FragmentKind::Index(h) => format!("index/nixpkgs-multiverse-{h}.nq.zst"),
            FragmentKind::Derived { ruleset, input } => format!("derived/{ruleset}/{input}.nq.zst"),
            FragmentKind::Run { ruleset, input } => format!("runs/{ruleset}/{input}.nq.zst"),
            FragmentKind::Value(h) => format!("values/{h}.nq.zst"),
            FragmentKind::Env(h) => format!("env/{h}.nq.zst"),
            FragmentKind::K8sSnapshot { cluster, hash } => {
                format!("k8s/{}/snapshot/{hash}.nq.zst", seg(cluster))
            }
            FragmentKind::K8sNodes { cluster, hash } => {
                format!("k8s/{}/nodes/{hash}.nq.zst", seg(cluster))
            }
            FragmentKind::K8sObserved {
                cluster,
                hash,
                observed_at,
            } => {
                format!(
                    "k8s/{}/observed/{hash}-{}.nq.zst",
                    seg(cluster),
                    seg(observed_at)
                )
            }
            FragmentKind::Ext { kind, id } => format!("ext/{}/{}.nq.zst", seg(kind), seg(id)),
        };
        PathBuf::from(p)
    }

    /// The named graph IRI: the fragment's identity. Where the fragment IS an
    /// entity (a derivation, a pin, an image) the graph IRI is that entity's
    /// IRI; otherwise it is under `frag:`.
    pub fn graph(&self) -> NamedNode {
        match self {
            FragmentKind::Drv(h) => iri::drv(h),
            FragmentKind::Out(h) => iri::fragment("out", h),
            FragmentKind::Src(n) => iri::src(n),
            FragmentKind::Pin(n) => iri::pin(n),
            FragmentKind::Nixpkgs(r) => iri::nixpkgs(r),
            FragmentKind::Commit { repo, sha } => iri::commit(repo, sha),
            FragmentKind::Gen(h) => iri::gen(h),
            FragmentKind::Image(d) => iri::image(d),
            FragmentKind::Index(h) => iri::fragment("index", &format!("nixpkgs-multiverse-{h}")),
            FragmentKind::Derived { ruleset, input } => iri::derived(ruleset, input),
            FragmentKind::Run { ruleset, input } => {
                iri::fragment("run", &format!("{ruleset}:{input}"))
            }
            FragmentKind::Value(h) => iri::fragment("values", h),
            FragmentKind::Env(h) => iri::fragment("env", h),
            FragmentKind::K8sSnapshot { cluster, hash } => iri::k8s_snapshot(cluster, hash),
            FragmentKind::K8sNodes { cluster, hash } => iri::k8s_nodes_fragment(cluster, hash),
            FragmentKind::K8sObserved {
                cluster,
                hash,
                observed_at,
            } => iri::fragment(
                "k8s-observed",
                &format!(
                    "{}/{hash}/{}",
                    iri::encode_segment(cluster),
                    iri::encode_segment(observed_at)
                ),
            ),
            FragmentKind::Ext { kind, id } => iri::id(&format!(
                "{}/{}",
                iri::encode_segment(kind),
                iri::encode_segment(id)
            )),
        }
    }
}

/// A set of triples destined for one named graph. Triples are kept as their
/// canonical N-Triples line so ordering and deduplication are by bytes.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub kind: FragmentKind,
    lines: BTreeSet<String>,
}

impl Fragment {
    pub fn new(kind: FragmentKind) -> Self {
        Fragment {
            kind,
            lines: BTreeSet::new(),
        }
    }

    pub fn graph(&self) -> NamedNode {
        self.kind.graph()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn add(
        &mut self,
        s: impl Into<NamedOrBlankNode>,
        p: impl Into<NamedNode>,
        o: impl Into<Term>,
    ) {
        let s: NamedOrBlankNode = s.into();
        let p: NamedNode = p.into();
        let o: Term = o.into();
        self.lines.insert(format!("{s} {p} {o}"));
    }

    pub fn add_type(&mut self, s: impl Into<NamedOrBlankNode>, class: NamedNode) {
        self.add(s, iri::rdf_type(), class);
    }

    pub fn add_str(&mut self, s: impl Into<NamedOrBlankNode>, p: NamedNode, v: &str) {
        self.add(s, p, Literal::new_simple_literal(v));
    }
    pub fn add_int(&mut self, s: impl Into<NamedOrBlankNode>, p: NamedNode, v: i64) {
        self.add(s, p, Literal::from(v));
    }
    pub fn add_bool(&mut self, s: impl Into<NamedOrBlankNode>, p: NamedNode, v: bool) {
        self.add(s, p, Literal::from(v));
    }
    pub fn add_typed(
        &mut self,
        s: impl Into<NamedOrBlankNode>,
        p: NamedNode,
        v: &str,
        dt: NamedNode,
    ) {
        self.add(s, p, Literal::new_typed_literal(v, dt));
    }

    /// Add an already-formed quad, ignoring its graph (the fragment's graph wins).
    pub fn add_quad(&mut self, q: &Quad) {
        self.lines
            .insert(format!("{} {} {}", q.subject, q.predicate, q.object));
    }

    /// Canonical N-Quads text: sorted, unique lines, each ending in the graph IRI.
    pub fn to_nquads(&self) -> String {
        let g = self.graph();
        let mut out = String::with_capacity(self.lines.len() * 128);
        for l in &self.lines {
            out.push_str(l);
            out.push(' ');
            out.push_str(&g.to_string());
            out.push_str(" .\n");
        }
        out
    }

    pub fn to_quads(&self) -> Result<Vec<Quad>> {
        parse_nquads(self.to_nquads().as_bytes())
    }

    /// Compressed canonical bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        compress(self.to_nquads().as_bytes())
    }

    pub fn write_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let bytes = self.to_bytes()?;
        // Write to a temp name then rename so a reader never sees a partial file.
        let tmp = path.with_extension("nq.zst.tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))?;
        Ok(())
    }

    /// Rebuild a fragment from quads that all share one graph.
    pub fn from_quads(kind: FragmentKind, quads: &[Quad]) -> Self {
        let mut f = Fragment::new(kind);
        for q in quads {
            f.add_quad(q);
        }
        f
    }

    /// Content hash of the canonical N-Quads text (not of the compressed bytes).
    pub fn content_hash(&self) -> String {
        crate::hash::sha256_hex(self.to_nquads().as_bytes())
    }
}

pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = zstd::stream::Encoder::new(Vec::new(), ZSTD_LEVEL)
        .map_err(|e| Error::Other(e.to_string()))?;
    enc.include_checksum(false)
        .map_err(|e| Error::Other(e.to_string()))?;
    enc.include_contentsize(true)
        .map_err(|e| Error::Other(e.to_string()))?;
    enc.write_all(data)
        .map_err(|e| Error::Other(e.to_string()))?;
    enc.finish().map_err(|e| Error::Other(e.to_string()))
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut dec = zstd::stream::Decoder::new(data).map_err(|e| Error::Other(e.to_string()))?;
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|e| Error::Other(e.to_string()))?;
    Ok(out)
}

pub fn read_nquads_zst(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let raw = decompress(&bytes)?;
    String::from_utf8(raw).map_err(|e| Error::Rdf(format!("{}: {e}", path.display())))
}

pub fn parse_nquads(bytes: &[u8]) -> Result<Vec<Quad>> {
    let mut quads = Vec::new();
    for q in oxttl::NQuadsParser::new().for_reader(bytes) {
        quads.push(q.map_err(|e| Error::Rdf(e.to_string()))?);
    }
    Ok(quads)
}

/// Read a fragment file back into quads.
pub fn read_quads(path: &Path) -> Result<Vec<Quad>> {
    let text = read_nquads_zst(path)?;
    parse_nquads(text.as_bytes())
}

/// The graph name of a quad as a NamedNode, if it has one.
pub fn graph_of(q: &Quad) -> Option<NamedNodeRef<'_>> {
    match &q.graph_name {
        GraphName::NamedNode(n) => Some(n.as_ref()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_and_roundtrip() {
        let mut f = Fragment::new(FragmentKind::Drv("abc".into()));
        f.add_str(iri::drv("abc"), crate::vocab::nix_terms::name(), "hello");
        f.add_type(iri::drv("abc"), crate::vocab::nix_terms::Derivation());
        f.add_str(iri::drv("abc"), crate::vocab::nix_terms::name(), "hello"); // dup
        let text = f.to_nquads();
        assert_eq!(text.lines().count(), 2);
        let bytes1 = f.to_bytes().unwrap();
        let bytes2 = f.to_bytes().unwrap();
        assert_eq!(bytes1, bytes2, "compression must be deterministic");
        let back = parse_nquads(decompress(&bytes1).unwrap().as_slice()).unwrap();
        assert_eq!(back.len(), 2);
        let g = Fragment::from_quads(FragmentKind::Drv("abc".into()), &back);
        assert_eq!(g.to_nquads(), text);
    }
}
