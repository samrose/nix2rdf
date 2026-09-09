//! A `NixSource` that replays recorded CLI output from a directory:
//!
//! ```text
//! <dir>/derivation-show.json      # nix derivation show -r <roots>
//! <dir>/path-info.json            # nix path-info --json <paths>
//! <dir>/flake-metadata.json       # nix flake metadata --json
//! <dir>/flake-show.json           # nix flake show --json
//! <dir>/build.json                # nix build --json
//! <dir>/eval/<label>.json         # nix eval --json, keyed by EvalArgs.label
//! ```
//!
//! Used by tests, the determinism check, and `--from-recorded`. Also written
//! by `nix2rdf record` so a live extraction can be replayed later.

use super::{BuildResult, DrvInfo, EvalArgs, NixSource, PathInfo};
use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RecordedNix {
    pub dir: PathBuf,
}

impl RecordedNix {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        RecordedNix { dir: dir.into() }
    }

    fn read_json(&self, name: &str) -> Result<serde_json::Value> {
        let p = self.dir.join(name);
        let bytes = std::fs::read(&p).map_err(|e| Error::io(&p, e))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn read_json_opt(&self, name: &str) -> Result<Option<serde_json::Value>> {
        let p = self.dir.join(name);
        if !p.exists() {
            return Ok(None);
        }
        self.read_json(name).map(Some)
    }

    pub fn write_json(dir: &Path, name: &str, v: &serde_json::Value) -> Result<()> {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::write(&p, serde_json::to_vec_pretty(v)?).map_err(|e| Error::io(&p, e))
    }
}

impl NixSource for RecordedNix {
    fn derivation_show(
        &self,
        installables: &[String],
        recursive: bool,
    ) -> Result<BTreeMap<String, DrvInfo>> {
        let all: BTreeMap<String, DrvInfo> =
            serde_json::from_value(self.read_json("derivation-show.json")?)?;
        if recursive {
            return Ok(all);
        }
        // Non-recursive: explicit .drv paths filter the recorded graph; anything
        // else (attribute installables) answers from derivation-show-roots.json.
        let wanted: Vec<&String> = installables
            .iter()
            .filter(|i| i.ends_with(".drv"))
            .collect();
        if !wanted.is_empty() {
            return Ok(all
                .into_iter()
                .filter(|(k, _)| wanted.contains(&k))
                .collect());
        }
        match self.read_json_opt("derivation-show-roots.json")? {
            Some(v) => Ok(serde_json::from_value(v)?),
            None => Ok(all),
        }
    }

    fn path_info(&self, paths: &[String]) -> Result<BTreeMap<String, Option<PathInfo>>> {
        let recorded: BTreeMap<String, Option<PathInfo>> =
            match self.read_json_opt("path-info.json")? {
                Some(v) => serde_json::from_value(v)?,
                None => BTreeMap::new(),
            };
        Ok(paths
            .iter()
            .map(|p| (p.clone(), recorded.get(p).cloned().flatten()))
            .collect())
    }

    fn flake_metadata(&self, _flake_ref: &str) -> Result<serde_json::Value> {
        self.read_json("flake-metadata.json")
    }

    fn eval_json(&self, a: &EvalArgs) -> Result<serde_json::Value> {
        self.read_json(&format!("eval/{}.json", a.label))
    }

    fn build_json(&self, _installable: &str) -> Result<Vec<BuildResult>> {
        Ok(serde_json::from_value(self.read_json("build.json")?)?)
    }

    fn flake_show(&self, _flake_ref: &str) -> Result<serde_json::Value> {
        self.read_json("flake-show.json")
    }

    fn version(&self) -> String {
        "recorded".into()
    }
}
