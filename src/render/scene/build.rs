//! Scene entry points, capture privacy, and output scaling.

use super::*;

pub fn build(
    renderer: &mut GlesRenderer,
    output: &Output,
    primary_output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    request: RenderRequest<'_>,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    build_with_privacy(
        renderer,
        output,
        primary_output,
        output_geometry,
        request,
        false,
    )
}

pub(crate) fn build_with_privacy(
    renderer: &mut GlesRenderer,
    output: &Output,
    primary_output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    mut request: RenderRequest<'_>,
    block_screencast: bool,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    let scale = output.current_scale().fractional_scale();
    let locked = request.desktop.session_lock.active();
    request.resources.ui_text.set_display_scale(scale);
    // Normal presentation is the common path; do not scan capture rules
    // unless a capture requests privacy filtering.
    let window_rules = block_screencast && request.desktop.window_rules.has_screencast_blocks();
    let layer_rules = block_screencast
        && !window_rules
        && request
            .desktop
            .layer_rules
            .iter()
            .any(|rule| rule.block_screencast);
    let privacy = CapturePrivacy::new(block_screencast, window_rules, layer_rules);
    let elements = build_logical(
        renderer,
        output,
        primary_output,
        output_geometry,
        &mut request,
        privacy,
    )?;
    Ok(elements
        .into_iter()
        .map(|element| crate::render::display_scale::element(element, scale, locked))
        .collect())
}
