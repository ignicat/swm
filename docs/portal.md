# Screen sharing and the desktop portal

`xdg-desktop-portal-swm` (`swm-portal`) handles `org.freedesktop.impl.portal.ScreenCast` via swm's private chooser and local capture IPC. Applications cannot grab surfaces silently: swm opens a modal chooser and casts only the confirmed source.

Sibling portals stay external:
- `org.freedesktop.impl.portal.Screenshot`: provided separately by `swm-screenshoter`.
- File pickers, Settings/appearance, etc.: delegated to `xdg-desktop-portal-gtk`.

## Source chooser

If both Monitor and Window sources are requested, swm shows a bottom chooser bar (`Monitor` / `Window`); single-type requests jump straight into Monitor or Window picking. Navigate with pointer or `Left`/`Right` + `Enter` (`Esc` steps back from picking only when both types were offered, else cancels). Windows blocked by `block-screencast` are omitted from candidates. Chooser dim/chrome is never captured into the stream.

## Privacy rules & indicator

- `block-screencast true` in a matching `window-rule` or `layer-rule` ([window-rules.md](window-rules.md)) blanks/omits that surface from monitor captures (exposing what sits behind it), hides it from window pickers, and blacks out active individual-window casts (while remaining visible on your physical display). Direct `wlr-screencopy` / `ext-image-copy-capture` grants are separate ([security.md](security.md)).
- While privacy rules are active, monitor shares blank out the Alt+Tab `focus-cycle` scene and suppress cached `OverlayPreviewCache` hover previews; XWayland override-redirect surfaces are also omitted when any window privacy rule exists.
- Directly cast windows get a local red outline configured under `decorations.screencast-indicator` (`size` default `4`, `colour` default `"#f0333d"`), omitted from captured streams.

## PipeWire & OBS details

Add **Screen Capture (PipeWire)** in OBS. Closing the source tears down the session and PipeWire node.

- Supports `Hidden`, `Embedded`, and `Metadata` cursor modes per client request.
- Advertises renderer/GBM-compatible DMA-BUF modifiers (preferred tiled first) + producer MemFd fallback, querying swm's render node instead of guessing GPU devices. Exception: capturing a **Window** with **Embedded** cursor disables DMA-BUF (`wants_dmabuf = false`) and advertises MemFd only.
- DMA-BUF frames ack only after the renderer fence signals, emit `DMABUF_CHUNK_MARKER` (`9`), and include variable/max framerate (up to 60 fps) + damage metadata.
- Consumer-driven: swm renders capture frames only when PipeWire pulls a buffer.

## Session routing

`XDG_CURRENT_DESKTOP` must include `swm`. `swm`'s `packaging/xdg-desktop-portal/swm-portals.conf` routes `ScreenCast=swm;`, `Screenshot=swm-screenshoter;gtk;`, and `default=gtk;` cleanly without package ownership conflicts (`xdg-desktop-portal` skips `swm-screenshoter` if uninstalled and falls back to `gtk`). Units in `packaging/` bind `xdg-desktop-portal-swm.service` to `PartOf=graphical-session.target xdg-desktop-portal.service`; swm refreshes the frontend after TTY startup env export so stale portal state never lingers across logins. After editing unit files run `systemctl --user daemon-reload` (or `systemctl --user restart xdg-desktop-portal.service` in a running session).
