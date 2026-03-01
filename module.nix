{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.xzar-server;
  xzar-server = pkgs.xzar-server;
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
      hapi = {
        host = "::";
        port = cfg.port;
      };

      storage = cfg.storage;

      db = {
        client = "pg";
        connection = {
          host = "/run/postgresql";
          database = "xzar";
        };
        migrations = builtins.unsafeDiscardStringContext "${pkgs.xzar-server}/migrations";
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

    systemd.services.xzar-server = with pkgs; {
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      requires = [ "network-online.target" ];

      description = "xzar-server";

      environment.CONFIG = with builtins; toFile "config.json" (toJSON cfg.config);

      serviceConfig = {
        Type = "simple";
        DynamicUser = true;
        User = "xzar-server";
        StateDirectory = "xzar-server";
        ExecStart = "${xzar-server}/bin/xzar-server";
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
