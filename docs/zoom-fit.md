# Zoom to fit

`Mod+O` (`zoom-fit`) zooms every monitor's camera out independently until all its active windows and collapsed nodes fit inside the viewport (`48px` margin, clamped `0.1..=1.0`, never magnifies past 100%; requires `field.zoom.enabled true`).

While zoomed out:

- Left-click a **window**: zooms back in centered on it and focuses it.
- Left-click a **collapsed node**: zooms back in centered on it and restores it.
- Left-click **empty Field**, press `Mod+O`, or press `Esc`: returns each monitor to its prior camera view.
- `Mod+Left-click` / `Mod+drag` and right-click/modified binds bypass zoom-fit click handling and reach normal Field drag/grab handlers.
- Manual panning or zooming overrides zoom-fit and clears the saved camera view.

## Gestures

Built-in 4-finger touchpad binds use `swipe-up-4 "zoom-fit-open"` and `zoom-fit-swipe-down-4 "zoom-fit-close"`:

```rune
input:
  gestures:
    swipe-up-4 "zoom-fit-open"
    zoom-fit-swipe-down-4 "zoom-fit-close"
  end
end
```
