# Vendored dependencies

## nemo (rule engine)

Source: https://github.com/knowsys/nemo, commit `e578c283c996bb1b029e5642ce96f814d1b0bfae`
(2026-08-20), MIT/Apache-2.0. Only the `nemo` and `nemo-physical` crates are
kept; the root `Cargo.toml` is trimmed to those two workspace members.

Why vendored rather than a git dependency: Nemo and Oxigraph share the
`spargebra` crate. Oxigraph enables its `sep-0006` feature (SPARQL `LATERAL`),
which adds a `GraphPattern::Lateral` variant that Nemo's SPARQL import does
not match, so the two cannot be linked into one binary unpatched.

Patch (the only change to the source):
- `nemo/src/io/formats/sparql/queries.rs`: handle `GraphPattern::Lateral` in
  `variables_in_pattern` (like `Join`) and `rename_in_graph_pattern`.
- `nemo/Cargo.toml`: enable `spargebra` features `sep-0002`, `sep-0006` so the
  crate compiles the same way with and without Oxigraph present.

Upgrading: replace the two directories with the new upstream commit, re-apply
the patch above (or drop it if upstream handles `Lateral`), update the commit
in this file and in `crates/nix2rdf-core/src/nemo_engine.rs`. That module is
the only place the Nemo API is used.
- `nemo/src/execution/planning/operations/restricted_head.rs`: a `println!`
  ("generated predicate: …") that Nemo emits for every existential rule is
  turned into `log::debug!`, so library output does not pollute the CLI's
  stdout (which carries query results).
