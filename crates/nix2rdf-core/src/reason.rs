//! Run semantics (see DESIGN.md §7):
//! 1. resolve packs in dependency order; hash → ruleset-hash
//! 2. select input graphs (fragments) → input-hash
//! 3. export to N-Quads, run Nemo in-process, collect `new/3`
//! 4. write a derived fragment `derived:<ruleset>:<input>` and a RuleRun
//!    provenance fragment; load both into Oxigraph
//! 5. same inputs + same rules → same graph name → idempotent

use crate::error::{Error, Result};
use crate::fragment::{self, Fragment, FragmentKind};
use crate::graph::Graph;
use crate::hash;
use crate::iri;
use crate::nemo_engine::{self, DerivedTriple, RdfTerm};
use crate::packs::{self, Pack};
use crate::store::Store;
use crate::vocab::nix_terms as t;
use crate::vocab::xsd_date_time;
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Quad, Term};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// One selected input: a graph IRI, the path of its fragment, its text.
#[derive(Debug, Clone)]
pub struct InputGraph {
    pub graph: NamedNode,
    pub path: PathBuf,
    pub nquads: String,
}

/// Which fragments participate in a run.
#[derive(Debug, Clone, Default)]
pub struct InputSelection {
    /// Explicit fragment paths (relative to the store or absolute).
    pub paths: Vec<PathBuf>,
    /// Snapshot IRIs (pins, generations, images, commits, k8s snapshots)
    /// whose closure fragments are pulled in when `closure` is set.
    pub roots: Vec<NamedNode>,
    pub closure: bool,
    /// Every snapshot fragment in the store (pin, gen, image, commit, k8s).
    pub all_snapshots: bool,
    /// Everything in the store.
    pub everything: bool,
}

fn read_input(store: &Store, path: &Path) -> Result<InputGraph> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        store.root().join(path)
    };
    let nquads = fragment::read_nquads_zst(&abs)?;
    // The graph IRI is the last token before " ." of the first line.
    let graph = nquads
        .lines()
        .next()
        .and_then(|l| l.rsplit(' ').nth(1))
        .map(|g| g.trim_matches(|c| c == '<' || c == '>').to_string())
        .ok_or_else(|| Error::Reason(format!("{}: empty fragment", abs.display())))?;
    Ok(InputGraph {
        graph: NamedNode::new(graph)?,
        path: abs,
        nquads,
    })
}

/// Object IRIs of a predicate in a fragment's text (cheap scan; no parse).
fn objects_of(nquads: &str, predicate: &NamedNode) -> BTreeSet<String> {
    let p = predicate.to_string();
    let mut out = BTreeSet::new();
    for line in nquads.lines() {
        let mut it = line.splitn(4, ' ');
        let (_s, pp, o) = (it.next(), it.next(), it.next());
        if pp == Some(p.as_str()) {
            if let Some(o) = o {
                if o.starts_with('<') {
                    out.insert(o.trim_matches(|c| c == '<' || c == '>').to_string());
                }
            }
        }
    }
    out
}

/// Map an entity IRI back to the fragment that defines it, if the mapping is
/// a function of the IRI (drv, src, pin, gen, image, nixpkgs, commit, k8s).
pub fn fragment_for_iri(iri_str: &str) -> Option<FragmentKind> {
    let rest = iri_str.strip_prefix(iri::ID_BASE)?;
    let (kind, id) = rest.split_once('/')?;

    Some(match kind {
        "drv" => FragmentKind::Drv(id.to_string()),
        "src" => FragmentKind::Src(decode_nar(id)),
        "pin" if !id.contains('/') => FragmentKind::Pin(decode_nar(id)),
        "gen" if !id.contains('/') => FragmentKind::Gen(id.to_string()),
        "nixpkgs" if !id.contains('/') => FragmentKind::Nixpkgs(id.to_string()),
        "oci" => FragmentKind::Image(id.strip_prefix("sha256/")?.to_string()),
        "git" => {
            let (repo, sha) = id.rsplit_once('/')?;
            FragmentKind::Commit {
                repo: percent_decode(repo),
                sha: sha.to_string(),
            }
        }
        "k8s" => {
            let (cluster, rest) = id.split_once('/')?;
            let (k, h) = rest.split_once('/')?;
            match k {
                "snapshot" => FragmentKind::K8sSnapshot {
                    cluster: percent_decode(cluster),
                    hash: h.to_string(),
                },
                "nodes" => FragmentKind::K8sNodes {
                    cluster: percent_decode(cluster),
                    hash: h.to_string(),
                },
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Inverse of `iri::nar_hash_segment` (base64url → SRI base64).
fn decode_nar(seg: &str) -> String {
    use base64::Engine;
    if let Some((algo, b)) = seg.split_once('-') {
        if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b) {
            return format!(
                "{algo}-{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
        }
    }
    seg.to_string()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Resolve a selection to concrete input graphs. Closure following pulls in:
/// closureContains/hasRoot → drv fragments → inputSrc → src fragments, plus
/// out/ fragments when present; k8s snapshots → runsImage images and
/// hostGeneration generations (and their closures); commits → snapshots.
pub fn select_inputs(store: &Store, sel: &InputSelection) -> Result<Vec<InputGraph>> {
    let mut by_graph: BTreeMap<String, InputGraph> = BTreeMap::new();
    let mut queue: Vec<PathBuf> = Vec::new();

    if sel.everything {
        queue.extend(store.all_fragment_paths()?);
    } else {
        queue.extend(sel.paths.iter().cloned());
        if sel.all_snapshots {
            for sub in ["pin", "gen", "image", "commit", "nixpkgs", "k8s"] {
                queue.extend(store.fragment_paths_under(sub)?);
            }
        }
        for r in &sel.roots {
            match fragment_for_iri(r.as_str()) {
                Some(k) => queue.push(store.path_of(&k)),
                None => warn!(target: "nix2rdf::reason", iri = %r, "cannot map IRI to a fragment"),
            }
        }
    }

    let follow = sel.closure && !sel.everything;
    let mut seen_paths: BTreeSet<PathBuf> = BTreeSet::new();
    while let Some(p) = queue.pop() {
        let abs = if p.is_absolute() {
            p.clone()
        } else {
            store.root().join(&p)
        };
        if !seen_paths.insert(abs.clone()) {
            continue;
        }
        if !abs.exists() {
            warn!(target: "nix2rdf::reason", path = %abs.display(), "fragment not found; skipped");
            continue;
        }
        let ig = read_input(store, &abs)?;
        if follow {
            let mut refs: Vec<FragmentKind> = Vec::new();
            for pred in [
                t::closureContains(),
                t::hasRoot(),
                t::snapshot(),
                t::inputSrc(),
                t::imageDerivation(),
            ] {
                for o in objects_of(&ig.nquads, &pred) {
                    if let Some(k) = fragment_for_iri(&o) {
                        if let FragmentKind::Drv(h) = &k {
                            refs.push(FragmentKind::Out(h.clone()));
                        }
                        refs.push(k);
                    }
                }
            }
            for pred in [
                crate::vocab::k8s_terms::runsImage(),
                crate::vocab::k8s_terms::hostGeneration(),
            ] {
                for o in objects_of(&ig.nquads, &pred) {
                    if let Some(k) = fragment_for_iri(&o) {
                        refs.push(k);
                    }
                }
            }
            for k in refs {
                let path = store.path_of(&k);
                if path.exists() && !seen_paths.contains(&path) {
                    queue.push(path);
                }
            }
        }
        by_graph.insert(ig.graph.as_str().to_string(), ig);
    }
    Ok(by_graph.into_values().collect())
}

/// input-hash: over sorted (graph IRI, content hash of its N-Quads).
pub fn input_hash(inputs: &[InputGraph]) -> String {
    let mut items: Vec<(String, String)> = inputs
        .iter()
        .map(|i| {
            (
                i.graph.as_str().to_string(),
                hash::sha256_hex(i.nquads.as_bytes()),
            )
        })
        .collect();
    items.sort();
    hash::sha256_hex_items(items.iter().flat_map(|(g, h)| [g.clone(), h.clone()]))
}

/// The pack vocabularies as N-Quads in `pack:<hash>` graphs, so RDFS/OWL
/// declarations are facts the `rdfs`/`owl-rl-subset` rules can use.
pub fn vocab_quads(packs: &[Pack]) -> Result<String> {
    let mut out = String::new();
    for p in packs {
        let Some(ttl) = &p.vocab_ttl else { continue };
        let g = iri::pack(&p.content_hash);
        for tr in oxttl::TurtleParser::new().for_reader(ttl.as_bytes()) {
            let tr = tr.map_err(|e| Error::Pack(format!("{} vocab.ttl: {e}", p.manifest.name)))?;
            let q = Quad::new(tr.subject, tr.predicate, tr.object, g.clone());
            out.push_str(&q.to_string());
            out.push_str(" .\n");
        }
    }
    Ok(out)
}

/// Assemble the full Nemo program.
pub fn build_program(packs: &[Pack], input_path: &Path) -> Result<String> {
    let (prefixes, body) = packs::collect_prefixes(packs)?;
    let mut prog = String::new();
    prog.push_str("%% nix2rdf generated program. Convention: t(?s,?p,?o) is the monotone working\n%% triple set (asserted + derived); q(?g,?s,?p,?o) carries the graph;\n%% out(?s,?p,?o) is the final layer (may negate/aggregate over t, is never read by t).\n");
    let mut all_prefixes = prefixes.clone();
    for (p, u) in iri::PREFIXES {
        all_prefixes
            .entry(p.to_string())
            .or_insert_with(|| u.to_string());
    }
    for (p, u) in &all_prefixes {
        prog.push_str(&format!("@prefix {p}: <{u}> .\n"));
    }
    // The import goes into `raw`; `q` is derived from it through a join with a
    // second, lower-arity projection. This keeps Nemo's FilterImports
    // transformation from internalizing `q` into the RDF import (its
    // projection handling reports "multiple arities"), and makes `q` an
    // ordinary predicate that pack rules may read alone.
    prog.push_str(&format!(
        "@import raw :- rdf{{resource = \"{}\"}} .\n",
        input_path.display()
    ));
    prog.push_str("rawGraph(?g) :- raw(?g, ?s, ?p, ?o) .\n");
    prog.push_str("q(?g, ?s, ?p, ?o) :- raw(?g, ?s, ?p, ?o), rawGraph(?g) .\n");
    prog.push_str("asserted(?s, ?p, ?o) :- q(?g, ?s, ?p, ?o) .\n");
    prog.push_str("t(?s, ?p, ?o) :- asserted(?s, ?p, ?o) .\n\n");
    prog.push_str(&body);
    prog.push_str("\n%% ---- epilogue ----\nnew(?s, ?p, ?o) :- t(?s, ?p, ?o), ~asserted(?s, ?p, ?o) .\nnew(?s, ?p, ?o) :- out(?s, ?p, ?o) .\n@export new :- csv {} .\n");
    Ok(prog)
}

/// Give every existential null a deterministic IRI: the hash of the sorted
/// set of triples it appears in, with all nulls replaced by a placeholder.
/// Two nulls with identical neighbourhoods collapse into one node (they are
/// indistinguishable anyway). See DESIGN.md.
pub fn canonicalize_nulls(
    triples: &[DerivedTriple],
    derived_graph: &NamedNode,
) -> Vec<(Term, NamedNode, Term)> {
    fn key(t: &RdfTerm) -> String {
        match t {
            RdfTerm::Null(_) => "_".into(),
            RdfTerm::Iri(i) => format!("<{i}>"),
            RdfTerm::Literal(l, d) => format!("\"{l}\"^^<{d}>"),
            RdfTerm::LangString(l, g) => format!("\"{l}\"@{g}"),
        }
    }
    let mut sigs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for tr in triples {
        let line = format!("{} {} {}", key(&tr.s), key(&tr.p), key(&tr.o));
        for term in [&tr.s, &tr.o] {
            if let RdfTerm::Null(id) = term {
                let pos = if std::ptr::eq(term, &tr.s) { "S" } else { "O" };
                sigs.entry(id.clone())
                    .or_default()
                    .insert(format!("{pos} {line}"));
            }
        }
    }
    let null_iri: BTreeMap<String, NamedNode> = sigs
        .into_iter()
        .map(|(id, sig)| {
            let h = hash::sha256_hex_items(sig.iter());
            (
                id,
                NamedNode::new_unchecked(format!(
                    "{}/n/{}",
                    derived_graph.as_str(),
                    hash::short(&h)
                )),
            )
        })
        .collect();
    let conv = |t: &RdfTerm| -> Term {
        match t {
            RdfTerm::Iri(i) => Term::NamedNode(NamedNode::new_unchecked(i.clone())),
            RdfTerm::Literal(l, d) => Term::Literal(Literal::new_typed_literal(
                l.clone(),
                NamedNode::new_unchecked(d.clone()),
            )),
            RdfTerm::LangString(l, g) => Term::Literal(
                Literal::new_language_tagged_literal_unchecked(l.clone(), g.clone()),
            ),
            RdfTerm::Null(id) => Term::NamedNode(null_iri[id].clone()),
        }
    };
    let mut out: Vec<(Term, NamedNode, Term)> = Vec::with_capacity(triples.len());
    for tr in triples {
        let RdfTerm::Iri(p) = &tr.p else { continue };
        out.push((
            conv(&tr.s),
            NamedNode::new_unchecked(p.clone()),
            conv(&tr.o),
        ));
    }
    out.sort_by(|a, b| {
        a.0.to_string()
            .cmp(&b.0.to_string())
            .then(a.1.cmp(&b.1))
            .then(a.2.to_string().cmp(&b.2.to_string()))
    });
    out.dedup();
    out
}

#[derive(Debug)]
pub struct RunResult {
    pub ruleset_hash: String,
    pub input_hash: String,
    pub derived: Fragment,
    pub run: Fragment,
    pub inputs: usize,
    pub derived_facts: usize,
    pub already_existed: bool,
}

/// Run the packs over the inputs. Pure with respect to the derived fragment;
/// only the RuleRun fragment carries the wall-clock time.
pub fn run(
    store: &Store,
    pack_list: &[Pack],
    inputs: &[InputGraph],
    force: bool,
) -> Result<RunResult> {
    let ruleset = packs::ruleset_hash(pack_list);
    let ih = input_hash(inputs);
    let kind = FragmentKind::Derived {
        ruleset: ruleset.clone(),
        input: ih.clone(),
    };
    let derived_graph = kind.graph();
    info!(target: "nix2rdf::reason", ruleset_hash = %ruleset, input_hash = %ih, inputs = inputs.len(), packs = ?pack_list.iter().map(|p| p.name()).collect::<Vec<_>>(), "rule run");

    if store.exists(&kind) && !force {
        let quads = fragment::read_quads(&store.path_of(&kind))?;
        let derived = Fragment::from_quads(kind.clone(), &quads);
        let run = run_fragment(&ruleset, &ih, pack_list, inputs, derived.len());
        return Ok(RunResult {
            ruleset_hash: ruleset,
            input_hash: ih,
            derived,
            run,
            inputs: inputs.len(),
            derived_facts: 0,
            already_existed: true,
        });
    }

    let tmp = tempfile::Builder::new()
        .prefix("nix2rdf-reason-")
        .tempdir()
        .map_err(|e| Error::Reason(e.to_string()))?;
    let input_path = tmp.path().join("input.nq");
    {
        let mut f = std::fs::File::create(&input_path).map_err(|e| Error::io(&input_path, e))?;
        for i in inputs {
            f.write_all(i.nquads.as_bytes())
                .map_err(|e| Error::io(&input_path, e))?;
        }
        f.write_all(vocab_quads(pack_list)?.as_bytes())
            .map_err(|e| Error::io(&input_path, e))?;
    }
    let program = build_program(pack_list, &input_path)?;
    let program_path = tmp.path().join("program.rls");
    std::fs::write(&program_path, &program).map_err(|e| Error::io(&program_path, e))?;
    // Debug aid: NIX2RDF_KEEP_PROGRAM=<dir> keeps the program and input.
    if let Ok(keep) = std::env::var("NIX2RDF_KEEP_PROGRAM") {
        let keep = PathBuf::from(keep);
        let _ = std::fs::create_dir_all(&keep);
        let _ = std::fs::copy(&program_path, keep.join("program.rls"));
        let _ = std::fs::copy(&input_path, keep.join("input.nq"));
    }

    let out = nemo_engine::run_program(&program)?;
    info!(target: "nix2rdf::reason", new_triples = out.triples.len(), derived_facts = out.derived_facts, "nemo finished");

    let mut derived = Fragment::new(kind);
    for (s, p, o) in canonicalize_nulls(&out.triples, &derived_graph) {
        let subj: NamedOrBlankNode = match s {
            Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
            _ => continue, // literal subjects cannot be RDF
        };
        derived.add(subj, p, o);
    }
    for p in pack_list {
        packs::copy_to_store(store, p)?;
    }
    let run = run_fragment(&ruleset, &ih, pack_list, inputs, derived.len());
    Ok(RunResult {
        ruleset_hash: ruleset,
        input_hash: ih,
        derived,
        run,
        inputs: inputs.len(),
        derived_facts: out.derived_facts,
        already_existed: false,
    })
}

fn run_fragment(
    ruleset: &str,
    input: &str,
    pack_list: &[Pack],
    inputs: &[InputGraph],
    quads: usize,
) -> Fragment {
    let derived_graph = FragmentKind::Derived {
        ruleset: ruleset.into(),
        input: input.into(),
    }
    .graph();
    let mut f = Fragment::new(FragmentKind::Run {
        ruleset: ruleset.into(),
        input: input.into(),
    });
    f.add_type(derived_graph.clone(), t::RuleRun());
    f.add_type(derived_graph.clone(), t::Fragment());
    f.add_str(derived_graph.clone(), t::fragmentKind(), "derived");
    f.add_str(derived_graph.clone(), t::rulesetHash(), ruleset);
    f.add_str(derived_graph.clone(), t::inputHash(), input);
    f.add_typed(
        derived_graph.clone(),
        t::ranAt(),
        &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        xsd_date_time(),
    );
    f.add_int(derived_graph.clone(), t::derivedQuadCount(), quads as i64);
    f.add_str(
        derived_graph.clone(),
        t::nemoVersion(),
        nemo_engine::NEMO_VERSION,
    );
    f.add(derived_graph.clone(), t::derivedBy(), derived_graph.clone());
    for p in pack_list {
        let pn = iri::pack(&p.content_hash);
        f.add(derived_graph.clone(), t::usesPack(), pn.clone());
        f.add_type(pn.clone(), t::OntologyPack());
        f.add_str(pn.clone(), t::packName(), &p.manifest.name);
        f.add_str(pn.clone(), t::packVersion(), &p.manifest.version);
        for d in &p.manifest.depends_on {
            if let Some(dp) = pack_list.iter().find(|x| &x.manifest.name == d) {
                f.add(pn.clone(), t::dependsOnPack(), iri::pack(&dp.content_hash));
            }
        }
    }
    for i in inputs {
        f.add(derived_graph.clone(), t::inputGraph(), i.graph.clone());
    }
    f
}

/// Write the derived + run fragments and load them into the index.
pub fn persist(store: &Store, graph: Option<&Graph>, r: &RunResult) -> Result<()> {
    let d = store.write(&r.derived)?;
    let run = store.overwrite(&r.run)?;
    if let Some(g) = graph {
        if !r.already_existed || !g.has_derived(&r.ruleset_hash, &r.input_hash)? {
            g.load_paths(&[d.path.clone(), run.path.clone()])?;
        }
    }
    Ok(())
}

/// Drop derived graphs whose ruleset hash is not in `keep` (for `gc-derived`).
pub fn gc_derived(store: &Store, graph: Option<&Graph>, keep_rulesets: &[String]) -> Result<usize> {
    let mut removed = 0;
    for sub in ["derived", "runs"] {
        for p in store.fragment_paths_under(sub)? {
            let rel = store.relative(&p);
            let ruleset = rel
                .components()
                .nth(1)
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .unwrap_or_default();
            if keep_rulesets.iter().any(|k| k == &ruleset) {
                continue;
            }
            if sub == "derived" {
                if let Some(g) = graph {
                    let input = p
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .trim_end_matches(".nq.zst")
                        .to_string();
                    let _ = g.drop_graph(
                        &FragmentKind::Derived {
                            ruleset: ruleset.clone(),
                            input,
                        }
                        .graph(),
                    );
                }
            }
            std::fs::remove_file(&p).map_err(|e| Error::io(&p, e))?;
            removed += 1;
        }
    }
    Ok(removed)
}
