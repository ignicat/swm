# XWayland policy and conformance baseline

swm layers spatial policy on top of unmodified upstream Smithay's XWM (`WM_S0`, reparenting, X11 events, selection/DND, `_NET_CLIENT_LIST*`). Reference specs: [ICCCM](https://www.x.org/releases/current/doc/xorg-docs/icccm/icccm.pdf) and [EWMH](https://specifications.freedesktop.org/wm/latest-single/).

## Ownership boundary

- Smithay owns `WM_S0`, `_NET_CLIENT_LIST`, `_NET_CLIENT_LIST_STACKING`, and client events. swm synchronizes stacking order via `X11Wm::update_stacking_order_upwards` (bottom-to-top, guarded by a compare memo so no-op sweeps avoid extra X server grabs).
- swm's auxiliary X11 connection updates root EWMH properties (`_NET_SUPPORTED`, support window `"SWM"`, desktop geometry, `_NET_ACTIVE_WINDOW`, allowed actions, `_NET_FRAME_EXTENTS`, XKB repeat) and explicitly publishes RandR primary output via `randr_set_output_primary` from swm's primary/first output (`randr_primary_output_change` notifications from external callers are ignored).

## Cursor theme

`cursor.theme` / `cursor.size` export `XCURSOR_THEME` / `XCURSOR_SIZE` for swm children and native session activation, and update XWayland's default root cursor on startup/reload (individual apps or toolkits may still override via their own theme config or env).

## Lifecycle & configure

- Each XID carries a monotonic generation so reused XIDs never inherit stale state. States: `Withdrawn` -> `Normal` (`NormalState`) -> `Iconic` (`IconicState` + `_NET_WM_STATE_HIDDEN` when collapsed to a node).
- `_NET_ACTIVE_WINDOW` updates only on effective change; stale activation timestamps and ICCCM urgency map to demands-attention until focused or unmapped.
- `xwayland::xwm::configure` enforces `WM_NORMAL_HINTS` (min/max, base + resize increments, aspect ratio) safely against malformed/overflow hints. swm owns ordinary window coordinates (denying move/restack with synthetic `ConfigureNotify`), honors transient offsets, and anchors left/top resizes after increment snapping.
- **Coordinate boundary**: `Space` uses Field coords while `X11Surface::last_configure()` holds X root desktop coords (`X11Surface::geometry()` is surface-local only). Owner-linked X11 OR popups preserve root delta relative to their parent's published X origin and inherit the parent's Field presentation transform so menus track zoom/pan cleanly.

## Fullscreen & games

- Game windows keep accepted output geometry authoritative across RandR/mode switches, answer even no-op mode requests cleanly, and zero `_NET_FRAME_EXTENTS` in fullscreen (restored on exit).
- With fullscreen animations off, transitions wait for the client's resized repaint up to `150ms` (`EXTERNAL_COMMIT_WAIT`) before switching.
- Compositor fullscreen (`Mod+F`) raises X11 windows once on entry; client fullscreen requests and exit preserve stack slot ([keybinds.md](keybinds.md)). Leaving fullscreen fits oversized saved frames into the usable work area without rewriting the game's internal video config. First-time windowed restore keeps frame center.

## Advertised EWMH subset

`_NET_SUPPORTED` omits unimplemented features: no `_NET_CLOSE_WINDOW`/`_NET_WM_ACTION_CLOSE` (or `_NET_REQUEST_FRAME_EXTENTS`, which would bypass Smithay's root event mask), shading, sticky state, or above/below stacking layers.

## Validation

Unit tests cover XID generations, 32-bit timestamps, `WM_NORMAL_HINTS` math, and fullscreen policy. For nested Winit integration tests in `tests/xwayland_disconnect.rs` (including live map/stack/fullscreen/minimize/XWayland checks), see [docs/testing.md](testing.md).

Local sanity checks:
1. Inspect `_NET_SUPPORTED`, support window `"SWM"`, `_NET_CLIENT_LIST*`, `_NET_FRAME_EXTENTS`, and `xrandr --listmonitors` matching swm's active output order.
2. Verify collapse sets `IconicState` + hidden, clears `_NET_ACTIVE_WINDOW`, and restore flips back cleanly.
3. Verify Field pan/zoom/fullscreen restore keeps X root coordinates consistent.
