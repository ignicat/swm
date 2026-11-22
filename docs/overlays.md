# Compositor overlays

Shared visual contract for compositor-owned UI: Alt+Tab rail, portal source chooser, config notices, zoom indicator, and quit confirmation. Coordinates/sizes are logical pixels scaled per output ([display-scale.md](display-scale.md)).

```rune
overlays:
  background-colour "auto"
  text-colour "auto"
  error-colour "#fb4934"
  border-colour "#d65d26"
  radius 8
  borders true
  border-size 3

  notifications:
    position "top-center"
    offset-x 0
    offset-y 0
    success-duration-ms 4000
    error-duration-ms 9000
  end

  zoom-indicator:
    enabled true
    position "bottom-center"
    hold-duration-ms 750
    fade-duration-ms 180
    background true
    opacity 1.0

    # Optional overrides (omitted values inherit above)
    # text-size 18
    # text-colour "auto"
    # background-colour "auto"
    # border-colour "#d65d26"
    # borders true
    # radius 8
  end
end
```

(`swm.default.rune` omits `overlays:` and relies on these built-in defaults; `examples/swm.reference.rune` lists them explicitly.)

- **Palette & chrome**: colour keys (`-color` American aliases accepted, plus legacy `overlay.` prefix) take `"auto"`, `"system"` (live XDG Settings `color-scheme`, fallback `"auto"`), `"light"`, `"dark"`, or `#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`. `radius` clamps `0..=256`; `border-size` clamps `0..=64` (`borders false` hides strokes). Alt+Tab `NODE` pills, monitor badges, and title bands stay borderless (`border_px = 0`), while Alt+Tab preview textures inherit `overlays.radius`.
- **Notifications**: positions `top-left`, `top-center`, `top-right`, `bottom-left`, `bottom-center`, `bottom-right`; positive ms durations; signed `offset-x`/`offset-y` default to `0`.
- **Typography**: uses `font.family` and `font.size` globally ([fonts.md](fonts.md)); only `zoom-indicator.text-size` (`6..=96`) overrides size locally.

## Zoom indicator

Displays active monitor zoom (`0.75x`). Resets its `hold-duration-ms` timer while zoom keys, wheel binds, resets, or pinch gestures remain in motion, then fades over `fade-duration-ms`. Setting `background false` draws bare text (suppressing card fill and border). `opacity` clamps `0.0..=1.0`; optional `zoom-indicator.radius` clamps `0..=256`.

## Config notices & verification

- Initial load: `Configuration successfully loaded from /path/to/swm.rune`
- Invalid live reload: `Current configuration was unable to load properly. Run \`swmctl config verify\` to see why.` (keeps prior valid config active; next valid save dismisses it).

```sh
swmctl config edit
swmctl config verify [-c PATH]
```

Without `-c`, `swmctl` queries the running compositor for its active file (fallback `~/.config/swm/swm.rune`). Exit codes: `0` valid, `1` config parse/validation error, `2` CLI/path error.

## Quit confirmation

`quit` bind or `swmctl quit` opens the modal prompt: `Enter` exits swm, `Escape` cancels. Captures keyboard/pointer input while preserving focus underneath.
