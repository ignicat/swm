use std::ffi::OsStr;

use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufSource};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::sync::SyncPoint;
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::drm_syncobj::{DrmSyncPointSource, DrmSyncobjState};
use smithay::wayland::idle_notify::IdleNotifierState;
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::seat::WaylandFocus;

use super::output::{OutputChange, OutputConfiguration, OutputState};
use crate::cursor::CursorManager;
use crate::input::Keyboard;
use crate::input::pointer::Pointer;
use crate::presentation::camera::OutputCameras;
use crate::wayland::WaylandState;

/// Rendering and buffer-import mechanics supplied by a session backend.
pub trait RenderDriver: 'static {
    fn dmabuf_capabilities(&mut self) -> crate::backend::dmabuf::DmabufCapabilities;
    fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool;
    fn dmabuf_feedback(
        &self,
        output: &Output,
    ) -> Option<&crate::backend::dmabuf::SurfaceDmabufFeedback>;
    fn request_redraw(&mut self, output: Option<&Output>);
    fn with_renderer<T>(&mut self, f: impl FnOnce(&mut GlesRenderer) -> T) -> T;
    fn schedule_render_completion(
        &mut self,
        sync: SyncPoint,
        completion: Box<dyn FnOnce() + 'static>,
    ) -> Result<(), String>;
    fn register_dmabuf_source(&mut self, client: Client, source: DmabufSource) -> bool;
    fn defer_client_error(
        &mut self,
        client: Client,
        display: DisplayHandle,
        error: smithay::reexports::wayland_server::backend::protocol::ProtocolError,
    );
    fn register_drm_syncobj_source(
        &mut self,
        _client: Client,
        _source: DrmSyncPointSource,
    ) -> bool {
        false
    }
}

/// Output discovery and hardware-policy mechanics supplied by a backend.
pub trait OutputDriver: crate::ipc::OutputInfoSource + 'static {
    fn primary_output(&self) -> &Output;
    fn frame_callback_sequence(&self, output: &Output) -> u32;
    fn output_states(&self) -> Vec<OutputState>;
    fn test_output_configuration(
        &mut self,
        configuration: &[OutputConfiguration],
    ) -> Result<(), String>;
    fn apply_output_configuration(
        &mut self,
        configuration: &[OutputConfiguration],
    ) -> Result<Vec<OutputChange>, String>;
    fn gamma_size(&self, _output: &Output) -> Result<u32, String> {
        Err("gamma control is not supported by this backend".into())
    }
    fn set_gamma(&mut self, _output: &Output, _ramp: Option<Vec<u16>>) -> Result<(), String> {
        Err("gamma control is not supported by this backend".into())
    }
    fn apply_dpms(
        &mut self,
        _command: swm_ipc::DpmsCommand,
        _output: Option<&str>,
    ) -> Result<(), swm_ipc::ServerError> {
        Err(swm_ipc::ServerError::new(
            swm_ipc::ServerErrorKind::Unsupported,
            "dpms is only supported on the tty backend",
        ))
    }
    fn refresh_output_power_support(&mut self) {}
    /// None means disconnected, logically disabled, or unsupported.
    fn output_power(&self, _output: &Output) -> Option<bool> {
        None
    }
    fn capture_output_available(&self, output: &Output) -> bool {
        self.output_requires_lock_frame(output)
    }
    fn output_requires_lock_frame(&self, _output: &Output) -> bool {
        true
    }
}

/// Minimal coordinator contract shared by backend-independent session policy.
pub trait SessionDriver: RenderDriver + OutputDriver + 'static {
    const BACKEND_KIND: crate::input::keybinds::BackendKind;
    fn stop(&mut self);
    /// Nested backends leave physical keyboard indicators to their host.
    fn update_keyboard_leds(&mut self, _state: smithay::input::keyboard::LedState) {}
}

/// Backend-independent compositor state.
///
/// `D` owns only backend mechanics. Wayland policy, input state, cameras, and
/// runtime visual state live here once so nested and real-hardware sessions
/// cannot evolve different behavior.
pub struct Session<D: SessionDriver> {
    pub driver: D,
    pub keyboard: Keyboard,
    pub(super) key_repeat: super::input::repeat::Policy<D>,
    pub(super) launch: super::launch::LaunchState,
    pub pointer: Pointer,
    pub cursor: CursorManager,
    pub(crate) cursor_policy: super::cursor::Policy<D>,
    pub wayland: WaylandState,
    pub(super) seat_state: SeatState<Self>,
    pub seat: Seat<Self>,
    pub(crate) popup_grab: Option<smithay::desktop::PopupGrab<Self>>,
    pub idle_notifier_state: IdleNotifierState<Self>,
    pub(crate) idle_service: Option<crate::idle_service::Service>,
    /// Owns the `wp_presentation` global for the session's lifetime. swm
    /// never reads it; presentation feedback is sent per frame elsewhere.
    pub(super) _presentation_state: PresentationState,
    pub(super) commit_timing: super::commit_timing::Policy<D>,
    pub(super) dpms_requests: super::output_power::Requests<D>,
    pub(super) drm_syncobj_state: Option<DrmSyncobjState>,
    pub session_lock: crate::wayland::session_lock::State,
    pub start_time: std::time::Instant,
    pub(crate) wayland_display: Option<std::ffi::OsString>,
    pub(crate) config_source: super::config_source::ConfigSource,
    pub shell: crate::shell::state::ShellState,
    pub settings: super::RuntimeSettings,
    pub nodes: crate::nodes::NodesState,
    pub(crate) trail: crate::trail::TrailState,
    pub(crate) api_subscriptions: crate::ipc::ApiSubscriptions,
    pub window_rules: crate::window::rules::WindowRulesState,
    pub(super) presentation_close_size_recovery:
        crate::window::recovery::PresentationCloseSizeRecovery,
    pub cameras: OutputCameras,
    pub capture: crate::capture::CaptureState,
    pub interactions: super::InteractionState,
    pub(super) touch: super::touch::TouchState,
    pub(super) gestures: super::gesture::GestureState,
    #[cfg(feature = "xwayland")]
    pub(super) window_trace: super::trace::WindowTrace,
    pub keyboard_monitor: Option<crate::accessibility::KeyboardMonitorService>,
    pub opening_origins: super::opening::OpeningOrigins,
    pub window_animations: crate::animation::WindowAnimations,
    pub render: crate::render::resources::RenderState,
    pub fullscreen: crate::wayland::fullscreen::FullscreenManager,
    pub maximize: crate::presentation::maximize::FieldMaximizeManager,
    pub xwayland: crate::xwayland::State<D>,
}

/// Backend-specific startup parameters passed into [`Session::new`].
pub(super) struct SessionInit<'a, D: SessionDriver> {
    pub driver: D,
    pub runtime_config: &'a swm_config::RuntimeConfig,
    pub applied_input: swm_config::Input,
    pub backend_kind: crate::input::keybinds::BackendKind,
    pub loop_handle: calloop::LoopHandle<'static, Session<D>>,
    pub autostart: super::autostart::Autostart,
    pub track_idle: bool,
    pub wayland: WaylandState,
    pub seat_state: SeatState<Session<D>>,
    pub seat: Seat<Session<D>>,
    pub idle_notifier_state: IdleNotifierState<Session<D>>,
    pub presentation_state: PresentationState,
    pub drm_syncobj_state: Option<DrmSyncobjState>,
    pub session_lock: crate::wayland::session_lock::State,
    pub config_path: Option<std::path::PathBuf>,
    pub diagnostic: Option<Box<swm_config::ConfigDiagnostic>>,
    pub system_color_scheme: swm_config::SystemColorScheme,
    pub cameras: OutputCameras,
    pub keyboard_monitor: Option<crate::accessibility::KeyboardMonitorService>,
    pub xwayland: crate::xwayland::State<D>,
}

impl<D: SessionDriver> Session<D> {
    pub(super) fn new(init: SessionInit<'_, D>) -> Self {
        let mut launch_environment =
            super::environment::LaunchEnvironment::new(&init.runtime_config.env);
        launch_environment.set_cursor_theme(&init.runtime_config.cursor.theme);
        let launch_path = launch_environment.path();
        Self {
            driver: init.driver,
            keyboard: Keyboard::from_config(
                &init.runtime_config.keybinds,
                init.backend_kind,
                launch_path.as_deref(),
            ),
            key_repeat: super::input::repeat::Policy::new(init.loop_handle.clone()),
            launch: super::launch::LaunchState::new(
                launch_environment,
                init.autostart,
                init.track_idle,
            ),
            pointer: Pointer::new((100.0, 100.0)),
            cursor: CursorManager::new(&init.runtime_config.cursor),
            cursor_policy: super::cursor::Policy::new(
                &init.runtime_config.cursor,
                init.loop_handle.clone(),
            ),
            wayland: init.wayland,
            seat_state: init.seat_state,
            seat: init.seat,
            popup_grab: None,
            idle_notifier_state: init.idle_notifier_state,
            idle_service: None,
            _presentation_state: init.presentation_state,
            commit_timing: super::commit_timing::Policy::new(init.loop_handle.clone()),
            dpms_requests: super::output_power::Requests::new(init.loop_handle),
            drm_syncobj_state: init.drm_syncobj_state,
            session_lock: init.session_lock,
            start_time: std::time::Instant::now(),
            wayland_display: None,
            config_source: super::config_source::ConfigSource::new(
                init.config_path,
                init.diagnostic,
            ),
            shell: crate::shell::state::ShellState::new(),
            settings: super::RuntimeSettings::new_with_color_scheme(
                init.runtime_config,
                init.applied_input,
                init.system_color_scheme,
            ),
            nodes: crate::nodes::NodesState::new_with_color_scheme(
                init.runtime_config,
                init.system_color_scheme,
            ),
            trail: crate::trail::TrailState::new(init.runtime_config.trail),
            api_subscriptions: crate::ipc::ApiSubscriptions::default(),
            window_rules: crate::window::rules::WindowRulesState::new(
                init.runtime_config.window_rules.clone(),
            ),
            presentation_close_size_recovery:
                crate::window::recovery::PresentationCloseSizeRecovery::default(),
            cameras: init.cameras,
            capture: crate::capture::CaptureState::default(),
            interactions: super::InteractionState::default(),
            touch: super::touch::TouchState::default(),
            gestures: super::gesture::GestureState::default(),
            #[cfg(feature = "xwayland")]
            window_trace: super::trace::WindowTrace::from_env(),
            keyboard_monitor: init.keyboard_monitor,
            opening_origins: super::opening::OpeningOrigins::default(),
            window_animations: crate::animation::WindowAnimations::new(
                init.runtime_config.animations.clone(),
            ),
            render: crate::render::resources::RenderState::new(
                init.runtime_config.animations.clone(),
                &init.runtime_config.font,
            ),
            fullscreen: crate::wayland::fullscreen::FullscreenManager::new(
                init.runtime_config.animations.clone(),
            ),
            maximize: crate::presentation::maximize::FieldMaximizeManager::new(
                init.runtime_config.field,
                init.runtime_config.animations.clone(),
            ),
            xwayland: init.xwayland,
        }
    }

    pub(super) fn install_io_services(&mut self, loop_handle: &calloop::LoopHandle<'static, Self>) {
        if let Err(err) = crate::ipc::init_ipc_listener(
            loop_handle,
            |app: &mut Self, request| {
                crate::ipc::handle_request(app, request);
            },
            |app, client_id| {
                if app.capture.screencast.disconnect(client_id) {
                    app.refresh_window_cast_rules();
                    app.request_redraw();
                }
                app.api_subscriptions.disconnect(client_id);
            },
        ) {
            eventline::error!("ipc: failed to start listener: {err}");
        }
    }

    pub(super) fn install_watchers_and_timers(
        &mut self,
        loop_handle: &calloop::LoopHandle<'static, Self>,
        on_reload: fn(&mut Self, crate::config::ConfigReload),
    ) {
        if let Some(path) = self.config_source.path().map(std::path::Path::to_path_buf) {
            match crate::config::watch(loop_handle, path, on_reload) {
                Ok(watcher) => self.config_source.set_watcher(watcher),
                Err(err) => eventline::warn!("config: failed to start watcher: {err}"),
            }
        }
        if let Err(err) = crate::appearance::watch(loop_handle, |app: &mut Self, scheme| {
            app.apply_system_color_scheme(scheme);
        }) {
            eventline::warn!("appearance: failed to start system colour watcher: {err}");
        }
        if let Err(err) = super::install_node_decay_timer(loop_handle) {
            eventline::warn!("nodes: failed to start decay timer: {err}");
        }
        if let Err(err) = super::install_overlay_timer(loop_handle) {
            eventline::warn!("overlays: failed to start lifecycle timer: {err}");
        }
        super::install_frame_callback_fallback_timer(loop_handle)
            .expect("failed to start frame-callback and FIFO progress timer");
    }

    pub(super) fn apply_reload(
        &mut self,
        reload: crate::config::ConfigReload,
        apply_backend: impl FnOnce(&mut Self, &swm_config::RuntimeConfig),
    ) {
        let accepted = matches!(reload, crate::config::ConfigReload::Loaded(_));
        match reload {
            crate::config::ConfigReload::Loaded(config) => {
                let config = *config;
                self.apply_common_config(&config);
                apply_backend(self, &config);
                self.clear_config_reload_error();
            }
            crate::config::ConfigReload::Rejected(diagnostic) => {
                eventline::debug!("config: rejected reload for {:?}", diagnostic.path);
                self.show_config_reload_error();
            }
        }
        crate::ipc::publish_config_reload(self, accepted);
    }

    pub(super) fn send_output_frame_callbacks(
        &mut self,
        output: &Output,
        frame_callback_sequence: u32,
        send_lock_frames: bool,
        filter_output_windows: bool,
        hover_now: std::time::Duration,
    ) {
        let elapsed = self.start_time.elapsed();
        let primary = filter_output_windows.then(|| self.driver.primary_output().clone());
        if self.session_lock.active() {
            if send_lock_frames {
                crate::wayland::session_lock::send_frames(
                    &self.session_lock,
                    output,
                    elapsed,
                    frame_callback_sequence,
                );
            }
        } else if self.shell.focus_cycle.is_active() {
            crate::shell::focus_cycle::send_preview_frames(
                &self.shell.focus_cycle,
                &self.nodes,
                output,
                elapsed,
                frame_callback_sequence,
            );
        } else {
            self.wayland
                .space
                .elements()
                .filter(|window| {
                    primary.as_ref().is_none_or(|primary| {
                        crate::wayland::window_is_on_output(window, output, primary)
                    })
                })
                .for_each(|window| {
                    let compositor_snapshot = window.wl_surface().is_some_and(|surface| {
                        self.render
                            .fullscreen_textures
                            .awaiting_target(surface.as_ref())
                    });
                    let require_visible =
                        crate::wayland::frame_callbacks::requires_render_visibility(
                            compositor_snapshot,
                        );
                    window.send_frame(
                        output,
                        elapsed,
                        crate::wayland::frame_callbacks::FALLBACK_THROTTLE,
                        |surface, states| {
                            crate::wayland::frame_callbacks::callback_output(
                                surface,
                                states,
                                output,
                                frame_callback_sequence,
                                require_visible,
                            )
                        },
                    );
                });
            crate::nodes::send_hover_preview_frame(
                &self.nodes,
                output,
                elapsed,
                hover_now,
                frame_callback_sequence,
            );
        }
        crate::wayland::layer_shell::send_frames(output, elapsed, frame_callback_sequence);
        crate::cursor::surface::send_frame(
            &self.cursor,
            &self.wayland.space,
            output,
            self.pointer.position(),
            elapsed,
            frame_callback_sequence,
        );
        crate::wayland::dnd::send_frame(
            self.wayland.dnd_icon.as_ref(),
            output,
            elapsed,
            frame_callback_sequence,
        );
        self.wayland.space.refresh();
        if crate::xwayland::sync_positions(self) {
            // The client's root coordinates just moved under a possibly stationary
            // pointer, so its hover state is stale until it is re-routed.
            super::pointer::refresh_desktop_client_focus(
                self,
                self.start_time.elapsed().as_millis() as u32,
            );
        }
        crate::xwayland::sync_stacking_order(self);
        crate::wayland::layer_shell::cleanup(&mut self.wayland);
    }

    pub(super) fn finish_frame_transitions(&mut self, now: std::time::Duration) -> bool {
        let window_cleanup = self.window_animations.cleanup(now);
        if window_cleanup {
            // Owe one live-geometry frame after retiring a window movement
            // timeline so stale state cannot remain latched.
            super::pointer::update_client_state(self, self.start_time.elapsed().as_millis() as u32);
        }
        self.render.window_close_animations.cleanup(now);
        let fullscreen_cleanup = self.cleanup_fullscreen(now);
        if fullscreen_cleanup {
            // Cleanup retires the transition and drops its crossfade textures, so
            // the scene this frame rendered is the last one drawn from those
            // textures. One more frame is owed to swap back to the live surfaces;
            // without it the swap waits on unrelated damage and lands as a pop.
            super::sync_keyboard_focus(self, smithay::utils::SERIAL_COUNTER.next_serial());
            super::pointer::update_client_state(self, self.start_time.elapsed().as_millis() as u32);
        }
        window_cleanup || fullscreen_cleanup
    }

    pub(crate) fn window_capture_context(&self) -> crate::capture::WindowCaptureContext<'_> {
        crate::capture::WindowCaptureContext::new(
            &self.wayland,
            self.driver.primary_output(),
            &self.fullscreen,
            &self.settings.decorations,
        )
    }

    /// End the compositor pointer grab: clear it and drop the grab cursor
    /// together, so a grab can never end with its move or resize cursor left
    /// on screen.
    pub(crate) fn end_grab(&mut self) {
        self.interactions.end_grab_keeping_cursor();
        self.cursor
            .set_override(crate::cursor::OverrideSource::Grab, None);
    }

    pub(crate) fn keyboard_handle(&self) -> smithay::input::keyboard::KeyboardHandle<Self> {
        self.seat
            .get_keyboard()
            .expect("keyboard capability added at seat setup")
    }

    pub(crate) fn pointer_handle(&self) -> smithay::input::pointer::PointerHandle<Self> {
        self.seat
            .get_pointer()
            .expect("pointer capability added at seat setup")
    }

    #[cfg(any(feature = "xwayland", feature = "dbus"))]
    pub(crate) fn launch_environment(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.launch.environment().iter()
    }

    pub(crate) fn arm_autostart_once(&mut self, wayland_display: &OsStr, commands: Vec<String>) {
        self.launch.arm_once(wayland_display, commands);
    }

    pub(crate) fn run_autostart_once(&mut self) {
        let x11_display = self.xwayland.display_name();
        self.launch
            .run_once(x11_display.as_deref(), self.cursor.size());
    }

    pub(crate) fn run_autostart_reload(&mut self, commands: &[String]) {
        let x11_display = self.xwayland.display_name();
        self.launch
            .run_reload(commands, x11_display.as_deref(), self.cursor.size());
    }

    pub fn request_redraw(&mut self) {
        self.driver.request_redraw(None);
    }

    pub fn request_output_redraw(&mut self, output: &Output) {
        self.driver.request_redraw(Some(output));
    }

    pub fn apply_system_color_scheme(&mut self, scheme: swm_config::SystemColorScheme) {
        let changed = self.settings.set_system_color_scheme(scheme)
            | self.nodes.set_system_color_scheme(scheme);
        if changed {
            eventline::debug!("appearance: applied system colour scheme {scheme:?}");
            self.request_redraw();
        }
    }

    pub fn notification_output_name(&self) -> String {
        crate::wayland::focus::selected_output(&self.wayland)
            .unwrap_or_else(|| self.driver.primary_output())
            .name()
    }

    pub fn initialize_config_notification(&mut self) {
        let now = crate::frame_clock::monotonic_now();
        let output = self.notification_output_name();
        if self.config_source.take_startup_error() {
            self.shell.overlays.show_config_error(
                output,
                self.settings.overlays.notifications.error_duration_ms,
                now,
            );
        } else if let Some(path) = self.config_source.path() {
            self.shell.overlays.show_config_success(
                output,
                path,
                self.settings.overlays.notifications.success_duration_ms,
                now,
            );
        }
        self.request_redraw();
    }

    pub fn show_config_reload_error(&mut self) {
        let output = self.notification_output_name();
        self.shell.overlays.show_config_error(
            output,
            self.settings.overlays.notifications.error_duration_ms,
            crate::frame_clock::monotonic_now(),
        );
        self.request_redraw();
    }

    pub fn clear_config_reload_error(&mut self) {
        if self
            .shell
            .overlays
            .clear_config_error(crate::frame_clock::monotonic_now())
        {
            self.request_redraw();
        }
    }

    pub fn show_exit_confirmation(&mut self) {
        if !self
            .shell
            .overlays
            .show_exit(crate::frame_clock::monotonic_now())
        {
            return;
        }
        super::cancel_compositor_grab(self);
        super::gesture::cancel_all(self);
        super::touch::cancel_all(self);
        self.request_redraw();
    }

    pub fn cancel_exit_confirmation(&mut self) {
        if self
            .shell
            .overlays
            .cancel_exit(crate::frame_clock::monotonic_now())
        {
            self.request_redraw();
        }
    }

    pub fn confirm_exit(&mut self) {
        if self.shell.overlays.exit_modal_active() {
            self.driver.stop();
        }
    }

    /// Applies every backend-independent setting from one validated config
    /// snapshot. Output hardware policy remains with the concrete driver.
    pub fn apply_common_config(&mut self, config: &swm_config::RuntimeConfig) {
        self.key_repeat.cancel();
        let cancel_touch = self.settings.input.gestures.touch_passthrough
            && !config.input.gestures.touch_passthrough;
        let cancel_gestures = self.settings.input.gestures != config.input.gestures;
        if cancel_touch {
            super::touch::cancel_all(self);
        }
        if cancel_gestures {
            super::gesture::cancel_all(self);
        }
        let launch_path = self
            .launch
            .reload_environment(&config.env, &config.cursor.theme);
        self.keyboard
            .reload(&config.keybinds, D::BACKEND_KIND, launch_path.as_deref());
        crate::input::config::reload(self, &config.input);
        let cursor_changed = self.cursor.reload(&config.cursor);
        #[cfg(feature = "xwayland")]
        if cursor_changed {
            self.xwayland.sync_cursor(&self.cursor);
        }
        let cursor_visibility_changed = self.cursor_policy.reload(&config.cursor);
        if cursor_changed {
            self.launch
                .publish_cursor_settings(&config.cursor.theme, config.cursor.size);
        }
        let window_rules_redraw = self.window_rules_reload_changes_visuals(&config.window_rules);
        let decorations_changed = self.settings.decorations != config.decorations;
        if decorations_changed {
            self.render.overlay_previews.mark_all_dirty();
        }
        let redraw = self.settings.visuals_changed(config)
            || window_rules_redraw
            || cursor_changed
            || cursor_visibility_changed;
        let nodes_redraw = self
            .nodes
            .reload(config, crate::frame_clock::monotonic_now());
        self.trail.reload(config.trail);
        let zoom_indicator_redraw = self
            .shell
            .overlays
            .reload_zoom_indicator(&config.overlays.zoom_indicator);
        let font_redraw = self.render.ui_text.reload_font(&config.font);
        if D::BACKEND_KIND == crate::input::keybinds::BackendKind::Tty
            && self.settings.debug.disable_explicit_sync != config.debug.disable_explicit_sync
        {
            eventline::info!(
                "config: debug.disable-explicit-sync changed; restart swm to apply it"
            );
        }
        self.settings.reload_non_input(config);
        if decorations_changed {
            // A changed border width resizes the frame every X11 client
            // computes its root coordinates against.
            self.xwayland
                .sync_all_frame_extents(&self.wayland.space, &self.settings.decorations);
        }
        self.window_animations.reload(config.animations.clone());
        self.render
            .window_close_animations
            .reload(config.animations.clone());
        self.render.window_shaders.reload(&config.animations);
        let fullscreen_redraw = self.fullscreen.reload(config.animations.clone());
        if fullscreen_redraw {
            self.render.fullscreen_textures.remove_owner(
                crate::render::fullscreen_texture::TextureTransitionOwner::Fullscreen,
            );
        }
        let maximize_redraw = self
            .maximize
            .reload(config.field, config.animations.clone());
        if maximize_redraw {
            self.render
                .fullscreen_textures
                .remove_owner(crate::render::fullscreen_texture::TextureTransitionOwner::Maximize);
        }
        if nodes_redraw {
            crate::nodes::reconcile_landmarks(self, None);
        }
        if redraw
            || nodes_redraw
            || zoom_indicator_redraw
            || font_redraw
            || fullscreen_redraw
            || maximize_redraw
        {
            self.request_redraw();
        }
    }

    pub fn background_animates_on_output(
        &self,
        _output: &Output,
        _now: std::time::Duration,
    ) -> bool {
        false
    }

    pub(crate) fn refresh_dynamic_window_rules(&mut self) {
        let windows = self
            .wayland
            .space
            .elements()
            .cloned()
            .chain(self.wayland.collapsed.values().cloned())
            .chain(self.wayland.unmapped.values().cloned())
            .collect::<Vec<_>>();
        let mut changed = false;
        for window in windows {
            if let Some(surface) = window.wl_surface() {
                let before = self.window_rules.applied(surface.as_ref());
                let after = self.window_rules.track_window(&window);
                changed |= before != after;
            }
        }
        if changed {
            self.render.overlay_previews.mark_all_dirty();
            self.request_redraw();
        }
    }

    pub(crate) fn refresh_window_cast_rules(&mut self) {
        if self
            .window_rules
            .set_casts(self.capture.screencast.window_cast_targets())
        {
            self.refresh_dynamic_window_rules();
        }
    }

    fn window_rules_reload_changes_visuals(&mut self, rules: &[swm_config::WindowRule]) -> bool {
        self.window_rules.reload(rules.to_vec());
        let windows = self
            .wayland
            .space
            .elements()
            .cloned()
            .chain(self.wayland.collapsed.values().cloned())
            .chain(self.wayland.unmapped.values().cloned())
            .collect::<Vec<_>>();
        let mut changed = false;
        for window in windows {
            let Some(surface) = window.wl_surface().map(|surface| surface.into_owned()) else {
                continue;
            };
            let before = self.window_rules.applied(&surface);
            let after = self.window_rules.track_window(&window);
            changed |= before.opacity != after.opacity || before.border != after.border;
        }
        changed
    }

    pub fn cleanup_fullscreen(&mut self, now: std::time::Duration) -> bool {
        #[cfg(feature = "xwayland")]
        let mut native_started = !self
            .fullscreen
            .release_external_commit_waits(&mut self.wayland, now)
            .is_empty();
        #[cfg(not(feature = "xwayland"))]
        let mut native_started = false;
        for surface in self.fullscreen.native_size_retries_due(now) {
            let Some(window) = self
                .wayland
                .space
                .elements()
                .find(|window| {
                    window
                        .wl_surface()
                        .is_some_and(|candidate| candidate.as_ref() == &surface)
                })
                .cloned()
            else {
                continue;
            };
            let textures = &mut self.render.fullscreen_textures;
            let has_snapshot = textures.awaiting_target(&surface);
            let owner = crate::render::fullscreen_texture::TextureTransitionOwner::Fullscreen;
            // Validate the current tree again: a newer commit may have
            // replaced the complete endpoint which originally armed the wait.
            let ready = !has_snapshot
                || self.driver.with_renderer(|renderer| {
                    textures
                        .prepare_target(renderer, &window, owner)
                        .unwrap_or(false)
                });
            let buffer_size =
                smithay::backend::renderer::utils::with_renderer_surface_state(&surface, |state| {
                    state.surface_size()
                })
                .flatten();
            if self.fullscreen.handle_commit(
                &mut self.wayland,
                &self.cameras,
                &surface,
                buffer_size,
                ready,
                now,
            ) {
                if has_snapshot {
                    // prepare_target succeeded in the same event-loop turn,
                    // so capture_target adopts that exact validated endpoint.
                    let _ = self.driver.with_renderer(|renderer| {
                        textures.capture_target(renderer, &window, owner)
                    });
                }
                native_started = true;
            } else {
                // A newer incomplete tree or incompatible protocol state
                // needs another client commit, rather than repeated GPU
                // captures of the same rejected endpoint on every frame.
                self.fullscreen
                    .defer_native_size_retry_until_commit(&surface);
            }
        }
        let cleanup = self.fullscreen.cleanup(now);
        let maximize_cleanup = self.maximize.cleanup(now);
        for surface in cleanup.finished_surfaces {
            self.render.fullscreen_textures.remove(&surface);
        }
        for surface in maximize_cleanup.finished_surfaces {
            self.render.fullscreen_textures.remove(&surface);
        }
        let outputs = self.wayland.space.outputs().cloned().collect::<Vec<_>>();
        for output in outputs {
            self.sync_fullscreen_camera(&output, now);
        }
        native_started || cleanup.visual_finished || maximize_cleanup.visual_finished
    }

    pub fn sync_fullscreen_camera(
        &mut self,
        output: &smithay::output::Output,
        now: std::time::Duration,
    ) -> bool {
        let frame = self
            .wayland
            .space
            .output_geometry(output)
            .and_then(|geometry| {
                self.fullscreen
                    .camera_frame_matching(output, geometry, now, |_| true)
            });
        if let Some(frame) = frame {
            let field_changed = self.cameras.apply_field_maximize(&output.name(), None);
            field_changed | self.cameras.apply_fullscreen(&output.name(), Some(frame))
        } else {
            let fullscreen_changed = self.cameras.apply_fullscreen(&output.name(), None);
            let maximize_progress = self.maximize.camera_progress(output, now);
            fullscreen_changed
                | self
                    .cameras
                    .apply_field_maximize(&output.name(), maximize_progress)
        }
    }

    #[cfg(feature = "xwayland")]
    pub fn finish_x11_fullscreen_presentation(&mut self, surface: &WlSurface) -> bool {
        let root =
            crate::xwayland::pointer_constraint_proxy_authority(&self.wayland.space, surface)
                .unwrap_or_else(|| crate::wayland::compositor::root_surface(surface));
        let window = self
            .wayland
            .space
            .elements()
            .find(|window| {
                window.x11_surface().is_some()
                    && window
                        .wl_surface()
                        .is_some_and(|candidate| candidate.as_ref() == &root)
            })
            .cloned()
            .or_else(|| {
                self.nodes
                    .records()
                    .find(|record| {
                        record.window.x11_surface().is_some()
                            && record
                                .window
                                .wl_surface()
                                .is_some_and(|candidate| candidate.as_ref() == &root)
                    })
                    .map(|record| record.window.clone())
            });
        let Some(window) = window else {
            return false;
        };
        if !self
            .fullscreen
            .finish_external_presentation(&mut self.wayland, &window)
        {
            return false;
        }
        // Forced completion removes the timeline before normal frame cleanup
        // can observe it. Retire its snapshots now, in the same event-loop
        // turn, so a later transition cannot inherit or lose its textures.
        self.render.fullscreen_textures.remove(&root);
        true
    }

    #[cfg(not(feature = "xwayland"))]
    pub fn finish_x11_fullscreen_presentation(&mut self, _surface: &WlSurface) -> bool {
        false
    }
}
