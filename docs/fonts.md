# Fonts

Global compositor typography setting in `swm.rune`:

```rune
font:
  family "monospace"
  size 11
end
```

- `family`: Cosmic Text generic (`monospace`, `sans-serif`, `serif`, `cursive`, `fantasy`) or any installed font family, with optional trailing style/weight suffixes (`Bold`, `Semi Bold`, `Extra Bold`, `Light`, `Thin`, `Black`, `Medium`, `Italic`, `Oblique`, etc.). Missing glyphs fall back automatically.
- `size`: logical pixels clamped `6..=96` (multiplied per output by [display scale](display-scale.md)). Applies to compositor labels, badges, dialogs, notifications, and overlays (`overlays.zoom-indicator.text-size`, also `6..=96`, is the sole per-overlay size override).

Config reloads clear cached text textures and redraw immediately without restarting swm. Note: node markers reserve their center for app icons (never showing temporary fallback text while loading).
