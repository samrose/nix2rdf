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
        self.env
            .get("__structuredAttrs")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false)
            || self.env.contains_key("__json")
    }

    /// Structured attrs (the `__json` variable) parsed, if present.
    pub fn structured_attrs(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        let j = self.env.get("__json")?;
        serde_json::from_str::<serde_json::Value>(j)
            .ok()?
            .as_object()
            .cloned()
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

/// The store directory used to turn the base names Nix ≥ 2.34 prints back
/// into full paths.
pub fn store_dir() -> String {
    std::env::var("NIX_STORE_DIR").unwrap_or_else(|_| "/nix/store".to_string())
}

fn full_path(name: &str) -> String {
    if name.starts_with('/') {
        name.to_string()
    } else {
        format!("{}/{name}", store_dir())
    }
}

/// Parse `nix derivation show` output of any schema Nix has shipped into the
/// classic shape:
/// - Nix ≤ 2.33: `{"/nix/store/<h>-x.drv": {name, env (with __json), inputDrvs, inputSrcs, outputs{path: full}}}`
/// - Nix ≥ 2.34: `{"version": 4, "derivations": {"<h>-x.drv": {inputs{drvs,srcs}, structuredAttrs, outputs{path: basename}}}}`
///
/// Every derived fragment must be byte-identical regardless of the Nix
/// version that produced the JSON, so the new shape is normalized to the old
/// one: base names get the store directory back, and `structuredAttrs` goes
/// into `env.__json` serialized the way Nix itself does (sorted keys, no
/// whitespace), which is what the .drv file holds.
pub fn parse_derivation_show(
    v: serde_json::Value,
) -> Result<BTreeMap<String, DrvInfo>, serde_json::Error> {
    let obj = match v.as_object() {
        Some(o) => o,
        None => return serde_json::from_value(v),
    };
    if !obj.contains_key("version") || !obj.contains_key("derivations") {
        return serde_json::from_value(v);
    }
    let mut out = BTreeMap::new();
    let Some(drvs) = obj.get("derivations").and_then(|d| d.as_object()) else {
        return Ok(out);
    };
    for (key, e) in drvs {
        let mut d = DrvInfo {
            name: e
                .get("name")
                .and_then(|n| n.as_str())
                .map(String::from)
                .unwrap_or_default(),
            system: e
                .get("system")
                .and_then(|n| n.as_str())
                .map(String::from)
                .unwrap_or_default(),
            builder: e
                .get("builder")
                .and_then(|n| n.as_str())
                .map(String::from)
                .unwrap_or_default(),
            args: e
                .get("args")
                .map(|a| serde_json::from_value(a.clone()))
                .transpose()?
                .unwrap_or_default(),
            env: e
                .get("env")
                .map(|a| serde_json::from_value(a.clone()))
                .transpose()?
                .unwrap_or_default(),
            ..Default::default()
        };
        if let Some(sa) = e.get("structuredAttrs").filter(|x| x.is_object()) {
            d.env
                .insert("__json".into(), crate::hash::canonical_json(sa));
        }
        if let Some(inputs) = e.get("inputs") {
            if let Some(drvs) = inputs.get("drvs").and_then(|x| x.as_object()) {
                for (p, spec) in drvs {
                    d.input_drvs
                        .insert(full_path(p), serde_json::from_value(spec.clone())?);
                }
            }
            if let Some(srcs) = inputs.get("srcs").and_then(|x| x.as_array()) {
                d.input_srcs = srcs
                    .iter()
                    .filter_map(|s| s.as_str())
                    .map(full_path)
                    .collect();
            }
        }
        if let Some(outs) = e.get("outputs").and_then(|x| x.as_object()) {
            for (name, o) in outs {
                let mut info: OutputInfo = serde_json::from_value(o.clone())?;
                if let Some(p) = &info.path {
                    info.path = Some(full_path(p));
                }
                // Nix ≥ 2.34 omits the path of fixed-output derivations (it is
                // derivable from the hash) but still exports it as the output's
                // environment variable. Floating content-addressed outputs have a
                // placeholder there instead, which is not a store path.
                if info.path.is_none() && info.hash.is_some() {
                    if let Some(p) = d.env.get(name) {
                        if p.starts_with(&store_dir()) && crate::iri::store_path_hash(p).is_some() {
                            info.path = Some(p.clone());
                        }
                    }
                }
                // Hash as `<algo>-<base64>` (SRI) → hex plus hashAlgo, the classic form.
                if let Some(h) = info.hash.clone() {
                    if let Some((algo, b64)) = h.split_once('-') {
                        if info.hash_algo.is_none() {
                            use base64::Engine;
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64)
                            {
                                info.hash = Some(hex::encode(bytes));
                                info.hash_algo = Some(algo.to_string());
                            }
                        }
                    }
                }
                d.outputs.insert(name.clone(), info);
            }
        }
        if d.name.is_empty() {
            d.name = crate::iri::store_path_name(key)
                .unwrap_or_default()
                .trim_end_matches(".drv")
                .to_string();
        }
        out.insert(full_path(key), d);
    }
    Ok(out)
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
        let Some(obj) = meta.as_object() else {
            return m;
        };
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
            serde_json::Value::Array(a) => {
                a.first().and_then(|x| x.as_str()).map(|s| s.to_string())
            }
            _ => None,
        });
        m.description = obj
            .get("description")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    const V3: &str = r#"{"/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-x-1.drv": {
      "args": ["-e"], "builder": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-bash/bin/bash",
      "env": {"__json": "{\"a\":[1,2],\"name\":\"x-1\",\"pname\":\"x\",\"version\":\"1\"}", "out": "/nix/store/cccccccccccccccccccccccccccccccc-x-1"},
      "inputDrvs": {"/nix/store/dddddddddddddddddddddddddddddddd-y.drv": {"dynamicOutputs": {}, "outputs": ["out"]}},
      "inputSrcs": ["/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-s.sh"],
      "name": "x-1", "outputs": {"out": {"path": "/nix/store/cccccccccccccccccccccccccccccccc-x-1"}}, "system": "x86_64-linux"}}"#;

    const V4: &str = r#"{"version": 4, "derivations": {"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-x-1.drv": {
      "args": ["-e"], "builder": "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-bash/bin/bash",
      "env": {"out": "/nix/store/cccccccccccccccccccccccccccccccc-x-1"},
      "inputs": {"drvs": {"dddddddddddddddddddddddddddddddd-y.drv": {"dynamicOutputs": {}, "outputs": ["out"]}}, "srcs": ["eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-s.sh"]},
      "name": "x-1", "outputs": {"out": {"path": "cccccccccccccccccccccccccccccccc-x-1"}},
      "structuredAttrs": {"version": "1", "pname": "x", "name": "x-1", "a": [1, 2]}, "system": "x86_64-linux", "version": 4}}}"#;

    const V3_FIXED: &str = r#"{"/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-source.drv": {
      "args": [], "builder": "builtin:fetchurl", "env": {"out": "/nix/store/cccccccccccccccccccccccccccccccc-source"},
      "inputDrvs": {}, "inputSrcs": [], "name": "source", "system": "builtin",
      "outputs": {"out": {"hash": "2949042b3da342af35e65b36e6ed0931899a77870df4f4bf9c2dc4e5e575f5ca", "hashAlgo": "sha256", "method": "nar", "path": "/nix/store/cccccccccccccccccccccccccccccccc-source"}}}}"#;
    const V4_FIXED: &str = r#"{"version": 4, "derivations": {"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-source.drv": {
      "args": [], "builder": "builtin:fetchurl", "env": {"out": "/nix/store/cccccccccccccccccccccccccccccccc-source"},
      "inputs": {"drvs": {}, "srcs": []}, "name": "source", "system": "builtin",
      "outputs": {"out": {"hash": "sha256-KUkEKz2jQq815ls25u0JMYmad4cN9PS/nC3E5eV19co=", "method": "nar"}}, "version": 4}}}"#;

    #[test]
    fn fixed_output_path_is_recovered_from_env() {
        let a = parse_derivation_show(serde_json::from_str(V3_FIXED).unwrap()).unwrap();
        let b = parse_derivation_show(serde_json::from_str(V4_FIXED).unwrap()).unwrap();
        let (x, y) = (a.values().next().unwrap(), b.values().next().unwrap());
        assert_eq!(
            serde_json::to_value(x).unwrap(),
            serde_json::to_value(y).unwrap()
        );
        assert!(!y.is_content_addressed());
        assert_eq!(
            y.outputs["out"].path.as_deref(),
            Some("/nix/store/cccccccccccccccccccccccccccccccc-source")
        );
    }

    #[test]
    fn v3_and_v4_normalize_to_the_same_derivation() {
        let a = parse_derivation_show(serde_json::from_str(V3).unwrap()).unwrap();
        let b = parse_derivation_show(serde_json::from_str(V4).unwrap()).unwrap();
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
        let (x, y) = (a.values().next().unwrap(), b.values().next().unwrap());
        assert_eq!(
            serde_json::to_value(x).unwrap(),
            serde_json::to_value(y).unwrap()
        );
        assert_eq!(x.attr("pname").as_deref(), Some("x"));
        assert!(x.uses_structured_attrs());
        assert_eq!(
            x.input_srcs,
            vec!["/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-s.sh"]
        );
    }
}
