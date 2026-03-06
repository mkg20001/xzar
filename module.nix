{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.xzar-server;
  xzar-server = pkgs.xzar-server;

  configFile = pkgs.writeText "xzar-config.yaml" (builtins.toJSON cfg.config);

  xzar-server-wrapped = pkgs.writeShellScriptBin "xzar-server" ''
    export XZAR_CONFIG=${configFile}
    exec ${xzar-server}/bin/xzar-server "$@"
  '';
in
{
  options = {
    services.xzar-server = {
      enable = mkEnableOption "xzar-server";

      port = mkOption {
        description = "Port to listen at";
        type = types.int;
        default = 17788;
      };

      storage = mkOption {
        description = "Path to store NAR files at";
        type = types.str;
        default = "/var/lib/xzar-server/storage";
      };

      config = mkOption {
        description = "configuration";
        type = types.attrs;
      };

      openFirewall = mkOption {
        type = types.bool;
        default = false;
        description = "Open ports in the firewall for xzar-server.";
      };
    };
  };

  config = mkIf (cfg.enable) {
    services.xzar-server.config = {
      rocket = {
        host = "::";
        port = cfg.port;
      };

      storage = cfg.storage;

      db = {
        client = "pg";
        connection = "postgres://xzar-server@/xzar?host=/run/postgresql";
      };
    };

    services.postgresql = {
      enable = true;

      ensureUsers = [{
        name = "xzar-server";
        ensureClauses.superuser = true;
        # ensurePermissions = { "DATABASE xzar" = "ALL PRIVILEGES"; };
      }];

      ensureDatabases = [ "xzar" ];
    };

    networking.firewall = mkIf cfg.openFirewall {
      allowedTCPPorts = [ cfg.port ];
    };

    environment.systemPackages = [ xzar-server-wrapped ];

    systemd.services.xzar-server = {
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      requires = [ "network-online.target" ];

      description = "xzar-server";

      serviceConfig = {
        Type = "simple";
        DynamicUser = true;
        User = "xzar-server";
        StateDirectory = "xzar-server";
        ExecStart = "${xzar-server-wrapped}/bin/xzar-server serve";
      };
    };

    users.users.xzar-server = {
      isSystemUser = true;
      description = "Xzar Server";
      group = "xzar-server";
    };

    users.groups.xzar-server = {};
  };
}
