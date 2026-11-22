# Wayland protocol support

Summary of client protocols, compositor policy, and hardware/nested boundaries beyond standard `wl_compositor` (v6) and XDG shell.

## Scaling, sync & frame timing

- **`wp_fractional_scale_manager_v1` (v1)**: tracks preferred integer/fractional scale per owning monitor across moves, transfers, popups, subsurfaces, and scale reloads ([display-scale.md](display-scale.md)).
- **`wp_fifo_manager_v1` (v1)**: lets clients hold committed updates until an output refresh opportunity (independent of GPU fences and frame callbacks). TTY releases captured barriers on page-flip; idle TTY/nested EGL uses a refresh estimate timer; a 1s fallback releases stalled barriers (collapsed/occluded/DPMS-off/removed). Tests: `cargo test -p swm --test client_compatibility_protocol fifo_tests --locked` and `fifo_nested`.
- **`wp_commit_timing_manager_v1` (v1)**: holds surface commits until their `CLOCK_MONOTONIC` timestamp (past/zero timestamps apply immediately; out-of-range ns `>= 1_000_000_000` or invalid requests emit protocol errors; synchronized subsurfaces wait on both parent and child; unblocked by FIFO's 1s fallback). Tests: `timing_tests` and `timing_nested`.
- **`wp_presentation` (v2)**: `CLOCK_MONOTONIC` feedback completed at DRM page-flip (`vsync`, `hw_clock`, `hw_completion`, per-surface `zero_copy`) on TTY, or host submit (`vsync`) on Winit. Only actually drawn surface elements receive feedback.
- **`wp_single_pixel_buffer_manager_v1` (v1)**, **`wp_content_type_manager_v1` (v1)**, **`xdg_toplevel_icon_manager_v1` (v1)**: Smithay state tracking (`toplevel-icon` metadata is stored without yet rendering client-supplied icons in swm chrome).

### Explicit & implicit GPU sync

On TTY, swm advertises `wp_linux_drm_syncobj_manager_v1` (v1) when the primary DRM node supports syncobj eventfd notifications and `debug.disable-explicit-sync` is `false` (default; omitted on Winit):

- Demand-driven: timelines/sources allocate only when a client attaches acquire/release points.
- Acquire waits block that surface transaction without stalling `calloop`, keeping buffer/scanout refs until primary-context reads finish.
- Workaround for driver/app flicker:
  ```rune
  debug:
    disable-explicit-sync true
  end
  ```
  Requires restarting swm (live reload logs that a restart is needed).
- **Implicit sync**: unready DMA-BUF planes wait asynchronously on readiness fences without blocking `calloop`. If readiness event-source registration fails, `buffer_readiness::reject_commit` installs a blocked barrier and terminates the client with `WL_DISPLAY_ERROR_IMPLEMENTATION` (`3`).

## Input, pointer & IME

- **`wp_pointer_warp_v1` (v1)**: surface-local cursor warps keyed by current pointer-enter serial, mapped through Field/scale/fullscreen/subsurface geometry; respects pointer locks/confinement and ignores stale serials or locked sessions (`tests/support/pointer_warp.rs`).
- **`zwp_text_input_manager_v3` (v1) & `zwp_input_method_manager_v2` (v1)**: `input-method-v2` is gated by `SWM_ALLOW_INPUT_METHOD` ([security.md](security.md)); XWayland keeps X11 IME. (`v2` in `zwp_input_method_manager_v2` is protocol generation; advertised interface version is `1`; `text-input-v3` advertises v1.) On session lock, IME suspends in place (discarding pending preedit, blocking lock-screen keystrokes/grabs) and resumes cleanly on unlock. Fcitx5 also needs `SWM_ALLOW_VIRTUAL_KEYBOARD`. Smoke tests: `tests/text_input_protocol.rs`.
- **Keyboard shortcuts inhibitor / exclusive layer-shell focus**: compositor shortcuts still evaluate unless locked or inhibited by the focused client.

## Window chrome, dialogs & popups

- **`zxdg_exporter_v2` / `zxdg_importer_v2` (v1) & `xdg_wm_dialog_v1` (v1)**: cross-connection portal parenting and modal focus redirection to frontmost mapped child dialog.
- **Popups**: XDG popups and X11 override-redirect menus promote above `Top` layer-shell bars (below `Overlay` and compositor UI) and inverse-map constraints through live fullscreen/maximize presentation.
- **Initial size recovery**: clients requesting maximize on map receive a 3/4-work-area configure (delayed until first commit so rules/recovery take precedence). Closing a fullscreen/maximized client caches a bounded non-fullscreen session size in memory for the next launch of that app ID (explicit window rules override it; never written to disk).

## Clipboard, idle & session lock

- **`ext_data_control_manager_v1` (v1)**: visible only to `SWM_ALLOW_CLIPBOARD_CONTROL` peers ([security.md](security.md)). Shares seat selections zero-copy via pipe FDs without compositor buffering.
- **`ext_idle_notifier_v1` (v2) & `zwp_idle_inhibit_manager_v1` (v1)**: physical input updates idle timers even when compositor chooser/modal UI grabs input. Surface inhibitors suppress idle only while actually visible in a composed frame.
- **D-Bus `org.freedesktop.ScreenSaver`** (TTY `dbus` feature): native `Inhibit`/`UnInhibit` fallback merged with Wayland inhibitors; nested Winit leaves host service alone.
- **`ext_session_lock_manager_v1` (v1)**: paints black fail-closed scenes across powered outputs and waits for actual scanout/submission before sending `locked`. Blocks bindings, capture, virtual keyboard, and IME until the active lock unlocks (stays locked if the locker crashes).

## Outputs, gamma & foreign toplevels

- **`zwlr_output_manager_v1` (v4)**: atomic multi-head configuration (`0.25..=10.0` scale rounded to `1/120`, rejects custom modes and paused-seat applies; Winit allows local scale only).
- **`zwlr_output_power_manager_v1` (v1)** (TTY only): gated by `SWM_ALLOW_OUTPUT_POWER` ([output-power.md](output-power.md)).
- **`zwlr_gamma_control_manager_v1` (v1)** (TTY only): one controller per CRTC, native `u16` R/G/B LUT blobs, restores linear ramp on release/disable/VT leave.
- **`ext_foreign_toplevel_list_v1` (v1) & `zwlr_foreign_toplevel_manager_v1` (v3)**: lists active + collapsed managed windows (excludes popups/layers/X11 OR), supports taskbar activate/close/minimize/maximize/fullscreen (skipped when locked/grabbed). Ignored integration tests in `tests/foreign_toplevel_protocol.rs`: set `SWM_TEST_WAYLAND_DISPLAY` (plus `SWM_TEST_X11_DISPLAY` for `nested_swm_xwayland_actions_and_override_redirect_exclusion`).

## Screen capture

- **`zwlr_screencopy_manager_v1` (v3)**: gated by `SWM_ALLOW_SCREEN_CAPTURE` ([security.md](security.md)). Supports XRGB8888 SHM/DMA-BUF, cursor toggle, `copy_with_damage`. Fails during session lock or on disabled/missing outputs.
- **`ext_image_copy_capture_manager_v1` & `ext_output_image_capture_source_manager_v1` (v1)**: gated by `SWM_ALLOW_SCREEN_CAPTURE`. Whole-output XRGB8888 SHM captures at transformed native pixels (no DMA-BUF or toplevel capture yet). Tests: `image_copy_capture_protocol` + `image_copy_nested`.
