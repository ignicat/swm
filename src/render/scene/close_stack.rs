//! Compose closing-window snapshots into the desktop stack.

use super::*;

pub(super) struct ClosingSceneContext<'a> {
    pub animations: &'a mut crate::render::close::WindowCloseAnimations,
    pub shaders: &'a mut crate::render::window_shader::WindowAnimationShaders,
    pub cameras: &'a crate::presentation::camera::OutputCameras,
    pub now: std::time::Duration,
    pub redact: bool,
}

pub(super) fn stack_groups(
    renderer: &mut GlesRenderer,
    output: &Output,
    output_geometry: Rectangle<i32, Logical>,
    context: ClosingSceneContext<'_>,
) -> Result<Vec<StackGroup>, Box<dyn Error>> {
    let stack = context
        .animations
        .renders_for_output(
            renderer,
            context.shaders,
            output,
            output_geometry,
            context.cameras,
            context.now,
        )
        .into_iter()
        .map(|closing| -> Result<StackGroup, Box<dyn Error>> {
            if context.redact {
                return Ok(StackGroup {
                    window: None,
                    stack_index: closing.stack_index,
                    order: closing.order,
                    elements: Vec::new(),
                });
            }
            let mut elements = Vec::new();
            let shader = closing.shader;
            if let Some(border) = closing.border {
                elements.extend(
                    crate::render::border_strips(
                        std::array::from_fn(|index| closing.border_id.namespaced(100 + index)),
                        closing.destination,
                        border.width,
                        border.color,
                    )
                    .into_iter()
                    .map(SceneElement::Border),
                );
            }
            if let Some(shader) = shader {
                elements.push(SceneElement::WindowShader(shader));
            } else {
                elements.push(SceneElement::Closing(closing.texture));
            }
            Ok(StackGroup {
                window: None,
                stack_index: closing.stack_index,
                order: closing.order,
                elements,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stack)
}
