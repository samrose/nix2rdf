//! The IRI scheme. See IRI.md for the guarantees.
//!
//! Two namespaces hold vocabulary (`nix:`, `k8s:`); every instance lives under
//! the single base `https://w3id.org/nix2rdf/id/`, partitioned by the first path
//! segment. Nothing here is ever minted from non-deterministic input.

use base64::Engine;
use oxrdf::NamedNode;

pub const NIX_NS: &str = "https://w3id.org/nix2rdf/ns#";
pub const K8S_NS: &str = "https://w3id.org/nix2rdf/k8s#";
pub const ID_BASE: &str = "https://w3id.org/nix2rdf/id/";
pub const POLICY_BASE: &str = "https://w3id.org/nix2rdf/policy/";
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// Prefixes as used in the documentation and in SPARQL. The `nix:` in the
/// original design's instance table (`nix:drv/<hash>`) is what we call
/// `nixid:` here, because one prefix cannot expand to two bases.
pub const PREFIXES: &[(&str, &str)] = &[
    ("nix", NIX_NS),
    ("k8s", K8S_NS),
    ("nixid", ID_BASE),
    ("git", "https://w3id.org/nix2rdf/id/git/"),
    ("oci", "https://w3id.org/nix2rdf/id/oci/"),
    ("pack", "https://w3id.org/nix2rdf/id/pack/"),
    ("derived", "https://w3id.org/nix2rdf/id/derived/"),
    ("frag", "https://w3id.org/nix2rdf/id/fragment/"),
    ("k8sid", "https://w3id.org/nix2rdf/id/k8s/"),
    ("org", "https://w3id.org/nix2rdf/id/org/"),
    ("policy", POLICY_BASE),
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("xsd", XSD),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("spdx", "https://spdx.org/rdf/3.0.1/terms/Core/"),
];

pub fn sparql_prologue() -> String {
    PREFIXES
        .iter()
        .map(|(p, u)| format!("PREFIX {p}: <{u}>\n"))
        .collect()
}

/// Percent-encode everything that is not unreserved or a safe sub-delim, so
/// that any string can be a path segment. `/` is encoded (it would split
/// segments). Deterministic and reversible.
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'@'
            | b'+'
            | b'!'
            | b','
            | b'='
            | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A NAR hash as an IRI segment: the SRI form `sha256-<base64>` becomes
/// `sha256-<base64url without padding>` so it contains no `/`, `+` or `=`.
/// The original SRI string is always also stored as a `nix:narHash` literal.
pub fn nar_hash_segment(sri: &str) -> String {
    if let Some((algo, b64)) = sri.split_once('-') {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
            let url = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            return format!("{algo}-{url}");
        }
    }
    encode_segment(sri)
}

/// The 32-character hash part of a store path, e.g.
/// `/nix/store/7rnrwhl2b91gb9b7na73q6c4cvvv1hi8-hello-2.12.3.drv` → `7rnr...`.
pub fn store_path_hash(path: &str) -> Option<&str> {
    let base = path.rsplit('/').next()?;
    let (hash, _) = base.split_once('-')?;
    if hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Some(hash)
    } else {
        None
    }
}

/// The name part of a store path (after the hash and dash).
pub fn store_path_name(path: &str) -> Option<&str> {
    let base = path.rsplit('/').next()?;
    base.split_once('-').map(|(_, n)| n)
}

fn nn(s: String) -> NamedNode {
    NamedNode::new(s).expect("IRIs built here are always valid")
}

pub fn id(path: &str) -> NamedNode {
    nn(format!("{ID_BASE}{path}"))
}
pub fn nix(term: &str) -> NamedNode {
    nn(format!("{NIX_NS}{term}"))
}
pub fn k8s(term: &str) -> NamedNode {
    nn(format!("{K8S_NS}{term}"))
}
pub fn xsd(t: &str) -> NamedNode {
    nn(format!("{XSD}{t}"))
}
pub fn rdf_type() -> NamedNode {
    nn(RDF_TYPE.to_string())
}

// ----- nix: instances -------------------------------------------------------

pub fn drv(store_hash: &str) -> NamedNode {
    id(&format!("drv/{store_hash}"))
}
pub fn out(store_hash: &str) -> NamedNode {
    id(&format!("out/{store_hash}"))
}
pub fn src(nar_hash_sri: &str) -> NamedNode {
    id(&format!("src/{}", nar_hash_segment(nar_hash_sri)))
}
pub fn pin(nar_hash_sri: &str) -> NamedNode {
    id(&format!("pin/{}", nar_hash_segment(nar_hash_sri)))
}
pub fn flake_input(root_nar_hash_sri: &str, parent_node_id: &str, input_name: &str) -> NamedNode {
    id(&format!(
        "pin/{}/input/{}/{}",
        nar_hash_segment(root_nar_hash_sri),
        encode_segment(parent_node_id),
        encode_segment(input_name)
    ))
}
pub fn pin_attr(root_nar_hash_sri: &str, attr_path: &str) -> NamedNode {
    id(&format!(
        "pin/{}/attr/{}",
        nar_hash_segment(root_nar_hash_sri),
        encode_segment(attr_path)
    ))
}
pub fn nixpkgs(rev: &str) -> NamedNode {
    id(&format!("nixpkgs/{rev}"))
}
pub fn nixpkgs_attr(rev: &str, attr_path: &str) -> NamedNode {
    id(&format!("nixpkgs/{rev}/attr/{}", encode_segment(attr_path)))
}
pub fn package_version(attr_path: &str, version: &str) -> NamedNode {
    id(&format!(
        "pkgver/{}/{}",
        encode_segment(attr_path),
        encode_segment(version)
    ))
}
pub fn release(name: &str) -> NamedNode {
    id(&format!("release/{}", encode_segment(name)))
}
pub fn commit(repo: &str, sha: &str) -> NamedNode {
    id(&format!("git/{}/{sha}", encode_segment(repo)))
}
pub fn gen(system_hash: &str) -> NamedNode {
    id(&format!("gen/{system_hash}"))
}
pub fn nixos_configuration(flake_nar_hash_sri: &str, name: &str) -> NamedNode {
    id(&format!(
        "pin/{}/nixosConfigurations/{}",
        nar_hash_segment(flake_nar_hash_sri),
        encode_segment(name)
    ))
}
pub fn option(path: &str) -> NamedNode {
    id(&format!("option/{}", encode_segment(path)))
}
pub fn option_value(gen_hash: &str, path: &str) -> NamedNode {
    id(&format!("gen/{gen_hash}/opt/{}", encode_segment(path)))
}
pub fn option_definition(gen_hash: &str, path: &str, index: usize) -> NamedNode {
    id(&format!(
        "gen/{gen_hash}/opt/{}/def/{index}",
        encode_segment(path)
    ))
}
pub fn value(hash: &str) -> NamedNode {
    id(&format!("value/{hash}"))
}
pub fn image(sha256_hex: &str) -> NamedNode {
    id(&format!("oci/sha256/{sha256_hex}"))
}
pub fn layer(sha256_hex: &str) -> NamedNode {
    id(&format!("oci/layer/sha256/{sha256_hex}"))
}
pub fn pack(content_hash: &str) -> NamedNode {
    id(&format!("pack/{content_hash}"))
}
pub fn derived(ruleset_hash: &str, input_hash: &str) -> NamedNode {
    id(&format!("derived/{ruleset_hash}:{input_hash}"))
}
pub fn fragment(kind: &str, ident: &str) -> NamedNode {
    id(&format!("fragment/{kind}/{ident}"))
}
pub fn policy(pack: &str, name: &str) -> NamedNode {
    nn(format!("{POLICY_BASE}{pack}/{name}"))
}

// ----- k8s: instances -------------------------------------------------------

pub fn k8s_cluster(cluster_id: &str) -> NamedNode {
    id(&format!("k8s/cluster/{}", encode_segment(cluster_id)))
}
pub fn k8s_obj(cluster_id: &str, kind: &str, ns: Option<&str>, name: &str) -> NamedNode {
    match ns {
        Some(ns) => id(&format!(
            "k8s/{}/{kind}/{}/{}",
            encode_segment(cluster_id),
            encode_segment(ns),
            encode_segment(name)
        )),
        None => id(&format!(
            "k8s/{}/{kind}/{}",
            encode_segment(cluster_id),
            encode_segment(name)
        )),
    }
}
pub fn k8s_node(cluster_id: &str, name: &str) -> NamedNode {
    k8s_obj(cluster_id, "node", None, name)
}
pub fn k8s_namespace(cluster_id: &str, name: &str) -> NamedNode {
    k8s_obj(cluster_id, "ns", None, name)
}
pub fn k8s_container(
    cluster_id: &str,
    kind: &str,
    ns: &str,
    name: &str,
    container: &str,
) -> NamedNode {
    id(&format!(
        "k8s/{}/{kind}/{}/{}/c/{}",
        encode_segment(cluster_id),
        encode_segment(ns),
        encode_segment(name),
        encode_segment(container)
    ))
}
pub fn k8s_snapshot(cluster_id: &str, content_hash: &str) -> NamedNode {
    id(&format!(
        "k8s/{}/snapshot/{content_hash}",
        encode_segment(cluster_id)
    ))
}
pub fn k8s_nodes_fragment(cluster_id: &str, content_hash: &str) -> NamedNode {
    id(&format!(
        "k8s/{}/nodes/{content_hash}",
        encode_segment(cluster_id)
    ))
}
pub fn k8s_failure_domain(cluster_id: &str, kind: &str, value: &str) -> NamedNode {
    id(&format!(
        "k8s/{}/domain/{kind}/{}",
        encode_segment(cluster_id),
        encode_segment(value)
    ))
}
pub fn k8s_permission(cluster_id: &str, api_group: &str, resource: &str, verb: &str) -> NamedNode {
    id(&format!(
        "k8s/{}/permission/{}/{}/{}",
        encode_segment(cluster_id),
        encode_segment(if api_group.is_empty() {
            "core"
        } else {
            api_group
        }),
        encode_segment(resource),
        encode_segment(verb)
    ))
}
pub fn owner(label_value: &str) -> NamedNode {
    id(&format!("org/owner/{}", encode_segment(label_value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_hash() {
        assert_eq!(
            store_path_hash("/nix/store/7rnrwhl2b91gb9b7na73q6c4cvvv1hi8-hello-2.12.3.drv"),
            Some("7rnrwhl2b91gb9b7na73q6c4cvvv1hi8")
        );
        assert_eq!(store_path_hash("/nix/store/short-x"), None);
    }

    #[test]
    fn nar_segment_is_url_safe() {
        let seg = nar_hash_segment("sha256-WRTgeV8aGM4+Ztxpsp5js/lFDm0OACpCJwbLlpJfJ20=");
        assert!(!seg.contains('/') && !seg.contains('+') && !seg.contains('='));
        assert!(seg.starts_with("sha256-"));
    }

    #[test]
    fn segment_encoding() {
        assert_eq!(
            encode_segment("python3Packages.requests"),
            "python3Packages.requests"
        );
        assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
    }
}
