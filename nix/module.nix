# NixOS module for SpeakoFlow voice assistant
#
# Handles system-level configuration that the package wrapper cannot:
#   - udev rule for /dev/uinput (rdev grab() needs it for virtual input)
#
# Note: users must add themselves to the "input" group for evdev hotkey access.
#
# Usage in your flake:
#
#   inputs.speakoflow.url = "github:AbhishekBarali/SpeakoFlow";
#
#   nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
#     modules = [
#       speakoflow.nixosModules.default
#       { programs.speakoflow.enable = true; }
#     ];
#   };
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.speakoflow;
in
{
  options.programs.speakoflow = {
    enable = lib.mkEnableOption "SpeakoFlow offline voice assistant";

    package = lib.mkOption {
      type = lib.types.package;
      defaultText = lib.literalExpression "speakoflow.packages.\${system}.speakoflow";
      description = "The SpeakoFlow package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    # rdev grab() creates virtual input devices via /dev/uinput.
    # Default permissions are crw------- root root — open it to the input group.
    services.udev.extraRules = ''
      KERNEL=="uinput", GROUP="input", MODE="0660"
    '';
  };
}
