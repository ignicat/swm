//! Resolve pointer targets for Field interaction.

use super::*;

pub(super) fn output_at_pointer(
    space: &Space<Window>,
    position: (f64, f64),
) -> Option<(Output, Rectangle<i32, Logical>)> {
    let output = space.output_under(position).next()?.clone();
    let geometry = space.output_geometry(&output)?;
    Some((output, geometry))
}

pub(super) fn node_at_pointer<D: SessionDriver>(
    session: &Session<D>,
) -> Option<(swm_core::field::NodeId, Output)> {
    let position = session.pointer.position();
    let (output, geometry) = output_at_pointer(&session.wayland.space, position)?;
    if crate::session::pointer::presentation_covers_field_point(
        session,
        &output,
        Point::from(position),
        crate::frame_clock::monotonic_now(),
    ) {
        return None;
    }
    let camera = session.cameras.get(&output.name())?;
    let id = session.nodes.hit_test(
        &output,
        geometry,
        camera,
        Point::<f64, Logical>::from(position),
    )?;
    Some((id, output))
}
