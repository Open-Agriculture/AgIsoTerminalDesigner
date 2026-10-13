//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

mod allowed_object_relationships;
mod editor_project;
mod interactive_rendering_simple;
mod layout_containment;
mod object_configuring;
mod object_defaults;
mod object_info;
mod object_rendering;
mod output_string_rendering;
mod possible_events;
mod project_file;
mod smart_naming;

// Operation system and automation support
pub mod ai;
pub mod canvas;
pub mod components;
pub mod effective_colour;
pub mod graphics;
pub mod isobus_colour;
pub mod object_picker;
pub mod object_properties;
pub mod operations;
pub mod pool_validation;

pub use editor_project::EditorProject;
pub use interactive_rendering_simple::InteractiveMaskRenderer;
pub use object_configuring::render_property_editor;
pub use object_defaults::default_object;
pub use object_info::ObjectInfo;
pub use object_rendering::RenderableObject;

// Operation system exports
pub use ai::snapshot::{DesignSnapshot, PoolSnapshot};
pub use ai::{AiDesignState, AiProposal, ProjectRevision};
pub use object_properties::{
    get_properties, PropertyAccess, PropertyAccessError, PropertyDescriptor, PropertyError,
    PropertySemantic,
};
pub use operations::{
    AppliedTransaction, ChildPlacement, DiagnosticSeverity, MacroReferenceTarget, ObjectTarget,
    ObjectTargetList, Operation, OperationError, OperationExecutor, OperationHistory,
    OperationTransaction, PreparedTransaction, TransactionDiagnostic, TransactionDiagnostics,
    ValidationContext, CURRENT_OPERATION_SCHEMA_VERSION,
};
pub use pool_validation::{ValidationDiagnostic, ValidationSeverity};
