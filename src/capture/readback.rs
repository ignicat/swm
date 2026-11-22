use std::error::Error;
use std::io;
use std::path::Path;

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::sync::SyncPoint;
use smithay::backend::renderer::utils::draw_render_elements;
use smithay::backend::renderer::{Bind, Color32F, ExportMem, Frame, Offscreen, Renderer, Texture};
use smithay::output::Output;
use smithay::utils::{Buffer, Logical, Physical, Rectangle, Scale, Transform};
use smithay::wayland::seat::WaylandFocus;

use crate::render::{
    self, CursorContext, DesktopContext, FrameContext, OverlayContext, RenderRequest, VisualContext,
};
use crate::session::{Session, SessionDriver};

pub(crate) fn capture_source_pixels<D: SessionDriver>(
    session: &mut Session<D>,
    source: &swm_ipc::CaptureSource,
    show_cursor: bool,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if session.session_lock.active() {
        return Err(io::Error::other("session is locked").into());
    }
    match source {
        swm_ipc::CaptureSource::Monitor {
            name,
            width,
            height,
            ..
        } => with_monitor_scene(
            session,
            name,
            *width,
            *height,
            show_cursor,
            true,
            |renderer, elements, size| {
                capture_elements(
                    renderer,
                    Fourcc::Abgr8888,
                    size,
                    elements,
                    render::CLEAR_COLOR,
                )
            },
        ),
        swm_ipc::CaptureSource::Window {
            window_id,
            width,
            height,
        } => {
            let window = resolve_source_window(&session.wayland.space, &session.nodes, *window_id)?;
            validate_source_window_size(session, &window, *width, *height)?;
            if window.wl_surface().is_some_and(|surface| {
                session
                    .window_rules
                    .applied(surface.as_ref())
                    .block_screencast
            }) {
                let len = (*width as usize)
                    .checked_mul(*height as usize)
                    .and_then(|v| v.checked_mul(4))
                    .ok_or_else(|| io::Error::other("window frame dimensions overflow"))?;
                return Ok(vec![0; len]);
            }
            capture_decorated_window_pixels(session, &window)
        }
    }
}

pub(crate) fn render_source_dmabuf<D: SessionDriver>(
    session: &mut Session<D>,
    source: &swm_ipc::CaptureSource,
    show_cursor: bool,
    dmabuf: &mut Dmabuf,
) -> Result<SyncPoint, Box<dyn Error>> {
    if session.session_lock.active() {
        return Err(io::Error::other("session is locked").into());
    }
    if !session.driver.import_dmabuf(dmabuf) {
        return Err(io::Error::other("renderer rejected capture DMA-BUF").into());
    }
    match source {
        swm_ipc::CaptureSource::Monitor {
            name,
            width,
            height,
            ..
        } => with_monitor_scene(
            session,
            name,
            *width,
            *height,
            show_cursor,
            true,
            |renderer, elements, size| {
                render_elements_to_dmabuf(renderer, dmabuf, size, elements, render::CLEAR_COLOR)
            },
        ),
        swm_ipc::CaptureSource::Window {
            window_id,
            width,
            height,
        } => {
            let window = resolve_source_window(&session.wayland.space, &session.nodes, *window_id)?;
            validate_source_window_size(session, &window, *width, *height)?;
            if window.wl_surface().is_some_and(|surface| {
                session
                    .window_rules
                    .applied(surface.as_ref())
                    .block_screencast
            }) {
                return session.driver.with_renderer(|renderer| {
                    render_elements_to_dmabuf::<render::scene::SceneElement>(
                        renderer,
                        dmabuf,
                        (*width, *height).into(),
                        &[],
                        Color32F::BLACK,
                    )
                });
            }
            with_decorated_window(session, &window, |renderer, element, size| {
                render_elements_to_dmabuf(renderer, dmabuf, size, &[element], Color32F::TRANSPARENT)
            })
        }
    }
}

pub(crate) fn with_monitor_scene<D, T>(
    session: &mut Session<D>,
    name: &str,
    width: i32,
    height: i32,
    show_cursor: bool,
    block_screencast: bool,
    consume: impl FnOnce(
        &mut GlesRenderer,
        &[render::scene::SceneElement],
        smithay::utils::Size<i32, Physical>,
    ) -> Result<T, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>>
where
    D: SessionDriver,
{
    let output = session
        .wayland
        .space
        .outputs()
        .find(|output| output.name() == name)
        .cloned()
        .ok_or_else(|| io::Error::other(format!("unknown output {name}")))?;
    let geometry = session
        .wayland
        .space
        .output_geometry(&output)
        .ok_or_else(|| io::Error::other(format!("output {name} has no geometry")))?;
    if render::output_physical_size(&output) != (width, height).into() {
        return Err(io::Error::other("selected output size changed").into());
    }
    let primary = session.driver.primary_output().clone();
    let target_time = crate::frame_clock::monotonic_now();
    let node_grab_active = matches!(
        session.interactions.grab(),
        crate::input::grab::Grab::PendingNode { .. } | crate::input::grab::Grab::MoveNode { .. }
    );
    let driver = &mut session.driver;
    let cursor = &session.cursor;
    let pointer_position = session.pointer.position();
    let wayland = &session.wayland;
    let decorations = &session.settings.decorations;
    let cameras = &session.cameras;
    let window_animations = &session.window_animations;
    let fullscreen = &session.fullscreen;
    let maximize = &session.maximize;
    let nodes = &session.nodes;
    let window_rules = &session.window_rules;
    let overlays = &session.shell.overlays;
    let overlay_config = &session.settings.overlays;
    let resources = &mut session.render;
    let session_lock = &session.session_lock;
    driver.with_renderer(|renderer| {
        let elements = render::scene::build_with_privacy(
            renderer,
            &output,
            &primary,
            geometry,
            RenderRequest {
                frame: FrameContext {
                    target_presentation_time: target_time,
                    vrr_auto_eligible: false,
                    force_full_repaint: false,
                    clear: render::CLEAR_COLOR,
                },
                desktop: DesktopContext {
                    fifo: &wayland.fifo,
                    session_lock,
                    space: &wayland.space,
                    focused: wayland.focused_window.as_ref(),
                    cameras,
                    window_animations,
                    fullscreen,
                    maximize,
                    nodes,
                    window_rules,
                    screencast: &session.capture.screencast,
                    layer_rules: &session.settings.layer_rules,
                    node_grab_active,
                },
                cursor: CursorContext {
                    cursor,
                    dnd_icon: show_cursor.then_some(wayland.dnd_icon.as_ref()).flatten(),
                    cursor_position: pointer_position,
                    show_cursor,
                    cursor_override: None,
                },
                overlays: OverlayContext {
                    capture_overlay: crate::capture::CaptureOverlay::None,
                    focus_cycle: &session.shell.focus_cycle,
                    overlays,
                    overlay_config,
                },
                visuals: VisualContext {
                    decorations,
                    pins: &session.settings.field.pins,
                    background: &session.settings.background,
                    background_base: session.config_source.path().and_then(Path::parent),
                },
                resources: crate::render::resources::RenderResources::from(resources),
            },
            block_screencast,
        )?;
        consume(renderer, &elements, render::output_physical_size(&output))
    })
}

pub(crate) fn capture_monitor_region_pixels<D: SessionDriver>(
    session: &mut Session<D>,
    output: &Output,
    region: Rectangle<i32, Physical>,
    show_cursor: bool,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if session.wayland.space.output_geometry(output).is_none() {
        return Err(io::Error::other("screencopy output is not mapped").into());
    }
    with_monitor_scene(
        session,
        &output.name(),
        render::output_physical_size(output).w,
        render::output_physical_size(output).h,
        show_cursor,
        false,
        |renderer, elements, _| {
            let offset = region.loc.upscale(-1);
            let relocated = elements
                .iter()
                .map(|element| {
                    RelocateRenderElement::from_element(element, offset, Relocate::Relative)
                })
                .collect::<Vec<_>>();
            capture_elements(
                renderer,
                Fourcc::Xrgb8888,
                region.size,
                &relocated,
                render::CLEAR_COLOR,
            )
        },
    )
}

pub(crate) fn render_monitor_region_dmabuf<D: SessionDriver>(
    session: &mut Session<D>,
    output: &Output,
    region: Rectangle<i32, Physical>,
    show_cursor: bool,
    dmabuf: &mut Dmabuf,
) -> Result<SyncPoint, Box<dyn Error>> {
    if session.wayland.space.output_geometry(output).is_none() {
        return Err(io::Error::other("screencopy output is not mapped").into());
    }
    session.driver.import_dmabuf(dmabuf);
    with_monitor_scene(
        session,
        &output.name(),
        render::output_physical_size(output).w,
        render::output_physical_size(output).h,
        show_cursor,
        false,
        |renderer, elements, _| {
            let offset = region.loc.upscale(-1);
            let relocated = elements
                .iter()
                .map(|element| {
                    RelocateRenderElement::from_element(element, offset, Relocate::Relative)
                })
                .collect::<Vec<_>>();
            render_elements_to_dmabuf(
                renderer,
                dmabuf,
                region.size,
                &relocated,
                render::CLEAR_COLOR,
            )
        },
    )
}

pub(crate) fn resolve_source_window(
    space: &smithay::desktop::Space<smithay::desktop::Window>,
    nodes: &crate::nodes::NodesState,
    window_id: u64,
) -> Result<smithay::desktop::Window, Box<dyn Error>> {
    let window = space
        .elements()
        .find(|window| {
            window.wl_surface().is_some_and(|surface| {
                nodes
                    .id_for_surface(surface.as_ref())
                    .is_some_and(|id| id.as_u64() == window_id)
            })
        })
        .cloned()
        .ok_or_else(|| io::Error::other("selected window is no longer mapped"))?;
    Ok(window)
}

fn validate_source_window_size<D: SessionDriver>(
    session: &Session<D>,
    window: &smithay::desktop::Window,
    width: i32,
    height: i32,
) -> Result<(), Box<dyn Error>> {
    if session.window_capture_context().size(window) != (width, height).into() {
        return Err(io::Error::other("selected window size changed").into());
    }
    Ok(())
}

fn capture_decorated_window_pixels<D: SessionDriver>(
    session: &mut Session<D>,
    window: &smithay::desktop::Window,
) -> Result<Vec<u8>, Box<dyn Error>> {
    with_decorated_window(session, window, |renderer, element, size| {
        capture_elements(
            renderer,
            Fourcc::Abgr8888,
            size,
            &[element],
            Color32F::TRANSPARENT,
        )
    })
}

fn with_decorated_window<D, T>(
    session: &mut Session<D>,
    window: &smithay::desktop::Window,
    consume: impl FnOnce(
        &mut GlesRenderer,
        smithay::backend::renderer::element::texture::TextureRenderElement<GlesTexture>,
        smithay::utils::Size<i32, Physical>,
    ) -> Result<T, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>>
where
    D: SessionDriver,
{
    let chrome_visible = session.window_capture_context().chrome_visible(window);
    let focused = window
        .wl_surface()
        .is_some_and(|surface| session.wayland.focused_window.as_ref() == Some(surface.as_ref()));
    let draw_border = window
        .wl_surface()
        .is_none_or(|surface| session.window_rules.applied(surface.as_ref()).border != Some(false));
    let scale = session.window_capture_context().display_scale(window);
    let decorations = &session.settings.decorations;
    session.driver.with_renderer(|renderer| {
        let texture = crate::render::window_texture::capture_decorated_at_scale(
            renderer,
            window,
            None,
            scale,
            crate::render::window_texture::CaptureChrome {
                decorations,
                focused,
                visible: chrome_visible,
                draw_border,
            },
        )?;
        let size = texture
            .texture
            .size()
            .to_logical(1, Transform::Normal)
            .to_physical(1);
        let element = texture.render_element(Id::new(), Rectangle::from_size(size), 1.0);
        consume(renderer, element, size)
    })
}

pub(crate) fn render_elements_to_dmabuf<E>(
    renderer: &mut GlesRenderer,
    dmabuf: &mut Dmabuf,
    size: smithay::utils::Size<i32, Physical>,
    elements: &[E],
    clear: Color32F,
) -> Result<SyncPoint, Box<dyn Error>>
where
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
{
    let damage = Rectangle::<i32, Physical>::from_size(size);
    let mut target = renderer.bind(dmabuf)?;
    let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
    frame.clear(clear, &[damage])?;
    draw_render_elements(&mut frame, 1.0, elements, &[damage])?;
    Ok(frame.finish()?)
}

pub(crate) fn capture_cursor_surface_tree(
    renderer: &mut GlesRenderer,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    snapshot: Option<&crate::cursor::CursorSurfaceSnapshot>,
    geometry: Rectangle<i32, Logical>,
    scale: f64,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let size = geometry.size.to_f64().to_physical(scale).to_i32_round();
    let location = smithay::utils::Point::from((-geometry.loc.x, -geometry.loc.y))
        .to_f64()
        .to_physical(scale)
        .to_i32_round();
    let elements = crate::cursor::render::surface_elements(
        renderer,
        surface,
        snapshot,
        location,
        Scale::from(scale),
        Kind::Cursor,
    )?;
    if elements.is_empty() {
        return Err(io::Error::other("cursor surface tree is empty").into());
    }
    let elements = elements
        .into_iter()
        .map(|element| {
            crate::render::display_scale::element(
                crate::render::scene::SceneElement::Cursor(element),
                scale,
                true,
            )
        })
        .collect::<Vec<_>>();
    capture_elements(
        renderer,
        Fourcc::Abgr8888,
        size,
        &elements,
        Color32F::TRANSPARENT,
    )
}

fn capture_elements<E>(
    renderer: &mut GlesRenderer,
    format: Fourcc,
    size: smithay::utils::Size<i32, Physical>,
    elements: &[E],
    clear: Color32F,
) -> Result<Vec<u8>, Box<dyn Error>>
where
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
{
    let buffer_size = smithay::utils::Size::<i32, Buffer>::from((size.w, size.h));
    let mut texture =
        <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(renderer, format, buffer_size)?;
    let damage = Rectangle::<i32, Physical>::from_size(size);
    {
        let mut target = renderer.bind(&mut texture)?;
        let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
        frame.clear(clear, &[damage])?;
        draw_render_elements(&mut frame, 1.0, elements, &[damage])?;
        let _ = frame.finish()?;
    }
    let mapping = renderer.copy_texture(
        &texture,
        Rectangle::<i32, Buffer>::from_size(buffer_size),
        format,
    )?;
    Ok(renderer.map_texture(&mapping)?.to_vec())
}
