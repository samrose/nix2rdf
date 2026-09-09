# Writing a rule pack

A pack is a directory:

```
packs/<name>/
  pack.toml        # name, version, depends_on = ["core", ...], description
  vocab.ttl        # the pack's vocabulary (RDFS/OWL as notation), loaded as facts
  semantics.rls    # rules giving the vocabulary meaning
  mapping.rls      # rules from nix: core terms → this pack's terms
  policy/*.rls     # optional: rules deriving nix:Violation nodes
  tests/<case>/    # input.nq, expected.nq, optional forbidden.nq
```

Load order inside a pack: `semantics.rls`, `mapping.rls`, then `policy/*.rls`
sorted by name. Packs are concatenated dependencies-first.

## The convention every rule follows

The tool generates a program around your rules:

```
@prefix ... (every @prefix from every pack, deduplicated; conflicts are an error)
@import raw :- rdf{resource = "<input>.nq"} .
rawGraph(?g) :- raw(?g, ?s, ?p, ?o) .
q(?g, ?s, ?p, ?o) :- raw(?g, ?s, ?p, ?o), rawGraph(?g) .
asserted(?s, ?p, ?o) :- q(?g, ?s, ?p, ?o) .
t(?s, ?p, ?o) :- asserted(?s, ?p, ?o) .
   ... your rules ...
new(?s, ?p, ?o) :- t(?s, ?p, ?o), ~asserted(?s, ?p, ?o) .
new(?s, ?p, ?o) :- out(?s, ?p, ?o) .
```

- **Read and write `t(?s, ?p, ?o)`**: the monotone working triple set. Facts
  you derive into `t` are visible to every later rule, including other
  packs'. Rules with head `t` must not negate or aggregate over anything
  derived from `t` (that is a cycle through negation; Nemo rejects the
  program as unstratified). Negating a pure fact table (`~inTable(?n)`) or a
  predicate derived only from `q` is fine.
- **Write `out(?s, ?p, ?o)`** for the final, non-monotone layer: policies
  (`~hasPdb(?w)`), fallbacks (`~hasRuntime(?img)`), and aggregates
  (`#count`). `out` may read `t` and `out`; nothing writes back from `out`
  into `t`. Both layers end up in the derived graph.
- **Read `q(?g, ?s, ?p, ?o)`** when the named graph matters (which snapshot a
  fact belongs to). Do not write `q`.
- Helper predicates (`refT`, `roleOf`, `prodWorkload`, …) are fine; only
  `t` reaches the output. Name them distinctively; all packs share one
  namespace of predicate names. Helpers of a dependency (e.g. `roleOf`,
  `prodWorkload` from `k8s`) may be reused.
- **Existentials** create nodes: `out(!v, rdf:type, nix:Violation),
  out(!v, nix:violates, <policy>), out(!v, nix:subject, ?x) :- ...`. The tool
  turns each null into a deterministic IRI (see DESIGN.md §3).
- Literals: `"text"`, `"true"^^xsd:boolean`, integers `1`. String functions:
  `CONCAT`, `STRSTARTS`, `CONTAINS`, `STRBEFORE`, `STRAFTER`, `fullStr(?x)`
  (an IRI as a string), `isIri(?x)`. Negation `~p(...)` needs all variables
  bound elsewhere. Aggregates: `#count(?x)`.
- Prefixes `nix:`, `k8s:`, `rdf:`, `rdfs:`, `owl:`, `xsd:` and the ones in
  `iri::PREFIXES` are always available; declare your own with `@prefix`.
- IRIs with `/` in the local part (`policy:pack/name`) do not lex as prefixed
  names; write them in full: `<https://w3id.org/nix/policy/k8s/no-pdb>`.

## Performance: extract narrow relations, then join

Nemo materializes a fresh ordering of the whole triple table for every
distinct join pattern over `t/3`, so a rule like

```
x(?a, ?b) :- t(?a, k8s:inCluster, ?cl), t(?b, k8s:inCluster, ?cl), t(?a, rdf:type, k8s:Workload), ...
```

is a four-way self-join of a 300k-row table and takes minutes. Extract each
property once into a small relation and join those:

```
kInCluster(?x, ?cl) :- t(?x, k8s:inCluster, ?cl) .
kWorkload(?w) :- t(?w, rdf:type, k8s:Workload) .
wCluster(?w, ?cl) :- kWorkload(?w), kInCluster(?w, ?cl) .
sameCluster(?a, ?b) :- wCluster(?a, ?cl), wCluster(?b, ?cl) .
```

The `k8s` pack does this for every property it reads (its `k*` relations
are available to downstream packs); it went from >10 minutes to <10 seconds
on the same input. Filters such as `STRSTARTS` should sit on a small
relation too, never on a join of `t` with `t`. Computed terms (`CONCAT`,
`fullStr`) must be produced in a plain rule and passed to an existential
rule by variable; Nemo's planner rejects them directly in a head with `!v`.

## Semantics vs mapping vs policy

- `semantics.rls` gives your own vocabulary meaning (closures, inverses,
  derived classifications).
- `mapping.rls` projects `nix:` ground truth onto your vocabulary
  (SPDX, PROV, PURL). Never the other way round; `nix:` stays lossless.
- `policy/*.rls` derives `nix:Violation` nodes with `nix:violates <policy
  IRI>` and `nix:subject`. Add `nix:inSnapshot` when scoped and
  `nix:message` for detail. Declare each policy IRI in `vocab.ttl` so the
  docs list it. A CI check is a SPARQL `ASK` (`queries/check-no-violations.rq`).

## Vocabulary declarations are facts

Every selected pack's `vocab.ttl` is loaded into the input as the graph
`pack:<content-hash>`. So `rdfs:subClassOf`, `rdfs:domain`, `owl:inverseOf`
in your vocabulary are used by the `rdfs` and `owl-rl-subset` packs when
they are selected. The `core` pack's `vocab.ttl` is a symlink to
`ontology/nix.ttl`; `k8s`'s is `ontology/k8s.ttl` plus its derived-only terms.

`nix2rdf ontology validate` fails if a rule uses a `nix:` or `k8s:` term no
vocabulary declares, so typos in rules are caught at build time.

## Tests

`tests/<case>/input.nq` is N-Quads (the fourth column is the graph, use
any IRI). `expected.nq` lists triples that must be derived; a blank node
(`_:x`) in subject or object position is a wildcard, which is how you test
existentials. `forbidden.nq` lists triples that must not be derived (the
negative fixture every policy should have). Run with:

```
nix2rdf pack test --all            # or: nix2rdf pack test k8s
nix2rdf pack check core spdx3      # parse/validate the combined program
```

Tests run the pack with its dependencies, over the fixture only, in a
temporary store.

## Versioning and hashing

`pack.toml` `version` is for humans. Identity is the content hash of the
directory (tests excluded): `pack:<sha256>`, recorded on every `RuleRun`.
Changing any rule changes the ruleset hash and therefore the derived graph
name; old derived graphs stay until `nix2rdf gc-derived`.

Ship packs as flake inputs of the consumer so they are pinned in its
`flake.lock`.
