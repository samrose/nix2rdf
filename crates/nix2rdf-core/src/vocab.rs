//! Every vocabulary term the Rust code emits, as constants, so a typo is a
//! compile error and the ontology validation test can check that each one is
//! declared in ontology/*.ttl.

use crate::iri::{k8s, nix, xsd};
use oxrdf::NamedNode;

macro_rules! terms {
    ($ns:ident; $($name:ident),* $(,)?) => {
        $( #[allow(non_snake_case)] pub fn $name() -> NamedNode { $ns(stringify!($name)) } )*
        pub const ALL: &[&str] = &[ $( stringify!($name) ),* ];
    };
}

pub mod nix_terms {
    use super::*;
    terms!(nix;
        // classes
        Snapshot, Derivation, Output, Source, FlakeInput, FlakePin, NixpkgsRevision, Release,
        PackageVersion, Attribute, Commit, NixosConfiguration, NixosGeneration, NixosOption,
        OptionValue, OptionDefinition, Image, Layer, Violation, OntologyPack, RuleRun, Fragment,
        // object properties
        inputDrv, dependsOn, inputSrc, hasOutput, references, evaluatesTo, closureContains,
        hasRoot, pinnedTo, locksInput, follows, snapshot, configuration, fromPin, hasOptionValue,
        option, hasDefinition, hasLayer, layerContains, imageDerivation, derivedBy, usesPack,
        inputGraph, dependsOnPack, violates, subject, inSnapshot, hasAttribute, lastSeenIn,
        releasePinnedTo, hasFragment,
        // datatype properties
        drvPath, storePath, name, pname, version, system, builder, arg, outputName,
        inputDrvOutput, envHash, envVar, structuredAttrs, contentAddressed, license, homepage,
        description, sourceProvenance, unfree, attrPath, narHash, rev, r#ref, lastModified,
        committedAt, repo, lockedType, lockedOwner, lockedRepo, lockedUrl, lockedDir,
        originalRef, isFlake, lockNodeId, channelName, date, indexOffset, currentAtIndexTip,
        releaseBuild, optionPath, optionType, declaredIn, definedIn, finalValue, definitionValue,
        valueHash, value, isDefined, digest, mediaType, layerIndex, imageTool, imageName,
        imageTag, packName, packVersion, rulesetHash, inputHash, ranAt, derivedQuadCount,
        nemoVersion, fragmentKind, message,
    );
}

pub mod k8s_terms {
    use super::*;
    terms!(k8s;
        Cluster, Node, Namespace, Workload, Deployment, StatefulSet, DaemonSet, Job, CronJob,
        ContainerSpec, Service, Ingress, ConfigMap, Secret, PersistentVolumeClaim,
        PersistentVolume, StorageClass, NetworkPolicy, ServiceAccount, User, Group, Role,
        ClusterRole, RoleBinding, ClusterRoleBinding, PodDisruptionBudget, Permission, Snapshot,
        SnapshotKind, liveState, desiredState, FailureDomain, Owner,
        contains, snapshotKind, ofCluster, inCluster, inNamespace, hasContainer, runsImage,
        scheduledOn, selects, routesTo, mounts, boundTo, pinnedToNode, usesStorageClass,
        usesServiceAccount, grants, permits, subjectOf, aggregates, allowsIngressFrom,
        allowsEgressTo, appliesTo, inFailureDomain, hasPDB, ownedBy, hostGeneration,
        clusterId, name, kind, apiVersion, label, imageRef, imageTag, imageRepository,
        unresolvedImage, initContainer, replicas, readyReplicas, replicasOnNode, requestsCpu,
        requestsMemory, limitsCpu, limitsMemory, verb, resource, apiGroup, domainKind,
        ownerUnknown, costCenter, observedAt, partial, failedKind, nodeRole, kubeletVersion,
        containerRuntimeVersion, kernelVersion, osImage, policyType, allowsAllIngress,
        allowsAllEgress, minAvailable, maxUnavailable, danglingRef, exemptSingleDomain,
    );
}

pub fn xsd_string() -> NamedNode {
    xsd("string")
}
pub fn xsd_integer() -> NamedNode {
    xsd("integer")
}
pub fn xsd_boolean() -> NamedNode {
    xsd("boolean")
}
pub fn xsd_date() -> NamedNode {
    xsd("date")
}
pub fn xsd_date_time() -> NamedNode {
    xsd("dateTime")
}
pub fn xsd_any_uri() -> NamedNode {
    xsd("anyURI")
}
