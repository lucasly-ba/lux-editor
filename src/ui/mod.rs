//! The terminal user interface: rendering the editor and its theme.

pub mod renderer;
pub mod theme;

pub use renderer::{CompletionMenu, HighlightSpan, LineDiagnostic, View, render};
pub use theme::Theme;
