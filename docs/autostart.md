# Autostart commands and logs

`autostart.once` runs once on native TTY session startup after Wayland sockets are ready (`autostart.on-reload` runs on every valid config reload). Nested Winit sessions run neither.

```rune
autostart:
  once "waybar"
  once "mako"
end
```

- `once` deduplicates exact whitespace-trimmed command strings (preserving first appearance; differing args stay distinct).
- `on-reload` always reruns every listed entry, so avoid long-running daemons there unless they handle duplicate invokes themselves.

## Output logging

Each command logs to `$XDG_STATE_HOME/swm/autostart/` (fallback `~/.local/state/swm/autostart/`, mode `0700`): `<exe_sanitized>-<16hex_cmd_hash>.log` (first whitespace token's basename sanitized to `[A-Za-z0-9-]`, max 32 chars, plus FNV/stable hash of full command line). Exact path is printed at launch in the session log.

```sh
tail -n 100 ~/.local/state/swm/autostart/waybar-*.log
```

- Captures timestamp, command, `WAYLAND_DISPLAY` / `DISPLAY`, merged stdout+stderr, and exit status via lightweight helper binary `swm-autolog`. If log dir creation fails, the command still launches.
- Rotation: current `.log` + `.log.1` + `.log.2`, max 1 MiB each, mode `0600`.
- Non-zero exits log once with exit status and log path (no auto-restart). Helper failures truncate stderr detail at 4 KiB (`MAX_HELPER_DETAIL_BYTES = 4096`).

## Shutdown and systemd services

On clean native logout, swm sends `SIGTERM` to process groups it spawned (waiting up to 1s total before escalating). Keybind launches and external services manage their own lifetime.

For supervised daemons (restart policies, daemonizing tools), use user units tied to `graphical-session.target` rather than `autostart`:

```ini
[Unit]
PartOf=graphical-session.target
Requisite=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=/usr/bin/waybar
Restart=on-failure
```

Enable under `swm.service.wants/` (for `swm-session` / systemd managed sessions) or `swm-direct-session.target.wants/` (`swm --session`), and remove the matching `autostart.once` line so the daemon isn't started twice.
