use std::collections::HashMap;

use crate::field::{Field, NodeId, NodeState, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortalDir {
    N,
    E,
    S,
    W,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn inset(&self, pad: f32) -> Rect {
        Rect {
            x: self.x + pad,
            y: self.y + pad,
            w: (self.w - 2.0 * pad).max(0.0),
            h: (self.h - 2.0 * pad).max(0.0),
        }
    }
}

/// True if `id` is eligible to receive focus via cycling (alt-tab-style):
/// visible and in a representation state that's actually a real window
/// (`Active` or the decayed `Node` dot).
pub fn is_focus_cycle_candidate(field: &Field, id: NodeId) -> bool {
    field.node(id).is_some_and(|node| {
        field.is_visible(id) && matches!(node.state, NodeState::Active | NodeState::Node)
    })
}

/// Orders focus-cycle candidates most-recent-first (breaking ties by descending ID),
/// with `origin` placed first when still eligible.
pub fn focus_cycle_candidates_where(
    field: &Field,
    last_focus_ms: &HashMap<NodeId, u64>,
    origin: Option<NodeId>,
    include: impl Fn(NodeId) -> bool,
) -> Vec<NodeId> {
    let mut candidates: Vec<NodeId> = field
        .node_ids_all()
        .into_iter()
        .filter(|&id| include(id))
        .collect();

    candidates.sort_by(|a, b| {
        let a_at = last_focus_ms.get(a).copied().unwrap_or(0);
        let b_at = last_focus_ms.get(b).copied().unwrap_or(0);
        b_at.cmp(&a_at).then_with(|| b.as_u64().cmp(&a.as_u64()))
    });

    if let Some(origin) = origin
        && let Some(index) = candidates.iter().position(|&id| id == origin)
    {
        let o = candidates.remove(index);
        candidates.insert(0, o);
    }

    candidates
}

/// World-space (field) rectangle for a visible directional-focus target.
/// Uses the node's current representation footprint so expanded windows and
/// collapsed nodes participate with their visible size.
pub fn node_field_rect(field: &Field, id: NodeId) -> Option<Rect> {
    let node = field.node(id)?;
    if !field.is_visible(id) {
        return None;
    }
    let size = node.footprint;
    let w = size.x.max(1.0);
    let h = size.y.max(1.0);
    Some(Rect {
        x: node.pos.x - w * 0.5,
        y: node.pos.y - h * 0.5,
        w,
        h,
    })
}

fn rect_center(rect: Rect) -> Vec2 {
    Vec2 {
        x: rect.x + rect.w * 0.5,
        y: rect.y + rect.h * 0.5,
    }
}

fn range_gap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    if a1 < b0 {
        b0 - a1
    } else if b1 < a0 {
        a0 - b1
    } else {
        0.0
    }
}

/// Directional-focus neighbor scoring: lower `(orth_gap, main_delta,
/// dist2, tie_axis)` (compared lexicographically) is a better match.
/// Returns `None` if `candidate` isn't actually positioned in `direction`
/// from `current` (`main_delta <= 0.5`).
///
/// Uses `world::PortalDir` to model the four cardinal directions (`N`=up,
/// `S`=down, `E`=right, `W`=left).
pub fn directional_candidate_score(
    current: Rect,
    candidate: Rect,
    direction: PortalDir,
) -> Option<(f32, f32, f32, f32)> {
    let current_center = rect_center(current);
    let candidate_center = rect_center(candidate);
    let (main_delta, orth_gap, tie_axis) = match direction {
        PortalDir::W => (
            current_center.x - candidate_center.x,
            range_gap(
                current.y,
                current.y + current.h,
                candidate.y,
                candidate.y + candidate.h,
            ),
            candidate_center.y,
        ),
        PortalDir::E => (
            candidate_center.x - current_center.x,
            range_gap(
                current.y,
                current.y + current.h,
                candidate.y,
                candidate.y + candidate.h,
            ),
            candidate_center.y,
        ),
        PortalDir::N => (
            current_center.y - candidate_center.y,
            range_gap(
                current.x,
                current.x + current.w,
                candidate.x,
                candidate.x + candidate.w,
            ),
            candidate_center.x,
        ),
        PortalDir::S => (
            candidate_center.y - current_center.y,
            range_gap(
                current.x,
                current.x + current.w,
                candidate.x,
                candidate.x + candidate.w,
            ),
            candidate_center.x,
        ),
    };
    if main_delta <= 0.5 {
        return None;
    }
    let dx = candidate_center.x - current_center.x;
    let dy = candidate_center.y - current_center.y;
    Some((orth_gap, main_delta, dx * dx + dy * dy, tie_axis))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Vec2;

    #[test]
    fn cycle_candidate_excludes_hidden() {
        let mut field = Field::new();
        let active = field.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let hidden = field.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        field.set_hidden(hidden, true);

        assert!(is_focus_cycle_candidate(&field, active));
        assert!(!is_focus_cycle_candidate(&field, hidden));
    }

    #[test]
    fn cycle_candidates_order_by_recency_then_move_origin_to_front() {
        let mut field = Field::new();
        let a = field.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let b = field.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let c = field.spawn_surface("C", Vec2 { x: 20.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let mut last_focus_ms = HashMap::new();
        last_focus_ms.insert(a, 100);
        last_focus_ms.insert(b, 300);
        last_focus_ms.insert(c, 200);

        let candidates = focus_cycle_candidates_where(&field, &last_focus_ms, None, |id| {
            is_focus_cycle_candidate(&field, id)
        });
        assert_eq!(candidates, vec![b, c, a]);

        let candidates = focus_cycle_candidates_where(&field, &last_focus_ms, Some(a), |id| {
            is_focus_cycle_candidate(&field, id)
        });
        assert_eq!(candidates, vec![a, b, c]);
    }

    #[test]
    fn cycle_candidates_defaults_missing_timestamps_to_zero() {
        let mut field = Field::new();
        let known = field.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });
        let unknown = field.spawn_surface("B", Vec2 { x: 10.0, y: 0.0 }, Vec2 { x: 10.0, y: 10.0 });

        let mut last_focus_ms = HashMap::new();
        last_focus_ms.insert(known, 50);

        let candidates = focus_cycle_candidates_where(&field, &last_focus_ms, None, |id| {
            is_focus_cycle_candidate(&field, id)
        });
        assert_eq!(candidates, vec![known, unknown]);
    }

    #[test]
    fn node_field_rect_uses_visible_window_and_node_footprints() {
        let mut field = Field::new();
        let active = field.spawn_surface("A", Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });
        let dot = field.spawn_surface("B", Vec2 { x: 100.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });
        field.set_state(dot, NodeState::Node);
        let hidden =
            field.spawn_surface("D", Vec2 { x: 300.0, y: 0.0 }, Vec2 { x: 100.0, y: 50.0 });
        field.set_hidden(hidden, true);

        let rect = node_field_rect(&field, active).unwrap();
        assert_eq!(rect.x, -50.0);
        assert_eq!(rect.y, -25.0);
        assert_eq!(rect.w, 100.0);
        assert_eq!(rect.h, 50.0);

        let dot_rect = node_field_rect(&field, dot).unwrap();
        assert!(dot_rect.w < rect.w);
        assert!(dot_rect.h < rect.h);
        assert!(node_field_rect(&field, hidden).is_none());
    }

    fn active_rect(field: &mut Field, x: f32, y: f32) -> Rect {
        let id = field.spawn_surface("N", Vec2 { x, y }, Vec2 { x: 100.0, y: 100.0 });
        node_field_rect(field, id).unwrap()
    }

    #[test]
    fn directional_score_picks_nearest_neighbor_per_direction() {
        let mut field = Field::new();
        let center = active_rect(&mut field, 400.0, 300.0);
        let right = active_rect(&mut field, 700.0, 300.0);
        let up = active_rect(&mut field, 400.0, 60.0);
        let overlap = active_rect(&mut field, 460.0, 300.0);

        let best = |current: Rect, candidates: &[Rect], dir: PortalDir| -> Option<Rect> {
            candidates
                .iter()
                .copied()
                .filter_map(|c| directional_candidate_score(current, c, dir).map(|s| (s, c)))
                .min_by(|(a, _), (b, _)| a.partial_cmp(b).unwrap())
                .map(|(_, c)| c)
        };

        assert_eq!(best(center, &[right, up, overlap], PortalDir::N), Some(up));
        assert_eq!(
            best(center, &[right, up, overlap], PortalDir::E),
            Some(overlap)
        );
        assert_eq!(
            best(overlap, &[right, up, center], PortalDir::E),
            Some(right)
        );
        assert_eq!(best(center, &[right, up, overlap], PortalDir::W), None);
    }
}
