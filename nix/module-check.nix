{
  nixpkgs,
  system,
  self,
}:
let
  pkgs = nixpkgs.legacyPackages.${system};
  evaluated = nixpkgs.lib.nixosSystem {
    inherit system;
    modules = [
      self.nixosModules.default
      {
        programs.swm.enable = true;
        boot.loader.grub.enable = false;
        fileSystems."/" = {
          device = "none";
          fsType = "tmpfs";
        };
        system.stateVersion = "26.05";
      }
    ];
  };
  cfg = evaluated.config;
in
assert builtins.all (entry: entry.assertion) cfg.assertions;
assert builtins.elem cfg.programs.swm.package cfg.services.displayManager.sessionPackages;
assert builtins.elem cfg.programs.swm.package cfg.xdg.portal.extraPortals;
assert cfg.hardware.graphics.enable;
assert cfg.services.pipewire.enable;
assert !cfg.systemd.user.services.swm.restartIfChanged;
assert !cfg.systemd.user.services.swm.enableDefaultPath;
pkgs.runCommand "swm-nixos-module-check" { } ''
  touch "$out"
''
