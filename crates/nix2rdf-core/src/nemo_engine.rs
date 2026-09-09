//! THE ONLY module that uses the `nemo` / `nemo_physical` crates.
//!
//! Pinned to upstream commit e578c283c996bb1b029e5642ce96f814d1b0bfae
//! (vendored under vendor/nemo, see vendor/PATCHES.md). Nemo's API is
//! unstable; keep every use of it inside this file so an upgrade touches
//! one place. The rest of the crate sees only [`run_program`] and
//! [`DerivedTriple`].
//!
//! Contract with the rest of the crate:
//! - The caller writes an N-Quads file and a full Nemo program that
//!   `@import`s it (see `reason.rs` for the prelude/epilogue).
//! - The program must define the ternary predicate `new(?s, ?p, ?o)`;
//!   its rows are returned as RDF terms.
//! - Existential nulls come back as [`RdfTerm::Null`] labelled by Nemo's
//!   own canonical string; the caller assigns deterministic IRIs.

use crate::error::{Error, Result};
use nemo::api::{load_string, reason, Engine};
use nemo::rule_model::components::tag::Tag;
use nemo_physical::datavalues::{AnyDataValue, DataValue, ValueDomain};

pub const NEMO_VERSION: &str = "0.10.2-dev+e578c283";

/// An RDF term as produced by the reasoner.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RdfTerm {
    Iri(String),
    /// (lexical form, datatype IRI) — plain strings carry xsd:string.
    Literal(String, String),
    LangString(String, String),
    /// A Nemo existential null, by its engine-internal label.
    Null(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DerivedTriple {
    pub s: RdfTerm,
    pub p: RdfTerm,
    pub o: RdfTerm,
}

fn to_term(v: &AnyDataValue) -> RdfTerm {
    match v.value_domain() {
        ValueDomain::Iri => RdfTerm::Iri(v.to_iri_unchecked()),
        ValueDomain::PlainString => RdfTerm::Literal(v.to_plain_string_unchecked(), "http://www.w3.org/2001/XMLSchema#string".into()),
        ValueDomain::LanguageTaggedString => {
            let (s, l) = v.to_language_tagged_string_unchecked();
            RdfTerm::LangString(s, l)
        }
        ValueDomain::Null => RdfTerm::Null(v.canonical_string()),
        _ => {
            // Nemo reports integers with the narrowest XSD type that fits
            // (xsd:int, xsd:long, ...). RDF data uses xsd:integer; normalize so
            // derived literals compare equal to asserted ones.
            let dt = v.datatype_iri();
            let dt = match dt.rsplit('#').next().unwrap_or("") {
                "int" | "long" | "short" | "byte" | "unsignedInt" | "unsignedLong" | "unsignedShort" | "unsignedByte"
                | "nonNegativeInteger" | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" => {
                    "http://www.w3.org/2001/XMLSchema#integer".to_string()
                }
                _ => dt,
            };
            RdfTerm::Literal(v.lexical_value(), dt)
        }
    }
}

/// Parse and run a complete Nemo program; return the rows of `new/3`.
pub fn run_program(program: &str) -> Result<RunOutput> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| Error::Reason(e.to_string()))?;
    rt.block_on(async {
        let mut engine: Engine = load_string(program.to_string()).await.map_err(|e| Error::Reason(format!("nemo: {e}")))?;
        reason(&mut engine).await.map_err(|e| Error::Reason(format!("nemo reasoning: {e}")))?;
        let tag = Tag::new("new".to_string());
        let mut triples = Vec::new();
        if let Some(rows) = engine.predicate_rows(&tag).await.map_err(|e| Error::Reason(format!("nemo output: {e}")))? {
            for row in rows {
                if row.len() != 3 {
                    continue;
                }
                triples.push(DerivedTriple { s: to_term(&row[0]), p: to_term(&row[1]), o: to_term(&row[2]) });
            }
        }
        let derived_facts = engine.count_facts_in_memory_for_derived_predicates();
        Ok(RunOutput { triples, derived_facts })
    })
}

/// Validate a program without running it (used by `pack check`).
pub fn validate_program(program: &str) -> Result<()> {
    let report = nemo::api::validate(program.to_string(), "nix2rdf".to_string());
    // A ProgramReport that still parses to a program is fine; otherwise report errors.
    match nemo::api::load_program(program.to_string(), "nix2rdf".to_string()) {
        Ok(_) => Ok(()),
        Err(e) => Err(Error::Reason(format!("nemo program invalid:\n{e}\n{report:?}"))),
    }
}

#[derive(Debug)]
pub struct RunOutput {
    pub triples: Vec<DerivedTriple>,
    pub derived_facts: usize,
}
