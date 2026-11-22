//! Assemble the Field desktop and its window, node, and popup planes.

use super::*;

/// Insertion positions established by the overlay and top-layer pass.
pub(super) struct DesktopPlacement {
    pub(super) popup: usize,
    pub(super) privacy: CapturePrivacy,
}

pub(super) fn append(
    renderer: &mut GlesRenderer,
    output: &Output,
    primary_output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    request: &mut RenderRequest<'_>,
    placement: DesktopPlacement,
    elements: &mut Vec<SceneElement>,
) -> Result<(), Box<dyn Error>> {
    let privacy = placement.privacy;

    let hover_preview = hover_preview_elements(
        super::previews::PreviewScene {
            renderer,
            output,
            output_geometry,
            desktop: super::previews::PreviewDesktop::from_request(request),
            draw: super::previews::PreviewRenderers {
                node: request.resources.node_renderer,
                decoration: request.resources.window_decoration_renderer,
                text: request.resources.ui_text,
                previews: request.resources.overlay_previews,
            },
            now: request.frame.target_presentation_time,
        },
        request.overlays.overlay_config,
    )?;
    if !privacy.redact_cached_scene() {
        elements.extend(hover_preview);
    }

    let node_groups = node_elements(
        renderer,
        request.resources.node_renderer,
        request.resources.ui_text,
        NodeElementContext {
            output,
            output_geometry,
            nodes: request.desktop.nodes,
            node_grab_active: request.desktop.node_grab_active,
            cameras: request.desktop.cameras,
            decorations: request.visuals.decorations,
            pins: request.visuals.pins,
            overlays: request.overlays.overlay_config,
            pin_renderer: request.resources.pin_renderer,
            now: request.frame.target_presentation_time,
        },
    )?;

    let mut stack = close_stack::stack_groups(
        renderer,
        output,
        output_geometry,
        close_stack::ClosingSceneContext {
            animations: request.resources.window_close_animations,
            shaders: request.resources.window_shaders,
            cameras: request.desktop.cameras,
            now: request.frame.target_presentation_time,
            redact: privacy.redact_cached_windows(),
        },
    )?;
    stack.extend(node_groups);
    let context = LiveWindowContext {
        space: request.desktop.space,
        output,
        output_geometry,
        cameras: request.desktop.cameras,
        nodes: request.desktop.nodes,
        target_presentation_time: request.frame.target_presentation_time,
        focused: request.desktop.focused,
        decorations: request.visuals.decorations,
        pins: request.visuals.pins,
        overlays: request.overlays.overlay_config,
        window_animations: request.desktop.window_animations,
        fullscreen: request.desktop.fullscreen,
        maximize: request.desktop.maximize,
        window_rules: request.desktop.window_rules,
        instance_identity: None,
    };
    let collection = live_collection::collect(
        renderer,
        primary_output,
        context,
        LiveWindowRenderers {
            fullscreen_textures: request.resources.fullscreen_textures,
            node: request.resources.node_renderer,
            pin: request.resources.pin_renderer,
            window_shaders: request.resources.window_shaders,
        },
        request.resources.window_close_animations,
        request.desktop.screencast,
        privacy,
    )?;
    let live_windows = collection.normal;
    let planes = window_stack::arrange(request.desktop.space, stack, live_windows);
    elements.extend(planes.desktop);
    let mut popup_stack = planes.popups;

    sort_stack_groups(&mut popup_stack);
    crate::window::stacking::sort_above_parents(request.desktop.space, &mut popup_stack, |group| {
        group.window.as_ref()
    });
    elements.splice(
        placement.popup..placement.popup,
        popup_stack
            .into_iter()
            .rev()
            .flat_map(|group| group.elements),
    );

    Ok(())
}
