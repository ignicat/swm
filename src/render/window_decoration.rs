use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};

use cgmath::{Matrix3, SquareMatrix, Vector2};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, WaylandSurfaceTexture,
};
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
    UniformType,
};
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::backend::renderer::{ContextId, ImportMem, Renderer, Texture};
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

const JOIN_OVERLAP_PX: f32 = 0.75;

#[cfg(test)]
#[path = "window_decoration/pixel_tests.rs"]
mod pixel_tests;

/// Namespaces that let several decoration parts share one window's identity.
///
/// A window's surface already has a stable `Id`, so its chrome does not need a
/// registry — only a distinct namespace per part. Keeping these identities
/// stable is what stops each frame from telling the damage tracker that every
/// border and tint was destroyed and recreated.
pub mod slot {
    pub const BORDER: usize = 1;
    pub const WINDOW_OPEN: usize = 7;
    pub const BORDER_FALLBACK: usize = 48;
}

/// Stable identity for one decoration part of `surface`.
pub fn surface_slot(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    slot: usize,
) -> Id {
    Id::from_wayland_resource(surface).namespaced(slot)
}

/// Stable chrome identity for one presentation of a window.
pub fn surface_slot_for_instance(
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    slot: usize,
    instance: Option<&str>,
) -> Id {
    surface_slot(surface, instance_slot_namespace(slot, instance))
}

fn instance_slot_namespace(slot: usize, instance: Option<&str>) -> usize {
    let mut hash = DefaultHasher::new();
    slot.hash(&mut hash);
    instance.unwrap_or("canonical").hash(&mut hash);
    hash.finish() as usize
}

fn commit_seed(tag: u8, base: CommitCounter) -> DefaultHasher {
    let mut hash = DefaultHasher::new();
    tag.hash(&mut hash);
    base.distance(Some(CommitCounter::default()))
        .unwrap_or(usize::MAX)
        .hash(&mut hash);
    hash
}

fn hash_rect<K>(rect: Rectangle<i32, K>, hash: &mut DefaultHasher) {
    rect.loc.x.hash(hash);
    rect.loc.y.hash(hash);
    rect.size.w.hash(hash);
    rect.size.h.hash(hash);
}

fn hash_radii(radii: CornerRadii, hash: &mut DefaultHasher) {
    radii.top.to_bits().hash(hash);
    radii.bottom.to_bits().hash(hash);
}

fn finish_commit(hash: DefaultHasher) -> CommitCounter {
    CommitCounter::from(hash.finish() as usize)
}

pub fn solid_color_commit(color: smithay::backend::renderer::Color32F) -> CommitCounter {
    let mut hash = commit_seed(0, CommitCounter::default());
    color.r().to_bits().hash(&mut hash);
    color.g().to_bits().hash(&mut hash);
    color.b().to_bits().hash(&mut hash);
    color.a().to_bits().hash(&mut hash);
    finish_commit(hash)
}

fn rounded_surface_commit(
    base: CommitCounter,
    destination: Rectangle<i32, Physical>,
    clip: Rectangle<i32, Physical>,
    radii: CornerRadii,
) -> CommitCounter {
    let mut hash = commit_seed(1, base);
    hash_rect(destination, &mut hash);
    hash_rect(clip, &mut hash);
    hash_radii(radii, &mut hash);
    finish_commit(hash)
}

const SURFACE_SHADER: &str = r#"
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

precision highp float;
varying vec2 v_coords;

uniform float alpha;
uniform vec2 clip_size;
uniform vec2 draw_offset;
uniform vec2 corner_radii;
uniform vec3 uv_to_draw_col_0;
uniform vec3 uv_to_draw_col_1;
uniform vec3 uv_to_draw_col_2;
uniform vec4 content_color;

// Signed distance to a rounded rectangle, negative inside.
//
// The interior term `min(max(q.x, q.y), 0.0)` is load-bearing: without it an
// interior pixel reports a distance of exactly -radius rather than its real
// depth, so once the radius eases below the smoothstep half-width the band
// swallows the whole surface and fades it toward 50% opacity. That is only
// invisible while the radius stays large - a radius animating to zero, as the
// fullscreen transition does, dims the entire window just before it lands.
float rounded_alpha(vec2 coords, vec2 size, vec2 radii) {
    float radius = coords.y < size.y * 0.5 ? radii.x : radii.y;
    radius = clamp(radius, 0.0, min(size.x, size.y) * 0.5);
    vec2 half_size = size * 0.5;
    vec2 q = abs(coords - half_size) - (half_size - vec2(radius));
    float distance_to_edge =
        length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    return 1.0 - smoothstep(-0.75, 0.75, distance_to_edge);
}

void main() {
    mat3 uv_to_draw = mat3(uv_to_draw_col_0, uv_to_draw_col_1, uv_to_draw_col_2);
    vec2 coords = draw_offset + (uv_to_draw * vec3(v_coords, 1.0)).xy;
    vec2 size = max(clip_size, vec2(1.0));
    float mask = 0.0;
    if (coords.x >= 0.0 && coords.y >= 0.0 && coords.x <= size.x && coords.y <= size.y) {
        mask = rounded_alpha(coords, size, corner_radii);
    }

    vec4 sampled = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    sampled.a = 1.0;
#endif
    // Smithay uses premultiplied blending. Apply the mask exactly once to
    // both RGB and alpha; multiplying RGB by alpha here creates dark halos.
    gl_FragColor = sampled * content_color * (mask * alpha);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub content_radius: f32,
    pub border_width: f32,
    pub outer_radius: f32,
    pub inner_offset: f32,
    pub inner_radius: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadii {
    pub top: f32,
    pub bottom: f32,
}

impl CornerRadii {
    pub fn all(radius: f32) -> Self {
        Self {
            top: radius,
            bottom: radius,
        }
    }
}

pub fn metrics(content_radius: f32, border_width: f32) -> Metrics {
    let content_radius = content_radius.max(0.0);
    let border_width = border_width.max(0.0);
    Metrics {
        content_radius,
        border_width,
        outer_radius: if content_radius > 0.0 {
            content_radius + border_width
        } else {
            0.0
        },
        inner_offset: border_width + JOIN_OVERLAP_PX,
        inner_radius: (content_radius - JOIN_OVERLAP_PX).max(0.0),
    }
}

struct Resources {
    context: ContextId<GlesTexture>,
    white: GlesTexture,
    surface: GlesTexProgram,
}

#[derive(Default)]
pub struct WindowDecorationRenderer {
    resources: Option<Resources>,
    failed_context: Option<ContextId<GlesTexture>>,
}

impl WindowDecorationRenderer {
    pub fn available(&mut self, renderer: &mut GlesRenderer) -> bool {
        if self.ensure_resources(renderer).is_ok() {
            true
        } else {
            let context = renderer.context_id();
            if self.failed_context.as_ref() != Some(&context) {
                eventline::warn!(
                    "rounded window rendering unavailable; using square fallback for this GL context"
                );
                self.failed_context = Some(context);
            }
            false
        }
    }

    pub fn surface_element_with_radii(
        &mut self,
        renderer: &mut GlesRenderer,
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        destination: Rectangle<i32, Physical>,
        clip: Rectangle<i32, Physical>,
        radii: CornerRadii,
    ) -> Option<RoundedSurfaceElement> {
        self.available(renderer);
        let resources = self.resources.as_ref()?;
        let commit = rounded_surface_commit(
            inner.current_commit(),
            destination,
            clip,
            clamp_radii(radii, clip.size.w, clip.size.h),
        );
        Some(RoundedSurfaceElement {
            inner,
            destination,
            clip,
            radii: clamp_radii(radii, clip.size.w, clip.size.h),
            program: resources.surface.clone(),
            white: resources.white.clone(),
            commit,
        })
    }

    pub fn texture_element(
        &mut self,
        renderer: &mut GlesRenderer,
        base: TextureRenderElement<GlesTexture>,
        texture: GlesTexture,
        clip: Rectangle<i32, Physical>,
        radius: f32,
    ) -> Option<RoundedTextureElement> {
        self.texture_element_with_radii(
            renderer,
            base,
            texture,
            clip,
            CornerRadii::all(radius),
            (1.0, 1.0, 1.0, 1.0),
        )
    }

    pub fn texture_element_with_radii(
        &mut self,
        renderer: &mut GlesRenderer,
        base: TextureRenderElement<GlesTexture>,
        texture: GlesTexture,
        clip: Rectangle<i32, Physical>,
        radii: CornerRadii,
        content_color: (f32, f32, f32, f32),
    ) -> Option<RoundedTextureElement> {
        self.available(renderer);
        let resources = self.resources.as_ref()?;
        let mut commit = commit_seed(2, base.current_commit());
        hash_rect(clip, &mut commit);
        hash_radii(radii, &mut commit);
        for value in [
            content_color.0,
            content_color.1,
            content_color.2,
            content_color.3,
        ] {
            value.to_bits().hash(&mut commit);
        }
        Some(RoundedTextureElement {
            base,
            texture,
            clip,
            radii: clamp_radii(radii, clip.size.w, clip.size.h),
            program: resources.surface.clone(),
            content_color,
            commit: finish_commit(commit),
        })
    }

    fn ensure_resources(&mut self, renderer: &mut GlesRenderer) -> Result<(), Box<dyn Error>> {
        let context = renderer.context_id();
        if self
            .resources
            .as_ref()
            .is_some_and(|resources| resources.context == context)
        {
            return Ok(());
        }
        if self.failed_context.as_ref() == Some(&context) {
            return Err("rounded shaders previously failed for this GL context".into());
        }

        let result = (|| {
            let white = renderer.import_memory(
                &[255_u8; 4 * 4 * 4],
                Fourcc::Abgr8888,
                (4, 4).into(),
                false,
            )?;
            let surface = renderer.compile_custom_texture_shader(
                SURFACE_SHADER,
                &[
                    UniformName::new("clip_size", UniformType::_2f),
                    UniformName::new("draw_offset", UniformType::_2f),
                    UniformName::new("corner_radii", UniformType::_2f),
                    UniformName::new("uv_to_draw_col_0", UniformType::_3f),
                    UniformName::new("uv_to_draw_col_1", UniformType::_3f),
                    UniformName::new("uv_to_draw_col_2", UniformType::_3f),
                    UniformName::new("content_color", UniformType::_4f),
                ],
            )?;
            Ok::<_, Box<dyn Error>>(Resources {
                context: context.clone(),
                white,
                surface,
            })
        })();

        match result {
            Ok(resources) => {
                self.resources = Some(resources);
                self.failed_context = None;
                Ok(())
            }
            Err(err) => {
                self.resources = None;
                self.failed_context = Some(context);
                Err(err)
            }
        }
    }
}

pub struct RoundedSurfaceElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    destination: Rectangle<i32, Physical>,
    clip: Rectangle<i32, Physical>,
    radii: CornerRadii,
    program: GlesTexProgram,
    white: GlesTexture,
    commit: CommitCounter,
}

impl Element for RoundedSurfaceElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.destination
    }

    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> {
        self.destination.loc
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        // A hardware plane would bypass the rounded mask.
        Kind::Unspecified
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }
}

impl RenderElement<GlesRenderer> for RoundedSurfaceElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        match self.inner.texture() {
            WaylandSurfaceTexture::Texture(texture) => draw_masked_texture(
                frame,
                texture,
                MaskedTextureDraw {
                    src,
                    dst,
                    damage,
                    transform: self.transform(),
                    alpha: self.alpha(),
                },
                &self.program,
                MaskGeometry {
                    source: self.inner.src(),
                    destination: self.destination,
                    clip: self.clip,
                    radii: self.radii,
                },
                (1.0, 1.0, 1.0, 1.0),
            ),
            WaylandSurfaceTexture::SolidColor(color) => {
                let white_full = Rectangle::<f64, Buffer>::from_size(self.white.size().to_f64());
                let white_src = proxy_texture_source(src, self.inner.src(), white_full.size);
                draw_masked_texture(
                    frame,
                    &self.white,
                    MaskedTextureDraw {
                        src: white_src,
                        dst,
                        damage,
                        transform: self.transform(),
                        alpha: self.alpha(),
                    },
                    &self.program,
                    MaskGeometry {
                        source: white_full,
                        destination: self.destination,
                        clip: self.clip,
                        radii: self.radii,
                    },
                    (color.r(), color.g(), color.b(), color.a()),
                )
            }
        }
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[derive(Debug)]
pub struct RoundedTextureElement {
    base: TextureRenderElement<GlesTexture>,
    texture: GlesTexture,
    clip: Rectangle<i32, Physical>,
    radii: CornerRadii,
    program: GlesTexProgram,
    content_color: (f32, f32, f32, f32),
    commit: CommitCounter,
}

impl Element for RoundedTextureElement {
    fn id(&self) -> &Id {
        self.base.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.commit
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.base.geometry(scale)
    }
    fn transform(&self) -> Transform {
        self.base.transform()
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.base.src()
    }
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }
    fn alpha(&self) -> f32 {
        self.base.alpha()
    }
    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for RoundedTextureElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        draw_masked_texture(
            frame,
            &self.texture,
            MaskedTextureDraw {
                src,
                dst,
                damage,
                transform: self.transform(),
                alpha: self.alpha(),
            },
            &self.program,
            MaskGeometry {
                source: self.base.src(),
                destination: self.base.geometry(Scale::from(1.0)),
                clip: self.clip,
                radii: self.radii,
            },
            self.content_color,
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

struct MaskGeometry {
    source: Rectangle<f64, Buffer>,
    destination: Rectangle<i32, Physical>,
    clip: Rectangle<i32, Physical>,
    radii: CornerRadii,
}

fn proxy_texture_source(
    source: Rectangle<f64, Buffer>,
    original: Rectangle<f64, Buffer>,
    proxy_size: smithay::utils::Size<f64, Buffer>,
) -> Rectangle<f64, Buffer> {
    let scale_x = proxy_size.w / original.size.w.max(f64::EPSILON);
    let scale_y = proxy_size.h / original.size.h.max(f64::EPSILON);
    Rectangle::new(
        (
            (source.loc.x - original.loc.x) * scale_x,
            (source.loc.y - original.loc.y) * scale_y,
        )
            .into(),
        (source.size.w * scale_x, source.size.h * scale_y).into(),
    )
}

struct MaskedTextureDraw<'a> {
    src: Rectangle<f64, Buffer>,
    dst: Rectangle<i32, Physical>,
    damage: &'a [Rectangle<i32, Physical>],
    transform: Transform,
    alpha: f32,
}

fn draw_masked_texture(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    draw: MaskedTextureDraw<'_>,
    program: &GlesTexProgram,
    mask: MaskGeometry,
    content_color: (f32, f32, f32, f32),
) -> Result<(), GlesError> {
    let MaskedTextureDraw {
        src,
        dst,
        damage,
        transform,
        alpha,
    } = draw;
    // Recover coordinates in the original surface geometry from texture UVs.
    // `src`/`dst` may have been cropped or moved into an opening snapshot;
    // combining that relocated destination with an output-local clip shifts
    // the mask and removes the top/left of real clients.
    let uv_to_draw = texture_matrix(
        mask.source,
        mask.destination,
        texture.size(),
        transform,
        texture.is_y_inverted(),
    )
    .invert()
    .unwrap_or_else(Matrix3::identity);
    let uniforms = [
        Uniform::new(
            "clip_size",
            (mask.clip.size.w as f32, mask.clip.size.h as f32),
        ),
        Uniform::new(
            "draw_offset",
            (
                (mask.destination.loc.x - mask.clip.loc.x) as f32,
                (mask.destination.loc.y - mask.clip.loc.y) as f32,
            ),
        ),
        Uniform::new("corner_radii", (mask.radii.top, mask.radii.bottom)),
        Uniform::new(
            "uv_to_draw_col_0",
            (uv_to_draw.x.x, uv_to_draw.x.y, uv_to_draw.x.z),
        ),
        Uniform::new(
            "uv_to_draw_col_1",
            (uv_to_draw.y.x, uv_to_draw.y.y, uv_to_draw.y.z),
        ),
        Uniform::new(
            "uv_to_draw_col_2",
            (uv_to_draw.z.x, uv_to_draw.z.y, uv_to_draw.z.z),
        ),
        Uniform::new("content_color", content_color),
    ];
    frame.render_texture_from_to(
        texture,
        src,
        dst,
        damage,
        &[],
        transform,
        alpha,
        Some(program),
        &uniforms,
    )
}

fn texture_matrix(
    src: Rectangle<f64, Buffer>,
    destination: Rectangle<i32, Physical>,
    texture_size: smithay::utils::Size<i32, Buffer>,
    transform: Transform,
    y_inverted: bool,
) -> Matrix3<f32> {
    let transformed_source_size = transform.transform_size(src.size);
    let scale = transformed_source_size.to_f64() / destination.size.to_f64();
    let mut matrix = Matrix3::from_nonuniform_scale(scale.x as f32, scale.y as f32);
    let transformed_size = transformed_source_size;
    let translation = match transform {
        Transform::Normal => Matrix3::identity(),
        Transform::_90 => Matrix3::from_translation(Vector2::new(0.0, transformed_size.w as f32)),
        Transform::_180 => Matrix3::from_translation(Vector2::new(
            transformed_size.w as f32,
            transformed_size.h as f32,
        )),
        Transform::_270 => Matrix3::from_translation(Vector2::new(transformed_size.h as f32, 0.0)),
        Transform::Flipped => {
            Matrix3::from_translation(Vector2::new(transformed_size.w as f32, 0.0))
        }
        Transform::Flipped90 => Matrix3::identity(),
        Transform::Flipped180 => {
            Matrix3::from_translation(Vector2::new(0.0, transformed_size.h as f32))
        }
        Transform::Flipped270 => Matrix3::from_translation(Vector2::new(
            transformed_size.h as f32,
            transformed_size.w as f32,
        )),
    };
    let transform = transform.matrix().to_cols_array();
    matrix = Matrix3::new(
        transform[0],
        transform[1],
        0.0,
        transform[2],
        transform[3],
        0.0,
        transform[4],
        transform[5],
        1.0,
    ) * matrix;
    matrix = translation * matrix;
    matrix = Matrix3::from_translation(Vector2::new(src.loc.x as f32, src.loc.y as f32)) * matrix;
    matrix = Matrix3::from_nonuniform_scale(
        1.0 / texture_size.w.max(1) as f32,
        1.0 / texture_size.h.max(1) as f32,
    ) * matrix;
    if y_inverted {
        matrix = Matrix3::new(1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0) * matrix;
    }
    matrix
}

fn clamp_radius(radius: f32, width: i32, height: i32) -> f32 {
    radius.max(0.0).min(width.min(height).max(0) as f32 * 0.5)
}

fn clamp_radii(radii: CornerRadii, width: i32, height: i32) -> CornerRadii {
    CornerRadii {
        top: clamp_radius(radii.top, width, height),
        bottom: clamp_radius(radii.bottom, width, height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_commits_follow_shader_content() {
        let focused = smithay::backend::renderer::Color32F::new(0.8, 0.2, 0.1, 1.0);
        let unfocused = smithay::backend::renderer::Color32F::new(0.2, 0.2, 0.2, 1.0);
        assert_eq!(solid_color_commit(focused), solid_color_commit(focused));
        assert_ne!(solid_color_commit(focused), solid_color_commit(unfocused));
    }

    /// Rust mirror of the `rounded_alpha` GLSL in `SURFACE_SHADER` and
    /// `RESIZE_SHADER`. Kept in step by hand to pin the interior distance term.
    fn rounded_alpha(coords: (f32, f32), size: (f32, f32), radius: f32) -> f32 {
        let radius = radius.clamp(0.0, size.0.min(size.1) * 0.5);
        let half = (size.0 * 0.5, size.1 * 0.5);
        let q = (
            (coords.0 - half.0).abs() - (half.0 - radius),
            (coords.1 - half.1).abs() - (half.1 - radius),
        );
        let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
        let distance = outside + q.0.max(q.1).min(0.0) - radius;
        let t = ((distance + 0.75) / 1.5).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    }

    #[test]
    fn a_radius_easing_to_zero_never_dims_the_interior() {
        // The fullscreen transition eases the content radius to zero. Once it
        // drops under the smoothstep half-width, an interior-blind distance
        // reports -radius everywhere and fades the whole window toward 50%,
        // which read as an opacity flash just before the animation landed.
        let size = (1280.0, 800.0);
        for radius in [12.0, 1.0, 0.75, 0.5, 0.1, 0.001, 0.0] {
            let center = rounded_alpha((640.0, 400.0), size, radius);
            assert!(
                (center - 1.0).abs() < 1e-4,
                "radius {radius} dimmed the interior to {center}"
            );
        }
    }

    #[test]
    fn corners_are_still_rounded_and_edges_antialiased() {
        let size = (1280.0, 800.0);
        assert!(
            rounded_alpha((0.0, 0.0), size, 12.0) < 0.01,
            "corner is cut"
        );
        assert!(
            (rounded_alpha((0.0, 400.0), size, 12.0) - 0.5).abs() < 0.1,
            "straight edge keeps its half-covered antialiasing"
        );
    }
}
