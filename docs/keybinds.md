# Keybind triggers and actions

Keybinds live in `keybinds:` inside `~/.config/swm/swm.rune`:

```rune
keybinds:
  mod "super"

  "Print" "swm-screenshoter"
  "$var.mod+Page_Up" "some-command"
  "$var.mod+click-forward" "some-other-command"
  "$var.mod+scroll-up" "zoom-in"
end
```

Format: `"chord" "action" [with repeat true|false] [with scope "global|field"]`. Everything before the last `+` is a modifier; matching is case-insensitive and **exact** (extra held modifiers prevent firing).

## Default bindings (`swm.default.rune`)

Declaring `keybinds:` replaces built-in defaults completely. `node-move` and `window-transfer` are omitted in the starter config and bound to `Super+Alt+Arrow` / `Super+Alt+Shift+Arrow` in [`examples/swm.reference.rune`](../examples/swm.reference.rune).

| Keybind | Action | Notes |
| --- | --- | --- |
| `Super+Shift+E` | `quit` | Quit confirmation overlay (`Enter` leaves, `Esc` cancels) |
| `Super+Q` | `close-focused` | Closes focused active window or collapsed node |
| `Super+F` / `Super+M` | `toggle-fullscreen` / `maximize-focused` | Fullscreen vs Field maximize |
| `Super+N` / `Super+P` | `toggle-state` / `toggle-focused-pin` | Collapse/restore / pin toggle |
| `Super+O` | `zoom-fit` | Per-monitor zoom-to-fit |
| `Alt+Tab` / `Alt+Shift+Tab` | `cycle-focus` / `cycle-focus-backward` | Global MRU carousel |
| `Super+Arrow` / `Super+H` | `focus-DIRECTION` / `center-last-focused` | Spatial focus / center MRU |
| `Super+,` / `Super+.` | `trail-prev` / `trail-next` | Per-monitor Trail history |
| `Super+Shift+Arrow` | `monitor-focus DIRECTION` | Switch output |
| `Super+Ctrl+Arrow` | `resize-window-DIRECTION` (`scope "field"`) | Resize focused Field window |
| `Super+-` / `Super+=` / `Super+Shift+0` | `zoom-out` / `zoom-in` / `zoom-reset` | Field camera zoom |
| `Super+T` / `Super+D` / `Print` | `default-terminal` / `swm-lift` / `swm-screenshoter` | `swm-lift` and `swm-screenshoter` are separate companion binaries |
| `Super+Shift+R` | `reload` | Live config reload |
| `Super+Left-drag` / `Super+Right-drag` | `move-window` / `resize-window` | `move-window` pans if started on empty Field |
| `Super+Shift+Left-drag` / `Left-drag` | `drag-pan` / `pan-field` | Single-monitor carry pan / empty-Field pan |

## Modifiers and scopes

- Modifiers: `$var.mod` (expands `mod`), `super`/`logo`/`mod4`, `lsuper`/`rsuper`, `alt`, `lalt`/`ralt`, `ctrl`/`control`, `lctrl`/`rctrl`, `shift`, `lshift`/`rshift`. Under nested Winit, `$var.mod` `"super"` remaps to `Alt` (`lsuper` -> `lalt`, etc.) so swm doesn't steal host bindings.
- Scopes: bindings default to `global` (except Field move/resize actions). Use `with scope "field"` or `with scope "global"` to bind two actions on the same chord across scopes.

## Triggers

swm accepts any case-insensitive XKB keysym known to `libxkbcommon`, plus raw evdev codes:

- **Printable keys**: `a`-`z`, `0`-`9`, `space`, `grave`/`asciitilde`, `minus`/`underscore`, `equal`/`plus`, `bracketleft`/`right`, `braceleft`/`right`, `backslash`/`bar`, `semicolon`/`colon`, `apostrophe`/`quotedbl`, `comma`/`less`, `period`/`greater`, `slash`/`question`, `exclam`, `at`, `numbersign`, `dollar`, `percent`, `asciicircum`, `ampersand`, `asterisk`, `parenleft`/`right`. Shifted characters need `shift` in the chord (`"shift+plus"`) or raw `keycode-N`.
- **Navigation / function / lock / keypad / media**: `Escape`, `Tab`, `ISO_Left_Tab`, `Return`, `BackSpace`, `Insert`, `Delete`, `Home`, `End`, `Page_Up`, `Page_Down`, `Up`/`Down`/`Left`/`Right`, `F1`-`F35`, `Print`, `Caps_Lock`, `Num_Lock`, `Scroll_Lock`, `KP_0`-`KP_9`, `KP_Enter`, `XF86Audio*`, `XF86MonBrightness*`, `XF86PowerOff`, etc.
- **Standalone modifiers**: `Shift_L`/`R`, `Control_L`/`R`, `Alt_L`/`R`, `Super_L`/`R`, `ISO_Level3_Shift` ignore the modifier state they themselves set.
- **TTY VT switching**: plain `Ctrl+Alt+F1..F12` are reserved on native TTY (even under session lock or modals). Adding `Shift` or `Super` frees the chord for user binds.
- **Raw Linux evdev codes**: `keycode-N` (keyboard) and `button-N` (mouse button), using raw kernel codes from `wev` / `libinput debug-events` (do not add XKB's +8 offset).
- **Mouse buttons & physical wheel**: `click-left` (272), `click-right` (273), `click-middle` (274), `click-back` (275/278), `click-forward` (276/277), `scroll-up`/`down`/`left`/`right` (physical mouse wheel notches only; trackpad scrolling goes to clients unless `scroll-pan` handles empty Field).

## Built-in actions

- **Core**: `quit`, `close-focused`, `toggle-fullscreen`, `maximize-focused`, `toggle-state`, `toggle-focused-pin`, `zoom-fit`, `cycle-focus`, `cycle-focus-backward`, `trail-prev`, `trail-next`, `center-last-focused`, `default-terminal`, `reload`, `zoom-in`, `zoom-out`, `zoom-reset`.
- **Parameterized (`DIRECTION` = `left`|`right`|`up`|`down`)**: `focus-DIRECTION`, `node-move DIRECTION [1..=2147483647]`, `resize-window-DIRECTION` (`left`/`up` shrink, `right`/`down` grow), `window-transfer DIRECTION`, `pan-field DIRECTION` (10% viewport step), `monitor-focus DIRECTION|<OUTPUT>`.
- **Pointer drags**: `move-window`, `resize-window`, `drag-pan` (single-monitor carry pan; requires a fully visible non-maximized/non-fullscreen window), `pan-field` (empty Field only), `pan-field-always` (starts over windows, collapsed nodes, and Background/Bottom layers; declines over fullscreen/locked/Top/Overlay/X11 OR/client grabs). Pointer actions cannot bind to touchpad gestures, and ignore keyboard bindings.
- **Shell commands**: anything else (unless it's a malformed built-in like invalid `node-move`) executes as a command. `default-terminal` picks the first installed binary in: `alacritty`, `kitty`, `ghostty`, `wezterm`, `foot`, `footclient`, `rio`, `contour`, `kgx`, `gnome-terminal`, `konsole`, `xfce4-terminal`, `tilix`, `terminator`, `mate-terminal`, `qterminal`, `lxterminal`, `xterm`.

`window-transfer` moves either an active Field window or a collapsed node (preserving collapsed state) to the adjacent output's viewport center and warps the pointer there (unless a pointer grab/lock/confinement blocks it).

## Key repeat

Continuous built-ins (focus, node move, resize, pan, monitor focus, zoom in/out) repeat after `input.repeat-delay` at `input.repeat-rate` (`0` disables repeat); toggles/commands are one-shot. Override via `with repeat true|false` (not valid on pointer/wheel binds).

## Fullscreen & X11 stack behavior

Entering compositor fullscreen (`Mod+F`) raises X11 windows once; client-requested X11 fullscreen and leaving fullscreen keep the existing stack slot (`Mod+F` negotiates geometry and restores work-area-fitted bounds if needed). Both native and X11 fullscreen park `Top` layer-shell panels on that output only. See [XWayland policy](xwayland.md) for game modes.

## Legacy action aliases

`swm-config` still accepts older spellings (and `_` instead of `-` in the first word), though new configs should use current names:

- `close-window` -> `close-focused`, `fullscreen` -> `toggle-fullscreen`, `toggle-maximize` -> `maximize-focused`, `toggle-pin`/`pin-toggle` -> `toggle-focused-pin`
- `overview`/`apogee` -> `zoom-fit`, `trail-previous` -> `trail-prev`, `field-jump` -> `drag-pan`, `reload-config` -> `reload`, `open-terminal` -> `default-terminal`
- `focus DIR`, `node move DIR`/`move DIR`/`move-DIR`/`node-move-DIR [N]`, `resize DIR`/`resize-window DIR`, `monitor focus TGT`/`monitor-focus-DIR` -> hyphenated current forms
