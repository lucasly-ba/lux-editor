//! The text layer: a [`Buffer`] (a [`Rope`](crate::rope::Rope) plus file
//! metadata), the [`Edit`] type that represents every change, and the
//! [`Position`] coordinate type used to talk about cursor locations.

mod buffer;
mod edit;
mod position;

pub use buffer::Buffer;
pub use edit::Edit;
pub use position::Position;
