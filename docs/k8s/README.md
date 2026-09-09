# Kubernetes / k3s integration

## Snapshots

```
# desired state, per GitOps commit (like the flake front-end)
nix2rdf k8s snapshot --cluster-id edge-site-1 --manifests ./rendered \
  --commit $GIT_SHA --repo gitops --committed-at $(git log -1 --format=%cI)

# live state, every 15 minutes (docs/k8s/cronjob.yaml) or ad hoc
nix2rdf k8s snapshot --cluster-id edge-site-1 --context edge-site-1 \
  --nixos-node-map nodes.txt        # optional: "node=generation-hash" lines
```

Each run writes `k8s/<cluster>/snapshot/<hash>` (state), `nodes/<hash>`
(inventory with `hostGeneration` links) and `observed/<hash>-<time>`. An
unchanged cluster produces the same hashes and no new state fragment.

Node → generation links come from the `nix2rdf.io/generation` annotation the
`services.nix2rdf-node-annotator` NixOS module sets on activation, or from
`--nixos-node-map`. The generation hash is `nix:gen/<hash>`, the store-path
hash of the system closure, so `k8s:hostGeneration` joins directly to the
NixOS front-end's fragments.

## Offline and partial (k3s at the edge)

The snapshot command never needs the store's index: it writes files. Ship
them later with `nix2rdf publish` + object storage, and `fetch-fragments`
on the server. If a resource kind fails to list, the snapshot is written
anyway, marked `k8s:partial true` with `k8s:failedKind` per kind.

## RBAC

`docs/k8s/rbac.yaml` is the minimal read-only ClusterRole. Secrets are
listed as metadata only (values never leave the API server).

## Reasoning

```
nix2rdf reason --pack k8s --pack policy-examples --all-snapshots --input-closure
nix2rdf query queries/violations-by-cluster-owner.rq
nix2rdf check queries/check-no-violations.rq      # non-zero exit on a hit
```

`--input-closure` pulls in the images (`runsImage`) and generations
(`hostGeneration`) the snapshots reference, and their closures, so
`runningClosure` bridges build and run. A referenced image with no loaded
`nix:Image` fragment is reported as `k8s:unknownImage`, not an error.

## Admission control (integration note, not code)

nix2rdf does not implement a webhook. Two patterns work with Kyverno or
OPA/Gatekeeper:

1. **ASK at admission.** A Kyverno `apiCall`/OPA `http.send` to the SPARQL
   endpoint: `ASK { ?v nix:violates ?p . ?v nix:subject <oci:sha256/DIGEST> }`
   over the derived graphs (`GET /query?query=…`, JSON result). Deny on `true`.
2. **Consume the derived violation graph.** Export it as JSON
   (`nix2rdf query queries/violations-by-policy.rq --format json`) on each
   run and load it as policy data (OPA bundle / Kyverno ConfigMap); policies
   then match digests locally with no runtime dependency on the endpoint.

Because derived graphs are named by `(ruleset, input)` and the input set
includes the vulnerability feed, loading a new feed re-runs the rules and a
digest admitted yesterday can be denied today. Admission control has no
memory; the derived graph is the memory.
