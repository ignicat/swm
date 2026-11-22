# Desktop capability permissions

Wayland clients start with zero privileged desktop globals. Capabilities are explicit opt-in via environment variables set before launching swm.

Note: these restrict Wayland/D-Bus protocol visibility, not arbitrary unsandboxed processes running as your own Unix UID (anything that can rewrite your session env or config already shares your user trust boundary).

## Privileged Wayland globals

Set each variable to a colon-separated list of **absolute executable paths**:

| Env var | Granted protocols | Example |
| --- | --- | --- |
| `SWM_ALLOW_SCREEN_CAPTURE` | `zwlr_screencopy_manager_v1`, `ext_image_copy_capture_manager_v1`, `ext_output_image_capture_source_manager_v1` | `/usr/bin/grim` |
| `SWM_ALLOW_CLIPBOARD_CONTROL` | `ext_data_control_manager_v1` | `/usr/bin/wl-paste` |
| `SWM_ALLOW_VIRTUAL_KEYBOARD` | `zwp_virtual_keyboard_manager_v1` | `/usr/bin/wtype` |
| `SWM_ALLOW_INPUT_METHOD` | `zwp_input_method_manager_v2` | `/usr/bin/fcitx5` |
| `SWM_ALLOW_OUTPUT_POWER` | `zwlr_output_power_manager_v1` (TTY only) | `/usr/bin/wlopm` |

- swm matches `SO_PEERCRED` PID -> executable `dev`/`ino` against approved paths at connect time (no basenames, no wildcards, no copied-binary inheritance; restart swm to change policy).
- Never grant these to shells, interpreters, launchers, or sandbox brokers (children would inherit the grant).
- Fcitx5 needs both input-method and virtual-keyboard to forward unconsumed keys:
  ```sh
  SWM_ALLOW_INPUT_METHOD=/usr/bin/fcitx5 \
  SWM_ALLOW_VIRTUAL_KEYBOARD=/usr/bin/fcitx5 swm
  ```
- Virtual keyboard events are ignored during `ext_session_lock_manager_v1`.
- Focused clipboard copy/paste (`wl_data_device`, `zwp_primary_selection`) and `text-input-v3` need no grants.
- Prefer portals (`xdg-desktop-portal-swm` / `swm-screenshoter`) over unrestricted capture grants when possible.

## Accessibility keyboard monitor

Denied by default. Start your screen reader on D-Bus, grab its exact unique bus name (`busctl --user status org.gnome.Orca.KeyboardMonitor`), and pass `SWM_ACCESSIBILITY_MONITOR=:1.42` before starting swm. The approved connection must actively own `org.gnome.Orca.KeyboardMonitor`; restarting the reader invalidates approval.

## Portal & IPC boundaries

- **Portal sessions**: requests/sessions bind strictly to the initiating D-Bus unique name and app ID; foreign connections cannot touch another connection's session.
- **Private IPC socket** (`$XDG_RUNTIME_DIR/swm/swm.sock`): administrative user boundary (keep out of untrusted sandboxes). Limits: max `64` concurrent clients, `30s` frame read deadline, `5s` write timeout, `300s` (5m) deferred reply expiry, `30s` idle timeout on non-subscription connections.
- **DMA-BUF registrations**: per-connection scoped, max `16` buffers/client, `128` total, max `4` planes/FDs per buffer; auto-freed on disconnect.
