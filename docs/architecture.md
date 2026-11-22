# swm architecture

Balanced module boundaries keep platform backends, spatial math, protocol handling, and GLES rendering isolated.

## Dependency direction & subsystem layout

- `swm-core`: pure Field, node, focus, trail, camera, viewport, and collision physics math. Zero Wayland or Smithay deps.
- `session`: top-level compositor policy coordinating `RuntimeSettings` (config reload), `InteractionState` (input transactions), and submodules under `src/session/`: `capture/`, `diagnostics/`, `display/`, `drivers/`, `gesture/`, `input/` (incl. `hit_test.rs`), `pointer/` (`constraints.rs`, `warp.rs`), `protocol/`, `startup/` (`autostart.rs`, `environment.rs`, etc.), `windows/` (`window_presentation.rs`), and `workspace/` (`node_controls.rs`, `navigation.rs`, `zoom_fit.rs`).
- `backend`: segregated `RenderDriver`, `OutputDriver`, and `SessionDriver` traits implemented by `tty` (DRM/GBM/EGL/DPMS/VRR) and `winit`.
- `presentation`: shared camera transforms, window presentation, and maximize state consumed symmetrically by rendering and input mapping.
- `render`: `RenderState` + scene builders (`scene/`, `overlays/`, `shaders/`, `window_decoration.rs`, `window_shader.rs`, `text.rs`). Crosses `session`/`render` strictly via 6 immutable/explicit contexts (`FrameContext`, `DesktopContext`, `CursorContext`, `OverlayContext`, `VisualContext`, `RenderResources`).
- `window`: backend-neutral window rules (`rules.rs`), game classification (`games.rs`), stacking, closing, recovery (`xdg_shell` and `xwayland` feed identities in without duplicating policy).
- `shell`: renderer-independent overlays (`ShellState`, `OverlayManager`, `FocusCycleState`).
- `capture`: portal source chooser, `CaptureFrame` readback (`readback.rs`), screencast DMA-BUF pool (`screencast.rs`), and capture geometry (`window_geometry.rs`).
- `wayland` & `xwayland`: protocol state machines; X11 isolates `control.rs`, `configure.rs`, `policy.rs` (`ExternalPresentationPolicy`), `override_redirect.rs`, `presentation.rs`, and `lifecycle.rs`.
- `animation`: pure elapsed-time curves and springs with zero GLES/backend knowledge.
- `swm-cli`: splits `cmd/` (per-area parsers), `help.rs`, `print.rs`, `config.rs`, and `main.rs` cleanly over `swm-api`.
- `swm-ipc`: internal postcard framing (`SWM_IPC_VERSION` in `swm-ipc/src/types.rs`).

## Protected boundaries

- `session::pointer::constraints`: lock and confinement geometry algorithms live here alone; refactors and renderers must not duplicate constraint math.
- Explicit sync / implicit acquire fences: never block the `calloop` event loop waiting on client GPU work.

## Merge gates

```sh
python3 .github/scripts/check-smithay-source.py
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
```

Change `SWM_IPC_VERSION` in `swm-ipc/src/types.rs` only for intentional wire breaks; preserve config key spelling and `swm-api` semantic compatibility.
