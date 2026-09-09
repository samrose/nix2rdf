//! The Nix CLI back-end. Every call is logged with the exact argv so the
//! provenance of each fragment is reconstructible from the structured log.

use super::{BuildResult, DrvInfo, EvalArgs, NixSource, PathInfo};
use crate::error::{Error, Result};
use std::collections::BTreeMap;
use std::process::Command;
use tracing::{debug, info};

#[derive(Debug, Clone)]
pub struct CliNix {
    pub nix_bin: String,
    pub extra_args: Vec<String>,
    version: String,
}

impl CliNix {
    pub fn new() -> Self {
        Self::with_binary("nix")
    }

    pub fn with_binary(bin: &str) -> Self {
        let version = Command::new(bin)
            .arg("--version")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".into());
        CliNix {
            nix_bin: bin.to_string(),
            extra_args: vec![
                "--extra-experimental-features".into(),
                "nix-command flakes".into(),
            ],
            version,
        }
    }

    pub fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut cmd = Command::new(&self.nix_bin);
        cmd.args(&self.extra_args).args(args);
        let cmd_line = format!("{} {}", self.nix_bin, args.join(" "));
        info!(target: "nix2rdf::nix", cmd = %cmd_line, nix_version = %self.version, "invoking nix");
        let out = cmd.output().map_err(|e| Error::Nix {
            cmd: cmd_line.clone(),
            stderr: e.to_string(),
        })?;
        if !out.status.success() {
            return Err(Error::Nix {
                cmd: cmd_line,
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            });
        }
        debug!(target: "nix2rdf::nix", bytes = out.stdout.len(), "nix returned");
        Ok(out.stdout)
    }

    pub fn run_json(&self, args: &[&str]) -> Result<serde_json::Value> {
        let out = self.run(args)?;
        Ok(serde_json::from_slice(&out)?)
    }

    /// Run `nix-eval-jobs` (a separate binary) and stream its JSON lines.
    pub fn eval_jobs(&self, args: &[&str]) -> Result<Vec<super::EvalJob>> {
        let cmd_line = format!("nix-eval-jobs {}", args.join(" "));
        info!(target: "nix2rdf::nix", cmd = %cmd_line, "invoking nix-eval-jobs");
        let out = Command::new("nix-eval-jobs")
            .args(args)
            .output()
            .map_err(|e| Error::Nix {
                cmd: cmd_line.clone(),
                stderr: e.to_string(),
            })?;
        if !out.status.success() {
            return Err(Error::Nix {
                cmd: cmd_line,
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            });
        }
        let mut jobs = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if line.trim().is_empty() {
                continue;
            }
            jobs.push(serde_json::from_str::<super::EvalJob>(line)?);
        }
        Ok(jobs)
    }
}

impl Default for CliNix {
    fn default() -> Self {
        Self::new()
    }
}

impl NixSource for CliNix {
    fn derivation_show(
        &self,
        installables: &[String],
        recursive: bool,
    ) -> Result<BTreeMap<String, DrvInfo>> {
        let mut args: Vec<&str> = vec!["derivation", "show"];
        if recursive {
            args.push("-r");
        }
        args.extend(installables.iter().map(|s| s.as_str()));
        let v = self.run_json(&args)?;
        Ok(serde_json::from_value(v)?)
    }

    fn path_info(&self, paths: &[String]) -> Result<BTreeMap<String, Option<PathInfo>>> {
        let mut result = BTreeMap::new();
        // Batch to keep argv under OS limits.
        for chunk in paths.chunks(500) {
            let mut args: Vec<&str> = vec!["path-info", "--json"];
            args.extend(chunk.iter().map(|s| s.as_str()));
            let v = self.run_json(&args)?;
            // Nix ≥ 2.19: object keyed by path; value null when missing.
            // Nix < 2.19: array of {path, narHash, references, ...}.
            match v {
                serde_json::Value::Object(m) => {
                    for (k, val) in m {
                        let info = if val.is_null() {
                            None
                        } else {
                            Some(serde_json::from_value::<PathInfo>(val)?)
                        };
                        result.insert(k, info);
                    }
                }
                serde_json::Value::Array(a) => {
                    for item in a {
                        let path = item
                            .get("path")
                            .and_then(|p| p.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let valid = item.get("valid").and_then(|b| b.as_bool()).unwrap_or(true);
                        let info = if valid {
                            Some(serde_json::from_value::<PathInfo>(item.clone())?)
                        } else {
                            None
                        };
                        result.insert(path, info);
                    }
                }
                _ => {
                    return Err(Error::Nix {
                        cmd: "path-info".into(),
                        stderr: "unexpected JSON shape".into(),
                    })
                }
            }
        }
        Ok(result)
    }

    fn flake_metadata(&self, flake_ref: &str) -> Result<serde_json::Value> {
        self.run_json(&["flake", "metadata", "--json", flake_ref])
    }

    fn eval_json(&self, a: &EvalArgs) -> Result<serde_json::Value> {
        let mut args: Vec<&str> = vec!["eval", "--json"];
        if a.impure {
            args.push("--impure");
        }
        if let Some(e) = &a.expr {
            args.push("--expr");
            args.push(e);
        } else if let Some(i) = &a.installable {
            args.push(i);
        }
        if let Some(ap) = &a.apply {
            args.push("--apply");
            args.push(ap);
        }
        self.run_json(&args)
    }

    fn build_json(&self, installable: &str) -> Result<Vec<BuildResult>> {
        let v = self.run_json(&["build", "--json", "--no-link", installable])?;
        Ok(serde_json::from_value(v)?)
    }

    fn flake_show(&self, flake_ref: &str) -> Result<serde_json::Value> {
        self.run_json(&["flake", "show", "--json", "--legacy", flake_ref])
    }

    fn version(&self) -> String {
        self.version.clone()
    }
}
