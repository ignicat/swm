use std::time::Instant;

use calloop::{EventLoop, LoopHandle};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self as smithay_winit, WinitEvent};
use smithay::input::{Seat, SeatState};
use smithay::reexports::wayland_server::Display;
use smithay::reexports::winit::dpi::LogicalSize;
use smithay::reexports::winit::window::WindowAttributes;
use smithay::wayland::seat::WaylandFocus;

use crate::backend::winit::WinitBackend;
use crate::backend::{self, Renderable};
use crate::input::keybinds::BackendKind;
use crate::render::{
    self, CursorContext, DesktopContext, FrameContext, OverlayContext, RenderRequest, VisualContext,
};

use super::{Session, state::OutputDriver};

#[cfg(test)]
mod test_support;

struct WinitDriver {
    backend: WinitBackend,
    loop_handle: LoopHandle<'static, App>,
    exit: bool,
    last_camera_tick: Instant,
    frame_callback_sequence: u32,
    #[cfg(test)]
    extra_outputs: Vec<smithay::output::Output>,
    #[cfg(test)]
    keyboard_led_update: Option<smithay::input::keyboard::LedState>,
    #[cfg(test)]
    keyboard_led_update_count: u32,
}

impl crate::ipc::OutputInfoSource for WinitDriver {
    fn output_info(&self) -> Vec<swm_ipc::OutputInfo> {
        let outputs = crate::ipc::OutputInfoSource::output_info(&self.backend);
        #[cfg(test)]
        let outputs = outputs
            .into_iter()
            .chain(self.extra_outputs.iter().map(test_support::output_info))
            .collect();
        outputs
    }
}

impl super::RenderDriver for WinitDriver {
    fn dmabuf_capabilities(&mut self) -> crate::backend::dmabuf::DmabufCapabilities {
        self.backend.dmabuf_capabilities()
    }

    fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        self.backend.import_dmabuf(dmabuf)
    }

    fn dmabuf_feedback(
        &self,
        _output: &smithay::output::Output,
    ) -> Option<&crate::backend::dmabuf::SurfaceDmabufFeedback> {
        None
    }

    fn request_redraw(&mut self, _output: Option<&smithay::output::Output>) {
        self.backend.request_redraw();
    }

    fn with_renderer<T>(&mut self, f: impl FnOnce(&mut GlesRenderer) -> T) -> T {
        f(self.backend.renderer())
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
        client: smithay::reexports::wayland_server::Client,
        display: smithay::reexports::wayland_server::DisplayHandle,
        error: smithay::reexports::wayland_server::backend::protocol::ProtocolError,
    ) {
        self.loop_handle
            .insert_idle(move |_| client.kill(&display, error));
    }
}

impl super::OutputDriver for WinitDriver {
    fn primary_output(&self) -> &smithay::output::Output {
        self.backend.output()
    }

    fn frame_callback_sequence(&self, _output: &smithay::output::Output) -> u32 {
        self.frame_callback_sequence
    }

    fn output_states(&self) -> Vec<super::output::OutputState> {
        let output = self.backend.output();
        let states = vec![super::output::OutputState {
            output: output.clone(),
            enabled: true,
            mode: output
                .current_mode()
                .expect("winit output always has a current mode"),
            location: output.current_location(),
            transform: output.current_transform(),
            scale: output.current_scale().fractional_scale(),
            adaptive_sync: false,
            adaptive_sync_supported: false,
        }];
        #[cfg(test)]
        let states = states
            .into_iter()
            .chain(self.extra_outputs.iter().map(test_support::output_state))
            .collect();
        states
    }

    fn test_output_configuration(
        &mut self,
        configuration: &[super::output::OutputConfiguration],
    ) -> Result<(), String> {
        let current = self.output_states();
        super::output::validate_complete_configuration(&current, configuration)?;
        #[cfg(test)]
        if !self.extra_outputs.is_empty() {
            for requested in configuration {
                let state = current
                    .iter()
                    .find(|state| state.output == requested.output)
                    .unwrap();
                if !requested.enabled
                    || requested.mode != state.mode
                    || requested.transform != state.transform
                    || requested.adaptive_sync
                {
                    return Err("test outputs support location and scale changes only".into());
                }
            }
            return Ok(());
        }
        let requested = &configuration[0];
        let state = &current[0];
        if !requested.enabled
            || requested.mode != state.mode
            || requested.location != state.location
            || requested.transform != state.transform
            || requested.adaptive_sync
        {
            return Err("the nested output is controlled by the host compositor".into());
        }
        Ok(())
    }

    fn apply_output_configuration(
        &mut self,
        configuration: &[super::output::OutputConfiguration],
    ) -> Result<Vec<super::output::OutputChange>, String> {
        self.test_output_configuration(configuration)?;
        #[cfg(test)]
        if !self.extra_outputs.is_empty() {
            let before = self.output_states();
            for requested in configuration {
                requested.output.change_current_state(
                    None,
                    None,
                    Some(smithay::output::Scale::Fractional(requested.scale)),
                    Some(requested.location),
                );
            }
            let after = self.output_states();
            return Ok(before
                .into_iter()
                .zip(after)
                .filter(|(a, b)| a.location != b.location || a.scale != b.scale)
                .map(|(before, after)| super::output::OutputChange { before, after })
                .collect());
        }
        let before = self.output_states().remove(0);
        let scale =
            swm_config::output::normalize_scale(configuration[0].scale).expect("validated scale");
        if before.scale == scale {
            return Ok(Vec::new());
        }
        self.backend.set_display_scale(scale);
        let after = self.output_states().remove(0);
        Ok(vec![super::output::OutputChange { before, after }])
    }
}

impl super::SessionDriver for WinitDriver {
    const BACKEND_KIND: BackendKind = BackendKind::Winit;

    #[cfg(test)]
    fn update_keyboard_leds(&mut self, state: smithay::input::keyboard::LedState) {
        self.keyboard_led_update = Some(state);
        self.keyboard_led_update_count += 1;
    }

    fn stop(&mut self) {
        self.exit = true;
    }
}

type App = Session<WinitDriver>;

fn apply_runtime_config(app: &mut App, reload: crate::config::ConfigReload) {
    app.apply_reload(reload, |app, config| {
        let scale = config
            .outputs
            .iter()
            .find(|output| output.name == "winit")
            .map_or(1.0, |output| output.scale);
        let state = app.driver.output_states().remove(0);
        let requested = super::output::OutputConfiguration {
            output: state.output,
            enabled: true,
            mode: state.mode,
            location: state.location,
            transform: state.transform,
            scale,
            adaptive_sync: false,
        };
        if let Err(error) = app.apply_wayland_output_configuration(&[requested]) {
            eventline::warn!("nested display scale: {error}");
        }
    });
}

/// Runs the nested (winit) session - a real window on the host desktop,
/// standing in for real hardware output. Used when we're not the ones
/// actually driving a display (see `detect_nested_session` in `main.rs`) or
/// when `--winit` is passed explicitly.
pub fn run(explicit_config_path: Option<std::path::PathBuf>) {
    let initial = crate::config::load_initial(explicit_config_path);
    let config_path = initial.path;
    let runtime_config = initial.config;
    let window_attributes = WindowAttributes::default()
        .with_surface_size(LogicalSize::new(1280.0, 800.0))
        .with_title("swm");

    let (backend, winit_source) =
        smithay_winit::init_from_attributes::<GlesRenderer>(window_attributes)
            .expect("failed to initialize winit backend");

    // Winit only guarantees an initial RedrawRequested on some platforms -
    // request one explicitly so the first frame doesn't depend on that.
    backend.window().request_redraw();

    let mut event_loop: EventLoop<App> = EventLoop::try_new().expect("failed to create event loop");

    let display: Display<App> = Display::new().expect("failed to create wayland display");
    let dh = display.handle();

    let mut seat_state = SeatState::new();
    let mut seat: Seat<App> = seat_state.new_wl_seat(&dh, "seat0");
    let applied_keyboard = crate::input::config::add_keyboard(&mut seat, &runtime_config.input)
        .expect("failed to advertise keyboard capability on the wl_seat");
    seat.add_pointer();

    let winit_backend = WinitBackend::new(backend);
    let scale = runtime_config
        .outputs
        .iter()
        .find(|output| output.name == "winit")
        .map_or(1.0, |output| output.scale);
    winit_backend.set_display_scale(scale);

    let output_size = winit_backend.logical_size();
    let mut cameras = crate::presentation::camera::OutputCameras::default();
    cameras.insert(winit_backend.output().name(), output_size);

    let mut driver = WinitDriver {
        backend: winit_backend,
        loop_handle: event_loop.handle(),
        exit: false,
        last_camera_tick: Instant::now(),
        frame_callback_sequence: 0,
        #[cfg(test)]
        extra_outputs: Vec::new(),
        #[cfg(test)]
        keyboard_led_update: None,
        #[cfg(test)]
        keyboard_led_update_count: 0,
    };
    let mut wayland = App::create_wayland_state(dh.clone(), &mut driver);
    wayland.ensure_output_global::<App>(driver.backend.output());
    let idle_notifier_state =
        smithay::wayland::idle_notify::IdleNotifierState::new(&dh, event_loop.handle());
    let _presentation_state = smithay::wayland::presentation::PresentationState::new::<App>(
        &dh,
        smithay::utils::Clock::<smithay::utils::Monotonic>::new().id() as u32,
    );
    let session_lock = crate::wayland::session_lock::State::new::<WinitDriver>(&dh);
    let xwayland = crate::xwayland::State::<WinitDriver>::new(&dh, event_loop.handle());
    let mut applied_input = runtime_config.input.clone();
    applied_input.keyboard = applied_keyboard;
    let system_color_scheme = crate::appearance::current_color_scheme();
    let keyboard_monitor = {
        #[cfg(all(feature = "dbus", test))]
        {
            if std::env::var_os("SWM_TEST_ACCESSIBILITY").is_some() {
                crate::accessibility::KeyboardMonitorService::start().ok()
            } else {
                None
            }
        }
        #[cfg(not(all(feature = "dbus", test)))]
        {
            None
        }
    };
    let mut app = App::new(super::SessionInit {
        driver,
        runtime_config: &runtime_config,
        applied_input,
        backend_kind: BackendKind::Winit,
        loop_handle: event_loop.handle(),
        autostart: super::autostart::Autostart::disabled(),
        track_idle: false,
        wayland,
        seat_state,
        seat,
        idle_notifier_state,
        presentation_state: _presentation_state,
        drm_syncobj_state: None,
        session_lock,
        config_path,
        diagnostic: initial.diagnostic,
        system_color_scheme,
        cameras,
        keyboard_monitor,
        xwayland,
    });
    app.wayland
        .space
        .map_output(app.driver.backend.output(), (0, 0));
    app.initialize_config_notification();

    let socket_name = super::protocol::init_wayland_listener(display, &mut event_loop);
    app.wayland_display = Some(socket_name.clone());
    eventline::info!("swm (winit) starting, WAYLAND_DISPLAY={socket_name:?}");
    if let Err(err) = crate::xwayland::start(&event_loop.handle(), &mut app, false) {
        eventline::warn!("xwayland: unavailable: {err}");
    }

    app.install_io_services(&event_loop.handle());
    app.install_watchers_and_timers(&event_loop.handle(), apply_runtime_config);

    #[cfg(test)]
    test_support::install(&mut app, &event_loop.handle());

    event_loop
        .handle()
        .insert_source(winit_source, move |event, _, app| match event {
            WinitEvent::CloseRequested => {
                app.show_exit_confirmation();
            }
            WinitEvent::Redraw => {
                let now = Instant::now();
                let target_presentation_time = crate::frame_clock::monotonic_now();
                let edge_pan_output =
                    super::input::tick_grabbed_window_edge_pan(app, target_presentation_time);
                let physics_animating = crate::nodes::tick_physics(app, target_presentation_time);
                let dt = now
                    .duration_since(app.driver.last_camera_tick)
                    .as_secs_f32();
                app.driver.last_camera_tick = now;
                let output = app.driver.backend.output().clone();
                let output_name = output.name();
                let edge_pan_animating = edge_pan_output.as_deref() == Some(output_name.as_str());
                crate::window::enforce_dialog_stacking(&mut app.wayland);
                let view_before = app.cameras.view(&output_name);
                let fullscreen_camera_changed =
                    app.sync_fullscreen_camera(&output, target_presentation_time);
                let zoom_scale_before =
                    app.cameras.get(&output_name).map(crate::input::zoom::scale);
                let mut camera_animating = false;
                for (camera, fullscreen_returning) in app.cameras.iter_for_tick() {
                    camera_animating |= crate::input::zoom::tick_camera(
                        camera,
                        fullscreen_returning,
                        &app.settings.zoom,
                        app.settings.input.gestures.pan_decay_rate,
                        dt,
                    )
                    .1;
                }
                let zoom_scale_after = app.cameras.get(&output_name).map(crate::input::zoom::scale);
                if let (Some(before), Some(after)) = (zoom_scale_before, zoom_scale_after)
                    && before != after
                {
                    if after < before {
                        crate::nodes::reconcile_landmarks_for_zoom(app, &output_name, after);
                    } else {
                        crate::nodes::restore_landmarks_for_zoom(app, &output_name, after);
                    }
                    app.shell.overlays.show_zoom_indicator(
                        &output_name,
                        after,
                        &app.settings.overlays.zoom_indicator,
                        target_presentation_time,
                    );
                }
                if view_before != app.cameras.view(&output_name) {
                    super::pointer::update_client_state(
                        app,
                        app.start_time.elapsed().as_millis() as u32,
                    );
                }

                let window_animating = app.wayland.space.elements().any(|window| {
                    window.wl_surface().is_some_and(|surface| {
                        app.window_animations
                            .is_animating(surface.as_ref(), target_presentation_time)
                    })
                });
                let move_animating = app.wayland.space.elements().any(|window| {
                    window.wl_surface().is_some_and(|surface| {
                        app.window_animations
                            .is_moving(surface.as_ref(), target_presentation_time)
                    })
                });
                let fullscreen_animating = app.fullscreen.is_animating_on_output_matching(
                    &output,
                    target_presentation_time,
                    |_| true,
                );
                let maximize_animating = app
                    .maximize
                    .is_animating_on_output(&output, target_presentation_time);
                let closing_animating = app
                    .render
                    .window_close_animations
                    .is_animating_on_output(&output, target_presentation_time);
                let node_animating = app
                    .nodes
                    .is_animating_on_output(&output.name(), target_presentation_time);
                let focus_cycle_animating = app.shell.focus_cycle.tick(target_presentation_time);
                let background_animating =
                    app.background_animates_on_output(&output, target_presentation_time);
                let pointer_time = app.start_time.elapsed().as_millis() as u32;
                if fullscreen_animating || maximize_animating || move_animating {
                    super::pointer::update_client_state(app, pointer_time);
                }
                super::trace::snapshot(app);
                let position = app.pointer.position();
                let show_cursor = super::pointer::cursor_visible(app);
                let cursor_override = super::pointer::cursor_override(app);
                crate::cursor::surface::refresh_outputs(&app.cursor, &app.wayland.space, position);
                crate::wayland::dnd::refresh_outputs(
                    app.wayland.dnd_icon.as_ref(),
                    &app.wayland.space,
                    position,
                );
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
                    .schedule_animation(&output, next_cursor_frame);
                let outcome = app.driver.backend.render(
                    &output,
                    RenderRequest {
                        frame: FrameContext {
                            target_presentation_time,
                            vrr_auto_eligible: false,
                            force_full_repaint: false,
                            clear: render::CLEAR_COLOR,
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
                            cursor_position: position,
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
                            background_base: app
                                .config_source
                                .path()
                                .and_then(std::path::Path::parent),
                        },
                        resources: crate::render::resources::RenderResources::from(&mut app.render),
                    },
                );
                let (submitted, element_states) = match outcome {
                    Ok(outcome) if outcome.status() == backend::RenderStatus::Submitted => {
                        super::schedule_fifo_barriers(
                            app,
                            &app.driver.loop_handle.clone(),
                            &output,
                            outcome.fifo_barriers.clone(),
                        );
                        if let Some(generation) = app.session_lock.frame_generation() {
                            app.session_lock.presented(&output, generation);
                        }
                        let element_states = outcome.element_states().cloned();
                        (true, element_states)
                    }
                    Ok(outcome) => (false, outcome.element_states().cloned()),
                    Err(err) => {
                        eventline::error!("render failed: {err}");
                        (false, None)
                    }
                };
                if let Some(element_states) = element_states {
                    app.update_idle_inhibit_visibility(&output, &element_states);
                    app.service_image_capture(&output);
                }
                if submitted {
                    app.service_screencopy(&output);
                }

                // Lets clients know their last commit was actually
                // presented, so they schedule their next frame - without
                // this a client's redraw loop just stalls forever.
                let output = app.driver.backend.output().clone();
                app.driver.frame_callback_sequence =
                    app.driver.frame_callback_sequence.wrapping_add(1);
                let frame_callback_sequence = app.driver.frame_callback_sequence;
                app.send_output_frame_callbacks(
                    &output,
                    frame_callback_sequence,
                    submitted,
                    false,
                    target_presentation_time,
                );
                let transition_cleanup = app.finish_frame_transitions(target_presentation_time);

                let overlay_animating = app
                    .shell
                    .overlays
                    .animating_on_output(&output.name(), target_presentation_time);
                #[cfg(feature = "xwayland")]
                let has_external_commit_wait = app.fullscreen.has_external_commit_wait();
                #[cfg(not(feature = "xwayland"))]
                let has_external_commit_wait = false;
                if camera_animating
                    || edge_pan_animating
                    || fullscreen_camera_changed
                    || window_animating
                    || closing_animating
                    || node_animating
                    || focus_cycle_animating
                    || background_animating
                    || overlay_animating
                    || fullscreen_animating
                    || maximize_animating
                    || physics_animating
                    || app.render.node_renderer.has_pending_icons()
                    || transition_cleanup
                    || has_external_commit_wait
                {
                    app.request_redraw();
                }
            }
            WinitEvent::Resized { .. } => {
                // `WinitGraphicsBackend::bind()` resizes the EGL surface when `window_size()` changes.
                app.driver.backend.update_output_mode();
                crate::wayland::session_lock::configure_surfaces(app);
                smithay::desktop::layer_map_for_output(app.driver.backend.output()).arrange();
                // Snap zoom and pan back to rest across host window resizes.
                let output_size = app.driver.backend.logical_size();
                app.cameras
                    .reset(app.driver.backend.output().name(), output_size);
                let external = app
                    .fullscreen
                    .reconfigure_output(&app.wayland, app.driver.backend.output());
                crate::xwayland::reconfigure_fullscreen(external);
                super::pointer::update_client_state(
                    app,
                    app.start_time.elapsed().as_millis() as u32,
                );
                app.notify_output_management();
                app.request_redraw();
            }
            WinitEvent::Input(event) => {
                // Test sessions open real windows on the developer's desktop.
                // They receive input only from their private control socket
                // or their own clients; host pointer motion and keys must
                // never steer a test, with or without a control socket.
                if cfg!(test) {
                    return;
                }
                super::input::handle(app, &event, &socket_name);
            }
            _ => {}
        })
        .expect("failed to insert winit event source");

    // Only an explicitly isolated test session should publish a second desktop
    // accessibility tree; ordinary nested sessions share the host bus.
    #[cfg(all(feature = "dbus", test))]
    let mut accessibility_tree = if std::env::var_os("SWM_TEST_ACCESSIBILITY").is_some() {
        crate::accessibility::AccessibilityTree::start().ok()
    } else {
        None
    };
    while !app.driver.exit {
        event_loop
            .dispatch(None, &mut app)
            .expect("event loop dispatch failed");
        #[cfg(all(feature = "dbus", test))]
        if let Some(tree) = &mut accessibility_tree {
            tree.refresh(&app);
        }
        crate::ipc::publish_api_events(&mut app);
        super::foreign_toplevel::sync(&mut app);
        let _ = app.wayland.display_handle.flush_clients();
    }
}
