//! Shared shell overlay composition for all scene modes.

use super::*;

pub(super) fn prepend_common_overlays(
    renderer: &mut GlesRenderer,
    _output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    request: &mut RenderRequest<'_>,
    overlay_snapshot: crate::shell::overlay::OverlaySnapshot,
    elements: &mut Vec<SceneElement>,
) -> Result<(), Box<dyn Error>> {
    let overlay_elements = crate::render::overlays::shell::elements(
        &mut crate::render::draw::DrawContext {
            renderer,
            node: request.resources.node_renderer,
            text: request.resources.ui_text,
        },
        output_geometry,
        overlay_snapshot,
        request.overlays.overlay_config,
    )?;
    elements.splice(0..0, overlay_elements);

    Ok(())
}
