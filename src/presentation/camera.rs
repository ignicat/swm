use std::collections::HashMap;

use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use swm_core::camera::Camera;
use swm_core::field::Vec2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputView {
    pub center: Point<f32, Physical>,
    pub scale: f32,
}

/// The output-wide camera presentation owned by a fullscreen transaction.
///
/// Fullscreen is a monitor state, not a special transform applied only to its
/// client. Driving the shared camera with the same progress keeps any windows
/// stacked above the fullscreen surface at the same 1.0 zoom and pan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FullscreenCameraFrame {
    pub center: Point<f32, Physical>,
    pub progress: f32,
    pub desired: bool,
    pub transition_active: bool,
}

#[derive(Clone, Copy, Debug)]
struct FullscreenCameraRestore {
    center: Vec2,
    view_size: Vec2,
    /// The snapshot before any pan. `center` drifts from it when an exit that
    /// was panned reverses into fullscreen and keeps that pan as destination.
    original_center: Vec2,
    /// Last camera center produced by the fullscreen track, before user pan.
    /// Present only during a committed exit; input and camera easing can then
    /// move relative to this track without the next frame erasing that motion.
    pan_origin: Option<Vec2>,
    handoff_from_center: Option<Vec2>,
    handoff_from_view_size: Option<Vec2>,
    handoff_target_center: Option<Vec2>,
    handoff_target_view_size: Option<Vec2>,
}

/// Fullscreen locks the camera, except during a committed exit.
fn allows_pan(restore: Option<&FullscreenCameraRestore>) -> bool {
    restore.is_none_or(|restore| restore.pan_origin.is_some())
}

impl FullscreenCameraRestore {
    fn pan_offset(&self, center: Vec2) -> Vec2 {
        self.pan_origin
            .map_or(Vec2 { x: 0.0, y: 0.0 }, |origin| Vec2 {
                x: center.x - origin.x,
                y: center.y - origin.y,
            })
    }
}

#[derive(Clone, Copy, Debug)]
struct FieldMaximizeCameraRestore {
    center: Vec2,
    view_size: Vec2,
    /// Where the camera eases *from* at progress zero, when that is not the
    /// restore snapshot itself. Only the fullscreen handoff sets these: the
    /// window is already presented at native zoom on the fullscreen camera, so
    /// starting the maximize track from the zoomed-out pre-fullscreen snapshot
    /// would pop the whole output on the handoff frame.
    from_center: Option<Vec2>,
    from_view_size: Option<Vec2>,
    target_center: Option<Vec2>,
    target_view_size: Option<Vec2>,
}

/// Independent per-output camera state keyed by Smithay output name.
#[derive(Default)]
pub struct OutputCameras {
    cameras: HashMap<String, Camera>,
    fullscreen: HashMap<String, FullscreenCameraRestore>,
    field_maximize: HashMap<String, FieldMaximizeCameraRestore>,
    pending_field_center: HashMap<String, Vec2>,
}

impl OutputCameras {
    pub fn insert(&mut self, output_name: String, output_size: Size<i32, Physical>) {
        self.pending_field_center.remove(&output_name);
        self.fullscreen.remove(&output_name);
        self.field_maximize.remove(&output_name);
        self.cameras
            .insert(output_name, camera_at_rest(output_size));
    }

    pub fn reset(&mut self, output_name: String, output_size: Size<i32, Physical>) {
        self.insert(output_name, output_size);
    }

    /// Display scaling changes the viewport extent while retaining the Field
    /// location, zoom, and any fullscreen/maximize restore state.
    pub fn resize_for_display_scale(
        &mut self,
        output_name: &str,
        output_size: Size<i32, Physical>,
    ) {
        let Some(camera) = self.cameras.get_mut(output_name) else {
            return;
        };
        let ratio = Vec2 {
            x: output_size.w as f32 / camera.base_size.x.max(1.0),
            y: output_size.h as f32 / camera.base_size.y.max(1.0),
        };
        let resize = |size: Vec2| Vec2 {
            x: size.x * ratio.x,
            y: size.y * ratio.y,
        };
        camera.base_size = Vec2 {
            x: output_size.w as f32,
            y: output_size.h as f32,
        };
        camera.view_size = resize(camera.view_size);
        camera.target_view_size = resize(camera.target_view_size);
        if let Some(restore) = self.fullscreen.get_mut(output_name) {
            restore.view_size = resize(restore.view_size);
            restore.handoff_from_view_size = restore.handoff_from_view_size.map(resize);
            restore.handoff_target_view_size = restore.handoff_target_view_size.map(resize);
        }
        if let Some(restore) = self.field_maximize.get_mut(output_name) {
            restore.view_size = resize(restore.view_size);
            restore.from_view_size = restore.from_view_size.map(resize);
            restore.target_view_size = restore.target_view_size.map(resize);
        }
    }

    pub fn remove(&mut self, output_name: &str) {
        self.pending_field_center.remove(output_name);
        self.cameras.remove(output_name);
        self.fullscreen.remove(output_name);
        self.field_maximize.remove(output_name);
    }

    pub fn get(&self, output_name: &str) -> Option<&Camera> {
        self.cameras.get(output_name)
    }

    pub fn get_mut(&mut self, output_name: &str) -> Option<&mut Camera> {
        if self.is_locked(output_name) {
            return None;
        }
        self.cameras.get_mut(output_name)
    }

    /// Whether fullscreen or Field maximize currently owns this output's camera
    /// and therefore rejects mutation.
    fn is_locked(&self, output_name: &str) -> bool {
        self.fullscreen.contains_key(output_name) || self.field_maximize.contains_key(output_name)
    }

    /// Whether a pointer drag could move this output's camera right now.
    ///
    /// This is exactly the condition under which [`Self::get_mut_for_pan`] yields a
    /// camera, so a compositor pan grab can refuse to begin (and leave the
    /// press to the client) instead of swallowing a click it cannot honor.
    pub fn is_pannable(&self, output_name: &str) -> bool {
        self.cameras.contains_key(output_name)
            && allows_pan(self.fullscreen.get(output_name))
            && !self.field_maximize.contains_key(output_name)
    }

    /// Panning may accompany fullscreen exit; other camera mutations stay locked.
    pub fn get_mut_for_pan(&mut self, output_name: &str) -> Option<&mut Camera> {
        self.is_pannable(output_name)
            .then(|| self.cameras.get_mut(output_name))
            .flatten()
    }

    fn fullscreen_pan_offset(&self, output_name: &str) -> Vec2 {
        let Some(restore) = self.fullscreen.get(output_name) else {
            return Vec2 { x: 0.0, y: 0.0 };
        };
        restore.pan_offset(self.cameras[output_name].center)
    }

    /// Screen-space shift of the windowed destination caused by user pan,
    /// including pan carried over from an exit that reversed into fullscreen.
    pub(crate) fn fullscreen_pan_translation(&self, output_name: &str) -> Point<i32, Physical> {
        let Some(restore) = self.fullscreen.get(output_name) else {
            return (0, 0).into();
        };
        let offset = self.fullscreen_pan_offset(output_name);
        let scale = self.cameras[output_name].base_size.x / restore.view_size.x.max(1.0);
        (
            (-(restore.center.x - restore.original_center.x + offset.x) * scale).round() as i32,
            (-(restore.center.y - restore.original_center.y + offset.y) * scale).round() as i32,
        )
            .into()
    }

    /// Queue an exact Field center without disturbing fullscreen or maximize
    /// camera ownership. Navigation may prepare its destination underneath a
    /// maximize transition so it begins easing as soon as the Field becomes
    /// visible again.
    pub fn center_field_on(&mut self, output_name: &str, center: Vec2) -> bool {
        if !allows_pan(self.fullscreen.get(output_name))
            || self.field_maximize.contains_key(output_name)
        {
            return false;
        }
        let Some(camera) = self.cameras.get_mut(output_name) else {
            return false;
        };
        let changed =
            camera.target_center != center || camera.pan_vel.x != 0.0 || camera.pan_vel.y != 0.0;
        camera.target_center = center;
        camera.pan_vel = Vec2 { x: 0.0, y: 0.0 };
        changed
    }

    /// The Field view that will be restored after presentation exit. Reveal
    /// destinations use this extent so restoring zoom cannot undo the reveal.
    pub fn field_restore_view(&self, output_name: &str) -> Option<OutputView> {
        let camera = self.cameras.get(output_name)?;
        let (center, view_size) = if let Some(restore) = self.fullscreen.get(output_name) {
            (restore.center, restore.view_size)
        } else if let Some(restore) = self.field_maximize.get(output_name) {
            (restore.center, restore.view_size)
        } else {
            (camera.center, camera.view_size)
        };
        Some(OutputView {
            center: (center.x, center.y).into(),
            scale: (camera.base_size.x / view_size.x.max(1.0)).min(1.0),
        })
    }

    /// Navigation has already requested presentation exit. Maximize keeps its
    /// camera until its exit finishes; begin the queued pan when it releases.
    pub fn queue_field_center(&mut self, output_name: &str, center: Vec2) {
        if self.field_maximize.contains_key(output_name) {
            self.pending_field_center
                .insert(output_name.to_string(), center);
        } else {
            self.center_field_on(output_name, center);
        }
    }

    pub fn cancel_queued_field_center(&mut self, output_name: &str) {
        self.pending_field_center.remove(output_name);
    }

    /// The output's camera if it may tick, and whether a fullscreen exit
    /// still owns its zoom (see [`crate::input::zoom::tick_camera`]).
    pub fn get_mut_for_tick(&mut self, output_name: &str) -> Option<(&mut Camera, bool)> {
        let returning = self.fullscreen.contains_key(output_name);
        self.get_mut_for_pan(output_name)
            .map(|camera| (camera, returning))
    }

    /// Every camera that may tick, as for [`Self::get_mut_for_tick`].
    pub fn iter_for_tick(&mut self) -> impl Iterator<Item = (&mut Camera, bool)> {
        let Self {
            cameras,
            fullscreen,
            field_maximize,
            pending_field_center: _,
        } = self;
        cameras.iter_mut().filter_map(move |(name, camera)| {
            (allows_pan(fullscreen.get(name)) && !field_maximize.contains_key(name))
                .then_some((camera, fullscreen.contains_key(name)))
        })
    }

    pub fn view(&self, output_name: &str) -> Option<OutputView> {
        self.get(output_name).map(|camera| OutputView {
            center: Point::from((camera.center.x, camera.center.y)),
            scale: scale(camera),
        })
    }

    /// Applies or releases fullscreen ownership of one output camera.
    ///
    /// The first fullscreen frame captures the live pre-fullscreen camera.
    /// Every subsequent frame derives both center and view size from that one
    /// snapshot and the fullscreen transition progress, preventing independent
    /// easing tracks from drifting apart. Releasing restores that snapshot and
    /// preserves any user pan accumulated during the return, including its
    /// pending target and momentum. Fullscreen continues to own the zoom track.
    pub fn apply_fullscreen(
        &mut self,
        output_name: &str,
        frame: Option<FullscreenCameraFrame>,
    ) -> bool {
        let Some(camera) = self.cameras.get_mut(output_name) else {
            self.fullscreen.remove(output_name);
            return false;
        };
        let before = *camera;
        match frame {
            Some(frame) => {
                let restore = self.fullscreen.entry(output_name.to_string()).or_insert(
                    FullscreenCameraRestore {
                        center: camera.center,
                        view_size: camera.view_size,
                        original_center: camera.center,
                        pan_origin: None,
                        handoff_from_center: None,
                        handoff_from_view_size: None,
                        handoff_target_center: None,
                        handoff_target_view_size: None,
                    },
                );
                let pan_origin = restore.pan_origin;
                let pan_offset = restore.pan_offset(camera.center);
                let pan_target_offset = restore.pan_offset(camera.target_center);
                let progress = frame.progress.clamp(0.0, 1.0);
                if frame.desired && pan_origin.is_some() {
                    // Reverse smoothly toward fullscreen and retain the pan as
                    // the next exit's destination, rather than snapping it away.
                    restore.center.x += pan_offset.x;
                    restore.center.y += pan_offset.y;
                    restore.handoff_from_center = rebase_target(
                        Vec2 {
                            x: frame.center.x,
                            y: frame.center.y,
                        },
                        camera.center,
                        1.0 - progress,
                    );
                    restore.handoff_from_view_size = Some(restore.view_size);
                    restore.handoff_target_center = None;
                }
                if !frame.desired && restore.handoff_from_center.is_some() {
                    restore.handoff_from_center = None;
                    restore.handoff_from_view_size = None;
                    restore.handoff_target_center =
                        rebase_target(restore.center, camera.center, progress);
                    restore.handoff_target_view_size =
                        rebase_target(restore.view_size, camera.view_size, progress);
                } else if frame.desired && !frame.transition_active && progress >= 1.0 {
                    restore.handoff_from_center = None;
                    restore.handoff_from_view_size = None;
                }
                let source_center = restore.handoff_from_center.unwrap_or(restore.center);
                let source_view_size = restore.handoff_from_view_size.unwrap_or(restore.view_size);
                let target_center = restore.handoff_target_center.unwrap_or(Vec2 {
                    x: frame.center.x,
                    y: frame.center.y,
                });
                let target_view_size = restore.handoff_target_view_size.unwrap_or(camera.base_size);
                let motion_center = lerp_vec2(source_center, target_center, progress);
                let pannable = !frame.desired && frame.transition_active;
                restore.pan_origin = pannable.then_some(motion_center);
                camera.center = motion_center;
                camera.view_size = lerp_vec2(source_view_size, target_view_size, progress);
                camera.target_center = camera.center;
                camera.target_view_size = camera.view_size;
                if pannable {
                    camera.center.x += pan_offset.x;
                    camera.center.y += pan_offset.y;
                    camera.target_center.x += pan_target_offset.x;
                    camera.target_center.y += pan_target_offset.y;
                } else {
                    camera.pan_vel = Vec2 { x: 0.0, y: 0.0 };
                }
                camera.zoom_log_vel = 0.0;
            }
            None => {
                let Some(restore) = self.fullscreen.remove(output_name) else {
                    return false;
                };
                let pan_offset = restore.pan_offset(camera.center);
                let pan_target_offset = restore.pan_offset(camera.target_center);
                camera.center = Vec2 {
                    x: restore.center.x + pan_offset.x,
                    y: restore.center.y + pan_offset.y,
                };
                camera.view_size = restore.view_size;
                camera.target_center = Vec2 {
                    x: restore.center.x + pan_target_offset.x,
                    y: restore.center.y + pan_target_offset.y,
                };
                camera.target_view_size = restore.view_size;
                if restore.pan_origin.is_none() {
                    camera.pan_vel = Vec2 { x: 0.0, y: 0.0 };
                }
                camera.zoom_log_vel = 0.0;
            }
        }
        *camera != before
    }

    /// Transfers fullscreen camera ownership straight to field maximize.
    ///
    /// The pre-fullscreen snapshot moves into the field-maximize slot, so
    /// un-maximizing later still restores the camera the window had before any
    /// of this started. The camera's live state becomes the maximize track's
    /// progress-zero endpoint, which is what keeps the handoff frame identical
    /// to the frame before it: releasing fullscreen normally
    /// (`apply_fullscreen(.., None)`) would snap the output back to the
    /// zoomed-out snapshot instead.
    pub fn handoff_fullscreen_to_field_maximize(&mut self, output_name: &str) -> bool {
        let Some(camera) = self.cameras.get(output_name) else {
            return false;
        };
        let Some(restore) = self.fullscreen.remove(output_name) else {
            return false;
        };
        self.field_maximize.insert(
            output_name.to_string(),
            FieldMaximizeCameraRestore {
                center: restore.center,
                view_size: restore.view_size,
                from_center: Some(camera.center),
                from_view_size: Some(camera.view_size),
                target_center: Some(camera.center),
                target_view_size: Some(camera.view_size),
            },
        );
        true
    }

    /// Transfers a maximized camera straight to fullscreen without moving the
    /// desktop underneath the window. The original pre-maximize snapshot
    /// remains the restore endpoint for a later fullscreen exit.
    pub fn handoff_field_maximize_to_fullscreen(&mut self, output_name: &str) -> bool {
        let Some(camera) = self.cameras.get(output_name) else {
            return false;
        };
        let Some(restore) = self.field_maximize.remove(output_name) else {
            return false;
        };
        self.fullscreen.insert(
            output_name.to_string(),
            FullscreenCameraRestore {
                center: restore.center,
                view_size: restore.view_size,
                original_center: restore.center,
                pan_origin: None,
                handoff_from_center: Some(camera.center),
                handoff_from_view_size: Some(camera.view_size),
                handoff_target_center: Some(camera.center),
                handoff_target_view_size: Some(camera.view_size),
            },
        );
        true
    }

    /// Drops a recorded handoff source, so the field-maximize track eases
    /// between its own endpoints again.
    ///
    /// The source is entry-only: progress runs backwards on un-maximize, so a
    /// stale one would walk the camera toward the retired fullscreen view and
    /// then snap to the restore snapshot when the track releases.
    pub fn clear_field_maximize_handoff(&mut self, output_name: &str, progress: Option<f32>) {
        let Some(camera) = self.cameras.get(output_name).copied() else {
            return;
        };
        if let Some(restore) = self.field_maximize.get_mut(output_name) {
            let progress = progress.unwrap_or(1.0).clamp(0.0, 1.0);
            if restore.from_center.is_some() {
                restore.target_center = rebase_target(restore.center, camera.center, progress);
                restore.target_view_size =
                    rebase_target(restore.view_size, camera.view_size, progress);
            }
            restore.from_center = None;
            restore.from_view_size = None;
        }
    }

    /// Owns one output camera for field maximize while keeping its center
    /// fixed and easing its scale to native 1.0. Releasing restores the exact
    /// pre-maximize camera snapshot.
    pub fn apply_field_maximize(&mut self, output_name: &str, progress: Option<f32>) -> bool {
        let Some(camera) = self.cameras.get_mut(output_name) else {
            self.field_maximize.remove(output_name);
            return false;
        };
        let before = *camera;
        match progress {
            Some(progress) => {
                let restore = self
                    .field_maximize
                    .entry(output_name.to_string())
                    .or_insert(FieldMaximizeCameraRestore {
                        center: camera.center,
                        view_size: camera.view_size,
                        from_center: None,
                        from_view_size: None,
                        target_center: None,
                        target_view_size: None,
                    });
                let progress = progress.clamp(0.0, 1.0);
                camera.center = lerp_vec2(
                    restore.from_center.unwrap_or(restore.center),
                    restore.target_center.unwrap_or(restore.center),
                    progress,
                );
                camera.view_size = lerp_vec2(
                    restore.from_view_size.unwrap_or(restore.view_size),
                    restore.target_view_size.unwrap_or(camera.base_size),
                    progress,
                );
                camera.target_center = camera.center;
                camera.target_view_size = camera.view_size;
                camera.pan_vel = Vec2 { x: 0.0, y: 0.0 };
                camera.zoom_log_vel = 0.0;
            }
            None => {
                let Some(restore) = self.field_maximize.remove(output_name) else {
                    return false;
                };
                camera.center = restore.center;
                camera.view_size = restore.view_size;
                camera.target_center = self
                    .pending_field_center
                    .remove(output_name)
                    .unwrap_or(restore.center);
                camera.target_view_size = restore.view_size;
                camera.pan_vel = Vec2 { x: 0.0, y: 0.0 };
                camera.zoom_log_vel = 0.0;
            }
        }
        *camera != before
    }
}

fn lerp_vec2(from: Vec2, to: Vec2, progress: f32) -> Vec2 {
    Vec2 {
        x: from.x + (to.x - from.x) * progress,
        y: from.y + (to.y - from.y) * progress,
    }
}

fn rebase_target(restore: Vec2, current: Vec2, progress: f32) -> Option<Vec2> {
    (progress > f32::EPSILON).then(|| Vec2 {
        x: restore.x + (current.x - restore.x) / progress,
        y: restore.y + (current.y - restore.y) / progress,
    })
}

pub fn scale(camera: &Camera) -> f32 {
    (camera.base_size.x / camera.view_size.x.max(1.0)).min(1.0)
}

/// Rebases an output-local camera center into SWM's global world space.
///
/// Cameras stay local so panning one output cannot move another. Windows,
/// however, live in one global `Space`, so rendering and initial placement
/// must apply the same output-layout offset to the selected camera.
pub fn global_center(
    local_camera_center: Point<f32, Physical>,
    output_geometry: Rectangle<i32, Logical>,
) -> Point<f32, Physical> {
    Point::from((
        output_geometry.loc.x as f32 + local_camera_center.x,
        output_geometry.loc.y as f32 + local_camera_center.y,
    ))
}

/// Returns the portion of global world space currently visible through an
/// output camera. XDG popup constraints are expressed in the same world
/// coordinates as `Space`, so their target must follow this inverse of the
/// render transform rather than the output's fixed layout rectangle.
pub fn world_viewport(
    view: OutputView,
    output_geometry: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let center = global_center(view.center, output_geometry);
    let scale = view.scale.max(f32::EPSILON);
    let half_width = output_geometry.size.w as f32 / scale / 2.0;
    let half_height = output_geometry.size.h as f32 / scale / 2.0;
    let left = (center.x - half_width).floor() as i32;
    let top = (center.y - half_height).floor() as i32;
    let right = (center.x + half_width).ceil() as i32;
    let bottom = (center.y + half_height).ceil() as i32;

    Rectangle::new((left, top).into(), (right - left, bottom - top).into())
}

/// Maps a physical-pixel rect from world (`Space`) coordinates to screen
/// coordinates around `camera_center` at `scale` (`<= 1.0`).
pub fn camera_rect(
    rect: Rectangle<i32, Physical>,
    camera_center: Point<f32, Physical>,
    output_size: Size<i32, Physical>,
    scale: f32,
) -> Rectangle<i32, Physical> {
    let scale = f64::from(scale);
    // Round the camera translation once, independently of each surface's
    // dimensions. Center rounding gave odd/even rectangles different phases,
    // making client pixels and chrome take their last pixel step separately.
    let translation = Point::<i32, Physical>::from((
        (f64::from(output_size.w) / 2.0 - f64::from(camera_center.x) * scale).round() as i32,
        (f64::from(output_size.h) / 2.0 - f64::from(camera_center.y) * scale).round() as i32,
    ));
    // Shared world edges also stay shared under zoom. Panning changes only
    // translation, preserving every part's size and relative placement.
    let left = (f64::from(rect.loc.x) * scale).round() as i32;
    let top = (f64::from(rect.loc.y) * scale).round() as i32;
    let right = ((f64::from(rect.loc.x) + f64::from(rect.size.w)) * scale).round() as i32;
    let bottom = ((f64::from(rect.loc.y) + f64::from(rect.size.h)) * scale).round() as i32;
    Rectangle::new(
        Point::from((left, top)) + translation,
        Size::from((right - left, bottom - top)),
    )
}

fn camera_at_rest(output_size: Size<i32, Physical>) -> Camera {
    Camera::new(
        Vec2 {
            x: output_size.w as f32 / 2.0,
            y: output_size.h as f32 / 2.0,
        },
        Vec2 {
            x: output_size.w as f32,
            y: output_size.h as f32,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_waits_for_maximize_exit_and_preserves_restored_zoom() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), (1280, 800).into());
        let camera = cameras.get_mut("DP-1").unwrap();
        camera.view_size = Vec2 {
            x: 2560.0,
            y: 1600.0,
        };
        camera.target_view_size = camera.view_size;
        let original = *camera;
        cameras.apply_field_maximize("DP-1", Some(1.0));
        let restore = cameras.field_restore_view("DP-1").unwrap();
        assert_eq!(restore.scale, 0.5);
        let destination = Vec2 {
            x: 1700.0,
            y: 900.0,
        };
        cameras.queue_field_center("DP-1", destination);
        cameras.apply_field_maximize("DP-1", Some(0.5));
        assert_eq!(cameras.get("DP-1").unwrap().target_center, original.center);
        cameras.apply_field_maximize("DP-1", None);
        let camera = cameras.get("DP-1").unwrap();
        assert_eq!(camera.center, original.center);
        assert_eq!(camera.target_center, destination);
        assert_eq!(camera.view_size, original.view_size);
    }

    #[test]
    fn display_scale_resize_preserves_field_location_and_zoom() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), (1280, 800).into());
        let camera = cameras.get_mut("DP-1").unwrap();
        camera.center = Vec2 { x: 740.0, y: 480.0 };
        camera.target_center = Vec2 { x: 780.0, y: 530.0 };
        camera.view_size = Vec2 {
            x: 2560.0,
            y: 1600.0,
        };
        camera.target_view_size = Vec2 {
            x: 1920.0,
            y: 1200.0,
        };
        let before = *camera;
        cameras.resize_for_display_scale("DP-1", (640, 400).into());
        let after = cameras.get("DP-1").unwrap();
        assert_eq!(after.center, before.center);
        assert_eq!(after.target_center, before.target_center);
        assert_eq!(scale(after), scale(&before));
        assert_eq!(
            after.view_size,
            Vec2 {
                x: 1280.0,
                y: 800.0
            }
        );
        assert_eq!(after.target_view_size, Vec2 { x: 960.0, y: 600.0 });
    }

    #[test]
    fn outputs_keep_independent_camera_state() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), Size::from((2560, 1440)));
        cameras.insert("DP-2".into(), Size::from((1920, 1200)));

        cameras.get_mut("DP-2").unwrap().view_size = Vec2 {
            x: 3840.0,
            y: 2400.0,
        };
        cameras.get_mut("DP-2").unwrap().center.x += 100.0;

        assert_eq!(cameras.view("DP-1").unwrap().scale, 1.0);
        assert_eq!(
            cameras.view("DP-1").unwrap().center,
            Point::from((1280.0, 720.0))
        );
        assert_eq!(cameras.view("DP-2").unwrap().scale, 0.5);
        assert_eq!(
            cameras.view("DP-2").unwrap().center,
            Point::from((1060.0, 600.0))
        );
    }

    #[test]
    fn output_local_camera_center_rebases_into_global_space() {
        let secondary = Rectangle::<i32, Logical>::new((2560, 0).into(), (1920, 1200).into());

        assert_eq!(
            global_center(Point::from((960.0, 600.0)), secondary),
            Point::from((3520.0, 600.0))
        );
        assert_eq!(
            global_center(Point::from((1060.0, 550.0)), secondary),
            Point::from((3620.0, 550.0))
        );
    }

    #[test]
    fn resting_camera_viewport_matches_output_layout() {
        let secondary = Rectangle::<i32, Logical>::new((2560, 0).into(), (1920, 1200).into());
        let view = OutputView {
            center: Point::from((960.0, 600.0)),
            scale: 1.0,
        };

        assert_eq!(world_viewport(view, secondary), secondary);
    }

    #[test]
    fn zoomed_camera_viewport_expands_around_global_center() {
        let secondary = Rectangle::<i32, Logical>::new((2560, 0).into(), (1920, 1200).into());
        let view = OutputView {
            center: Point::from((1060.0, 550.0)),
            scale: 0.5,
        };

        assert_eq!(
            world_viewport(view, secondary),
            Rectangle::new((1700, -650).into(), (3840, 2400).into())
        );
    }

    #[test]
    fn fullscreen_owns_zoom_and_pan_then_restores_the_live_camera() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), Size::from((1920, 1080)));
        let camera = cameras.get_mut("DP-1").unwrap();
        camera.center = Vec2 { x: 700.0, y: 420.0 };
        camera.view_size = Vec2 {
            x: 3840.0,
            y: 2160.0,
        };
        camera.target_center = camera.center;
        camera.target_view_size = camera.view_size;

        assert!(cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((1100.0, 620.0)),
                progress: 0.5,
                desired: true,
                transition_active: true,
            }),
        ));
        assert!(
            cameras.get_mut("DP-1").is_none(),
            "fullscreen must reject camera mutations"
        );
        assert_eq!(
            cameras.get("DP-1").unwrap().center,
            Vec2 { x: 900.0, y: 520.0 }
        );
        assert_eq!(
            cameras.get("DP-1").unwrap().view_size,
            Vec2 {
                x: 2880.0,
                y: 1620.0,
            }
        );

        cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((1100.0, 620.0)),
                progress: 1.0,
                desired: true,
                transition_active: false,
            }),
        );
        assert_eq!(cameras.view("DP-1").unwrap().scale, 1.0);

        assert!(cameras.apply_fullscreen("DP-1", None));
        let restored = cameras.get_mut("DP-1").unwrap();
        assert_eq!(restored.center, Vec2 { x: 700.0, y: 420.0 });
        assert_eq!(
            restored.view_size,
            Vec2 {
                x: 3840.0,
                y: 2160.0,
            }
        );
    }

    #[test]
    fn fullscreen_retarget_keeps_the_original_restore_snapshot() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), Size::from((1000, 800)));
        cameras.get_mut("DP-1").unwrap().center = Vec2 { x: 300.0, y: 250.0 };

        cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((500.0, 400.0)),
                progress: 1.0,
                desired: true,
                transition_active: false,
            }),
        );
        cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((700.0, 600.0)),
                progress: 1.0,
                desired: true,
                transition_active: false,
            }),
        );
        cameras.apply_fullscreen("DP-1", None);

        assert_eq!(
            cameras.get("DP-1").unwrap().center,
            Vec2 { x: 300.0, y: 250.0 }
        );
    }

    #[test]
    fn fullscreen_to_maximize_handoff_keeps_output_camera_fixed() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), Size::from((1920, 1080)));
        let camera = cameras.get_mut("DP-1").unwrap();
        camera.center = Vec2 { x: 700.0, y: 420.0 };
        camera.view_size = Vec2 {
            x: 3840.0,
            y: 2160.0,
        };
        camera.target_center = camera.center;
        camera.target_view_size = camera.view_size;
        let restore = *camera;

        cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((1100.0, 620.0)),
                progress: 1.0,
                desired: true,
                transition_active: false,
            }),
        );
        let fullscreen = *cameras.get("DP-1").unwrap();
        assert!(cameras.handoff_fullscreen_to_field_maximize("DP-1"));

        for progress in [0.0, 0.5, 1.0] {
            cameras.apply_field_maximize("DP-1", Some(progress));
            let camera = cameras.get("DP-1").unwrap();
            assert_eq!(camera.center, fullscreen.center);
            assert_eq!(camera.view_size, fullscreen.view_size);
        }

        cameras.clear_field_maximize_handoff("DP-1", Some(1.0));
        cameras.apply_field_maximize("DP-1", Some(0.5));
        let midpoint = cameras.get("DP-1").unwrap();
        assert_eq!(
            midpoint.center,
            lerp_vec2(restore.center, fullscreen.center, 0.5)
        );
        assert_eq!(
            midpoint.view_size,
            lerp_vec2(restore.view_size, fullscreen.view_size, 0.5)
        );

        cameras.apply_field_maximize("DP-1", Some(0.0));
        let camera = cameras.get("DP-1").unwrap();
        assert_eq!(camera.center, restore.center);
        assert_eq!(camera.view_size, restore.view_size);
    }

    #[test]
    fn fullscreen_exit_pan_keeps_its_easing_zoom_track_and_cleanup_destination() {
        let mut cameras = OutputCameras::default();
        for name in ["DP-1", "DP-2"] {
            cameras.insert(name.into(), (1920, 1080).into());
            let camera = cameras.get_mut(name).unwrap();
            camera.center = Vec2 { x: 700.0, y: 420.0 };
            camera.target_center = camera.center;
            camera.view_size = Vec2 {
                x: 3840.0,
                y: 2160.0,
            };
            camera.target_view_size = camera.view_size;
        }
        let frame = |progress, desired, transition_active| {
            Some(FullscreenCameraFrame {
                center: (1100.0, 620.0).into(),
                progress,
                desired,
                transition_active,
            })
        };
        for name in ["DP-1", "DP-2"] {
            cameras.apply_fullscreen(name, frame(1.0, true, false));
            cameras.apply_fullscreen(name, frame(1.0, false, false));
            assert!(
                !cameras.is_pannable(name),
                "pending restore must stay locked"
            );
            cameras.apply_fullscreen(name, frame(0.8, false, true));
        }
        assert!(
            cameras.get_mut("DP-1").is_none(),
            "zoom is still owned by fullscreen"
        );
        let camera = cameras.get_mut_for_pan("DP-1").unwrap();
        camera.pan_target(Vec2 {
            x: 200.0,
            y: -100.0,
        });
        camera.pan_vel = Vec2 {
            x: 400.0,
            y: -200.0,
        };
        let view_size = camera.view_size;
        crate::input::zoom::tick_camera(
            camera,
            true,
            &swm_config::Zoom {
                enabled: false,
                ..Default::default()
            },
            8.0,
            1.0 / 60.0,
        );
        assert_eq!(
            camera.view_size, view_size,
            "pan must not clamp the animated zoom"
        );
        let offset = cameras.fullscreen_pan_offset("DP-1");
        let panned = *cameras.get("DP-1").unwrap();
        let velocity = panned.pan_vel;
        let baseline = *cameras.get("DP-2").unwrap();
        let target_offset = Vec2 {
            x: panned.target_center.x - baseline.center.x,
            y: panned.target_center.y - baseline.center.y,
        };
        for progress in [0.6, 0.3, 0.0] {
            cameras.apply_fullscreen("DP-1", frame(progress, false, true));
            cameras.apply_fullscreen("DP-2", frame(progress, false, true));
            let panned = cameras.get("DP-1").unwrap();
            let baseline = cameras.get("DP-2").unwrap();
            assert!((panned.center.x - baseline.center.x - offset.x).abs() < 0.001);
            assert!((panned.center.y - baseline.center.y - offset.y).abs() < 0.001);
            assert!((panned.target_center.x - baseline.center.x - target_offset.x).abs() < 0.001);
            assert_eq!(panned.view_size, baseline.view_size);
            assert_eq!(panned.pan_vel, velocity);
            assert!(cameras.is_pannable("DP-1"));
        }
        let before_cleanup = *cameras.get("DP-1").unwrap();
        cameras.apply_fullscreen("DP-1", None);
        assert_eq!(
            *cameras.get("DP-1").unwrap(),
            before_cleanup,
            "cleanup must preserve live pan, its target, and momentum"
        );
    }

    #[test]
    fn panned_fullscreen_exit_can_reverse_smoothly_and_retains_its_field_destination() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), (1920, 1080).into());
        let restore = *cameras.get("DP-1").unwrap();
        let frame = |progress, desired, transition_active| {
            Some(FullscreenCameraFrame {
                center: (1100.0, 620.0).into(),
                progress,
                desired,
                transition_active,
            })
        };
        cameras.apply_fullscreen("DP-1", frame(1.0, true, false));
        assert!(!cameras.is_pannable("DP-1"));
        cameras.apply_fullscreen("DP-1", frame(0.4, false, true));
        let camera = cameras.get_mut_for_pan("DP-1").unwrap();
        camera.center.x += 100.0;
        camera.target_center = camera.center;
        let before = *camera;
        cameras.apply_fullscreen("DP-1", frame(0.4, true, false));
        assert!((cameras.get("DP-1").unwrap().center.x - before.center.x).abs() < 0.001);
        assert_eq!(cameras.get("DP-1").unwrap().view_size, before.view_size);
        assert!(!cameras.is_pannable("DP-1"));
        cameras.apply_fullscreen("DP-1", frame(1.0, true, false));
        assert_eq!(
            cameras.get("DP-1").unwrap().center,
            Vec2 {
                x: 1100.0,
                y: 620.0
            }
        );
        cameras.apply_fullscreen("DP-1", frame(0.6, false, true));
        assert_eq!(cameras.fullscreen_pan_translation("DP-1"), (-100, 0).into());
        cameras.apply_fullscreen("DP-1", frame(0.0, false, true));
        cameras.apply_fullscreen("DP-1", None);
        assert_eq!(
            cameras.get("DP-1").unwrap().center,
            Vec2 {
                x: restore.center.x + 100.0,
                y: restore.center.y
            }
        );
    }

    #[test]
    fn pointer_pan_is_possible_only_while_the_camera_is_unlocked() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), Size::from((1920, 1080)));
        cameras.insert("DP-2".into(), Size::from((1920, 1080)));

        assert!(cameras.is_pannable("DP-1"));
        assert!(
            !cameras.is_pannable("HDMI-9"),
            "an output without a camera has nothing to pan"
        );

        // Fullscreen owns only its own output's camera.
        cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((1100.0, 620.0)),
                progress: 1.0,
                desired: true,
                transition_active: false,
            }),
        );
        assert!(!cameras.is_pannable("DP-1"));
        assert!(
            cameras.is_pannable("DP-2"),
            "a fullscreen window on one monitor must not lock its neighbor"
        );
        cameras.apply_fullscreen("DP-1", None);
        assert!(cameras.is_pannable("DP-1"));

        // Field maximize keeps ownership of the camera while it is active.
        cameras.apply_field_maximize("DP-1", Some(1.0));
        assert!(!cameras.is_pannable("DP-1"));
        cameras.apply_field_maximize("DP-1", None);
        assert!(cameras.is_pannable("DP-1"));
    }

    #[test]
    fn maximize_to_fullscreen_handoff_keeps_output_camera_fixed() {
        let mut cameras = OutputCameras::default();
        cameras.insert("DP-1".into(), Size::from((1920, 1080)));
        let camera = cameras.get_mut("DP-1").unwrap();
        camera.center = Vec2 { x: 700.0, y: 420.0 };
        camera.view_size = Vec2 {
            x: 3840.0,
            y: 2160.0,
        };
        camera.target_center = camera.center;
        camera.target_view_size = camera.view_size;
        let restore = *camera;

        cameras.apply_field_maximize("DP-1", Some(1.0));
        let maximized = *cameras.get("DP-1").unwrap();
        assert!(cameras.handoff_field_maximize_to_fullscreen("DP-1"));

        for (progress, transition_active) in [(0.0, true), (0.5, true), (1.0, false)] {
            cameras.apply_fullscreen(
                "DP-1",
                Some(FullscreenCameraFrame {
                    center: Point::from((1200.0, 700.0)),
                    progress,
                    desired: true,
                    transition_active,
                }),
            );
            let camera = cameras.get("DP-1").unwrap();
            assert_eq!(camera.center, maximized.center);
            assert_eq!(camera.view_size, maximized.view_size);
        }

        cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((1200.0, 700.0)),
                progress: 0.5,
                desired: false,
                transition_active: true,
            }),
        );
        let midpoint = cameras.get("DP-1").unwrap();
        assert_eq!(
            midpoint.center,
            lerp_vec2(restore.center, maximized.center, 0.5)
        );
        assert_eq!(
            midpoint.view_size,
            lerp_vec2(restore.view_size, maximized.view_size, 0.5)
        );

        cameras.apply_fullscreen(
            "DP-1",
            Some(FullscreenCameraFrame {
                center: Point::from((1200.0, 700.0)),
                progress: 0.0,
                desired: false,
                transition_active: false,
            }),
        );
        let camera = cameras.get("DP-1").unwrap();
        assert_eq!(camera.center, restore.center);
        assert_eq!(camera.view_size, restore.view_size);
    }

    fn output_center(output_size: Size<i32, Physical>) -> Point<f32, Physical> {
        Point::from((output_size.w as f32 / 2.0, output_size.h as f32 / 2.0))
    }

    #[test]
    fn at_rest_is_the_identity_transform() {
        let output_size = Size::<i32, Physical>::from((1280, 800));
        let rect = Rectangle::<i32, Physical>::new((320, 160).into(), (640, 480).into());
        let result = camera_rect(rect, output_center(output_size), output_size, 1.0);
        assert_eq!(result, rect);
    }

    #[test]
    fn zoom_out_shrinks_toward_output_center_when_camera_is_unpanned() {
        let output_size = Size::<i32, Physical>::from((1280, 800));
        // Exactly centered on the output - shrinking around center should
        // leave it centered, only smaller.
        let rect = Rectangle::<i32, Physical>::new((320, 160).into(), (640, 480).into());
        let result = camera_rect(rect, output_center(output_size), output_size, 0.5);
        assert_eq!(result.size, Size::from((320, 240)));
        assert_eq!(result.loc, (480, 280).into());
    }

    #[test]
    fn pan_shifts_everything_opposite_the_camera_offset_at_scale_one() {
        let output_size = Size::<i32, Physical>::from((1280, 800));
        let rect = Rectangle::<i32, Physical>::new((320, 160).into(), (640, 480).into());
        // Camera panned 100px right/50px down from the output's own center -
        // content should appear shifted 100px left/50px up on screen.
        let panned_center = output_center(output_size) + Point::from((100.0, 50.0));
        let result = camera_rect(rect, panned_center, output_size, 1.0);
        assert_eq!(result.loc, (220, 110).into());
        assert_eq!(result.size, rect.size);
    }

    #[test]
    fn camera_projection_preserves_odd_sized_rectangles_at_rest() {
        for output_size in [Size::from((1280, 800)), Size::from((1279, 799))] {
            let rect = Rectangle::new((-21, 17).into(), (641, 479).into());
            assert_eq!(
                camera_rect(rect, output_center(output_size), output_size, 1.0),
                rect
            );
        }
    }

    #[test]
    fn slow_pan_moves_different_sized_surface_parts_together() {
        let output = Size::from((1280, 800));
        let parts = [
            Rectangle::new((320, 160).into(), (640, 480).into()),
            Rectangle::new((333, 177).into(), (101, 79).into()),
            Rectangle::new((-21, -17).into(), (643, 481).into()),
        ];
        for scale in [1.0, 0.75, 0.35] {
            let initial = parts.map(|part| camera_rect(part, output_center(output), output, scale));
            for step in 0..200 {
                let camera = output_center(output)
                    + Point::from((step as f32 * 0.031, step as f32 * -0.047));
                let moved = parts.map(|part| camera_rect(part, camera, output, scale));
                let shift = moved[0].loc - initial[0].loc;
                for index in 1..parts.len() {
                    assert_eq!(
                        moved[index].loc - initial[index].loc,
                        shift,
                        "scale={scale} step={step} part={index}"
                    );
                    assert_eq!(moved[index].size, initial[index].size);
                }
            }
        }
    }

    #[test]
    fn zoomed_adjacent_surface_parts_keep_their_shared_edge() {
        let output = Size::from((1280, 800));
        for scale in [0.35, 0.5, 0.75, 1.0] {
            for step in 0..30 {
                let camera = output_center(output) + Point::from((step as f32 * 0.13, 0.0));
                let left = camera_rect(
                    Rectangle::new((-21, 17).into(), (103, 79).into()),
                    camera,
                    output,
                    scale,
                );
                let right = camera_rect(
                    Rectangle::new((82, 17).into(), (101, 79).into()),
                    camera,
                    output,
                    scale,
                );
                assert_eq!(
                    left.loc.x + left.size.w,
                    right.loc.x,
                    "scale={scale} step={step}"
                );
            }
        }
    }
}
