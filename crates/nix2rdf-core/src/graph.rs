//! Oxigraph: the query index over the canonical fragment files. Always
//! rebuildable; never load-bearing.

use crate::error::{Error, Result};
use crate::fragment::{self, FragmentKind};
use crate::iri;
use crate::store::Store;
use oxigraph::io::RdfFormat;
use oxigraph::sparql::results::{QueryResultsFormat, QueryResultsSerializer};
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store as OxStore;
use oxrdf::{GraphNameRef, NamedNode, Quad};
use std::path::Path;
use tracing::info;

pub struct Graph {
    inner: OxStore,
}

pub enum QueryOutput {
    Boolean(bool),
    Text(Vec<u8>),
}

impl Graph {
    pub fn open(store: &Store) -> Result<Graph> {
        let dir = store.oxigraph_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        Ok(Graph { inner: OxStore::open(&dir)? })
    }

    pub fn open_read_only(store: &Store) -> Result<Graph> {
        let dir = store.oxigraph_dir();
        Ok(Graph { inner: OxStore::open_read_only(&dir)? })
    }

    /// In-memory graph, for tests and one-off checks.
    pub fn in_memory() -> Result<Graph> {
        Ok(Graph { inner: OxStore::new()? })
    }

    pub fn inner(&self) -> &OxStore {
        &self.inner
    }

    /// Bulk-load one fragment file (non-transactional bulk loader, committed at the end).
    pub fn load_fragment(&self, path: &Path) -> Result<usize> {
        self.load_paths(std::slice::from_ref(&path.to_path_buf()))
    }

    /// Bulk-load many fragment files with one loader and one commit.
    pub fn load_paths(&self, paths: &[std::path::PathBuf]) -> Result<usize> {
        let mut total = 0;
        let mut loader = self.inner.bulk_loader();
        for p in paths {
            let text = fragment::read_nquads_zst(p)?;
            total += text.lines().count();
            loader.load_from_reader(RdfFormat::NQuads, text.as_bytes())?;
        }
        loader.commit()?;
        info!(target: "nix2rdf::graph", files = paths.len(), quads = total, "bulk load done");
        Ok(total)
    }

    pub fn load_all(&self, store: &Store) -> Result<usize> {
        let paths = store.all_fragment_paths()?;
        self.load_paths(&paths)
    }

    /// Load quads directly (used by the reasoner's derived output and tests).
    pub fn load_quads(&self, quads: &[Quad]) -> Result<()> {
        let mut text = String::new();
        for q in quads {
            text.push_str(&q.to_string());
            text.push_str(" .\n");
        }
        let mut loader = self.inner.bulk_loader();
        loader.load_from_reader(RdfFormat::NQuads, text.as_bytes())?;
        loader.commit()?;
        Ok(())
    }

    /// Delete the index and load every fragment from the files.
    pub fn rebuild(store: &Store) -> Result<Graph> {
        let dir = store.oxigraph_dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
        let g = Graph::open(store)?;
        g.load_all(store)?;
        g.optimize()?;
        Ok(g)
    }

    pub fn optimize(&self) -> Result<()> {
        self.inner.optimize()?;
        Ok(())
    }

    /// Prepend the standard prefixes so shipped queries stay short.
    pub fn with_prologue(q: &str) -> String {
        format!("{}\n{q}", iri::sparql_prologue())
    }

    /// Evaluate with the union of all named graphs as the default graph:
    /// every quad lives in a fragment's named graph, so a plain triple
    /// pattern must see all of them; `GRAPH ?g { }` still scopes when needed.
    fn evaluate(&self, sparql: &str) -> Result<QueryResults<'static>> {
        let q = Self::with_prologue(sparql);
        let mut prepared = SparqlEvaluator::new().parse_query(&q).map_err(|e| Error::Sparql(e.to_string()))?;
        prepared.dataset_mut().set_default_graph_as_union();
        Ok(prepared.on_store(&self.inner).execute()?)
    }

    pub fn query(&self, sparql: &str, format: QueryResultsFormat) -> Result<QueryOutput> {
        let results = self.evaluate(sparql)?;
        match results {
            QueryResults::Boolean(b) => Ok(QueryOutput::Boolean(b)),
            QueryResults::Solutions(sols) => {
                let vars = sols.variables().to_vec();
                let mut ser = QueryResultsSerializer::from_format(format)
                    .serialize_solutions_to_writer(Vec::new(), vars)
                    .map_err(|e| Error::Sparql(e.to_string()))?;
                for s in sols {
                    let s = s?;
                    ser.serialize(s.iter()).map_err(|e| Error::Sparql(e.to_string()))?;
                }
                let buf = ser.finish().map_err(|e| Error::Sparql(e.to_string()))?;
                Ok(QueryOutput::Text(buf))
            }
            QueryResults::Graph(triples) => {
                let mut buf = String::new();
                for t in triples {
                    let t = t?;
                    buf.push_str(&t.to_string());
                    buf.push_str(" .\n");
                }
                Ok(QueryOutput::Text(buf.into_bytes()))
            }
        }
    }

    pub fn ask(&self, sparql: &str) -> Result<bool> {
        match self.query(sparql, QueryResultsFormat::Json)? {
            QueryOutput::Boolean(b) => Ok(b),
            _ => Err(Error::Sparql("query is not an ASK".into())),
        }
    }

    /// Solutions as rows of (variable, term string) for programmatic use.
    pub fn select(&self, sparql: &str) -> Result<Vec<Vec<(String, String)>>> {
        let results = self.evaluate(sparql)?;
        let mut rows = Vec::new();
        if let QueryResults::Solutions(sols) = results {
            for s in sols {
                let s = s?;
                let mut row = Vec::new();
                for (v, t) in s.iter() {
                    row.push((v.as_str().to_string(), t.to_string()));
                }
                rows.push(row);
            }
        }
        Ok(rows)
    }

    pub fn graph_names(&self) -> Result<Vec<NamedNode>> {
        let mut out = Vec::new();
        for g in self.inner.named_graphs() {
            if let oxrdf::NamedOrBlankNode::NamedNode(n) = g? {
                out.push(n);
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn contains_graph(&self, g: &NamedNode) -> Result<bool> {
        Ok(self.inner.contains_named_graph(g.as_ref())?)
    }

    pub fn drop_graph(&self, g: &NamedNode) -> Result<()> {
        self.inner.remove_named_graph(g.as_ref())?;
        Ok(())
    }

    pub fn graph_quads(&self, g: &NamedNode) -> Result<Vec<Quad>> {
        let mut out = Vec::new();
        for q in self.inner.quads_for_pattern(None, None, None, Some(GraphNameRef::NamedNode(g.as_ref()))) {
            out.push(q?);
        }
        Ok(out)
    }

    pub fn len(&self) -> Result<usize> {
        Ok(self.inner.len()?)
    }
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.inner.is_empty()?)
    }

    /// Is a derived fragment for this run already present?
    pub fn has_derived(&self, ruleset: &str, input: &str) -> Result<bool> {
        self.contains_graph(&FragmentKind::Derived { ruleset: ruleset.into(), input: input.into() }.graph())
    }
}
