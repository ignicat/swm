use super::nodes::ease_in_out_cubic;
use super::*;

use crate::render::draw::DrawContext;

#[derive(Clone, Copy)]
pub(super) struct PreviewDesktop<'a> {
    pub cameras: &'a crate::presentation::camera::OutputCameras,
    pub nodes: &'a crate::nodes::NodesState,
    pub fullscreen: &'a crate::wayland::fullscreen::FullscreenManager,
    pub decorations: &'a swm_config::Decorations,
}

impl<'a> PreviewDesktop<'a> {
    pub(super) fn from_request(request: &RenderRequest<'a>) -> Self {
        Self {
            cameras: request.desktop.cameras,
            nodes: request.desktop.nodes,
            fullscreen: request.desktop.fullscreen,
            decorations: request.visuals.decorations,
        }
    }
}

pub(super) struct PreviewRenderers<'a> {
    pub node: &'a mut crate::render::node::NodeRenderer,
    pub decoration: &'a mut crate::render::window_decoration::WindowDecorationRenderer,
    pub text: &'a mut crate::render::text::UiTextRenderer,
    pub previews: &'a mut crate::render::overlays::preview::OverlayPreviewCache,
}

pub(super) struct FocusCycleRenderers<'a> {
    pub decoration: &'a mut crate::render::window_decoration::WindowDecorationRenderer,
    pub node: &'a mut crate::render::node::NodeRenderer,
    pub text: &'a mut crate::render::text::UiTextRenderer,
}

pub(super) struct PreviewScene<'a> {
    pub renderer: &'a mut GlesRenderer,
    pub output: &'a Output,
    pub output_geometry: Rectangle<i32, Logical>,
    pub desktop: PreviewDesktop<'a>,
    pub draw: PreviewRenderers<'a>,
    pub now: std::time::Duration,
}

pub(super) fn preview_content_radius(overlay_radius: f32) -> f32 {
    overlay_radius.max(0.0)
}

fn push_preview_texture(
    elements: &mut Vec<SceneElement>,
    renderer: &mut GlesRenderer,
    window_decoration_renderer: &mut crate::render::window_decoration::WindowDecorationRenderer,
    preview: smithay::backend::renderer::element::texture::TextureRenderElement<
        smithay::backend::renderer::gles::GlesTexture,
    >,
    texture: smithay::backend::renderer::gles::GlesTexture,
    destination: Rectangle<i32, Physical>,
    overlay_radius: f32,
) {
    let radius = preview_content_radius(overlay_radius);
    if radius > 0.0 && window_decoration_renderer.available(renderer) {
        let preview = window_decoration_renderer
            .texture_element(renderer, preview, texture, destination, radius)
            .expect("rounded resources were checked above");
        elements.push(SceneElement::RoundedTexture(preview));
    } else {
        elements.push(SceneElement::Closing(preview));
    }
}

struct LivePreviewStyle<'a> {
    alpha: f32,
    overlay_radius: f32,
    decorations: &'a swm_config::Decorations,
    chrome_visible: bool,
}

/// Render a mapped preview directly from the client's surface tree, avoiding
/// two full-window offscreen passes and preserving Smithay's real surface damage.
fn push_live_preview(
    elements: &mut Vec<SceneElement>,
    renderer: &mut GlesRenderer,
    window: &smithay::desktop::Window,
    destination: Rectangle<i32, Physical>,
    style: LivePreviewStyle<'_>,
    window_decoration_renderer: &mut crate::render::window_decoration::WindowDecorationRenderer,
) -> Result<bool, Box<dyn Error>> {
    let LivePreviewStyle {
        alpha,
        overlay_radius,
        decorations,
        chrome_visible,
    } = style;
    let Some(surface) = window.wl_surface() else {
        return Ok(false);
    };
    let geometry = window.geometry();
    if geometry.size.w <= 0 || geometry.size.h <= 0 {
        return Ok(false);
    }

    let native_client = Rectangle::<i32, Physical>::from_size(geometry.size.to_physical(1));
    let chrome = crate::window_chrome::WindowChrome::for_window(window, decorations);
    let native_outer = if chrome_visible {
        chrome
            .outer_rect(Rectangle::<i32, Logical>::from_size(geometry.size))
            .to_physical(1)
    } else {
        native_client
    };
    let content = crate::animation::map_rect(native_client, native_outer, destination);
    if content.size.w <= 0 || content.size.h <= 0 {
        return Ok(false);
    }

    let location = smithay::utils::Point::from((-geometry.loc.x, -geometry.loc.y)).to_physical(1);
    let surface_elements: Vec<
        smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement<GlesRenderer>,
    > = smithay::backend::renderer::element::surface::render_elements_from_surface_tree(
        renderer,
        surface.as_ref(),
        location,
        1.0,
        alpha,
        Kind::Unspecified,
    );
    if surface_elements.is_empty() {
        return Ok(false);
    }

    // The preview is framed by overlay chrome, so its mask must use the
    // overlay's radius in output pixels. Scaling the window's native radius
    // down with the thumbnail made the texture visibly squarer than its card.
    let content_radius = preview_content_radius(overlay_radius);
    let rounded = content_radius > 0.0 && window_decoration_renderer.available(renderer);

    for surface_element in surface_elements {
        let native_geometry = surface_element.geometry(Scale::from(1.0));
        let target = crate::animation::map_rect(native_geometry, native_client, content);
        if rounded {
            let radii = crate::render::window_decoration::CornerRadii::all(content_radius);
            let rounded = window_decoration_renderer
                .surface_element_with_radii(renderer, surface_element, target, content, radii)
                .expect("rounded resources were checked above");
            if let Some(cropped) = CropRenderElement::from_element(rounded, 1.0, content) {
                elements.push(SceneElement::RoundedCropped(cropped));
            }
        } else {
            let rescaled = crate::render::rescale::RescaledElement::new(surface_element, target);
            if let Some(cropped) = CropRenderElement::from_element(rescaled, 1.0, content) {
                elements.push(SceneElement::Cropped(cropped));
            }
        }
    }

    Ok(true)
}

pub(super) struct FocusCycleRenderContext<'a> {
    pub state: &'a crate::shell::focus_cycle::FocusCycleState,
    pub nodes: &'a crate::nodes::NodesState,
    pub overlay_config: &'a swm_config::Overlays,
    pub decorations: &'a swm_config::Decorations,
    pub fullscreen: &'a crate::wayland::fullscreen::FullscreenManager,
    pub maximize: &'a crate::presentation::maximize::FieldMaximizeManager,
    pub now: std::time::Duration,
}

/// Top-left `NODE` pill for collapsed windows in Alt+Tab.
fn push_node_badge(
    ctx: &mut DrawContext<'_>,
    elements: &mut Vec<SceneElement>,
    body: Rectangle<i32, Physical>,
    overlay_visuals: crate::render::overlays::shell::OverlayVisuals,
    alpha: f32,
) -> Result<(), Box<dyn Error>> {
    const LABEL: &str = "NODE";
    let Some(size) = ctx
        .text
        .measure(ctx.renderer, LABEL, overlay_visuals.border.bytes())?
    else {
        return Ok(());
    };
    let badge = Rectangle::<i32, Physical>::new(
        (body.loc.x + 8, body.loc.y + 8).into(),
        (size.w + 14, size.h + 8).into(),
    );
    if let Some(text) = ctx.text.element(
        ctx.renderer,
        (badge.loc.x + 7, badge.loc.y + 4).into(),
        LABEL,
        overlay_visuals.border.bytes(),
        alpha,
    )? {
        elements.push(SceneElement::UiText(text.element));
    }
    elements.push(SceneElement::NodeLabel(ctx.label_card(
        badge,
        overlay_visuals,
        overlay_visuals.fill,
        0.94 * alpha,
    )?));
    Ok(())
}

pub(super) fn focus_cycle_elements(
    renderer: &mut GlesRenderer,
    output_geometry: Rectangle<i32, Logical>,
    context: FocusCycleRenderContext<'_>,
    overlay_previews: &mut crate::render::overlays::preview::OverlayPreviewCache,
    renderers: FocusCycleRenderers<'_>,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    let FocusCycleRenderers {
        decoration: window_decoration_renderer,
        node: node_renderer,
        text: ui_text,
    } = renderers;
    let Some(session) = context.state.session() else {
        return Ok(Vec::new());
    };
    let open = session.open_progress(context.now);
    let close = session.close_progress(context.now);
    let alpha = (open * (1.0 - close)).clamp(0.0, 1.0);
    if alpha <= 0.001 {
        return Ok(Vec::new());
    }

    let screen = output_geometry.size.to_physical(1);
    let overlay_visuals = crate::render::overlays::shell::resolve_visuals(context.overlay_config);
    let rail_step = (screen.w as f32 * 0.28).clamp(260.0, 440.0) + 9.0;
    let center_y = screen.h as f32 * 0.5;
    let mut cards = session
        .visible_slots(crate::shell::focus_cycle::VISIBLE_RADIUS)
        .into_iter()
        .filter_map(|(_, id)| {
            let record = context.nodes.record(id)?;
            let index = session
                .candidates
                .iter()
                .position(|candidate| *candidate == id)?;
            let offset = session.visual_offset(index, context.now);
            let distance = offset.abs().min(2.0);
            let base_h = (screen.h as f32 * 0.46).clamp(240.0, 480.0);
            let scale = if distance <= 1.0 {
                1.0 + (0.82 - 1.0) * distance
            } else {
                0.82 + (0.64 - 0.82) * (distance - 1.0)
            };
            let preview_h = (base_h * scale).round().max(1.0) as i32;
            let aspect = (record.geometry.size.w.max(1) as f32
                / record.geometry.size.h.max(1) as f32)
                .clamp(0.7, 2.0);
            let preview_w = (preview_h as f32 * aspect).round().max(1.0) as i32;
            let cx = screen.w as f32 * 0.5 + offset * rail_step;
            let cy = center_y + distance * 22.0 + (1.0 - open) * 22.0 + close * 18.0;
            let pose_scale = 0.88 + 0.12 * open - 0.08 * close;
            let card_w = (preview_w as f32 * pose_scale).round().max(1.0) as i32;
            let card_h = (preview_h as f32 * pose_scale).round().max(1.0) as i32;
            Some((
                distance,
                id,
                Rectangle::<i32, Physical>::new(
                    (
                        (cx - card_w as f32 * 0.5).round() as i32,
                        (cy - card_h as f32 * 0.5).round() as i32,
                    )
                        .into(),
                    (card_w, card_h).into(),
                ),
            ))
        })
        .collect::<Vec<_>>();
    cards.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut elements = Vec::new();
    overlay_previews.retain(session.candidates.iter().copied());
    for (distance, id, card) in cards {
        let Some(record) = context.nodes.record(id) else {
            continue;
        };
        let selected = distance < 0.45;
        let distance_step = distance.round().clamp(0.0, 2.0) as i32;
        let pad = if distance_step >= 2 { 4 } else { 6 };
        let body_bounds = Rectangle::<i32, Physical>::new(
            (card.loc.x + pad, card.loc.y + pad).into(),
            (
                (card.size.w - pad * 2).max(1),
                (card.size.h - pad * 2).max(1),
            )
                .into(),
        );
        let chrome_visible = preview_chrome_visible(&record.window, context.fullscreen);
        let fallback_size = preview_outer_size(
            &record.window,
            record.geometry.size,
            context.decorations,
            chrome_visible,
        );
        let (source_width, source_height) = overlay_previews
            .source_dimensions(id)
            .unwrap_or((fallback_size.w, fallback_size.h));
        let body = aspect_fit_rect(body_bounds, source_width, source_height);
        let card = preview_card_rect(body, overlay_visuals.border_px);

        // Top-right monitor badge, over the preview.
        let monitor = truncate_chars(&record.output, 10);
        if let Some(size) = ui_text.measure(renderer, &monitor, overlay_visuals.text.bytes())? {
            let badge = Rectangle::<i32, Physical>::new(
                (body.loc.x + body.size.w - size.w - 22, body.loc.y + 8).into(),
                (size.w + 14, size.h + 8).into(),
            );
            if let Some(text) = ui_text.element(
                renderer,
                (badge.loc.x + 7, badge.loc.y + 4).into(),
                &monitor,
                overlay_visuals.text.bytes(),
                alpha,
            )? {
                elements.push(SceneElement::UiText(text.element));
            }
            elements.push(SceneElement::NodeLabel(
                crate::render::overlays::shell::label_card_element(
                    renderer,
                    node_renderer,
                    badge,
                    overlay_visuals,
                    overlay_visuals.fill.mix(overlay_visuals.border, 0.12),
                    if selected { 0.95 * alpha } else { 0.78 * alpha },
                )?,
            ));
        }

        if record.collapsed {
            push_node_badge(
                &mut crate::render::draw::DrawContext {
                    renderer,
                    node: node_renderer,
                    text: ui_text,
                },
                &mut elements,
                body,
                overlay_visuals,
                alpha,
            )?;
        }

        // Overlay the title and app icon at the bottom of the thumbnail
        // instead of adding a footer.
        let title = truncate_chars(
            &record.title,
            match distance_step {
                0 => 42,
                1 => 30,
                _ => 20,
            },
        );
        let text_size = ui_text
            .measure(renderer, &title, overlay_visuals.text.bytes())?
            .unwrap_or_default();
        let band_margin = 6;
        let band_h = (text_size.h + 10)
            .max(24)
            .min((body.size.h - band_margin * 2).max(1));
        let band = Rectangle::<i32, Physical>::new(
            (
                body.loc.x + band_margin,
                body.loc.y + body.size.h - band_h - band_margin,
            )
                .into(),
            ((body.size.w - band_margin * 2).max(1), band_h).into(),
        );
        let mut text_x = band.loc.x + 10;
        if distance_step < 2 {
            let icon_size = (band_h - 6).clamp(14, 30);
            let icon_rect = Rectangle::<i32, Physical>::new(
                (band.loc.x + 5, band.loc.y + (band_h - icon_size) / 2).into(),
                (icon_size, icon_size).into(),
            );
            if let Some(app_id) = record.app_id.as_deref()
                && let Some(icon) =
                    node_renderer.app_icon_element(renderer, app_id, icon_rect, alpha)
            {
                elements.push(SceneElement::NodeTexture(icon));
                text_x = icon_rect.loc.x + icon_rect.size.w + 8;
            }
        }
        if let Some(text) = ui_text.element(
            renderer,
            (text_x, band.loc.y + (band_h - text_size.h) / 2).into(),
            &title,
            overlay_visuals.text.bytes(),
            alpha,
        )? {
            elements.push(SceneElement::UiText(text.element));
        }
        elements.push(SceneElement::NodeLabel(
            crate::render::overlays::shell::label_card_element(
                renderer,
                node_renderer,
                band,
                overlay_visuals,
                if selected {
                    overlay_visuals.fill.mix(overlay_visuals.border, 0.18)
                } else {
                    overlay_visuals.fill
                },
                if selected { 0.96 * alpha } else { 0.88 * alpha },
            )?,
        ));

        let maximized = record
            .window
            .wl_surface()
            .is_some_and(|surface| context.maximize.contains(surface.as_ref()));
        let direct = !record.collapsed
            && push_live_preview(
                &mut elements,
                renderer,
                &record.window,
                body,
                LivePreviewStyle {
                    alpha,
                    overlay_radius: overlay_visuals.radius,
                    decorations: context.decorations,
                    chrome_visible,
                },
                window_decoration_renderer,
            )?;
        if !direct {
            match overlay_previews.element_with_texture(
                renderer,
                crate::render::overlays::preview::OverlayPreviewRequest {
                    id,
                    window: &record.window,
                    destination: body,
                    alpha,
                    allow_refresh: !record.collapsed,
                    live: false,
                    decorations: context.decorations,
                    chrome_visible,
                    maximized,
                },
            ) {
                Ok((preview, texture)) => push_preview_texture(
                    &mut elements,
                    renderer,
                    window_decoration_renderer,
                    preview,
                    texture,
                    body,
                    overlay_visuals.radius,
                ),
                Err(_) => {
                    if let Some(app_id) = record.app_id.as_deref()
                        && let Some(icon) =
                            node_renderer.app_icon_element(renderer, app_id, body, alpha)
                    {
                        elements.push(SceneElement::NodeTexture(icon));
                    }
                }
            }
        }
        elements.push(SceneElement::NodeLabel(
            crate::render::overlays::shell::card_element(
                renderer,
                node_renderer,
                card,
                overlay_visuals,
                if selected {
                    overlay_visuals.fill.mix(overlay_visuals.border, 0.12)
                } else {
                    overlay_visuals.fill
                },
                if selected { 0.99 * alpha } else { 0.82 * alpha },
            )?,
        ));
    }

    let hints = "Tab  next     Shift+Tab  previous     Esc  cancel";
    if let Some(size) = ui_text.measure(renderer, hints, overlay_visuals.subtext.bytes())?
        && let Some(text) = ui_text.element(
            renderer,
            ((screen.w - size.w) / 2, screen.h - size.h - 28).into(),
            hints,
            overlay_visuals.subtext.bytes(),
            alpha,
        )?
    {
        elements.push(SceneElement::UiText(text.element));
    }
    let backdrop_color = smithay::backend::renderer::Color32F::new(0.02, 0.03, 0.05, 0.55 * alpha);
    elements.push(SceneElement::Border(crate::render::solid_color_element(
        node_renderer.active_slot_id(crate::render::node::NodeSlot::FocusCycleBackdrop),
        Rectangle::from_size(screen),
        backdrop_color,
    )));
    Ok(elements)
}

pub(super) fn hover_preview_elements(
    scene: PreviewScene<'_>,
    overlay_config: &swm_config::Overlays,
) -> Result<Vec<SceneElement>, Box<dyn Error>> {
    let PreviewScene {
        renderer,
        output,
        output_geometry,
        now,
        desktop,
        draw,
    } = scene;
    let PreviewDesktop {
        cameras,
        nodes,
        fullscreen,
        decorations,
    } = desktop;
    let PreviewRenderers {
        node: node_renderer,
        decoration: window_decoration_renderer,
        text: ui_text,
        previews: overlay_previews,
        ..
    } = draw;
    let Some((id, raw_mix)) = nodes.preview_hover_mix(&output.name(), now) else {
        return Ok(Vec::new());
    };
    let Some(record) = nodes
        .record(id)
        .filter(|record| record.collapsed && record.attached && record.output == output.name())
    else {
        return Ok(Vec::new());
    };
    let Some(node) = nodes.field().node(id) else {
        return Ok(Vec::new());
    };
    let Some(camera) = cameras.get(&output.name()) else {
        return Ok(Vec::new());
    };

    let preview_mix = ease_in_out_cubic(raw_mix.clamp(0.0, 1.0));
    let alpha = (preview_mix * preview_mix).clamp(0.0, 1.0);
    if alpha <= 0.01 {
        return Ok(Vec::new());
    }

    let screen = output_geometry.size.to_physical(1);
    let preview_size_base = ((screen.w.min(screen.h) as f32) * 0.30)
        .round()
        .clamp(220.0, 360.0) as i32;
    let chrome_visible = preview_chrome_visible(&record.window, fullscreen);
    let source = preview_outer_size(
        &record.window,
        record.geometry.size,
        decorations,
        chrome_visible,
    )
    .to_physical(1);
    let source_side = source.w.max(source.h).max(1);
    let base_side = (source_side + 24).clamp(120, preview_size_base);
    let preview_size = ((base_side as f32) * (0.94 + 0.06 * preview_mix))
        .round()
        .max(120.0) as i32;

    let landmark = nodes.landmark_position(id, node.pos, now);
    let center =
        crate::nodes::screen_from_world(landmark, camera, output_geometry) - output_geometry.loc;
    let card = Rectangle::<i32, Physical>::new(
        (
            (center.x - preview_size / 2).clamp(10, (screen.w - preview_size - 10).max(10)),
            (center.y - preview_size / 2).clamp(10, (screen.h - preview_size - 10).max(10)),
        )
            .into(),
        (preview_size, preview_size).into(),
    );
    let pad = ((preview_size as f32) * 0.045).round() as i32;
    let label_h = 30.min((preview_size / 5).max(22));
    let body_bounds = Rectangle::<i32, Physical>::new(
        (card.loc.x + pad, card.loc.y + pad).into(),
        (
            (card.size.w - pad * 2).max(1),
            (card.size.h - pad * 2 - label_h).max(1),
        )
            .into(),
    );
    let body = aspect_fit_rect(body_bounds, source.w, source.h);
    let visuals = crate::render::overlays::shell::resolve_visuals(overlay_config);
    let mut elements = Vec::new();

    let title = truncate_chars(&record.title, 24);
    if !title.is_empty()
        && let Some(text_size) = ui_text.measure(renderer, &title, visuals.text.bytes())?
        && let Some(text) = ui_text.element(
            renderer,
            (
                card.loc.x + (card.size.w - text_size.w).max(0) / 2,
                card.loc.y + card.size.h - pad - label_h + (label_h - text_size.h).max(0) / 2,
            )
                .into(),
            &title,
            visuals.text.bytes(),
            0.94 * alpha,
        )?
    {
        elements.push(SceneElement::UiText(text.element));
    }

    match overlay_previews.element_with_texture(
        renderer,
        crate::render::overlays::preview::OverlayPreviewRequest {
            id,
            window: &record.window,
            destination: body,
            alpha,
            allow_refresh: false,
            live: false,
            decorations,
            chrome_visible,
            maximized: false,
        },
    ) {
        Ok((preview, texture)) => push_preview_texture(
            &mut elements,
            renderer,
            window_decoration_renderer,
            preview,
            texture,
            body,
            visuals.radius,
        ),
        Err(_) => elements.push(SceneElement::Border(crate::render::solid_color_element(
            node_renderer.active_slot_id(crate::render::node::NodeSlot::HoverPreviewFallback),
            body,
            smithay::backend::renderer::Color32F::new(0.02, 0.03, 0.05, 0.76 * alpha),
        ))),
    }

    elements.push(SceneElement::NodeLabel(
        crate::render::overlays::shell::card_element(
            renderer,
            node_renderer,
            card,
            visuals,
            visuals.fill,
            0.86 * alpha,
        )?,
    ));
    Ok(elements)
}

fn preview_chrome_visible(
    window: &smithay::desktop::Window,
    fullscreen: &crate::wayland::fullscreen::FullscreenManager,
) -> bool {
    !fullscreen.suppresses_window_chrome(window)
}

fn preview_outer_size(
    window: &smithay::desktop::Window,
    client: smithay::utils::Size<i32, Logical>,
    decorations: &swm_config::Decorations,
    chrome_visible: bool,
) -> smithay::utils::Size<i32, Logical> {
    if chrome_visible {
        crate::window_chrome::WindowChrome::for_window(window, decorations)
            .outer_rect(Rectangle::from_size(client))
            .size
    } else {
        client
    }
}

pub(super) fn aspect_fit_rect(
    bounds: Rectangle<i32, Physical>,
    source_width: i32,
    source_height: i32,
) -> Rectangle<i32, Physical> {
    let source_width = source_width.max(1) as f32;
    let source_height = source_height.max(1) as f32;
    let scale = (bounds.size.w as f32 / source_width).min(bounds.size.h as f32 / source_height);
    let width = (source_width * scale).round().max(1.0) as i32;
    let height = (source_height * scale).round().max(1.0) as i32;
    Rectangle::new(
        (
            bounds.loc.x + (bounds.size.w - width) / 2,
            bounds.loc.y + (bounds.size.h - height) / 2,
        )
            .into(),
        (width, height).into(),
    )
}

pub(super) fn preview_card_rect(
    body: Rectangle<i32, Physical>,
    border_px: f32,
) -> Rectangle<i32, Physical> {
    let outset = border_px.max(0.0).ceil() as i32;
    Rectangle::new(
        (body.loc.x - outset, body.loc.y - outset).into(),
        (
            body.size.w + outset.saturating_mul(2),
            body.size.h + outset.saturating_mul(2),
        )
            .into(),
    )
}

pub(super) fn truncate_chars(text: &str, max: usize) -> String {
    let mut chars = text.trim().chars();
    let prefix = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}
