# Display scaling

Configure scale per connector inside `view.output`:

```rune
view:
  output:
    name "eDP-1"
    scale 1.5
  end
end
```

- `scale`: unquoted float (`1.0` = 100%, `1.5` = 150%, `2.0` = 200%; default `1.0`). Clamped/validated in `0.25..=10.0` and snapped to Wayland fractional-scale units (`1/120`). Check effective values with `swmctl outputs`.
- **Runtime changes**: `zwlr_output_manager_v1` tools (e.g. `wlr-randr --output eDP-1 --scale 1.5`) apply scale live without touching `swm.rune` (config reloads restore `swm.rune` values). On nested Winit (`name "winit"`), only `scale` changes locally; mode/position/transform/adaptive-sync stay host-owned.
- **Modes & coords**: Scale changes logical size (3840x2160 at `2.0` = 1920x1080 logical), not physical panel mode (`width`/`height`, `offset-x`/`offset-y`, `transform`, `rate`, `vrr` require `width`+`height` together). `offset-x`, `offset-y`, window sizes, notification offsets, borders, and `font.size` are logical pixels.
- **Negative offsets**: Valid. swm shifts the whole active monitor layout into nonnegative coordinates for XWayland's root window while keeping relative spacing intact.
- **Cross-monitor gaps**: Relative pointer motion jumps any gap along the exited edge to the nearest aligned output; dragged windows follow.
- **Scaling vs zoom**: `scale` sets native desktop UI scale (Wayland apps get fractional/integer preferred scale; XWayland surfaces scale at composition); camera zoom zooms the Field out and never magnifies past 100%.

Monitor readbacks (`CaptureFrame`, `wlr-screencopy`, `ext-image-copy-capture`) and ScreenCast always capture per output at native framebuffer resolution (clients such as `swm-screenshoter` handle cross-output resampling if stitching desktops). `scale 1.0` is omitted in starter `swm.default.rune` and shown in `examples/swm.reference.rune`.
