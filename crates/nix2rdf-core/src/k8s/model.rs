//! Helpers over raw Kubernetes objects (as `serde_json::Value`).

use serde_json::Value;
use std::collections::BTreeMap;

pub type Labels = BTreeMap<String, String>;

pub fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_str()
}
pub fn i(v: &Value, path: &[&str]) -> Option<i64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_i64()
}
pub fn arr<'a>(v: &'a Value, path: &[&str]) -> Vec<&'a Value> {
    let mut cur = v;
    for p in path {
        match cur.get(p) {
            Some(c) => cur = c,
            None => return vec![],
        }
    }
    cur.as_array().map(|a| a.iter().collect()).unwrap_or_default()
}
pub fn map(v: &Value, path: &[&str]) -> Labels {
    let mut cur = v;
    for p in path {
        match cur.get(p) {
            Some(c) => cur = c,
            None => return Labels::new(),
        }
    }
    cur.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default()
}

pub fn kind(v: &Value) -> &str {
    s(v, &["kind"]).unwrap_or("")
}
pub fn name(v: &Value) -> &str {
    s(v, &["metadata", "name"]).unwrap_or("")
}
pub fn namespace(v: &Value) -> Option<&str> {
    s(v, &["metadata", "namespace"])
}
pub fn labels(v: &Value) -> Labels {
    map(v, &["metadata", "labels"])
}
pub fn annotations(v: &Value) -> Labels {
    map(v, &["metadata", "annotations"])
}

pub const WORKLOAD_KINDS: &[&str] = &["Deployment", "StatefulSet", "DaemonSet", "Job", "CronJob"];
pub const NAMESPACED_KINDS: &[&str] = &[
    "Deployment", "StatefulSet", "DaemonSet", "Job", "CronJob", "Service", "Ingress", "ConfigMap", "Secret",
    "PersistentVolumeClaim", "NetworkPolicy", "ServiceAccount", "Role", "RoleBinding", "PodDisruptionBudget",
];
pub const CLUSTER_KINDS: &[&str] = &["Node", "Namespace", "PersistentVolume", "StorageClass", "ClusterRole", "ClusterRoleBinding"];

/// The pod template spec of a workload.
pub fn pod_spec(w: &Value) -> Option<&Value> {
    match kind(w) {
        "CronJob" => w.get("spec")?.get("jobTemplate")?.get("spec")?.get("template")?.get("spec"),
        _ => w.get("spec")?.get("template")?.get("spec"),
    }
}
pub fn pod_labels(w: &Value) -> Labels {
    match kind(w) {
        "CronJob" => map(w, &["spec", "jobTemplate", "spec", "template", "metadata", "labels"]),
        _ => map(w, &["spec", "template", "metadata", "labels"]),
    }
}

/// Evaluate a LabelSelector (matchLabels + matchExpressions) against labels.
/// A selector that is null/empty matches nothing (Service semantics for a
/// missing selector) unless `empty_matches_all` (NetworkPolicy podSelector).
pub fn selector_matches(sel: Option<&Value>, labels: &Labels, empty_matches_all: bool) -> bool {
    let Some(sel) = sel else { return empty_matches_all };
    let ml = map(sel, &["matchLabels"]);
    let exprs = arr(sel, &["matchExpressions"]);
    if ml.is_empty() && exprs.is_empty() {
        return empty_matches_all;
    }
    for (k, v) in &ml {
        if labels.get(k) != Some(v) {
            return false;
        }
    }
    for e in exprs {
        let key = s(e, &["key"]).unwrap_or("");
        let op = s(e, &["operator"]).unwrap_or("");
        let values: Vec<&str> = arr(e, &["values"]).into_iter().filter_map(|x| x.as_str()).collect();
        let have = labels.get(key);
        let ok = match op {
            "In" => have.map(|h| values.contains(&h.as_str())).unwrap_or(false),
            "NotIn" => have.map(|h| !values.contains(&h.as_str())).unwrap_or(true),
            "Exists" => have.is_some(),
            "DoesNotExist" => have.is_none(),
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Service selectors are plain label maps.
pub fn plain_selector_matches(sel: &Labels, labels: &Labels) -> bool {
    !sel.is_empty() && sel.iter().all(|(k, v)| labels.get(k) == Some(v))
}

/// Split an image reference into (repository, tag, digest).
pub fn parse_image_ref(r: &str) -> (String, Option<String>, Option<String>) {
    let (rest, digest) = match r.split_once('@') {
        Some((a, d)) => (a.to_string(), Some(d.to_string())),
        None => (r.to_string(), None),
    };
    // A ':' after the last '/' is a tag (not a registry port).
    let last_slash = rest.rfind('/').map(|i| i + 1).unwrap_or(0);
    let (repo, tag) = match rest[last_slash..].rfind(':') {
        Some(i) => (rest[..last_slash + i].to_string(), Some(rest[last_slash + i + 1..].to_string())),
        None => (rest.clone(), None),
    };
    (repo, tag, digest)
}

pub fn digest_hex(d: &str) -> Option<String> {
    let h = d.strip_prefix("sha256:")?;
    if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(h.to_ascii_lowercase())
    } else {
        None
    }
}

/// Strip fields that change without the state changing, so that the
/// snapshot hash is stable. Secrets lose their data unconditionally.
pub fn normalize(v: &mut Value) {
    if let Some(meta) = v.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        for k in ["resourceVersion", "uid", "managedFields", "creationTimestamp", "generation", "selfLink", "deletionTimestamp", "deletionGracePeriodSeconds"] {
            meta.remove(k);
        }
        if let Some(a) = meta.get_mut("annotations").and_then(|a| a.as_object_mut()) {
            a.remove("kubectl.kubernetes.io/last-applied-configuration");
            a.remove("deployment.kubernetes.io/revision");
        }
    }
    if kind(v) == "Secret" {
        if let Some(o) = v.as_object_mut() {
            o.remove("data");
            o.remove("stringData");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_refs() {
        assert_eq!(parse_image_ref("nginx"), ("nginx".into(), None, None));
        assert_eq!(parse_image_ref("registry:5000/a/b:1.2"), ("registry:5000/a/b".into(), Some("1.2".into()), None));
        let (r, t, d) = parse_image_ref("ghcr.io/x/y:v1@sha256:abcd");
        assert_eq!((r.as_str(), t.as_deref(), d.as_deref()), ("ghcr.io/x/y", Some("v1"), Some("sha256:abcd")));
    }
}
