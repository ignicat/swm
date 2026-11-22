# Alt+Tab

`Alt+Tab` / `Alt+Shift+Tab` opens an MRU window rail covering both active windows and collapsed nodes. Companion retrieval paths: `Mod+Arrow` (spatial neighbors), [zoom to fit](zoom-fit.md) (per-output full-Field view), and `swm-lift` (search by name).

## Scope and layout

Collects one global MRU list across all monitors (`candidates.len() >= 2` to open) and mirrors the 5-card rail (`VISIBLE_RADIUS = 2`) across outputs with per-card monitor badges and a `NODE` pill on collapsed targets.

- Every visible non-collapsed card refreshes live (`push_live_preview` + frame callbacks).
- Collapsed nodes and non-direct fallbacks use cached GPU stills (`OverlayPreviewCache`).
- Outer card containers use `overlays` border/radius/fill styling; internal title bands, monitor badges, and `NODE` pills use borderless label chrome (`border_px = 0`) and `font.size`.

## Visibility and presentation policy

Releasing `Alt` commits: focuses/raises the center card, reveals it if needed, and warps the pointer to its output center. `Esc` cancels without touching focus, camera, or cursor.

Commit behavior depends solely on whether the target's decorated frame fits inside its output viewport (occlusion by other windows does not count as offscreen; touching the viewport edge counts as fitting):

- **Fits onscreen**: raises in place. Camera, fullscreen, and Field maximize stay untouched.
- **Partially or fully offscreen**: pans minimally on that destination monitor (centering axes that exceed viewport size; collapsed nodes reserve their restored decorated bounds). Only on the destination monitor: ordinary apps exit fullscreen/Field maximize so the Field can pan; identified fullscreen games keep their fullscreen state/buffer while their presentation parks.
- **Fullscreen/maximized targets**: selecting one keeps its active presentation. Departing monitors keep their presentations untouched.

Note: `field.close-restore-pan` and `node.click-collapsed-pan` do not override Alt+Tab.

Parked games resume fullscreen on direct click (or cross-monitor hover re-entry in `hover` focus mode; same-monitor hover stays click-to-resume). See [window rules](window-rules.md) for game flags/detection.
