//! Access to Nix. Two back-ends implement [`NixSource`]:
//! - [`CliNix`]: spawns the `nix` CLI (the reference extraction path).
//! - [`RecordedNix`]: replays JSON recorded from the CLI, for tests and the
//!   determinism check (no Nix needed, works inside the Nix build sandbox).
//!
//! Every fragment the extractor writes is a pure function of what a
//! `NixSource` returns, so both back-ends must yield identical fragments.

pub mod cli;
pub mod drv_file;
pub mod model;
pub mod recorded;

pub use cli::CliNix;
pub use model::*;
pub use recorded::RecordedNix;

use crate::error::Result;
use std::collections::BTreeMap;

/// What the extractor needs from Nix. Keep this small: every method maps to
/// one Nix CLI invocation and one recorded JSON file.
pub trait NixSource {
    /// `nix derivation show [-r] <installables...>` → drv path → info.
    fn derivation_show(
        &self,
        installables: &[String],
        recursive: bool,
    ) -> Result<BTreeMap<String, DrvInfo>>;
    /// `nix path-info --json <paths...>` → store path → info (None when not present locally).
    fn path_info(&self, paths: &[String]) -> Result<BTreeMap<String, Option<PathInfo>>>;
    /// `nix flake metadata --json <ref>`.
    fn flake_metadata(&self, flake_ref: &str) -> Result<serde_json::Value>;
    /// `nix eval --json <installable>` or `--expr`.
    fn eval_json(&self, args: &EvalArgs) -> Result<serde_json::Value>;
    /// `nix build --json <installable>` → built outputs (drvPath + outputs).
    fn build_json(&self, installable: &str) -> Result<Vec<BuildResult>>;
    /// `nix flake show --json <ref>` (only the attribute skeleton).
    fn flake_show(&self, flake_ref: &str) -> Result<serde_json::Value>;
    /// `nix --version`, for provenance logs.
    fn version(&self) -> String;
}

#[derive(Debug, Clone, Default)]
pub struct EvalArgs {
    pub installable: Option<String>,
    pub expr: Option<String>,
    pub apply: Option<String>,
    pub impure: bool,
    /// A stable label used by RecordedNix to find the recorded answer.
    pub label: String,
}
