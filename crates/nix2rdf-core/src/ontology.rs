//! Ontology tooling: parse the Turtle vocabularies, check that every
//! `nix:`/`k8s:` term used in Rust or in rule packs is declared, and generate
//! human documentation (HTML and Markdown) from the `.ttl` files — the
//! "Widoco or equivalent" of the build.

use crate::error::{Error, Result};
use crate::iri::{K8S_NS, NIX_NS};
use oxrdf::{NamedNode, Term, Triple};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";

#[derive(Debug, Default, Clone)]
pub struct TermDoc {
    pub iri: String,
    pub kinds: BTreeSet<String>,
    pub label: Option<String>,
    pub comment: Option<String>,
    pub domain: Vec<String>,
    pub range: Vec<String>,
    pub sub_class_of: Vec<String>,
    pub sub_property_of: Vec<String>,
    pub inverse_of: Vec<String>,
    pub deprecated: bool,
}

#[derive(Debug, Default)]
pub struct Vocabulary {
    pub namespace: String,
    pub prefix: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub version_iri: Option<String>,
    pub terms: BTreeMap<String, TermDoc>,
}

pub fn parse_turtle(text: &str) -> Result<Vec<Triple>> {
    let mut out = Vec::new();
    for t in oxttl::TurtleParser::new().for_reader(text.as_bytes()) {
        out.push(t.map_err(|e| Error::Rdf(e.to_string()))?);
    }
    Ok(out)
}

fn lit(t: &Term) -> Option<String> {
    match t {
        Term::Literal(l) => Some(l.value().to_string()),
        _ => None,
    }
}
fn iri_of(t: &Term) -> Option<String> {
    match t {
        Term::NamedNode(n) => Some(n.as_str().to_string()),
        _ => None,
    }
}

/// Build a vocabulary description from a Turtle document.
pub fn load_vocabulary(text: &str) -> Result<Vocabulary> {
    let triples = parse_turtle(text)?;
    let mut v = Vocabulary::default();
    // Namespace: the one used by the majority of subjects with a '#'.
    let mut ns_count: BTreeMap<String, usize> = BTreeMap::new();
    for t in &triples {
        if let oxrdf::NamedOrBlankNode::NamedNode(n) = &t.subject {
            if let Some(i) = n.as_str().find('#') {
                *ns_count.entry(n.as_str()[..=i].to_string()).or_default() += 1;
            }
        }
    }
    v.namespace = ns_count
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(n, _)| n)
        .unwrap_or_default();
    v.prefix = if v.namespace == NIX_NS {
        "nix".into()
    } else if v.namespace == K8S_NS {
        "k8s".into()
    } else {
        "ns".into()
    };
    for t in &triples {
        let s = match &t.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
            _ => continue,
        };
        let p = t.predicate.as_str();
        let is_ontology = !s.starts_with(&v.namespace);
        if is_ontology {
            match p {
                "http://purl.org/dc/terms/title" => v.title = lit(&t.object),
                "http://purl.org/dc/terms/description" => v.description = lit(&t.object),
                x if x == format!("{OWL}versionInfo") => v.version = lit(&t.object),
                x if x == format!("{OWL}versionIRI") => v.version_iri = iri_of(&t.object),
                _ => {}
            }
            continue;
        }
        let d = v.terms.entry(s.clone()).or_insert_with(|| TermDoc {
            iri: s.clone(),
            ..Default::default()
        });
        match p {
            RDF_TYPE => {
                if let Some(k) = iri_of(&t.object) {
                    d.kinds.insert(k);
                }
            }
            x if x == format!("{RDFS}label") => d.label = lit(&t.object),
            x if x == format!("{RDFS}comment") => d.comment = lit(&t.object),
            x if x == format!("{RDFS}domain") => d.domain.extend(iri_of(&t.object)),
            x if x == format!("{RDFS}range") => d.range.extend(iri_of(&t.object)),
            x if x == format!("{RDFS}subClassOf") => d.sub_class_of.extend(iri_of(&t.object)),
            x if x == format!("{RDFS}subPropertyOf") => d.sub_property_of.extend(iri_of(&t.object)),
            x if x == format!("{OWL}inverseOf") => d.inverse_of.extend(iri_of(&t.object)),
            x if x == format!("{OWL}deprecated") => {
                d.deprecated = lit(&t.object).as_deref() == Some("true")
            }
            _ => {}
        }
    }
    Ok(v)
}

impl TermDoc {
    pub fn is_class(&self) -> bool {
        self.kinds.iter().any(|k| k.ends_with("#Class"))
    }
    pub fn is_property(&self) -> bool {
        self.kinds.iter().any(|k| {
            k.ends_with("#Property")
                || k.ends_with("ObjectProperty")
                || k.ends_with("DatatypeProperty")
        })
    }
    pub fn is_individual(&self) -> bool {
        !self.is_class() && !self.is_property()
    }
    pub fn local(&self) -> &str {
        self.iri.rsplit('#').next().unwrap_or(&self.iri)
    }
}

/// Terms (as full IRIs) used by Rust: everything in `vocab::*::ALL`.
pub fn rust_terms() -> Vec<String> {
    let mut v: Vec<String> = crate::vocab::nix_terms::ALL
        .iter()
        .map(|t| format!("{NIX_NS}{}", t.trim_start_matches("r#")))
        .collect();
    v.extend(
        crate::vocab::k8s_terms::ALL
            .iter()
            .map(|t| format!("{K8S_NS}{t}")),
    );
    v
}

/// Terms used by rule files: `nix:foo`, `k8s:foo` prefixed names and full IRIs
/// under the two namespaces.
pub fn rule_terms(rule_text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let re = regex::Regex::new(r"\b(nix|k8s):([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let re_iri = regex::Regex::new(r"<(https://w3id\.org/nix/(?:ns|k8s)#[A-Za-z0-9_]+)>").unwrap();
    for line in rule_text.lines() {
        let line = line.split('%').next().unwrap_or("");
        for c in re.captures_iter(line) {
            let ns = if &c[1] == "nix" { NIX_NS } else { K8S_NS };
            out.insert(format!("{ns}{}", &c[2]));
        }
        for c in re_iri.captures_iter(line) {
            out.insert(c[1].to_string());
        }
    }
    out
}

/// Validate: every ttl parses; every term used in Rust and in the packs'
/// rule files is declared in one of the vocabularies (nix.ttl, k8s.ttl, or a
/// pack's vocab.ttl for its own namespace).
pub fn validate(
    ttl_paths: &[std::path::PathBuf],
    pack_dirs: &[std::path::PathBuf],
) -> Result<Vec<String>> {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut problems = Vec::new();
    for p in ttl_paths {
        let text = std::fs::read_to_string(p).map_err(|e| Error::io(p, e))?;
        let v = load_vocabulary(&text).map_err(|e| Error::Rdf(format!("{}: {e}", p.display())))?;
        declared.extend(v.terms.keys().cloned());
    }
    let packs = crate::packs::discover(pack_dirs)?;
    for p in packs.values() {
        if let Some(ttl) = &p.vocab_ttl {
            match load_vocabulary(ttl) {
                Ok(v) => declared.extend(v.terms.keys().cloned()),
                Err(e) => problems.push(format!("pack {}: vocab.ttl: {e}", p.manifest.name)),
            }
        }
    }
    for term in rust_terms() {
        if !declared.contains(&term) {
            problems.push(format!("Rust uses undeclared term {term}"));
        }
    }
    for p in packs.values() {
        for (name, text) in &p.rule_files {
            for term in rule_terms(text) {
                if !declared.contains(&term) {
                    problems.push(format!(
                        "pack {}/{name} uses undeclared term {term}",
                        p.manifest.name
                    ));
                }
            }
        }
    }
    Ok(problems)
}

fn short(iri: &str) -> String {
    for (p, u) in crate::iri::PREFIXES {
        if let Some(rest) = iri.strip_prefix(u) {
            return format!("{p}:{rest}");
        }
    }
    iri.to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn markdown(vocabs: &[Vocabulary]) -> String {
    let mut out = String::new();
    out.push_str("# Ontology reference\n\nGenerated from `ontology/*.ttl` by `nix2rdf ontology doc`. Do not edit by hand.\n\n");
    out.push_str("OWL and RDFS terms in these vocabularies are **notation only**: they record intent for readers and for the `rdfs`/`owl-rl-subset` rule packs. No OWL reasoner runs; every entailment in the dataset was produced by an authored Nemo rule.\n\n");
    for v in vocabs {
        out.push_str(&format!(
            "## {} (`{}:`)\n\n",
            v.title.clone().unwrap_or_else(|| v.namespace.clone()),
            v.prefix
        ));
        out.push_str(&format!(
            "Namespace: `{}`  \nVersion: {}  \nVersion IRI: {}\n\n",
            v.namespace,
            v.version.clone().unwrap_or_default(),
            v.version_iri.clone().unwrap_or_default()
        ));
        if let Some(d) = &v.description {
            out.push_str(d);
            out.push_str("\n\n");
        }
        for (title, pred) in [("Classes", 0), ("Properties", 1), ("Individuals", 2)] {
            let items: Vec<&TermDoc> = v
                .terms
                .values()
                .filter(|t| match pred {
                    0 => t.is_class(),
                    1 => t.is_property(),
                    _ => t.is_individual(),
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            out.push_str(&format!("### {title}\n\n"));
            for t in items {
                out.push_str(&format!(
                    "#### `{}:{}`{}\n\n",
                    v.prefix,
                    t.local(),
                    if t.deprecated { " (deprecated)" } else { "" }
                ));
                if let Some(c) = &t.comment {
                    out.push_str(c);
                    out.push_str("\n\n");
                }
                let mut facts = Vec::new();
                if !t.sub_class_of.is_empty() {
                    facts.push(format!(
                        "subClassOf: {}",
                        t.sub_class_of
                            .iter()
                            .map(|x| format!("`{}`", short(x)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !t.sub_property_of.is_empty() {
                    facts.push(format!(
                        "subPropertyOf: {}",
                        t.sub_property_of
                            .iter()
                            .map(|x| format!("`{}`", short(x)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !t.domain.is_empty() {
                    facts.push(format!(
                        "domain: {}",
                        t.domain
                            .iter()
                            .map(|x| format!("`{}`", short(x)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !t.range.is_empty() {
                    facts.push(format!(
                        "range: {}",
                        t.range
                            .iter()
                            .map(|x| format!("`{}`", short(x)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !t.inverse_of.is_empty() {
                    facts.push(format!(
                        "inverseOf: {}",
                        t.inverse_of
                            .iter()
                            .map(|x| format!("`{}`", short(x)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if t.kinds.iter().any(|k| k.ends_with("TransitiveProperty")) {
                    facts.push("transitive (notation; closure computed by the core pack)".into());
                }
                for f in facts {
                    out.push_str(&format!("- {f}\n"));
                }
                out.push('\n');
            }
        }
    }
    out
}

pub fn html(v: &Vocabulary) -> String {
    let mut s = String::new();
    let title = v.title.clone().unwrap_or_else(|| v.namespace.clone());
    s.push_str(&format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\n",
        esc(&title)
    ));
    s.push_str("<style>body{font-family:system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;line-height:1.5}code{background:#f4f4f4;padding:0 .2em}dt{font-weight:600;margin-top:1rem}dd{margin:0 0 .5rem 1rem}.meta{color:#555;font-size:.9em}</style></head><body>\n");
    s.push_str(&format!("<h1>{}</h1>\n<p class=\"meta\">Namespace <code>{}</code> · version {} · <a href=\"{}\">Turtle</a></p>\n", esc(&title), esc(&v.namespace), esc(&v.version.clone().unwrap_or_default()), if v.prefix == "k8s" { "k8s.ttl" } else { "ns.ttl" }));
    if let Some(d) = &v.description {
        s.push_str(&format!("<p>{}</p>\n", esc(d)));
    }
    s.push_str("<p>OWL/RDFS terms are notation only; semantics are given by Nemo rule packs. Terms are immutable: added, never removed; deprecated with <code>owl:deprecated</code>.</p>\n");
    for (title, pred) in [("Classes", 0), ("Properties", 1), ("Individuals", 2)] {
        let items: Vec<&TermDoc> = v
            .terms
            .values()
            .filter(|t| match pred {
                0 => t.is_class(),
                1 => t.is_property(),
                _ => t.is_individual(),
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        s.push_str(&format!("<h2>{title}</h2>\n<dl>\n"));
        for t in items {
            s.push_str(&format!(
                "<dt id=\"{}\"><code>{}:{}</code>{}</dt>\n",
                esc(t.local()),
                v.prefix,
                esc(t.local()),
                if t.deprecated {
                    " <em>(deprecated)</em>"
                } else {
                    ""
                }
            ));
            if let Some(c) = &t.comment {
                s.push_str(&format!("<dd>{}</dd>\n", esc(c)));
            }
            let mut facts = Vec::new();
            for (k, vals) in [
                ("subClassOf", &t.sub_class_of),
                ("subPropertyOf", &t.sub_property_of),
                ("domain", &t.domain),
                ("range", &t.range),
                ("inverseOf", &t.inverse_of),
            ] {
                if !vals.is_empty() {
                    facts.push(format!(
                        "{k}: {}",
                        vals.iter()
                            .map(|x| format!("<code>{}</code>", esc(&short(x))))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if !facts.is_empty() {
                s.push_str(&format!("<dd class=\"meta\">{}</dd>\n", facts.join(" · ")));
            }
        }
        s.push_str("</dl>\n");
    }
    s.push_str("</body></html>\n");
    s
}

pub fn load_file(p: &Path) -> Result<Vocabulary> {
    let text = std::fs::read_to_string(p).map_err(|e| Error::io(p, e))?;
    load_vocabulary(&text)
}

#[allow(dead_code)]
fn _unused(_: NamedNode) {}
