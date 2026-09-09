//! Turn a normalized set of Kubernetes objects into three fragments:
//! - `k8s/<cluster>/snapshot/<hash>`: workloads, containers (by digest),
//!   services, storage, RBAC, network policies, ownership — content-addressed.
//! - `k8s/<cluster>/nodes/<hash>`: node inventory with hostGeneration links.
//! - `k8s/<cluster>/observed/<hash>-<time>`: the timestamp of this observation.
//!
//! Identical cluster state → identical snapshot hash → no new fragment.

use super::model::*;
use super::LabelConfig;
use crate::fragment::{Fragment, FragmentKind};
use crate::hash;
use crate::iri;
use crate::vocab::k8s_terms as k;
use crate::vocab::nix_terms as n;
use crate::vocab::xsd_date_time;
use oxrdf::NamedNode;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    Live,
    Desired,
}

#[derive(Debug, Clone)]
pub struct SnapshotOptions {
    pub cluster_id: String,
    pub state: StateKind,
    pub labels: LabelConfig,
    /// node name → NixOS generation hash (from --nixos-node-map).
    pub node_map: BTreeMap<String, String>,
    /// RFC 3339; defaults to now.
    pub observed_at: Option<String>,
}

#[derive(Debug, Default)]
pub struct SnapshotInput {
    pub objects: Vec<Value>,
    pub pods: Vec<Value>,
    pub failed_kinds: Vec<String>,
}

#[derive(Debug)]
pub struct SnapshotOutput {
    pub snapshot: Fragment,
    pub nodes: Fragment,
    pub observed: Fragment,
    pub snapshot_iri: NamedNode,
    pub snapshot_hash: String,
    pub nodes_hash: String,
    pub objects: usize,
}

const PLACEHOLDER: &str = "PENDINGHASH";

struct Ctx<'a> {
    cid: &'a str,
    cfg: &'a LabelConfig,
    /// (kind, ns, name) → object
    index: BTreeMap<(String, String, String), &'a Value>,
    ns_meta: BTreeMap<String, (Labels, Labels)>,
}

impl<'a> Ctx<'a> {
    fn get(&self, kind: &str, ns: &str, name: &str) -> Option<&'a Value> {
        self.index
            .get(&(kind.to_string(), ns.to_string(), name.to_string()))
            .copied()
    }
    fn of_kind(&self, kind: &str) -> Vec<&'a Value> {
        self.index
            .iter()
            .filter(|((k, _, _), _)| k == kind)
            .map(|(_, v)| *v)
            .collect()
    }
    fn obj_iri(&self, kind: &str, ns: Option<&str>, name: &str) -> NamedNode {
        match kind {
            "Node" => iri::k8s_node(self.cid, name),
            "Namespace" => iri::k8s_namespace(self.cid, name),
            _ => iri::k8s_obj(self.cid, kind, ns, name),
        }
    }
    fn first_key(&self, keys: &[String], labels: &Labels, ann: &Labels) -> Option<String> {
        for key in keys {
            if let Some(v) = labels.get(key) {
                return Some(v.clone());
            }
        }
        for key in keys {
            if let Some(v) = ann.get(key) {
                return Some(v.clone());
            }
        }
        None
    }
    /// Owner precedence: workload label > workload annotation > namespace label > namespace annotation.
    fn owner(&self, labels: &Labels, ann: &Labels, ns: Option<&str>) -> Option<String> {
        self.first_key(&self.cfg.owner_keys, labels, ann)
            .or_else(|| {
                let (nl, na) = ns
                    .and_then(|n| self.ns_meta.get(n))
                    .cloned()
                    .unwrap_or_default();
                self.first_key(&self.cfg.owner_keys, &nl, &na)
            })
    }
    fn cost_center(&self, labels: &Labels, ann: &Labels, ns: Option<&str>) -> Option<String> {
        self.first_key(&self.cfg.cost_center_keys, labels, ann)
            .or_else(|| {
                let (nl, na) = ns
                    .and_then(|n| self.ns_meta.get(n))
                    .cloned()
                    .unwrap_or_default();
                self.first_key(&self.cfg.cost_center_keys, &nl, &na)
            })
    }
}

fn add_labels(f: &mut Fragment, subj: &NamedNode, labels: &Labels) {
    for (key, v) in labels {
        f.add_str(subj.clone(), k::label(), &format!("{key}={v}"));
    }
}

fn nodes_fragment(ctx: &Ctx, opts: &SnapshotOptions) -> (Fragment, BTreeSet<String>) {
    let mut f = Fragment::new(FragmentKind::K8sNodes {
        cluster: opts.cluster_id.clone(),
        hash: PLACEHOLDER.into(),
    });
    let cluster = iri::k8s_cluster(ctx.cid);
    let mut names = BTreeSet::new();
    for node in ctx.of_kind("Node") {
        let nm = name(node);
        names.insert(nm.to_string());
        let ni = iri::k8s_node(ctx.cid, nm);
        f.add_type(ni.clone(), k::Node());
        f.add_str(ni.clone(), k::name(), nm);
        f.add(ni.clone(), k::inCluster(), cluster.clone());
        let labels = labels(node);
        let ann = annotations(node);
        add_labels(&mut f, &ni, &labels);
        for key in labels.keys() {
            if let Some(role) = key.strip_prefix("node-role.kubernetes.io/") {
                f.add_str(ni.clone(), k::nodeRole(), role);
            }
        }
        for (prop, path) in [
            (k::kubeletVersion(), "kubeletVersion"),
            (k::containerRuntimeVersion(), "containerRuntimeVersion"),
            (k::kernelVersion(), "kernelVersion"),
            (k::osImage(), "osImage"),
        ] {
            if let Some(v) = s(node, &["status", "nodeInfo", path]) {
                f.add_str(ni.clone(), prop, v);
            }
        }
        let mut domains: Vec<(&str, String)> = Vec::new();
        if let Some(z) = ctx.cfg.zone_keys.iter().find_map(|kk| labels.get(kk)) {
            domains.push(("zone", z.clone()));
        }
        if let Some(r) = ctx.cfg.region_keys.iter().find_map(|kk| labels.get(kk)) {
            domains.push(("region", r.clone()));
        }
        if let Some(r) = labels.get(&ctx.cfg.rack_key) {
            domains.push(("rack", r.clone()));
        }
        if let Some(r) = labels.get(&ctx.cfg.site_key) {
            domains.push(("site", r.clone()));
        }
        for (kind, val) in domains {
            let d = iri::k8s_failure_domain(ctx.cid, kind, &val);
            f.add_type(d.clone(), k::FailureDomain());
            f.add_str(d.clone(), k::domainKind(), kind);
            f.add_str(d.clone(), k::name(), &val);
            f.add(d.clone(), k::inCluster(), cluster.clone());
            f.add(ni.clone(), k::inFailureDomain(), d);
        }
        let gen = opts
            .node_map
            .get(nm)
            .cloned()
            .or_else(|| ann.get(&ctx.cfg.generation_annotation).cloned());
        if let Some(g) = gen {
            f.add(ni.clone(), k::hostGeneration(), iri::gen(&g));
        }
    }
    (f, names)
}

fn container_specs(
    f: &mut Fragment,
    ctx: &Ctx,
    w: &Value,
    wi: &NamedNode,
    pulled: &BTreeMap<String, BTreeSet<String>>,
) {
    let Some(ps) = pod_spec(w) else { return };
    let ns = namespace(w).unwrap_or("default");
    for (list, init) in [("containers", false), ("initContainers", true)] {
        for c in arr(ps, &[list]) {
            let cname = s(c, &["name"]).unwrap_or("");
            let ci = iri::k8s_container(ctx.cid, kind(w), ns, name(w), cname);
            f.add_type(ci.clone(), k::ContainerSpec());
            f.add_str(ci.clone(), k::name(), cname);
            f.add(wi.clone(), k::hasContainer(), ci.clone());
            if init {
                f.add_bool(ci.clone(), k::initContainer(), true);
            }
            let image = s(c, &["image"]).unwrap_or("");
            f.add_str(ci.clone(), k::imageRef(), image);
            let (repo, tag, digest) = parse_image_ref(image);
            f.add_str(ci.clone(), k::imageRepository(), &repo);
            if let Some(t) = &tag {
                f.add_str(ci.clone(), k::imageTag(), t);
            }
            let mut digests: BTreeSet<String> = BTreeSet::new();
            if let Some(h) = digest.as_deref().and_then(digest_hex) {
                digests.insert(h);
            }
            if let Some(p) = pulled.get(cname) {
                digests.extend(p.iter().cloned());
            }
            if digests.is_empty() {
                f.add_str(ci.clone(), k::unresolvedImage(), image);
            }
            for d in digests {
                f.add(ci.clone(), k::runsImage(), iri::image(&d));
            }
            for (prop, path) in [
                (k::requestsCpu(), ["requests", "cpu"]),
                (k::requestsMemory(), ["requests", "memory"]),
                (k::limitsCpu(), ["limits", "cpu"]),
                (k::limitsMemory(), ["limits", "memory"]),
            ] {
                if let Some(v) = c
                    .get("resources")
                    .and_then(|r| r.get(path[0]))
                    .and_then(|r| r.get(path[1]))
                    .and_then(|v| v.as_str())
                {
                    f.add_str(ci.clone(), prop, v);
                }
            }
            // env / envFrom references → mounts on the workload.
            for e in arr(c, &["env"]) {
                if let Some(nm) = s(e, &["valueFrom", "configMapKeyRef", "name"]) {
                    f.add(
                        wi.clone(),
                        k::mounts(),
                        ctx.obj_iri("ConfigMap", Some(ns), nm),
                    );
                }
                if let Some(nm) = s(e, &["valueFrom", "secretKeyRef", "name"]) {
                    f.add(wi.clone(), k::mounts(), ctx.obj_iri("Secret", Some(ns), nm));
                }
            }
            for e in arr(c, &["envFrom"]) {
                if let Some(nm) = s(e, &["configMapRef", "name"]) {
                    f.add(
                        wi.clone(),
                        k::mounts(),
                        ctx.obj_iri("ConfigMap", Some(ns), nm),
                    );
                }
                if let Some(nm) = s(e, &["secretRef", "name"]) {
                    f.add(wi.clone(), k::mounts(), ctx.obj_iri("Secret", Some(ns), nm));
                }
            }
        }
    }
    for v in arr(ps, &["volumes"]) {
        if let Some(nm) = s(v, &["configMap", "name"]) {
            f.add(
                wi.clone(),
                k::mounts(),
                ctx.obj_iri("ConfigMap", Some(ns), nm),
            );
        }
        if let Some(nm) = s(v, &["secret", "secretName"]) {
            f.add(wi.clone(), k::mounts(), ctx.obj_iri("Secret", Some(ns), nm));
        }
        if let Some(nm) = s(v, &["persistentVolumeClaim", "claimName"]) {
            f.add(
                wi.clone(),
                k::mounts(),
                ctx.obj_iri("PersistentVolumeClaim", Some(ns), nm),
            );
        }
        for src in arr(v, &["projected", "sources"]) {
            if let Some(nm) = s(src, &["configMap", "name"]) {
                f.add(
                    wi.clone(),
                    k::mounts(),
                    ctx.obj_iri("ConfigMap", Some(ns), nm),
                );
            }
            if let Some(nm) = s(src, &["secret", "name"]) {
                f.add(wi.clone(), k::mounts(), ctx.obj_iri("Secret", Some(ns), nm));
            }
        }
    }
    // StatefulSet volumeClaimTemplates create PVCs named <template>-<sts>-<i>; record the template's storage class.
    for t in arr(w, &["spec", "volumeClaimTemplates"]) {
        if let Some(sc) = s(t, &["spec", "storageClassName"]) {
            f.add(
                wi.clone(),
                k::usesStorageClass(),
                ctx.obj_iri("StorageClass", None, sc),
            );
        }
    }
    let sa = s(ps, &["serviceAccountName"])
        .or(s(ps, &["serviceAccount"]))
        .unwrap_or("default");
    f.add(
        wi.clone(),
        k::usesServiceAccount(),
        ctx.obj_iri("ServiceAccount", Some(ns), sa),
    );
}

/// Resolve pod → owning workload (kind, ns, name) through ReplicaSets and Jobs.
fn pod_owner(ctx: &Ctx, pod: &Value) -> Option<(String, String, String)> {
    let ns = namespace(pod).unwrap_or("default").to_string();
    let owners = arr(pod, &["metadata", "ownerReferences"]);
    let o = owners.first()?;
    let mut cur_kind = s(o, &["kind"]).unwrap_or("").to_string();
    let mut cur_name = s(o, &["name"]).unwrap_or("").to_string();
    for _ in 0..3 {
        if WORKLOAD_KINDS.contains(&cur_kind.as_str()) {
            // A Job owned by a CronJob is attributed to the CronJob.
            if cur_kind == "Job" {
                if let Some(j) = ctx.get("Job", &ns, &cur_name) {
                    if let Some(o) = arr(j, &["metadata", "ownerReferences"]).first() {
                        if s(o, &["kind"]) == Some("CronJob") {
                            return Some((
                                "CronJob".into(),
                                ns,
                                s(o, &["name"]).unwrap_or("").to_string(),
                            ));
                        }
                    }
                }
            }
            return Some((cur_kind, ns, cur_name));
        }
        if cur_kind == "ReplicaSet" {
            let rs = ctx.get("ReplicaSet", &ns, &cur_name)?;
            let o = arr(rs, &["metadata", "ownerReferences"]).first().copied()?;
            cur_kind = s(o, &["kind"]).unwrap_or("").to_string();
            cur_name = s(o, &["name"]).unwrap_or("").to_string();
            continue;
        }
        return None;
    }
    None
}

pub fn build_snapshot(input: &SnapshotInput, opts: &SnapshotOptions) -> SnapshotOutput {
    let cid = opts.cluster_id.as_str();
    let mut index: BTreeMap<(String, String, String), &Value> = BTreeMap::new();
    let mut ns_meta: BTreeMap<String, (Labels, Labels)> = BTreeMap::new();
    for o in &input.objects {
        let kd = kind(o).to_string();
        if kd.is_empty() || kd == "Pod" {
            continue;
        }
        let ns = if CLUSTER_KINDS.contains(&kd.as_str()) {
            String::new()
        } else {
            namespace(o).unwrap_or("default").to_string()
        };
        index.insert((kd.clone(), ns, name(o).to_string()), o);
        if kd == "Namespace" {
            ns_meta.insert(name(o).to_string(), (labels(o), annotations(o)));
        }
    }
    let ctx = Ctx {
        cid,
        cfg: &opts.labels,
        index,
        ns_meta,
    };
    let cluster = iri::k8s_cluster(cid);

    // Pods → placement and pulled digests per workload.
    let mut placement: BTreeMap<(String, String, String), BTreeMap<String, i64>> = BTreeMap::new();
    let mut pulled: BTreeMap<(String, String, String), BTreeMap<String, BTreeSet<String>>> =
        BTreeMap::new();
    for pod in &input.pods {
        let Some(owner) = pod_owner(&ctx, pod) else {
            continue;
        };
        if let Some(node) = s(pod, &["spec", "nodeName"]) {
            *placement
                .entry(owner.clone())
                .or_default()
                .entry(node.to_string())
                .or_default() += 1;
        }
        for cs in arr(pod, &["status", "containerStatuses"])
            .into_iter()
            .chain(arr(pod, &["status", "initContainerStatuses"]))
        {
            let cname = s(cs, &["name"]).unwrap_or("").to_string();
            if let Some(id) = s(cs, &["imageID"]) {
                let d = id.rsplit_once("@").map(|(_, d)| d).unwrap_or(id);
                let d = d.strip_prefix("docker-pullable://").unwrap_or(d);
                if let Some(h) = digest_hex(d.rsplit('@').next().unwrap_or(d)) {
                    pulled
                        .entry(owner.clone())
                        .or_default()
                        .entry(cname)
                        .or_default()
                        .insert(h);
                }
            }
        }
    }

    let (nodes_frag, node_names) = nodes_fragment(&ctx, opts);
    let nodes_hash = hash::sha256_hex(nodes_frag.to_nquads().as_bytes());
    let nodes = Fragment::from_quads(
        FragmentKind::K8sNodes {
            cluster: cid.into(),
            hash: nodes_hash.clone(),
        },
        &rehash(&nodes_frag, &nodes_hash),
    );

    let mut f = Fragment::new(FragmentKind::K8sSnapshot {
        cluster: cid.into(),
        hash: PLACEHOLDER.into(),
    });
    let snap = f.graph();
    f.add_type(snap.clone(), k::Snapshot());
    f.add(
        snap.clone(),
        k::snapshotKind(),
        if opts.state == StateKind::Live {
            k::liveState()
        } else {
            k::desiredState()
        },
    );
    f.add(snap.clone(), k::ofCluster(), cluster.clone());
    f.add_type(cluster.clone(), k::Cluster());
    f.add_str(cluster.clone(), k::clusterId(), cid);
    if !input.failed_kinds.is_empty() {
        f.add_bool(snap.clone(), k::partial(), true);
        for fk in &input.failed_kinds {
            f.add_str(snap.clone(), k::failedKind(), fk);
        }
    }
    for nn in &node_names {
        f.add(snap.clone(), k::contains(), iri::k8s_node(cid, nn));
    }

    let mut count = 0usize;
    for ((kd, ns, nm), o) in &ctx.index {
        if kd == "ReplicaSet" {
            continue;
        }
        count += 1;
        let nsopt = if ns.is_empty() {
            None
        } else {
            Some(ns.as_str())
        };
        let oi = ctx.obj_iri(kd, nsopt, nm);
        f.add(snap.clone(), k::contains(), oi.clone());
        f.add_str(oi.clone(), k::name(), nm);
        f.add_str(oi.clone(), k::kind(), kd);
        if let Some(av) = s(o, &["apiVersion"]) {
            f.add_str(oi.clone(), k::apiVersion(), av);
        }
        f.add(oi.clone(), k::inCluster(), cluster.clone());
        if let Some(ns) = nsopt {
            f.add(oi.clone(), k::inNamespace(), iri::k8s_namespace(cid, ns));
        }
        let labels = labels(o);
        let ann = annotations(o);
        match kd.as_str() {
            "Node" => { /* in the nodes fragment */ }
            "Namespace" => {
                f.add_type(oi.clone(), k::Namespace());
                add_labels(&mut f, &oi, &labels);
                match ctx.owner(&labels, &ann, None) {
                    Some(ow) => {
                        let on = iri::owner(&ow);
                        f.add_type(on.clone(), k::Owner());
                        f.add_str(on.clone(), k::name(), &ow);
                        f.add(oi.clone(), k::ownedBy(), on);
                    }
                    None => f.add_bool(oi.clone(), k::ownerUnknown(), true),
                }
            }
            w if WORKLOAD_KINDS.contains(&w) => {
                f.add_type(oi.clone(), k::Workload());
                f.add_type(oi.clone(), iri::k8s(w));
                add_labels(&mut f, &oi, &labels);
                let replicas = match w {
                    "DaemonSet" => i(o, &["status", "desiredNumberScheduled"]),
                    "CronJob" => None,
                    "Job" => i(o, &["spec", "parallelism"]).or(Some(1)),
                    _ => i(o, &["spec", "replicas"]).or(Some(1)),
                };
                if let Some(r) = replicas {
                    f.add_int(oi.clone(), k::replicas(), r);
                }
                let ready = match w {
                    "DaemonSet" => i(o, &["status", "numberReady"]),
                    "Job" => i(o, &["status", "succeeded"]),
                    _ => i(o, &["status", "readyReplicas"]),
                };
                if let Some(r) = ready {
                    f.add_int(oi.clone(), k::readyReplicas(), r);
                }
                let key = (kd.clone(), ns.clone(), nm.clone());
                if let Some(pl) = placement.get(&key) {
                    for (node, cnt) in pl {
                        f.add(oi.clone(), k::scheduledOn(), iri::k8s_node(cid, node));
                        f.add_str(oi.clone(), k::replicasOnNode(), &format!("{node}:{cnt}"));
                    }
                }
                let empty = BTreeMap::new();
                container_specs(&mut f, &ctx, o, &oi, pulled.get(&key).unwrap_or(&empty));
                match ctx.owner(&labels, &ann, nsopt) {
                    Some(ow) => {
                        let on = iri::owner(&ow);
                        f.add_type(on.clone(), k::Owner());
                        f.add_str(on.clone(), k::name(), &ow);
                        f.add(oi.clone(), k::ownedBy(), on);
                    }
                    None => f.add_bool(oi.clone(), k::ownerUnknown(), true),
                }
                if let Some(cc) = ctx.cost_center(&labels, &ann, nsopt) {
                    f.add_str(oi.clone(), k::costCenter(), &cc);
                }
                if ann
                    .get(&ctx.cfg.exempt_single_domain_annotation)
                    .map(|v| v == "true")
                    .unwrap_or(false)
                {
                    f.add_bool(oi.clone(), k::exemptSingleDomain(), true);
                }
                // PDBs selecting this workload's pods.
                let pl = pod_labels(o);
                for pdb in ctx.of_kind("PodDisruptionBudget") {
                    if namespace(pdb).unwrap_or("default") == ns
                        && selector_matches(
                            pdb.get("spec").and_then(|x| x.get("selector")),
                            &pl,
                            false,
                        )
                    {
                        f.add(
                            oi.clone(),
                            k::hasPDB(),
                            ctx.obj_iri("PodDisruptionBudget", nsopt, name(pdb)),
                        );
                    }
                }
            }
            "Service" => {
                f.add_type(oi.clone(), k::Service());
                let sel = map(o, &["spec", "selector"]);
                for w in ctx
                    .index
                    .iter()
                    .filter(|((wk, wns, _), _)| WORKLOAD_KINDS.contains(&wk.as_str()) && wns == ns)
                    .map(|(_, v)| *v)
                {
                    if plain_selector_matches(&sel, &pod_labels(w)) {
                        f.add(
                            oi.clone(),
                            k::selects(),
                            ctx.obj_iri(kind(w), nsopt, name(w)),
                        );
                    }
                }
            }
            "Ingress" => {
                f.add_type(oi.clone(), k::Ingress());
                let mut svcs: BTreeSet<String> = BTreeSet::new();
                if let Some(d) = s(o, &["spec", "defaultBackend", "service", "name"]) {
                    svcs.insert(d.to_string());
                }
                for r in arr(o, &["spec", "rules"]) {
                    for p in arr(r, &["http", "paths"]) {
                        if let Some(sv) = s(p, &["backend", "service", "name"]) {
                            svcs.insert(sv.to_string());
                        }
                    }
                }
                for sv in svcs {
                    f.add(
                        oi.clone(),
                        k::routesTo(),
                        ctx.obj_iri("Service", nsopt, &sv),
                    );
                }
            }
            "ConfigMap" => f.add_type(oi.clone(), k::ConfigMap()),
            "Secret" => f.add_type(oi.clone(), k::Secret()),
            "ServiceAccount" => f.add_type(oi.clone(), k::ServiceAccount()),
            "StorageClass" => f.add_type(oi.clone(), k::StorageClass()),
            "PodDisruptionBudget" => {
                f.add_type(oi.clone(), k::PodDisruptionBudget());
                if let Some(v) = o.get("spec").and_then(|x| x.get("minAvailable")) {
                    f.add_str(
                        oi.clone(),
                        k::minAvailable(),
                        v.to_string().trim_matches('"'),
                    );
                }
                if let Some(v) = o.get("spec").and_then(|x| x.get("maxUnavailable")) {
                    f.add_str(
                        oi.clone(),
                        k::maxUnavailable(),
                        v.to_string().trim_matches('"'),
                    );
                }
            }
            "PersistentVolumeClaim" => {
                f.add_type(oi.clone(), k::PersistentVolumeClaim());
                if let Some(sc) = s(o, &["spec", "storageClassName"]) {
                    f.add(
                        oi.clone(),
                        k::usesStorageClass(),
                        ctx.obj_iri("StorageClass", None, sc),
                    );
                }
                if let Some(pv) = s(o, &["spec", "volumeName"]) {
                    f.add(
                        oi.clone(),
                        k::boundTo(),
                        ctx.obj_iri("PersistentVolume", None, pv),
                    );
                }
            }
            "PersistentVolume" => {
                f.add_type(oi.clone(), k::PersistentVolume());
                if let Some(sc) = s(o, &["spec", "storageClassName"]) {
                    f.add(
                        oi.clone(),
                        k::usesStorageClass(),
                        ctx.obj_iri("StorageClass", None, sc),
                    );
                }
                for term in arr(
                    o,
                    &["spec", "nodeAffinity", "required", "nodeSelectorTerms"],
                ) {
                    for e in arr(term, &["matchExpressions"]) {
                        if s(e, &["key"]) == Some("kubernetes.io/hostname")
                            && s(e, &["operator"]) == Some("In")
                        {
                            for v in arr(e, &["values"]) {
                                if let Some(nn) = v.as_str() {
                                    f.add(oi.clone(), k::pinnedToNode(), iri::k8s_node(cid, nn));
                                }
                            }
                        }
                    }
                }
            }
            "NetworkPolicy" => {
                f.add_type(oi.clone(), k::NetworkPolicy());
                let types: Vec<String> = arr(o, &["spec", "policyTypes"])
                    .into_iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                let types = if types.is_empty() {
                    vec!["Ingress".to_string()]
                } else {
                    types
                };
                for t in &types {
                    f.add_str(oi.clone(), k::policyType(), t);
                }
                let pod_sel = o.get("spec").and_then(|x| x.get("podSelector"));
                let workloads_in_ns: Vec<&Value> = ctx
                    .index
                    .iter()
                    .filter(|((wk, wns, _), _)| WORKLOAD_KINDS.contains(&wk.as_str()) && wns == ns)
                    .map(|(_, v)| *v)
                    .collect();
                for w in &workloads_in_ns {
                    if selector_matches(pod_sel, &pod_labels(w), true) {
                        f.add(
                            oi.clone(),
                            k::appliesTo(),
                            ctx.obj_iri(kind(w), nsopt, name(w)),
                        );
                    }
                }
                let mut peers = |rules: Vec<&Value>, prop: NamedNode, all_prop: NamedNode| {
                    for rule in rules {
                        let from = arr(rule, &["from"])
                            .into_iter()
                            .chain(arr(rule, &["to"]))
                            .collect::<Vec<_>>();
                        if from.is_empty() {
                            f.add_bool(oi.clone(), all_prop.clone(), true);
                            continue;
                        }
                        for peer in from {
                            let ns_sel = peer.get("namespaceSelector");
                            let p_sel = peer.get("podSelector");
                            // Namespaces this peer covers: own ns unless a namespaceSelector is given.
                            let nss: Vec<String> = match ns_sel {
                                Some(sel) => ctx
                                    .ns_meta
                                    .iter()
                                    .filter(|(_, (l, _))| selector_matches(Some(sel), l, true))
                                    .map(|(n, _)| n.clone())
                                    .collect(),
                                None => vec![ns.clone()],
                            };
                            for pns in nss {
                                match p_sel {
                                    Some(ps) => {
                                        for w in ctx
                                            .index
                                            .iter()
                                            .filter(|((wk, wns, _), _)| {
                                                WORKLOAD_KINDS.contains(&wk.as_str()) && wns == &pns
                                            })
                                            .map(|(_, v)| *v)
                                        {
                                            if selector_matches(Some(ps), &pod_labels(w), true) {
                                                f.add(
                                                    oi.clone(),
                                                    prop.clone(),
                                                    ctx.obj_iri(kind(w), Some(&pns), name(w)),
                                                );
                                            }
                                        }
                                    }
                                    None => f.add(
                                        oi.clone(),
                                        prop.clone(),
                                        iri::k8s_namespace(cid, &pns),
                                    ),
                                }
                            }
                        }
                    }
                };
                if types.iter().any(|t| t == "Ingress") {
                    peers(
                        arr(o, &["spec", "ingress"]),
                        k::allowsIngressFrom(),
                        k::allowsAllIngress(),
                    );
                }
                if types.iter().any(|t| t == "Egress") {
                    peers(
                        arr(o, &["spec", "egress"]),
                        k::allowsEgressTo(),
                        k::allowsAllEgress(),
                    );
                }
            }
            "Role" | "ClusterRole" => {
                f.add_type(
                    oi.clone(),
                    if kd == "Role" {
                        k::Role()
                    } else {
                        k::ClusterRole()
                    },
                );
                for rule in arr(o, &["rules"]) {
                    let groups: Vec<String> = arr(rule, &["apiGroups"])
                        .into_iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect();
                    let groups = if groups.is_empty() {
                        vec!["".to_string()]
                    } else {
                        groups
                    };
                    for g in &groups {
                        for r in arr(rule, &["resources"]) {
                            for v in arr(rule, &["verbs"]) {
                                let (r, v) = (r.as_str().unwrap_or(""), v.as_str().unwrap_or(""));
                                let pi = iri::k8s_permission(cid, g, r, v);
                                f.add_type(pi.clone(), k::Permission());
                                f.add_str(pi.clone(), k::apiGroup(), g);
                                f.add_str(pi.clone(), k::resource(), r);
                                f.add_str(pi.clone(), k::verb(), v);
                                f.add(oi.clone(), k::permits(), pi);
                            }
                        }
                    }
                }
                if kd == "ClusterRole" {
                    for sel in arr(o, &["aggregationRule", "clusterRoleSelectors"]) {
                        for other in ctx.of_kind("ClusterRole") {
                            if name(other) != nm
                                && selector_matches(Some(sel), &labels_of(other), false)
                            {
                                f.add(
                                    oi.clone(),
                                    k::aggregates(),
                                    ctx.obj_iri("ClusterRole", None, name(other)),
                                );
                            }
                        }
                    }
                }
            }
            "RoleBinding" | "ClusterRoleBinding" => {
                f.add_type(
                    oi.clone(),
                    if kd == "RoleBinding" {
                        k::RoleBinding()
                    } else {
                        k::ClusterRoleBinding()
                    },
                );
                let rk = s(o, &["roleRef", "kind"]).unwrap_or("");
                let rn = s(o, &["roleRef", "name"]).unwrap_or("");
                let role = if rk == "ClusterRole" {
                    ctx.obj_iri("ClusterRole", None, rn)
                } else {
                    ctx.obj_iri("Role", nsopt, rn)
                };
                f.add(oi.clone(), k::grants(), role);
                for sub in arr(o, &["subjects"]) {
                    let sk = s(sub, &["kind"]).unwrap_or("");
                    let sn = s(sub, &["name"]).unwrap_or("");
                    let subj = match sk {
                        "ServiceAccount" => {
                            let sns = s(sub, &["namespace"]).or(nsopt).unwrap_or("default");
                            ctx.obj_iri("ServiceAccount", Some(sns), sn)
                        }
                        "User" => {
                            let u = ctx.obj_iri("user", None, sn);
                            f.add_type(u.clone(), k::User());
                            f.add_str(u.clone(), k::name(), sn);
                            u
                        }
                        _ => {
                            let g = ctx.obj_iri("group", None, sn);
                            f.add_type(g.clone(), k::Group());
                            f.add_str(g.clone(), k::name(), sn);
                            g
                        }
                    };
                    f.add(oi.clone(), k::subjectOf(), subj);
                }
            }
            _ => {}
        }
    }

    let snapshot_hash = hash::sha256_hex(f.to_nquads().as_bytes());
    let snapshot = Fragment::from_quads(
        FragmentKind::K8sSnapshot {
            cluster: cid.into(),
            hash: snapshot_hash.clone(),
        },
        &rehash(&f, &snapshot_hash),
    );
    let snapshot_iri = snapshot.graph();

    let observed_at = opts
        .observed_at
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    let mut observed = Fragment::new(FragmentKind::K8sObserved {
        cluster: cid.into(),
        hash: snapshot_hash.clone(),
        observed_at: observed_at.clone(),
    });
    observed.add_typed(
        snapshot_iri.clone(),
        k::observedAt(),
        &observed_at,
        xsd_date_time(),
    );
    observed.add(
        snapshot_iri.clone(),
        n::hasFragment(),
        iri::k8s_nodes_fragment(cid, &nodes_hash),
    );
    info!(target: "nix2rdf::k8s", cluster = cid, objects = count, nodes = node_names.len(), snapshot = %snapshot_hash, nodes_hash = %nodes_hash, partial = !input.failed_kinds.is_empty(), "snapshot built");
    SnapshotOutput {
        snapshot,
        nodes,
        observed,
        snapshot_iri,
        snapshot_hash,
        nodes_hash,
        objects: count,
    }
}

fn labels_of(v: &Value) -> Labels {
    labels(v)
}

/// Replace the placeholder graph hash with the real one in every quad.
fn rehash(f: &Fragment, real: &str) -> Vec<oxrdf::Quad> {
    let text = f.to_nquads().replace(PLACEHOLDER, real);
    crate::fragment::parse_nquads(text.as_bytes()).unwrap_or_default()
}
