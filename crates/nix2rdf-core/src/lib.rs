//! nix2rdf-core: turn Nix artifacts into a content-addressed RDF dataset,
//! store it in Oxigraph, and reason over it with Nemo rule packs.
//!
//! Module map (see DESIGN.md):
//! - [`iri`]      the IRI scheme and namespaces (IRI.md)
//! - [`vocab`]    constants for every `nix:` / `k8s:` term used in Rust
//! - [`fragment`] the immutable unit: one named graph, canonical N-Quads, zstd
//! - [`store`]    on-disk layout of fragments (files are canonical)
//! - [`nix`]      the Nix CLI and recorded-fixture back-ends
//! - [`extract`]  the core extractor and the thin front-ends
//! - [`graph`]    Oxigraph: load, rebuild, query, check
//! - [`packs`]    rule-pack discovery, dependency order, content hashing
//! - [`nemo_engine`] THE ONLY module that touches the `nemo` crate
//! - [`reason`]   run semantics: select graphs → Nemo → derived fragment
//! - [`publish`]  fragment publishing and fetching
//! - [`ontology`] Turtle validation and documentation generation
//! - [`k8s`]      cluster snapshot front-end

pub mod error;
pub mod hash;
pub mod iri;
pub mod vocab;
pub mod fragment;
pub mod store;
pub mod nix;
pub mod extract;
pub mod graph;
pub mod packs;
pub mod nemo_engine;
pub mod reason;
pub mod publish;
pub mod ontology;
pub mod k8s;

pub use error::{Error, Result};
pub use fragment::{Fragment, FragmentKind};
pub use store::Store;
