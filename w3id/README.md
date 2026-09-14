# Registering `w3id.org/nix2rdf`

The namespace is the project's own name, not `w3id.org/nix`. "Nix" and
"NixOS" are marks of the NixOS Foundation; a bare `/nix` namespace on a shared
permanent-identifier host would read as the official Nix vocabulary, which
this project is not, and w3id identifiers are permanent, so the name has to
be right before the first dataset is published.

Status (2026-09-14): `curl -I https://w3id.org/nix2rdf` → 404 and there is no
`nix2rdf/` directory in `perma-id/w3id.org`, so the identifier is unclaimed.

Steps:

1. Create the hosting target: a public `nix2rdf-ontology` GitHub repository
   with Pages enabled, containing the output of `nix build .#ontology-docs`
   (`ns.ttl`, `ns.html`, `k8s.ttl`, `k8s.html`, `versions/<semver>/*.ttl`).
2. Fork https://github.com/perma-id/w3id.org, copy this directory's
   `nix2rdf/` to `ids/nix2rdf/` in the fork (`.htaccess` + `README.md`),
   open a PR.
3. After merge, verify:
   `curl -I -H "Accept: text/turtle" https://w3id.org/nix2rdf/ns` → 303 to
   `ns.ttl`; without the header → 303 to `ns.html`.
4. The namespace strings in `ontology/*.ttl`, `crates/nix2rdf-core/src/iri.rs`
   and IRI.md are `https://w3id.org/nix2rdf/…`; nothing changes after
   registration.

Only the target URLs in `.htaccess` ever change afterwards; the identifiers
do not. Keep the contact in `nix2rdf/README.md` current.
