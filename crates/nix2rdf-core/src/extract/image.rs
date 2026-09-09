//! OCI image front-end. Builds the image derivation, then reads the artifact:
//! - dockerTools (`buildImage`/`buildLayeredImage`): a docker-archive tarball
//!   (optionally gzip/zstd). Image identity = sha256 of the config JSON (the
//!   docker image ID); layer identity = sha256 of the uncompressed layer.tar
//!   (the diff ID). Layer contents = the store paths whose files it holds.
//! - nix2container: a JSON description with layers and their store paths.
//!   Image identity = sha256 of the canonical JSON unless `--digest` is given.
//!
//! The digest a registry reports (the manifest digest that Kubernetes shows
//! in `imageID`) only exists after a push. CI passes it with `--digest` so
//! the image node carries the identity the cluster will see.

use super::drv::{extract_graph, ExtractOptions};
use crate::error::{Error, Result};
use crate::fragment::{Fragment, FragmentKind};
use crate::hash;
use crate::iri;
use crate::nix::NixSource;
use crate::vocab::nix_terms as t;
use oxrdf::NamedNode;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct ImageOptions {
    pub flake_ref: String,
    pub attr: String,
    /// Registry (manifest) digest, `sha256:<hex>`, if known from the push step.
    pub digest: Option<String>,
    pub extract: ExtractOptions,
}

#[derive(Debug, Default, Clone)]
pub struct LayerDesc {
    pub digest_hex: String,
    pub media_type: Option<String>,
    pub size: Option<u64>,
    /// Store paths (full) found in the layer.
    pub store_paths: BTreeSet<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ImageDesc {
    pub tool: String,
    pub config_digest_hex: Option<String>,
    pub repo_tags: Vec<String>,
    pub layers: Vec<LayerDesc>,
}

struct HashingReader<R: Read> {
    inner: R,
    hasher: Sha256,
    n: u64,
}
impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let k = self.inner.read(buf)?;
        self.hasher.update(&buf[..k]);
        self.n += k as u64;
        Ok(k)
    }
}

/// Store paths mentioned by a tar entry path such as `nix/store/<hash>-name/bin/x`.
fn store_path_of_entry(p: &str) -> Option<String> {
    let rest = p.strip_prefix("./").unwrap_or(p);
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let rest = rest.strip_prefix("nix/store/")?;
    let comp = rest.split('/').next()?;
    if comp.len() > 33 && comp.as_bytes()[32] == b'-' {
        Some(format!("/nix/store/{comp}"))
    } else {
        None
    }
}

fn open_maybe_compressed(path: &Path) -> Result<Box<dyn Read>> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut magic = [0u8; 4];
    let mut f2 = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let _ = f2.read(&mut magic);
    if magic[..2] == [0x1f, 0x8b] {
        Ok(Box::new(flate2::read::GzDecoder::new(f)))
    } else if magic == [0x28, 0xb5, 0x2f, 0xfd] {
        Ok(Box::new(zstd::stream::Decoder::new(f).map_err(|e| Error::Other(e.to_string()))?))
    } else {
        Ok(Box::new(f))
    }
}

/// Parse a docker-archive tarball. Layers are hashed while their entries are
/// listed, in one streaming pass over the outer archive.
pub fn read_docker_archive(path: &Path) -> Result<ImageDesc> {
    let reader = open_maybe_compressed(path)?;
    let mut archive = tar::Archive::new(reader);
    let mut manifest: Option<serde_json::Value> = None;
    let mut config_name: Option<String> = None;
    let mut layers_by_name: BTreeMap<String, LayerDesc> = BTreeMap::new();
    let mut config_digest_from_content: Option<String> = None;
    for entry in archive.entries().map_err(|e| Error::Other(e.to_string()))? {
        let mut entry = entry.map_err(|e| Error::Other(e.to_string()))?;
        let name = entry.path().map_err(|e| Error::Other(e.to_string()))?.to_string_lossy().to_string();
        let name = name.strip_prefix("./").unwrap_or(&name).to_string();
        if name == "manifest.json" {
            let mut s = String::new();
            entry.read_to_string(&mut s).map_err(|e| Error::Other(e.to_string()))?;
            manifest = Some(serde_json::from_str(&s)?);
        } else if name.ends_with(".json") && !name.contains('/') && name != "repositories" {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| Error::Other(e.to_string()))?;
            let h = hash::sha256_hex(&buf);
            config_name = Some(name.clone());
            config_digest_from_content = Some(h);
        } else if name.ends_with("layer.tar") || name.ends_with(".tar") {
            let size = entry.header().size().ok();
            let mut hr = HashingReader { inner: &mut entry, hasher: Sha256::new(), n: 0 };
            let mut paths = BTreeSet::new();
            {
                let mut inner = tar::Archive::new(&mut hr);
                for e in inner.entries().map_err(|e| Error::Other(e.to_string()))? {
                    let e = e.map_err(|e| Error::Other(e.to_string()))?;
                    if let Ok(p) = e.path() {
                        if let Some(sp) = store_path_of_entry(&p.to_string_lossy()) {
                            paths.insert(sp);
                        }
                    }
                }
            }
            // Drain any trailing bytes so the hash covers the whole layer file.
            let mut sink = [0u8; 8192];
            while let Ok(k) = hr.read(&mut sink) {
                if k == 0 {
                    break;
                }
            }
            let digest = hex::encode(hr.hasher.finalize());
            layers_by_name.insert(name.clone(), LayerDesc { digest_hex: digest, media_type: Some("application/vnd.oci.image.layer.v1.tar".into()), size, store_paths: paths });
        }
    }
    let manifest = manifest.ok_or_else(|| Error::Other(format!("{}: no manifest.json (not a docker archive?)", path.display())))?;
    let m0 = manifest.get(0).cloned().unwrap_or(manifest.clone());
    let mut desc = ImageDesc { tool: "dockerTools".into(), ..Default::default() };
    let cfg = m0.get("Config").and_then(|c| c.as_str()).map(String::from).or(config_name);
    desc.config_digest_hex = cfg
        .as_deref()
        .and_then(|c| c.strip_suffix(".json").map(String::from))
        .filter(|h| h.len() == 64)
        .or(config_digest_from_content);
    if let Some(tags) = m0.get("RepoTags").and_then(|x| x.as_array()) {
        desc.repo_tags = tags.iter().filter_map(|x| x.as_str().map(String::from)).collect();
    }
    if let Some(ls) = m0.get("Layers").and_then(|x| x.as_array()) {
        for l in ls {
            let lname = l.as_str().unwrap_or_default();
            match layers_by_name.get(lname) {
                Some(d) => desc.layers.push(d.clone()),
                None => warn!(target: "nix2rdf::image", layer = lname, "layer listed in manifest but not found in archive"),
            }
        }
    }
    Ok(desc)
}

/// Parse a nix2container image JSON.
pub fn read_nix2container(path: &Path) -> Result<ImageDesc> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)?;
    let mut desc = ImageDesc { tool: "nix2container".into(), ..Default::default() };
    desc.config_digest_hex = Some(hash::sha256_hex(hash::canonical_json(&v).as_bytes()));
    let layers = v.get("layers").and_then(|l| l.as_array()).cloned().unwrap_or_default();
    for l in layers {
        // A layer can be inline or a reference to another JSON file.
        let l = if let Some(p) = l.as_str() { serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).map_err(|e| Error::io(p, e))?)? } else { l };
        let digest = l.get("digest").and_then(|d| d.as_str()).unwrap_or_default();
        let digest_hex = digest.strip_prefix("sha256:").unwrap_or(digest).to_string();
        let mut ld = LayerDesc { digest_hex, media_type: l.get("mediatype").and_then(|m| m.as_str()).map(String::from), size: l.get("size").and_then(|s| s.as_u64()), store_paths: BTreeSet::new() };
        if let Some(paths) = l.get("paths").and_then(|p| p.as_array()) {
            for p in paths {
                let s = p.get("path").and_then(|x| x.as_str()).or(p.as_str()).unwrap_or_default();
                if let Some(h) = iri::store_path_hash(s) {
                    let _ = h;
                    ld.store_paths.insert(s.to_string());
                }
            }
        }
        desc.layers.push(ld);
    }
    Ok(desc)
}

pub fn read_image_artifact(path: &Path) -> Result<ImageDesc> {
    let meta = std::fs::metadata(path).map_err(|e| Error::io(path, e))?;
    if meta.is_dir() {
        return Err(Error::Other(format!("{}: is a directory; streamLayeredImage scripts and OCI directories are not supported in v1", path.display())));
    }
    let mut head = [0u8; 2];
    let mut f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let _ = f.read(&mut head);
    if head[0] == b'{' || path.extension().map(|e| e == "json").unwrap_or(false) {
        read_nix2container(path)
    } else {
        read_docker_archive(path)
    }
}

pub fn image_fragment(image_id_hex: &str, desc: &ImageDesc, image_drv: &NamedNode, closure: &[String], registry_digest: Option<&str>) -> Fragment {
    let img = iri::image(image_id_hex);
    let mut f = Fragment::new(FragmentKind::Image(image_id_hex.to_string()));
    f.add_type(img.clone(), t::Image());
    f.add_type(img.clone(), t::Snapshot());
    f.add_str(img.clone(), t::digest(), &format!("sha256:{image_id_hex}"));
    if let Some(c) = &desc.config_digest_hex {
        if c != image_id_hex {
            f.add_str(img.clone(), t::digest(), &format!("sha256:{c}"));
        }
    }
    if let Some(r) = registry_digest {
        f.add_str(img.clone(), t::digest(), r);
    }
    f.add_str(img.clone(), t::imageTool(), &desc.tool);
    for tag in &desc.repo_tags {
        let (name, tg) = tag.rsplit_once(':').unwrap_or((tag.as_str(), "latest"));
        f.add_str(img.clone(), t::imageName(), name);
        f.add_str(img.clone(), t::imageTag(), tg);
    }
    f.add(img.clone(), t::imageDerivation(), image_drv.clone());
    f.add(img.clone(), t::hasRoot(), image_drv.clone());
    for h in closure {
        f.add(img.clone(), t::closureContains(), iri::drv(h));
    }
    for (i, l) in desc.layers.iter().enumerate() {
        let ln = iri::layer(&l.digest_hex);
        f.add(img.clone(), t::hasLayer(), ln.clone());
        f.add_str(img.clone(), t::layerIndex(), &format!("{i}:sha256:{}", l.digest_hex));
        f.add_type(ln.clone(), t::Layer());
        f.add_str(ln.clone(), t::digest(), &format!("sha256:{}", l.digest_hex));
        if let Some(m) = &l.media_type {
            f.add_str(ln.clone(), t::mediaType(), m);
        }
        for p in &l.store_paths {
            if let Some(h) = iri::store_path_hash(p) {
                let o = iri::out(h);
                f.add(ln.clone(), t::layerContains(), o.clone());
                f.add_type(o.clone(), t::Output());
                f.add_str(o, t::storePath(), p);
            }
        }
    }
    f
}

pub fn extract_image(nix: &dyn NixSource, opts: &ImageOptions) -> Result<(Vec<Fragment>, NamedNode)> {
    let installable = format!("{}#{}", opts.flake_ref, opts.attr);
    let built = nix.build_json(&installable)?;
    let b = built.first().ok_or_else(|| Error::Other("nix build returned nothing".into()))?;
    let out = b.outputs.get("out").or_else(|| b.outputs.values().next()).ok_or_else(|| Error::Other("image has no output".into()))?;
    info!(target: "nix2rdf::image", drv = %b.drv_path, artifact = %out, "image built");
    let desc = read_image_artifact(Path::new(out))?;
    let graph = extract_graph(nix, &[installable.clone()], &opts.extract)?;
    let image_drv = graph.drv_iri(&b.drv_path).ok_or_else(|| Error::Other("image derivation not in graph".into()))?;
    let image_id = match &opts.digest {
        Some(d) => d.strip_prefix("sha256:").unwrap_or(d).to_string(),
        None => desc.config_digest_hex.clone().ok_or_else(|| Error::Other("cannot determine image digest; pass --digest".into()))?,
    };
    let closure: Vec<String> = graph.drv_hashes.values().cloned().collect();
    let mut frags = graph.fragments.clone();
    let img = iri::image(&image_id);
    frags.push(image_fragment(&image_id, &desc, &image_drv, &closure, opts.digest.as_deref()));
    // Runtime references for the layer contents, when the paths are local.
    let paths: Vec<String> = desc.layers.iter().flat_map(|l| l.store_paths.iter().cloned()).collect();
    if !paths.is_empty() {
        match super::drv::extract_runtime_closure(nix, &paths) {
            Ok((outs, _)) => frags.extend(outs),
            Err(e) => warn!(target: "nix2rdf::image", error = %e, "runtime references unavailable"),
        }
    }
    Ok((frags, img))
}
