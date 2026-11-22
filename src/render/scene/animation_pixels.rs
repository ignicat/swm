use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::egl::{EGLContext, EGLDisplay, native::EGLSurfacelessDisplay};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::backend::renderer::{Bind, Color32F, ExportMem, Offscreen, Texture};
use smithay::output::{Mode, PhysicalProperties, Subpixel};
use smithay::utils::Transform;

use super::*;
use crate::render::node::NodeRenderer;
use crate::render::text::UiTextRenderer;
use crate::session::tty::FrameDemand;
use crate::shell::overlay::{OverlayManager, OverlaySnapshot};

const SIZE: (i32, i32) = (641, 481);

fn output(name: &str) -> Output {
    let output = Output::new(
        name.into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "test".into(),
            model: "test".into(),
            serial_number: name.into(),
        },
    );
    output.change_current_state(
        Some(Mode {
            size: SIZE.into(),
            refresh: 60_000,
        }),
        Some(Transform::Normal),
        Some(smithay::output::Scale::Integer(1)),
        Some((0, 0).into()),
    );
    output
}

struct SceneResources {
    nodes: NodeRenderer,
    text: UiTextRenderer,
    background: [Id; 3],
}

impl SceneResources {
    fn new() -> Self {
        Self {
            nodes: NodeRenderer::default(),
            text: UiTextRenderer::new(&swm_config::Font::default()),
            background: std::array::from_fn(|_| Id::new()),
        }
    }

    fn scene(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        snapshot: OverlaySnapshot,
        config: &swm_config::Overlays,
        label_mix: Option<f32>,
    ) -> Vec<SceneElement> {
        let scale = output.current_scale().fractional_scale();
        self.text.set_display_scale(scale);
        self.nodes.begin_scene(&output.name());
        self.text.begin_scene();
        let mut elements = crate::render::overlays::shell::elements(
            &mut crate::render::draw::DrawContext {
                renderer,
                node: &mut self.nodes,
                text: &mut self.text,
            },
            Rectangle::from_size(SIZE.into()),
            snapshot,
            config,
        )
        .unwrap();
        if let Some(hover_mix) = label_mix {
            elements.extend(
                nodes::landmark_label_elements(
                    renderer,
                    &mut self.nodes,
                    &mut self.text,
                    nodes::LandmarkLabel {
                        center: (220, 220),
                        marker_side: 64,
                        output_size: SIZE,
                        text: "Landmark label",
                        shape: swm_config::NodeShape::Squircle,
                        fill: (0.15, 0.18, 0.22),
                        ring: (0.6, 0.3, 0.1),
                        hover_mix,
                        alpha: 0.9,
                    },
                )
                .unwrap(),
            );
        }
        for (id, rect, color) in [
            (
                &self.background[0],
                Rectangle::new((210, 0).into(), (100, SIZE.1).into()),
                Color32F::new(0.1, 0.7, 0.2, 1.0),
            ),
            (
                &self.background[1],
                Rectangle::new((0, 320).into(), (SIZE.0, 45).into()),
                Color32F::new(0.8, 0.2, 0.1, 1.0),
            ),
            (
                &self.background[2],
                Rectangle::from_size(SIZE.into()),
                Color32F::new(0.1, 0.2, 0.4, 1.0),
            ),
        ] {
            elements.push(SceneElement::Border(crate::render::solid_color_element(
                id.clone(),
                rect,
                color,
            )));
        }
        elements
            .into_iter()
            .map(|element| crate::render::display_scale::element(element, scale, false))
            .collect()
    }
}

fn read_pixels<E: RenderElement<GlesRenderer>>(
    renderer: &mut GlesRenderer,
    target: &mut GlesTexture,
    tracker: &mut OutputDamageTracker,
    scene: &[E],
    age: usize,
) -> (Vec<u8>, Vec<Rectangle<i32, Physical>>) {
    let size = target.size();
    let mut fbo = renderer.bind(target).unwrap();
    let result = tracker
        .render_output(renderer, &mut fbo, age, scene, Color32F::BLACK)
        .unwrap();
    result.sync.wait().unwrap();
    let damage = result.damage.cloned().unwrap_or_default();
    let mapping = renderer
        .copy_framebuffer(&fbo, Rectangle::from_size(size), Fourcc::Abgr8888)
        .unwrap();
    (renderer.map_texture(&mapping).unwrap().to_vec(), damage)
}

#[test]
#[ignore = "requires surfaceless GLES; run with LIBGL_ALWAYS_SOFTWARE=1 and --ignored"]
fn conservative_animation_pixels_match_full_repaint_with_reused_buffers() {
    let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }.unwrap();
    let context = EGLContext::new(&display).unwrap();
    let mut renderer = unsafe { GlesRenderer::new(context) }.unwrap();
    let actual_output = output("actual");
    let reference_output = output("reference");
    let mut comparisons = 0;
    for buffers in 1..=3 {
        for case in ["notification", "zoom"] {
            let config = swm_config::Overlays {
                zoom_indicator: swm_config::ZoomIndicator {
                    hold_duration_ms: 120,
                    fade_duration_ms: 180,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut overlays = OverlayManager::default();
            if case == "notification" {
                overlays.show_config_error("actual".into(), 300, Duration::ZERO);
            } else if case == "zoom" {
                overlays.show_zoom_indicator(
                    "actual",
                    0.75,
                    &config.zoom_indicator,
                    Duration::ZERO,
                );
            }
            let mut actual_resources = SceneResources::new();
            let mut reference_resources = SceneResources::new();
            let mut targets: Vec<GlesTexture> = (0..buffers)
                .map(|_| {
                    renderer
                        .create_buffer(Fourcc::Abgr8888, SIZE.into())
                        .unwrap()
                })
                .collect();
            let mut reference = renderer
                .create_buffer(Fourcc::Abgr8888, SIZE.into())
                .unwrap();
            let mut tracker = OutputDamageTracker::new(SIZE, 1.0, Transform::Normal);
            let mut saw_forced_full = false;
            let mut saw_reused = false;
            let mut saw_settled = false;
            let mut saw_visible = false;
            // Include enough settled frames to drain damage history for all buffer ages.
            let ticks = 42;
            for tick in 0..ticks {
                let now = Duration::from_millis(tick as u64 * 15);
                overlays.wakeup(now);
                let label_mix = None;
                let local_animating = overlays.animating_on_output("actual", now);
                let demand = FrameDemand::new(false, local_animating);
                assert_eq!(demand.keep_redrawing, local_animating);
                assert_eq!(demand.force_full_repaint, local_animating);
                let snapshot = overlays.snapshot("actual", now);
                let scene = actual_resources.scene(
                    &mut renderer,
                    &actual_output,
                    snapshot.clone(),
                    &config,
                    label_mix,
                );
                saw_visible |= scene
                    .iter()
                    .any(|e| matches!(e, SceneElement::UiText(_)) && e.alpha() > 0.1);
                let age = if tick < buffers || demand.force_full_repaint {
                    0
                } else {
                    buffers
                };
                saw_reused |= age == buffers;
                let (actual, damage) = read_pixels(
                    &mut renderer,
                    &mut targets[tick % buffers],
                    &mut tracker,
                    &scene,
                    age,
                );
                let reference_scene = reference_resources.scene(
                    &mut renderer,
                    &reference_output,
                    snapshot,
                    &config,
                    label_mix,
                );
                let (expected, _) = read_pixels(
                    &mut renderer,
                    &mut reference,
                    &mut OutputDamageTracker::new(SIZE, 1.0, Transform::Normal),
                    &reference_scene,
                    0,
                );
                let max_error = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
                assert!(
                    max_error <= 1,
                    "case={case} buffers={buffers} tick={tick} max_error={max_error}"
                );
                if tick >= buffers {
                    if demand.force_full_repaint {
                        assert!(
                            damage
                                .iter()
                                .any(|r| r.size.w * r.size.h == SIZE.0 * SIZE.1),
                            "animated {case} must repaint the entire output"
                        );
                        saw_forced_full = true;
                    } else {
                        saw_settled |= !demand.keep_redrawing && damage.is_empty();
                    }
                }
                comparisons += 1;
            }
            assert!(
                saw_visible && saw_forced_full && saw_reused && saw_settled,
                "case={case} buffers={buffers}"
            );
        }
    }
    eprintln!(
        "{comparisons} conservative-animation GLES comparisons passed, buffer ages 1-3, max permitted channel error=1/255"
    );
}

#[test]
#[ignore = "requires surfaceless GLES; run with LIBGL_ALWAYS_SOFTWARE=1 and --ignored"]
fn display_scaling_matches_full_repaint_with_text_and_buffer_reuse() {
    let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }.unwrap();
    let context = EGLContext::new(&display).unwrap();
    let mut renderer = unsafe { GlesRenderer::new(context) }.unwrap();
    let mut comparisons = 0;
    for scale in [0.75, 1.0, 1.25, 1.5, 2.0] {
        let size = (
            (SIZE.0 as f64 * scale).round() as i32,
            (SIZE.1 as f64 * scale).round() as i32,
        );
        let actual_output = output("scaled-actual");
        let reference_output = output("scaled-reference");
        for output in [&actual_output, &reference_output] {
            output.change_current_state(
                Some(Mode {
                    size: size.into(),
                    refresh: 60_000,
                }),
                None,
                Some(smithay::output::Scale::Fractional(scale)),
                None,
            );
        }
        let mut actual_resources = SceneResources::new();
        let mut reference_resources = SceneResources::new();
        let mut actual_target = renderer
            .create_buffer(Fourcc::Abgr8888, size.into())
            .unwrap();
        let mut reference_target = renderer
            .create_buffer(Fourcc::Abgr8888, size.into())
            .unwrap();
        let mut tracker = OutputDamageTracker::new(size, 1.0, Transform::Normal);
        let config = swm_config::Overlays::default();
        let mut overlays = OverlayManager::default();
        overlays.show_config_error("scaled-actual".into(), 300, Duration::ZERO);
        for tick in 0..45 {
            let now = Duration::from_millis(tick * 20);
            overlays.wakeup(now);
            let snapshot = overlays.snapshot("scaled-actual", now);
            let scene = actual_resources.scene(
                &mut renderer,
                &actual_output,
                snapshot.clone(),
                &config,
                None,
            );
            let reference_scene = reference_resources.scene(
                &mut renderer,
                &reference_output,
                snapshot,
                &config,
                None,
            );
            let age = usize::from(tick > 0 && !overlays.animating_on_output("scaled-actual", now));
            let (actual, _) =
                read_pixels(&mut renderer, &mut actual_target, &mut tracker, &scene, age);
            let (reference, _) = read_pixels(
                &mut renderer,
                &mut reference_target,
                &mut OutputDamageTracker::new(size, 1.0, Transform::Normal),
                &reference_scene,
                0,
            );
            assert_eq!(actual, reference, "scale={scale} tick={tick}");
            // A stripe at logical x=210..310 must retain the configured width and position.
            let (x, y) = (
                (250.0 * scale).round() as usize,
                (100.0 * scale).round() as usize,
            );
            let pixel = &actual[(y * size.0 as usize + x) * 4..][..4];
            assert!(
                pixel[1] > 150 && pixel[0] < 40 && pixel[2] < 80,
                "stripe was misplaced at scale={scale}: {pixel:?}"
            );
            comparisons += 1;
        }
    }
    println!("verified {comparisons} scaled frames with independent full-repaint references");
}

#[test]
#[ignore = "requires surfaceless GLES; run with LIBGL_ALWAYS_SOFTWARE=1 and --ignored"]
fn display_scale_preserves_native_client_pixels_without_double_scaling() {
    use smithay::backend::renderer::element::memory::{
        MemoryRenderBuffer, MemoryRenderBufferRenderElement,
    };
    let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }.unwrap();
    let context = EGLContext::new(&display).unwrap();
    let mut renderer = unsafe { GlesRenderer::new(context) }.unwrap();
    let size = (32, 32);
    let pixels = (0..size.1)
        .flat_map(|_| {
            (0..size.0).flat_map(|x| {
                let value = if x % 2 == 0 { 0 } else { 255 };
                [value, value, value, 255]
            })
        })
        .collect::<Vec<u8>>();
    let buffer =
        MemoryRenderBuffer::from_slice(&pixels, Fourcc::Abgr8888, size, 2, Transform::Normal, None);
    let inner = MemoryRenderBufferRenderElement::from_buffer(
        &mut renderer,
        (0.0, 0.0),
        &buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    )
    .unwrap();
    let element = crate::render::display_scale::element(
        SceneElement::SourceChooser(
            crate::render::overlays::source_chooser::SourceChooserElement::Icon(inner),
        ),
        2.0,
        false,
    );
    assert_eq!(element.geometry(1.0.into()).size, size.into());
    assert_eq!(element.geometry(2.0.into()).size, size.into());
    let mut target = renderer
        .create_buffer(Fourcc::Abgr8888, size.into())
        .unwrap();
    let (actual, _) = read_pixels(
        &mut renderer,
        &mut target,
        &mut OutputDamageTracker::new(size, 2.0, Transform::Normal),
        &[element],
        0,
    );
    assert_eq!(actual, pixels, "native one-pixel stripes must stay sharp");
}
