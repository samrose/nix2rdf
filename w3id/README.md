# Registering `w3id.org/nix`

Status (2026-09-09): `curl -I https://w3id.org/nix` → 404, and there is no
`nix/` directory in `perma-id/w3id.org`, so the identifier is unclaimed.
Fallbacks, in order: `w3id.org/nix-ontology`, `w3id.org/nix2rdf`.

Steps:

1. Create the hosting target: a `nix-ontology` GitHub repository with Pages
   enabled, containing the output of `nix build .#ontology-docs`
   (`ns.ttl`, `ns.html`, `k8s.ttl`, `k8s.html`, `versions/<semver>/*.ttl`).
2. Fork https://github.com/perma-id/w3id.org, copy this directory's `nix/`
   into the fork root (`.htaccess` + `README.md`), open a PR.
3. After merge, verify:
   `curl -I -H "Accept: text/turtle" https://w3id.org/nix/ns` → 303 to `ns.ttl`;
   without the header → 303 to `ns.html`.
4. The namespace strings in `ontology/*.ttl`, `crates/nix2rdf-core/src/iri.rs`
   and IRI.md are already `https://w3id.org/nix/…`; nothing changes after
   registration. If a fallback name is needed, change them once, before the
   first published dataset.

Only the target URLs in `.htaccess` ever change afterwards; the identifiers
do not. Keep the contact in `nix/README.md` current.
