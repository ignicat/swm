use super::*;

pub(super) fn capture_overlay_elements(
    renderer: &mut GlesRenderer,
    output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    overlay: crate::capture::CaptureOverlay<'_>,
    node_renderer: &mut crate::render::node::NodeRenderer,
    overlay_config: &swm_config::Overlays,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    let visuals = crate::render::overlays::shell::resolve_visuals(overlay_config);
    match overlay {
        crate::capture::CaptureOverlay::None => Ok(Vec::new()),
        crate::capture::CaptureOverlay::Highlight(region) => {
            let mut elements = capture_highlight_elements(
                renderer,
                node_renderer,
                &output.name(),
                output_geometry,
                region,
                visuals,
            )?;
            // Built back-to-front; the scene consumes front-to-back.
            elements.reverse();
            Ok(elements)
        }
        crate::capture::CaptureOverlay::SourceMenu {
            output_name,
            selected,
            hovered,
            monitor_available,
            window_available,
        } if output.name() == output_name => {
            let mut elements = Vec::new();
            elements.extend(
                crate::render::overlays::source_chooser::menu_elements(
                    renderer,
                    node_renderer,
                    output_geometry,
                    crate::render::overlays::source_chooser::SourceChooserMenuState {
                        selected,
                        hovered,
                        monitor_available,
                        window_available,
                    },
                    visuals,
                )?
                .into_iter()
                .rev()
                .map(SceneElement::SourceChooser),
            );
            elements.push(source_chooser_backdrop(output_geometry, node_renderer));
            Ok(elements)
        }
        crate::capture::CaptureOverlay::SourceMenu { .. } => Ok(vec![source_chooser_backdrop(
            output_geometry,
            node_renderer,
        )]),
    }
}

fn source_chooser_backdrop(
    output: Rectangle<i32, Logical>,
    node_renderer: &mut crate::render::node::NodeRenderer,
) -> SceneElement {
    SceneElement::Border(crate::render::solid_color_element(
        node_renderer.active_slot_id(crate::render::node::NodeSlot::SourceChooserBackdrop),
        Rectangle::from_size(output.size.to_physical(1)),
        crate::render::overlays::shell::backdrop_dim(0.45),
    ))
}

const HIGHLIGHT_BORDER_PX: f32 = 2.0;

struct HighlightLayout {
    dim: Vec<Rectangle<i32, Physical>>,
    outline: Rectangle<i32, Physical>,
}

fn highlight_layout(
    output: Rectangle<i32, Logical>,
    selection: Rectangle<i32, Logical>,
) -> Option<HighlightLayout> {
    let output_local = Rectangle::<i32, Physical>::from_size(output.size.to_physical(1));
    let clipped = output.intersection(selection).map(|intersection| {
        Rectangle::<i32, Physical>::new(
            (intersection.loc - output.loc).to_physical(1),
            intersection.size.to_physical(1),
        )
    })?;

    let right = clipped.loc.x + clipped.size.w;
    let bottom = clipped.loc.y + clipped.size.h;
    let dim = [
        Rectangle::new(
            (0, 0).into(),
            (output_local.size.w, clipped.loc.y.max(0)).into(),
        ),
        Rectangle::new(
            (0, bottom).into(),
            (output_local.size.w, (output_local.size.h - bottom).max(0)).into(),
        ),
        Rectangle::new(
            (0, clipped.loc.y).into(),
            (clipped.loc.x.max(0), clipped.size.h).into(),
        ),
        Rectangle::new(
            (right, clipped.loc.y).into(),
            ((output_local.size.w - right).max(0), clipped.size.h).into(),
        ),
    ]
    .into_iter()
    .filter(|rect| rect.size.w > 0 && rect.size.h > 0)
    .collect();

    let outline = Rectangle::<i32, Physical>::new(
        (selection.loc - output.loc).to_physical(1),
        selection.size.to_physical(1),
    );
    Some(HighlightLayout { dim, outline })
}

fn capture_highlight_elements(
    renderer: &mut GlesRenderer,
    node_renderer: &mut crate::render::node::NodeRenderer,
    output_name: &str,
    output: Rectangle<i32, Logical>,
    selection: Rectangle<i32, Logical>,
    visuals: crate::render::overlays::shell::OverlayVisuals,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    use crate::render::node::NodeSlot;

    let dim_color = crate::render::overlays::shell::backdrop_dim(0.62);
    let accent = (
        visuals.border.r,
        visuals.border.g,
        visuals.border.b,
        visuals.border.a,
    );

    let Some(layout) = highlight_layout(output, selection) else {
        let id = node_renderer.slot_id(output_name, NodeSlot::PickerBackdrop);
        return Ok(vec![SceneElement::Border(SolidColorRenderElement::new(
            id,
            Rectangle::<i32, Physical>::from_size(output.size.to_physical(1)),
            CommitCounter::default(),
            dim_color,
            Kind::Unspecified,
        ))]);
    };

    let mut elements = Vec::with_capacity(5);
    for (index, rect) in layout.dim.iter().enumerate() {
        let id = node_renderer.slot_id(output_name, NodeSlot::PickerDim(index as u8));
        elements.push(SceneElement::Border(SolidColorRenderElement::new(
            id,
            *rect,
            CommitCounter::default(),
            dim_color,
            Kind::Unspecified,
        )));
    }

    elements.push(SceneElement::DashedOutline(
        node_renderer.dashed_outline_element(
            renderer,
            output_name,
            NodeSlot::PickerOutline,
            layout.outline,
            crate::render::node::DashedOutlineStyle {
                rgba: accent,
                thickness: HIGHLIGHT_BORDER_PX,
                dash: (1.0, 1.0),
            },
        )?,
    ));

    Ok(elements)
}
