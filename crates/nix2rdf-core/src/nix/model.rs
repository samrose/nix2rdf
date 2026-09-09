//! Serde models of the Nix CLI's JSON output.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One entry of `nix derivation show`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DrvInfo {
    pub name: String,
    pub system: String,
    pub builder: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub input_drvs: BTreeMap<String, InputDrv>,
    #[serde(default)]
    pub input_srcs: Vec<String>,
    #[serde(default)]
    pub outputs: BTreeMap<String, OutputInfo>,
}

/// Nix ≥ 2.19 prints `{"dynamicOutputs": {}, "outputs": ["out"]}`; older
/// versions print a bare array. Accept both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputDrv {
    Structured {
        #[serde(default)]
        outputs: Vec<String>,
        #[serde(default, rename = "dynamicOutputs")]
        dynamic_outputs: serde_json::Value,
    },
    List(Vec<String>),
}

impl InputDrv {
    pub fn outputs(&self) -> &[String] {
        match self {
            InputDrv::Structured { outputs, .. } => outputs,
            InputDrv::List(l) => l,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OutputInfo {
    /// Absent for content-addressed derivations (unknown until built).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_algo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl DrvInfo {
    /// True when any output lacks a path (content-addressed / floating).
    pub fn is_content_addressed(&self) -> bool {
        self.outputs.values().any(|o| o.path.is_none())
    }

    pub fn uses_structured_attrs(&self) -> bool {
        self.env.get("__structuredAttrs").map(|v| v == "1" || v == "true").unwrap_or(false) || self.env.contains_key("__json")
    }

    /// Structured attrs (the `__json` variable) parsed, if present.
    pub fn structured_attrs(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        let j = self.env.get("__json")?;
        serde_json::from_str::<serde_json::Value>(j).ok()?.as_object().cloned()
    }

    /// Look up a scalar attribute in env or structured attrs.
    pub fn attr(&self, key: &str) -> Option<String> {
        if let Some(v) = self.env.get(key) {
            return Some(v.clone());
        }
        let sa = self.structured_attrs()?;
        match sa.get(key)? {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

/// One entry of `nix path-info --json` (Nix ≥ 2.19 object form).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    #[serde(default)]
    pub nar_hash: Option<String>,
    #[serde(default)]
    pub nar_size: Option<u64>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub deriver: Option<String>,
    #[serde(default)]
    pub ca: Option<String>,
    #[serde(default)]
    pub signatures: Vec<String>,
}

/// One entry of `nix build --json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BuildResult {
    pub drv_path: String,
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
}

/// One line of `nix-eval-jobs` output.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EvalJob {
    #[serde(default)]
    pub attr: String,
    #[serde(default)]
    pub attr_path: Vec<String>,
    #[serde(default)]
    pub drv_path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
    #[serde(default)]
    pub input_drvs: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// meta.* fields the extractor records (from nix eval or nix-eval-jobs --meta).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub licenses: Vec<String>,
    pub unfree: bool,
    pub homepage: Option<String>,
    pub description: Option<String>,
    pub source_provenance: Vec<String>,
}

impl Meta {
    /// Parse a `meta` JSON object. Licenses: `spdxId` when present, else
    /// `shortName`, else the raw string. Accepts a single license, a list, or a string.
    pub fn from_json(meta: &serde_json::Value) -> Meta {
        let mut m = Meta::default();
        let Some(obj) = meta.as_object() else { return m };
        fn one(v: &serde_json::Value, out: &mut Vec<String>, unfree: &mut bool) {
            match v {
                serde_json::Value::String(s) => out.push(s.clone()),
                serde_json::Value::Object(o) => {
                    if let Some(id) = o.get("spdxId").and_then(|x| x.as_str()) {
                        out.push(id.to_string());
                    } else if let Some(sn) = o.get("shortName").and_then(|x| x.as_str()) {
                        out.push(sn.to_string());
                    } else if let Some(fn_) = o.get("fullName").and_then(|x| x.as_str()) {
                        out.push(fn_.to_string());
                    }
                    if o.get("free").and_then(|x| x.as_bool()) == Some(false) {
                        *unfree = true;
                    }
                }
                serde_json::Value::Array(a) => {
                    for x in a {
                        one(x, out, unfree);
                    }
                }
                _ => {}
            }
        }
        if let Some(l) = obj.get("license") {
            one(l, &mut m.licenses, &mut m.unfree);
        }
        m.licenses.sort();
        m.licenses.dedup();
        m.homepage = obj.get("homepage").and_then(|h| match h {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str()).map(|s| s.to_string()),
            _ => None,
        });
        m.description = obj.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
        if let Some(sp) = obj.get("sourceProvenance").and_then(|x| x.as_array()) {
            for p in sp {
                if let Some(s) = p.get("shortName").and_then(|x| x.as_str()) {
                    m.source_provenance.push(s.to_string());
                }
            }
            m.source_provenance.sort();
        }
        m
    }
}
