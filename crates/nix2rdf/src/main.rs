//! The nix2rdf command line. Thin: every command maps onto a function in
//! nix2rdf-core and reports what it wrote.

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use nix2rdf_core::extract::drv::{EnvMode, ExtractOptions};
use nix2rdf_core::extract::flake::{CommitInfo, FlakeOptions};
use nix2rdf_core::extract::image::ImageOptions;
use nix2rdf_core::extract::nixos::NixosOptions;
use nix2rdf_core::extract::nixpkgs_eval::EvalOptions;
use nix2rdf_core::extract::nixpkgs_index::{index_fragment, IndexFiles};
use nix2rdf_core::graph::{Graph, QueryOutput};
use nix2rdf_core::nix::{CliNix, NixSource, RecordedNix};
use nix2rdf_core::reason::{self, InputSelection};
use nix2rdf_core::{extract, fragment, iri, k8s, ontology, packs, publish, Fragment, Store};
use oxigraph::sparql::results::QueryResultsFormat;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "nix2rdf",
    version,
    about = "Nix artifacts as a content-addressed RDF dataset"
)]
struct Cli {
    /// The fragment store directory (canonical files + the Oxigraph index).
    #[arg(long, global = true, env = "NIX2RDF_STORE", default_value = "store")]
    store: PathBuf,
    /// Log format: text or json (structured, one line per event).
    #[arg(
        long,
        global = true,
        env = "NIX2RDF_LOG_FORMAT",
        default_value = "text"
    )]
    log_format: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum Reader {
    Cli,
    DrvFiles,
}

#[derive(Args, Clone)]
struct SourceArgs {
    /// Replay recorded Nix output from this directory instead of calling nix.
    #[arg(long)]
    from_recorded: Option<PathBuf>,
    /// Record every Nix answer into this directory (for fixtures).
    #[arg(long)]
    record: Option<PathBuf>,
    /// Emit the full derivation environment as side fragments (env/<hash>).
    #[arg(long, value_parser = ["hash", "full"], default_value = "hash")]
    env: String,
}

#[derive(Args, Clone)]
struct CommitArgs {
    /// Consumer commit SHA (how CI calls it; adds a Commit node on the time axis).
    #[arg(long, requires_all = ["repo", "committed_at"])]
    commit: Option<String>,
    /// Operator-assigned repository id.
    #[arg(long)]
    repo: Option<String>,
    /// RFC 3339 commit timestamp.
    #[arg(long)]
    committed_at: Option<String>,
}

impl CommitArgs {
    fn info(&self) -> Option<CommitInfo> {
        Some(CommitInfo {
            repo: self.repo.clone()?,
            sha: self.commit.clone()?,
            committed_at: self.committed_at.clone()?,
        })
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Flake front-end: lock graph, reachable derivation graph, snapshot (+ commit).
    Flake {
        /// Flake reference (optional with --from-recorded).
        flake_ref: Option<String>,
        /// Attribute path(s) relative to the flake; default packages.<system>.*
        #[arg(long = "attr")]
        attrs: Vec<String>,
        #[arg(long)]
        system: Option<String>,
        /// Record meta.license/homepage/description for the roots.
        #[arg(long)]
        meta: bool,
        #[command(flatten)]
        commit: CommitArgs,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// NixOS front-end: system closure + option tree of <flake>#nixosConfigurations.<name>.
    Nixos {
        installable: String,
        /// Include options no module defined (defaults). ~10x larger.
        #[arg(long)]
        all_options: bool,
        /// Record runtime `references` for the built system closure.
        #[arg(long)]
        runtime_closure: bool,
        #[command(flatten)]
        commit: CommitArgs,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// OCI image front-end: image derivation, runtime closure, layers.
    Image {
        installable: String,
        /// Registry (manifest) digest from the push step, sha256:<hex>.
        #[arg(long)]
        digest: Option<String>,
        #[command(flatten)]
        commit: CommitArgs,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Core extractor only: the derivation graph reachable from installables.
    Drv {
        installables: Vec<String>,
        #[arg(long, value_enum, default_value = "cli")]
        reader: Reader,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// After a build: outputs + runtime references for installables (out/ fragments).
    AttachOutputs {
        installables: Vec<String>,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Layer 0: ingest the nixpkgs-multiverse index (revisions, releases, versions).
    NixpkgsIndex {
        /// Flake reference of the index (prefetched with nix), or use --dir.
        #[arg(long, default_value = "github:fzakaria/nixpkgs-multiverse")]
        from: String,
        /// A local checkout containing revisions.json, releases.json, index/versions.json.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Layer 1 (opt-in): evaluate a full nixpkgs revision with nix-eval-jobs.
    NixpkgsEval {
        rev: String,
        #[arg(long)]
        index_dir: Option<PathBuf>,
        #[arg(long)]
        nar_hash: Option<String>,
        #[arg(long)]
        system: Option<String>,
        #[arg(long)]
        expr: Option<String>,
        #[arg(long, default_value_t = 4)]
        workers: usize,
        #[arg(long, default_value_t = 4096)]
        max_memory_mb: usize,
        #[arg(long)]
        allow_unfree: bool,
        #[arg(long, value_enum, default_value = "drv-files")]
        reader: Reader,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Mirror a published fragment set (URL or directory with manifest.json).
    FetchFragments { url: String },
    /// Publish fragments (all, or the given store-relative paths) to a directory.
    Publish {
        #[arg(long)]
        dest: PathBuf,
        #[arg(long)]
        all: bool,
        paths: Vec<PathBuf>,
    },
    /// Bulk-load fragments into the Oxigraph index.
    Load {
        #[arg(long)]
        all: bool,
        /// Only fragments not loaded before (tracked in oxigraph/.loaded).
        #[arg(long)]
        new: bool,
        paths: Vec<PathBuf>,
    },
    /// Recreate the Oxigraph index from the fragment files.
    Rebuild,
    /// Serve the SPARQL 1.1 endpoint.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7878")]
        listen: String,
    },
    /// Run a SPARQL query file.
    Query {
        file: PathBuf,
        #[arg(long, default_value = "json", value_parser = ["json", "csv", "tsv", "xml"])]
        format: String,
    },
    /// Run an ASK; exit non-zero when it is true (for CI).
    Check { file: PathBuf },
    /// Run rule packs over selected graphs; write and load the derived graph.
    Reason {
        #[arg(long = "pack", required = true)]
        packs: Vec<String>,
        #[arg(long = "packs")]
        pack_dirs: Vec<PathBuf>,
        /// Fragment paths (store-relative or absolute).
        #[arg(long = "input")]
        inputs: Vec<PathBuf>,
        /// Snapshot IRIs (pin, generation, image, commit, k8s snapshot).
        #[arg(long = "root")]
        roots: Vec<String>,
        /// Follow closureContains/hasRoot/inputSrc/runsImage/hostGeneration to their fragments.
        #[arg(long)]
        input_closure: bool,
        /// Every snapshot fragment in the store.
        #[arg(long)]
        all_snapshots: bool,
        /// Everything in the store as one input set.
        #[arg(long)]
        everything: bool,
        /// Re-run even if the derived graph exists.
        #[arg(long)]
        force: bool,
        /// Do not load into Oxigraph (only write files).
        #[arg(long)]
        no_load: bool,
    },
    /// Delete derived graphs produced by rulesets other than the given packs' current one.
    GcDerived {
        #[arg(long = "pack")]
        packs: Vec<String>,
        #[arg(long = "packs")]
        pack_dirs: Vec<PathBuf>,
    },
    /// Rule pack tooling.
    Pack {
        #[command(subcommand)]
        cmd: PackCmd,
    },
    /// Ontology tooling.
    Ontology {
        #[command(subcommand)]
        cmd: OntologyCmd,
    },
    /// Kubernetes / k3s front-end.
    K8s {
        #[command(subcommand)]
        cmd: K8sCmd,
    },
    /// Disk usage per fragment kind.
    Stats,
    /// Debug: run a complete Nemo program file and print the rows of new/3.
    #[command(hide = true)]
    NemoRun { file: PathBuf },
}

#[derive(Subcommand)]
enum PackCmd {
    List {
        #[arg(long = "packs")]
        pack_dirs: Vec<PathBuf>,
    },
    /// Run fixture tests (tests/<case>/{input,expected,forbidden}.nq).
    Test {
        #[arg(long = "packs")]
        pack_dirs: Vec<PathBuf>,
        #[arg(long)]
        all: bool,
        names: Vec<String>,
    },
    /// Parse and validate the combined program of the packs.
    Check {
        #[arg(long = "packs")]
        pack_dirs: Vec<PathBuf>,
        names: Vec<String>,
    },
}

#[derive(Subcommand)]
enum OntologyCmd {
    /// Parse the vocabularies; verify every term used in Rust and packs is declared.
    Validate {
        #[arg(long = "packs")]
        pack_dirs: Vec<PathBuf>,
        ttl: Vec<PathBuf>,
    },
    /// Generate documentation (HTML for one file, Markdown for several).
    Doc {
        #[arg(long)]
        html: bool,
        #[arg(long)]
        markdown: bool,
        ttl: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum K8sCmd {
    /// Snapshot a cluster (live via API, or desired via manifests).
    Snapshot {
        #[arg(long)]
        cluster_id: String,
        #[arg(long, conflicts_with = "manifests")]
        context: Option<String>,
        #[arg(long)]
        manifests: Option<PathBuf>,
        /// Read the API server (default when --manifests is absent).
        #[arg(long)]
        live: bool,
        /// JSON or "name=hash" lines mapping node names to NixOS generation hashes.
        #[arg(long)]
        nixos_node_map: Option<PathBuf>,
        /// RFC 3339 timestamp of the observation (default: now).
        #[arg(long)]
        observed_at: Option<String>,
        #[arg(long = "owner-key")]
        owner_keys: Vec<String>,
        #[arg(long)]
        rack_label: Option<String>,
        #[arg(long)]
        site_label: Option<String>,
        #[command(flatten)]
        commit: CommitArgs,
    },
}

fn init_logging(format: &str) {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if format == "json" {
        fmt()
            .json()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    } else {
        fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }
}

/// Pick the Nix back-end; wrap in a recorder when --record is given.
fn nix_source(src: &SourceArgs) -> Result<Box<dyn NixSource>> {
    if let Some(d) = &src.from_recorded {
        return Ok(Box::new(RecordedNix::new(d)));
    }
    let cli = CliNix::new();
    if let Some(d) = &src.record {
        return Ok(Box::new(Recording {
            inner: cli,
            dir: d.clone(),
        }));
    }
    Ok(Box::new(cli))
}

/// A NixSource that forwards to the CLI and writes every answer to a fixture dir.
struct Recording {
    inner: CliNix,
    dir: PathBuf,
}

impl NixSource for Recording {
    fn derivation_show(
        &self,
        installables: &[String],
        recursive: bool,
    ) -> nix2rdf_core::Result<BTreeMap<String, nix2rdf_core::nix::DrvInfo>> {
        // Record Nix's raw JSON (whatever schema version this Nix prints), not
        // the normalized form, so replaying exercises the same parsing path.
        let mut args: Vec<&str> = vec!["derivation", "show"];
        if recursive {
            args.push("-r");
        }
        args.extend(installables.iter().map(|s| s.as_str()));
        let raw = self.inner.run_json(&args)?;
        let name = if recursive {
            "derivation-show.json"
        } else {
            "derivation-show-roots.json"
        };
        if recursive || !self.dir.join(name).exists() {
            RecordedNix::write_json(&self.dir, name, &raw)?;
        }
        Ok(nix2rdf_core::nix::model::parse_derivation_show(raw)?)
    }
    fn path_info(
        &self,
        paths: &[String],
    ) -> nix2rdf_core::Result<BTreeMap<String, Option<nix2rdf_core::nix::PathInfo>>> {
        let r = self.inner.path_info(paths)?;
        let p = self.dir.join("path-info.json");
        let mut all: BTreeMap<String, Option<nix2rdf_core::nix::PathInfo>> = if p.exists() {
            serde_json::from_slice(&std::fs::read(&p).unwrap_or_default()).unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        all.extend(r.clone());
        RecordedNix::write_json(&self.dir, "path-info.json", &serde_json::to_value(&all)?)?;
        Ok(r)
    }
    fn flake_metadata(&self, flake_ref: &str) -> nix2rdf_core::Result<serde_json::Value> {
        let r = self.inner.flake_metadata(flake_ref)?;
        RecordedNix::write_json(&self.dir, "flake-metadata.json", &r)?;
        Ok(r)
    }
    fn eval_json(
        &self,
        a: &nix2rdf_core::nix::EvalArgs,
    ) -> nix2rdf_core::Result<serde_json::Value> {
        let r = self.inner.eval_json(a)?;
        RecordedNix::write_json(&self.dir, &format!("eval/{}.json", a.label), &r)?;
        Ok(r)
    }
    fn build_json(
        &self,
        installable: &str,
    ) -> nix2rdf_core::Result<Vec<nix2rdf_core::nix::BuildResult>> {
        let r = self.inner.build_json(installable)?;
        RecordedNix::write_json(&self.dir, "build.json", &serde_json::to_value(&r)?)?;
        Ok(r)
    }
    fn flake_show(&self, flake_ref: &str) -> nix2rdf_core::Result<serde_json::Value> {
        let r = self.inner.flake_show(flake_ref)?;
        RecordedNix::write_json(&self.dir, "flake-show.json", &r)?;
        Ok(r)
    }
    fn version(&self) -> String {
        self.inner.version()
    }
}

fn extract_opts(src: &SourceArgs) -> ExtractOptions {
    ExtractOptions {
        env: if src.env == "full" {
            EnvMode::Full
        } else {
            EnvMode::HashOnly
        },
    }
}

fn write_and_report(store: &Store, frags: &[Fragment], label: &str) -> Result<()> {
    let outcomes = extract::write_all(store, frags, label)?;
    let new = outcomes.iter().filter(|o| o.written).count();
    let bytes: u64 = outcomes.iter().filter(|o| o.written).map(|o| o.bytes).sum();
    let mut by_kind: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for o in &outcomes {
        let e = by_kind.entry(o.kind.kind_name()).or_default();
        e.0 += 1;
        if o.written {
            e.1 += 1;
        }
    }
    println!(
        "{label}: {} fragments ({new} new, {} KiB written)",
        outcomes.len(),
        bytes / 1024
    );
    for (k, (total, newc)) in by_kind {
        println!("  {k:<14} {total:>7} total {newc:>7} new");
    }
    Ok(())
}

fn pack_dirs_or_default(dirs: &[PathBuf]) -> Vec<PathBuf> {
    if dirs.is_empty() {
        packs::default_pack_dirs()
    } else {
        dirs.to_vec()
    }
}

fn parse_node_map(p: &PathBuf) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
    if text.trim_start().starts_with('{') {
        return Ok(serde_json::from_str(&text)?);
    }
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            m.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    Ok(m)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_logging(&cli.log_format);
    let store = Store::open(&cli.store)?;

    match cli.cmd {
        Cmd::Flake {
            flake_ref,
            attrs,
            system,
            meta,
            commit,
            source,
        } => {
            let nix = nix_source(&source)?;
            let flake_ref = match (flake_ref, &source.from_recorded) {
                (Some(r), _) => r,
                (None, Some(_)) => "recorded".to_string(),
                (None, None) => {
                    bail!("a flake reference is required unless --from-recorded is given")
                }
            };
            // Accept `.#`, `.#hello` and `github:o/r#pkg`: the fragment is an attribute.
            let (flake_ref, mut attrs) = match flake_ref.split_once('#') {
                Some((r, frag)) => {
                    let mut attrs = attrs;
                    if !frag.is_empty() {
                        attrs.push(frag.to_string());
                    }
                    (r.to_string(), attrs)
                }
                None => (flake_ref, attrs),
            };
            if flake_ref.is_empty() {
                bail!("empty flake reference");
            }
            attrs.sort();
            attrs.dedup();
            let opts = FlakeOptions {
                flake_ref,
                attrs,
                system: system.unwrap_or_else(extract::flake::current_system),
                commit: commit.info(),
                with_meta: meta,
                extract: extract_opts(&source),
            };
            let ex = extract::flake::extract_flake(nix.as_ref(), &opts)?;
            write_and_report(&store, &ex.fragments, "flake")?;
            println!("snapshot: {}", ex.snapshot);
        }
        Cmd::Nixos {
            installable,
            all_options,
            runtime_closure,
            commit,
            source,
        } => {
            let (flake_ref, attr) = installable
                .split_once('#')
                .context("expected <flake>#nixosConfigurations.<name>")?;
            let name = attr
                .strip_prefix("nixosConfigurations.")
                .context("expected nixosConfigurations.<name>")?;
            let nix = nix_source(&source)?;
            let opts = NixosOptions {
                flake_ref: flake_ref.into(),
                name: name.into(),
                all_options,
                with_runtime_closure: runtime_closure,
                extract: extract_opts(&source),
            };
            let mut frags = extract::nixos::extract_nixos(nix.as_ref(), &opts)?;
            if let Some(c) = commit.info() {
                if let Some(g) = frags.last().map(|f| f.graph()) {
                    frags.push(extract::flake::commit_fragment(&c, &g));
                }
            }
            write_and_report(&store, &frags, "nixos")?;
        }
        Cmd::Image {
            installable,
            digest,
            commit,
            source,
        } => {
            let (flake_ref, attr) = installable
                .split_once('#')
                .context("expected <flake>#<attr>")?;
            let nix = nix_source(&source)?;
            let opts = ImageOptions {
                flake_ref: flake_ref.into(),
                attr: attr.into(),
                digest,
                extract: extract_opts(&source),
            };
            let (mut frags, img) = extract::image::extract_image(nix.as_ref(), &opts)?;
            if let Some(c) = commit.info() {
                frags.push(extract::flake::commit_fragment(&c, &img));
            }
            write_and_report(&store, &frags, "image")?;
            println!("image: {img}");
        }
        Cmd::Drv {
            installables,
            reader,
            source,
        } => {
            let nix = nix_source(&source)?;
            let opts = extract_opts(&source);
            let ex = match reader {
                Reader::Cli => extract::extract_graph(nix.as_ref(), &installables, &opts)?,
                Reader::DrvFiles => {
                    let drvs = nix2rdf_core::nix::drv_file::read_closure(&installables)?;
                    extract::drv::extract_from_drvs(nix.as_ref(), drvs, &opts)?
                }
            };
            write_and_report(&store, &ex.fragments, "drv")?;
        }
        Cmd::AttachOutputs {
            installables,
            source,
        } => {
            let nix = nix_source(&source)?;
            let mut built = Vec::new();
            for i in &installables {
                built.extend(nix.build_json(i)?);
            }
            let frags = extract::drv::extract_outputs(nix.as_ref(), &built)?;
            write_and_report(&store, &frags, "outputs")?;
        }
        Cmd::NixpkgsIndex { from, dir } => {
            let dir = match dir {
                Some(d) => d,
                None => {
                    let nix = CliNix::new();
                    let v = nix.run_json(&["flake", "prefetch", "--json", &from])?;
                    PathBuf::from(
                        v.get("storePath")
                            .and_then(|s| s.as_str())
                            .context("nix flake prefetch gave no storePath")?,
                    )
                }
            };
            let files = IndexFiles::read(&dir)?;
            let frag = index_fragment(&files)?;
            write_and_report(&store, &[frag], "nixpkgs-index")?;
        }
        Cmd::NixpkgsEval {
            rev,
            index_dir,
            nar_hash,
            system,
            expr,
            workers,
            max_memory_mb,
            allow_unfree,
            reader,
            source,
        } => {
            let revision = match &index_dir {
                Some(d) => extract::nixpkgs_index::find_revision(&IndexFiles::read(d)?, &rev)?
                    .context("revision not in index")?,
                None => extract::nixpkgs_index::Revision {
                    rev: rev.clone(),
                    date: String::new(),
                    name: String::new(),
                    nar_hash: nar_hash.clone().unwrap_or_default(),
                },
            };
            let opts = EvalOptions {
                revision,
                system: system.unwrap_or_else(extract::flake::current_system),
                expr,
                workers,
                max_memory_mb,
                allow_unfree,
                drv_files: matches!(reader, Reader::DrvFiles),
                extract: extract_opts(&source),
            };
            let nix = CliNix::new();
            let frags = extract::nixpkgs_eval::extract_nixpkgs(&nix, &opts)?;
            write_and_report(&store, &frags, "nixpkgs-eval")?;
        }
        Cmd::FetchFragments { url } => {
            let n = publish::fetch(&store, &url)?;
            println!("fetched {n} new fragments from {url}");
        }
        Cmd::Publish { dest, all, paths } => {
            let paths = if all {
                store.all_fragment_paths()?
            } else {
                paths
            };
            let m = publish::publish(&store, &paths, &dest)?;
            println!(
                "published {} fragments to {}",
                m.fragments.len(),
                dest.display()
            );
        }
        Cmd::Load { all, new, paths } => {
            let g = Graph::open(&store)?;
            let loaded_marker = store.oxigraph_dir().join(".loaded");
            let mut already: std::collections::BTreeSet<String> =
                std::fs::read_to_string(&loaded_marker)
                    .map(|t| t.lines().map(String::from).collect())
                    .unwrap_or_default();
            let candidates: Vec<PathBuf> = if all || new {
                store.all_fragment_paths()?
            } else {
                paths
            };
            let todo: Vec<PathBuf> = if new {
                candidates
                    .into_iter()
                    .filter(|p| !already.contains(&store.relative(p).to_string_lossy().to_string()))
                    .collect()
            } else {
                candidates
            };
            let n = g.load_paths(&todo)?;
            for p in &todo {
                already.insert(store.relative(p).to_string_lossy().to_string());
            }
            std::fs::write(
                &loaded_marker,
                already.iter().map(|s| format!("{s}\n")).collect::<String>(),
            )?;
            println!(
                "loaded {} fragments ({n} quads); store now has {} quads",
                todo.len(),
                g.len()?
            );
        }
        Cmd::Rebuild => {
            let g = Graph::rebuild(&store)?;
            let all: Vec<String> = store
                .all_fragment_paths()?
                .iter()
                .map(|p| store.relative(p).to_string_lossy().to_string())
                .collect();
            std::fs::write(store.oxigraph_dir().join(".loaded"), all.join("\n") + "\n")?;
            println!("rebuilt: {} quads from {} fragments", g.len()?, all.len());
        }
        Cmd::Serve { listen } => {
            let g = Graph::open(&store)?;
            serve(g, &listen)?;
        }
        Cmd::Query { file, format } => {
            let q = std::fs::read_to_string(&file)?;
            let g = Graph::open_read_only(&store).or_else(|_| Graph::open(&store))?;
            let fmt = match format.as_str() {
                "csv" => QueryResultsFormat::Csv,
                "tsv" => QueryResultsFormat::Tsv,
                "xml" => QueryResultsFormat::Xml,
                _ => QueryResultsFormat::Json,
            };
            match g.query(&q, fmt)? {
                QueryOutput::Boolean(b) => println!("{b}"),
                QueryOutput::Text(t) => {
                    use std::io::Write;
                    std::io::stdout().write_all(&t)?;
                }
            }
        }
        Cmd::Check { file } => {
            let q = std::fs::read_to_string(&file)?;
            let g = Graph::open_read_only(&store).or_else(|_| Graph::open(&store))?;
            let hit = g.ask(&q)?;
            if hit {
                eprintln!("check FAILED: {} matched", file.display());
                std::process::exit(1);
            }
            println!("check ok: {}", file.display());
        }
        Cmd::Reason {
            packs: sel,
            pack_dirs,
            inputs,
            roots,
            input_closure,
            all_snapshots,
            everything,
            force,
            no_load,
        } => {
            let all = packs::discover(&pack_dirs_or_default(&pack_dirs))?;
            let order = packs::resolve(&all, &sel)?;
            let roots: Vec<oxrdf::NamedNode> = roots
                .iter()
                .map(|r| oxrdf::NamedNode::new(r.clone()))
                .collect::<std::result::Result<_, _>>()?;
            let selection = InputSelection {
                paths: inputs,
                roots,
                closure: input_closure,
                all_snapshots,
                everything,
            };
            let ins = reason::select_inputs(&store, &selection)?;
            if ins.is_empty() {
                bail!("no input graphs selected");
            }
            let r = reason::run(&store, &order, &ins, force)?;
            let graph = if no_load {
                None
            } else {
                Some(Graph::open(&store)?)
            };
            reason::persist(&store, graph.as_ref(), &r)?;
            println!(
                "derived graph {} ({} quads, {} inputs){}",
                r.derived.graph(),
                r.derived.len(),
                r.inputs,
                if r.already_existed {
                    " [already existed]"
                } else {
                    ""
                }
            );
        }
        Cmd::GcDerived {
            packs: sel,
            pack_dirs,
        } => {
            let all = packs::discover(&pack_dirs_or_default(&pack_dirs))?;
            let keep = if sel.is_empty() {
                vec![]
            } else {
                vec![packs::ruleset_hash(&packs::resolve(&all, &sel)?)]
            };
            let g = Graph::open(&store).ok();
            let n = reason::gc_derived(&store, g.as_ref(), &keep)?;
            println!("removed {n} derived/run fragments");
        }
        Cmd::Pack { cmd } => match cmd {
            PackCmd::List { pack_dirs } => {
                for p in packs::discover(&pack_dirs_or_default(&pack_dirs))?.values() {
                    println!(
                        "{:<18} {:<8} {}  deps={:?}  {}",
                        p.manifest.name,
                        p.manifest.version,
                        &p.content_hash[..16],
                        p.manifest.depends_on,
                        p.manifest.description
                    );
                }
            }
            PackCmd::Test {
                pack_dirs,
                all,
                names,
            } => {
                let allp = packs::discover(&pack_dirs_or_default(&pack_dirs))?;
                let names: Vec<String> = if all {
                    allp.keys().cloned().collect()
                } else {
                    names
                };
                let mut failures = 0;
                let mut total = 0;
                for n in &names {
                    let p = allp.get(n).with_context(|| format!("unknown pack {n}"))?;
                    for t in packs::tests_of(p)? {
                        total += 1;
                        match run_pack_test(&allp, n, &t) {
                            Ok(()) => println!("ok   {n}/{}", t.name),
                            Err(e) => {
                                failures += 1;
                                println!("FAIL {n}/{}\n{e:#}", t.name);
                            }
                        }
                    }
                }
                println!("{total} tests, {failures} failures");
                if failures > 0 {
                    std::process::exit(1);
                }
            }
            PackCmd::Check { pack_dirs, names } => {
                let allp = packs::discover(&pack_dirs_or_default(&pack_dirs))?;
                let order = packs::resolve(&allp, &names)?;
                let tmp = tempfile::tempdir()?;
                let empty = tmp.path().join("empty.nq");
                std::fs::write(&empty, "")?;
                let prog = reason::build_program(&order, &empty)?;
                nix2rdf_core::nemo_engine::validate_program(&prog)?;
                println!(
                    "ok: {} packs, ruleset {}",
                    order.len(),
                    packs::ruleset_hash(&order)
                );
            }
        },
        Cmd::Ontology { cmd } => match cmd {
            OntologyCmd::Validate { pack_dirs, ttl } => {
                let problems = ontology::validate(&ttl, &pack_dirs_or_default(&pack_dirs))?;
                for p in &problems {
                    println!("{p}");
                }
                if !problems.is_empty() {
                    bail!("{} ontology problems", problems.len());
                }
                println!("ontology ok: {} files, all terms declared", ttl.len());
            }
            OntologyCmd::Doc {
                html,
                markdown,
                ttl,
            } => {
                let vocabs: Vec<ontology::Vocabulary> = ttl
                    .iter()
                    .map(|p| ontology::load_file(p))
                    .collect::<nix2rdf_core::Result<_>>()?;
                if html && !markdown {
                    let v = vocabs.first().context("need one ttl for --html")?;
                    print!("{}", ontology::html(v));
                } else {
                    print!("{}", ontology::markdown(&vocabs));
                }
            }
        },
        Cmd::K8s { cmd } => match cmd {
            K8sCmd::Snapshot {
                cluster_id,
                context,
                manifests,
                live: _,
                nixos_node_map,
                observed_at,
                owner_keys,
                rack_label,
                site_label,
                commit,
            } => {
                let mut labels = k8s::LabelConfig::default();
                if !owner_keys.is_empty() {
                    labels.owner_keys = owner_keys;
                }
                if let Some(r) = rack_label {
                    labels.rack_key = r;
                }
                if let Some(s) = site_label {
                    labels.site_key = s;
                }
                let node_map = match &nixos_node_map {
                    Some(p) => parse_node_map(p)?,
                    None => BTreeMap::new(),
                };
                let (input, state) = match &manifests {
                    Some(dir) => (
                        k8s::SnapshotInput {
                            objects: k8s::manifests::read_dir(dir)?,
                            pods: vec![],
                            failed_kinds: vec![],
                        },
                        k8s::snapshot::StateKind::Desired,
                    ),
                    None => {
                        let live = k8s::api::list_live(context.as_deref())?;
                        (
                            k8s::SnapshotInput {
                                objects: live.objects,
                                pods: live.pods,
                                failed_kinds: live.failed_kinds,
                            },
                            k8s::snapshot::StateKind::Live,
                        )
                    }
                };
                let opts = k8s::SnapshotOptions {
                    cluster_id,
                    state,
                    labels,
                    node_map,
                    observed_at,
                };
                let out = k8s::build_snapshot(&input, &opts);
                let mut frags = vec![out.snapshot, out.nodes, out.observed];
                if let Some(c) = commit.info() {
                    frags.push(extract::flake::commit_fragment(&c, &out.snapshot_iri));
                }
                write_and_report(&store, &frags, "k8s-snapshot")?;
                println!("snapshot: {}", out.snapshot_iri);
            }
        },
        Cmd::NemoRun { file } => {
            let prog = std::fs::read_to_string(&file)?;
            let out = nix2rdf_core::nemo_engine::run_program(&prog)?;
            println!(
                "derived facts: {}, new triples: {}",
                out.derived_facts,
                out.triples.len()
            );
            for t in out.triples.iter().take(50) {
                println!("{:?}", t);
            }
        }
        Cmd::Stats => {
            let mut by_kind: BTreeMap<String, (usize, u64)> = BTreeMap::new();
            for p in store.all_fragment_paths()? {
                let rel = store.relative(&p);
                let kind = rel
                    .components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .unwrap_or_default();
                let e = by_kind.entry(kind).or_default();
                e.0 += 1;
                e.1 += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            }
            let mut total = (0usize, 0u64);
            for (k, (n, b)) in &by_kind {
                println!("{k:<14} {n:>8} files {:>10} KiB", b / 1024);
                total.0 += n;
                total.1 += b;
            }
            println!(
                "{:<14} {:>8} files {:>10} KiB",
                "total",
                total.0,
                total.1 / 1024
            );
            let ox = store.oxigraph_dir();
            if ox.exists() {
                let mut b = 0u64;
                for e in walkdir::WalkDir::new(&ox)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    b += e.metadata().map(|m| m.len()).unwrap_or(0);
                }
                println!("{:<14} {:>8}       {:>10} KiB", "oxigraph", "", b / 1024);
            }
        }
    }
    Ok(())
}

/// Run one pack test: derive over input.nq with the pack (+deps); every
/// expected triple must be derived (blank nodes are wildcards), no forbidden one.
fn run_pack_test(
    all: &BTreeMap<String, packs::Pack>,
    name: &str,
    t: &packs::PackTest,
) -> Result<()> {
    let order = packs::resolve(all, &[name.to_string()])?;
    let tmp = tempfile::tempdir()?;
    let store = Store::open(tmp.path())?;
    let mut by_graph: BTreeMap<String, String> = BTreeMap::new();
    for q in fragment::parse_nquads(t.input.as_bytes())? {
        let g = fragment::graph_of(&q)
            .map(|g| g.as_str().to_string())
            .unwrap_or_else(|| iri::fragment("test", "input").as_str().to_string());
        let line = format!("{} {} {} <{g}> .\n", q.subject, q.predicate, q.object);
        by_graph.entry(g).or_default().push_str(&line);
    }
    let inputs: Vec<reason::InputGraph> = by_graph
        .into_iter()
        .map(|(g, nq)| reason::InputGraph {
            graph: oxrdf::NamedNode::new_unchecked(g),
            path: PathBuf::new(),
            nquads: nq,
        })
        .collect();
    let r = reason::run(&store, &order, &inputs, true)?;
    let derived: Vec<String> = r
        .derived
        .to_nquads()
        .lines()
        .map(|l| l.rsplitn(3, ' ').last().unwrap_or(l).to_string())
        .collect();
    let matches = |pattern: &oxrdf::Quad| -> bool {
        let s = match &pattern.subject {
            oxrdf::NamedOrBlankNode::BlankNode(_) => None,
            other => Some(other.to_string()),
        };
        let o = match &pattern.object {
            oxrdf::Term::BlankNode(_) => None,
            other => Some(other.to_string()),
        };
        let p = pattern.predicate.to_string();
        derived.iter().any(|line| {
            let mut it = line.splitn(3, ' ');
            let (ls, lp, lo) = (
                it.next().unwrap_or(""),
                it.next().unwrap_or(""),
                it.next().unwrap_or(""),
            );
            lp == p
                && s.as_deref().map(|x| x == ls).unwrap_or(true)
                && o.as_deref().map(|x| x == lo).unwrap_or(true)
        })
    };
    let mut missing = Vec::new();
    for q in fragment::parse_nquads(t.expected.as_bytes())? {
        if !matches(&q) {
            missing.push(format!("{} {} {}", q.subject, q.predicate, q.object));
        }
    }
    let mut present = Vec::new();
    if let Some(f) = &t.forbidden {
        for q in fragment::parse_nquads(f.as_bytes())? {
            if matches(&q) {
                present.push(format!("{} {} {}", q.subject, q.predicate, q.object));
            }
        }
    }
    if missing.is_empty() && present.is_empty() {
        return Ok(());
    }
    let mut msg = String::new();
    if !missing.is_empty() {
        msg.push_str(&format!(
            "  expected but not derived:\n    {}\n",
            missing.join("\n    ")
        ));
    }
    if !present.is_empty() {
        msg.push_str(&format!(
            "  forbidden but derived:\n    {}\n",
            present.join("\n    ")
        ));
    }
    msg.push_str(&format!(
        "  derived {} triples; program written to {}\n",
        derived.len(),
        t.dir.display()
    ));
    bail!("{msg}")
}

/// A minimal SPARQL 1.1 Protocol endpoint: GET/POST /query (and /sparql).
fn serve(g: Graph, listen: &str) -> Result<()> {
    use axum::{
        extract::{Query, State},
        http::{header, HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use std::sync::Arc;

    #[derive(serde::Deserialize)]
    struct Q {
        query: Option<String>,
    }
    async fn handle(
        State(g): State<Arc<Graph>>,
        headers: HeaderMap,
        Query(q): Query<Q>,
        body: String,
    ) -> impl IntoResponse {
        let ct = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let query = if let Some(q) = q.query {
            q
        } else if ct.starts_with("application/sparql-query") {
            body
        } else if ct.starts_with("application/x-www-form-urlencoded") {
            serde_urlencoded_get(&body, "query").unwrap_or_default()
        } else {
            body
        };
        if query.trim().is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "text/plain")],
                "missing query".to_string().into_bytes(),
            );
        }
        let accept = headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let (fmt, mime) = if accept.contains("text/csv") {
            (QueryResultsFormat::Csv, "text/csv")
        } else if accept.contains("text/tab-separated-values") {
            (QueryResultsFormat::Tsv, "text/tab-separated-values")
        } else if accept.contains("application/sparql-results+xml") {
            (QueryResultsFormat::Xml, "application/sparql-results+xml")
        } else {
            (QueryResultsFormat::Json, "application/sparql-results+json")
        };
        let g2 = g.clone();
        let res = tokio::task::spawn_blocking(move || g2.query(&query, fmt)).await;
        match res {
            Ok(Ok(QueryOutput::Boolean(b))) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime)],
                format!("{{\"head\":{{}},\"boolean\":{b}}}").into_bytes(),
            ),
            Ok(Ok(QueryOutput::Text(t))) => (StatusCode::OK, [(header::CONTENT_TYPE, mime)], t),
            Ok(Err(e)) => (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "text/plain")],
                e.to_string().into_bytes(),
            ),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain")],
                e.to_string().into_bytes(),
            ),
        }
    }
    fn serde_urlencoded_get(body: &str, key: &str) -> Option<String> {
        for pair in body.split('&') {
            let (k, v) = pair.split_once('=')?;
            if k == key {
                return Some(percent_decode(&v.replace('+', " ")));
            }
        }
        None
    }
    fn percent_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() + 1 && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).to_string()
    }
    async fn index() -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, "text/html")],
            "<!doctype html><title>nix2rdf</title><h1>nix2rdf SPARQL endpoint</h1><p>GET or POST <code>/query?query=...</code> (also <code>/sparql</code>). Accept: application/sparql-results+json (default), text/csv, text/tab-separated-values, application/sparql-results+xml.</p><form method=post action=/query><textarea name=query rows=12 cols=100>SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g ORDER BY DESC(?n) LIMIT 20</textarea><br><button>Run</button></form>",
        )
    }
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let state = Arc::new(g);
        let app = Router::new()
            .route("/", get(index))
            .route("/query", get(handle).post(handle))
            .route("/sparql", get(handle).post(handle))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(listen).await?;
        info!(target: "nix2rdf::serve", listen, "SPARQL endpoint listening");
        println!("listening on http://{listen}/query");
        axum::serve(listener, app).await?;
        Ok::<(), anyhow::Error>(())
    })?;
    warn!("server stopped");
    Ok(())
}
