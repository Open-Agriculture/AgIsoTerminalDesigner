//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

mod allowed_object_relationships;
mod editor_project;
mod id_reassignment;
mod interactive_rendering_simple;
mod layout_containment;
mod object_configuring;
mod object_defaults;
mod object_id_format;
mod object_id_ranges;
mod object_info;
mod object_rendering;
mod output_string_rendering;
mod possible_events;
mod project_file;
mod smart_naming;
mod soft_key_layout;

// Operation system and automation support
pub mod canvas;
pub mod cli;
pub mod components;
pub mod effective_colour;
pub mod graphics;
pub mod isobus_colour;
pub mod macro_id_remap;
pub mod macro_authoring;
pub mod object_picker;
pub mod object_properties;
pub mod operations;
pub mod pool_validation;
pub mod ui_style;
pub mod vt_compatibility;
pub mod vt_macro_commands;
pub mod vt_strings;

pub use editor_project::{EditorProject, ObjectIdAllocationError};
pub use id_reassignment::{IdReassignmentPlan, ReassignmentRow};
pub use interactive_rendering_simple::InteractiveMaskRenderer;
pub use object_configuring::{finish_property_edits, render_property_editor};
pub use object_defaults::{default_object, default_object_for_version};
pub use object_id_format::ObjectIdFormat;
pub use object_id_ranges::{ObjectIdRange, ObjectIdRanges};
pub use object_info::ObjectInfo;
pub use object_rendering::{render_object_with_version, RenderableObject};
pub use soft_key_layout::{SoftKeyArrangement, SoftKeyLayout};

// Operation system exports
pub use cli::snapshot::{DesignSnapshot, PoolSnapshot};
pub use cli::{AiDesignState, ProjectRevision};
pub use object_properties::{
    get_properties, PropertyAccess, PropertyAccessError, PropertyDescriptor, PropertyError,
    PropertyKey, PropertySemantic,
};
pub use operations::{
    AppliedTransaction, ChildPlacement, DiagnosticSeverity, MacroReferenceTarget, ObjectTarget,
    ObjectTargetList, Operation, OperationError, OperationExecutor, OperationHistory,
    OperationTransaction, PreparedTransaction, TransactionDiagnostic, TransactionDiagnostics,
    ValidationContext, CURRENT_OPERATION_SCHEMA_VERSION,
};
pub use pool_validation::{ValidationDiagnostic, ValidationSeverity};
