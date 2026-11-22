# Nodes and decay

Every managed Wayland toplevel or XWayland window gets one stable `NodeId` for its mapping lifetime. Collapsing shrinks a window's snapshot into a clickable landmark marker at the exact stack depth it already occupied (`Mod+N`, or client minimize). Clicking the marker once (or pressing `Mod+N` again) restores it in one step.

- **Pointer vs drag on markers**: plain left release before moving `8` px restores; moving `>= 8` px drags the marker without expanding; `Mod+Left-drag` grabs immediately.
- **Logical focus on collapsed nodes**: collapsing the focused window (or hovering a marker in `hover` focus mode) keeps logical focus on that node while clearing Wayland keyboard focus, so `Mod+Q` / `swmctl node close` still closes it.
- **XWayland**: sets `IconicState` + `_NET_WM_STATE_HIDDEN` on collapse and clears on restore.

## Pan on restore

```rune
node:
  click-collapsed-pan "never" # "never" | "if-offscreen" | "always"
end
```

Click-restore, `toggle-state`, and `swmctl node restore` follow `click-collapsed-pan`; Trail and directional focus promote `never` -> `if-offscreen` so targets never expand offscreen.

## Automatic decay

Off by default (`decay.enabled false`). When enabled, unfocused windows sitting **completely offscreen** collapse after `delay-seconds` (`600` default):

```rune
decay:
  enabled true
  delay-seconds 600
end
```

Any visible sliver on screen (at any zoom level) blocks decay and resets the timer; pinned, focused, fullscreen/pending-fullscreen, Field-maximized, or grabbed windows are immune. Output switches/disconnects reset or count offscreen accordingly.

## Appearance

```rune
node:
  show-labels "hover" # "off" | "hover" | "always"
  show-app-icons "always"
  shape "squircle" # "square" | "squircle"
  label-shape "squircle"
  icon-size 0.72
  opacity 1.0
  background-colour "auto" # "auto" | "system" | "light" | "dark" | #hex
  border-colour "#474d59" # accepts border-color
  border-colour-highlighted "#d65d26" # accepts border-color-highlighted
end
```

- `node-shape` / `node-label-shape` aliases are rejected.
- `border-colour` and `border-colour-highlighted` accept `-color` American aliases; `background-colour` only parses `background-colour`.
- Icons load async on a worker (blank center until ready, never placeholder text).
- `show-labels "hover"`: explicit pointer hover slides in the label pill, and 1500 ms uninterrupted dwell swaps it for a live-cached `OverlayPreviewCache` thumbnail. Movement/grab cancels dwell.

## Landmarks and zoom reflow

Collapsed nodes are landmarks: `field.gap` keeps clear spacing between nodes and active windows (excluding fullscreen/maximized presentations, which temporarily ignore landmarks). Marker size stays constant in screen pixels across zoom, so zooming out pushes unpinned landmarks around blocking windows/nodes and stores each moved node's pre-zoom home in memory. Zooming back in slides nodes back toward their remembered home; dragging/moving/re-collapsing/pinning/transferring overwrites the remembered home permanently.

## Rigid vs physics collision

```rune
physics:
  enabled false
  damping 0.45
end
```

- `enabled false` (default): dragged objects transfer displacement 1:1 rigidly; grabbed nodes slide around active windows.
- `enabled true`: dragged windows/nodes act as kinematic authorities, imparting bounded spring momentum to contacted nodes (and letting bumped nodes nudge active windows). Releasing an active window pins its final destination for 350 ms while displaced nodes finish settling. Pinned nodes stay immovable in both modes.

## `swmctl node` CLI

```sh
swmctl node list [-o OUTPUT] [--json]
swmctl node info [SELECTOR] [-o OUTPUT] [--json]
swmctl node focus|collapse|restore|toggle|close [SELECTOR] [-o OUTPUT]
swmctl node move left|right|up|down [SELECTOR] [-o OUTPUT] [--distance UNITS]
swmctl node move to X Y [SELECTOR] [-o OUTPUT]
```

- Selectors: `focused` (default fallback `latest`), `latest`, numeric `ID`, `id:ID`, `title:TEXT`, `app:APP_ID` (case-insensitive substrings, error on ambiguity).
- Plain text shows markers (`*` focused, `+` latest, `-` other), state, `app` when present, role, protocol, parent/modal, `child-popups` when `> 0`, Field pos/size. Pin status (`pinned`) appears in `--json`.
- `node move` (default 80 units, or `--distance 1..=2147483647` with `node-move-distance-v1`): expanded windows overlap other windows and stop at landmarks; `node move to X Y` (`node-move-to-v1`) places client top-left or node center at exact signed Field coordinates relative to monitor origin. Expanded-window moves use `animations.window-move`; pinned, detached, fullscreen, maximized, grabbed, or animating windows reject moves. Same rules apply to `node-move DIRECTION [UNITS]` keybinds.

Null-buffer unmaps, remaps, metadata commits, activation, fullscreen requests, and destruction stay live while collapsed. Layer-shells, popups, and X11 OR menus are not nodes.
