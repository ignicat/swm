mod build;
pub use build::build;
pub(crate) use build::build_with_privacy;

use std::error::Error;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::CropRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, render_elements};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::{CommitCounter, with_renderer_surface_state};
use smithay::desktop::{PopupManager, layer_map_for_output};
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Rectangle, Scale};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::wlr_layer::Layer;

use super::RenderRequest;
use crate::presentation::window::window_visual_state;
#[cfg(test)]
mod animation_pixels;
mod capture_ui;
mod chrome;
mod close_stack;
mod desktop;
mod layers;
mod live_collection;
pub(crate) mod nodes;
mod previews;
mod privacy;
mod window_stack;
mod windows;

use capture_ui::capture_overlay_elements;
use layers::{LayerSceneStyle, layer_surface_scene_elements};
use nodes::{NodeElementContext, node_elements};
use previews::{FocusCycleRenderContext, focus_cycle_elements, hover_preview_elements};
use privacy::CapturePrivacy;
use windows::{
    LiveWindowContext, LiveWindowRenderers, LiveWindowScene, StackGroup, live_window_elements,
    sort_stack_groups,
};

#[cfg(test)]
use previews::aspect_fit_rect;

#[cfg(test)]
use smithay::utils::Point;

render_elements! {
    /// The complete front-to-back scene consumed by both presentation
    /// backends. Keeping one element type and one builder prevents nested and
    /// real-hardware sessions from drifting in z-order or visual policy.
    pub SceneElement<=GlesRenderer>;
    Cursor=crate::cursor::render::CursorRenderElement,
    Background=super::background::BackgroundElement,
    Rescaled=super::rescale::RescaledElement,
    Cropped=CropRenderElement<super::rescale::RescaledElement>,
    RoundedCropped=CropRenderElement<super::window_decoration::RoundedSurfaceElement>,
    WindowResize=super::resize::ResizeRenderElement,
    WindowShader=super::window_shader::WindowShaderRenderElement,
    WindowOpen=super::window_open::WindowOpenElement,
    RoundedTexture=super::window_decoration::RoundedTextureElement,
    Node=super::node::NodeRenderElement,
    NodeLabel=super::node::LabelRenderElement,
    NodeTexture=super::node::NodeTextureElement,
    DashedOutline=super::node::DashedOutlineElement,
    UiText=super::text::UiTextElement,
    Closing=smithay::backend::renderer::element::texture::TextureRenderElement<
        smithay::backend::renderer::gles::GlesTexture
    >,
    SourceChooser=super::overlays::source_chooser::SourceChooserElement,
    Border=SolidColorRenderElement,
    DisplayScaled=super::display_scale::DisplayScaledElement,
    Layer=WaylandSurfaceRenderElement<GlesRenderer>,
}

fn indicate_window_cast(scene: &mut LiveWindowScene, decorations: &swm_config::Decorations) {
    let bounds = scene
        .elements
        .iter()
        .map(|element| element.geometry(Scale::from(1.0)))
        .reduce(|a, b| a.merge(b));
    if let Some(bounds) = bounds {
        let width = decorations
            .screencast_indicator_size
            .min(bounds.size.w / 2)
            .min(bounds.size.h / 2);
        if width == 0 {
            return;
        }
        let inner = Rectangle::new(
            (bounds.loc.x + width, bounds.loc.y + width).into(),
            (bounds.size.w - 2 * width, bounds.size.h - 2 * width).into(),
        );
        let border = super::border_strips(
            std::array::from_fn(|_| Id::new()),
            inner,
            width,
            super::decoration_color(decorations.screencast_indicator_color),
        );
        scene
            .elements
            .splice(0..0, border.into_iter().map(SceneElement::Border));
    }
}

fn omit_elements(elements: &mut Vec<SceneElement>) {
    elements.clear();
}

fn omit_window_scene(scene: &mut LiveWindowScene) {
    omit_elements(&mut scene.elements);
    omit_elements(&mut scene.popup_elements);
}

fn build_logical(
    renderer: &mut GlesRenderer,
    output: &Output,
    primary_output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    request: &mut RenderRequest<'_>,
    privacy: CapturePrivacy,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    request.resources.node_renderer.poll_icons(renderer);
    request
        .resources
        .window_shaders
        .ensure(renderer, request.visuals.background_base);
    request.resources.node_renderer.begin_scene(&output.name());
    request.resources.pin_renderer.begin_scene(&output.name());
    request.resources.ui_text.begin_scene();
    if request.desktop.session_lock.active() {
        let scale = Scale::from(output.current_scale().fractional_scale());
        let mut elements = request
            .desktop
            .session_lock
            .surfaces_for_output(output)
            .flat_map(|surface| {
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    surface.wl_surface(),
                    (0, 0),
                    scale,
                    1.0,
                    Kind::ScanoutCandidate,
                )
            })
            .map(SceneElement::Layer)
            .collect::<Vec<_>>();
        let backdrop_size = crate::render::output_physical_size(output);
        elements.push(SceneElement::Border(super::solid_color_element(
            request
                .resources
                .node_renderer
                .slot_id(&output.name(), super::node::NodeSlot::SessionLockBackdrop),
            Rectangle::from_size(backdrop_size),
            super::SESSION_LOCK_COLOR,
        )));
        prepend_pointer_elements(
            renderer,
            output,
            output_geometry,
            &request.cursor,
            request.frame.target_presentation_time,
            false,
            &mut elements,
        )?;
        return Ok(elements);
    }

    request
        .desktop
        .cameras
        .view(&output.name())
        .ok_or_else(|| format!("output {:?} has no camera", output.name()))?;
    // These replacement scenes render cached previews that may have been
    // captured before a privacy rule was enabled. Fail closed during them.
    if privacy.redact_cached_scene() && request.overlays.focus_cycle.is_active() {
        return Ok(vec![SceneElement::Border(super::solid_color_element(
            Id::new(),
            Rectangle::from_size(output_geometry.size.to_physical(1)),
            smithay::backend::renderer::Color32F::BLACK,
        ))]);
    }
    let overlay_snapshot = request
        .overlays
        .overlays
        .snapshot(&output.name(), request.frame.target_presentation_time);

    let mut elements = capture_overlay_elements(
        renderer,
        output,
        output_geometry,
        request.overlays.capture_overlay,
        request.resources.node_renderer,
        request.overlays.overlay_config,
    )?;
    elements.extend(layer_surface_scene_elements(
        renderer,
        output,
        Layer::Overlay,
        LayerSceneStyle {
            layer_rules: request.desktop.layer_rules,

            block_screencast: privacy.redact_layers(),
        },
    )?);
    let focus_cycle = focus_cycle_elements(
        renderer,
        output_geometry,
        FocusCycleRenderContext {
            state: request.overlays.focus_cycle,
            nodes: request.desktop.nodes,
            overlay_config: request.overlays.overlay_config,
            decorations: request.visuals.decorations,
            fullscreen: request.desktop.fullscreen,
            maximize: request.desktop.maximize,
            now: request.frame.target_presentation_time,
        },
        request.resources.overlay_previews,
        previews::FocusCycleRenderers {
            decoration: request.resources.window_decoration_renderer,
            node: request.resources.node_renderer,
            text: request.resources.ui_text,
        },
    )?;
    elements.extend(focus_cycle);
    // Native XDG and X11 override-redirect popups use a desktop popup plane.
    // It sits in front of panels (`Layer::Top`) so menus are not clipped by a
    // status bar, while overlay surfaces and compositor UI remain authoritative.
    let popup_foreground_index = elements.len();
    if !request.desktop.fullscreen.covers_top_matching(
        request.desktop.focused,
        output,
        request.frame.target_presentation_time,
        |_| true,
    ) {
        elements.extend(layer_surface_scene_elements(
            renderer,
            output,
            Layer::Top,
            LayerSceneStyle {
                layer_rules: request.desktop.layer_rules,

                block_screencast: privacy.redact_layers(),
            },
        )?);
    }
    desktop::append(
        renderer,
        output,
        primary_output,
        output_geometry,
        request,
        desktop::DesktopPlacement {
            popup: popup_foreground_index,
            privacy,
        },
        &mut elements,
    )?;

    elements.extend(layer_surface_scene_elements(
        renderer,
        output,
        Layer::Bottom,
        LayerSceneStyle {
            layer_rules: request.desktop.layer_rules,

            block_screencast: privacy.redact_layers(),
        },
    )?);
    elements.extend(layer_surface_scene_elements(
        renderer,
        output,
        Layer::Background,
        LayerSceneStyle {
            layer_rules: request.desktop.layer_rules,

            block_screencast: privacy.redact_layers(),
        },
    )?);
    append_background(
        renderer,
        request.resources.background_renderer,
        BackgroundSceneRequest {
            output_name: &output.name(),
            output_geometry,
            config: request.visuals.background,
            config_dir: request.visuals.background_base,
        },
        &mut elements,
    );

    chrome::prepend_common_overlays(
        renderer,
        output,
        output_geometry,
        request,
        overlay_snapshot,
        &mut elements,
    )?;

    prepend_pointer_elements(
        renderer,
        output,
        output_geometry,
        &request.cursor,
        request.frame.target_presentation_time,
        true,
        &mut elements,
    )?;

    Ok(elements)
}

fn prepend_pointer_elements(
    renderer: &mut GlesRenderer,
    output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    cursor_context: &super::CursorContext<'_>,
    time: std::time::Duration,
    show_dnd_icon: bool,
    elements: &mut Vec<SceneElement>,
) -> Result<(), Box<dyn Error>> {
    if show_dnd_icon {
        let dnd = crate::wayland::dnd::elements(
            renderer,
            cursor_context.dnd_icon,
            output,
            output_geometry,
            cursor_context.cursor_position,
        );
        elements.splice(0..0, dnd.into_iter().map(SceneElement::Layer));
    }
    if cursor_context.show_cursor {
        let cursor = crate::cursor::render::elements(
            renderer,
            cursor_context.cursor,
            output,
            output_geometry,
            cursor_context.cursor_position,
            time,
            cursor_context.cursor_override,
        )?;
        // Element lists are front-to-back, so cursor surface trees belong
        // immediately in front of the drag icon and every other element.
        elements.splice(0..0, cursor.into_iter().map(SceneElement::Cursor));
    }
    Ok(())
}

struct BackgroundSceneRequest<'a> {
    output_name: &'a str,
    output_geometry: Rectangle<i32, Logical>,
    config: &'a swm_config::Background,
    config_dir: Option<&'a std::path::Path>,
}

fn append_background(
    renderer: &mut GlesRenderer,
    background_renderer: &mut super::background::BackgroundRenderer,
    request: BackgroundSceneRequest<'_>,
    elements: &mut Vec<SceneElement>,
) {
    let output_name = request.output_name;

    if let Some(background) = background_renderer.element(
        renderer,
        super::background::BackgroundRequest {
            output_name,
            output_size: request.output_geometry.size.to_physical(1),
            config: request.config,
            config_dir: request.config_dir,
        },
    ) {
        // Scene lists are front-to-back. Appending after the protocol
        // background layer makes compositor backgrounds the absolute back.
        elements.push(SceneElement::Background(background));
    }
}
#[cfg(test)]
mod tests;
