# Nix and NixOS

`flake.nix` exports `packages.<system>.swm` (`default`), a `devShells` environment, `overlays.default`, and `nixosModules.{default, swm}` for `x86_64-linux` and `aarch64-linux`. The package builds `swm`, `swmctl`, `xdg-desktop-portal-swm`, `swm-session`, and packaging units, with `xwayland` wrapped into `PATH`.

Companion tools (`swm-lift`, `swm-screenshoter`), terminals, bars, notification daemons, and polkit agents live in separate packages. Default keybinds call `swm-lift` (`Super+D`) and `swm-screenshoter` (`Print`); install them separately or change those lines in `swm.rune`.

## Package build & nested run

```sh
nix build
./result/bin/swm --version
nix run . -- --winit
```

`nix profile add .` installs binaries locally without registering display-manager sessions or system portals.

## NixOS module

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    swm = {
      url = "github:<owner>/swm";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, swm, ... }: {
    nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        swm.nixosModules.default
        { programs.swm.enable = true; }
      ];
    };
  };
}
```

The module enables session registration, systemd user units, graphics, Polkit, PipeWire/WirePlumber, `xdg-desktop-portal-swm` (ScreenCast), and `xdg-desktop-portal-gtk` fallback. User `~/.config/swm/swm.rune` is never overwritten. Override the derivation with `programs.swm.package` or `swm.overlays.default` (`pkgs.swm`).

For `flake.lock` updates run `nix flake update swm`; whenever `Cargo.lock` updates the Smithay Git commit, update the fixed-output hash in `nix/package.nix` too. See [wayland-protocols.md#explicit--implicit-gpu-sync](wayland-protocols.md#explicit--implicit-gpu-sync) for `debug.disable-explicit-sync`.

## Checks

```sh
nix develop
cargo check --workspace --locked
nix flake check --no-update-lock-file --print-build-logs
```

`nix flake check` builds the package, runs non-ignored workspace tests, and validates `nix/module-check.nix` (hardware/GPU/live-compositor tests stay ignored). Run `--all-systems` only with both builders available.
