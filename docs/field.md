# Field behavior and maximize

Each output owns an independent infinite 2D Field with free window overlap, collapsed landmark clearance, zoom, maximize, and close-focus succession.

```rune
field:
  gap 20.0
  close-restore-focus true
  close-restore-nodes false
  close-restore-pan "if-offscreen"

  pins:
    corner "top-right"
    colour "#d65d26"
    background-colour "auto"
    size 1.0
  end

  zoom:
    enabled true
    step 1.10
    min 0.35
    smooth-rate 12.5
  end
end
```

- `gap`: logical pixels. Clearance between collapsed nodes and active windows, plus inset around Field-maximized windows. Active windows overlap each other freely.
- Fullscreen and maximized windows temporarily step outside landmark collision: nearby collapsed nodes stay put underneath without catching input or shifting stack depth.

## Pinning

`Mod+P` (`toggle-focused-pin`) toggles Field pins (runtime-only, visible in `swmctl node list --json` / `node info --json`).

- Pinned active windows and collapsed nodes cannot be dragged or IPC-moved, cannot be interactively resized, act as immovable collision anchors, and never auto-decay. Manual collapse/restore (`Mod+N`), focus, maximize, fullscreen, and close still work.
- `pins.corner`: `top-left`, `top_left`, `left`, `top-right`, `top_right`, `right`.
- `pins.colour` / `background-colour`: `auto`, `light`, `dark`, or hex (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`). Note: `"system"` is not accepted here.
- `pins.size`: clamped `0.5..=3.0`. Hidden during fullscreen with other compositor chrome.

## Camera zoom & panning

Zoom only goes outward (`min` `0.05..=1.0`, `step` `1.001..=8.0`, `smooth-rate` `0.1..=120.0`, default `1.0` max). Legacy keys (`field.zoom.max`, `smooth`, `filter`, `sharpen`) fail `swmctl config verify`.

`smooth-rate` controls both zoom easing and pan follow speed (`input.gestures.pan-decay-rate` controls trackpad fling decay). Window content and borders share a rounded translation so moving chrome stays stable and resting pixels stay sharp.

## Field maximize

`Mod+M` (`maximize-focused`, alias `toggle-maximize`) toggles Field maximize independently of fullscreen:

- Fills the monitor's usable layer-shell work area inset by `field.gap` (panels stay visible).
- Raises the target window once to the top on entry (subsequent raises can stack windows over it).
- Keeps camera center fixed, eases zoom to `1.0`, locks pan/zoom on that monitor, and notifies the client with Wayland/X11 `Maximized` state.
- Preserves landmarks in place and keeps `xdg-popup` constraints aligned in screen coordinates.
- Maximizing another window on the same monitor retargets cleanly; `Mod+Left-drag` unmaximizes smoothly at the grab point; resize is blocked while maximized; collapse/minimize/close/unmap/fullscreen exits maximize cleanly.
- Transition uses `animations.maximize` (default 240 ms `ease-in-out-cubic`, or `motion "spring"`). See [animations](animations.md).

## Focus after close (`close-restore-*`)

When `close-restore-focus true` (closing a background window keeps current focus untouched):

1. Tries nearest surviving, focusable parent window (native, portal, or X11 dialogs; skips collapsed parents).
2. Otherwise picks same-output MRU node, then global MRU node.
3. If the winner is collapsed: `close-restore-nodes false` (default) sets logical node focus without expanding it or moving the camera; `true` restores and focuses it.
4. `close-restore-pan` applies when focusing an active successor or restoring a node: `"never"`, `"if-offscreen"` (default: pans minimally only if completely offscreen), or `"always"` (centers).

Closing a Field-maximized window restores its pre-maximize camera first, then applies successor panning. `close-restore-focus false` clears focus on close regardless of `close-restore-nodes`.
