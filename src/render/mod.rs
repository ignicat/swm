mod app_icon;
pub mod background;
pub mod close;
pub mod display_scale;
pub(crate) mod draw;
pub mod fullscreen_texture;
pub mod ids;
pub mod node;
pub mod overlays;
pub mod pin;
pub mod rescale;
pub mod resize;
pub mod resources;
pub mod scene;
pub mod text;
pub mod window_decoration;
pub mod window_open;
pub mod window_shader;
pub mod window_texture;

use smithay::backend::renderer::Color32F;
use smithay::backend::renderer::element::RenderElementStates;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::desktop::{PopupManager, Space, Window};
use smithay::input::pointer::CursorIcon;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size};
use smithay::wayland::seat::WaylandFocus;
use swm_config::Decorations;

use crate::cursor::CursorManager;

/// Opaque black used only to clear pixels with no scene content.
///
/// `wallpaper.mode "none"` contributes no render element, so this is the
/// resulting empty desktop rather than a compositor-enforced wallpaper.
pub const CLEAR_COLOR: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);
/// Fail-closed backdrop used for every output while ext-session-lock-v1 owns
/// the session, including outputs whose locker surface is not ready yet.
pub const SESSION_LOCK_COLOR: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);

pub fn output_physical_size(output: &Output) -> Size<i32, Physical> {
    output.current_transform().transform_size(
        output
            .current_mode()
            .expect("mapped output has a mode")
            .size,
    )
}

pub fn window_surface_location(
    mapped_geometry_location: Point<i32, Logical>,
    local_geometry: Rectangle<i32, Logical>,
) -> Point<i32, Physical> {
    (mapped_geometry_location - local_geometry.loc).to_physical(1)
}

/// Build a window's normal surface tree separately from its popups.
///
/// Smithay's `Window::render_elements` combines both, which is convenient
/// until the compositor needs to crop client-side shadows to the window
/// geometry: popups must remain free to extend beyond that geometry.
pub fn window_surface_elements(
    renderer: &mut GlesRenderer,
    window: &Window,
    surface_location: Point<i32, Physical>,
    alpha: f32,
) -> (
    Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
    Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
) {
    let Some(surface) = window.wl_surface() else {
        return (Vec::new(), Vec::new());
    };
    let scale = Scale::from(1.0);
    let popup_elements = window
        .toplevel()
        .map(|toplevel| {
            PopupManager::popups_for_surface(toplevel.wl_surface())
                .flat_map(|(popup, popup_offset)| {
                    let offset = (window.geometry().loc + popup_offset - popup.geometry().loc)
                        .to_physical_precise_round(scale);
                    render_elements_from_surface_tree(
                        renderer,
                        popup.wl_surface(),
                        surface_location + offset,
                        scale,
                        alpha,
                        Kind::ScanoutCandidate,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let surface_elements = render_elements_from_surface_tree(
        renderer,
        surface.as_ref(),
        surface_location,
        scale,
        alpha,
        Kind::ScanoutCandidate,
    );

    (popup_elements, surface_elements)
}

/// Whether rendering an output submitted a frame that will produce a
/// presentation event, or found no new damage to submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderStatus {
    Submitted,
    Skipped,
}

#[derive(Debug)]
pub struct RenderOutcome {
    status: RenderStatus,
    element_states: Option<RenderElementStates>,
    needs_redraw: bool,
    pub fifo_barriers: Vec<crate::wayland::fifo::FrameBarrier>,
}

impl RenderOutcome {
    pub fn new(status: RenderStatus, element_states: Option<RenderElementStates>) -> Self {
        Self {
            status,
            element_states,
            needs_redraw: false,
            fifo_barriers: Vec::new(),
        }
    }

    /// Owe another scene even if no input, client damage, or animation remains.
    pub fn requiring_redraw(mut self) -> Self {
        self.needs_redraw = true;
        self
    }

    pub fn with_fifo(mut self, barriers: Vec<crate::wayland::fifo::FrameBarrier>) -> Self {
        self.fifo_barriers = barriers;
        self
    }

    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    pub fn status(&self) -> RenderStatus {
        self.status
    }

    pub fn element_states(&self) -> Option<&RenderElementStates> {
        self.element_states.as_ref()
    }
}

#[derive(Debug)]
pub struct FrameSubmission {
    pub fifo_barriers: Vec<crate::wayland::fifo::FrameBarrier>,
    pub target_presentation_time: std::time::Duration,
    pub presentation_feedback: smithay::desktop::utils::OutputPresentationFeedback,
    pub session_lock_generation: Option<u64>,
    pub variable_refresh: bool,
}

/// Per-frame scheduling and output policy.
pub struct FrameContext {
    pub target_presentation_time: std::time::Duration,
    /// True only when the output has a committed, fully settled fullscreen
    /// window and no compositor or layer-shell overlay is visible.
    pub vrr_auto_eligible: bool,
    /// Discard incremental buffer-age history for this frame. The DRM backend
    /// uses this while scene geometry is animated so every scan-out buffer is
    /// repainted from the same complete scene.
    pub force_full_repaint: bool,
    pub clear: Color32F,
}

/// Desktop state and presentation policy used to place live surfaces.
pub struct DesktopContext<'a> {
    pub fifo: &'a crate::wayland::fifo::State,
    pub session_lock: &'a crate::wayland::session_lock::State,
    pub space: &'a Space<Window>,
    pub focused: Option<&'a WlSurface>,
    pub cameras: &'a crate::presentation::camera::OutputCameras,
    pub window_animations: &'a crate::animation::WindowAnimations,
    pub fullscreen: &'a crate::wayland::fullscreen::FullscreenManager,
    pub maximize: &'a crate::presentation::maximize::FieldMaximizeManager,
    pub nodes: &'a crate::nodes::NodesState,
    pub window_rules: &'a crate::window::rules::WindowRulesState,
    pub screencast: &'a crate::capture::screencast::ScreencastState,
    pub layer_rules: &'a [swm_config::LayerRule],
    pub node_grab_active: bool,
}

/// Cursor presentation inputs. Pointer constraint policy is deliberately not
/// part of rendering and remains owned by the session input subsystem.
pub struct CursorContext<'a> {
    pub cursor: &'a CursorManager,
    pub dnd_icon: Option<&'a crate::wayland::dnd::DndIcon>,
    pub cursor_position: (f64, f64),
    pub show_cursor: bool,
    pub cursor_override: Option<CursorIcon>,
}

/// Shell-owned overlays and replacement scenes.
pub struct OverlayContext<'a> {
    pub capture_overlay: crate::capture::CaptureOverlay<'a>,
    pub focus_cycle: &'a crate::shell::focus_cycle::FocusCycleState,
    pub overlays: &'a crate::shell::overlay::OverlayManager,
    pub overlay_config: &'a swm_config::Overlays,
}

/// Immutable visual configuration for one frame.
pub struct VisualContext<'a> {
    pub decorations: &'a Decorations,
    pub pins: &'a swm_config::Pins,
    pub background: &'a swm_config::Background,
    pub background_base: Option<&'a std::path::Path>,
}

/// Complete input to one scene build, grouped by ownership boundary.
pub struct RenderRequest<'a> {
    pub frame: FrameContext,
    pub desktop: DesktopContext<'a>,
    pub cursor: CursorContext<'a>,
    pub overlays: OverlayContext<'a>,
    pub visuals: VisualContext<'a>,
    pub resources: resources::RenderResources<'a>,
}

/// A presentation backend for one already-described scene.
pub trait Renderable {
    fn render(
        &mut self,
        output: &Output,
        request: RenderRequest<'_>,
    ) -> Result<RenderOutcome, Box<dyn std::error::Error>>;
}

pub fn decoration_color(color: swm_config::BorderColor) -> Color32F {
    Color32F::new(color.r, color.g, color.b, 1.0)
}

/// Whichever color a window's border should be drawn in, given whether its
/// surface is the focused one.
pub fn window_border_color(decorations: &Decorations, is_focused: bool) -> Color32F {
    decoration_color(if is_focused {
        decorations.border_color_focused
    } else {
        decorations.border_color_unfocused
    })
}

pub fn solid_color_element(
    id: Id,
    geometry: Rectangle<i32, Physical>,
    color: Color32F,
) -> SolidColorRenderElement {
    SolidColorRenderElement::new(
        id,
        geometry,
        window_decoration::solid_color_commit(color),
        color,
        Kind::Unspecified,
    )
}

/// Four thin solid-color strips framing `bbox` from the outside so borders
/// never overlap window content.
pub fn border_strips(
    ids: [Id; 4],
    bbox: Rectangle<i32, Physical>,
    width: i32,
    color: Color32F,
) -> [SolidColorRenderElement; 4] {
    let make = |id: Id, rect: Rectangle<i32, Physical>| {
        SolidColorRenderElement::new(
            id,
            rect,
            window_decoration::solid_color_commit(color),
            color,
            Kind::Unspecified,
        )
    };
    let top = Rectangle::new(
        (bbox.loc.x - width, bbox.loc.y - width).into(),
        (bbox.size.w + width * 2, width).into(),
    );
    let bottom = Rectangle::new(
        (bbox.loc.x - width, bbox.loc.y + bbox.size.h).into(),
        (bbox.size.w + width * 2, width).into(),
    );
    let left = Rectangle::new(
        (bbox.loc.x - width, bbox.loc.y).into(),
        (width, bbox.size.h).into(),
    );
    let right = Rectangle::new(
        (bbox.loc.x + bbox.size.w, bbox.loc.y).into(),
        (width, bbox.size.h).into(),
    );
    [
        make(ids[0].clone(), top),
        make(ids[1].clone(), bottom),
        make(ids[2].clone(), left),
        make(ids[3].clone(), right),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_origin_accounts_for_client_side_geometry_offset() {
        let mapped = Point::<i32, Logical>::from((500, 300));
        let local_geometry = Rectangle::<i32, Logical>::new((12, 16).into(), (800, 600).into());

        assert_eq!(
            window_surface_location(mapped, local_geometry),
            Point::from((488, 284))
        );
    }
}
