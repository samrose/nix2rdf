//! Live state: read-only listing from the API server with `kube`.
//! Supports kubeconfig contexts and in-cluster service-account auth.
//! Secrets are listed as metadata only, so their values never leave the
//! API server. Each kind is listed independently; failures are recorded
//! (the snapshot becomes `k8s:partial`) rather than aborting, which is what
//! an edge k3s cluster with flaky components needs.

use crate::error::{Error, Result};
use k8s_openapi::api as k;
use kube::api::{ApiResource, DynamicObject, ListParams};
use kube::{Api, Client, Config};
use serde_json::Value;
use tracing::{info, warn};

pub struct LiveState {
    pub objects: Vec<Value>,
    /// Pods (normalized) — used for placement and pulled digests only.
    pub pods: Vec<Value>,
    pub failed_kinds: Vec<String>,
}

fn resources() -> Vec<(&'static str, ApiResource)> {
    vec![
        ("Namespace", ApiResource::erase::<k::core::v1::Namespace>(&())),
        ("Node", ApiResource::erase::<k::core::v1::Node>(&())),
        ("Deployment", ApiResource::erase::<k::apps::v1::Deployment>(&())),
        ("StatefulSet", ApiResource::erase::<k::apps::v1::StatefulSet>(&())),
        ("DaemonSet", ApiResource::erase::<k::apps::v1::DaemonSet>(&())),
        ("ReplicaSet", ApiResource::erase::<k::apps::v1::ReplicaSet>(&())),
        ("Job", ApiResource::erase::<k::batch::v1::Job>(&())),
        ("CronJob", ApiResource::erase::<k::batch::v1::CronJob>(&())),
        ("Service", ApiResource::erase::<k::core::v1::Service>(&())),
        ("Ingress", ApiResource::erase::<k::networking::v1::Ingress>(&())),
        ("ConfigMap", ApiResource::erase::<k::core::v1::ConfigMap>(&())),
        ("PersistentVolumeClaim", ApiResource::erase::<k::core::v1::PersistentVolumeClaim>(&())),
        ("PersistentVolume", ApiResource::erase::<k::core::v1::PersistentVolume>(&())),
        ("StorageClass", ApiResource::erase::<k::storage::v1::StorageClass>(&())),
        ("NetworkPolicy", ApiResource::erase::<k::networking::v1::NetworkPolicy>(&())),
        ("ServiceAccount", ApiResource::erase::<k::core::v1::ServiceAccount>(&())),
        ("Role", ApiResource::erase::<k::rbac::v1::Role>(&())),
        ("RoleBinding", ApiResource::erase::<k::rbac::v1::RoleBinding>(&())),
        ("ClusterRole", ApiResource::erase::<k::rbac::v1::ClusterRole>(&())),
        ("ClusterRoleBinding", ApiResource::erase::<k::rbac::v1::ClusterRoleBinding>(&())),
        ("PodDisruptionBudget", ApiResource::erase::<k::policy::v1::PodDisruptionBudget>(&())),
    ]
}

async fn client_for(context: Option<&str>) -> Result<Client> {
    let cfg = match context {
        Some(ctx) => Config::from_kubeconfig(&kube::config::KubeConfigOptions { context: Some(ctx.to_string()), ..Default::default() }).await.map_err(|e| Error::Kube(e.to_string()))?,
        None => Config::infer().await.map_err(|e| Error::Kube(e.to_string()))?,
    };
    Client::try_from(cfg).map_err(|e| Error::Kube(e.to_string()))
}

fn to_json(o: DynamicObject, kind: &str, api_version: &str) -> Value {
    let mut v = serde_json::to_value(&o).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("kind".into(), Value::String(kind.into()));
        obj.insert("apiVersion".into(), Value::String(api_version.into()));
    }
    super::model::normalize(&mut v);
    v
}

pub fn list_live(context: Option<&str>) -> Result<LiveState> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| Error::Kube(e.to_string()))?;
    rt.block_on(async {
        let client = client_for(context).await?;
        let mut st = LiveState { objects: vec![], pods: vec![], failed_kinds: vec![] };
        let lp = ListParams::default();
        for (kind, ar) in resources() {
            let api: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
            match api.list(&lp).await {
                Ok(list) => {
                    let n = list.items.len();
                    for o in list.items {
                        st.objects.push(to_json(o, kind, &ar.api_version));
                    }
                    info!(target: "nix2rdf::k8s", kind, count = n, "listed");
                }
                Err(e) => {
                    warn!(target: "nix2rdf::k8s", kind, error = %e, "list failed; snapshot will be partial");
                    st.failed_kinds.push(kind.to_string());
                }
            }
        }
        // Secrets: metadata only.
        let sec_ar = ApiResource::erase::<k::core::v1::Secret>(&());
        let sec: Api<DynamicObject> = Api::all_with(client.clone(), &sec_ar);
        match sec.list_metadata(&lp).await {
            Ok(list) => {
                for m in list.items {
                    let v = serde_json::json!({ "kind": "Secret", "apiVersion": "v1", "metadata": serde_json::to_value(&m.metadata).unwrap_or(Value::Null) });
                    let mut v = v;
                    super::model::normalize(&mut v);
                    st.objects.push(v);
                }
            }
            Err(e) => {
                warn!(target: "nix2rdf::k8s", kind = "Secret", error = %e, "list failed; snapshot will be partial");
                st.failed_kinds.push("Secret".into());
            }
        }
        // Pods: placement and pulled digests only.
        let pod_ar = ApiResource::erase::<k::core::v1::Pod>(&());
        let pods: Api<DynamicObject> = Api::all_with(client.clone(), &pod_ar);
        match pods.list(&lp).await {
            Ok(list) => {
                for o in list.items {
                    st.pods.push(to_json(o, "Pod", "v1"));
                }
            }
            Err(e) => {
                warn!(target: "nix2rdf::k8s", kind = "Pod", error = %e, "list failed; placement unknown");
                st.failed_kinds.push("Pod".into());
            }
        }
        Ok(st)
    })
}
