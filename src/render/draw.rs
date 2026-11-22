//! The renderers compositor-drawn UI needs, borrowed together.
//!
//! Overlay cards, badges and dialogs all draw with the same three renderers:
//! the GLES renderer, the node renderer that owns the card and label shaders,
//! and the UI text renderer. Passing them as one value keeps signatures short
//! and lets a fourth renderer be added in one place instead of at every call.

use std::error::Error;

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Physical, Rectangle};

use super::node::{LabelRenderElement, NodeRenderer};
use super::overlays::shell::{OverlayRgb, OverlayVisuals, card_element, label_card_element};
use super::text::UiTextRenderer;

pub(crate) struct DrawContext<'a> {
    pub renderer: &'a mut GlesRenderer,
    pub node: &'a mut NodeRenderer,
    pub text: &'a mut UiTextRenderer,
}

impl DrawContext<'_> {
    pub fn card(
        &mut self,
        destination: Rectangle<i32, Physical>,
        visuals: OverlayVisuals,
        fill: OverlayRgb,
        alpha: f32,
    ) -> Result<LabelRenderElement, Box<dyn Error>> {
        card_element(self.renderer, self.node, destination, visuals, fill, alpha)
    }

    pub fn label_card(
        &mut self,
        destination: Rectangle<i32, Physical>,
        visuals: OverlayVisuals,
        fill: OverlayRgb,
        alpha: f32,
    ) -> Result<LabelRenderElement, Box<dyn Error>> {
        label_card_element(self.renderer, self.node, destination, visuals, fill, alpha)
    }
}
