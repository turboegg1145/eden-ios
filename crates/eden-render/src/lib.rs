//! `eden-render` — Musica 1024x640 stage, viewport scrolling, rule-mask
//! transitions, `.ani` overlays, firefly particles and text.
//!
//! Two backends share the same scene description:
//!
//! * [`SoftwareRenderer`] — deterministic RGBA framebuffer used for pixel-diff
//!   calibration and headless screenshot export.
//! * [`gpu::GlowRenderer`] — OpenGL 3.3 / OpenGL ES 3.0 shader backend used on
//!   desktop and iOS (via `CAEAGLLayer`). Metal is provided by the iOS shell.

pub mod framebuffer;
pub mod gpu;
pub mod particles;
pub mod renderer;
pub mod scene;
pub mod text;
pub mod texture;

pub use framebuffer::Framebuffer;
pub use gpu::GlowRenderer;
pub use particles::{Firefly, FireflySystem};
pub use renderer::{rule_alpha, SoftwareRenderer};
pub use scene::{
    BlendMode, Layer, MessageOverlay, PanelOverlay, Scene, ShakeKind, ShakeState, StageSpec,
    Transition,
};
pub use text::{FontRenderer, TextLayout};
pub use texture::TextureCache;
