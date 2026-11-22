use super::*;
use crate::render::fullscreen_texture::TransitionFrame;

pub(super) struct StackGroup {
    pub(super) window: Option<smithay::desktop::Window>,
    pub(super) stack_index: usize,
    pub(super) order: u64,
    pub(super) elements: Vec<SceneElement>,
}

pub(super) fn sort_stack_groups(groups: &mut [StackGroup]) {
    groups.sort_by_key(|group| (group.stack_index, group.order));
}

pub(super) struct LiveWindowScene {
    pub(super) window: smithay::desktop::Window,
    /// XDG popup trees are composed in the desktop popup plane, above the
    /// top layer-shell plane but below overlays. Keeping them separate from
    /// the root preserves that plane while the root remains in its stack.
    pub(super) popup_elements: Vec<SceneElement>,
    pub(super) elements: Vec<SceneElement>,
    pub(super) above_field_landmarks: bool,
}

pub(super) struct LiveWindowRenderers<'a> {
    pub fullscreen_textures:
        &'a mut crate::render::fullscreen_texture::FullscreenTextureTransitions,
    pub node: &'a mut crate::render::node::NodeRenderer,
    pub pin: &'a mut crate::render::pin::PinRenderer,
    pub window_shaders: &'a mut crate::render::window_shader::WindowAnimationShaders,
}

impl LiveWindowRenderers<'_> {
    pub(super) fn reborrow(&mut self) -> LiveWindowRenderers<'_> {
        LiveWindowRenderers {
            fullscreen_textures: &mut *self.fullscreen_textures,
            node: &mut *self.node,
            pin: &mut *self.pin,
            window_shaders: &mut *self.window_shaders,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct LiveWindowContext<'a> {
    pub(super) space: &'a smithay::desktop::Space<smithay::desktop::Window>,
    pub(super) output: &'a Output,
    pub(super) output_geometry: Rectangle<i32, Logical>,
    pub(super) cameras: &'a crate::presentation::camera::OutputCameras,
    pub(super) nodes: &'a crate::nodes::NodesState,
    pub(super) target_presentation_time: std::time::Duration,
    pub(super) focused:
        Option<&'a smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    pub(super) decorations: &'a swm_config::Decorations,
    pub(super) pins: &'a swm_config::Pins,
    pub(super) overlays: &'a swm_config::Overlays,
    pub(super) window_animations: &'a crate::animation::WindowAnimations,
    pub(super) fullscreen: &'a crate::wayland::fullscreen::FullscreenManager,
    pub(super) maximize: &'a crate::presentation::maximize::FieldMaximizeManager,
    pub(super) window_rules: &'a crate::window::rules::WindowRulesState,
    pub(super) instance_identity: Option<&'static str>,
}

/// Keep the accepted live endpoint in the offscreen blend until motion is
/// actually complete. Retiring at 99.5% exposed any client repaint since the
/// first accepted frame as a final-frame jump, particularly through Xwayland.
const CROSSFADE_COMPLETE: f64 = 1.0;

fn active_crossfade_completion(completion: Option<f64>) -> Option<f64> {
    completion.filter(|completion| *completion < CROSSFADE_COMPLETE)
}

/// Window-rule opacity fades client pixels; compositor chrome uses only the
/// opening fade.
fn window_content_and_chrome_alpha(
    opening_alpha: f32,
    rule_opacity: f32,
    chrome_visible: bool,
) -> (f32, f32) {
    let content_alpha = opening_alpha * rule_opacity;
    let chrome_alpha = if chrome_visible { opening_alpha } else { 0.0 };
    (content_alpha, chrome_alpha)
}

fn should_hold_x11_fullscreen_exit(
    is_x11: bool,
    has_fullscreen_presentation: bool,
    is_fullscreen_or_pending: bool,
) -> bool {
    is_x11 && has_fullscreen_presentation && !is_fullscreen_or_pending
}

pub(super) fn live_window_elements(
    renderer: &mut GlesRenderer,
    window: &smithay::desktop::Window,
    context: LiveWindowContext<'_>,
    renderers: LiveWindowRenderers<'_>,
) -> Result<LiveWindowScene, Box<dyn Error>> {
    let LiveWindowRenderers {
        fullscreen_textures,
        node: _node_renderer,
        pin: pin_renderer,
        window_shaders,
    } = renderers;
    let Some(location) = context.space.element_location(window) else {
        return Ok(LiveWindowScene {
            window: window.clone(),
            popup_elements: Vec::new(),
            elements: Vec::new(),
            above_field_landmarks: false,
        });
    };
    let Some(window_surface) = window.wl_surface() else {
        return Ok(LiveWindowScene {
            window: window.clone(),
            popup_elements: Vec::new(),
            elements: Vec::new(),
            above_field_landmarks: false,
        });
    };
    let Some(visual) = window_visual_state(
        crate::presentation::window::WindowVisualContext {
            space: context.space,
            cameras: context.cameras,
            output: context.output,
            window_animations: context.window_animations,
            fullscreen: context.fullscreen,
            maximize: context.maximize,
            decorations: context.decorations,
            now: context.target_presentation_time,
        },
        window,
    ) else {
        return Ok(LiveWindowScene {
            window: window.clone(),
            popup_elements: Vec::new(),
            elements: Vec::new(),
            above_field_landmarks: false,
        });
    };
    if visual.animated_rect.size.w == 0 || visual.animated_rect.size.h == 0 {
        return Ok(LiveWindowScene {
            window: window.clone(),
            popup_elements: Vec::new(),
            elements: Vec::new(),
            above_field_landmarks: visual.above_field_landmarks,
        });
    }

    let mut popup_elements = Vec::new();
    let mut elements = Vec::new();
    let chrome = crate::window_chrome::WindowChrome::for_window(window, context.decorations);
    let managed = chrome.mode != crate::window_chrome::DecorationMode::Unmanaged;
    let rule_opacity = if managed {
        context.window_rules.opacity(window_surface.as_ref())
    } else {
        1.0
    };
    // X11 clients can churn fullscreen requests while changing video modes.
    // Keep their advertised EWMH state as a render-time backstop so a missing
    // or temporarily retired presentation entry cannot expose compositor
    // chrome around an otherwise-fullscreen game.
    let chrome_visible = !context.fullscreen.suppresses_window_chrome(window);
    let (content_alpha, chrome_alpha) =
        window_content_and_chrome_alpha(visual.opening_alpha, rule_opacity, chrome_visible);
    let chrome_alpha = chrome_alpha
        * context
            .fullscreen
            .chrome_alpha(window_surface.as_ref(), context.target_presentation_time);
    let node_id = context.nodes.id_for_surface(window_surface.as_ref());
    let user_pinned = node_id.is_some_and(|id| {
        context
            .nodes
            .field()
            .node(id)
            .is_some_and(|node| node.pinned)
    });
    let presentation_scale_y =
        decoration_presentation_scale(visual.presentation_rect.size.h, visual.animated_rect.size.h);
    let decoration_scale = visual.zoom_scale * presentation_scale_y.max(0.0);
    let border_width = crate::window_chrome::scaled_metric(chrome.border_width, decoration_scale);
    let surface_location = crate::render::window_surface_location(location, window.geometry());
    let (popup_surfaces, surface_elements) =
        crate::render::window_surface_elements(renderer, window, surface_location, content_alpha);
    popup_elements.extend(popup_surfaces.into_iter().map(|surface_element| {
        let native_geometry = surface_element.geometry(Scale::from(1.0));
        let destination = if visual.maps_from_source() {
            let destination = crate::animation::map_rect(
                native_geometry,
                visual.source_geometry.to_physical(1),
                visual.presentation_rect,
            );
            crate::animation::map_rect(destination, visual.presentation_rect, visual.animated_rect)
        } else {
            let final_destination = crate::presentation::camera::camera_rect(
                native_geometry,
                visual.camera_center,
                context.output_geometry.size.to_physical(1),
                visual.zoom_scale,
            );
            crate::animation::map_rect(final_destination, visual.camera_rect, visual.animated_rect)
        };
        SceneElement::Rescaled(crate::render::rescale::RescaledElement::new(
            surface_element,
            destination,
        ))
    }));
    let texture_transition_completion = active_crossfade_completion(
        visual
            .fullscreen
            .map(|presentation| presentation.transition_completion)
            .or_else(|| {
                visual
                    .maximize
                    .map(|presentation| presentation.transition_completion)
            }),
    );
    let texture_blend = if let Some(completion) = texture_transition_completion {
        let hold_x11_fullscreen_exit = should_hold_x11_fullscreen_exit(
            crate::xwayland::is_x11(window),
            visual.fullscreen.is_some(),
            context
                .fullscreen
                .is_fullscreen_or_pending(window_surface.as_ref()),
        );
        match fullscreen_textures.blend_element(
            renderer,
            crate::render::fullscreen_texture::BlendRequest {
                window,
                destination: visual.animated_rect,
                progress: completion,
                hold_previous_until_restored_buffer_matches: hold_x11_fullscreen_exit,
                alpha: content_alpha,
            },
        ) {
            Ok(blend) => blend,
            Err(err) => {
                eventline::warn!("window transition: failed to blend textures: {err}");
                None
            }
        }
    } else {
        None
    };
    if let Some(TransitionFrame::Snapshot(blend)) = texture_blend {
        elements.push(SceneElement::WindowResize(blend));
    } else {
        // Elements are front-to-back: the fading outgoing snapshot is pushed
        // first so the live tree underneath shows through as it fades.
        if let Some(TransitionFrame::Live {
            outgoing: Some(outgoing),
        }) = texture_blend
        {
            elements.push(SceneElement::WindowResize(outgoing));
        }
        for surface_element in surface_elements {
            let native_geometry = surface_element.geometry(Scale::from(1.0));
            let destination = if visual.maps_from_source() {
                let destination = crate::animation::map_rect(
                    native_geometry,
                    visual.source_geometry.to_physical(1),
                    visual.presentation_rect,
                );
                crate::animation::map_rect(
                    destination,
                    visual.presentation_rect,
                    visual.animated_rect,
                )
            } else {
                let final_destination = crate::presentation::camera::camera_rect(
                    native_geometry,
                    visual.camera_center,
                    context.output_geometry.size.to_physical(1),
                    visual.zoom_scale,
                );
                crate::animation::map_rect(
                    final_destination,
                    visual.camera_rect,
                    visual.animated_rect,
                )
            };
            let element =
                crate::render::rescale::RescaledElement::new(surface_element, destination);
            if let Some(element) =
                CropRenderElement::from_element(element, 1.0, visual.animated_rect)
            {
                elements.push(SceneElement::Cropped(element));
            }
        }
    }

    let is_focused = Some(window_surface.as_ref()) == context.focused;
    let border_color = crate::render::window_border_color(context.decorations, is_focused);
    if managed
        && context.window_rules.applied(window_surface.as_ref()).border != Some(false)
        && border_width > 0
        && chrome_alpha > 0.0
    {
        let strips = crate::render::border_strips(
            std::array::from_fn(|index| {
                crate::render::window_decoration::surface_slot_for_instance(
                    window_surface.as_ref(),
                    crate::render::window_decoration::slot::BORDER_FALLBACK + index,
                    context.instance_identity,
                )
            }),
            visual.animated_rect,
            border_width,
            border_color * chrome_alpha,
        );
        elements.extend(strips.into_iter().map(SceneElement::Border));
    }
    if user_pinned
        && chrome_visible
        && let Some(pin) = pin_renderer.element(
            renderer,
            &context.output.name(),
            crate::render::pin::PinSlot::Window(
                node_id.expect("pinned window has a node").as_u64(),
            ),
            crate::render::pin::window_badge_rect(
                context.pins,
                visual.animated_rect,
                visual.zoom_scale,
            ),
            chrome_alpha,
            crate::render::pin::PinTheme {
                pins: context.pins,
                overlays: context.overlays,
                decorations: context.decorations,
            },
        )
    {
        elements.insert(0, SceneElement::Closing(pin));
    }
    // Build the normal live scene first, then pass that complete composition
    // through the opening shader.
    if visual.shader_pixels()
        && window_shaders.open_available()
        && visual.fullscreen.is_none()
        && visual.maximize.is_none()
        && let Some(shader) = window_shaders.open_scene_element(
            renderer,
            crate::render::window_decoration::surface_slot_for_instance(
                window_surface.as_ref(),
                crate::render::window_decoration::slot::WINDOW_OPEN,
                context.instance_identity,
            ),
            &mut elements,
            crate::render::window_shader::ShaderTiming {
                progress: visual.opening_progress(),
                clamped_progress: visual.opening_clamped_progress(),
                random_seed: visual.opening_random_seed(),
            },
        )
    {
        elements.push(SceneElement::WindowOpen(shader));
    }
    // X11 models menus as independent override-redirect windows rather than
    // XDG popup roles. Put their complete unmanaged scene in the same desktop
    // popup plane so native Wayland and XWayland applications agree.
    if crate::xwayland::is_override_redirect(window) {
        popup_elements.append(&mut elements);
    }
    Ok(LiveWindowScene {
        window: window.clone(),
        popup_elements,
        elements,
        above_field_landmarks: visual.above_field_landmarks,
    })
}

fn decoration_presentation_scale(presentation_height: i32, animated_height: i32) -> f32 {
    if presentation_height > 0 {
        animated_height as f32 / presentation_height as f32
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        active_crossfade_completion, decoration_presentation_scale,
        should_hold_x11_fullscreen_exit, window_content_and_chrome_alpha,
    };

    #[test]
    fn accepted_endpoint_stays_offscreen_until_motion_exactly_finishes() {
        assert_eq!(active_crossfade_completion(Some(0.995)), Some(0.995));
        assert_eq!(
            active_crossfade_completion(Some(0.999_999)),
            Some(0.999_999)
        );
        assert_eq!(active_crossfade_completion(Some(1.0)), None);
    }

    #[test]
    fn presentation_scale_ratio_handles_zero_presentation_height() {
        assert_eq!(decoration_presentation_scale(0, 900), 1.0);
        assert_eq!(decoration_presentation_scale(600, 900), 1.5);
    }

    #[test]
    fn window_rule_opacity_fades_content_not_chrome() {
        let (content, chrome) = window_content_and_chrome_alpha(1.0, 0.9, true);
        assert_eq!(content, 0.9);
        assert_eq!(chrome, 1.0);
    }

    #[test]
    fn opening_fade_still_applies_to_chrome() {
        let (content, chrome) = window_content_and_chrome_alpha(0.5, 0.9, true);
        assert_eq!(content, 0.45);
        assert_eq!(chrome, 0.5);
    }

    #[test]
    fn hidden_chrome_stays_invisible_regardless_of_rule_opacity() {
        let (content, chrome) = window_content_and_chrome_alpha(1.0, 0.9, false);
        assert_eq!(content, 0.9);
        assert_eq!(chrome, 0.0);
    }

    #[test]
    fn only_x11_fullscreen_exit_waits_for_the_restored_buffer() {
        assert!(should_hold_x11_fullscreen_exit(true, true, false));
        assert!(!should_hold_x11_fullscreen_exit(true, true, true));
        assert!(!should_hold_x11_fullscreen_exit(true, false, false));
        assert!(!should_hold_x11_fullscreen_exit(false, true, false));
    }
}
