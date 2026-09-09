{
  description = "nix2rdf: Nix artifacts as a content-addressed RDF dataset, queried with SPARQL (Oxigraph), reasoned over with Nemo rule packs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    let
      # Everything per-system.
      perSystem = system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          inherit (pkgs) lib;

          # Nemo (vendored under vendor/nemo) needs nightly; the date is pinned
          # in rust-toolchain.toml and read from there so rustup and nix agree.
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          devToolchain = toolchain.override {
            extensions = [ "rust-src" "rust-analyzer" "rustfmt" "clippy" ];
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };

          # Native deps: RocksDB (via oxrocksdb-sys) is compiled from source and
          # its bindings are generated with bindgen, hence libclang.
          nativeBuildInputs = [ pkgs.pkg-config pkgs.cmake rustPlatform.bindgenHook ];
          buildInputs = [ pkgs.zstd ] ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.libiconv ];

          # Runtime tools the binary shells out to.
          runtimeDeps = [ pkgs.nix pkgs.nix-eval-jobs ];

          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./deny.toml
              ./rust-toolchain.toml
              ./crates
              ./vendor
              ./packs
              ./queries
              ./ontology
              ./fixtures
            ];
          };

          cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
          version = cargoToml.workspace.package.version;

          nix2rdf-unwrapped = rustPlatform.buildRustPackage {
            pname = "nix2rdf-unwrapped";
            inherit version src nativeBuildInputs buildInputs;
            cargoLock.lockFile = ./Cargo.lock;
            # Unit tests, pack tests, determinism and ontology tests all run
            # under `cargo test` (they only need the recorded fixtures).
            doCheck = true;
            cargoTestFlags = [ "--workspace" ];
            NIX2RDF_PACKS = "${src}/packs";
            postInstall = ''
              mkdir -p $out/share/nix2rdf
              cp -rL ${src}/packs $out/share/nix2rdf/packs
              cp -r ${src}/queries $out/share/nix2rdf/queries
              cp -r ${src}/ontology $out/share/nix2rdf/ontology
            '';
            meta = {
              description = "Nix artifacts as a content-addressed RDF dataset";
              mainProgram = "nix2rdf";
              license = lib.licenses.mit;
            };
          };

          # The installed binary finds `nix`/`nix-eval-jobs` on PATH and its
          # packs next to itself (share/nix2rdf/packs).
          nix2rdf = pkgs.symlinkJoin {
            name = "nix2rdf-${version}";
            paths = [ nix2rdf-unwrapped ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postBuild = ''
              wrapProgram $out/bin/nix2rdf \
                --prefix PATH : ${lib.makeBinPath runtimeDeps} \
                --set-default NIX2RDF_PACKS $out/share/nix2rdf/packs
            '';
            passthru = { unwrapped = nix2rdf-unwrapped; };
            inherit (nix2rdf-unwrapped) meta;
          };

          ontology-docs = pkgs.runCommand "nix2rdf-ontology-docs" { nativeBuildInputs = [ nix2rdf ]; } ''
            mkdir -p $out/versions/${version}
            cp ${./ontology}/nix.ttl $out/ns.ttl
            cp ${./ontology}/k8s.ttl $out/k8s.ttl
            cp ${./ontology}/nix.ttl $out/versions/${version}/ns.ttl
            cp ${./ontology}/k8s.ttl $out/versions/${version}/k8s.ttl
            nix2rdf ontology doc --html ${./ontology}/nix.ttl > $out/ns.html
            nix2rdf ontology doc --html ${./ontology}/k8s.ttl > $out/k8s.html
            nix2rdf ontology doc --markdown ${./ontology}/nix.ttl ${./ontology}/k8s.ttl > $out/ONTOLOGY.md
          '';

          # Checks that exercise the installed binary end to end on recorded
          # fixtures (no Nix evaluation inside the sandbox).
          mkCheck = name: script: pkgs.runCommand "nix2rdf-check-${name}" { nativeBuildInputs = [ nix2rdf pkgs.diffutils ]; } ''
            set -euo pipefail
            export HOME=$TMPDIR
            ${script}
            touch $out
          '';
          # Lint checks reuse the package derivation (vendored crates, toolchain,
          # native deps) with a different build phase.
          cargoCheck = name: phase: nix2rdf-unwrapped.overrideAttrs (old: {
            pname = "nix2rdf-${name}";
            nativeBuildInputs = old.nativeBuildInputs ++ [ devToolchain pkgs.cargo-deny ];
            buildPhase = phase;
            doCheck = false;
            installPhase = "touch $out";
          });
          lintTools = [ pkgs.nixpkgs-fmt pkgs.statix pkgs.deadnix pkgs.yamllint ];
          lintSrc = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [ ./flake.nix ./nix ./docs ./fixtures ./statix.toml ./deny.toml ];
          };
          yamllintConfig = pkgs.writeText "yamllint.yaml" ''
            extends: default
            rules:
              line-length: { max: 140 }
              document-start: disable
              truthy: disable
          '';
        in
        {
          packages = {
            default = nix2rdf;
            inherit nix2rdf ontology-docs;
            unwrapped = nix2rdf-unwrapped;
          };

          apps.default = {
            type = "app";
            program = "${nix2rdf}/bin/nix2rdf";
          };

          checks = {
            build = nix2rdf-unwrapped;
            # Rust: formatting, clippy with warnings as errors, license/ban audit.
            fmt = cargoCheck "fmt" "cargo fmt --all --check";
            clippy = cargoCheck "clippy" "cargo clippy --workspace --all-targets --offline -- -D warnings";
            deny = cargoCheck "deny" "cargo deny --offline check licenses bans sources";
            # Nix and YAML: formatting, anti-patterns, dead code, YAML style.
            lint = pkgs.runCommand "nix2rdf-lint" { nativeBuildInputs = lintTools; } ''
              cd ${lintSrc}
              nixpkgs-fmt --check flake.nix nix/*.nix fixtures/flake/flake.nix
              statix check .
              deadnix --fail .
              yamllint -c ${yamllintConfig} docs fixtures/k8s
              touch $out
            '';

            ontology = mkCheck "ontology" ''
              nix2rdf ontology validate --packs ${src}/packs ${./ontology}/nix.ttl ${./ontology}/k8s.ttl
            '';
            packs = mkCheck "packs" ''
              nix2rdf pack test --packs ${src}/packs --all
            '';
            determinism = mkCheck "determinism" ''
              nix2rdf --store $TMPDIR/a flake --from-recorded ${./fixtures/hello} --attr packages.aarch64-darwin.hello
              nix2rdf --store $TMPDIR/b flake --from-recorded ${./fixtures/hello} --attr packages.aarch64-darwin.hello
              cd $TMPDIR/a && find . -name '*.nq.zst' | sort > $TMPDIR/list
              while read f; do cmp "$TMPDIR/a/$f" "$TMPDIR/b/$f"; done < $TMPDIR/list
              echo "byte-identical: $(wc -l < $TMPDIR/list) fragments"
            '';
            e2e = mkCheck "e2e" ''
              nix2rdf --store $TMPDIR/s flake --from-recorded ${./fixtures/hello} --attr packages.aarch64-darwin.hello \
                --commit 0123456789abcdef0123456789abcdef01234567 --repo fixture --committed-at 2026-01-01T00:00:00Z
              nix2rdf --store $TMPDIR/s load --all
              nix2rdf --store $TMPDIR/s reason --pack core --input-closure --all-snapshots
              nix2rdf --store $TMPDIR/s check ${./fixtures/hello/checks/hello-in-closure.rq} && exit 1 || true
              nix2rdf --store $TMPDIR/s query ${./queries/pin-membership.rq} > $TMPDIR/out.json
              grep -q hello $TMPDIR/out.json
            '';
            k8s-determinism = mkCheck "k8s-determinism" ''
              nix2rdf --store $TMPDIR/a k8s snapshot --cluster-id fixture --manifests ${./fixtures/k8s/manifests} --observed-at 2026-01-01T00:00:00Z
              nix2rdf --store $TMPDIR/b k8s snapshot --cluster-id fixture --manifests ${./fixtures/k8s/manifests} --observed-at 2026-01-01T00:00:00Z
              diff <(cd $TMPDIR/a && find k8s -name '*.nq.zst' | sort) <(cd $TMPDIR/b && find k8s -name '*.nq.zst' | sort)
            '';
          };

          devShells.default = pkgs.mkShell {
            inherit buildInputs;
            nativeBuildInputs = nativeBuildInputs ++ [ devToolchain ] ++ runtimeDeps ++ [
              pkgs.cargo-nextest
              pkgs.cargo-deny
              pkgs.nixpkgs-fmt
              pkgs.statix
              pkgs.deadnix
              pkgs.yamllint
              pkgs.zstd
              pkgs.jq
              pkgs.kubectl
            ];
            NIX2RDF_PACKS = "${toString ./packs}";
            RUST_LOG = "info";
            shellHook = ''
              echo "nix2rdf dev shell: $(rustc --version)"
            '';
          };

          formatter = pkgs.nixpkgs-fmt;
        };
    in
    flake-utils.lib.eachDefaultSystem perSystem // {
      overlays.default = final: _prev: {
        nix2rdf = self.packages.${final.system}.nix2rdf;
      };
      nixosModules = {
        default = import ./nix/module.nix self;
        nix2rdf = import ./nix/module.nix self;
        node-annotator = import ./nix/node-annotator.nix;
      };
    };
}
