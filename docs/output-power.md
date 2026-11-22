# Output power management

On the native TTY backend, swm exposes `zwlr_output_power_manager_v1` (`wlr-output-power-management-unstable-v1`, v1) for initialized, enabled outputs. Nested Winit leaves power to the host compositor and never creates this global.

## Permissions and DPMS behavior

Set `SWM_ALLOW_OUTPUT_POWER` to a colon-separated list of absolute executable paths before launching swm (e.g. `SWM_ALLOW_OUTPUT_POWER=/usr/bin/wlopm`). Only approved peers see the global; restart tools if swm restarts with updated policy ([security](security.md)).

- Shares the same DPMS path as `swmctl dpms` and input wake. Power-off keeps logical membership, configured mode/scale/transform/position, and window positions intact.
- `swmctl dpms off` and `toggle` wait up to 1s for held keys to release before blanking displays (preventing `Return` release from immediately waking them back up). Protocol writes and `swmctl dpms on` apply immediately.
- **AMD RDNA3 wake workaround** (`src/backend/tty/wake.rs`): brings multi-display wakes up sequentially (capped at 1s per display until an ordinary frame commits) to work around [drm/amd#3504](https://gitlab.freedesktop.org/drm/amd/-/issues/3504) (~25s kernel stall on concurrent wake). Temporary until the kernel fix lands.
- Controls receive initial state immediately and updates across protocol/IPC/input wake. Repeating the current mode succeeds cleanly without spurious events; destroying a control/manager leaves power unchanged.

## Failure & lifecycle

- Disabled, unsupported, disconnected outputs or failed writes send `failed` (control becomes inert; recreate on reconnect). Unknown numeric modes raise `invalid_mode` and disconnect the client.
- Writes while paused by VT switch or sleep fail (even for an already-matching mode) to keep DRM state consistent.
- Connector hotplug allocation happens at startup; `UdevBackend` removal invalidates controls for unplugged connectors.
- Locked sessions still treat powered-off outputs as secure, and waking an output while locked renders the fail-closed lock screen only.

## Validation

- Socket suite: `cargo test --locked --test output_power_protocol` covers modes, idempotency, multiple observers, invalidation, paused writes, and `invalid_mode`.
- Physical hardware checks (2 outputs, DPMS off/on, unplug, VT switch, session lock) still require a live TTY session.
