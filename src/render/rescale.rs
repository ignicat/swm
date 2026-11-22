use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{
    Buffer, Physical, Point, Rectangle, Scale, Transform, user_data::UserDataMap,
};

/// Wraps a `WaylandSurfaceRenderElement` so it draws into an arbitrary
/// destination rect instead of its native buffer size.
///
/// Passing a zoom scale to `Window::render_elements` does not shrink content on
/// screen because `WaylandSurfaceRenderElement::geometry` recomputes size from
/// the query-time scale, and `draw_render_elements` always queries geometry at
/// `1.0`. Hardcoding `geometry` to `dst` causes the renderer driver to compute
/// scaled draw rects before forwarding them to the inner element.
pub struct RescaledElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    dst: Rectangle<i32, Physical>,
}

impl RescaledElement {
    pub fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        dst: Rectangle<i32, Physical>,
    ) -> Self {
        Self { inner, dst }
    }
}

impl Element for RescaledElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.dst
    }

    fn location(&self, _scale: Scale<f64>) -> Point<i32, Physical> {
        self.dst.loc
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        // A hardware plane would bypass camera zoom, which shows up as the
        // surface flickering between scan-out and composition.
        Kind::Unspecified
    }

    // Leave `opaque_regions` empty rather than forwarding unscaled regions
    // from `inner`, which would over-report coverage and falsely cull visible
    // elements when zoomed out. `damage_since` already derives from `geometry()`.
}

impl RenderElement<GlesRenderer> for RescaledElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // Caller-computed `src`, `dst`, and `damage` already reflect `self.geometry()`.
        self.inner
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
