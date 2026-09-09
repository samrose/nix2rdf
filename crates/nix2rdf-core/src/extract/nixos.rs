//! NixOS front-end: the system closure of `nixosConfigurations.<name>` plus
//! the option tree at v1 provenance depth: final value and every definition
//! with its file location. Large values go to `values/<hash>` side fragments.

use super::drv::{extract_graph, extract_runtime_closure, ExtractOptions};
use crate::error::{Error, Result};
use crate::fragment::{Fragment, FragmentKind};
use crate::hash;
use crate::iri;
use crate::nix::{EvalArgs, NixSource};
use crate::vocab::nix_terms as t;
use oxrdf::NamedNode;
use serde::Deserialize;
use tracing::{info, warn};

/// Values larger than this (bytes of canonical JSON) are stored by hash.
pub const VALUE_INLINE_THRESHOLD: usize = 1024;

#[derive(Debug, Clone)]
pub struct NixosOptions {
    pub flake_ref: String,
    pub name: String,
    /// Include options that no module defined (defaults). Off by default: it
    /// multiplies the fragment size by ~10 with little provenance value.
    pub all_options: bool,
    pub with_runtime_closure: bool,
    pub extract: ExtractOptions,
}

#[derive(Debug, Deserialize)]
struct OptRecord {
    path: String,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    declarations: Vec<String>,
    #[serde(default)]
    is_defined: bool,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    definitions: Vec<DefRecord>,
}

#[derive(Debug, Deserialize)]
struct DefRecord {
    file: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

/// The Nix expression that flattens `options` into a list of records. It
/// uses builtins only, so it works against any nixpkgs revision.
pub fn option_walker(all: bool) -> String {
    let filter = if all { "true" } else { "o.isDefined" };
    format!(
        r#"options: let
  isOpt = v: builtins.isAttrs v && (v._type or null) == "option";
  tryJson = v: let r = builtins.tryEval (builtins.deepSeq v (builtins.toJSON v)); in if r.success then builtins.fromJSON r.value else null;
  render = o: {{
    path = builtins.concatStringsSep "." o.loc;
    type = o.type.description or null;
    declarations = map toString (o.declarations or []);
    is_defined = o.isDefined;
    value = tryJson o.value;
    definitions = map (d: {{ file = toString d.file; value = tryJson d.value; }}) (o.definitionsWithLocations or []);
  }};
  walk = set: builtins.concatLists (map (n:
      let v = set.${{n}}; in
      if builtins.substring 0 1 n == "_" then []
      else if isOpt v then (let o = v; in if {filter} then [ (render o) ] else [])
      else if builtins.isAttrs v then walk v
      else []) (builtins.attrNames set));
in walk options"#
    )
}

fn value_literal(f: &mut Fragment, subject: &NamedNode, prop: NamedNode, v: &serde_json::Value, side: &mut Vec<Fragment>) {
    let json = hash::canonical_json(v);
    if json.len() <= VALUE_INLINE_THRESHOLD {
        f.add_str(subject.clone(), prop, &json);
    } else {
        let h = hash::sha256_hex(json.as_bytes());
        f.add_str(subject.clone(), t::valueHash(), &h);
        let vn = iri::value(&h);
        let mut vf = Fragment::new(FragmentKind::Value(h.clone()));
        vf.add_str(vn, t::value(), &json);
        side.push(vf);
    }
}

pub fn extract_nixos(nix: &dyn NixSource, opts: &NixosOptions) -> Result<Vec<Fragment>> {
    let cfg_attr = format!("nixosConfigurations.{}", opts.name);
    let toplevel = format!("{}#{cfg_attr}.config.system.build.toplevel", opts.flake_ref);
    let meta = nix.flake_metadata(&opts.flake_ref)?;
    let root_nar = meta
        .get("locked")
        .and_then(|l| l.get("narHash"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| Error::Other("flake metadata has no locked.narHash".into()))?
        .to_string();

    let graph = extract_graph(nix, &[toplevel.clone()], &opts.extract)?;
    let roots = nix.derivation_show(&[toplevel.clone()], false)?;
    let (top_path, top) = roots.iter().next().ok_or_else(|| Error::Other("toplevel has no derivation".into()))?;
    let top_iri = graph.drv_iri(top_path).ok_or_else(|| Error::Other("toplevel not in graph".into()))?;
    // Generation identity: the toplevel output path hash (what /run/current-system
    // points at); for a content-addressed toplevel fall back to the drv hash.
    let gen_hash = top
        .outputs
        .get("out")
        .and_then(|o| o.path.as_deref())
        .and_then(iri::store_path_hash)
        .map(String::from)
        .unwrap_or_else(|| graph.drv_hashes[top_path].clone());
    info!(target: "nix2rdf::nixos", configuration = %opts.name, generation = %gen_hash, derivations = graph.drv_hashes.len(), "system closure extracted");

    let gen = iri::gen(&gen_hash);
    let mut f = Fragment::new(FragmentKind::Gen(gen_hash.clone()));
    f.add_type(gen.clone(), t::NixosGeneration());
    f.add_type(gen.clone(), t::Snapshot());
    f.add(gen.clone(), t::hasRoot(), top_iri.clone());
    f.add(gen.clone(), t::fromPin(), iri::pin(&root_nar));
    if let Some(p) = top.outputs.get("out").and_then(|o| o.path.as_deref()) {
        f.add_str(gen.clone(), t::storePath(), p);
    }
    let cfg = iri::nixos_configuration(&root_nar, &opts.name);
    f.add_type(cfg.clone(), t::NixosConfiguration());
    f.add_str(cfg.clone(), t::name(), &opts.name);
    f.add(cfg.clone(), t::fromPin(), iri::pin(&root_nar));
    f.add(gen.clone(), t::configuration(), cfg);
    for h in graph.drv_hashes.values() {
        f.add(gen.clone(), t::closureContains(), iri::drv(h));
    }

    // Options.
    let args = EvalArgs {
        installable: Some(format!("{}#{cfg_attr}.options", opts.flake_ref)),
        apply: Some(option_walker(opts.all_options)),
        label: format!("options/{}", iri::encode_segment(&opts.name)),
        ..Default::default()
    };
    let mut side = Vec::new();
    match nix.eval_json(&args) {
        Ok(v) => {
            let records: Vec<OptRecord> = serde_json::from_value(v)?;
            info!(target: "nix2rdf::nixos", options = records.len(), "option tree evaluated");
            for r in &records {
                let opt = iri::option(&r.path);
                f.add_type(opt.clone(), t::NixosOption());
                f.add_str(opt.clone(), t::optionPath(), &r.path);
                if let Some(ty) = &r.r#type {
                    f.add_str(opt.clone(), t::optionType(), ty);
                }
                for d in &r.declarations {
                    f.add_str(opt.clone(), t::declaredIn(), d);
                }
                let ov = iri::option_value(&gen_hash, &r.path);
                f.add_type(ov.clone(), t::OptionValue());
                f.add(ov.clone(), t::option(), opt.clone());
                f.add(gen.clone(), t::hasOptionValue(), ov.clone());
                f.add_bool(ov.clone(), t::isDefined(), r.is_defined);
                if let Some(v) = &r.value {
                    value_literal(&mut f, &ov, t::finalValue(), v, &mut side);
                }
                for (i, d) in r.definitions.iter().enumerate() {
                    let dn = iri::option_definition(&gen_hash, &r.path, i);
                    f.add_type(dn.clone(), t::OptionDefinition());
                    f.add(ov.clone(), t::hasDefinition(), dn.clone());
                    f.add_str(dn.clone(), t::definedIn(), &d.file);
                    if let Some(v) = &d.value {
                        value_literal(&mut f, &dn, t::definitionValue(), v, &mut side);
                    }
                }
            }
        }
        Err(e) => warn!(target: "nix2rdf::nixos", error = %e, "option tree not evaluated"),
    }

    let mut fragments = graph.fragments.clone();
    fragments.extend(side);
    if opts.with_runtime_closure {
        if let Some(p) = top.outputs.get("out").and_then(|o| o.path.clone()) {
            match extract_runtime_closure(nix, &[p]) {
                Ok((outs, seen)) => {
                    info!(target: "nix2rdf::nixos", store_paths = seen.len(), "runtime closure recorded");
                    fragments.extend(outs);
                }
                Err(e) => warn!(target: "nix2rdf::nixos", error = %e, "runtime closure unavailable (not built?)"),
            }
        }
    }
    fragments.push(f);
    Ok(fragments)
}
