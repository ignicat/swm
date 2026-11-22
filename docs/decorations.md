# Window decorations

swm draws a square-cornered 4-strip rectangular border around managed top-level windows:

```rune
decorations:
  border:
    size 3
    colour-focused "#f4f5f7"
    colour-unfocused "#474d59"
  end

  resize-using-border true

  screencast-indicator:
    size 4
    colour "#f0333d"
  end
end
```

- `size`: logical pixels (`>= 0`), multiplied by output scale and camera zoom. Setting `0` removes the border and sets `_NET_FRAME_EXTENTS` / frame extents to `0`.
- Colors accept `#rgb`, `#rrggbb`, `#rgba`, `#rrggbbaa`, and American `color` aliases (`color-focused`, `color-unfocused`).
- `resize-using-border`: left-dragging the outer edge or corner resizes the window. Grab band width equals `border.size` when `> 0` (3 px by default) and keeps a `3` px hit band when `size 0`. Set `false` to require `Mod+Right-drag`.
- `screencast-indicator`: local-only border overlay drawn on windows being directly shared via the portal (`size` clamped `1..=32`). Not visible inside captured streams or on bystanders in monitor shares.

Direct window captures, window screencasts, and cached `OverlayPreviewCache` stills include compositor borders; live Alt+Tab preview cards render surface trees directly without border strips. Layer-shell surfaces, popups, and X11 override-redirect windows get no borders.
