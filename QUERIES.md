# Starter queries

All files in `queries/` run with `nix2rdf query <file>` (or `check` for the
ASKs) and against the endpoint. The standard prefixes (IRI.md) are prepended
automatically; queries do not need to declare them.

## Put the most selective pattern first

Oxigraph's planner is naive: it evaluates basic graph patterns mostly in the
order written, joining left to right. A query that starts with
`?snap nix:closureContains ?drv` scans every membership edge in the store
(millions once Layer 1 data is loaded) before filtering by name. Starting
with `?drv nix:pname "openssl"` touches a handful of triples and joins
outward. Rules of thumb used in every shipped query:

1. A literal lookup (`nix:pname`, `nix:rev`, `k8s:name`, `nix:optionPath`) or
   a `VALUES` block with a known IRI comes first.
2. Then the edges that fan out from it, in increasing fan-out.
3. `OPTIONAL`, `FILTER NOT EXISTS` and aggregates last.
4. Scope by named graph (`GRAPH ?snap { … }`) when you need "in this
   snapshot", and by `k8s:snapshotKind` for live vs desired.

## Set differences over closures: subquery + MINUS, not NOT EXISTS

"Derivations reachable from A but not from B" is the most common question
over `dependsOnTransitively`, and the obvious spelling is the slow one:

```sparql
# SLOW: the NOT EXISTS is re-evaluated per candidate over millions of edges
?rootA nix:dependsOnTransitively ?d .
FILTER NOT EXISTS { ?rootB nix:dependsOnTransitively ?d }
```

Correlated `FILTER NOT EXISTS` re-joins the closure for every binding of
`?d`; with a 6.6 M-edge closure that is minutes to hours. Compute both
sides as sets once and subtract:

```sparql
{ SELECT DISTINCT ?d WHERE { ?rootA nix:dependsOnTransitively ?d } }
MINUS { ?rootB nix:dependsOnTransitively ?d }
```

`closure-diff.rq` is written this way. Keep the selective pattern that
binds `?rootA` (an `nix:attrPath` or `nix:pname` literal) first inside the
subquery; `MINUS` then runs once over the right-hand set.

## The library

| File | Answers |
|---|---|
| `blast-radius.rq` | every pin/commit/image/generation whose closure contains a derivation |
| `blast-radius-source.rq` | which root pins/commits lock a given git rev |
| `pin-membership.rq` | pin membership over time, per commit |
| `closure-diff.rq` | runtime-closure diff between two snapshots |
| `fleet-drift.rq` | generations diverging from a reference, by derivation count |
| `option-provenance.rq` | who set an option: every definition with file, and the final value |
| `image-layer-sharing.rq` | layers shared by several images |
| `violations-by-policy.rq` | violations with subject, snapshot, message, producing run |
| `check-no-violations.rq` | ASK for CI: any violation of a policy |
| `runtime-blast-radius.rq` | source → images → workloads → clusters/nodes/owners (live only) |
| `running-image-digest.rq` | what runs this digest now, anywhere (latest live snapshot per cluster) |
| `cluster-layer-sharing.rq` | layer sharing across a cluster's running images |
| `drift-per-cluster.rq` | desired vs live drift per cluster |
| `failure-domain-report.rq` | zones spanned per production workload |
| `effective-permissions.rq` | effective RBAC permissions of a ServiceAccount |
| `host-from-nixpkgs-revision.rq` | workloads whose host runs kernel/runtime from a nixpkgs revision |
| `violations-by-cluster-owner.rq` | violations by policy, cluster, owner |
| `which-rules-produced.rq` | the RuleRun and packs behind a derived fact |

Most need the `core` pack's derived graph loaded (`nix2rdf reason --pack core
--all-snapshots --input-closure`); the Kubernetes ones need `k8s`.
