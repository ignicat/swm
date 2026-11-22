# Split Config Example

Demonstrates splitting `swm.rune` across files with `gather` (`swm.rune` is the entrypoint; paths resolve relative to the file containing each `gather`).

Compared with `../swm.reference.rune`, it splits visuals/animations, Field rules, and device settings into separate files while keeping inline blocks and overrides in `swm.rune`:

- [`swm.rune`](swm.rune): `gather`s, `view` (display scale), `autostart`, `keybinds`, `rules`, `debug`
- [`theme.rune`](theme.rune): `font`, `wallpaper`, `decorations`, `node`, `overlays`, `animations` (plus commented `custom-shader` lines)
- [`field.rune`](field.rune): `field`, `physics` (`enabled true`), `decay`, `trail`
- [`input.rune`](input.rune): `env`, `cursor`, `input`

Top-level unaliased `gather`s deep-merge in order (later gathered files win on key collisions; nested `gather`s inside an already-gathered file stay namespaced in `rune-cfg`). swm watches all files in the dependency tree and refreshes the watch list when edits occur.

Verify from the root file:

```sh
swmctl config verify --config examples/split-config/swm.rune
```

swm never rewrites, migrates, or backfills config files; omitted keys fall back to defaults.
