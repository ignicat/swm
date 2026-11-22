use super::*;
use smithay::utils::{Physical, Rectangle};

#[test]
fn hover_preview_aspect_fit_centers_wide_and_tall_windows() {
    let bounds = Rectangle::<i32, Physical>::new((10, 20).into(), (200, 160).into());
    assert_eq!(
        aspect_fit_rect(bounds, 1_600, 900),
        Rectangle::new((10, 43).into(), (200, 113).into())
    );
    assert_eq!(
        aspect_fit_rect(bounds, 900, 1_600),
        Rectangle::new((65, 20).into(), (90, 160).into())
    );
}

#[test]
fn secondary_window_geometry_becomes_output_local() {
    let secondary = Rectangle::<i32, Logical>::new((2560, 0).into(), (1920, 1200).into());
    let camera_center =
        crate::presentation::camera::global_center(Point::from((1060.0, 550.0)), secondary);
    let world_rect = Rectangle::<i32, Physical>::new((3520, 600).into(), (200, 100).into());

    assert_eq!(
        crate::presentation::camera::camera_rect(
            world_rect,
            camera_center,
            secondary.size.to_physical(1),
            0.5,
        ),
        Rectangle::new((910, 625).into(), (100, 50).into())
    );
}

#[test]
fn fullscreen_rect_interpolates_position_and_size() {
    let windowed = Rectangle::new((100, 50).into(), (800, 600).into());
    let start = crate::wayland::fullscreen::FullscreenPresentation {
        progress: 0.0,
        transition_completion: 0.0,
        windowed_geometry: None,
        windowed_output_rect: None,
        fullscreen_size: (1920, 1080).into(),
    };
    let end = crate::wayland::fullscreen::FullscreenPresentation {
        progress: 1.0,
        ..start
    };
    let middle = crate::wayland::fullscreen::FullscreenPresentation {
        progress: 0.5,
        ..start
    };

    assert_eq!(start.client_rect(windowed, (1920, 1080).into()), windowed);
    assert_eq!(
        end.client_rect(windowed, (1920, 1080).into()),
        Rectangle::new((0, 0).into(), (1920, 1080).into())
    );
    assert_eq!(
        middle.client_rect(windowed, (1920, 1080).into()),
        Rectangle::new((50, 25).into(), (1360, 840).into())
    );
}

#[test]
fn surface_rects_map_into_animated_client_bounds() {
    assert_eq!(
        crate::animation::map_rect(
            Rectangle::new((120, 70).into(), (200, 100).into()),
            Rectangle::new((100, 50).into(), (800, 600).into()),
            Rectangle::new((0, 0).into(), (1600, 1200).into()),
        ),
        Rectangle::new((40, 40).into(), (400, 200).into())
    );
}

#[test]
fn collapsed_node_keeps_front_middle_and_back_stack_depth() {
    let presented = |collapsed_index: usize, remaining: &[(usize, u64)]| {
        let mut groups = remaining
            .iter()
            .copied()
            .map(|(stack_index, order)| StackGroup {
                window: None,
                stack_index,
                order,
                elements: Vec::new(),
            })
            .chain([
                StackGroup {
                    window: None,
                    stack_index: collapsed_index,
                    order: 1,
                    elements: Vec::new(),
                },
                StackGroup {
                    window: None,
                    stack_index: collapsed_index,
                    order: 0,
                    elements: Vec::new(),
                },
            ])
            .collect::<Vec<_>>();
        sort_stack_groups(&mut groups);
        groups
            .into_iter()
            .rev()
            .map(|group| (group.stack_index, group.order))
            .collect::<Vec<_>>()
    };

    // u64::MAX identifies a live window. The close snapshot (1) and
    // resulting node (0) remain on the collapsed window's side of it.
    assert_eq!(
        presented(2, &[(0, u64::MAX), (1, u64::MAX)])[..2],
        [(2, 1), (2, 0)]
    );
    assert_eq!(
        presented(1, &[(0, u64::MAX), (1, u64::MAX)])[..3],
        [(1, u64::MAX), (1, 1), (1, 0)]
    );
    assert_eq!(
        presented(0, &[(0, u64::MAX), (1, u64::MAX)])[..4],
        [(1, u64::MAX), (0, u64::MAX), (0, 1), (0, 0)]
    );
}
