use swm_config::{Animations, Font};

use super::background::BackgroundRenderer;
use super::close::WindowCloseAnimations;
use super::fullscreen_texture::FullscreenTextureTransitions;
use super::node::NodeRenderer;
use super::overlays::preview::OverlayPreviewCache;
use super::pin::PinRenderer;
use super::text::UiTextRenderer;
use super::window_decoration::WindowDecorationRenderer;
use super::window_shader::WindowAnimationShaders;

/// Renderer-owned caches and GPU resources shared by every output.
///
/// Keeping these behind one session field makes their lifetime and reload
/// boundary explicit. Compositor policy remains in `Session`; this aggregate
/// contains only presentation resources that must survive across frames.
pub struct RenderState {
    pub(crate) window_close_animations: WindowCloseAnimations,
    pub(crate) background_renderer: BackgroundRenderer,
    pub(crate) fullscreen_textures: FullscreenTextureTransitions,
    pub(crate) overlay_previews: OverlayPreviewCache,
    pub(crate) node_renderer: NodeRenderer,
    pub(crate) pin_renderer: PinRenderer,
    pub(crate) window_decoration_renderer: WindowDecorationRenderer,
    pub(crate) ui_text: UiTextRenderer,
    pub(crate) window_shaders: WindowAnimationShaders,
}

/// Per-frame mutable view over [`RenderState`].
///
/// Splitting the aggregate once at the request boundary lets the scene builder
/// borrow independent caches without exposing many session-level fields.
pub struct RenderResources<'a> {
    pub window_close_animations: &'a mut WindowCloseAnimations,
    pub background_renderer: &'a mut BackgroundRenderer,
    pub fullscreen_textures: &'a mut FullscreenTextureTransitions,
    pub overlay_previews: &'a mut OverlayPreviewCache,
    pub node_renderer: &'a mut NodeRenderer,
    pub pin_renderer: &'a mut PinRenderer,
    pub window_decoration_renderer: &'a mut WindowDecorationRenderer,
    pub ui_text: &'a mut UiTextRenderer,
    pub window_shaders: &'a mut WindowAnimationShaders,
}

impl<'a> From<&'a mut RenderState> for RenderResources<'a> {
    fn from(state: &'a mut RenderState) -> Self {
        Self {
            window_close_animations: &mut state.window_close_animations,
            background_renderer: &mut state.background_renderer,
            fullscreen_textures: &mut state.fullscreen_textures,
            overlay_previews: &mut state.overlay_previews,
            node_renderer: &mut state.node_renderer,
            pin_renderer: &mut state.pin_renderer,
            window_decoration_renderer: &mut state.window_decoration_renderer,
            ui_text: &mut state.ui_text,
            window_shaders: &mut state.window_shaders,
        }
    }
}

impl RenderState {
    pub fn new(animations: Animations, font: &Font) -> Self {
        Self {
            window_close_animations: WindowCloseAnimations::new(animations.clone()),
            background_renderer: BackgroundRenderer::default(),
            fullscreen_textures: FullscreenTextureTransitions::default(),
            overlay_previews: OverlayPreviewCache::default(),
            node_renderer: NodeRenderer::default(),
            pin_renderer: PinRenderer::default(),
            window_decoration_renderer: WindowDecorationRenderer::default(),
            ui_text: UiTextRenderer::new(font),
            window_shaders: {
                let mut shaders = WindowAnimationShaders::default();
                shaders.reload(&animations);
                shaders
            },
        }
    }
}
