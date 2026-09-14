# IRI scheme

Every node in the dataset has an IRI; there are no blank nodes anywhere, not
even in derived graphs. Every IRI is minted from something Nix, git, OCI or
Kubernetes already identifies deterministically, so two independent runs on
two machines produce the same IRIs for the same things.

## Namespaces

| Prefix | Expands to | Holds |
|---|---|---|
| `nix:` | `https://w3id.org/nix2rdf/ns#` | the Nix vocabulary (classes, properties) |
| `k8s:` | `https://w3id.org/nix2rdf/k8s#` | the Kubernetes vocabulary |
| `nixid:` | `https://w3id.org/nix2rdf/id/` | **every instance**, partitioned by first path segment |
| `git:` | `https://w3id.org/nix2rdf/id/git/` | consumer commits |
| `oci:` | `https://w3id.org/nix2rdf/id/oci/` | images and layers |
| `pack:` | `https://w3id.org/nix2rdf/id/pack/` | rule packs by content hash |
| `derived:` | `https://w3id.org/nix2rdf/id/derived/` | derived graphs (rule runs) |
| `frag:` | `https://w3id.org/nix2rdf/id/fragment/` | fragments that are not themselves an entity |
| `k8sid:` | `https://w3id.org/nix2rdf/id/k8s/` | Kubernetes objects |
| `org:` | `https://w3id.org/nix2rdf/id/org/` | owners |
| `policy:` | `https://w3id.org/nix2rdf/policy/` | policy identifiers used by `nix:violates` |

The design note writes instances as `nix:drv/<hash>`; in the dataset that is
`nixid:drv/<hash>`, because one prefix cannot expand to both the vocabulary
(`#`) and the instance base (`/`). Both `nix:` and `nixid:` are permanent
identifiers under `w3id.org/nix2rdf` (registration: see `w3id/README.md`).

## Instance IRIs

| Concept | IRI | Identity comes from | Known at |
|---|---|---|---|
| Derivation | `nixid:drv/<store-hash>` | the 32-char hash of the `.drv` store path | eval |
| Output | `nixid:out/<store-hash>` | the hash of the output store path | eval (input-addressed) / build (CA) |
| Source | `nixid:src/<nar>` | SRI NAR hash, base64url without padding (`sha256-…`) | eval |
| Flake pin | `nixid:pin/<nar>` | NAR hash from `flake.lock` `locked.narHash` | lock |
| Flake input | `nixid:pin/<root-nar>/input/<node-id>/<name>` | the lock node that declares it and the input name | lock |
| Attribute at a pin | `nixid:pin/<root-nar>/attr/<attrPath>` | | eval |
| nixpkgs revision | `nixid:nixpkgs/<git-rev>` | full commit SHA | index |
| Attribute at a revision | `nixid:nixpkgs/<rev>/attr/<attrPath>` | | Layer 1 |
| Package version (Layer 0) | `nixid:pkgver/<attrPath>/<version>` | | index |
| Release | `nixid:release/<name>` | e.g. `24.11` | index |
| Commit | `git:<repo-id>/<sha>` | operator repo id + full SHA | CI |
| NixOS configuration | `nixid:pin/<root-nar>/nixosConfigurations/<name>` | | eval |
| NixOS generation | `nixid:gen/<hash>` | store hash of `config.system.build.toplevel`'s output (drv hash if CA) | eval |
| NixOS option | `nixid:option/<path>` | dotted option path; global | eval |
| Option value | `nixid:gen/<hash>/opt/<path>` | | eval |
| Option definition | `nixid:gen/<hash>/opt/<path>/def/<i>` | index in `definitionsWithLocations` | eval |
| Large value | `nixid:value/<sha256>` | sha256 of canonical JSON | eval |
| Image | `oci:sha256/<digest>` | image config digest (docker image ID), or `--digest` (registry manifest digest) | build / push |
| Layer | `oci:layer/sha256/<diff-id>` | sha256 of the uncompressed layer tar | build |
| Pack | `pack:<sha256>` | content hash of the pack directory (tests excluded) | always |
| Rule run / derived graph | `derived:<ruleset-hash>:<input-hash>` | see DESIGN.md §7 | run |
| Existential node in a derived graph | `derived:<r>:<i>/n/<sha256-prefix>` | hash of the node's neighbourhood (DESIGN.md) | run |
| Cluster | `k8sid:cluster/<cluster-id>` | operator-assigned | snapshot |
| Node | `k8sid:<cluster-id>/node/<name>` | | snapshot |
| Namespace | `k8sid:<cluster-id>/ns/<name>` | | snapshot |
| Workload, Service, … | `k8sid:<cluster-id>/<Kind>/<ns>/<name>` (`<Kind>/<name>` when cluster-scoped) | Kubernetes kind and name | snapshot |
| Container spec | `k8sid:<cluster-id>/<Kind>/<ns>/<name>/c/<container>` | | snapshot |
| Permission | `k8sid:<cluster-id>/permission/<group>/<resource>/<verb>` | | snapshot |
| Failure domain | `k8sid:<cluster-id>/domain/<kind>/<value>` | node labels only | snapshot |
| Snapshot | `k8sid:<cluster-id>/snapshot/<sha256>` | hash of the normalized snapshot content | snapshot |
| Node inventory | `k8sid:<cluster-id>/nodes/<sha256>` | | snapshot |
| Owner | `org:owner/<label-value>` | configured label/annotation | snapshot |

Path segments are percent-encoded except `A-Z a-z 0-9 - . _ ~ @ + ! , = :`.
A NAR hash segment is the SRI hash with base64url alphabet and no padding,
so it never contains `/`, `+` or `=`; the original SRI string is always also
stored as a `nix:narHash` literal for lookup.

## Fragment graph IRIs

Each fragment is one named graph; the graph IRI is the fragment's identity:

| Fragment | Graph IRI |
|---|---|
| `drv/<h>` | `nixid:drv/<h>` |
| `out/<h>` | `frag:out/<h>` (outputs of derivation `<h>`) |
| `src/<nar>` | `nixid:src/<nar>` |
| `pin/<nar>` | `nixid:pin/<nar>` |
| `nixpkgs/<rev>` | `nixid:nixpkgs/<rev>` |
| `commit/<repo>/<sha>` | `git:<repo>/<sha>` |
| `gen/<h>` | `nixid:gen/<h>` |
| `image/<digest>` | `oci:sha256/<digest>` |
| `index/nixpkgs-multiverse-<h>` | `frag:index/nixpkgs-multiverse-<h>` |
| `derived/<r>/<i>` | `derived:<r>:<i>` |
| `runs/<r>/<i>` | `frag:run/<r>:<i>` |
| `values/<h>`, `env/<h>` | `frag:values/<h>`, `frag:env/<h>` |
| `k8s/<c>/snapshot/<h>` | `k8sid:<c>/snapshot/<h>` |
| `k8s/<c>/nodes/<h>` | `k8sid:<c>/nodes/<h>` |
| `k8s/<c>/observed/<h>-<t>` | `frag:k8s-observed/<c>/<h>/<t>` |

## Guarantees

1. **Determinism.** The same input yields the same IRI on any machine. No
   IRI contains a timestamp, a counter, a hostname or a random value.
2. **No blank nodes.** Nemo existential nulls are relabelled to IRIs derived
   from the node's neighbourhood before a derived graph is written.
3. **Content addressing where Nix has it.** Derivations, outputs, sources,
   pins, images and layers reuse Nix's and OCI's own hashes, so a derivation
   fragment written by one front-end is byte-identical to the one another
   front-end would write.
4. **Immutability.** A fragment's content is a function of its identity.
   Nothing is ever rewritten in place; new facts about an existing entity
   go into a new fragment (e.g. `out/<h>` after a build, `runs/…` after a
   reasoning run).
5. **Stability across snapshots.** Kubernetes object IRIs do not include the
   snapshot hash, so the same Deployment is the same node across time and
   scope-dependent facts are separated by named graph.
