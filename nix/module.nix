# NixOS module: run nix2rdf as a service on a server.
#
#   services.nix2rdf = {
#     enable = true;
#     openFirewall = true;
#     index.enable = true;            # Layer 0 nixpkgs index, refreshed daily
#     fetchFragments.urls = [ "https://fragments.example.org/nixpkgs" ];
#     reason.packs = [ "core" "spdx3" ];
#   };
#
# One systemd service serves SPARQL; timers write fragments and restart the
# server, which bulk-loads new fragments on start. Fragment files under
# stateDir are canonical; the RocksDB index is rebuildable (`nix2rdf rebuild`).
self:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.nix2rdf;
  pkg = cfg.package;
  storeArg = "--store ${cfg.stateDir}";
  reasonCmd = lib.optionalString (cfg.reason.packs != [ ]) ''
    ${pkg}/bin/nix2rdf ${storeArg} reason ${lib.concatMapStringsSep " " (p: "--pack ${p}") cfg.reason.packs} --input-closure --all-snapshots
  '';
in
{
  options.services.nix2rdf = {
    enable = lib.mkEnableOption "the nix2rdf SPARQL service";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.nix2rdf;
      defaultText = "nix2rdf from the flake";
      description = "The nix2rdf package to run.";
    };

    stateDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/nix2rdf";
      description = "The fragment store (canonical files) and the Oxigraph index.";
    };

    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1";
      description = "Address the SPARQL endpoint binds to.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 7878;
      description = "Port of the SPARQL endpoint (GET/POST /query, /sparql).";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open the endpoint port in the firewall.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "nix2rdf";
      description = "User the services run as.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "nix2rdf";
      description = "Group the services run as.";
    };

    logLevel = lib.mkOption {
      type = lib.types.str;
      default = "info";
      description = "RUST_LOG filter for structured logs (journal, JSON lines).";
    };

    index = {
      enable = lib.mkEnableOption "the Layer 0 nixpkgs-multiverse index refresh";
      schedule = lib.mkOption {
        type = lib.types.str;
        default = "daily";
        description = "systemd OnCalendar expression for refreshing the index.";
      };
      source = lib.mkOption {
        type = lib.types.str;
        default = "github:fzakaria/nixpkgs-multiverse";
        description = "Flake reference of the nixpkgs-multiverse index.";
      };
    };

    fetchFragments = {
      urls = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Published fragment sets (Layer 1 evaluations, other teams' snapshots) to mirror into the store.";
      };
      schedule = lib.mkOption {
        type = lib.types.str;
        default = "hourly";
        description = "systemd OnCalendar expression for fetching fragments.";
      };
    };

    reason = {
      packs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "core" "spdx3" "policy-examples" ];
        description = "Packs to run over every snapshot after fragments change. Empty disables reasoning on the server.";
      };
    };

    extraServeArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Extra arguments for `nix2rdf serve`.";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users = lib.mkIf (cfg.user == "nix2rdf") {
      nix2rdf = {
        isSystemUser = true;
        group = cfg.group;
        home = cfg.stateDir;
        description = "nix2rdf service user";
      };
    };
    users.groups = lib.mkIf (cfg.group == "nix2rdf") { nix2rdf = { }; };

    systemd.tmpfiles.rules = [ "d ${cfg.stateDir} 0750 ${cfg.user} ${cfg.group} -" ];

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];

    # The server: loads any fragment not yet in the index, then serves SPARQL.
    systemd.services.nix2rdf-serve = {
      description = "nix2rdf SPARQL endpoint";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      environment = { RUST_LOG = cfg.logLevel; NIX2RDF_LOG_FORMAT = "json"; };
      serviceConfig = {
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDir;
        ExecStartPre = "${pkg}/bin/nix2rdf ${storeArg} load --new";
        ExecStart = "${pkg}/bin/nix2rdf ${storeArg} serve --listen ${cfg.listenAddress}:${toString cfg.port} ${lib.escapeShellArgs cfg.extraServeArgs}";
        Restart = "on-failure";
        RestartSec = 5;
        # Hardening.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.stateDir ];
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
      };
    };

    # Layer 0 index refresh: writes fragments (no index access), optionally
    # reasons, then restarts the server so it bulk-loads what is new.
    systemd.services.nix2rdf-index = lib.mkIf cfg.index.enable {
      description = "nix2rdf: refresh the nixpkgs-multiverse index (Layer 0)";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      environment = { RUST_LOG = cfg.logLevel; NIX2RDF_LOG_FORMAT = "json"; HOME = cfg.stateDir; };
      path = [ pkgs.nix ];
      script = ''
        ${pkg}/bin/nix2rdf ${storeArg} nixpkgs-index --from ${lib.escapeShellArg cfg.index.source}
        ${reasonCmd}
      '';
      serviceConfig = {
        Type = "oneshot";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDir;
        ExecStartPost = "+${pkgs.systemd}/bin/systemctl try-restart nix2rdf-serve.service";
      };
    };
    systemd.timers.nix2rdf-index = lib.mkIf cfg.index.enable {
      wantedBy = [ "timers.target" ];
      timerConfig = { OnCalendar = cfg.index.schedule; Persistent = true; RandomizedDelaySec = "10m"; };
    };

    systemd.services.nix2rdf-fetch = lib.mkIf (cfg.fetchFragments.urls != [ ]) {
      description = "nix2rdf: mirror published fragments";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      environment = { RUST_LOG = cfg.logLevel; NIX2RDF_LOG_FORMAT = "json"; };
      script = ''
        ${lib.concatMapStringsSep "\n" (u: "${pkg}/bin/nix2rdf ${storeArg} fetch-fragments ${lib.escapeShellArg u}") cfg.fetchFragments.urls}
        ${reasonCmd}
      '';
      serviceConfig = {
        Type = "oneshot";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDir;
        ExecStartPost = "+${pkgs.systemd}/bin/systemctl try-restart nix2rdf-serve.service";
      };
    };
    systemd.timers.nix2rdf-fetch = lib.mkIf (cfg.fetchFragments.urls != [ ]) {
      wantedBy = [ "timers.target" ];
      timerConfig = { OnCalendar = cfg.fetchFragments.schedule; Persistent = true; RandomizedDelaySec = "5m"; };
    };

    environment.systemPackages = [ pkg ];
  };
}
