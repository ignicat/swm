# Window and layer-shell rules

Managed rules apply equally to Wayland toplevels and normal XWayland windows (popups and X11 override-redirect windows are excluded).

```rune
rules:
  window-rule:
    app-id ["firefox", r"org\.mozilla\..*"]
    title r"Picture.*"
    width 720
    height 520
    opacity 0.90
    # block-screencast true
    spawn-placement "cursor"
  end
end
```

Evaluates top-to-bottom: defaults to first-match (`window-rule-mode "compose"` merges all matching rules, later explicit keys overwriting earlier ones). Exact strings are case-sensitive; `r"..."` are regexes; arrays are OR-lists; combining `app-id` + `title` is AND.

## Complex matchers (`match:` / `exclude:` / `match-all`)

```rune
rules:
  window-rule-mode "compose"
  window-rule:
    match:
      app-id "foot"
      title "Monitoring"
    end
    exclude:
      title "Private"
    end
    open-on-output "DP-2"
  end
end
```

- Multiple `match:` blocks OR together; fields inside one block AND together. `exclude:` vetoes a match. Empty groups fail validation.
- Conditions: `app-id`, `title`, `is-transient` (`true`|`false`), `at-startup` (`true` for first 60s of session; reloads don't reset clock), `is-focused`, `is-screencast-target` (window portal casts only).
- Do not mix `match:` or `match-all true` with top-level `app-id`/`title`. Dynamic matchers (`is-focused`, `is-screencast-target`) allow live visual properties only (no `width`/`height`/`open-*`/`spawn-placement`).
- Rename legacy `rules.rule` to `rules.window-rule` (`overlap-policy` parses as a no-op migration alias).

## Window rule properties

- **`width` / `height`**: initial dimensions (min clamp `96x72`). Unspecified axes use client sizing.
- **`open-on-output` / `open-focused` / `open-position [X, Y]`**: initial placement/routing relative to target monitor Field origin (`open-position` overrides `spawn-placement` and ignores landmark clearance; unavailable output falls back to active monitor). XWayland session geometry recovery may override.
- **`open-fullscreen` / `open-maximized`**: enters state after initial map (`open-fullscreen` wins if both true).
- **`spawn-placement`**: `viewport-center` (default), `cursor`, `center` (parent -> viewport), `adjacent` (right/left/below/above -> viewport), `app` (parent center -> adjacent -> viewport). Never moves already-mapped windows.
- **`opacity` (`0.0..=1.0`)**: scales client surface + popups (decorations/pins stay opaque). Live-reloaded.
- **`border` (`true`|`false`)**: hides/shows compositor border stroke without losing frame geometry/resize area (`border true` with global `size 0` draws no border). Note: cached hover/Alt+Tab fallback previews (`OverlayPreviewCache`) still draw the configured border. Live-reloaded.
- **`block-screencast` (`true`|`false`)**: omits window + popups from monitor portal captures, hides from window pickers, and blanks active window casts ([portal.md](portal.md)). Live-reloaded.
- **`game` (`true`|`false`)**: forces or disables fullscreen game parking on same-monitor focus switch ([alt-tab.md](alt-tab.md), [xwayland.md](xwayland.md)). When omitted, auto-detects via `wp_content_type` game hint, Steam game env/properties on the window PID, or `.desktop` `Game` category (excluding mixed launcher categories like `Network`/`Utility`). Live-reloaded for future switches.

## Layer-shell rules

Always first-match inside `rules:`. Requires `namespace` and/or `layer` (`"background" | "bottom" | "top" | "overlay"`) and mandatory `block-screencast`:

```rune
rules:
  layer-rule:
    namespace ["waybar", r"^fuzzel$"]
    layer ["top", "overlay"]
    block-screencast true
  end
end
```
