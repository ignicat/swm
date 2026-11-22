# Building and packaging

For Nix/NixOS see [docs/nix.md](nix.md).

## Feature flags

| Feature | Default | Scope |
| --- | --- | --- |
| `dbus` | yes | In-process D-Bus services (`ScreenSaver`, accessibility keyboard monitor) |
| `systemd` | yes | `sd_notify` readiness + systemd user session publishing |
| `dinit` | no | dinit readiness + user session publishing |
| `xwayland` | yes | XWayland server + XWM integration |
| `winit` | yes | Nested development/test backend |

Note: `dbus-update-activation-environment` works regardless of `dbus`, and `xdg-desktop-portal-swm` (`swm-portal` workspace crate) does not depend on swm's `dbus` feature.

```sh
cargo build --release --workspace                                               # default (systemd)
cargo build --release --workspace --no-default-features --features dbus,dinit,xwayland,winit  # dinit
cargo build --release -p swm --no-default-features                              # minimal Wayland-only TTY
```

Without `winit`, `--winit` or auto-detected `WAYLAND_DISPLAY`/`DISPLAY` fails cleanly instead of touching TTY DRM (`--session` still forces TTY).

Companion tools (`swm-lift`, `swm-screenshoter`) are packaged independently.

## Installed `/usr` layout

| Artifact | Path |
| --- | --- |
| `swm`, `swmctl`, `xdg-desktop-portal-swm` | `/usr/bin/` |
| `packaging/wayland-sessions/swm-session` | `/usr/bin/` |
| `packaging/wayland-sessions/swm.desktop` | `/usr/share/wayland-sessions/` |
| `packaging/xdg-desktop-portal/swm-portals.conf` | `/usr/share/xdg-desktop-portal/` |
| `packaging/xdg-desktop-portal/portals/swm.portal` | `/usr/share/xdg-desktop-portal/portals/` |
| `packaging/dbus-1/services/org.freedesktop.impl.portal.desktop.swm.service` | `/usr/share/dbus-1/services/` |
| `packaging/systemd-user/*` | `/usr/lib/systemd/user/` |
| `packaging/dinit/*` | `/usr/lib/dinit.d/user/` |

Install systemd units only with `systemd`, dinit units only with `dinit` (dinit unit expects a `dbus` user service). `runit/`, `s6/`, and `openrc/` contain supervision examples/README.

## Session launcher & lifecycle

`swm-session` imports login-shell env, then picks:
1. Active systemd user manager (when `swm.service` matches selected `SWM_BIN` and no extra compositor CLI args were passed)
2. Active dinit user manager (when `dinit` units exist, `/usr/bin/swm` is selected, and no extra compositor CLI args were passed)
3. Direct `exec "$SWM_BIN" --session "$@"` fallback (`SWM_NO_INIT_INTEGRATION=1` forces this; if `/usr/bin/swm` is absent, falls back to `swm` on `PATH`).

`SWM_BIN` overrides sibling `/usr/bin/swm` lookup. Logs append to `$XDG_RUNTIME_DIR/swm-session.log`.

- **Managed systemd (`swm.service`)**: `Type=notify` starts graphical targets after listeners bind; `swm-session` stops `swm-shutdown.target` on exit.
- **Direct native (`swm` / `swm --session`)**: starts `swm-direct-session.target` -> `graphical-session.target` itself and stops session services/env on exit (`SWM_NO_INIT_INTEGRATION=1` disables this). Nested Winit sessions skip target management.
- **Portal drop-ins**: `packaging/systemd-user/xdg-desktop-portal*.service.d/swm-session.conf` tie D-Bus activation to `graphical-session.target` so GTK doesn't spawn headless between logins (keep `systemctl --user daemon-reload` in sync; omit drop-ins on non-systemd setups).
- Startup publishes display env and signals readiness before async XDG Settings `color-scheme` queries so portal startup never stalls `calloop`.
