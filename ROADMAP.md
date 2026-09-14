# Roadmap

Open work, in rough priority order. Each item says what it is, why, and what
"done" looks like. Nothing here is started unless marked. Items that exist
because another project (Fathom) needs them are still written as generic
nix2rdf features: nix2rdf depends on nothing outside itself.

## 1. QLever as an opt-in query backend

**What.** A second SPARQL backend beside Oxigraph: the QLever engine
(`pkgs.qlever`, C++, University of Freiburg). Index built in one batch from
the fragment files with `qlever-index -F nq`; queries served by
`qlever-server` over HTTP. Selected with `--backend qlever` or
`NIX2RDF_BACKEND=qlever`; Oxigraph stays the default.

**Why.** Oxigraph's evaluator is slow on the joins this project relies on:
closure × index joins (the upgrade-debt query takes minutes on 7 M quads)
and anything resembling a property path. QLever is built for exactly those
shapes. Its model also matches this project's rule that fragments are the
truth and the index is disposable: QLever's index is immutable and rebuilt
from files, and its SPARQL Update is in-memory only, which costs nothing
here because `load --new` can be replayed from the tracked loaded-set.

**Not a replacement.** Oxigraph remains for parsing and serialization
(`oxrdf`, `sparesults`), for the embedded path used by tests and small
stores, and for machines where running a server is unwanted.

**Steps.**
1. Measure first: build a QLever index from a real store (pg-image-strip,
   13 k fragments, 7 M quads) and time the four slow queries against both
   engines. Record the numbers in this file. Abandon the item if the gain is
   under 5×.
2. Verify semantics: union default graph over all named graphs (nix2rdf
   queries assume it), N-Quads graph handling, named-graph updates in the
   packaged version (0.5.48; the upstream wiki still lists them as in
   progress).
3. Run the 18 shipped queries, every pack test, and the e2e check against
   QLever; fix or document dialect differences.
4. `Backend` trait with `load`, `rebuild`, `query`, `check`, `serve`;
   Oxigraph and QLever implementations; `rebuild` = index build,
   `load --new` = `INSERT DATA` of the new fragments, restart = rebuild or
   replay from `.loaded`.
5. NixOS module: `services.nix2rdf.backend = "qlever"` starts the server as
   a second unit and points `serve` at it.

**Done when** a store can be queried through either backend with identical
results on the shipped queries, and the flake has a `qlever` check.

## 2. Fragment shapes with rudof

**What.** A ShEx (or SHACL) schema per fragment kind, under `shapes/`,
validated with the `rudof` crate: `nix2rdf check --shapes` for a store or a
set of fragment files, and a `shapes` flake check over the fixtures.

**Why.** Every fragment kind has an implicit shape today (a derivation node
has exactly one name, one system, one hash, at least one output) and nothing
enforces it. The determinism check compares fixtures byte for byte, so when
an extractor change drops a property the fixture is regenerated and the loss
is invisible. Shapes catch that class of regression directly. They also make
the ingestion contract checkable at the boundary once fragments are produced
by tools other than nix2rdf, and they express cardinality constraints that
are clumsy as Nemo rules.

**Not for.** Reasoning, querying, or rewriting existing policy packs. Rules
give derived facts with provenance; shape reports do not replace that.

**Steps.**
1. Shapes for `drv`, `src`, `out`, `pin`, `commit`, `gen`, `image`, `index`,
   `k8s` fragments, written against the ontology.
2. `check --shapes` and the flake check over `fixtures/`.
3. Validate on `publish` and on `fetch-fragments` (reject or flag non-conforming
   fragments; flagging writes a `nix:shapeViolation` record, never silently drops).
4. Publish the shapes with the ontology docs.

**Done when** the fixtures validate, a deliberately broken fixture fails the
check, and the ingestion contract document points at the shapes as its
normative form.

## 3. nixpkgs-multiverse: finish Layer 0

- Ingest `index/history.json`: one run node per contiguous block a version
  shipped in (first and last revision), so "when did this version first
  appear", "which revisions ship both X and Y", and "was it removed and
  reintroduced" become answerable. About 1.5 M quads more.
- Fix the open-tip resolution: a `null` offset must resolve against the
  file's own `revisionCount`, not the length of `revisions.json` (the two
  diverge briefly between fetch and index). One-line change in
  `extract/nixpkgs_index.rs`.
- Fixture and flake check for the index extractor (none exist).
- Per-revision fragments: with `history.json` the deltas per revision are
  known, so Layer 0 can be one small immutable fragment per revision instead
  of one 7 MiB fragment per index snapshot. Re-indexing then costs almost
  nothing.

## 4. Generic features other consumers need

- `nix2rdf export`: named queries rendered to one stable, sorted document
  (JSON, tfvars, YAML), so a downstream tool's exporters are configuration.
- Admin surface on `serve`: `POST /admin/load`, `POST /admin/reason`, so
  writes go through the process that holds the index.
- Ingestion contract document plus a reference adapter in a non-Rust
  language; the shapes from item 2 are its normative form.
- Derived graphs as first-class reasoning inputs (`--input-derived`), so
  reasoning can be tiered: per-artifact closures derived once, cheap
  per-tick runs over observations plus those summaries. Works today by
  passing the derived fragment path with `--input`; not yet first-class.
- Per-entry rendering of `systemd.services` and `environment.etc` in the
  NixOS option walker.

## 5. Build

- `crane` for the Nix package, so sandbox builds reuse the dependency
  build instead of recompiling the workspace and vendored Nemo on every
  change.
