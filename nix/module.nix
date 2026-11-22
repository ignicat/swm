self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.swm;
in
{
  options.programs.swm = {
    enable = lib.mkEnableOption "swm (simple window manager for Wayland)";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.swm;
      defaultText = lib.literalExpression "swm.packages.\${pkgs.stdenv.hostPlatform.system}.swm";
      description = "The swm package to use for the window manager, session and portal.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    services.displayManager.sessionPackages = [ cfg.package ];
    systemd.packages = [ cfg.package ];
    services.dbus.packages = [ cfg.package ];

    # Preserve the login environment imported by swm-session. Never restart
    # a running desktop as a side effect of nixos-rebuild switch.
    systemd.user.services.swm = {
      restartIfChanged = false;
      enableDefaultPath = false;
    };

    hardware.graphics.enable = lib.mkDefault true;
    services.graphical-desktop.enable = true;
    security.polkit.enable = true;
    security.pam.services.swaylock = { };
    programs.dconf.enable = lib.mkDefault true;
    services.xserver.desktopManager.runXdgAutostartIfNone = lib.mkDefault true;

    services.pipewire = {
      enable = lib.mkDefault true;
      wireplumber.enable = lib.mkDefault true;
    };

    xdg.portal = {
      enable = lib.mkDefault true;
      extraPortals = [
        cfg.package
        pkgs.xdg-desktop-portal-gtk
      ];
      configPackages = [ cfg.package ];
    };
  };
}
