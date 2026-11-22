use super::*;

pub(super) struct LayerSceneStyle<'a> {
    pub layer_rules: &'a [swm_config::LayerRule],
    pub block_screencast: bool,
}

pub(super) fn layer_surface_scene_elements(
    renderer: &mut GlesRenderer,
    output: &Output,
    layer: Layer,
    style: LayerSceneStyle<'_>,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    let LayerSceneStyle {
        layer_rules,
        block_screencast,
    } = style;
    let map = layer_map_for_output(output);
    let scale = Scale::from(1.0);
    let mut elements = Vec::new();

    for surface in map.layers_on(layer).rev() {
        // A layer-shell object remains in Smithay's map after a null-buffer
        // commit so it can be configured and mapped again.
        let has_buffer =
            with_renderer_surface_state(surface.wl_surface(), |state| state.buffer().is_some())
                .unwrap_or(false);
        if !has_buffer {
            continue;
        }
        let Some(geometry) = map.layer_geometry(surface) else {
            continue;
        };
        let rule = layer_rules
            .iter()
            .find(|rule| rule.matches(surface.namespace(), layer_shell_layer(surface.layer())));
        if block_screencast && rule.is_some_and(|rule| rule.block_screencast) {
            continue;
        }
        for (popup, popup_offset) in PopupManager::popups_for_surface(surface.wl_surface()) {
            let popup_origin = geometry.loc + popup_offset - popup.geometry().loc;
            let location = popup_origin.to_f64().to_physical(scale).to_i32_round();
            elements.extend(
                smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                    renderer,
                    popup.wl_surface(),
                    location,
                    scale,
                    1.0,
                    Kind::ScanoutCandidate,
                )
                .into_iter()
                .map(SceneElement::Layer),
            );
        }

        let location = geometry.loc.to_f64().to_physical(scale).to_i32_round();
        elements.extend(
            smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
                renderer,
                surface.wl_surface(),
                location,
                scale,
                1.0,
                Kind::ScanoutCandidate,
            )
            .into_iter()
            .map(SceneElement::Layer),
        );
    }

    Ok(elements)
}

fn layer_shell_layer(layer: Layer) -> swm_config::LayerShellLayer {
    match layer {
        Layer::Background => swm_config::LayerShellLayer::Background,
        Layer::Bottom => swm_config::LayerShellLayer::Bottom,
        Layer::Top => swm_config::LayerShellLayer::Top,
        Layer::Overlay => swm_config::LayerShellLayer::Overlay,
    }
}
