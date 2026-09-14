# nix2rdf

Nix artifacts as a content-addressed RDF dataset. One extractor turns
derivation graphs into N-Quads fragments; Oxigraph answers SPARQL over them;
Nemo rule packs derive everything else: closures, SBOM projections (SPDX 3,
PROV), identifiers (PURL, CPE), policy violations, and, with the Kubernetes
front-end, what is running where.

```
nix2rdf flake .#                              # derivation graph + lock graph + snapshot
nix2rdf nixos .#nixosConfigurations.host      # system closure + option provenance
nix2rdf image .#dockerImage --digest sha256:… # image, layers, layer contents
nix2rdf nixpkgs-index                         # Layer 0: every nixpkgs revision (minutes)
nix2rdf k8s snapshot --cluster-id prod        # what runs, at digest level
nix2rdf load --all && nix2rdf reason --pack core --pack k8s --all-snapshots --input-closure
nix2rdf serve                                 # SPARQL 1.1 endpoint on :7878
nix2rdf check queries/check-no-violations.rq  # CI gate
```

Documents: [DESIGN.md](DESIGN.md) (architecture and every decision),
[IRI.md](IRI.md) (identity scheme), [PACKS.md](PACKS.md) (writing rules),
[QUERIES.md](QUERIES.md) (starter queries), `ONTOLOGY.md` (generated),
[docs/k8s](docs/k8s/README.md) (Kubernetes/k3s), [w3id](w3id/README.md)
(namespace registration), [ROADMAP.md](ROADMAP.md) (open work).

## Build

Everything goes through the flake; Nemo needs a pinned nightly Rust, which
rust-overlay provides from `rust-toolchain.toml`.

```
nix develop                 # toolchain, nix-eval-jobs, libclang for RocksDB
cargo build && cargo test   # inside the shell
nix build                   # the wrapped binary (packs, queries, ontology under share/)
nix flake check             # unit, pack, determinism, e2e, k8s, ontology checks
nix build .#ontology-docs   # ns.ttl / ns.html / ONTOLOGY.md for the w3id target
```

`nix flake check` runs, in the sandbox: the build with unit tests, the pack
fixture tests, the determinism test, the end-to-end fixture run, the
Kubernetes snapshot determinism test, ontology validation, and the linters:
`cargo fmt --check`, `cargo clippy -D warnings`, `cargo deny check licenses
bans sources` (license allow-list in `deny.toml`), `nixpkgs-fmt --check`,
`statix`, `deadnix`, and `yamllint`. The same tools are in the dev shell; the
usual loop is `cargo fmt && cargo clippy --all-targets && cargo test` before
`nix flake check`.

Vendored: `vendor/nemo` (patched; see `vendor/PATCHES.md`).

## Deploy

`nixosModules.default` runs the endpoint as a systemd service with timers
for the nixpkgs index and for mirroring published fragments;
`nixosModules.node-annotator` stamps a k3s node with its NixOS generation.

```nix
{
  inputs.nix2rdf.url = "github:samrose/nix2rdf";
  # ...
  imports = [ inputs.nix2rdf.nixosModules.default ];
  services.nix2rdf = {
    enable = true;
    listenAddress = "0.0.0.0"; openFirewall = true;
    index.enable = true;
    fetchFragments.urls = [ "https://fragments.example.org/nixpkgs-eval" ];
    reason.packs = [ "core" "spdx3" "policy-examples" ];
  };
}
```

## Store layout

```
store/
  drv/<hash>.nq.zst          out/<hash>.nq.zst         src/<nar>.nq.zst
  pin/<nar>.nq.zst           nixpkgs/<rev>.nq.zst      commit/<repo>/<sha>.nq.zst
  gen/<hash>.nq.zst          image/<digest>.nq.zst     index/nixpkgs-multiverse-<hash>.nq.zst
  values/<hash>.nq.zst       env/<hash>.nq.zst
  derived/<ruleset>/<input>.nq.zst   runs/<ruleset>/<input>.nq.zst
  k8s/<cluster>/{snapshot,nodes,observed}/…
  packs/<hash>/              oxigraph/   (rebuildable index)
```

Files are canonical. `nix2rdf rebuild` recreates the index from them.

## Disk estimate

Measured on this machine with `nix2rdf stats` (fragments are zstd level 9;
the index is Oxigraph's RocksDB store after `nix2rdf rebuild`):

| Unit | Quads | Fragment files on disk | Index (RocksDB) |
|---|---|---|---|
| One derivation fragment (`drv/<hash>`) | ~35 | ~0.95 KiB | ~7 KiB |
| One source fragment (`src/<nar>`) | 4 | ~0.25 KiB | ~1 KiB |
| `hello` closure: 541 derivations, 259 sources, pin, commit | 21.8k | 576 KiB (803 files) | 65 MB (incl. ~50 MB RocksDB baseline) |
| `core` pack derived graph for that closure | 94k | 265 KiB | ~19 MB |
| All derived graphs of the fixture (core, rdfs, identifiers, k8s, policies) | 477k | 1.3 MiB | ~40 MB |
| Layer 0: nixpkgs-multiverse index, all history (1,547 revisions, 310,577 package versions) | 1.28 M | 7.2 MiB (1 file; 323 MB uncompressed) | 180–400 MB |
| One Kubernetes snapshot of the fixture cluster (20 objects) | ~300 | 2 KiB (3 files) | negligible |

Rules of thumb that follow: **~1 KiB per derivation on disk, ~200 bytes per
quad in the index** (Oxigraph keeps several orderings of every quad), and
derived closure graphs (`dependsOnTransitively`, `closureContains`) are
roughly `derivations × average depth`, four to five quads per asserted quad
on the fixture.

Extrapolation (estimates, not measurements):

| Scenario | Fragments | Index |
|---|---|---|
| Layer 0 only (a server that answers "which nixpkgs revisions shipped X") | 8 MB | 0.4 GB |
| A team: 20 application flakes / NixOS hosts, ~5k derivations each, 90% shared, one commit per day for a year with `core` reasoning per snapshot | ~60 MB drv/src + ~10 MB snapshots + ~1–2 GB derived | 3–6 GB |
| Layer 1: one full nixpkgs revision for one system (~300k derivations) | ~300 MB per revision; ~30 MB incremental per later revision (≈90% of derivations unchanged) | ~2 GB per revision loaded; sharing keeps ten loaded revisions near 4–5 GB |
| Layer 1 for every unstable bump of a year (~150 revisions, shared) | ~5 GB | 20–30 GB (do not load them all; keep files, load on demand) |
| Kubernetes: 5 clusters × 500 objects, live snapshot every 15 min, ~10% of snapshots actually change | ~100 KiB per changed snapshot → ~1–2 GB/year, mostly `observed/` records | 1–3 GB/year |

Two costs to plan for: the filesystem block overhead of many small files
(a 300k-derivation revision is 300k files; on a 4 KiB-block filesystem that is
~1.2 GB of slack, so use a filesystem with small blocks or inline data), and
transitive-closure graphs, which dominate derived size; run `core` per
snapshot (`--root <snapshot> --input-closure`) rather than over the whole
store, and `gc-derived` when rules change.

## License

nix2rdf is MIT licensed (see `LICENSE`). Everything it links is
permissively licensed and compatible: Oxigraph (MIT OR Apache-2.0),
Nemo (MIT OR Apache-2.0; the vendored copy under `vendor/nemo` keeps its
`LICENSE-MIT`/`LICENSE-APACHE` and is used under MIT), kube (Apache-2.0),
RocksDB via oxrocksdb-sys (Apache-2.0), zstd (BSD-3-Clause). The
nixpkgs-multiverse index files it ingests are MIT. `cargo deny check
licenses` (in the dev shell and in `nix flake check`) enforces the allow
list in `deny.toml`.
