//! The on-disk store. Files are canonical; Oxigraph is a rebuildable index.
//!
//! ```text
//! store/
//!   drv/<hash>.nq.zst   out/<hash>.nq.zst   src/<nar>.nq.zst   pin/<nar>.nq.zst
//!   nixpkgs/<rev>.nq.zst   commit/<repo>/<sha>.nq.zst   gen/<hash>.nq.zst
//!   image/<digest>.nq.zst   index/nixpkgs-multiverse-<hash>.nq.zst
//!   derived/<ruleset>/<input>.nq.zst   runs/<ruleset>/<input>.nq.zst
//!   values/<hash>.nq.zst   env/<hash>.nq.zst
//!   k8s/<cluster>/{snapshot,nodes,observed}/<hash>.nq.zst
//!   packs/<content-hash>/   (copied pack contents)
//!   oxigraph/               (the RocksDB index; never load-bearing)
//! ```

use crate::error::{Error, Result};
use crate::fragment::{Fragment, FragmentKind};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct WriteOutcome {
    pub path: PathBuf,
    pub kind: FragmentKind,
    /// false when an identical file already existed (dedupe by identity).
    pub written: bool,
    pub quads: usize,
    pub bytes: u64,
}

impl Store {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;
        Ok(Store { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn oxigraph_dir(&self) -> PathBuf {
        self.root.join("oxigraph")
    }
    pub fn packs_dir(&self) -> PathBuf {
        self.root.join("packs")
    }
    pub fn path_of(&self, kind: &FragmentKind) -> PathBuf {
        self.root.join(kind.relative_path())
    }
    pub fn exists(&self, kind: &FragmentKind) -> bool {
        self.path_of(kind).exists()
    }

    /// Write a fragment unless a file with that identity already exists.
    /// Identity implies content for every kind except `Commit`/`Run`/
    /// `K8sObserved`, which carry timestamps; those are also skipped when
    /// present, because re-running for the same identity is a no-op by design.
    pub fn write(&self, frag: &Fragment) -> Result<WriteOutcome> {
        let path = self.path_of(&frag.kind);
        if path.exists() {
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            return Ok(WriteOutcome {
                path,
                kind: frag.kind.clone(),
                written: false,
                quads: frag.len(),
                bytes,
            });
        }
        frag.write_to(&path)?;
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Ok(WriteOutcome {
            path,
            kind: frag.kind.clone(),
            written: true,
            quads: frag.len(),
            bytes,
        })
    }

    /// Force-write even if present (used by `rebuild`-style refreshes).
    pub fn overwrite(&self, frag: &Fragment) -> Result<WriteOutcome> {
        let path = self.path_of(&frag.kind);
        frag.write_to(&path)?;
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Ok(WriteOutcome {
            path,
            kind: frag.kind.clone(),
            written: true,
            quads: frag.len(),
            bytes,
        })
    }

    /// All fragment files under the store, sorted, excluding the index and packs.
    pub fn all_fragment_paths(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        let skip = [self.oxigraph_dir(), self.packs_dir()];
        for entry in WalkDir::new(&self.root)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| !skip.iter().any(|s| e.path() == s))
        {
            let entry = entry.map_err(|e| Error::Store(e.to_string()))?;
            let p = entry.path();
            if p.is_file() && p.to_string_lossy().ends_with(".nq.zst") {
                out.push(p.to_path_buf());
            }
        }
        out.sort();
        Ok(out)
    }

    /// Fragment paths under one kind directory (e.g. "derived").
    pub fn fragment_paths_under(&self, sub: &str) -> Result<Vec<PathBuf>> {
        let dir = self.root.join(sub);
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for entry in WalkDir::new(&dir).sort_by_file_name() {
            let entry = entry.map_err(|e| Error::Store(e.to_string()))?;
            let p = entry.path();
            if p.is_file() && p.to_string_lossy().ends_with(".nq.zst") {
                out.push(p.to_path_buf());
            }
        }
        out.sort();
        Ok(out)
    }

    /// Relative path (inside the store) of an absolute fragment path.
    pub fn relative(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| path.to_path_buf())
    }
}
