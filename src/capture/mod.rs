mod window_geometry;
pub(crate) use window_geometry::WindowCaptureContext;
mod readback;
mod requests;
pub use requests::{cancel_portal, request_source};
pub mod screencast;
pub mod source_chooser;

use smithay::desktop::{Space, Window};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::seat::WaylandFocus;

pub(crate) use readback::{
    capture_cursor_surface_tree, capture_monitor_region_pixels, capture_source_pixels,
    render_monitor_region_dmabuf, render_source_dmabuf, resolve_source_window,
};
use source_chooser::{SourceChooser, SourceMode, SourcePhase};

use crate::session::{Session, SessionDriver};

#[derive(Default)]
pub struct CaptureState {
    pub(crate) screencast: screencast::ScreencastState,
    source_chooser: SourceChooser,
    pending: Option<PendingSourceChooser>,
}

struct PendingSourceChooser {
    request_handle: String,
    reply: crate::ipc::ReplySender,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturePress {
    Consumed,
    ActivateSource(SourceMode),
    Accept,
}

#[derive(Clone, Copy, Debug)]
pub enum CaptureOverlay<'a> {
    None,
    Highlight(Rectangle<i32, Logical>),
    SourceMenu {
        output_name: &'a str,
        selected: usize,
        hovered: Option<usize>,
        monitor_available: bool,
        window_available: bool,
    },
}

impl CaptureState {
    pub fn is_active(&self) -> bool {
        self.source_chooser.is_active()
    }

    pub fn menu_is_active(&self) -> bool {
        self.source_chooser.phase() == Some(SourcePhase::Menu)
    }

    #[cfg(any(feature = "dbus", test))]
    pub fn source_accessibility(
        &self,
    ) -> (Option<SourcePhase>, usize, Option<&swm_ipc::CaptureSource>) {
        (
            self.source_chooser.phase(),
            self.source_chooser.selected(),
            self.source_chooser.selected_source(),
        )
    }

    pub fn overlay(&self) -> CaptureOverlay<'_> {
        if self.source_chooser.phase() == Some(SourcePhase::Menu) {
            return CaptureOverlay::SourceMenu {
                output_name: self.source_chooser.output_name(),
                selected: self.source_chooser.selected(),
                hovered: self.source_chooser.hovered(),
                monitor_available: self.source_chooser.monitor_available(),
                window_available: self.source_chooser.window_available(),
            };
        }
        self.source_chooser
            .selection_geometry()
            .map(CaptureOverlay::Highlight)
            .unwrap_or(CaptureOverlay::None)
    }

    fn begin_source(
        &mut self,
        space: &Space<Window>,
        preferred_output: Option<&str>,
        source_types: u32,
        pending: PendingSourceChooser,
    ) -> Result<(), PendingSourceChooser> {
        if self.pending.is_some() {
            return Err(pending);
        }
        let output = preferred_output
            .and_then(|name| space.outputs().find(|output| output.name() == name))
            .or_else(|| space.outputs().next());
        let Some(output) = output else {
            return Err(pending);
        };
        let Some(geometry) = space.output_geometry(output) else {
            return Err(pending);
        };
        self.source_chooser
            .begin(source_types, output.name(), geometry);
        self.pending = Some(pending);
        Ok(())
    }

    /// False when the chooser's output disappeared; the caller cancels capture.
    pub fn update_layout(&mut self, space: &Space<Window>) -> bool {
        if !self.source_chooser.is_active() {
            return true;
        }
        let Some(output) = space
            .outputs()
            .find(|output| output.name() == self.source_chooser.output_name())
        else {
            return false;
        };
        if let Some(geometry) = space.output_geometry(output) {
            self.source_chooser.update_output_geometry(geometry);
        }
        true
    }

    pub fn press(&mut self, position: (f64, f64)) -> Option<CapturePress> {
        if !self.source_chooser.is_active() {
            return None;
        }
        if self.source_chooser.phase() == Some(SourcePhase::Menu) {
            Some(
                self.source_chooser
                    .hit_test(Point::from(position))
                    .map(|index| CapturePress::ActivateSource(SourceMode::ALL[index]))
                    .unwrap_or(CapturePress::Consumed),
            )
        } else {
            Some(CapturePress::Accept)
        }
    }

    pub fn motion(&mut self, position: (f64, f64)) -> bool {
        if self.source_chooser.phase() == Some(SourcePhase::Menu) {
            return self.source_chooser.hover_menu(Point::from(position));
        }
        self.source_chooser.is_active()
    }

    pub fn release(&mut self) -> bool {
        self.source_chooser.is_active()
    }

    pub fn move_menu_selection(&mut self, delta: i32) -> bool {
        self.source_chooser.phase() == Some(SourcePhase::Menu)
            && self.source_chooser.move_selection(delta)
    }

    pub fn activate_selected_menu(&mut self) -> bool {
        self.source_chooser.phase() == Some(SourcePhase::Menu)
            && self.source_chooser.activate_selected()
    }

    pub fn activate_source(&mut self, mode: SourceMode) -> bool {
        self.source_chooser.activate(mode)
    }

    pub fn return_to_menu(&mut self) -> bool {
        self.source_chooser.is_active() && self.source_chooser.return_to_menu()
    }

    pub fn hover_source(
        &mut self,
        monitor: swm_ipc::CaptureSource,
        window: Option<(swm_ipc::CaptureSource, Rectangle<i32, Logical>)>,
        monitor_geometry: Rectangle<i32, Logical>,
    ) -> bool {
        self.source_chooser
            .hover_source(monitor, window, monitor_geometry)
    }

    fn accept(&mut self) -> Option<(swm_ipc::CaptureSource, PendingSourceChooser)> {
        let source = self.source_chooser.take_selected()?;
        Some((source, self.pending.take()?))
    }

    fn cancel(&mut self) -> Option<PendingSourceChooser> {
        self.source_chooser.cancel();
        self.pending.take()
    }
}

/// Move between visible, shareable portal sources without moving the pointer.
pub fn move_source_selection<D: SessionDriver>(session: &mut Session<D>, delta: i32) -> bool {
    let phase = session.capture.source_chooser.phase();
    let mut candidates: Vec<(swm_ipc::CaptureSource, Rectangle<i32, Logical>)> = Vec::new();
    match phase {
        Some(SourcePhase::MonitorPick) => {
            for output in session.wayland.space.outputs() {
                if let Some(geometry) = session.wayland.space.output_geometry(output) {
                    candidates.push((
                        swm_ipc::CaptureSource::Monitor {
                            name: output.name(),
                            x: geometry.loc.x,
                            y: geometry.loc.y,
                            width: geometry.size.w,
                            height: geometry.size.h,
                        },
                        geometry,
                    ));
                }
            }
            candidates.sort_by(|a, b| match (&a.0, &b.0) {
                (
                    swm_ipc::CaptureSource::Monitor { name: a, .. },
                    swm_ipc::CaptureSource::Monitor { name: b, .. },
                ) => a.cmp(b),
                _ => std::cmp::Ordering::Equal,
            });
        }
        Some(SourcePhase::WindowPick) => candidates.extend(
            window_selection_candidates(session)
                .into_iter()
                .map(|candidate| (candidate.source, candidate.geometry)),
        ),
        _ => return false,
    }
    let current = session.capture.source_chooser.selected_source();
    let index = current.and_then(|source| {
        candidates
            .iter()
            .position(|(candidate, _)| candidate == source)
    });
    let Some(index) = next_source_index(candidates.len(), index, delta) else {
        return false;
    };
    let (source, geometry) = candidates.swap_remove(index);
    session
        .capture
        .source_chooser
        .select_source(source, geometry)
}

fn next_source_index(count: usize, current: Option<usize>, delta: i32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match current {
        Some(index) if index < count => Some(if delta < 0 {
            (index + count - 1) % count
        } else {
            (index + 1) % count
        }),
        _ => Some(if delta < 0 { count - 1 } else { 0 }),
    }
}

struct WindowSelectionCandidate {
    source: swm_ipc::CaptureSource,
    geometry: Rectangle<i32, Logical>,
}

fn window_selection_candidates<D: SessionDriver>(
    session: &Session<D>,
) -> Vec<WindowSelectionCandidate> {
    let mut records: Vec<_> = session
        .nodes
        .records()
        .filter(|record| {
            record.attached
                && !record.collapsed
                && !session
                    .window_rules
                    .applied(&record.surface)
                    .block_screencast
        })
        .collect();
    records.sort_by(|a, b| {
        (&a.output, &a.title, a.id.as_u64()).cmp(&(&b.output, &b.title, b.id.as_u64()))
    });
    let now = crate::frame_clock::monotonic_now();
    records
        .into_iter()
        .filter_map(|record| {
            let output = session
                .wayland
                .space
                .outputs()
                .find(|output| output.name() == record.output)?;
            let output_geometry = session.wayland.space.output_geometry(output)?;
            let presentation = crate::presentation::window::WindowPresentation::for_window(
                crate::presentation::window::WindowPresentationDesktop {
                    space: &session.wayland.space,
                    cameras: &session.cameras,
                    window_animations: &session.window_animations,
                    fullscreen: &session.fullscreen,
                    maximize: &session.maximize,
                    decorations: &session.settings.decorations,
                },
                &record.window,
                output,
                now,
            )?;
            let geometry = session
                .window_capture_context()
                .visual_geometry(&record.window, presentation.visual_geometry());
            geometry.intersection(output_geometry)?;
            let size = session.window_capture_context().size(&record.window);
            Some(WindowSelectionCandidate {
                geometry,
                source: swm_ipc::CaptureSource::Window {
                    window_id: record.id.as_u64(),
                    width: size.w,
                    height: size.h,
                },
            })
        })
        .collect()
}

pub fn accept_selected<D: SessionDriver>(session: &mut Session<D>) -> bool {
    let blocked = match session.capture.source_chooser.selected_source() {
        Some(swm_ipc::CaptureSource::Window { window_id, .. }) => {
            resolve_source_window(&session.wayland.space, &session.nodes, *window_id)
                .ok()
                .and_then(|window| {
                    window.wl_surface().map(|surface| {
                        session
                            .window_rules
                            .applied(surface.as_ref())
                            .block_screencast
                    })
                })
                .unwrap_or(false)
        }
        _ => false,
    };
    if blocked {
        let output = session.capture.source_chooser.output_name().to_string();
        session.shell.overlays.show_error(
            output,
            "This window is blocked from screen sharing.",
            3000,
            crate::frame_clock::monotonic_now(),
        );
        session.request_redraw();
        return false;
    }
    let Some((mut source, pending)) = session.capture.accept() else {
        return false;
    };
    if let swm_ipc::CaptureSource::Monitor {
        name,
        width,
        height,
        ..
    } = &mut source
        && let Some(output) = session
            .wayland
            .space
            .outputs()
            .find(|output| output.name() == *name)
    {
        let size = crate::render::output_physical_size(output);
        *width = size.w;
        *height = size.h;
    }
    let _ = pending.reply.send(
        swm_ipc::Response::Source(swm_ipc::SourceChooserResponse::Selected(source)),
        Vec::new(),
    );
    finish_modal_capture(session);
    true
}

pub fn cancel_selected<D: SessionDriver>(session: &mut Session<D>) -> bool {
    let Some(pending) = session.capture.cancel() else {
        return false;
    };
    let _ = pending.reply.send(
        swm_ipc::Response::Source(swm_ipc::SourceChooserResponse::Cancelled),
        Vec::new(),
    );
    finish_modal_capture(session);
    true
}

fn begin_modal_capture<D: SessionDriver>(session: &mut Session<D>) {
    let moving_node = match session.interactions.grab() {
        crate::input::grab::Grab::MoveNode { id, .. } => Some(*id),
        _ => None,
    };
    let moving_window = match session.interactions.grab() {
        crate::input::grab::Grab::MoveWindow { id, .. } => *id,
        _ => None,
    };
    if (moving_node.is_some() || moving_window.is_some()) && session.nodes.physics_enabled() {
        let _ = crate::nodes::tick_physics(session, crate::frame_clock::monotonic_now());
    }
    if let Some(id) = moving_node {
        session.nodes.clear_direct_motion(id);
    }
    if let Some(id) = moving_window
        && session.nodes.physics_enabled()
    {
        session
            .nodes
            .lock_released_window(id, crate::frame_clock::monotonic_now());
    }
    crate::session::cancel_compositor_grab(session);
    crate::session::note_pointer_activity(session);
    let keyboard = session.keyboard_handle();
    keyboard.set_focus(session, None, smithay::utils::SERIAL_COUNTER.next_serial());
    session.request_redraw();
}

fn finish_modal_capture<D: SessionDriver>(session: &mut Session<D>) {
    crate::session::note_pointer_activity(session);
    crate::session::sync_keyboard_focus(session, smithay::utils::SERIAL_COUNTER.next_serial());
    session.request_redraw();
}

#[cfg(test)]
mod tests;
