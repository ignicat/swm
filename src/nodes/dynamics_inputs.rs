//! Translate node presentation into physics bodies.
//!
//! This module only observes the state needed for collision geometry; session
//! lifecycle and synchronization remain with the caller.

use super::*;

pub(super) struct DynamicsBodyContext<'a> {
    pub(super) nodes: &'a NodesState,
    pub(super) cameras: &'a crate::presentation::camera::OutputCameras,
    pub(super) placement: PlacementContext<'a>,
}

/// Builds physics inputs without granting the collector access to the session.
pub(super) fn collect_dynamics_bodies(
    context: DynamicsBodyContext<'_>,
    scale_override: Option<(&str, f32)>,
) -> Vec<dynamics::Body> {
    let placement = context.placement;
    context
        .nodes
        .records()
        .filter(|record| {
            record.attached && (record.collapsed || placement.includes_window(&record.surface))
        })
        .filter_map(|record| {
            let node = context.nodes.field.node(record.id)?;
            let scale = scale_override
                .filter(|(output, _)| *output == record.output)
                .map(|(_, scale)| scale)
                .or_else(|| {
                    context
                        .cameras
                        .get(&record.output)
                        .map(crate::presentation::camera::scale)
                })
                .unwrap_or(1.0)
                .max(0.05);
            let (kind, extents) = if record.collapsed {
                (
                    dynamics::BodyKind::Node,
                    dynamics::CollisionExtents::symmetric(Vec2 {
                        x: NODE_DIAMETER_PX * 0.5 / scale,
                        y: NODE_DIAMETER_PX * 0.5 / scale,
                    }),
                )
            } else {
                let outer = crate::window_chrome::outer_rect_for_client(
                    &record.window,
                    record.geometry,
                    context.placement.decorations,
                );
                let center_x = record.geometry.loc.x as f32 + record.geometry.size.w as f32 * 0.5;
                let center_y = record.geometry.loc.y as f32 + record.geometry.size.h as f32 * 0.5;
                (
                    dynamics::BodyKind::Window,
                    dynamics::CollisionExtents {
                        left: center_x - outer.loc.x as f32,
                        right: outer.loc.x as f32 + outer.size.w as f32 - center_x,
                        top: center_y - outer.loc.y as f32,
                        bottom: outer.loc.y as f32 + outer.size.h as f32 - center_y,
                    },
                )
            };
            Some(dynamics::Body {
                id: record.id,
                kind,
                pos: node.pos,
                extents,
                gap: context.nodes.landmarks.gap_px / scale,
                pinned: node.pinned || context.nodes.release_locked(record.id),
                output: record.output.clone(),
            })
        })
        .collect::<Vec<_>>()
}
