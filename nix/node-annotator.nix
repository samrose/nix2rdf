# NixOS module: annotate the local k3s/Kubernetes node with the hash of the
# NixOS generation it runs, so `nix2rdf k8s snapshot` can link
# k8s:Node → nix:NixosGeneration without a node map file.
#
#   services.nix2rdf-node-annotator.enable = true;
#
# The generation hash is the store-path hash of /run/current-system's target,
# which is exactly the identity nix2rdf uses for nix:gen/<hash>.
{ config, lib, pkgs, ... }:
let
  cfg = config.services.nix2rdf-node-annotator;
in
{
  options.services.nix2rdf-node-annotator = {
    enable = lib.mkEnableOption "annotating the local Kubernetes node with the NixOS generation hash";

    annotation = lib.mkOption {
      type = lib.types.str;
      default = "nix2rdf.io/generation";
      description = "Annotation key written on the node object.";
    };

    nodeName = lib.mkOption {
      type = lib.types.str;
      default = config.networking.hostName;
      defaultText = "config.networking.hostName";
      description = "The Kubernetes node name of this host.";
    };

    kubeconfig = lib.mkOption {
      type = lib.types.path;
      default = "/etc/rancher/k3s/k3s.yaml";
      description = "Kubeconfig with permission to annotate nodes (k3s' server kubeconfig by default).";
    };

    kubectl = lib.mkOption {
      type = lib.types.package;
      default = pkgs.kubectl;
      defaultText = "pkgs.kubectl";
      description = "kubectl package to use.";
    };

    after = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "k3s.service" ];
      description = "Units the API server depends on; the annotator waits for them.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.nix2rdf-node-annotator = {
      description = "Annotate the Kubernetes node with the running NixOS generation";
      wantedBy = [ "multi-user.target" ];
      after = cfg.after ++ [ "network-online.target" ];
      wants = [ "network-online.target" ];
      # Re-run on every activation (switch-to-configuration restarts changed units;
      # the script text embeds nothing generation-specific, so force it).
      restartIfChanged = true;
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = false;
        Environment = "KUBECONFIG=${cfg.kubeconfig}";
      };
      script = ''
        set -euo pipefail
        target=$(readlink -f /run/current-system)
        hash=$(basename "$target" | cut -d- -f1)
        for attempt in $(seq 1 30); do
          if ${cfg.kubectl}/bin/kubectl annotate node ${lib.escapeShellArg cfg.nodeName} \
               ${lib.escapeShellArg cfg.annotation}="$hash" --overwrite; then
            echo "annotated ${cfg.nodeName} with generation $hash"
            exit 0
          fi
          sleep 10
        done
        echo "could not reach the API server to annotate the node" >&2
        exit 1
      '';
    };

    # Run again after each switch-to-configuration.
    system.activationScripts.nix2rdf-node-annotator = lib.stringAfter [ "etc" ] ''
      if [ -d /run/systemd/system ]; then
        ${pkgs.systemd}/bin/systemctl start --no-block nix2rdf-node-annotator.service || true
      fi
    '';
  };
}
