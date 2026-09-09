# Ontology reference

Generated from `ontology/*.ttl` by `nix2rdf ontology doc`. Do not edit by hand.

OWL and RDFS terms in these vocabularies are **notation only**: they record intent for readers and for the `rdfs`/`owl-rl-subset` rule packs. No OWL reasoner runs; every entailment in the dataset was produced by an authored Nemo rule.

## The Nix vocabulary (`nix:`)

Namespace: `https://w3id.org/nix/ns#`  
Version: 0.1.0  
Version IRI: https://w3id.org/nix/ontology/0.1.0

A small, lossless RDF vocabulary for Nix derivations, outputs, sources, flakes, nixpkgs revisions, NixOS configurations and OCI images built with Nix. OWL/RDFS terms are notation only; semantics are given by Nemo rule packs.

### Classes

#### `nix:Attribute`

An attribute path evaluated at a specific nixpkgs revision (Layer 1). Links the path string to the derivation it evaluates to.


#### `nix:Commit`

A commit of a consumer repository, as handed to nix2rdf by CI. The time axis: it links a snapshot to when it was committed.


#### `nix:Derivation`

A Nix derivation (.drv). Identity: the store-path hash of the .drv file. Everything Nix knows at evaluation time about a build step.


#### `nix:FlakeInput`

A named input edge in a flake.lock (the `inputs.<name>` of a node). Distinct from the pin it resolves to, because two inputs can resolve to one pin and an input can `follow` another.


#### `nix:FlakePin`

A locked flake source (a `locked` entry in flake.lock). Identity: its NAR hash. The root flake's own source is also a pin.


#### `nix:Fragment`

A named graph that is one immutable, content-addressed N-Quads file in the store. The graph IRI is the fragment's identity.


#### `nix:Image`

An OCI/Docker image built by Nix (dockerTools or nix2container). Identity: the sha256 of the image configuration (the image ID).

- subClassOf: `nix:Snapshot`

#### `nix:Layer`

One image layer. Identity: the sha256 of the uncompressed layer tarball (the diff ID), so identical layers in different images are one node.


#### `nix:NixosConfiguration`

A `nixosConfigurations.<name>` attribute of a flake: the named, un-evaluated configuration a generation was produced from.


#### `nix:NixosGeneration`

An evaluated NixOS system closure (config.system.build.toplevel). Identity: the store-path hash of the toplevel output.

- subClassOf: `nix:Snapshot`

#### `nix:NixosOption`

A NixOS option declaration, identified by its dotted path (e.g. services.nginx.enable). Global, not per generation.


#### `nix:NixpkgsRevision`

A git revision of nixpkgs. Identity: the full commit SHA. Layer 0 (index) creates these; Layer 1 (full evaluation) attaches evaluatesTo edges.

- subClassOf: `nix:Snapshot`

#### `nix:OntologyPack`

A rule pack (vocabulary + Nemo rules) as used by a rule run. Identity: the content hash of the pack directory.


#### `nix:OptionDefinition`

One definition of an option (a `services.foo.x = ...;` in some file) that contributed to a final value. Carries the file it was defined in.


#### `nix:OptionValue`

The final value of an option in one generation, with its definitions.


#### `nix:Output`

A derivation output (a store path such as /nix/store/<hash>-hello-2.12). Identity: the store-path hash. For content-addressed derivations the output is only known after the build.


#### `nix:PackageVersion`

An (attribute path, version string) pair from the nixpkgs-multiverse index, with the newest revision that shipped it. Layer 0 data: no evaluation, no derivation.


#### `nix:Release`

A NixOS release channel (e.g. 24.11) as recorded in the nixpkgs-multiverse index, pointing at the revision it was at when indexed.


#### `nix:RuleRun`

One execution of a set of packs over a set of input graphs. Its IRI is the derived graph's name: derived:<ruleset-hash>:<input-hash>.


#### `nix:Snapshot`

Anything that has a reachable derivation closure: a flake pin used as a root, a NixOS generation, an OCI image, a nixpkgs revision. Domain of closureContains.


#### `nix:Source`

A store path that was added to the store from outside a build: a fetched tree, a local file, a flake source. Identity: its NAR hash.


#### `nix:Violation`

A policy violation derived by a rule pack. Never asserted by the extractor; always created by a Nemo existential rule in a derived graph.


### Properties

#### `nix:arg`

One builder argument, as an indexed literal "<index>:<value>" so ordering survives set semantics.

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:attrPath`

Dotted attribute path (hello, python3Packages.requests).

- range: `xsd:string`

#### `nix:builder`

The builder executable path (e.g. .../bin/bash). A store path string, not a node, because builders are not always derivation outputs.

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:channelName`

The channel release name (nixos-24.11pre123.abcdef).

- domain: `nix:NixpkgsRevision`
- range: `xsd:string`

#### `nix:closureContains`

A snapshot's reachable derivation set. Asserted by front-ends from what Nix reported reachable (nix derivation show -r); the core pack additionally closes it under dependsOn from hasRoot so images and generations stay complete.

- domain: `nix:Snapshot`
- range: `nix:Derivation`

#### `nix:committedAt`

- domain: `nix:Commit`
- range: `xsd:dateTime`

#### `nix:configuration`

The named configuration this generation was evaluated from.

- domain: `nix:NixosGeneration`
- range: `nix:NixosConfiguration`

#### `nix:contentAddressed`

True when outputs are content-addressed and therefore unknown at evaluation time.

- domain: `nix:Derivation`
- range: `xsd:boolean`

#### `nix:currentAtIndexTip`

True when the index records no newest revision for this version, i.e. it is still shipped at the tip.

- domain: `nix:PackageVersion`
- range: `xsd:boolean`

#### `nix:date`

Calendar date of a revision or release as recorded by the index.

- range: `xsd:date`

#### `nix:declaredIn`

A file that declares the option (options.<x>.declarations).

- domain: `nix:NixosOption`
- range: `xsd:string`

#### `nix:definedIn`

The file a definition came from (definitionsWithLocations[].file).

- domain: `nix:OptionDefinition`
- range: `xsd:string`

#### `nix:definitionValue`

A definition's value as canonical JSON, subject to the same size threshold as finalValue.

- domain: `nix:OptionDefinition`
- range: `xsd:string`

#### `nix:dependsOn`

Direct build-time dependency (derivation to derivation). Asserted as a synonym of inputDrv; the core pack computes its transitive closure into dependsOnTransitively.

- domain: `nix:Derivation`
- range: `nix:Derivation`

#### `nix:dependsOnPack`

- domain: `nix:OntologyPack`
- range: `nix:OntologyPack`

#### `nix:dependsOnTransitively`

DERIVED ONLY (core pack). Transitive closure of dependsOn. Never asserted.

- domain: `nix:Derivation`
- range: `nix:Derivation`
- transitive (notation; closure computed by the core pack)

#### `nix:derivedBy`

A derived graph (as a named-graph IRI) was produced by this rule run.

- range: `nix:RuleRun`

#### `nix:derivedQuadCount`

- domain: `nix:RuleRun`
- range: `xsd:integer`

#### `nix:description`

meta.description, or an option's description.

- range: `xsd:string`

#### `nix:digest`

sha256:<hex> of an image config or layer, as OCI prints it.

- range: `xsd:string`

#### `nix:drvPath`

The full /nix/store/... .drv path.

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:envHash`

sha256 of the canonical JSON of the derivation's environment. The full environment is not inlined; use --env full to emit it as a side fragment.

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:envVar`

One environment variable as "<name>=<value>". Only emitted in the optional env side fragment.

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:evaluatesTo`

A nixpkgs revision (or an Attribute at a revision) evaluates to this derivation: it COULD be built from that tree. Layer 1 data. Contrast closureContains (actually depended on).

- range: `nix:Derivation`

#### `nix:finalValue`

The value as canonical JSON, inlined when below the size threshold; otherwise absent and valueHash points to a side fragment.

- range: `xsd:string`

#### `nix:follows`

This input is an alias for another input (an `inputs.x.follows = "y"` entry). The core pack resolves follows chains to the final pin.

- domain: `nix:FlakeInput`
- range: `nix:FlakeInput`

#### `nix:fragmentKind`

drv, out, src, pin, nixpkgs, commit, gen, image, index, derived, k8s-snapshot, ...

- domain: `nix:Fragment`
- range: `xsd:string`

#### `nix:fromPin`

The root flake pin a generation or image was evaluated from.

- range: `nix:FlakePin`

#### `nix:hasAttribute`

- domain: `nix:NixpkgsRevision`
- range: `nix:Attribute`

#### `nix:hasDefinition`

A definition that contributed to the final value (from definitionsWithLocations). Priorities and merge internals are not reconstructed.

- domain: `nix:OptionValue`
- range: `nix:OptionDefinition`

#### `nix:hasFragment`

- domain: `nix:RuleRun`
- range: `nix:Fragment`

#### `nix:hasLayer`

An image contains this layer. Order is carried by layerIndex on a per-image basis is NOT modelled; use the manifest literal for order.

- domain: `nix:Image`
- range: `nix:Layer`

#### `nix:hasOptionValue`

- domain: `nix:NixosGeneration`
- range: `nix:OptionValue`

#### `nix:hasOutput`

An output of the derivation. Emitted in the derivation fragment for input-addressed derivations, in a separate outputs fragment for content-addressed ones.

- domain: `nix:Derivation`
- range: `nix:Output`

#### `nix:hasRoot`

The derivation(s) a snapshot was evaluated from: the installables of a flake snapshot, the toplevel of a generation, the image derivation.

- domain: `nix:Snapshot`
- range: `nix:Derivation`

#### `nix:homepage`

- range: `xsd:anyURI`

#### `nix:imageDerivation`

The derivation that built the image artifact.

- domain: `nix:Image`
- range: `nix:Derivation`

#### `nix:imageName`

- domain: `nix:Image`
- range: `xsd:string`

#### `nix:imageTag`

- domain: `nix:Image`
- range: `xsd:string`

#### `nix:imageTool`

dockerTools or nix2container.

- domain: `nix:Image`
- range: `xsd:string`

#### `nix:inSnapshot`

The snapshot in which the violation was found, when the policy is snapshot-scoped.

- domain: `nix:Violation`
- range: `nix:Snapshot`

#### `nix:indexOffset`

Position in nixpkgs-multiverse revisions.json (0 = oldest). Lets SPARQL order revisions without parsing dates.

- domain: `nix:NixpkgsRevision`
- range: `xsd:integer`

#### `nix:inputDrv`

Direct build-time input derivation, exactly as listed in the .drv's inputDrvs. Not transitive.

- domain: `nix:Derivation`
- range: `nix:Derivation`

#### `nix:inputDrvOutput`

Which outputs of an input derivation are consumed, as "<input-drv-hash>!<outputName>". Keeps inputDrv lossless without a node per (drv, output) pair.

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:inputGraph`

A named graph that was part of the rule run's input.

- domain: `nix:RuleRun`

#### `nix:inputHash`

- domain: `nix:RuleRun`
- range: `xsd:string`

#### `nix:inputSrc`

A store path listed in the .drv's inputSrcs: a source file or tree the build reads.

- domain: `nix:Derivation`
- range: `nix:Source`

#### `nix:isDefined`

Whether any module defined the option (as opposed to the default applying).

- domain: `nix:OptionValue`
- range: `xsd:boolean`

#### `nix:isFlake`

False when the input was declared with flake = false.

- domain: `nix:FlakeInput`
- range: `xsd:boolean`

#### `nix:lastModified`

lastModified from flake.lock: Unix seconds. Part of the locked identity, hence allowed inside a fragment.

- range: `xsd:integer`

#### `nix:lastSeenIn`

The newest indexed revision that shipped this (attribute, version). From nixpkgs-multiverse index/versions.json.

- domain: `nix:PackageVersion`
- range: `nix:NixpkgsRevision`

#### `nix:layerContains`

A store path whose files are in this layer.

- domain: `nix:Layer`
- range: `nix:Output`

#### `nix:layerIndex`

Layer order within an image as "<index>:<layer-digest>", because a layer node is shared across images.

- domain: `nix:Image`
- range: `xsd:string`

#### `nix:license`

The raw nixpkgs license identifier(s) from meta.license (the `spdxId` when present, else the `shortName`). Never an SPDX expression; the spdx3 pack maps it.

- range: `xsd:string`

#### `nix:lockNodeId`

The node key inside flake.lock (nixpkgs, nixpkgs_2, ...).

- domain: `nix:FlakeInput`
- range: `xsd:string`

#### `nix:lockedDir`

- domain: `nix:FlakePin`
- range: `xsd:string`

#### `nix:lockedOwner`

- domain: `nix:FlakePin`
- range: `xsd:string`

#### `nix:lockedRepo`

- domain: `nix:FlakePin`
- range: `xsd:string`

#### `nix:lockedType`

flake.lock locked.type: github, gitlab, git, path, tarball, ...

- domain: `nix:FlakePin`
- range: `xsd:string`

#### `nix:lockedUrl`

- domain: `nix:FlakePin`
- range: `xsd:string`

#### `nix:locksInput`

A pin (a flake) declares this input. The root pin locks the root inputs; nested pins lock their own.

- domain: `nix:FlakePin`
- range: `nix:FlakeInput`

#### `nix:mediaType`

- range: `xsd:string`

#### `nix:message`

Optional human-readable detail attached by the policy rule.

- domain: `nix:Violation`
- range: `xsd:string`

#### `nix:name`

The derivation name (the `name` attribute, e.g. hello-2.12.3), or the name of an input / configuration.

- range: `xsd:string`

#### `nix:narHash`

SRI NAR hash (sha256-...) exactly as Nix prints it.

- range: `xsd:string`

#### `nix:nemoVersion`

- domain: `nix:RuleRun`
- range: `xsd:string`

#### `nix:option`

The option this value is the final value of.

- domain: `nix:OptionValue`
- range: `nix:NixosOption`

#### `nix:optionPath`

- domain: `nix:NixosOption`
- range: `xsd:string`

#### `nix:optionType`

type.description of the option (e.g. "boolean", "list of string").

- domain: `nix:NixosOption`
- range: `xsd:string`

#### `nix:originalRef`

The `original` entry of the lock node as a flake reference string (what the flake.nix asked for).

- domain: `nix:FlakeInput`
- range: `xsd:string`

#### `nix:outputName`

out, dev, lib, doc, ...

- domain: `nix:Output`
- range: `xsd:string`

#### `nix:packName`

- domain: `nix:OntologyPack`
- range: `xsd:string`

#### `nix:packVersion`

- domain: `nix:OntologyPack`
- range: `xsd:string`

#### `nix:pinnedTo`

The locked source a flake input resolves to in flake.lock.

- domain: `nix:FlakeInput`
- range: `nix:FlakePin`

#### `nix:pname`

Package name without version, from the derivation's `pname` environment variable (or structured attrs).

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:producedBy`

DERIVED ONLY (core pack). Inverse of hasOutput.

- domain: `nix:Output`
- range: `nix:Derivation`
- inverseOf: `nix:hasOutput`

#### `nix:ranAt`

Wall-clock time of the run. Lives on the RuleRun node in the run's provenance fragment, never inside a derived fragment.

- domain: `nix:RuleRun`
- range: `xsd:dateTime`

#### `nix:ref`

Git ref (branch/tag) recorded in a lock entry.

- range: `xsd:string`

#### `nix:references`

Runtime reference from one built store path to another, as recorded in the Nix store database (nix path-info). Only known after a build. Direct edges only.

- domain: `nix:Output`
- range: `nix:Output`

#### `nix:releaseBuild`

- domain: `nix:Release`
- range: `xsd:integer`

#### `nix:releasePinnedTo`

- domain: `nix:Release`
- range: `nix:NixpkgsRevision`

#### `nix:repo`

The operator-assigned repository id used in git:<repo-id>/<sha>.

- domain: `nix:Commit`
- range: `xsd:string`

#### `nix:resolvesTo`

DERIVED ONLY (core pack). The pin an input ends up at after following any `follows` chain.

- domain: `nix:FlakeInput`
- range: `nix:FlakePin`

#### `nix:rev`

Git revision (full SHA) of a pin or nixpkgs revision.

- range: `xsd:string`

#### `nix:rulesetHash`

- domain: `nix:RuleRun`
- range: `xsd:string`

#### `nix:runtimeClosureContains`

DERIVED ONLY (core pack). Outputs reachable from the snapshot's root outputs via references. The runtime closure, as opposed to the build closure.

- domain: `nix:Snapshot`
- range: `nix:Output`

#### `nix:runtimeDependsOn`

DERIVED ONLY (core pack). Derivation A runtime-depends on B when an output of A references an output of B (transitively). Never asserted.

- domain: `nix:Derivation`
- range: `nix:Derivation`

#### `nix:sharesLayerWith`

DERIVED ONLY (core pack). Two images that have at least one layer in common.

- domain: `nix:Image`
- range: `nix:Image`

#### `nix:snapshot`

The snapshot (root pin, generation, image) that a consumer commit evaluated to.

- domain: `nix:Commit`
- range: `nix:Snapshot`

#### `nix:sourceProvenance`

meta.sourceProvenance short names (fromSource, binaryNativeCode, ...).

- range: `xsd:string`

#### `nix:storePath`

The full /nix/store/... path of an Output or Source.

- range: `xsd:string`

#### `nix:structuredAttrs`

True when the derivation uses __structuredAttrs (its attributes live in the __json variable).

- domain: `nix:Derivation`
- range: `xsd:boolean`

#### `nix:subject`

The node the violation is about.

- domain: `nix:Violation`

#### `nix:system`

The platform string the derivation builds on (x86_64-linux, aarch64-darwin, ...).

- domain: `nix:Derivation`
- range: `xsd:string`

#### `nix:unfree`

meta.license.free == false for at least one license.

- range: `xsd:boolean`

#### `nix:usesPack`

- domain: `nix:RuleRun`
- range: `nix:OntologyPack`

#### `nix:value`

A large canonical-JSON value, stored in a side fragment keyed by its hash.

- range: `xsd:string`

#### `nix:valueHash`

sha256 of the canonical JSON value. The value itself lives in values/<hash>.nq.zst as a nix:value literal on the node <nixid:value/<hash>>.

- range: `xsd:string`

#### `nix:version`

Version string, from the derivation's `version` (or a PackageVersion's version).

- range: `xsd:string`

#### `nix:violates`

The policy IRI (a term in a pack's policy namespace) that the subject violates.

- domain: `nix:Violation`

## The Kubernetes deployment vocabulary for nix2rdf (`k8s:`)

Namespace: `https://w3id.org/nix/k8s#`  
Version: 0.1.0  
Version IRI: https://w3id.org/nix/k8s/0.1.0

Identity, references, placement and ownership of Kubernetes objects at the image-digest level, so build-time rules and queries also govern deployments.

### Classes

#### `k8s:Cluster`

A Kubernetes cluster under an operator-assigned stable id (never the kubeconfig context name).


#### `k8s:ClusterRole`


#### `k8s:ClusterRoleBinding`


#### `k8s:ConfigMap`


#### `k8s:ContainerSpec`

One container entry of a workload's pod template (init containers included, flagged with initContainer).


#### `k8s:CronJob`

- subClassOf: `k8s:Workload`

#### `k8s:DaemonSet`

- subClassOf: `k8s:Workload`

#### `k8s:Deployment`

- subClassOf: `k8s:Workload`

#### `k8s:FailureDomain`

A zone, region, rack or site, taken from node labels only. Never inferred.


#### `k8s:Group`

A Group subject of a binding.


#### `k8s:Ingress`


#### `k8s:Job`

- subClassOf: `k8s:Workload`

#### `k8s:Namespace`


#### `k8s:NetworkPolicy`


#### `k8s:Node`

A cluster node. When the node is a NixOS host, hostGeneration links it to its nix:NixosGeneration.


#### `k8s:Owner`

An owning team/person from a configured label or annotation.


#### `k8s:Permission`

A structured (verb, resource, apiGroup) node granted by a Role or ClusterRole.


#### `k8s:PersistentVolume`


#### `k8s:PersistentVolumeClaim`


#### `k8s:PodDisruptionBudget`


#### `k8s:Role`


#### `k8s:RoleBinding`


#### `k8s:Secret`

Identity and references only. Values are never read, hashed or stored.


#### `k8s:Service`


#### `k8s:ServiceAccount`


#### `k8s:Snapshot`

One observation of a cluster, live or desired. Identity: hash of the normalized content, so an unchanged cluster produces the same snapshot.


#### `k8s:SnapshotKind`


#### `k8s:StatefulSet`

- subClassOf: `k8s:Workload`

#### `k8s:StorageClass`


#### `k8s:User`

A User subject of a binding.


#### `k8s:Workload`

A pod-owning controller. Subclasses per kind.


### Properties

#### `k8s:aggregates`

ClusterRole aggregation (aggregationRule matched the other role's labels).

- domain: `k8s:ClusterRole`
- range: `k8s:ClusterRole`

#### `k8s:allowsAllEgress`

- domain: `k8s:NetworkPolicy`
- range: `xsd:boolean`

#### `k8s:allowsAllIngress`

- domain: `k8s:NetworkPolicy`
- range: `xsd:boolean`

#### `k8s:allowsEgressTo`

- domain: `k8s:NetworkPolicy`

#### `k8s:allowsIngressFrom`

Policy allows ingress to the workloads it selects from this workload or namespace.

- domain: `k8s:NetworkPolicy`

#### `k8s:apiGroup`

- domain: `k8s:Permission`
- range: `xsd:string`

#### `k8s:apiVersion`

- range: `xsd:string`

#### `k8s:appliesTo`

The workloads the policy's podSelector matched.

- domain: `k8s:NetworkPolicy`
- range: `k8s:Workload`

#### `k8s:boundTo`

- domain: `k8s:PersistentVolumeClaim`
- range: `k8s:PersistentVolume`

#### `k8s:clusterId`

- domain: `k8s:Cluster`
- range: `xsd:string`

#### `k8s:containerRuntimeVersion`

- domain: `k8s:Node`
- range: `xsd:string`

#### `k8s:contains`

Every object observed in a snapshot. Object IRIs are stable across snapshots; scope-dependent facts are in the snapshot's named graph.

- domain: `k8s:Snapshot`

#### `k8s:costCenter`

- range: `xsd:string`

#### `k8s:danglingRef`

DERIVED ONLY (k8s pack). A typed reference whose target is absent from the same snapshot, as "<property>:<target-iri>".

- range: `xsd:string`

#### `k8s:domainKind`

zone, region, rack or site.

- domain: `k8s:FailureDomain`
- range: `xsd:string`

#### `k8s:exemptSingleDomain`

Explicit exemption from the single-failure-domain policy, from the nix2rdf.io/exempt-single-domain annotation.

- domain: `k8s:Workload`
- range: `xsd:boolean`

#### `k8s:failedKind`

A resource kind that failed to list in a partial snapshot.

- domain: `k8s:Snapshot`
- range: `xsd:string`

#### `k8s:grants`

Binding to the Role or ClusterRole it grants.


#### `k8s:hasContainer`

- domain: `k8s:Workload`
- range: `k8s:ContainerSpec`

#### `k8s:hasPDB`

- domain: `k8s:Workload`
- range: `k8s:PodDisruptionBudget`

#### `k8s:hostGeneration`

The NixOS generation the node runs, from --nixos-node-map or the nix2rdf.io/generation annotation.

- domain: `k8s:Node`
- range: `nix:NixosGeneration`

#### `k8s:imageRef`

The image string exactly as written in the spec.

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:imageRepository`

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:imageTag`

The tag part of imageRef. A property, never identity.

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:inCluster`

- range: `k8s:Cluster`

#### `k8s:inFailureDomain`

- domain: `k8s:Node`
- range: `k8s:FailureDomain`

#### `k8s:inNamespace`

- range: `k8s:Namespace`

#### `k8s:initContainer`

- domain: `k8s:ContainerSpec`
- range: `xsd:boolean`

#### `k8s:kernelVersion`

- domain: `k8s:Node`
- range: `xsd:string`

#### `k8s:kind`

- range: `xsd:string`

#### `k8s:kubeletVersion`

- domain: `k8s:Node`
- range: `xsd:string`

#### `k8s:label`

A metadata label as "key=value". Stored for Nodes, Namespaces and Workloads so policies can match on them.

- range: `xsd:string`

#### `k8s:limitsCpu`

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:limitsMemory`

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:maxUnavailable`

- domain: `k8s:PodDisruptionBudget`
- range: `xsd:string`

#### `k8s:minAvailable`

- domain: `k8s:PodDisruptionBudget`
- range: `xsd:string`

#### `k8s:mounts`

A ConfigMap, Secret or PVC referenced by a volume, env, or envFrom. Typed edge, so dangling references are detectable.

- domain: `k8s:Workload`

#### `k8s:name`

metadata.name

- range: `xsd:string`

#### `k8s:nodeRole`

From node-role.kubernetes.io/<role> labels.

- domain: `k8s:Node`
- range: `xsd:string`

#### `k8s:observedAt`

When the snapshot was taken. Lives in the snapshot's provenance fragment (k8s/<cluster>/observations), NOT in the content-addressed snapshot fragment, so identical states dedupe.

- domain: `k8s:Snapshot`
- range: `xsd:dateTime`

#### `k8s:ofCluster`

- domain: `k8s:Snapshot`
- range: `k8s:Cluster`

#### `k8s:osImage`

- domain: `k8s:Node`
- range: `xsd:string`

#### `k8s:ownedBy`

From the configured owner label/annotation. Workload label wins over namespace label; documented in DESIGN.md.

- range: `k8s:Owner`

#### `k8s:ownerUnknown`

True when no configured owner label/annotation was present. Data, not an error.

- range: `xsd:boolean`

#### `k8s:partial`

True when at least one resource kind could not be listed (k3s edge, partial API availability).

- domain: `k8s:Snapshot`
- range: `xsd:boolean`

#### `k8s:permits`

Role/ClusterRole to a structured Permission node.

- range: `k8s:Permission`

#### `k8s:pinnedToNode`

From the PV's nodeAffinity (local volumes, hostPath on k3s).

- domain: `k8s:PersistentVolume`
- range: `k8s:Node`

#### `k8s:policyType`

Ingress or Egress.

- domain: `k8s:NetworkPolicy`
- range: `xsd:string`

#### `k8s:readyReplicas`

- domain: `k8s:Workload`
- range: `xsd:integer`

#### `k8s:replicas`

- domain: `k8s:Workload`
- range: `xsd:integer`

#### `k8s:replicasOnNode`

"<node-name>:<count>" so failure-domain spread can be computed without pods.

- domain: `k8s:Workload`
- range: `xsd:string`

#### `k8s:requestsCpu`

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:requestsMemory`

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:resource`

- domain: `k8s:Permission`
- range: `xsd:string`

#### `k8s:routesTo`

- domain: `k8s:Ingress`
- range: `k8s:Service`

#### `k8s:runsImage`

The image by digest (oci:sha256/<digest>). In live snapshots this is the digest that was pulled (containerStatuses[].imageID); in desired snapshots the digest in the spec, if any.

- domain: `k8s:ContainerSpec`
- range: `nix:Image`

#### `k8s:scheduledOn`

A node that ran at least one replica of the workload at snapshot time. Pods themselves are not stored.

- domain: `k8s:Workload`
- range: `k8s:Node`

#### `k8s:selects`

Service selector matched the workload's pod template labels.

- domain: `k8s:Service`
- range: `k8s:Workload`

#### `k8s:snapshotKind`

- domain: `k8s:Snapshot`
- range: `k8s:SnapshotKind`

#### `k8s:subjectOf`

Binding to a subject (ServiceAccount, User, Group).


#### `k8s:unresolvedImage`

Set (to the image reference) when the container has a mutable tag and no digest could be resolved. A policy violation by default.

- domain: `k8s:ContainerSpec`
- range: `xsd:string`

#### `k8s:usesServiceAccount`

- domain: `k8s:Workload`
- range: `k8s:ServiceAccount`

#### `k8s:usesStorageClass`

- range: `k8s:StorageClass`

#### `k8s:verb`

- domain: `k8s:Permission`
- range: `xsd:string`

### Individuals

#### `k8s:desiredState`

Read from rendered manifests (GitOps).


#### `k8s:liveState`

Read from the API server.


