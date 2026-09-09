//! Desired state: a directory of rendered manifests (YAML/JSON, multi-doc,
//! `kind: List` flattened). What a GitOps commit would apply.

use super::model::{kind, normalize};
use crate::error::{Error, Result};
use serde_json::Value;
use std::path::Path;
use tracing::info;

pub fn read_dir(dir: &Path) -> Result<Vec<Value>> {
    let mut files: Vec<_> = walkdir::WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .filter(|p| {
            matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("yaml" | "yml" | "json")
            )
        })
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| Error::io(f, e))?;
        for doc in serde_yaml_ng::Deserializer::from_str(&text) {
            let v: Value = match Value::deserialize(doc) {
                Ok(v) => v,
                Err(e) => return Err(Error::Other(format!("{}: {e}", f.display()))),
            };
            push_object(&mut out, v);
        }
    }
    info!(target: "nix2rdf::k8s", dir = %dir.display(), files = files.len(), objects = out.len(), "manifests read");
    Ok(out)
}

fn push_object(out: &mut Vec<Value>, mut v: Value) {
    if v.is_null() {
        return;
    }
    if kind(&v) == "List" || kind(&v).ends_with("List") {
        if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
            for it in items.clone() {
                push_object(out, it);
            }
        }
        return;
    }
    if !v.is_object() || kind(&v).is_empty() {
        return;
    }
    normalize(&mut v);
    out.push(v);
}

use serde::Deserialize;
