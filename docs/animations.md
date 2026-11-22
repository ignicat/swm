# Animation configuration

`animations.enabled` is the master toggle; each sub-block also has its own `enabled` switch. Disabling an animation (or setting easing `duration-ms 0`) snaps straight to endpoints.

## `window-move` (CLI & keybinds)

Covers `swmctl node move` / `node move to` and `node-move` keybinds (pointer dragging stays direct). Defaults in memory (omitted from `swm.default.rune` / `examples/swm.rune`, shown in `examples/swm.reference.rune`):

```rune
animations:
  window-move:
    enabled true
    motion "easing"
    duration-ms 250
    curve "ease-out-cubic"
  end
end
```

Runs in Field coordinates (tracks pan/zoom along with decorations, popups, and hit boxes). Repeated commands retarget from the live visual position (springs keep velocity). Collapsed landmarks keep their own 520 ms slide.

## `smooth-resize`

Low-pass filter on interactive pointer resize (`enabled true`, `duration-ms 90`) for Wayland and XWayland. Lower durations track tighter; release freezes at the current presented size.

## Motion models (`fullscreen`, `maximize`, `window-open`)

Accepts either `motion "spring"` (`damping-ratio`, `stiffness`) or `motion "easing"` (`duration-ms`, `curve`).

- Curves: `linear`, `ease-in-out-cubic`, `ease-out-quad`, `ease-out-cubic`, `ease-out-expo`, `elastic` (overshoots).
- `duration-ms`: total elapsed time from first frame to settled frame.
- `fullscreen` defaults to spring (`damping-ratio 1.0`, `stiffness 800.0`).
- `maximize` defaults to easing (`240 ms`, `ease-in-out-cubic`) and crossfades outgoing/incoming textures; reversing mid-flight keeps position and velocity. Maximizing mid-fullscreen-exit grabs the live animated rect while keeping the original windowed restore spot.

## `window-open` & `window-close`

`window-open` defaults to `type "center-out"`, `300 ms`, `curve "linear"`:

- `center-out`: scales outward from final center.
- `fade`: fades opacity in place.
- `launch`: arcs from origin at 80% scale -> 102% overshoot -> 100%, capped at 320 px travel (`curve` drives travel only; opacity/scale anchor to `duration-ms`). Origin priority: `xdg-activation` surface center -> current cursor coords (even on another output) -> focused window center -> output center.

`window-close` pairs (`shrink`, `fade`, `retract`; defaults to `type "shrink"`, `270 ms`, hardcoded `ease-in-out-cubic`):

- Wayland clients snapshot on actual unmap (so quit dialogs aren't frozen prematurely). X11 captures a snapshot prior to sending close, and shortcut close (`Mod+Q`) starts speculatively immediately (also hiding Steam auxiliary child windows).
- `retract` reverses `launch` toward the recorded launch origin (or same fallback chain, max 320 px).
- Preserves current opening opacity and tracks camera motion. Excludes layer-shell and X11 override-redirect surfaces.

Optional `custom-shader` on `window-open` / `window-close` replaces scale/fade while keeping `launch`/`retract` travel; see [window shaders](window-shaders.md).

## `node`

```rune
animations:
  node:
    enabled true
    duration-ms 280          # marker scale-in
    collapse-duration-ms 280 # window snapshot shrink-to-node
  end
end
```

`duration-ms 0` skips only marker scale-in; `collapse-duration-ms 0` skips only the window drop. Collapses always use CPU shrink-to-node (ignoring `window-close` type/duration/shaders) at the window's original stack depth. Restoring triggers `window-open`; optional centering pans concurrently with restore.
