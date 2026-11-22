use std::time::Duration;

mod frame;
mod sleep;
mod vt;

use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, LoopHandle, LoopSignal};
use smithay::backend::drm::{DrmEvent, DrmEventMetadata, DrmEventTime};
use smithay::backend::input::KeyboardKeyEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::drm::control::crtc;
use smithay::reexports::input::Libinput;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::Display;
use smithay::wayland::compositor::CompositorHandler;
use smithay::wayland::drm_syncobj::DrmSyncPointSource;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::backend::tty::TtyBackend;
use crate::backend::{RenderOutcome, RenderStatus, Renderable};
use crate::input::keybinds::BackendKind;
use crate::render::{
    CLEAR_COLOR, CursorContext, DesktopContext, FrameContext, OverlayContext, RenderRequest,
    VisualContext,
};
use crate::wayland;

pub(crate) use self::frame::FrameDemand;
use self::frame::{EstimatedVblankTimer, OutputFrameState, VblankAction};
use super::RenderDriver as _;

struct TtyDriver {
    backend: TtyBackend,
    physical_input: crate::input::devices::PhysicalInputDevices,
    loop_handle: LoopHandle<'static, TtyApp>,
    loop_signal: LoopSignal,
    output_frames: Vec<(Output, OutputFrameState)>,
    pause_reasons: PauseReasons,
    pending_output_config: Option<Vec<swm_config::OutputConfig>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PauseReasons {
    session: bool,
    system_sleep: bool,
}

impl PauseReasons {
    fn any(self) -> bool {
        self.session || self.system_sleep
    }

    fn can_render_output(self, powered: bool) -> bool {
        !self.any() && powered
    }
}

impl crate::ipc::OutputInfoSource for TtyDriver {
    fn output_info(&self) -> Vec<swm_ipc::OutputInfo> {
        crate::ipc::OutputInfoSource::output_info(&self.backend)
    }
}

impl super::RenderDriver for TtyDriver {
    fn dmabuf_capabilities(&mut self) -> crate::backend::dmabuf::DmabufCapabilities {
        self.backend.dmabuf_capabilities()
    }

    fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        self.backend.import_dmabuf(dmabuf)
    }

    fn dmabuf_feedback(
        &self,
        output: &Output,
    ) -> Option<&crate::backend::dmabuf::SurfaceDmabufFeedback> {
        self.backend.dmabuf_feedback(output)
    }

    fn request_redraw(&mut self, output: Option<&Output>) {
        if let Some(output) = output {
            if self.backend.output_dpms_enabled(output)
                && let Some(state) = self.output_frame_mut(output)
            {
                state.queue_redraw();
            }
            return;
        }
        for (output, state) in &mut self.output_frames {
            if self.backend.output_dpms_enabled(output) {
                state.queue_redraw();
            }
        }
    }

    fn with_renderer<T>(
        &mut self,
        f: impl FnOnce(&mut smithay::backend::renderer::gles::GlesRenderer) -> T,
    ) -> T {
        self.backend.with_renderer(f)
    }

    fn register_dmabuf_source(
        &mut self,
        client: smithay::reexports::wayland_server::Client,
        source: smithay::backend::allocator::dmabuf::DmabufSource,
    ) -> bool {
        self.loop_handle
            .insert_source(source, move |_, _, app| {
                let dh = app.wayland.display_handle.clone();
                smithay::wayland::compositor::CompositorHandler::client_compositor_state(
                    app, &client,
                )
                .blocker_cleared(app, &dh);
                Ok(())
            })
            .map(|_| true)
            .unwrap_or_else(|err| {
                eventline::warn!(
                    "implicit sync: failed to register buffer readiness source: {err}"
                );
                false
            })
    }

    fn schedule_render_completion(
        &mut self,
        sync: smithay::backend::renderer::sync::SyncPoint,
        completion: Box<dyn FnOnce() + 'static>,
    ) -> Result<(), String> {
        super::render_completion::schedule(&self.loop_handle, sync, completion)
    }

    fn defer_client_error(
        &mut self,
        client: Client,
        display: smithay::reexports::wayland_server::DisplayHandle,
        error: smithay::reexports::wayland_server::backend::protocol::ProtocolError,
    ) {
        self.loop_handle
            .insert_idle(move |_| client.kill(&display, error));
    }

    fn register_drm_syncobj_source(&mut self, client: Client, source: DrmSyncPointSource) -> bool {
        self.loop_handle
            .insert_source(source, move |_, _, app| {
                let dh = app.wayland.display_handle.clone();
                app.client_compositor_state(&client)
                    .blocker_cleared(app, &dh);
                Ok(())
            })
            .map(|_| true)
            .unwrap_or_else(|err| {
                eventline::warn!("explicit sync: failed to register acquire-point source: {err}");
                false
            })
    }
}

impl super::OutputDriver for TtyDriver {
    fn primary_output(&self) -> &Output {
        self.backend.primary_output()
    }

    fn frame_callback_sequence(&self, output: &Output) -> u32 {
        self.output_frame(output)
            .map(OutputFrameState::frame_callback_sequence)
            .unwrap_or_default()
    }

    fn output_states(&self) -> Vec<super::output::OutputState> {
        self.backend.output_states()
    }

    fn test_output_configuration(
        &mut self,
        configuration: &[super::output::OutputConfiguration],
    ) -> Result<(), String> {
        self.backend.test_output_configuration(configuration)
    }

    fn apply_output_configuration(
        &mut self,
        configuration: &[super::output::OutputConfiguration],
    ) -> Result<Vec<super::output::OutputChange>, String> {
        if self.pause_reasons.any() {
            return Err("cannot apply display configuration while the session is paused".into());
        }
        let changes = self
            .backend
            .apply_runtime_output_configuration(configuration)?;
        for change in &changes {
            if change.before.enabled != change.after.enabled {
                self.recover_output_frame(
                    &change.after.output,
                    crate::frame_clock::monotonic_now(),
                );
            }
            let interval = self
                .backend
                .refresh_interval_for_output(&change.after.output);
            let vrr_active = self.backend.output_vrr_active(&change.after.output);
            if change.before.mode != change.after.mode
                && let Some(state) = self.output_frame_mut(&change.after.output)
            {
                state.replace_clock(interval);
            }
            if let Some(state) = self.output_frame_mut(&change.after.output) {
                state.set_vrr(vrr_active);
            }
        }
        Ok(changes)
    }

    fn gamma_size(&self, output: &Output) -> Result<u32, String> {
        self.backend.gamma_size(output)
    }

    fn set_gamma(&mut self, output: &Output, ramp: Option<Vec<u16>>) -> Result<(), String> {
        self.backend.set_gamma(output, ramp)
    }

    fn apply_dpms(
        &mut self,
        command: swm_ipc::DpmsCommand,
        output: Option<&str>,
    ) -> Result<(), swm_ipc::ServerError> {
        self.apply_dpms_command(command, output)
    }

    fn refresh_output_power_support(&mut self) {
        self.refresh_output_power_availability(None);
    }

    fn output_power(&self, output: &Output) -> Option<bool> {
        self.backend.output_power(output)
    }

    fn capture_output_available(&self, output: &Output) -> bool {
        self.pause_reasons
            .can_render_output(self.backend.output_dpms_enabled(output))
    }

    fn output_requires_lock_frame(&self, output: &Output) -> bool {
        self.backend.output_dpms_enabled(output)
    }
}

impl super::SessionDriver for TtyDriver {
    const BACKEND_KIND: BackendKind = BackendKind::Tty;

    fn update_keyboard_leds(&mut self, state: smithay::input::keyboard::LedState) {
        self.physical_input.update_keyboard_leds(state);
    }

    fn stop(&mut self) {
        self.loop_signal.stop();
    }
}

type TtyApp = super::Session<TtyDriver>;

impl TtyDriver {
    fn output_frame(&self, output: &Output) -> Option<&OutputFrameState> {
        self.output_frames
            .iter()
            .find_map(|(candidate, state)| (candidate == output).then_some(state))
    }

    fn output_frame_mut(&mut self, output: &Output) -> Option<&mut OutputFrameState> {
        self.output_frames
            .iter_mut()
            .find_map(|(candidate, state)| (candidate == output).then_some(state))
    }

    fn refresh_output_power_availability(&mut self, removed: Option<libc::dev_t>) {
        if removed.is_none() && self.pause_reasons.any() {
            return;
        }
        let before: Vec<_> = self
            .backend
            .outputs()
            .map(|output| (output.clone(), self.backend.output_power(output)))
            .collect();
        if let Some(device_id) = removed {
            self.backend.output_power_device_removed(device_id);
        } else {
            self.backend.refresh_output_power_support();
        }
        let now = crate::frame_clock::monotonic_now();
        for (output, old) in before {
            if old != self.backend.output_power(&output) {
                self.recover_output_frame(&output, now);
            }
        }
    }

    fn recover_output_frame(&mut self, output: &Output, now: Duration) {
        let renderable = self
            .pause_reasons
            .can_render_output(self.backend.output_dpms_enabled(output));
        if let Some(state) = self.output_frame_mut(output) {
            for token in state.recover(now, renderable) {
                self.loop_handle.remove(token);
            }
        }
    }

    fn apply_dpms_command(
        &mut self,
        command: swm_ipc::DpmsCommand,
        output: Option<&str>,
    ) -> Result<(), swm_ipc::ServerError> {
        if self.pause_reasons.any() {
            return Err(swm_ipc::ServerError::new(
                swm_ipc::ServerErrorKind::InvalidRequest,
                "cannot change display power while the session is paused",
            ));
        }
        self.refresh_output_power_availability(None);
        let applied = self.backend.apply_dpms(command, output)?;
        let now = crate::frame_clock::monotonic_now();
        for change in applied.changes {
            self.recover_output_frame(&change.output, now);
        }
        match applied.error {
            Some(error) => Err(swm_ipc::ServerError::new(
                swm_ipc::ServerErrorKind::Internal,
                error,
            )),
            None => Ok(()),
        }
    }

    fn wake_dpms_on_input(&mut self, target: Option<&Output>) {
        let output = if !self.backend.any_output_dpms_enabled() {
            None
        } else {
            let Some(target) = target else {
                return;
            };
            if self.backend.output_dpms_enabled(target) {
                return;
            }
            Some(target.name())
        };
        if let Err(err) = self.apply_dpms_command(swm_ipc::DpmsCommand::On, output.as_deref()) {
            eventline::warn!("tty dpms: failed to wake on input: {}", err.message);
        }
    }
}

/// Runs the real-hardware (DRM/KMS) session - takes over the seat and a
/// free VT. Returns (rather than panicking) if `TtyBackend::new()` fails,
/// since that's expected when nested under a host compositor that already
/// holds exclusive session control.
pub fn run(explicit_config_path: Option<std::path::PathBuf>) {
    let initial = crate::config::load_initial(explicit_config_path);
    let config_path = initial.path;
    let runtime_config = initial.config;
    let (backend, session_notifier, drm_notifiers) = match TtyBackend::new(&runtime_config.outputs)
    {
        Ok(parts) => parts,
        Err(err) => {
            eventline::error!("TtyBackend::new() failed: {err}");
            return;
        }
    };
    eventline::info!("TtyBackend constructed successfully");
    let initially_paused = !backend.session().is_active();

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<_>>(backend.session().into());
    libinput_context
        .udev_assign_seat(&backend.session().seat())
        .expect("failed to assign udev seat for libinput");
    if initially_paused {
        eventline::debug!("tty input: session starts inactive; suspending libinput");
        libinput_context.suspend();
    }
    let mut libinput_for_session = libinput_context.clone();
    let libinput_backend = LibinputInputBackend::new(libinput_context);

    let mut event_loop: EventLoop<TtyApp> =
        EventLoop::try_new().expect("failed to create event loop");
    let loop_signal = event_loop.get_signal();
    let loop_handle = event_loop.handle();

    let display: Display<TtyApp> = Display::new().expect("failed to create wayland display");
    let dh = display.handle();

    let mut seat_state = SeatState::new();
    let mut seat: Seat<TtyApp> = seat_state.new_wl_seat(&dh, "seat0");
    let applied_keyboard = crate::input::config::add_keyboard(&mut seat, &runtime_config.input)
        .expect("failed to advertise keyboard capability on the wl_seat");
    seat.add_pointer();

    let outputs: Vec<_> = backend.outputs().cloned().collect();
    let now = crate::frame_clock::monotonic_now();

    let output_frames = outputs
        .iter()
        .cloned()
        .map(|output| {
            let interval = backend.refresh_interval_for_output(&output);
            let vrr_active = backend.output_vrr_active(&output);
            let mut state = OutputFrameState::new(interval);
            state.set_vrr(vrr_active);
            if initially_paused {
                drop(state.suspend(now));
            }
            (output, state)
        })
        .collect();

    let mut driver = TtyDriver {
        backend,
        physical_input: crate::input::devices::PhysicalInputDevices::default(),
        loop_handle: loop_handle.clone(),
        loop_signal,
        output_frames,
        pause_reasons: PauseReasons {
            session: initially_paused,
            system_sleep: false,
        },
        pending_output_config: None,
    };
    let mut wayland = TtyApp::create_wayland_state(dh.clone(), &mut driver);
    for output in &outputs {
        wayland.ensure_output_global::<TtyApp>(output);
    }
    let idle_notifier_state =
        smithay::wayland::idle_notify::IdleNotifierState::new(&dh, loop_handle.clone());
    let _presentation_state = smithay::wayland::presentation::PresentationState::new::<TtyApp>(
        &dh,
        smithay::utils::Clock::<smithay::utils::Monotonic>::new().id() as u32,
    );
    let session_lock = crate::wayland::session_lock::State::new::<TtyDriver>(&dh);
    let drm_syncobj_state = if runtime_config.debug.disable_explicit_sync {
        eventline::info!(
            "explicit sync: disabled by debug.disable-explicit-sync; restart swm to change"
        );
        None
    } else if smithay::wayland::drm_syncobj::supports_syncobj_eventfd(
        driver.backend.drm_device_fd(),
    ) {
        eventline::info!("explicit sync: linux-drm-syncobj-v1 available on the primary DRM device");
        Some(
            smithay::wayland::drm_syncobj::DrmSyncobjState::new::<TtyApp>(
                &dh,
                driver.backend.drm_device_fd().clone(),
            ),
        )
    } else {
        eventline::info!(
            "explicit sync: primary DRM device lacks syncobj eventfd support; protocol disabled"
        );
        None
    };
    let xwayland = crate::xwayland::State::<TtyDriver>::new(&dh, loop_handle.clone());
    let keyboard_monitor = match crate::accessibility::KeyboardMonitorService::start() {
        Ok(service) => Some(service),
        Err(err) => {
            eventline::warn!("accessibility: keyboard monitor unavailable: {err}");
            None
        }
    };
    let mut applied_input = runtime_config.input.clone();
    applied_input.keyboard = applied_keyboard;
    // Portal calls can activate GTK. Wait until the display environment and
    // graphical session are ready before asking for the initial appearance.
    let system_color_scheme = swm_config::SystemColorScheme::NoPreference;
    let mut app = TtyApp::new(super::SessionInit {
        driver,
        runtime_config: &runtime_config,
        applied_input,
        backend_kind: BackendKind::Tty,
        loop_handle: loop_handle.clone(),
        autostart: super::autostart::Autostart::enabled(),
        track_idle: true,
        wayland,
        seat_state,
        seat,
        idle_notifier_state,
        presentation_state: _presentation_state,
        drm_syncobj_state,
        session_lock,
        config_path,
        diagnostic: initial.diagnostic,
        system_color_scheme,
        cameras: crate::presentation::camera::OutputCameras::default(),
        keyboard_monitor,
        xwayland,
    });
    for output in outputs {
        app.wayland
            .space
            .map_output(&output, output.current_location());
        let geometry = app
            .wayland
            .space
            .output_geometry(&output)
            .expect("mapped tty output has geometry");
        app.cameras
            .insert(output.name(), geometry.size.to_physical(1));
    }
    if let Some(geometry) = app
        .wayland
        .space
        .output_geometry(app.driver.backend.primary_output())
    {
        app.pointer.set_position((
            f64::from(geometry.loc.x) + f64::from(geometry.size.w) * 0.5,
            f64::from(geometry.loc.y) + f64::from(geometry.size.h) * 0.5,
        ));
    }
    app.initialize_config_notification();

    let socket_name = super::protocol::init_wayland_listener(display, &mut event_loop);
    app.wayland_display = Some(socket_name.clone());
    eventline::info!("wayland socket ready, WAYLAND_DISPLAY={socket_name:?}");
    app.arm_autostart_once(&socket_name, runtime_config.autostart.once.clone());
    super::environment::activate_session(
        &socket_name,
        &runtime_config.cursor.theme,
        runtime_config.cursor.size,
    );
    if let Err(err) = crate::xwayland::start(&event_loop.handle(), &mut app, true) {
        eventline::warn!("xwayland: unavailable: {err}");
        app.run_autostart_once();
    }

    app.install_io_services(&event_loop.handle());
    if let Err(err) = crate::wayland::wlr_gamma_control::init_reader(&mut app, &event_loop.handle())
    {
        eventline::error!("gamma: failed to start reader: {err}");
    }
    #[cfg(feature = "dbus")]
    {
        let (sender, receiver) = calloop::channel::channel();
        match crate::idle_service::Service::start(sender) {
            Ok(service) => {
                app.idle_service = Some(service);
                event_loop
                    .handle()
                    .insert_source(receiver, |_, _, app: &mut TtyApp| {
                        app.refresh_idle_inhibit();
                    })
                    .expect("failed to register idle inhibitor changes");
            }
            Err(err) => eventline::warn!("idle inhibition: D-Bus service unavailable: {err}"),
        }
    }
    super::environment::notify_ready();
    super::environment::refresh_portals();
    app.install_watchers_and_timers(&event_loop.handle(), apply_runtime_config);
    if let Err(err) = sleep::install(&event_loop.handle()) {
        eventline::warn!("system sleep: failed to install monitor: {err}");
    }

    // Queue every output's first frame through the same state machine used
    // for all later redraws.
    queue_redraw(&mut app);
    redraw_queued_outputs(&mut app, &loop_handle);

    event_loop
        .handle()
        .insert_source(libinput_backend, move |event, _, app| {
            if let smithay::backend::input::InputEvent::Keyboard { event } = &event {
                if !crate::input::devices::seat_key_is_transition(
                    event.state(),
                    event.seat_key_count(),
                ) {
                    return;
                }
                let modifiers = app.keyboard_handle().modifier_state();
                if let Some(vt) = vt::target_from_keycode(
                    event.key_code().raw(),
                    event.state() == smithay::backend::input::KeyState::Pressed,
                    modifiers,
                ) {
                    // The switch may prevent every held key's release from
                    // reaching this VT. Do not retain compositor release-pair
                    // bookkeeping across that boundary.
                    app.interactions.steam_close_pressed = None;
                    app.interactions.suppressed_keys.clear();
                    app.key_repeat.cancel();
                    match app.driver.backend.change_vt(vt) {
                        Ok(()) => eventline::debug!("tty input: requested VT switch to {vt}"),
                        Err(err) => {
                            eventline::warn!("tty input: failed to switch to VT {vt}: {err}")
                        }
                    }
                    return;
                }
            }

            let wake_before = match &event {
                smithay::backend::input::InputEvent::Keyboard { .. } => {
                    crate::wayland::focus::selected_output(&app.wayland).cloned()
                }
                smithay::backend::input::InputEvent::PointerButton { .. }
                | smithay::backend::input::InputEvent::PointerAxis { .. } => output_at_pointer(app),
                _ => None,
            };
            if matches!(
                &event,
                smithay::backend::input::InputEvent::Keyboard { .. }
                    | smithay::backend::input::InputEvent::PointerButton { .. }
                    | smithay::backend::input::InputEvent::PointerAxis { .. }
            ) {
                app.driver.wake_dpms_on_input(wake_before.as_ref());
                app.sync_output_power();
            }
            match &event {
                smithay::backend::input::InputEvent::DeviceAdded { device } => {
                    let leds = app.keyboard_handle().led_state();
                    let first_touch =
                        app.driver
                            .physical_input
                            .added(device.clone(), &app.settings.input, leds);
                    if first_touch && app.seat.get_touch().is_none() {
                        app.seat.add_touch();
                    }
                }
                smithay::backend::input::InputEvent::DeviceRemoved { device }
                    if app.driver.physical_input.removed(device) =>
                {
                    super::touch::cancel_all(app);
                    app.seat.remove_touch();
                }
                smithay::backend::input::InputEvent::DeviceRemoved { .. } => {}
                _ => {}
            }
            super::input::handle(app, &event, &socket_name);
            if matches!(
                &event,
                smithay::backend::input::InputEvent::PointerMotion { .. }
                    | smithay::backend::input::InputEvent::PointerMotionAbsolute { .. }
            ) {
                let target = output_at_pointer(app);
                app.driver.wake_dpms_on_input(target.as_ref());
                app.sync_output_power();
            }
        })
        .expect("failed to insert libinput source");

    event_loop
        .handle()
        .insert_source(session_notifier, {
            let loop_handle = loop_handle.clone();
            move |event, _, app| match event {
                SessionEvent::PauseSession => {
                    eventline::info!("session event: pause");
                    let was_paused = app.driver.pause_reasons.any();
                    app.driver.pause_reasons.session = true;
                    app.interactions.steam_close_pressed = None;
                    libinput_for_session.suspend();
                    if !was_paused {
                        suspend_redraw_state(app, &loop_handle);
                    }
                    app.driver.backend.pause();
                }
                SessionEvent::ActivateSession => {
                    eventline::info!("session event: activate");
                    if libinput_for_session.resume().is_err() {
                        eventline::warn!(
                            "tty input: failed to resume libinput after VT activation"
                        );
                    }
                    match app.driver.backend.resume() {
                        // The whole DRM pipeline (and any frame that was in
                        // flight before the switch away) is gone - reset
                        // clean rather than trusting whatever redraw states
                        // said before the switch.
                        Ok(()) => {
                            app.driver.pause_reasons.session = false;
                            sync_keyboard_leds(app);
                            super::pointer::recover_after_session_resume(app);
                            if !app.driver.pause_reasons.any() {
                                if let Some(outputs) = app.driver.pending_output_config.take() {
                                    apply_tty_output_config(app, &outputs);
                                }
                                resume_redraw_state(app);
                            }
                        }
                        Err(err) => eventline::error!("resume failed: {err}"),
                    }
                }
            }
        })
        .expect("failed to insert session notifier");

    // Fail existing power objects promptly when an initialized connector is
    // unplugged. New connector/CRTC allocation remains startup-only.
    let power_monitor =
        smithay::backend::udev::UdevBackend::new(app.driver.backend.session().seat())
            .expect("failed to monitor DRM output power availability");
    event_loop
        .handle()
        .insert_source(power_monitor, |event, _, app| {
            if let smithay::backend::udev::UdevEvent::Removed { device_id } = event {
                app.driver
                    .refresh_output_power_availability(Some(device_id));
                app.sync_output_power();
            } else if !app.driver.pause_reasons.any() {
                app.driver.refresh_output_power_availability(None);
                app.sync_output_power();
            }
        })
        .expect("failed to insert output power availability monitor");

    let mut drm_tokens = Vec::with_capacity(drm_notifiers.len());
    for (gpu_index, drm_notifier) in drm_notifiers.into_iter().enumerate() {
        let token = event_loop
            .handle()
            .insert_source(drm_notifier, move |event, metadata, app| match event {
                DrmEvent::VBlank(crtc) => on_vblank(app, gpu_index, crtc, metadata.as_ref()),
                DrmEvent::Error(err) => eventline::error!("drm event: error {err:?}"),
            })
            .expect("failed to insert drm notifier");
        drm_tokens.push(token);
    }

    eventline::info!("session ready: outputs active; use the configured Quit chord to exit");
    #[cfg(feature = "dbus")]
    let mut accessibility_tree = crate::accessibility::AccessibilityTree::start().ok();
    event_loop
        .run(None, &mut app, |app| {
            #[cfg(feature = "dbus")]
            if let Some(tree) = &mut accessibility_tree {
                tree.refresh(app);
            }
            app.launch.reap_finished();
            if !app.driver.pause_reasons.any() {
                redraw_queued_outputs(app, &loop_handle);
            }
            crate::ipc::publish_api_events(app);
            super::foreign_toplevel::sync(app);
            app.sync_output_power();
            let _ = app.wayland.display_handle.flush_clients();
        })
        .expect("event loop run failed");
    eventline::info!("quit requested, exiting cleanly");
    app.launch.shutdown();
    super::environment::shutdown_session();
    app.idle_service.take();
    // Notifiers retain the DRM device. Release them before the backend drops
    // its outputs/devices, while the session notifier still owns the seat.
    for token in drm_tokens {
        event_loop.handle().remove(token);
    }
}

fn presentation_time(metadata: Option<&DrmEventMetadata>) -> Option<Duration> {
    match metadata?.time {
        DrmEventTime::Monotonic(time) if !time.is_zero() => Some(time),
        DrmEventTime::Monotonic(_) | DrmEventTime::Realtime(_) => None,
    }
}

fn on_vblank(
    app: &mut TtyApp,
    gpu_index: usize,
    crtc: crtc::Handle,
    metadata: Option<&DrmEventMetadata>,
) {
    let Some(output) = app.driver.backend.output_for_crtc(gpu_index, crtc).cloned() else {
        eventline::warn!("vblank received for unknown CRTC {crtc:?}");
        return;
    };
    let presented = presentation_time(metadata);
    let sequence = metadata
        .map(|metadata| u64::from(metadata.sequence))
        .unwrap_or(0);
    let refresh_interval = app.driver.backend.refresh_interval_for_output(&output);
    let Some(state) = app.driver.output_frame_mut(&output) else {
        return;
    };
    let throttle = state.throttle_vblank(presented, refresh_interval);
    if let Some(token) = throttle.cancel_timer {
        app.driver.loop_handle.remove(token);
    }
    if let Some(delay) = throttle.delay {
        if throttle.warn {
            eventline::warn!(
                "output {:?}: kernel reported a vblank less than half a refresh after the previous event; delaying completion by {delay:?}",
                output.name()
            );
        }
        let delayed_output = output.clone();
        let timer = Timer::from_duration(delay);
        match app
            .driver
            .loop_handle
            .insert_source(timer, move |_, _, app| {
                if let Some(state) = app.driver.output_frame_mut(&delayed_output) {
                    state.vblank_throttle_timer_fired();
                }
                complete_vblank(app, gpu_index, crtc, &delayed_output, presented, sequence);
                TimeoutAction::Drop
            }) {
            Ok(token) => {
                if let Some(state) = app.driver.output_frame_mut(&output) {
                    state.vblank_throttle_timer_armed(token);
                }
                return;
            }
            Err(err) => eventline::warn!(
                "output {:?}: failed to arm vblank throttle timer: {err}",
                output.name()
            ),
        }
    }

    complete_vblank(app, gpu_index, crtc, &output, presented, sequence);
}

fn complete_vblank(
    app: &mut TtyApp,
    gpu_index: usize,
    crtc: crtc::Handle,
    output: &Output,
    presented: Option<Duration>,
    sequence: u64,
) {
    let (submission, acknowledge_failed) = match app.driver.backend.frame_submitted(gpu_index, crtc)
    {
        Ok(submission) => (submission, false),
        Err(err) => {
            eventline::warn!(
                "failed to acknowledge vblank for {:?}: {err}",
                output.name()
            );
            (None, true)
        }
    };
    let Some(state) = app.driver.output_frame_mut(output) else {
        return;
    };
    let (mut action, unexpected) = state.on_vblank(presented);
    if acknowledge_failed {
        // Smithay may have failed while submitting an already-queued follow-up
        // frame. Always redraw after an acknowledgement error so the cleared
        // backend gate is exercised and the output cannot settle while stale.
        state.queue_redraw();
        action = VblankAction::Redraw;
    }
    if let Some(unexpected) = unexpected {
        eventline::warn!(
            "unexpected redraw state on vblank for {:?}: {unexpected}",
            output.name()
        );
    }
    if !app.driver.backend.output_dpms_enabled(output) {
        return;
    }

    if let Some(mut submission) = submission {
        super::release_fifo_barriers(app, std::mem::take(&mut submission.fifo_barriers));
        if let Some(generation) = submission.session_lock_generation {
            app.session_lock.presented(output, generation);
        }
        // Keep the prediction attached to its submitted frame for
        // diagnostics, while reporting the kernel page-flip timestamp to
        // clients whenever it is available.
        let _target_presentation_time = submission.target_presentation_time;
        let fixed_refresh = output
            .current_mode()
            .map(|mode| Duration::from_secs_f64(1_000.0 / mode.refresh as f64))
            .filter(|refresh| !refresh.is_zero());
        let refresh = match (submission.variable_refresh, fixed_refresh) {
            (true, Some(refresh)) => smithay::wayland::presentation::Refresh::variable(refresh),
            (false, Some(refresh)) => smithay::wayland::presentation::Refresh::fixed(refresh),
            (_, None) => smithay::wayland::presentation::Refresh::Unknown,
        };
        let (time, flags) = match presented {
            Some(time) => (
                smithay::utils::Time::<smithay::utils::Monotonic>::from(time),
                smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::Vsync
                    | smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::HwClock
                    | smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::HwCompletion,
            ),
            None => (
                smithay::utils::Clock::<smithay::utils::Monotonic>::new().now(),
                smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::Vsync,
            ),
        };
        submission
            .presentation_feedback
            .presented(time, refresh, sequence, flags);
    }

    if action == VblankAction::SendCallbacks {
        send_output_frame_callbacks(app, output);
    }
}

fn send_output_frame_callbacks(app: &mut TtyApp, output: &Output) {
    let frame_callback_sequence = app
        .driver
        .output_frame(output)
        .map(OutputFrameState::frame_callback_sequence)
        .unwrap_or_default();
    app.send_output_frame_callbacks(
        output,
        frame_callback_sequence,
        true,
        true,
        crate::frame_clock::monotonic_now(),
    );
}

/// Pure state transition - safe to call from any input/event handler with
/// no event-loop access. The actual rendering only ever happens in
/// `redraw_output()`, called from the `run()` tail once a redraw is actually
/// `Queued`.
fn queue_redraw(app: &mut TtyApp) {
    app.request_redraw();
}

fn output_at_pointer(app: &TtyApp) -> Option<Output> {
    app.wayland
        .space
        .output_under(app.pointer.position())
        .next()
        .cloned()
}

fn queue_output_redraw(app: &mut TtyApp, output: &Output) {
    app.request_output_redraw(output);
}

fn apply_runtime_config(app: &mut TtyApp, reload: crate::config::ConfigReload) {
    app.apply_reload(reload, |app, config| {
        let reload_commands = config.autostart.on_reload.clone();
        app.driver.physical_input.reload(&app.settings.input);

        if app.driver.pause_reasons.any() {
            app.driver.pending_output_config = Some(config.outputs.clone());
        } else {
            apply_tty_output_config(app, &config.outputs);
        }
        app.run_autostart_reload(&reload_commands);
    });
}

fn apply_tty_output_config(app: &mut TtyApp, outputs_config: &[swm_config::OutputConfig]) {
    let before = app.driver.backend.output_states();
    let pointer_output = app
        .wayland
        .space
        .output_under(app.pointer.position())
        .next()
        .map(|output| output.name());
    let changes = app.driver.backend.apply_output_config(outputs_config);
    let mut layout_changed = false;
    let mut output_changed = false;

    for change in changes {
        let moved = before
            .iter()
            .find(|state| state.output == change.output && state.enabled)
            .map(|state| change.output.current_location() - state.location)
            .filter(|delta| *delta != (0, 0).into());
        if let Some(delta) = moved {
            app.translate_output_geometry(&change.output.name(), delta);
            if pointer_output.as_deref() == Some(change.output.name().as_str()) {
                let (x, y) = app.pointer.position();
                app.pointer
                    .set_position((x + f64::from(delta.x), y + f64::from(delta.y)));
            }
        }
        output_changed = true;
        if change.enabled_changed {
            app.driver
                .recover_output_frame(&change.output, crate::frame_clock::monotonic_now());
        }
        if change.mode_changed {
            let interval = app
                .driver
                .backend
                .refresh_interval_for_output(&change.output);
            if let Some(state) = app.driver.output_frame_mut(&change.output) {
                state.replace_clock(interval);
            }
        }
        let vrr_active = app.driver.backend.output_vrr_active(&change.output);
        if let Some(state) = app.driver.output_frame_mut(&change.output) {
            state.set_vrr(vrr_active);
        }

        if change.layout_changed {
            app.wayland.ensure_output_global::<TtyApp>(&change.output);
            app.wayland
                .space
                .map_output(&change.output, change.output.current_location());
            smithay::desktop::layer_map_for_output(&change.output).arrange();
            layout_changed = true;
        }

        if (change.size_changed || moved.is_some())
            && let Some(geometry) = app.wayland.space.output_geometry(&change.output)
        {
            if change.scale_changed {
                app.cameras
                    .resize_for_display_scale(&change.output.name(), geometry.size.to_physical(1));
            } else if change.size_changed {
                app.cameras
                    .reset(change.output.name(), geometry.size.to_physical(1));
            }
            let external = app
                .fullscreen
                .reconfigure_output(&app.wayland, &change.output);
            crate::xwayland::reconfigure_fullscreen(external);
        }

        queue_output_redraw(app, &change.output);
    }

    if layout_changed {
        app.wayland.space.refresh();
        crate::wayland::display_scale::refresh(&app.wayland, app.driver.backend.primary_output());
        // A pure output move rebases every window's global coordinates; the X
        // server only learns about it through this resync.
        crate::xwayland::sync_positions(app);
        if !app.capture.update_layout(&app.wayland.space) {
            crate::capture::cancel_selected(app);
        }
    }
    if layout_changed || output_changed {
        app.xwayland.sync_desktop_geometry(&app.wayland.space);
    }
    if output_changed {
        crate::wayland::session_lock::configure_surfaces(app);
        super::pointer::update_client_state(app, app.start_time.elapsed().as_millis() as u32);
        app.notify_output_management();
    }
}

fn redraw_queued_outputs(app: &mut TtyApp, loop_handle: &LoopHandle<'_, TtyApp>) {
    let now = crate::frame_clock::monotonic_now();
    let _ = super::input::tick_grabbed_window_edge_pan(app, now);
    let _ = crate::nodes::tick_physics(app, now);
    // Match startup's connector/CRTC activation order. Iterating the
    // `HashMap` made a multi-output DPMS wake nondeterministic, so the
    // primary could intermittently queue its modeset before the secondary
    // that normally commits first during compositor startup.
    let outputs: Vec<_> = app
        .driver
        .backend
        .outputs()
        .filter(|output| {
            app.driver
                .output_frame(output)
                .is_some_and(OutputFrameState::is_redraw_queued)
        })
        .cloned()
        .collect();

    for output in outputs {
        redraw_output(app, &output, loop_handle);
    }
}

fn redraw_output(app: &mut TtyApp, output: &Output, loop_handle: &LoopHandle<'_, TtyApp>) {
    let now = crate::frame_clock::monotonic_now();
    crate::window::enforce_dialog_stacking(&mut app.wayland);
    let (target_presentation_time, dt) = {
        let state = app
            .driver
            .output_frame_mut(output)
            .expect("redraw output has frame state");
        state.next_frame_sample(now)
    };

    let pointer_is_on_output = app
        .wayland
        .space
        .output_under(app.pointer.position())
        .next()
        .is_some_and(|under| under == output);
    let view_before = pointer_is_on_output
        .then(|| app.cameras.view(&output.name()))
        .flatten();
    let fullscreen_camera_changed = app.sync_fullscreen_camera(output, target_presentation_time);
    let zoom_tick =
        app.cameras
            .get_mut_for_tick(&output.name())
            .map(|(camera, fullscreen_returning)| {
                let before = crate::input::zoom::scale(camera);
                let (after, animating) = crate::input::zoom::tick_camera(
                    camera,
                    fullscreen_returning,
                    &app.settings.zoom,
                    app.settings.input.gestures.pan_decay_rate,
                    dt.as_secs_f32(),
                );
                (animating, (before != after).then_some((before, after)))
            });
    let camera_animating = zoom_tick.is_some_and(|(animating, _)| animating);
    let edge_pan_animating = super::input::grabbed_window_edge_pan_active_on(app, &output.name());
    if let Some((before, after)) = zoom_tick.and_then(|(_, scales)| scales) {
        if after < before {
            crate::nodes::reconcile_landmarks_for_zoom(app, &output.name(), after);
        } else {
            crate::nodes::restore_landmarks_for_zoom(app, &output.name(), after);
        }
        app.shell.overlays.show_zoom_indicator(
            &output.name(),
            after,
            &app.settings.overlays.zoom_indicator,
            target_presentation_time,
        );
    }
    let primary = app.driver.backend.primary_output();
    let window_animating = app.wayland.space.elements().any(|window| {
        wayland::window_is_on_output(window, output, primary)
            && window.wl_surface().is_some_and(|surface| {
                app.window_animations
                    .is_animating(surface.as_ref(), target_presentation_time)
            })
    });
    let move_animating = app.wayland.space.elements().any(|window| {
        wayland::window_is_on_output(window, output, primary)
            && window.wl_surface().is_some_and(|surface| {
                app.window_animations
                    .is_moving(surface.as_ref(), target_presentation_time)
            })
    });
    let fullscreen_animating =
        app.fullscreen
            .is_animating_on_output_matching(output, target_presentation_time, |_| true);
    let maximize_animating = app
        .maximize
        .is_animating_on_output(output, target_presentation_time);
    let closing_animating = app
        .render
        .window_close_animations
        .is_animating_on_output(output, target_presentation_time);
    let node_animating = app
        .nodes
        .is_animating_on_output(&output.name(), target_presentation_time);
    let focus_cycle_animating = app.shell.focus_cycle.tick(target_presentation_time);
    let background_animating = app.background_animates_on_output(output, target_presentation_time);
    let overlay_animating = app
        .shell
        .overlays
        .animating_on_output(&output.name(), target_presentation_time);
    let show_cursor = super::pointer::cursor_visible(app);
    let cursor_override = super::pointer::cursor_override(app);
    crate::cursor::surface::refresh_outputs(
        &app.cursor,
        &app.wayland.space,
        app.pointer.position(),
    );
    crate::wayland::dnd::refresh_outputs(
        app.wayland.dnd_icon.as_ref(),
        &app.wayland.space,
        app.pointer.position(),
    );
    if pointer_is_on_output {
        let next_cursor_frame = show_cursor
            .then(|| {
                app.cursor.current_next_frame_in_with_override(
                    output.current_scale().integer_scale(),
                    target_presentation_time,
                    cursor_override,
                )
            })
            .flatten();
        app.cursor_policy
            .schedule_animation(output, next_cursor_frame);
    }
    // Node animation still includes collapse, slide, preview and physics
    // geometry. Keep its conservative safeguard alongside camera/window
    // transitions; local overlays report their own geometry/alpha damage.
    let geometry_animating = camera_animating
        || edge_pan_animating
        || fullscreen_camera_changed
        || window_animating
        || closing_animating
        || node_animating
        || background_animating
        || fullscreen_animating
        || maximize_animating;
    let local_animating = focus_cycle_animating || overlay_animating;
    let mut frame_demand = FrameDemand::new(geometry_animating, local_animating);
    if pointer_is_on_output {
        let time = app.start_time.elapsed().as_millis() as u32;
        if fullscreen_animating || maximize_animating || move_animating {
            super::pointer::update_client_state(app, time);
        }
    }
    let view_after = pointer_is_on_output
        .then(|| app.cameras.view(&output.name()))
        .flatten();
    if view_before != view_after {
        super::pointer::update_client_state(app, app.start_time.elapsed().as_millis() as u32);
    }
    super::trace::snapshot(app);
    let vrr_auto_eligible = auto_vrr_eligible(app, output, target_presentation_time);

    let outcome = match app.driver.backend.render(
        output,
        RenderRequest {
            frame: FrameContext {
                target_presentation_time,
                vrr_auto_eligible,
                force_full_repaint: frame_demand.force_full_repaint,
                clear: CLEAR_COLOR,
            },
            desktop: DesktopContext {
                fifo: &app.wayland.fifo,
                session_lock: &app.session_lock,
                space: &app.wayland.space,
                focused: app.wayland.focused_window.as_ref(),
                cameras: &app.cameras,
                window_animations: &app.window_animations,
                fullscreen: &app.fullscreen,
                maximize: &app.maximize,
                nodes: &app.nodes,
                window_rules: &app.window_rules,
                screencast: &app.capture.screencast,
                layer_rules: &app.settings.layer_rules,
                node_grab_active: app.interactions.grab().landmark_active(),
            },
            cursor: CursorContext {
                cursor: &app.cursor,
                dnd_icon: app.wayland.dnd_icon.as_ref(),
                cursor_position: app.pointer.position(),
                show_cursor,
                cursor_override,
            },
            overlays: OverlayContext {
                capture_overlay: app.capture.overlay(),
                focus_cycle: &app.shell.focus_cycle,
                overlays: &app.shell.overlays,
                overlay_config: &app.settings.overlays,
            },
            visuals: VisualContext {
                decorations: &app.settings.decorations,
                pins: &app.settings.field.pins,
                background: &app.settings.background,
                background_base: app.config_source.path().and_then(std::path::Path::parent),
            },
            resources: crate::render::resources::RenderResources::from(&mut app.render),
        },
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            eventline::error!("render failed for {:?}: {err}", output.name());
            frame_demand.keep_redrawing = true;
            RenderOutcome::new(RenderStatus::Skipped, None)
        }
    };
    frame_demand.keep_redrawing |=
        outcome.needs_redraw() || app.render.node_renderer.has_pending_icons();
    #[cfg(feature = "xwayland")]
    {
        // Keep frames coming so an animations-off X11 switch's deadline fires.
        frame_demand.keep_redrawing |= app.fullscreen.has_external_commit_wait();
    }
    if app.finish_frame_transitions(target_presentation_time) {
        frame_demand.keep_redrawing = true;
    }
    let feedback = app.driver.dmabuf_feedback(output).cloned();
    if let (Some(feedback), Some(element_states)) = (feedback.as_ref(), outcome.element_states()) {
        let primary_output = app.driver.backend.primary_output().clone();
        wayland::dmabuf::send_output_feedback(
            &app.wayland,
            output,
            &primary_output,
            &app.session_lock,
            feedback,
            element_states,
        );
    }
    if let Some(element_states) = outcome.element_states().cloned() {
        app.update_idle_inhibit_visibility(output, &element_states);
        app.service_image_capture(output);
    }
    let vrr_active = app.driver.backend.output_vrr_active(output);
    if let Some(state) = app.driver.output_frame_mut(output) {
        state.set_vrr(vrr_active);
    }

    if outcome.status() == RenderStatus::Submitted {
        let state = app
            .driver
            .output_frame_mut(output)
            .expect("rendered output has frame state");
        state.advance_frame_callback_sequence();
        if let Some(token) = state.frame_submitted(frame_demand.keep_redrawing) {
            loop_handle.remove(token);
        }
        // The compositor has latched every client buffer used by this frame.
        // Release the next callbacks now; waiting for the page-flip event can
        // starve an otherwise idle output while input is active elsewhere.
        // A commit produced by these callbacks remains gated on that vblank.
        send_output_frame_callbacks(app, output);
        app.service_screencopy(output);
        return;
    }

    let scene_ready = outcome.element_states().is_some();
    super::schedule_fifo_barriers(app, loop_handle, output, outcome.fifo_barriers);
    queue_estimated_vblank_timer(
        app,
        output,
        frame_demand.keep_redrawing,
        scene_ready,
        loop_handle,
    );
}

fn auto_vrr_eligible(app: &TtyApp, output: &Output, now: Duration) -> bool {
    if app.session_lock.active()
        || app.capture.is_active()
        || app.shell.focus_cycle.session().is_some()
        || app
            .nodes
            .hover_preview_visible_on_output(&output.name(), now)
    {
        return false;
    }
    let overlays = app.shell.overlays.snapshot(&output.name(), now);
    if overlays.exit_mix.is_some()
        || overlays.confirmation.is_some()
        || overlays.notification.is_some()
        || overlays.zoom_indicator.is_some()
    {
        return false;
    }
    if smithay::desktop::layer_map_for_output(output)
        .layers_on(Layer::Overlay)
        .next()
        .is_some()
    {
        return false;
    }
    app.fullscreen
        .stable_fullscreen_surface_on_output_matching(output, now, |_| true)
        .is_some_and(|surface| {
            app.window_rules.opacity(surface) >= 0.999
                && !app.capture.screencast.is_window_cast(surface)
        })
}

fn queue_estimated_vblank_timer(
    app: &mut TtyApp,
    output: &Output,
    keep_redrawing: bool,
    scene_ready: bool,
    loop_handle: &LoopHandle<'_, TtyApp>,
) {
    let state = app
        .driver
        .output_frame_mut(output)
        .expect("estimated-vblank output has frame state");
    let now = crate::frame_clock::monotonic_now();
    let EstimatedVblankTimer::ArmAfter(delay) =
        state.frame_skipped(keep_redrawing, scene_ready, now)
    else {
        return;
    };
    let output = output.clone();
    let token = loop_handle
        .insert_source(Timer::from_duration(delay), move |_, _, app| {
            on_estimated_vblank_timer(app, &output);
            TimeoutAction::Drop
        })
        .expect("failed to arm estimated-vblank timer");
    state.timer_armed(token);
}

fn on_estimated_vblank_timer(app: &mut TtyApp, output: &Output) {
    let Some(state) = app.driver.output_frame_mut(output) else {
        return;
    };
    match state.estimated_vblank_fired() {
        Ok(true) => {
            state.advance_frame_callback_sequence();
            send_output_frame_callbacks(app, output)
        }
        Ok(false) => {}
        Err(unexpected) => {
            eventline::warn!(
                "unexpected redraw state on estimated-vblank timer for {:?}: {unexpected}",
                output.name()
            );
        }
    }
}

fn suspend_redraw_state(app: &mut TtyApp, loop_handle: &LoopHandle<'_, TtyApp>) {
    app.wayland.ext_image_copy_capture_state.stop_all();
    let now = crate::frame_clock::monotonic_now();
    for (_, state) in &mut app.driver.output_frames {
        for token in state.suspend(now) {
            loop_handle.remove(token);
        }
    }
}

#[cfg(feature = "dbus")]
fn handle_system_sleep(app: &mut TtyApp, preparing: bool) {
    if preparing {
        if app.driver.pause_reasons.system_sleep {
            return;
        }
        eventline::info!("system sleep: preparing");
        let was_paused = app.driver.pause_reasons.any();
        app.driver.pause_reasons.system_sleep = true;
        if !was_paused {
            let loop_handle = app.driver.loop_handle.clone();
            suspend_redraw_state(app, &loop_handle);
        }
        return;
    }

    eventline::info!("system sleep: resumed");
    app.driver.pause_reasons.system_sleep = false;
    if app.driver.pause_reasons.any() {
        // VT activation will recover the backend after reacquiring the seat.
        // Do not touch KMS while another session owns it.
        eventline::debug!("system sleep: display recovery deferred until VT activation");
        return;
    }
    app.driver.backend.recover_after_system_sleep();
    sync_keyboard_leds(app);
    super::pointer::recover_after_session_resume(app);
    if let Some(outputs) = app.driver.pending_output_config.take() {
        apply_tty_output_config(app, &outputs);
    }
    // A delayed subscription can miss PrepareForSleep(true). In that case
    // request_redraw alone still waits for a pre-sleep flip that may be lost.
    resume_redraw_state(app);
}

fn sync_keyboard_leds(app: &mut TtyApp) {
    if let Some(keyboard) = app.seat.get_keyboard() {
        app.driver
            .physical_input
            .update_keyboard_leds(keyboard.led_state());
    }
}

fn resume_redraw_state(app: &mut TtyApp) {
    let now = crate::frame_clock::monotonic_now();
    let outputs: Vec<_> = app
        .driver
        .output_frames
        .iter()
        .map(|(output, _)| output.clone())
        .collect();
    for output in outputs {
        app.driver.recover_output_frame(&output, now);
    }
}

#[cfg(test)]
mod pause_tests {
    use super::PauseReasons;
    use super::frame::OutputFrameState;
    use std::time::Duration;

    #[test]
    fn overlapping_pause_reasons_require_both_resumes() {
        let mut reasons = PauseReasons {
            session: true,
            system_sleep: true,
        };
        assert!(reasons.any());

        reasons.system_sleep = false;
        assert!(reasons.any());

        reasons.session = false;
        assert!(!reasons.any());
    }

    #[test]
    fn vt_and_sleep_resume_in_either_order_preserve_powered_off_outputs() {
        for session_first in [false, true] {
            let mut reasons = PauseReasons {
                session: true,
                system_sleep: true,
            };
            let mut on = OutputFrameState::new(Duration::from_millis(10));
            let mut off = OutputFrameState::new(Duration::from_millis(20));
            for phase in 0..3 {
                if phase == 1 {
                    if session_first {
                        reasons.session = false;
                    } else {
                        reasons.system_sleep = false;
                    }
                } else if phase == 2 {
                    reasons = PauseReasons::default();
                }
                let now = Duration::from_secs(phase);
                assert!(on.recover(now, reasons.can_render_output(true)).is_empty());
                assert!(
                    off.recover(now, reasons.can_render_output(false))
                        .is_empty()
                );
                assert_eq!(on.is_redraw_queued(), phase == 2);
                assert!(!off.is_redraw_queued());
            }
        }
    }
}
