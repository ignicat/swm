# Custom window open and close shaders

Optional GLES fragment shaders for `animations.window-open` and `animations.window-close`. Off unless `custom-shader` is set.

`type` still controls motion (`launch`/`retract` travel; `center-out`/`fade`/`shrink` stay at window bounds). When a shader compiles cleanly, it replaces built-in scale/opacity fading. Node collapse never uses custom shaders (CPU shrink path).

```rune
animations:
  window-open:
    enabled true
    type "center-out"
    duration-ms 900
    curve "linear"
    custom-shader "open-wave.frag" # relative to swm.rune dir (or ~/)
  end
  window-close:
    enabled true
    type "shrink"
    duration-ms 900
    custom-shader "close-wave.frag"
  end
end
```

Try the shipped wave shaders by copying `examples/shaders/{open,close}-wave.frag` beside `~/.config/swm/swm.rune` with the 900 ms config above. Failed reads/compiles log once and fall back to the configured `type`. Reloads automatically when file path or mtime changes (in-flight animations keep the shader they started with).

## Shader contract (not a stable API guarantee)

swm prepends Smithay's texture preamble (`//_DEFINES_`, `v_coords`, `tex`, `alpha`) and appends `main`. Implement one entrypoint returning **premultiplied alpha**:

- Open: `vec4 open_color(vec3 coords_geo, vec3 size_geo)`
- Close: `vec4 close_color(vec3 coords_geo, vec3 size_geo)`

Details:
- `coords_geo.xy` spans `0..1` inside decorated bounds (can exceed `0..1` across the padded quad). `size_geo.xy` / `swm_geo_size` is size in pixels.
- Opening renders the live client + decorations into a backdrop-preserving snapshot each frame (client keeps receiving frame callbacks); `alpha` is `1.0` for opening and carries `opening_alpha * rule.opacity` for closing (do not multiply `alpha` manually; swm's epilogue does).
- Uniforms: `tex`, `swm_progress` (can overshoot on springs/elastic; linear wall-clock on close), `swm_clamped_progress` (`0..1`), `swm_random_seed` (`[0, 1)`), `swm_tex_scale`, `swm_tex_offset`, `swm_geo_size`.

Sample via:

```glsl
vec2 coords_tex = coords_geo.xy * swm_tex_scale + swm_tex_offset;
vec4 color = texture2D(tex, coords_tex);
```

Opening windows in fullscreen or Field maximize stay on live trees (skipping shaders); closing to nodes keeps CPU collapse.
