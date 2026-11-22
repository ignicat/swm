use smithay::desktop::{Space, Window};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle};
use smithay::wayland::compositor::{SubsurfaceCachedState, get_parent, with_states};
use smithay::wayland::seat::WaylandFocus;

use crate::presentation::camera::OutputCameras;

/// The presentation geometry shared by scene construction and input routing.
///
/// Keeping this in the input-independent presentation module makes the
/// compositor render and invert the exact same camera, fullscreen, and
/// opening-animation rectangles.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowVisualState {
    pub(crate) source_geometry: Rectangle<i32, Logical>,
    pub(crate) camera_rect: Rectangle<i32, Physical>,
    pub(crate) presentation_rect: Rectangle<i32, Physical>,
    pub(crate) animated_rect: Rectangle<i32, Physical>,
    pub(crate) opening_alpha: f32,
    opening_progress: f32,
    opening_clamped_progress: f32,
    opening_random_seed: f32,
    shader_pixels: bool,
    pub(crate) fullscreen: Option<crate::wayland::fullscreen::FullscreenPresentation>,
    pub(crate) maximize: Option<crate::presentation::maximize::FieldMaximizePresentation>,
    pub(crate) camera_center: Point<f32, Physical>,
    pub(crate) zoom_scale: f32,
    pub(crate) presentation_space: PresentationSpace,
    pub(crate) above_field_landmarks: bool,
    inherited_presentation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresentationSpace {
    Field,
    OutputLocal,
}

fn output_local_or_field_rect(
    output_local: Option<Rectangle<i32, Physical>>,
    world: Rectangle<i32, Logical>,
    camera_center: Point<f32, Physical>,
    output_size: smithay::utils::Size<i32, Physical>,
    zoom_scale: f32,
) -> Rectangle<i32, Physical> {
    output_local.unwrap_or_else(|| {
        crate::presentation::camera::camera_rect(
            world.to_physical(1),
            camera_center,
            output_size,
            zoom_scale,
        )
    })
}

impl WindowVisualState {
    pub(crate) fn maps_from_source(self) -> bool {
        self.inherited_presentation || self.fullscreen.is_some() || self.maximize.is_some()
    }

    pub(crate) fn shader_pixels(self) -> bool {
        self.shader_pixels
    }

    pub(crate) fn opening_progress(self) -> f32 {
        self.opening_progress
    }

    pub(crate) fn opening_clamped_progress(self) -> f32 {
        self.opening_clamped_progress
    }

    pub(crate) fn opening_random_seed(self) -> f32 {
        self.opening_random_seed
    }
}

fn output_local_zoom_scale(space: PresentationSpace, view_scale: f32) -> f32 {
    match space {
        PresentationSpace::OutputLocal => 1.0,
        PresentationSpace::Field => view_scale,
    }
}

/// Chrome scale for a fullscreen/maximize crossfade.
///
/// Field destinations start at the camera-scaled on-screen rect; using native
/// 1.0 there draws a full-size border on a still-zoomed window, and using
/// the live camera fights the output-local dest. Scale chrome by dest vs the
/// native size at the same progress.
fn presentation_display_scale(
    animated: smithay::utils::Size<i32, Physical>,
    windowed_native: smithay::utils::Size<i32, Physical>,
    target_native: smithay::utils::Size<i32, Physical>,
    progress: f64,
) -> f32 {
    let native = (f64::from(windowed_native.h)
        + f64::from(target_native.h - windowed_native.h) * progress)
        .round()
        .max(1.0);
    animated.h as f32 / native as f32
}

fn inherited_visual_rects(
    source: Rectangle<i32, Physical>,
    owner_source: Rectangle<i32, Physical>,
    owner_camera: Rectangle<i32, Physical>,
    owner_presentation: Rectangle<i32, Physical>,
    owner_animated: Rectangle<i32, Physical>,
) -> (
    Rectangle<i32, Physical>,
    Rectangle<i32, Physical>,
    Rectangle<i32, Physical>,
) {
    let camera = crate::animation::map_rect(source, owner_source, owner_camera);
    let presentation = crate::animation::map_rect(source, owner_source, owner_presentation);
    let animated = crate::animation::map_rect(presentation, owner_presentation, owner_animated);
    (camera, presentation, animated)
}

/// Shared inputs for resolving a window's on-screen presentation.
/// Render, hit-testing, and constraints use the same explicit context.
#[derive(Clone, Copy)]
pub(crate) struct WindowVisualContext<'a> {
    pub space: &'a Space<Window>,
    pub cameras: &'a OutputCameras,
    pub output: &'a Output,
    pub window_animations: &'a crate::animation::WindowAnimations,
    pub fullscreen: &'a crate::wayland::fullscreen::FullscreenManager,
    pub maximize: &'a crate::presentation::maximize::FieldMaximizeManager,
    pub decorations: &'a swm_config::Decorations,
    pub now: std::time::Duration,
}

impl WindowVisualContext<'_> {
    /// Landmarks remain usable outside the live bounds of presentations above Field.
    pub(crate) fn covers_field_point(
        self,
        primary_output: &Output,
        position: Point<f64, Logical>,
    ) -> bool {
        let Some(geometry) = self.space.output_geometry(self.output) else {
            return false;
        };
        let local = position - geometry.loc.to_f64();
        self.space.elements().any(|window| {
            if !crate::wayland::window_is_on_output(window, self.output, primary_output) {
                return false;
            }
            let Some(surface) = window.wl_surface() else {
                return false;
            };
            if self
                .fullscreen
                .presentation(surface.as_ref(), self.output, self.now)
                .is_none()
                && self
                    .maximize
                    .presentation(surface.as_ref(), self.output, geometry, self.now)
                    .is_none()
                && crate::wayland::window_presentation_owner(window).is_none()
            {
                return false;
            }
            let Some(visual) = window_visual_state(self, window) else {
                return false;
            };
            let rect = visual.animated_rect.to_logical(1);
            let rect = if visual.fullscreen.is_none() {
                crate::window_chrome::outer_rect_for_client(window, rect, self.decorations)
            } else {
                rect
            };
            presentation_hit_contains(
                visual.above_field_landmarks,
                visual.opening_alpha,
                rect,
                local,
            )
        })
    }
}

fn presentation_hit_contains(
    above_field_landmarks: bool,
    opening_alpha: f32,
    rect: Rectangle<i32, Logical>,
    position: Point<f64, Logical>,
) -> bool {
    above_field_landmarks && opening_alpha > 0.01 && rect.to_f64().contains(position)
}

pub(crate) fn window_visual_state(
    context: WindowVisualContext<'_>,
    window: &Window,
) -> Option<WindowVisualState> {
    if crate::window::closing::pending(window) {
        return None;
    }
    let WindowVisualContext {
        space,
        cameras,
        output,
        window_animations,
        fullscreen,
        maximize,
        decorations: _,
        now,
    } = context;
    let output_geometry = space.output_geometry(output)?;
    let output_size = output_geometry.size.to_physical(1);
    let view = cameras.view(&output.name())?;
    let camera_center = crate::presentation::camera::global_center(view.center, output_geometry);
    let source_geometry = space.element_geometry(window)?;
    let window_surface = window.wl_surface()?;
    // Unparented override-redirect surfaces are positioned by the X11 client
    // in root-screen coordinates. Keep their size and position independent of
    // the Field camera; attached menus still inherit their owner's transform.
    let output_local_rect = standalone_x11_screen_rect(
        crate::xwayland::is_override_redirect(window),
        crate::wayland::window_presentation_owner(window).is_some(),
        source_geometry,
        output_geometry.loc,
    );
    let mut camera_rect = output_local_or_field_rect(
        output_local_rect,
        source_geometry,
        camera_center,
        output_size,
        view.scale,
    );
    let opening_visual = window_animations
        .visual(window_surface.as_ref(), now, camera_rect)
        .unwrap_or_default();
    let fullscreen_presentation = fullscreen.presentation(window_surface.as_ref(), output, now);
    let maximize_presentation = fullscreen_presentation
        .is_none()
        .then(|| maximize.presentation(window_surface.as_ref(), output, output_geometry, now))
        .flatten();
    let mut presentation_rect = fullscreen_presentation
        .map(|presentation| {
            let windowed = presentation.windowed_geometry.map_or_else(
                || presentation.fullscreen_rect(output_size),
                |geometry| {
                    let windowed_output_rect = presentation.windowed_output_rect.map(|mut rect| {
                        rect.loc += cameras.fullscreen_pan_translation(&output.name());
                        rect
                    });
                    output_local_or_field_rect(
                        windowed_output_rect.or(output_local_rect),
                        geometry,
                        camera_center,
                        output_size,
                        view.scale,
                    )
                },
            );
            presentation.client_rect(windowed, output_size)
        })
        .or_else(|| {
            maximize_presentation.map(|presentation| {
                let windowed = output_local_or_field_rect(
                    presentation.windowed_output_rect.or(output_local_rect),
                    presentation.windowed_rect,
                    camera_center,
                    output_size,
                    view.scale,
                );
                presentation.client_rect(windowed)
            })
        })
        .unwrap_or(camera_rect);
    let movement_rect = (output_local_rect.is_none()
        && fullscreen_presentation.is_none()
        && maximize_presentation.is_none())
    .then(|| window_animations.movement_visual(window_surface.as_ref(), now, source_geometry))
    .flatten()
    .map(|world| output_local_or_field_rect(None, world, camera_center, output_size, view.scale))
    .unwrap_or(presentation_rect);
    let opening_rect = opening_visual.transform_rect(movement_rect, movement_rect);
    let mut animated_rect = opening_rect;
    let mut opening_alpha = opening_visual.alpha();
    let mut opening_progress = opening_visual.progress() as f32;
    let mut opening_clamped_progress = opening_visual.clamped_progress() as f32;
    let mut opening_random_seed = opening_visual.random_seed();
    let mut shader_pixels = opening_visual.shader_pixels();
    let mut inherited_presentation = output_local_rect.is_some();
    let mut presentation_space = if output_local_rect.is_some()
        || fullscreen_presentation.is_some()
        || maximize_presentation.is_some()
    {
        PresentationSpace::OutputLocal
    } else {
        PresentationSpace::Field
    };
    let mut inherited_camera_center = camera_center;
    // Field windows take an output-local dest that starts camera-scaled:
    // decorations have to follow dest/native, not live camera zoom and not
    // native 1.0.
    let mut inherited_zoom_scale = if let Some(presentation) = fullscreen_presentation {
        let windowed = presentation.windowed_geometry.map_or_else(
            || presentation.fullscreen_size.to_physical(1),
            |geometry| geometry.size.to_physical(1),
        );
        presentation_display_scale(
            animated_rect.size,
            windowed,
            presentation.fullscreen_size.to_physical(1),
            presentation.progress,
        )
    } else if let Some(presentation) = maximize_presentation {
        presentation_display_scale(
            animated_rect.size,
            presentation.windowed_rect.size.to_physical(1),
            presentation.target_rect.size,
            presentation.progress,
        )
    } else {
        output_local_zoom_scale(presentation_space, view.scale)
    };
    let mut above_field_landmarks = (fullscreen_presentation.is_some()
        && !fullscreen.is_presentation_paused(window_surface.as_ref()))
        || maximize_presentation.is_some();

    if let Some(owner_xid) = crate::wayland::window_presentation_owner(window)
        && let Some(owner) = crate::xwayland::window_for_xid(space, owner_xid)
        && let Some(owner_visual) = window_visual_state(context, &owner)
    {
        let source = source_geometry.to_physical(1);
        let owner_source = owner_visual.source_geometry.to_physical(1);
        (camera_rect, presentation_rect, animated_rect) = inherited_visual_rects(
            source,
            owner_source,
            owner_visual.camera_rect,
            owner_visual.presentation_rect,
            owner_visual.animated_rect,
        );
        opening_alpha = owner_visual.opening_alpha;
        opening_progress = owner_visual.opening_progress;
        opening_clamped_progress = owner_visual.opening_clamped_progress;
        opening_random_seed = owner_visual.opening_random_seed;
        shader_pixels = owner_visual.shader_pixels;
        inherited_camera_center = owner_visual.camera_center;
        inherited_zoom_scale = owner_visual.zoom_scale;
        above_field_landmarks = owner_visual.above_field_landmarks;
        presentation_space = owner_visual.presentation_space;
        inherited_presentation = true;
    }

    Some(WindowVisualState {
        source_geometry,
        camera_rect,
        presentation_rect,
        animated_rect,
        opening_alpha,
        opening_progress,
        opening_clamped_progress,
        opening_random_seed,
        shader_pixels,
        fullscreen: fullscreen_presentation,
        maximize: maximize_presentation,
        camera_center: inherited_camera_center,
        zoom_scale: inherited_zoom_scale,
        presentation_space,
        above_field_landmarks,
        inherited_presentation,
    })
}

/// The live mapping between a window's compositor-space geometry and its
/// current on-screen presentation.
///
/// Pointer routing and pointer constraints both consume this type. Keeping
/// the inverse pair here prevents either path from reconstructing a surface
/// origin from the pointer's last event location.
#[derive(Clone, Debug)]
pub struct WindowPresentation {
    root: WlSurface,
    source_geometry: Rectangle<i32, Logical>,
    root_origin: Point<f64, Logical>,
    visual_geometry: Rectangle<i32, Logical>,
    hit_geometry: Rectangle<i32, Logical>,
    decoration_scale: f32,
}

#[derive(Clone, Copy)]
pub struct WindowPresentationDesktop<'a> {
    pub space: &'a Space<Window>,
    pub cameras: &'a OutputCameras,
    pub window_animations: &'a crate::animation::WindowAnimations,
    pub fullscreen: &'a crate::wayland::fullscreen::FullscreenManager,
    pub maximize: &'a crate::presentation::maximize::FieldMaximizeManager,
    pub decorations: &'a swm_config::Decorations,
}

impl WindowPresentation {
    pub fn for_window(
        desktop: WindowPresentationDesktop<'_>,
        window: &Window,
        output: &Output,
        now: std::time::Duration,
    ) -> Option<Self> {
        let root = window.wl_surface()?.into_owned();
        let output_geometry = desktop.space.output_geometry(output)?;
        let visual = window_visual_state(
            crate::presentation::window::WindowVisualContext {
                space: desktop.space,
                cameras: desktop.cameras,
                output,
                window_animations: desktop.window_animations,
                fullscreen: desktop.fullscreen,
                maximize: desktop.maximize,
                decorations: desktop.decorations,
                now,
            },
            window,
        )?;
        let source_geometry = visual.source_geometry;
        let source_bbox = desktop.space.element_bbox(window)?;
        let element_location = desktop.space.element_location(window)?;
        let root_origin = (element_location - window.geometry().loc).to_f64();
        let output_size = output_geometry.size.to_physical(1);
        let global_geometry = |local: Rectangle<i32, Physical>| {
            Rectangle::new(
                output_geometry.loc + local.loc.to_logical(1),
                local.size.to_logical(1),
            )
        };
        let hit_rect = if visual.maps_from_source() {
            let presented = crate::animation::map_rect(
                source_bbox.to_physical(1),
                source_geometry.to_physical(1),
                visual.presentation_rect,
            );
            crate::animation::map_rect(presented, visual.presentation_rect, visual.animated_rect)
        } else {
            let camera_bbox = crate::presentation::camera::camera_rect(
                source_bbox.to_physical(1),
                visual.camera_center,
                output_size,
                visual.zoom_scale,
            );
            crate::animation::map_rect(camera_bbox, visual.camera_rect, visual.animated_rect)
        };
        let visual_geometry = global_geometry(visual.animated_rect);
        let hit_geometry = global_geometry(hit_rect);

        let decoration_scale = visual.zoom_scale;

        Some(Self {
            root,
            source_geometry,
            root_origin,
            visual_geometry,
            hit_geometry,
            decoration_scale,
        })
    }

    pub fn decoration_scale(&self) -> f32 {
        self.decoration_scale
    }

    pub fn for_surface(
        desktop: WindowPresentationDesktop<'_>,
        primary: &Output,
        surface: &WlSurface,
        now: std::time::Duration,
    ) -> Option<Self> {
        let root = crate::wayland::compositor::root_surface(surface);
        let window = desktop.space.elements().find(|window| {
            window
                .wl_surface()
                .is_some_and(|candidate| candidate.as_ref() == &root)
        })?;
        let output = desktop
            .space
            .outputs()
            .find(|output| crate::wayland::window_is_on_output(window, output, primary))?;
        Self::for_window(desktop, window, output, now)
    }

    pub fn visual_geometry(&self) -> Rectangle<i32, Logical> {
        self.visual_geometry
    }

    #[cfg(feature = "xwayland")]
    pub(crate) fn source_geometry(&self) -> Rectangle<i32, Logical> {
        self.source_geometry
    }

    #[cfg(feature = "xwayland")]
    pub(crate) fn hit_geometry(&self) -> Rectangle<i32, Logical> {
        self.hit_geometry
    }

    pub fn contains_screen(&self, screen: Point<f64, Logical>) -> bool {
        self.hit_geometry.to_f64().contains(screen)
    }

    pub fn source_from_screen(&self, screen: Point<f64, Logical>) -> Point<f64, Logical> {
        map_point(
            screen,
            self.visual_geometry.to_f64(),
            self.source_geometry.to_f64(),
        )
    }

    pub fn screen_from_source(&self, source: Point<f64, Logical>) -> Point<f64, Logical> {
        map_point(
            source,
            self.source_geometry.to_f64(),
            self.visual_geometry.to_f64(),
        )
    }

    /// Output rectangle expressed in this window's parent-geometry coordinates.
    ///
    /// xdg-positioner constraint targets use this space. Inverse-mapping the
    /// output through the live presentation keeps fullscreen and field-maximize
    /// menus on screen instead of inside the field camera's world viewport.
    pub fn popup_constraint_target(
        &self,
        output_geometry: Rectangle<i32, Logical>,
        popup_toplevel_coords: Point<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        popup_constraint_in_parent(
            output_geometry,
            self.source_geometry,
            self.visual_geometry,
            popup_toplevel_coords,
        )
    }

    /// The root wl_surface origin expressed in X11 root-desktop coordinates.
    ///
    /// `Space` stores the root in Field coordinates, while an X11 client sees
    /// one fixed desktop coordinate space.  The two only coincide when the
    /// output camera is at rest, so XWayland geometry must use this mapped
    /// origin rather than `Space::element_location`.
    #[cfg(feature = "xwayland")]
    pub(crate) fn root_screen_origin(&self) -> Point<i32, Logical> {
        self.screen_from_source(self.root_origin).to_i32_round()
    }

    /// The root wl_surface origin in Field/source coordinates.
    ///
    /// X11 child windows express offsets from the owner's last published root
    /// position in native client units. Keeping this origin available lets the
    /// XWM preserve that relative offset before the owner's live presentation
    /// transform scales it.
    pub(crate) fn root_source_origin(&self) -> Point<i32, Logical> {
        self.root_origin.to_i32_round()
    }

    /// Converts an X11 root-desktop point back into the source coordinate of
    /// this window's root wl_surface.
    #[cfg(feature = "xwayland")]
    pub(crate) fn root_source_from_screen(
        &self,
        screen: Point<i32, Logical>,
    ) -> Point<i32, Logical> {
        self.source_from_screen(screen.to_f64()).to_i32_round()
    }

    pub fn surface_origin(&self, surface: &WlSurface) -> Option<Point<f64, Logical>> {
        let offset = subsurface_offset_from_root(surface, &self.root)?;
        Some(self.root_origin + offset.to_f64())
    }

    pub fn surface_from_screen(
        &self,
        surface: &WlSurface,
        screen: Point<f64, Logical>,
    ) -> Option<Point<f64, Logical>> {
        Some(self.source_from_screen(screen) - self.surface_origin(surface)?)
    }

    pub fn screen_from_surface(
        &self,
        surface: &WlSurface,
        local: Point<f64, Logical>,
    ) -> Option<Point<f64, Logical>> {
        Some(self.screen_from_source(self.surface_origin(surface)? + local))
    }
}

pub(crate) fn subsurface_offset_from_root(
    surface: &WlSurface,
    expected_root: &WlSurface,
) -> Option<Point<i32, Logical>> {
    let mut current = surface.clone();
    let mut offset = Point::from((0, 0));
    while &current != expected_root {
        let parent = get_parent(&current)?;
        let location = with_states(&current, |states| {
            states
                .cached_state
                .get::<SubsurfaceCachedState>()
                .current()
                .location
        });
        offset += location;
        current = parent;
    }
    Some(offset)
}

fn map_point(
    point: Point<f64, Logical>,
    source: Rectangle<f64, Logical>,
    destination: Rectangle<f64, Logical>,
) -> Point<f64, Logical> {
    let scale_x = destination.size.w / source.size.w.max(1.0);
    let scale_y = destination.size.h / source.size.h.max(1.0);
    (
        destination.loc.x + (point.x - source.loc.x) * scale_x,
        destination.loc.y + (point.y - source.loc.y) * scale_y,
    )
        .into()
}

/// Inverse-maps `output_geometry` through a window's visual presentation into
/// parent-geometry-relative coordinates for `xdg_positioner` constraints.
pub(crate) fn popup_constraint_in_parent(
    output_geometry: Rectangle<i32, Logical>,
    source_geometry: Rectangle<i32, Logical>,
    visual_geometry: Rectangle<i32, Logical>,
    popup_toplevel_coords: Point<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let output_in_source = crate::animation::map_rect(
        output_geometry.to_physical(1),
        visual_geometry.to_physical(1),
        source_geometry.to_physical(1),
    )
    .to_logical(1);
    Rectangle::new(
        output_in_source.loc - source_geometry.loc - popup_toplevel_coords,
        output_in_source.size,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn field_landmark_hit_test_uses_visible_presentation_bounds() {
        use super::presentation_hit_contains;
        let rect = smithay::utils::Rectangle::new((10, 20).into(), (100, 50).into());
        assert!(presentation_hit_contains(
            true,
            1.0,
            rect,
            (11.0, 21.0).into()
        ));
        assert!(!presentation_hit_contains(
            false,
            1.0,
            rect,
            (11.0, 21.0).into()
        ));
        assert!(!presentation_hit_contains(
            true,
            0.01,
            rect,
            (11.0, 21.0).into()
        ));
        assert!(!presentation_hit_contains(
            true,
            1.0,
            rect,
            (110.0, 21.0).into()
        ));
    }

    use super::*;

    fn transform(
        source: Rectangle<i32, Logical>,
        visual: Rectangle<i32, Logical>,
    ) -> (Rectangle<f64, Logical>, Rectangle<f64, Logical>) {
        (source.to_f64(), visual.to_f64())
    }

    #[test]
    fn output_local_presentations_ignore_field_camera_zoom() {
        assert_eq!(
            output_local_zoom_scale(PresentationSpace::OutputLocal, 0.5),
            1.0
        );
        assert_eq!(output_local_zoom_scale(PresentationSpace::Field, 0.5), 0.5);
    }

    #[test]
    fn field_crossfade_chrome_follows_the_zoomed_destination() {
        let start = presentation_display_scale(
            (350, 210).into(),
            (1000, 600).into(),
            (2000, 1200).into(),
            0.0,
        );
        let end = presentation_display_scale(
            (2000, 1200).into(),
            (1000, 600).into(),
            (2000, 1200).into(),
            1.0,
        );

        assert!((start - 0.35).abs() < f32::EPSILON);
        assert_eq!(end, 1.0);
    }

    #[test]
    fn screen_and_surface_mapping_are_exact_inverses() {
        let (source, visual) = transform(
            Rectangle::new((400, 200).into(), (1280, 720).into()),
            Rectangle::new((2560, 0).into(), (1920, 1080).into()),
        );
        let local = Point::from((720.0, 380.0));
        let screen = map_point(local, source, visual);

        assert_eq!(screen, Point::from((3040.0, 270.0)));
        assert_eq!(map_point(screen, visual, source), local);
    }

    #[test]
    fn x11_field_origin_maps_to_the_settled_secondary_root_position() {
        let (source, visual) = transform(
            Rectangle::new((1546, -214).into(), (1532, 1009).into()),
            Rectangle::new((2655, 181).into(), (1532, 1009).into()),
        );

        assert_eq!(
            map_point(Point::from((1546.0, -214.0)), source, visual),
            Point::from((2655.0, 181.0))
        );
    }

    #[test]
    fn root_surface_offset_is_mapped_without_changing_native_size() {
        let (source, visual) = transform(
            Rectangle::new((400, 200).into(), (800, 600).into()),
            Rectangle::new((2600, 100).into(), (400, 300).into()),
        );
        let root_source = Point::from((380.0, 170.0));

        assert_eq!(
            map_point(root_source, source, visual),
            Point::from((2590.0, 85.0))
        );
    }

    #[test]
    fn camera_zoom_and_pan_mapping_round_trips() {
        let (source, visual) = transform(
            Rectangle::new((1700, -650).into(), (3840, 2400).into()),
            Rectangle::new((2560, 0).into(), (1920, 1200).into()),
        );
        for point in [
            Point::from((1700.0, -650.0)),
            Point::from((3620.0, 550.0)),
            Point::from((5539.0, 1749.0)),
        ] {
            let screen = map_point(point, source, visual);
            let round_trip = map_point(screen, visual, source);
            assert!((round_trip.x - point.x).abs() < f64::EPSILON);
            assert!((round_trip.y - point.y).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn fullscreen_letterbox_mapping_uses_live_client_rectangle() {
        let (source, visual) = transform(
            Rectangle::new((0, 0).into(), (1024, 768).into()),
            Rectangle::new((240, 0).into(), (1440, 1080).into()),
        );

        assert_eq!(
            map_point(Point::from((512.0, 384.0)), source, visual),
            Point::from((960.0, 540.0))
        );
        assert_eq!(
            map_point(Point::from((960.0, 540.0)), visual, source),
            Point::from((512.0, 384.0))
        );
    }

    #[test]
    fn x11_popup_inherits_owners_panned_and_fullscreen_presentation() {
        let owner_source = Rectangle::<i32, Physical>::new((1090, 425).into(), (1920, 1200).into());
        let popup_source = Rectangle::<i32, Physical>::new((1158, 491).into(), (147, 198).into());
        let owner_camera = Rectangle::<i32, Physical>::new((0, 0).into(), (1920, 1200).into());
        let owner_fullscreen = Rectangle::<i32, Physical>::new((0, 0).into(), (1920, 1200).into());
        let owner_animated = Rectangle::<i32, Physical>::new((96, 60).into(), (1728, 1080).into());

        let (camera, fullscreen, animated) = inherited_visual_rects(
            popup_source,
            owner_source,
            owner_camera,
            owner_fullscreen,
            owner_animated,
        );

        assert_eq!(camera, Rectangle::new((68, 66).into(), (147, 198).into()));
        assert_eq!(fullscreen, camera);
        assert_eq!(
            animated,
            Rectangle::new((157, 119).into(), (133, 179).into())
        );
    }

    #[test]
    fn subsurface_coordinates_share_the_window_transform() {
        let source = Rectangle::<f64, Logical>::new((400.0, 200.0).into(), (800.0, 600.0).into());
        let visual = Rectangle::<f64, Logical>::new((100.0, 50.0).into(), (1600.0, 1200.0).into());
        let root_origin = Point::from((380.0, 170.0));
        let subsurface_offset = Point::from((30.0, 40.0));
        let local = Point::from((25.0, 15.0));
        let source_point = root_origin + subsurface_offset + local;
        let screen = map_point(source_point, source, visual);
        let round_trip = map_point(screen, visual, source) - root_origin - subsurface_offset;

        assert_eq!(screen, Point::from((170.0, 100.0)));
        assert_eq!(round_trip, local);
    }

    #[test]
    fn settled_fullscreen_popup_constraint_covers_the_output() {
        let output = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let source = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let visual = output;
        let windowed_center = Point::<f32, Physical>::from((1040.0, 560.0));
        let camera_viewport = crate::presentation::camera::world_viewport(
            crate::presentation::camera::OutputView {
                center: windowed_center,
                scale: 1.0,
            },
            output,
        );

        let target = popup_constraint_in_parent(output, source, visual, Point::from((0, 0)));

        assert_eq!(target, Rectangle::new((0, 0).into(), (1920, 1080).into()));
        assert_ne!(
            {
                let mut camera_target = camera_viewport;
                camera_target.loc -= source.loc;
                camera_target
            },
            target,
            "the field camera stays on the old windowed center and must not own the constraint"
        );
    }

    #[test]
    fn settled_maximize_popup_constraint_includes_the_gap() {
        let output = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let source = Rectangle::<i32, Logical>::new((20, 20).into(), (1880, 1040).into());
        let visual = source;

        assert_eq!(
            popup_constraint_in_parent(output, source, visual, Point::from((0, 0))),
            Rectangle::new((-20, -20).into(), (1920, 1080).into())
        );
    }

    #[test]
    fn panned_maximize_camera_does_not_steal_the_popup_constraint() {
        let output = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let source = Rectangle::<i32, Logical>::new((20, 20).into(), (1880, 1040).into());
        let visual = source;
        let camera_viewport = crate::presentation::camera::world_viewport(
            crate::presentation::camera::OutputView {
                center: Point::from((3000.0, 540.0)),
                scale: 1.0,
            },
            output,
        );
        let mut camera_target = camera_viewport;
        camera_target.loc -= source.loc;
        let target = popup_constraint_in_parent(output, source, visual, Point::from((0, 0)));

        assert_eq!(
            target,
            Rectangle::new((-20, -20).into(), (1920, 1080).into())
        );
        assert!(
            camera_target.loc.x > source.size.w,
            "today's camera formula would place the box beside the client"
        );
        assert!(!camera_target.overlaps(Rectangle::from_size(source.size)));
        assert!(target.overlaps(Rectangle::from_size(source.size)));
    }

    #[test]
    fn field_popup_constraint_matches_the_camera_world_viewport() {
        let output = Rectangle::<i32, Logical>::new((2560, 0).into(), (1920, 1200).into());
        let world_viewport = crate::presentation::camera::world_viewport(
            crate::presentation::camera::OutputView {
                center: Point::from((1060.0, 550.0)),
                scale: 0.5,
            },
            output,
        );
        assert_eq!(
            world_viewport,
            Rectangle::new((1700, -650).into(), (3840, 2400).into())
        );
        let source = Rectangle::<i32, Logical>::new((1800, -570).into(), (800, 600).into());
        let visual = Rectangle::<i32, Logical>::new((2610, 40).into(), (400, 300).into());
        let popup_coords = Point::<i32, Logical>::from((12, 8));
        let mut expected = world_viewport;
        expected.loc -= source.loc;
        expected.loc -= popup_coords;

        let target = popup_constraint_in_parent(output, source, visual, popup_coords);
        assert!(target.loc.x.abs_diff(expected.loc.x) <= 1);
        assert!(target.loc.y.abs_diff(expected.loc.y) <= 1);
        assert!(target.size.w.abs_diff(expected.size.w) <= 1);
        assert!(target.size.h.abs_diff(expected.size.h) <= 1);
    }

    #[test]
    fn nested_popup_constraint_subtracts_toplevel_coords_once() {
        let output = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let source = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let nested = popup_constraint_in_parent(output, source, output, Point::from((80, 40)));
        let root = popup_constraint_in_parent(output, source, output, Point::from((0, 0)));

        assert_eq!(nested.loc, root.loc - Point::from((80, 40)));
        assert_eq!(nested.size, root.size);
    }

    #[test]
    fn positioner_slide_keeps_a_corner_menu_on_the_new_fullscreen_target() {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{
            Anchor, ConstraintAdjustment, Gravity,
        };
        use smithay::wayland::shell::xdg::PositionerState;

        let positioner = PositionerState {
            rect_size: (200, 300).into(),
            anchor_rect: Rectangle::new((50, 50).into(), (1, 1).into()),
            anchor_edges: Anchor::TopLeft,
            gravity: Gravity::BottomRight,
            constraint_adjustment: ConstraintAdjustment::SlideX,
            offset: (0, 0).into(),
            ..Default::default()
        };

        let output = Rectangle::<i32, Logical>::new((0, 0).into(), (1920, 1080).into());
        let source = output;
        let visual = output;
        let correct = popup_constraint_in_parent(output, source, visual, Point::from((0, 0)));
        let mut camera_target = crate::presentation::camera::world_viewport(
            crate::presentation::camera::OutputView {
                center: Point::from((1040.0, 560.0)),
                scale: 1.0,
            },
            output,
        );
        camera_target.loc -= source.loc;

        let requested = positioner.get_geometry();
        let slid = positioner.get_unconstrained_geometry(camera_target);
        let kept = positioner.get_unconstrained_geometry(correct);

        assert_eq!(requested.loc, Point::from((50, 50)));
        assert_ne!(
            slid.loc.x, requested.loc.x,
            "the old camera box must actually slide a corner menu"
        );
        assert_eq!(kept.loc, requested.loc);
    }
}

fn standalone_x11_screen_rect(
    override_redirect: bool,
    has_owner: bool,
    geometry: Rectangle<i32, Logical>,
    output_origin: Point<i32, Logical>,
) -> Option<Rectangle<i32, Physical>> {
    (override_redirect && !has_owner).then(|| {
        Rectangle::new(
            (geometry.loc - output_origin).to_physical(1),
            geometry.size.to_physical(1),
        )
    })
}

#[cfg(test)]
mod standalone_popup_tests {
    use super::*;
    #[test]
    fn standalone_popup_keeps_screen_geometry_across_output_origins() {
        let rect = Rectangle::new((2500, -300).into(), (772, 2849).into());
        for origin in [Point::from((0, 0)), Point::from((2560, 0))] {
            let local = standalone_x11_screen_rect(true, false, rect, origin).unwrap();
            for zoom in [0.25, 0.5, 1.0, 2.0] {
                let rendered = output_local_or_field_rect(
                    Some(local),
                    rect,
                    Point::from((8000.0, -2000.0)),
                    (2560, 1440).into(),
                    zoom,
                );
                assert_eq!(rendered, local);
                assert_eq!(rendered.size, rect.size.to_physical(1));
            }
        }
    }
    #[test]
    fn managed_windows_and_attached_popups_keep_existing_transforms() {
        let rect = Rectangle::new((100, 200).into(), (400, 300).into());
        assert!(standalone_x11_screen_rect(false, false, rect, (0, 0).into()).is_none());
        assert!(standalone_x11_screen_rect(true, true, rect, (0, 0).into()).is_none());
    }
}
