# Wallpaper

Disabled by default:

```rune
wallpaper:
  mode "none"
end
```

With `mode "none"`, swm draws no wallpaper element and clears empty areas to opaque black.

## Classic image mode

```rune
wallpaper:
  mode "classic"
  path "~/Pictures/wallpaper.png"
  fit "cover" # "cover" | "contain" | "stretch"
  intensity 1.0
end
```

- `path`: `~/` expands `$HOME`; relative paths resolve against `swm.rune`'s directory.
- `fit`: `cover` (fill & crop), `contain` (letterbox in black), `stretch`.
- `intensity`: clamped `0.0..=1.0` opacity multiplier.
- Uploaded once per renderer context, rendered behind `Layer::Background` (below panels, windows, overlays, and cursor), shared across TTY/Winit/readbacks, and hidden behind the black fail-closed lock screen. Broken/unreadable images fall back to black cleanly without failing frames.
- `background:` and `gesso:` alias to `wallpaper:`.
