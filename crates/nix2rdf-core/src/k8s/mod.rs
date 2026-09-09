//! Kubernetes / k3s front-end: a cluster snapshot at the image-digest level.
//!
//! Both sources — a directory of rendered manifests (desired state) and the
//! API server (live state) — produce plain JSON objects; `snapshot` turns
//! the normalized set into fragments. Pods are read only to learn which node
//! a workload runs on and which image digest was actually pulled; nothing
//! per pod is stored. Secret values are dropped at parse time and never held.

pub mod api;
pub mod manifests;
pub mod model;
pub mod snapshot;

pub use snapshot::{build_snapshot, SnapshotInput, SnapshotOptions, SnapshotOutput};

/// Which labels/annotations carry ownership, cost and topology. Documented
/// precedence: workload label > workload annotation > namespace label >
/// namespace annotation, first matching key in list order.
#[derive(Debug, Clone)]
pub struct LabelConfig {
    pub owner_keys: Vec<String>,
    pub cost_center_keys: Vec<String>,
    pub zone_keys: Vec<String>,
    pub region_keys: Vec<String>,
    pub rack_key: String,
    pub site_key: String,
    pub generation_annotation: String,
    pub exempt_single_domain_annotation: String,
    /// A namespace is "production" when it carries one of these `key=value` labels.
    pub production_labels: Vec<String>,
}

impl Default for LabelConfig {
    fn default() -> Self {
        LabelConfig {
            owner_keys: vec!["nix2rdf.io/owner".into(), "owner".into(), "team".into()],
            cost_center_keys: vec!["nix2rdf.io/cost-center".into(), "cost-center".into()],
            zone_keys: vec!["topology.kubernetes.io/zone".into(), "failure-domain.beta.kubernetes.io/zone".into()],
            region_keys: vec!["topology.kubernetes.io/region".into(), "failure-domain.beta.kubernetes.io/region".into()],
            rack_key: "topology.nix2rdf.io/rack".into(),
            site_key: "topology.nix2rdf.io/site".into(),
            generation_annotation: "nix2rdf.io/generation".into(),
            exempt_single_domain_annotation: "nix2rdf.io/exempt-single-domain".into(),
            production_labels: vec!["environment=production".into(), "env=prod".into(), "nix2rdf.io/production=true".into()],
        }
    }
}
