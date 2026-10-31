//! Placement-aware layout editing for the active mask. Canvas state is transient;
//! only completed, validated commands enter project history.
pub mod edits;
mod geometry;
mod layout;
pub mod mask_view;
mod preview;
mod softkeys;
mod ui;

pub use mask_view::MaskViewState;
pub use ui::{AddObjectRequest, CanvasState, HierarchyOccurrence};
