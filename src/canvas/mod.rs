//! Placement-aware layout editing for the active mask. Canvas state is transient;
//! only completed, validated commands enter project history.
pub mod edits;
mod geometry;
mod layout;
mod ui;

pub use ui::CanvasState;
