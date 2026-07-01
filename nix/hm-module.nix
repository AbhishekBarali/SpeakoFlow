# Home-manager module for SpeakoFlow voice assistant
#
# Provides a systemd user service for autostart.
# Usage: imports = [ speakoflow.homeManagerModules.default ];
#        services.speakoflow.enable = true;
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.speakoflow;
in
{
  options.services.speakoflow = {
    enable = lib.mkEnableOption "SpeakoFlow voice assistant user service";

    package = lib.mkOption {
      type = lib.types.package;
      defaultText = lib.literalExpression "speakoflow.packages.\${system}.speakoflow";
      description = "The SpeakoFlow package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.speakoflow = {
      Unit = {
        Description = "SpeakoFlow voice assistant";
        After = [ "graphical-session.target" ];
        PartOf = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = "${cfg.package}/bin/speakoflow";
        Restart = "on-failure";
        RestartSec = 5;
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
