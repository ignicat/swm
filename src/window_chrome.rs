use smithay::desktop::Window;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::utils::Rectangle;

/// The client's decoration contract, independent of temporary fullscreen
/// suppression of compositor chrome.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DecorationMode {
    ServerSide,
    ClientSide,
    Unmanaged,
}

/// One coherent snapshot of the border frame SWM owns around a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowChrome {
    pub mode: DecorationMode,
    pub border_width: i32,
}

impl WindowChrome {
    pub fn for_window(window: &Window, decorations: &swm_config::Decorations) -> Self {
        Self::from_mode(decoration_mode(window), decorations)
    }

    pub fn from_mode(mode: DecorationMode, decorations: &swm_config::Decorations) -> Self {
        let managed = mode != DecorationMode::Unmanaged;
        let border_width = if managed {
            decorations.border_width_px.max(0)
        } else {
            0
        };
        Self { mode, border_width }
    }

    pub fn frame_extents(self) -> (i32, i32, i32, i32) {
        (
            self.border_width,
            self.border_width,
            self.border_width,
            self.border_width,
        )
    }

    pub fn outer_rect<K>(self, client: Rectangle<i32, K>) -> Rectangle<i32, K> {
        let (left, right, top, bottom) = self.frame_extents();
        Rectangle::new(
            (client.loc.x - left, client.loc.y - top).into(),
            (
                client
                    .size
                    .w
                    .saturating_add(left.saturating_add(right))
                    .max(1),
                client
                    .size
                    .h
                    .saturating_add(top.saturating_add(bottom))
                    .max(1),
            )
                .into(),
        )
    }

    pub fn client_rect<K>(self, outer: Rectangle<i32, K>) -> Rectangle<i32, K> {
        let (left, right, top, bottom) = self.frame_extents();
        Rectangle::new(
            (outer.loc.x + left, outer.loc.y + top).into(),
            (
                outer
                    .size
                    .w
                    .saturating_sub(left.saturating_add(right))
                    .max(1),
                outer
                    .size
                    .h
                    .saturating_sub(top.saturating_add(bottom))
                    .max(1),
            )
                .into(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hit {
    Resize(crate::input::grab::ResizeHandle),
}

pub fn decoration_mode(window: &Window) -> DecorationMode {
    if crate::xwayland::is_override_redirect(window) {
        return DecorationMode::Unmanaged;
    }
    if let Some(toplevel) = window.toplevel() {
        return if toplevel.with_committed_state(|state| {
            state.and_then(|state| state.decoration_mode) == Some(Mode::ServerSide)
        }) {
            DecorationMode::ServerSide
        } else {
            DecorationMode::ClientSide
        };
    }
    if crate::xwayland::uses_server_decorations(window) {
        DecorationMode::ServerSide
    } else {
        DecorationMode::ClientSide
    }
}

pub fn client_rect_for_outer(
    window: &Window,
    outer: Rectangle<i32, smithay::utils::Logical>,
    decorations: &swm_config::Decorations,
) -> Rectangle<i32, smithay::utils::Logical> {
    WindowChrome::for_window(window, decorations).client_rect(outer)
}

pub fn outer_rect_for_client(
    window: &Window,
    client: Rectangle<i32, smithay::utils::Logical>,
    decorations: &swm_config::Decorations,
) -> Rectangle<i32, smithay::utils::Logical> {
    WindowChrome::for_window(window, decorations).outer_rect(client)
}

pub fn outer_size_for_client(
    window: &Window,
    client: smithay::utils::Size<i32, smithay::utils::Logical>,
    decorations: &swm_config::Decorations,
) -> smithay::utils::Size<i32, smithay::utils::Logical> {
    WindowChrome::for_window(window, decorations)
        .outer_rect(Rectangle::from_size(client))
        .size
}

/// The frame SWM draws around a client, as `(left, right, top, bottom)`.
///
/// Derived from the same inputs as [`outer_size_for_client`] and
/// [`client_location_for_outer`], so a published `_NET_FRAME_EXTENTS` cannot
/// drift from the frame that is actually rendered.
pub fn frame_extents(
    window: &Window,
    decorations: &swm_config::Decorations,
) -> (i32, i32, i32, i32) {
    WindowChrome::for_window(window, decorations).frame_extents()
}

pub fn client_location_for_outer(
    window: &Window,
    outer: smithay::utils::Point<i32, smithay::utils::Logical>,
    decorations: &swm_config::Decorations,
) -> smithay::utils::Point<i32, smithay::utils::Logical> {
    let (left, _, top, _) = WindowChrome::for_window(window, decorations).frame_extents();
    (outer.x.saturating_add(left), outer.y.saturating_add(top)).into()
}

pub fn scaled_metric(base: i32, scale: f32) -> i32 {
    let base = base.max(0) as f32;
    let scaled = (base * scale.max(0.0)).round();
    if base > 0.0 {
        scaled.max(1.0) as i32
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use smithay::utils::{Logical, Rectangle};

    use super::*;

    #[test]
    fn chrome_policy_uses_uniform_border_for_managed_windows_and_zero_for_unmanaged() {
        let decorations = swm_config::Decorations {
            border_width_px: 3,
            ..swm_config::Decorations::default()
        };

        let ssd = WindowChrome::from_mode(DecorationMode::ServerSide, &decorations);
        let csd = WindowChrome::from_mode(DecorationMode::ClientSide, &decorations);
        let unmanaged = WindowChrome::from_mode(DecorationMode::Unmanaged, &decorations);

        assert_eq!(ssd.frame_extents(), (3, 3, 3, 3));
        assert_eq!(csd.frame_extents(), (3, 3, 3, 3));
        assert_eq!(unmanaged.frame_extents(), (0, 0, 0, 0));
    }

    #[test]
    fn chrome_outer_and_client_geometry_round_trip() {
        let decorations = swm_config::Decorations {
            border_width_px: 3,
            ..swm_config::Decorations::default()
        };
        let chrome = WindowChrome::from_mode(DecorationMode::ServerSide, &decorations);
        let client = Rectangle::<i32, Logical>::new((100, 80).into(), (640, 480).into());

        assert_eq!(chrome.client_rect(chrome.outer_rect(client)), client);
    }

    #[test]
    fn nonzero_metrics_survive_zooming_out() {
        assert_eq!(scaled_metric(8, 0.05), 1);
        assert_eq!(scaled_metric(0, 0.05), 0);
    }
}
