# Testing

## CI workflows

Run on pushes to `main`, `pull_request`, and `workflow_dispatch` (no path filters).

- [Rust](../.github/workflows/rust.yml): Smithay upstream guard (`.github/scripts/check-smithay-source.py`), `cargo fmt --all -- --check`, `actionlint`, strict `clippy` + `rustdoc` (`-D warnings`), workspace tests/clippy/release builds/binary smoke checks across `default`, `minimal` (`--no-default-features`), and `dinit` (`--no-default-features --features dbus,dinit,xwayland,winit`). Pinned baseline: Rust `1.97.1` on `ubuntu-26.04` (needs `libdisplay-info` 0.3).
- [Nix](../.github/workflows/nix.yml): `nix flake check --no-update-lock-file --print-build-logs` on `x86_64-linux` and `aarch64-linux` (builds package, workspace tests, installed-resource checks, NixOS module check).
- [XWayland](../.github/workflows/xwayland.yml): runs the 11 tests listed in `.github/scripts/test-xwayland.sh` sequentially under headless `weston` + Mesa `llvmpipe` + private D-Bus (`2` `steam_close`, `3` `collapsed_node_close`, `2` X11 dialog focus/stacking, `1` `offscreen_pointer`, `3` `x11_fullscreen_negotiation`). Artifacts save to `target/ci-xwayland/` on failure for 7 days.

## Local checks

Install Ubuntu 26.04 system deps via `bash .github/scripts/install-dependencies.sh`, then:

```sh
python3 .github/scripts/check-smithay-source.py
cargo +1.97.1 fmt --all -- --check
cargo +1.97.1 clippy --locked --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo +1.97.1 doc --locked --workspace --all-features --no-deps
cargo +1.97.1 test --locked --workspace
```

Repeat test/clippy/release build with `--no-default-features` and `--no-default-features --features dbus,dinit,xwayland,winit` when changing features or platform drivers. For Nix: `nix flake check --no-update-lock-file --print-build-logs`.

Headless XWayland CI repro (after installing `weston xwayland libgl1-mesa-dri libegl-mesa0 fonts-dejavu-core`):

```sh
sudo install -d -m 1777 /tmp/.X11-unix
dbus-run-session -- bash .github/scripts/test-xwayland.sh
```

## Ignored nested & hardware suites

### DRM acquire sync

```sh
SWM_TEST_DRM_RENDER_NODE=/dev/dri/renderD128 \
  cargo test --locked --test drm_syncobj_protocol -- --ignored --test-threads=1
```

Opens only the chosen render node (no KMS takeover). Verifies timeline acquire readiness with real GBM DMA-BUFs; does not prove flicker-free scanout on a specific GPU/app.

### Nested Winit integration (`xwayland_disconnect`)

Multi-output and full `xwayland_disconnect` runs require a Wayland session, EGL, XWayland, and the private `cfg(test)` server binary (`cargo test --locked --test nested_fullscreen_server --no-run` for path):

```sh
SWM_TEST_WINIT_SERVER=/path/to/nested_fullscreen_server \
  cargo test --locked --test xwayland_disconnect <filter> -- --ignored --test-threads=1
```

`nested_fullscreen_server` ignores host mouse/keyboard input so tests stay isolated (`--test-threads=1` keeps nested compositors sequential). Targeted suites:

- `fullscreen_multi_output`: 12 cross-monitor fullscreen/Alt+Tab scenarios (Wayland + X11, games vs ordinary apps).
- `alt_tab_visibility`: visible vs offscreen target reveal rules ([alt-tab.md](alt-tab.md)).
- `offscreen_pointer`: pointer lock/confinement + XWayland grab recentering across scales.
- `x11_fullscreen_negotiation`: EWMH state/extents, game mode changes, minimize/remap, `Mod+F` restore ([xwayland.md](xwayland.md)).
- `resize_drag`: GTK4 Zathura PDF resize + titlebar drag + redundant `unset_fullscreen` checks (requires `zathura` + PDF plugin).
- `landmark_presentation`: 12 single-output scenarios verifying landmark positions underneath fullscreen/maximize (no `SWM_TEST_WINIT_SERVER` needed).
- `keyboard_leds`: unit tests (`cargo test --locked keyboard_leds`) + nested lock LEDs/keymap/IME modifier ordering checks.
- `multi_keyboard`: simulated libinput holder-count overlapping modifier tests across multiple keyboards.

Nested suites do not replace live TTY DRM/KMS, physical monitor hotplug/DPMS, or real game-engine testing.
